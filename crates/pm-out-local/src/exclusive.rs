//! Exclusive USB: the app drives the phone's USB DAC itself, with its own driver
//! ([`pm_usb_audio::driver`]) on the USB device Android hands over (`UsbExclusive.kt`), for phones
//! below Android 14 or where Android refuses bit-perfect.
//!
//! * The listener chooses it (Settings > This phone, "Exclusive USB"): the app asks Android's
//!   permission for the DAC then, and only then ([`settings_changed`]). Auto uses it by itself
//!   only where bit-perfect isn't offered or was refused, and only once that permission exists:
//!   it never asks.
//! * The DAC is held from the first song until the mode changes, the DAC goes or the app closes
//!   ([`release`]): between songs it stays claimed, so Android's driver doesn't come and go.
//! * Never louder: the DAC's feature unit starts at what Android's media volume played at through
//!   it, less a margin ([`pm_usb_audio::volume::handover_cur`]), and goes back no louder than it
//!   was. It is the zone's volume while "Use the DAC's volume" is on. A DAC without one plays at
//!   full level: as with bit-perfect, Auto uses it only at Android's full volume, and Exclusive
//!   USB below that only with the listener's agreement ([`volume_rule`]).
//! * Confirmed only when the clock read back exactly the track's rate and the stream runs.
//!
//! Everything here but the JNI backend runs on any host, against a fake DAC.

use crate::usb_dac::{self, Dac, LastOpen};
use pm_model::{
    AudioBlock, AudioFormat, OutputCaps, QualityCeiling, SampleKind, Samples, Volume, VolumeKind,
};
use pm_usb_audio::driver::{self, Driver, DriverError, Plan, UsbError};
use pm_usb_audio::quirks::{self, Quirks};
use pm_usb_audio::schedule::Speed;
use pm_usb_audio::volume::{self as vol, VolumeRange};
use pm_usb_audio::{Uac, UsbAudioDevice};
use pm_zones::{AudioSink, BufferLevel, DeviceFormat, DeviceMode, OutputId, PreferredMode, SinkError};
use std::sync::{Arc, Mutex, RwLock};

pub use pm_usb_audio::driver::Transport;
pub use pm_zones::UsbIds;

/// The app's side (`UsbExclusive.kt`, over JNI): Android's permission and the open device, the
/// DAC found by its USB ids.
pub trait UsbBackend: Send + Sync {
    /// The app may open the DAC (Android's permission was given). Never asks.
    fn has_permission(&self, usb: UsbIds) -> bool;
    /// Ask for Android's permission (its dialog): only when the listener chose Exclusive USB.
    fn request_permission(&self, usb: UsbIds);
    /// Open the DAC with its audio interfaces taken from Android's driver: the transport on its
    /// file descriptor, and its raw descriptors.
    fn open(&self, usb: UsbIds) -> Result<(Arc<dyn Transport>, Vec<u8>), String>;
    /// Close the connection (the driver gave the interfaces back first).
    fn release(&self);
}

/// `UsbExclusive.open`'s answer: the file descriptor and the raw descriptors, or why not.
pub fn parse_open_answer(json: &str) -> Result<(i32, Vec<u8>), String> {
    #[derive(serde::Deserialize)]
    struct Answer {
        ok: bool,
        #[serde(default)]
        why: Option<String>,
        #[serde(default)]
        fd: i32,
        #[serde(default)]
        descriptors: String,
    }
    let a: Answer = serde_json::from_str(json).map_err(|e| format!("the USB driver's answer: {e}"))?;
    if !a.ok {
        return Err(a.why.unwrap_or_else(|| "Android would not open the DAC".into()));
    }
    if a.fd < 0 {
        return Err("Android gave no file descriptor for the DAC".into());
    }
    let raw = usb_dac::unhex(&a.descriptors).ok_or("the DAC's descriptors could not be read")?;
    Ok((a.fd, raw))
}

/// The kernel's usbfs on the file descriptor Android gave (it stays the app's to close).
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn usbfs(fd: i32) -> Arc<dyn Transport> {
    Arc::new(pm_usb_audio::usbfs::Usbfs::new(fd))
}

/// The USB ids of the DAC (`None`: Android didn't say).
fn ids(dac: &Dac) -> Option<UsbIds> {
    dac.device.usb
}

static BACKEND: RwLock<Option<Arc<dyn UsbBackend>>> = RwLock::new(None);

/// The app's USB side (set once at start).
pub fn set_backend(b: Arc<dyn UsbBackend>) {
    *BACKEND.write().unwrap_or_else(|e| e.into_inner()) = Some(b);
}

pub fn backend() -> Option<Arc<dyn UsbBackend>> {
    BACKEND.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The DAC while the app holds it.
struct Held {
    dac_id: i32,
    dac_name: String,
    t: Arc<dyn Transport>,
    dev: UsbAudioDevice,
    clock_rates: Vec<u32>,
    speed: Speed,
    quirks: Quirks,
    /// The feature unit (unit, AudioControl interface, range), when it has a usable one.
    unit: Option<(u8, u8, VolumeRange)>,
    /// Its level before the app took it over, and now (1/256 dB).
    before: Option<i16>,
    level: Option<i16>,
    muted: bool,
    /// The driver, between songs (a sink has it while it plays).
    driver: Option<Driver>,
    /// A sink plays on it now.
    in_use: bool,
    /// Give it back once the sink closes.
    release_pending: bool,
}

static HELD: Mutex<Option<Held>> = Mutex::new(None);

fn with_held<T>(f: impl FnOnce(&mut Option<Held>) -> T) -> T {
    f(&mut HELD.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Whether the app holds the DAC Android knows as `dac_id`.
pub fn holds(dac_id: i32) -> bool {
    with_held(|h| h.as_ref().is_some_and(|h| h.dac_id == dac_id))
}

/// Whether the app holds any DAC.
pub fn holding() -> bool {
    with_held(|h| h.is_some())
}

/// Whether this app and DAC allow Exclusive USB at all (a UAC1 or UAC2 DAC with USB ids, the
/// app's driver present), or why not.
pub fn offer(dac: &Dac) -> Result<(), String> {
    if backend().is_none() {
        return Err("this app has no USB driver here".into());
    }
    if dac.device.usb.is_none() {
        return Err("Android did not say which USB device the DAC is".into());
    }
    Ok(())
}

/// How the phone plays when [`PreferredMode`] is `mode`: exclusive or not. Auto takes it only where
/// bit-perfect is not offered (or was refused) and permission was given before.
pub fn chosen(mode: PreferredMode, bit_perfect_offered: bool, permission: bool) -> bool {
    match mode {
        PreferredMode::Exclusive => true,
        PreferredMode::Auto => !bit_perfect_offered && permission,
        _ => false,
    }
}

/// Never louder: with a feature unit the hand-over sets its level, so it may start at any media
/// volume. Without one the DAC plays at full level: Auto only at Android's full volume; Exclusive
/// USB below that only with the listener's agreement (then `Ok(Some(warning))`).
pub fn volume_rule(
    mode: PreferredMode,
    feature_unit: bool,
    media_percent: Option<u8>,
    full_level_ok: bool,
) -> Result<Option<String>, String> {
    if feature_unit || media_percent == Some(100) {
        return Ok(None);
    }
    let at = media_percent.map(|p| format!(" ({p}%)")).unwrap_or_default();
    match (mode, full_level_ok) {
        (PreferredMode::Exclusive, true) => Ok(Some(format!(
            "Exclusive USB plays at full level: this DAC has no volume of its own, and Android's volume{at} does not apply. Set the level on the amplifier"
        ))),
        (PreferredMode::Exclusive, false) => Err(format!(
            "Exclusive USB plays at full level on this DAC (it has no volume of its own), and Android's volume{at} would no longer apply: confirm Exclusive USB in Settings to use it"
        )),
        _ => Err(match media_percent {
            Some(p) => format!(
                "Android's volume is at {p}%, and this DAC has no volume of its own: exclusive USB would play at full level"
            ),
            None => "Android's volume could not be read, and this DAC has no volume of its own".into(),
        }),
    }
}

/// The DAC's descriptors as the app reported them (with an earlier permission), parsed.
fn reported(dac: &Dac) -> Option<UsbAudioDevice> {
    dac.descriptors
        .as_deref()
        .map(pm_usb_audio::parse)
        .filter(|d| d.uac.is_some())
}

/// Whether the DAC has a feature unit the driver can use, as far as is known now.
fn has_unit(dac: &Dac) -> bool {
    if let Some(h) = with_held(|h| {
        h.as_ref()
            .filter(|h| h.dac_id == dac.id)
            .map(|h| h.unit.is_some())
    }) {
        return h;
    }
    let ids = dac.device.usb.map(|u| (u.vendor_id, u.product_id));
    let quirk = ids.is_some_and(|(v, p)| quirks::lookup(v, p).no_hardware_volume);
    !quirk
        && reported(dac)
            .is_some_and(|d| d.uac == Some(Uac::V2) && d.hardware_volume.as_ref().is_some_and(|v| v.volume))
}

/// Every format the DAC plays unchanged in Exclusive USB: from the held DAC (its clock's ranges)
/// or, before, from its reported descriptors and the rates Android lists for it.
pub fn formats(dac: &Dac) -> Vec<DeviceFormat> {
    let held = with_held(|h| {
        h.as_ref()
            .filter(|h| h.dac_id == dac.id)
            .map(|h| pm_usb_audio::stream::formats(&h.dev, &h.clock_rates))
    });
    let list = match held {
        Some(l) => l,
        None => match reported(dac) {
            Some(d) => pm_usb_audio::stream::formats(&d, &dac.device.rates),
            None => Vec::new(),
        },
    };
    list.into_iter()
        .map(|(rate_hz, bits, channels)| DeviceFormat {
            rate_hz,
            bits,
            channels,
        })
        .collect()
}

/// The stereo rates and sizes listed together (every rate with every size): what a node may plan
/// with when it plans rate and size apart. More rates first, then the larger size.
pub fn rectangle(formats: &[DeviceFormat]) -> (Vec<u32>, Vec<u8>) {
    let stereo: Vec<&DeviceFormat> = formats.iter().filter(|f| f.channels == 2).collect();
    let mut rates: Vec<u32> = stereo.iter().map(|f| f.rate_hz).collect();
    rates.sort_unstable();
    rates.dedup();
    let sets: [&[u8]; 6] = [&[16, 24, 32], &[24, 32], &[16, 24], &[32], &[24], &[16]];
    let mut best: (Vec<u32>, Vec<u8>) = (Vec::new(), Vec::new());
    for set in sets {
        let r: Vec<u32> = rates
            .iter()
            .copied()
            .filter(|rate| {
                set.iter()
                    .all(|b| stereo.iter().any(|f| f.rate_hz == *rate && f.bits == *b))
            })
            .collect();
        if r.len() > best.0.len() || (r.len() == best.0.len() && !r.is_empty() && set.last() > best.1.last())
        {
            best = (r, set.to_vec());
        }
    }
    best
}

/// The zone's volume on the DAC: its feature unit's range in dB, while the app holds it, has one,
/// and "Use the DAC's volume" is on.
pub fn hw_volume(dac: &Dac, dac_volume: bool) -> Option<VolumeKind> {
    if !dac_volume {
        return None;
    }
    with_held(|h| {
        h.as_ref()
            .filter(|h| h.dac_id == dac.id)
            .and_then(|h| h.unit)
            .map(|(_, _, r)| VolumeKind::Db {
                min: vol::cur_to_db(r.min),
                max: vol::cur_to_db(r.max),
                step: f32::from(r.res.max(1)) / 256.0,
            })
    })
}

/// Whether the phone plays this DAC exclusive now: `None` when it isn't chosen (or offered);
/// `Some(Ok(warning))` when it plays exclusive; `Some(Err(why))` when it was chosen but can't
/// (no permission yet, or louder than now).
pub fn decide(
    dac: &Dac,
    settings: pm_zones::DeviceSettings,
    full_level_ok: bool,
    media: Option<u8>,
) -> Option<Result<Option<String>, String>> {
    let b = backend()?;
    offer(dac).ok()?;
    let permission = holds(dac.id) || b.has_permission(ids(dac)?);
    if !chosen(settings.mode, crate::bit_perfect::offer(dac).is_ok(), permission) {
        return None;
    }
    if !permission {
        return Some(Err(
            "Exclusive USB needs Android's permission for the DAC: allow it when Android asks (choose Exclusive USB again in Settings to ask)".into(),
        ));
    }
    Some(volume_rule(settings.mode, has_unit(dac), media, full_level_ok))
}

/// Whether the phone plays this DAC exclusive now (offered, chosen, allowed, never louder).
pub fn plays_exclusive(
    dac: &Dac,
    settings: pm_zones::DeviceSettings,
    full_level_ok: bool,
    media: Option<u8>,
) -> bool {
    matches!(decide(dac, settings, full_level_ok, media), Some(Ok(_)))
}

/// Auto after Android refused bit-perfect: Exclusive USB instead, when permission was given.
pub fn auto_fallback(dac: &Dac, media: Option<u8>) -> Option<Option<String>> {
    let b = backend()?;
    offer(dac).ok()?;
    if !(holds(dac.id) || b.has_permission(ids(dac)?)) {
        return None;
    }
    volume_rule(PreferredMode::Auto, has_unit(dac), media, false).ok()
}

/// What the phone's output takes in Exclusive USB: the DAC's own rates and sizes, its volume
/// when the zone may use it. `None` when it doesn't play exclusive or nothing is known yet.
pub fn live_caps(
    dac: &Dac,
    settings: pm_zones::DeviceSettings,
    full_level_ok: bool,
    media: Option<u8>,
    shared: &OutputCaps,
) -> Option<OutputCaps> {
    if !plays_exclusive(dac, settings, full_level_ok, media) {
        return None;
    }
    let (rates, bits) = rectangle(&formats(dac));
    if rates.is_empty() {
        return None;
    }
    Some(OutputCaps {
        max_pcm_bits: bits.last().copied().unwrap_or(16),
        pcm_rates: rates,
        pcm_bits: bits,
        max_channels: 2,
        dsd: None,
        hw_volume: hw_volume(dac, settings.dac_volume),
        quality_ceiling: QualityCeiling::BitPerfect,
        ..shared.clone()
    })
}

/// The listener changed the settings: choosing Exclusive USB asks Android's permission for the
/// DAC (the only place it is asked); a mode that doesn't use it gives the DAC back.
pub fn settings_changed(mode: PreferredMode) {
    match mode {
        PreferredMode::Exclusive => {
            let usb = usb_dac::current_dac().as_ref().and_then(ids);
            if let (Some(b), Some(usb)) = (backend(), usb) {
                if !holding() && !b.has_permission(usb) {
                    b.request_permission(usb);
                }
            }
        }
        PreferredMode::Auto => {}
        _ => release(),
    }
}

/// Give the DAC back to Android: now when no song plays on it, else when that sink closes.
pub fn release() {
    let now = with_held(|h| match h.as_mut() {
        Some(x) if x.in_use => {
            x.release_pending = true;
            None
        }
        _ => h.take(),
    });
    if let Some(h) = now {
        give_back(h);
    }
}

/// The DAC went or another took its place: let go of it at once (its sink stops with Gone).
pub fn dac_changed(dac_id: Option<i32>) {
    let gone = with_held(|h| match h.as_ref() {
        Some(x) if Some(x.dac_id) != dac_id => h.take(),
        _ => None,
    });
    if let Some(h) = gone {
        give_back(h);
    }
}

/// Every audio interface the app claimed back to Android's driver, the AudioControl one last (the
/// driver that probes it then finds the streaming ones free). Unknown: the first eight.
fn hand_back(t: &dyn Transport, dev: &UsbAudioDevice) {
    let mut all: Vec<u8> = dev.audio_interfaces.clone();
    if all.is_empty() {
        all = (0..8).collect();
    }
    let ac = dev.control_interface;
    for i in all.iter().filter(|i| Some(**i) != ac) {
        t.release(*i);
    }
    if let Some(ac) = ac {
        t.release(ac);
    }
}

fn give_back(mut h: Held) {
    if let (Some((unit, ac, _)), Some(level)) = (h.unit, h.level) {
        let _ = driver::set_volume(&*h.t, unit, ac, vol::giveback_cur(h.before, level));
    }
    if let Some(mut d) = h.driver.take() {
        d.stop();
    }
    hand_back(&*h.t, &h.dev);
    tracing::info!(dac = %h.dac_name, "Exclusive USB: the DAC goes back to Android");
    if let Some(b) = backend() {
        b.release();
    }
}

/// Why the exclusive driver gave no sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExclusiveError {
    /// Not used (no permission, louder than now, not a DAC it drives): Android plays it, and why.
    Refused(String),
    /// The DAC can't play this format unchanged.
    Unsupported(String),
    /// The DAC is held but would not start: nothing plays (never another device).
    Failed(String),
    /// The DAC went away.
    Gone(String),
}

/// Take the DAC over (or keep it): its descriptors, clock rates and feature unit.
fn acquire(dac: &Dac) -> Result<(), ExclusiveError> {
    if holds(dac.id) {
        return Ok(());
    }
    // Another DAC still held: back to Android first
    dac_changed(Some(dac.id));
    let b = backend().ok_or_else(|| ExclusiveError::Refused("this app has no USB driver here".into()))?;
    let usb = ids(dac)
        .ok_or_else(|| ExclusiveError::Refused("Android did not say which USB device the DAC is".into()))?;
    if !b.has_permission(usb) {
        return Err(ExclusiveError::Refused(
            "Exclusive USB needs Android's permission for the DAC: allow it when Android asks (choose Exclusive USB again in Settings to ask)".into(),
        ));
    }
    let (t, raw) = b.open(usb).map_err(ExclusiveError::Refused)?;
    let dev = pm_usb_audio::parse(&raw);
    // Not one it drives, or gone while it was read: back to Android
    let let_go = |e: ExclusiveError| {
        hand_back(&*t, &dev);
        b.release();
        e
    };
    if (dev.vendor_id, dev.product_id) != (usb.vendor_id, usb.product_id) {
        return Err(let_go(ExclusiveError::Refused(
            "Android opened another USB device".into(),
        )));
    }
    let Some(uac @ (Uac::V1 | Uac::V2)) = dev.uac else {
        return Err(let_go(ExclusiveError::Refused(
            "not a USB Audio Class 1 or 2 DAC".into(),
        )));
    };
    let Some(ac) = dev.control_interface else {
        return Err(let_go(ExclusiveError::Refused(
            "the DAC has no audio control interface".into(),
        )));
    };
    let quirks = quirks::lookup(dev.vendor_id, dev.product_id);
    let read = || -> Result<(Vec<u32>, Option<(u8, u8, VolumeRange)>, Option<i16>), UsbError> {
        let mut clock_rates = Vec::new();
        if uac == Uac::V2 {
            if let Some(c) = dev.clocks.first() {
                clock_rates = driver::clock_rates(&*t, c.id, ac)?;
            }
            if clock_rates.is_empty() {
                clock_rates = dac.device.rates.clone();
            }
        }
        let (mut unit, mut before) = (None, None);
        if let (Uac::V2, Some(hv), false) = (uac, dev.hardware_volume.as_ref(), quirks.no_hardware_volume) {
            if hv.volume {
                if let Some(r) = driver::volume_range(&*t, hv.unit_id, ac)? {
                    unit = Some((hv.unit_id, ac, r));
                    before = driver::get_volume(&*t, hv.unit_id, ac)?;
                }
            }
        }
        Ok((clock_rates, unit, before))
    };
    let (clock_rates, unit, before) = match read() {
        Ok(x) => x,
        Err(UsbError::Gone) => {
            return Err(let_go(ExclusiveError::Gone(format!(
                "{} was unplugged",
                dac.device.name
            ))))
        }
        Err(UsbError::Other(e)) => return Err(let_go(ExclusiveError::Failed(e))),
    };
    let speed = t
        .speed()
        .unwrap_or(if uac == Uac::V2 { Speed::High } else { Speed::Full });
    tracing::info!(dac = %dac.device.name, ?uac, ?speed, rates = ?clock_rates, volume = ?unit, "Exclusive USB: the app holds the DAC");
    with_held(|h| {
        *h = Some(Held {
            dac_id: dac.id,
            dac_name: dac.device.name.clone(),
            t,
            dev,
            clock_rates,
            speed,
            quirks,
            unit,
            before,
            level: None,
            muted: false,
            driver: None,
            in_use: false,
            release_pending: false,
        })
    });
    // What the output takes is now exactly known: the node learns it
    usb_dac::notify_change();
    Ok(())
}

/// Open the exclusive driver for `fmt` on `dac`, with `warning` to keep (full level).
pub fn open(
    id: OutputId,
    dac: &Dac,
    fmt: AudioFormat,
    warning: Option<String>,
) -> Result<ExclusiveSink, ExclusiveError> {
    if fmt.kind != SampleKind::Pcm {
        return Err(ExclusiveError::Unsupported(
            "Exclusive USB plays PCM only (DSD is to come)".into(),
        ));
    }
    let opened_for = usb_dac::OpenedFor {
        dac_id: dac.id,
        name: dac.device.name.clone(),
        gone_at: usb_dac::gone_count(),
    };
    let generation = usb_dac::generation();
    acquire(dac)?;
    let media = crate::bit_perfect::backend().and_then(|b| b.media_volume());
    // After the open the feature unit is known for sure: a DAC without one at full level only
    // as the settings allow
    let settings = usb_dac::settings();
    if let Err(why) = volume_rule(settings.mode, has_unit(dac), media, usb_dac::full_level_consent()) {
        release();
        return Err(ExclusiveError::Failed(why));
    }
    let taken = with_held(|h| {
        let h = h.as_mut().filter(|h| h.dac_id == dac.id)?;
        let setting =
            pm_usb_audio::stream::choose(&h.dev, &h.clock_rates, fmt.rate_hz, fmt.bits, fmt.channels);
        let plan = setting.map(|setting| Plan {
            uac: h.dev.uac.unwrap_or(Uac::V2),
            control_interface: h.dev.control_interface.unwrap_or(0),
            setting,
            rate: fmt.rate_hz,
            speed: h.speed,
            quirks: h.quirks,
        });
        // The level before the first sample: the hand-over once, then the listener's
        if let Some((unit, ac, range)) = h.unit {
            let level = *h
                .level
                .get_or_insert_with(|| vol::handover_cur(media, h.before, range));
            let mut r = driver::set_volume(&*h.t, unit, ac, level);
            // Muted by the listener: it stays muted
            if r.is_ok() && h.muted {
                r = driver::set_mute(&*h.t, unit, ac, true);
            }
            if r == Err(UsbError::Gone) {
                return Some(Err(ExclusiveError::Gone(format!("{} was unplugged", h.dac_name))));
            }
        }
        let plan = match plan {
            Ok(p) => p,
            Err(why) => return Some(Err(ExclusiveError::Unsupported(why))),
        };
        h.in_use = true;
        Some(Ok((h.t.clone(), h.driver.take(), plan)))
    });
    let (t, kept, plan) = match taken {
        Some(Ok(x)) => x,
        Some(Err(e)) => return Err(e),
        None => return Err(ExclusiveError::Gone(format!("{} was unplugged", dac.device.name))),
    };
    let started = match kept {
        Some(mut d) => d.retune(plan.rate, plan.setting.clone()).map(|_| d),
        None => Driver::open(t, plan.clone()),
    };
    let driver = match started {
        Ok(d) => d,
        Err(e) => {
            with_held(|h| {
                if let Some(h) = h.as_mut() {
                    h.in_use = false;
                }
            });
            return Err(match e {
                DriverError::Gone => {
                    dac_changed(None);
                    ExclusiveError::Gone(format!("{} was unplugged", dac.device.name))
                }
                other => ExclusiveError::Failed(format!("Exclusive USB could not start: {other}")),
            });
        }
    };
    tracing::info!(format = %fmt, readback = ?driver.readback(), "Exclusive USB");
    usb_dac::record_open(LastOpen {
        dac_id: dac.id,
        format: fmt,
        mode: DeviceMode::Exclusive,
        confirmed: false,
        note: warning,
    });
    Ok(ExclusiveSink {
        id,
        fmt,
        subslot: plan.setting.subslot_bytes,
        driver: Some(driver),
        dac: dac.device.name.clone(),
        dac_id: dac.id,
        confirmed: false,
        _in_use: usb_dac::DacUse::new(Some(&opened_for)),
        opened_for,
        flushed: false,
        generation,
        written: 0,
        bytes: Vec::new(),
    })
}

/// Rule (c) for Exclusive USB: whether the held DAC may (go on) play(ing) now. With a feature unit
/// the hand-over keeps it never louder: always. Without one it plays at full level:
/// [`volume_rule`] on the live settings, Android's volume now and the consent for this DAC.
pub fn entry_gate(dac_id: i32) -> Result<Option<String>, String> {
    let unit = with_held(|h| {
        h.as_ref()
            .filter(|h| h.dac_id == dac_id)
            .map(|h| h.unit.is_some())
    });
    if unit != Some(false) {
        return Ok(None);
    }
    let media = crate::bit_perfect::backend().and_then(|b| b.media_volume());
    volume_rule(
        usb_dac::settings().mode,
        false,
        media,
        usb_dac::full_level_consent(),
    )
}

/// Set the zone's volume on the held DAC's feature unit.
pub fn set_volume(v: Volume) -> Result<(), String> {
    with_held(|h| {
        let h = h.as_mut().ok_or("no DAC is held")?;
        let (unit, ac, range) = h.unit.ok_or("the DAC has no volume of its own")?;
        let now = h.level.unwrap_or(range.min);
        let r = match v {
            Volume::Mute(m) => {
                h.muted = m;
                return driver::set_mute(&*h.t, unit, ac, m).map_err(|e| format!("{e:?}"));
            }
            Volume::Db(db) => vol::db_to_cur(db, range),
            Volume::Percent(p) => {
                let (lo, hi) = (vol::cur_to_db(range.min), vol::cur_to_db(range.max));
                vol::db_to_cur(lo + (hi - lo) * f32::from(p.min(100)) / 100.0, range)
            }
            Volume::Up => vol::db_to_cur(vol::cur_to_db(now) + 2.0, range),
            Volume::Down => vol::db_to_cur(vol::cur_to_db(now) - 2.0, range),
        };
        h.level = Some(r);
        h.muted = false;
        driver::set_volume(&*h.t, unit, ac, r).map_err(|e| format!("{e:?}"))
    })
}

/// The zone's sink on the exclusive driver.
pub struct ExclusiveSink {
    id: OutputId,
    fmt: AudioFormat,
    subslot: u8,
    driver: Option<Driver>,
    dac: String,
    dac_id: i32,
    confirmed: bool,
    /// The DAC it was opened for (rule (b), [`usb_dac::may_reach`]).
    opened_for: usb_dac::OpenedFor,
    _in_use: usb_dac::DacUse,
    /// Flushed (a pause, a seek) and not written since: the next write starts it again.
    flushed: bool,
    generation: u64,
    written: u64,
    bytes: Vec<u8>,
}

impl ExclusiveSink {
    /// Rule (b): its DAC still the phone's (held), none gone since; else Gone (the zone knows).
    fn gone(&self) -> Option<SinkError> {
        let why = usb_dac::may_reach(Some(&self.opened_for)).err()?;
        usb_dac::gone_seen();
        Some(SinkError::Gone(self.id.clone(), why))
    }

    fn map(&self, e: DriverError) -> SinkError {
        match e {
            DriverError::Gone => {
                dac_changed(None);
                usb_dac::gone_seen();
                SinkError::Gone(self.id.clone(), format!("{} was unplugged", self.dac))
            }
            other => SinkError::Device(other.to_string()),
        }
    }

    fn driver(&mut self) -> Result<&mut Driver, SinkError> {
        if let Some(e) = self.gone() {
            return Err(e);
        }
        let id = self.id.clone();
        let dac = self.dac.clone();
        self.driver
            .as_mut()
            .ok_or_else(|| SinkError::Gone(id, format!("{dac} was unplugged")))
    }

    /// Once the clock read back the rate and the stream runs: confirmed, and the node learns it.
    fn check_confirmed(&mut self) {
        if self.confirmed || !self.driver.as_ref().is_some_and(|d| d.confirmed()) {
            return;
        }
        self.confirmed = true;
        usb_dac::confirm_open(self.dac_id, self.fmt);
    }
}

impl AudioSink for ExclusiveSink {
    fn format(&self) -> AudioFormat {
        self.fmt
    }

    fn write(&mut self, block: &AudioBlock) -> Result<(), SinkError> {
        pm_zones::sink::check_block(self.fmt, block)?;
        let Samples::PcmI32(samples) = &block.samples else {
            return Err(SinkError::FloatNotAccepted);
        };
        let mut bytes = std::mem::take(&mut self.bytes);
        bytes.clear();
        pm_usb_audio::stream::pack(self.fmt.bits, self.subslot, samples, &mut bytes);
        let r = self.driver()?.write(&bytes);
        self.bytes = bytes;
        match r {
            Ok(()) => {
                self.flushed = false;
                self.written += block.frames() as u64;
                self.check_confirmed();
                Ok(())
            }
            Err(e) => Err(self.map(e)),
        }
    }

    fn buffer_level(&self) -> BufferLevel {
        match self.driver.as_ref() {
            Some(d) => BufferLevel {
                queued_frames: d.queued_frames(),
                capacity_frames: d.capacity_frames(),
            },
            None => BufferLevel {
                queued_frames: 0,
                capacity_frames: 1,
            },
        }
    }

    fn flush(&mut self) -> Result<(), SinkError> {
        let r = self.driver()?.flush();
        self.flushed = true;
        r.map_err(|e| self.map(e))?;
        // Emptied, and its DAC gone: said (a song boundary pauses)
        match self.gone() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn drain(&mut self) -> Result<(), SinkError> {
        let r = self.driver()?.drain();
        r.map_err(|e| self.map(e))
    }

    fn frames_written(&self) -> u64 {
        self.written
    }

    /// The settings or the DAC changed since it opened, or full level is no longer allowed (a
    /// DAC without a volume of its own, Android's volume down since): the next song opens afresh.
    fn reopen_at_track(&self) -> bool {
        usb_dac::generation() != self.generation || entry_gate(self.dac_id).is_err()
    }

    /// Flushed (a pause), and full level no longer allowed: whoever writes opens afresh first.
    fn reopen_now(&self) -> bool {
        self.flushed && entry_gate(self.dac_id).is_err()
    }
}

impl Drop for ExclusiveSink {
    /// Between songs the DAC stays held, idle; given back if that was asked meanwhile.
    fn drop(&mut self) {
        let mut d = self.driver.take();
        if let Some(mut x) = d.take() {
            // Gone or broken: nothing to keep (its owner gives the interfaces back)
            match x.idle() {
                Ok(()) => d = Some(x),
                Err(_) => x.stop(),
            }
        }
        let give = with_held(|h| match h.as_mut() {
            Some(x) if x.dac_id == self.dac_id => {
                x.in_use = false;
                x.driver = d.take();
                if x.release_pending {
                    h.take()
                } else {
                    None
                }
            }
            _ => None,
        });
        if let Some(mut d) = d {
            d.stop();
        }
        if let Some(h) = give {
            give_back(h);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use pm_usb_audio::driver::fake::FakeDac;

    /// The FiiO KA17's descriptors (pm-usb-audio's fixture).
    pub(crate) const KA17: &[u8] = include_bytes!("../../pm-usb-audio/tests/fixtures/fiio-ka17.bin");

    /// The app's side: a fake DAC behind it, permission as set.
    pub(crate) struct FakeApp {
        pub dac: FakeDac,
        pub permission: Mutex<bool>,
        pub asked: Mutex<u32>,
        pub released: Mutex<u32>,
        pub opened: Mutex<u32>,
    }

    impl FakeApp {
        pub(crate) fn new(permission: bool) -> Arc<Self> {
            let dac = FakeDac::high_speed();
            {
                let mut s = dac.state();
                // The KA17's clock: 44.1 to 768 kHz; its unit: -127..0 dB in 0.5 dB, at 0 now
                let rates = [
                    44_100u32, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000, 705_600, 768_000,
                ];
                let mut b = (rates.len() as u16).to_le_bytes().to_vec();
                for r in rates {
                    b.extend(r.to_le_bytes());
                    b.extend(r.to_le_bytes());
                    b.extend(0u32.to_le_bytes());
                }
                s.rate_ranges = Some(b);
                let mut v = 1u16.to_le_bytes().to_vec();
                v.extend((-127i16 * 256).to_le_bytes());
                v.extend(0i16.to_le_bytes());
                v.extend(128u16.to_le_bytes());
                s.volume_range = Some(v);
                s.volume = Some(0);
                s.feedback = Some(((12u32) << 16).to_le_bytes().to_vec());
            }
            Arc::new(FakeApp {
                dac,
                permission: Mutex::new(permission),
                asked: Mutex::new(0),
                released: Mutex::new(0),
                opened: Mutex::new(0),
            })
        }
    }

    impl UsbBackend for FakeApp {
        fn has_permission(&self, usb: UsbIds) -> bool {
            assert_eq!((usb.vendor_id, usb.product_id), (0x2972, 0x0093), "the KA17");
            *self.permission.lock().unwrap()
        }
        fn request_permission(&self, _usb: UsbIds) {
            *self.asked.lock().unwrap() += 1;
        }
        fn open(&self, _usb: UsbIds) -> Result<(Arc<dyn Transport>, Vec<u8>), String> {
            *self.opened.lock().unwrap() += 1;
            Ok((Arc::new(self.dac.clone()), KA17.to_vec()))
        }
        fn release(&self) {
            *self.released.lock().unwrap() += 1;
        }
    }

    #[test]
    fn auto_takes_it_only_where_bit_perfect_is_not_offered_and_never_asks() {
        assert!(chosen(PreferredMode::Exclusive, true, false));
        assert!(chosen(PreferredMode::Auto, false, true));
        assert!(!chosen(PreferredMode::Auto, false, false), "Auto never asks");
        assert!(!chosen(PreferredMode::Auto, true, true), "bit-perfect first");
        assert!(!chosen(PreferredMode::BitPerfect, false, true));
        assert!(!chosen(PreferredMode::Shared, false, true));
    }

    #[test]
    fn without_a_volume_of_its_own_it_is_never_louder() {
        // A feature unit: the hand-over sets it, any media volume
        assert_eq!(volume_rule(PreferredMode::Auto, true, Some(30), false), Ok(None));
        // None: Auto only at full volume
        assert_eq!(
            volume_rule(PreferredMode::Auto, false, Some(100), false),
            Ok(None)
        );
        assert!(
            volume_rule(PreferredMode::Auto, false, Some(99), true).is_err(),
            "Auto never takes the consent"
        );
        assert!(volume_rule(PreferredMode::Auto, false, None, false).is_err());
        // Exclusive USB below full: only with the listener's agreement, and it says so
        let e = volume_rule(PreferredMode::Exclusive, false, Some(40), false).unwrap_err();
        assert!(e.contains("confirm Exclusive USB in Settings"), "{e}");
        let w = volume_rule(PreferredMode::Exclusive, false, Some(40), true)
            .unwrap()
            .unwrap();
        assert!(w.contains("(40%)") && w.contains("full level"), "{w}");
    }

    #[test]
    fn the_apps_answer() {
        assert_eq!(
            parse_open_answer(r#"{"ok":true,"fd":42,"descriptors":"12010002"}"#),
            Ok((42, vec![0x12, 0x01, 0x00, 0x02]))
        );
        assert_eq!(
            parse_open_answer(r#"{"ok":false,"why":"another app holds the DAC"}"#),
            Err("another app holds the DAC".into())
        );
        assert!(parse_open_answer(r#"{"ok":true,"fd":-1,"descriptors":""}"#).is_err());
        assert!(parse_open_answer(r#"{"ok":true,"fd":3,"descriptors":"zz"}"#).is_err());
        assert!(parse_open_answer("not json").is_err());
    }

    /// UsbExclusive.kt writes the keys read here: its open answer ([`parse_open_answer`]) and the
    /// DAC it holds (UsbAudio's report, `held`).
    #[test]
    fn the_kotlin_side_writes_the_keys_read_here() {
        let kt = include_str!("../../../tools/release/android/UsbExclusive.kt");
        let written: std::collections::BTreeSet<&str> = kt
            .match_indices(".put(\"")
            .filter_map(|(at, m)| kt[at + m.len()..].split('"').next())
            .collect();
        let read: std::collections::BTreeSet<&str> =
            ["ok", "why", "fd", "descriptors", "vendor_id", "product_id"]
                .into_iter()
                .collect();
        assert_eq!(written, read);
        // The calls the JNI side makes, with the signatures it uses
        for f in [
            "fun hasPermission(vendor: Int, product: Int): Boolean",
            "fun requestPermission(vendor: Int, product: Int)",
            "fun open(vendor: Int, product: Int): String",
            "fun release()",
        ] {
            assert!(kt.contains(f), "{f}");
        }
        assert_eq!(kt.matches("@JvmStatic").count(), 4);
    }

    #[test]
    fn the_ka17s_rates_and_sizes_listed_together() {
        let dev = pm_usb_audio::parse(KA17);
        let rates = [44_100u32, 48_000, 96_000, 192_000, 768_000];
        let f: Vec<DeviceFormat> = pm_usb_audio::stream::formats(&dev, &rates)
            .into_iter()
            .map(|(rate_hz, bits, channels)| DeviceFormat {
                rate_hz,
                bits,
                channels,
            })
            .collect();
        // 16 and 24 in the 32-bit setting too: every rate with every size
        assert_eq!(rectangle(&f), (rates.to_vec(), vec![16, 24, 32]));
        // 96/24 is listed as itself: the node sends it unchanged
        assert!(f.contains(&DeviceFormat {
            rate_hz: 96_000,
            bits: 24,
            channels: 2
        }));
    }

    use crate::bit_perfect::{open_phone, PhoneOpen, Track, TrackBackend};
    use crate::usb_dac::MixerEntry;
    use crate::PHONE_TESTS as SERIAL;
    use pm_zones::DeviceSettings;

    /// Android's media volume, for the hand-over (no bit-perfect player: Android 13).
    struct Media(Mutex<Option<u8>>);

    impl TrackBackend for Media {
        fn open(
            &self,
            _: i32,
            _: &MixerEntry,
        ) -> Result<(Box<dyn Track>, crate::bit_perfect::OpenReport), crate::bit_perfect::OpenError> {
            Err(crate::bit_perfect::OpenError::Refused(
                "no bit-perfect player in this test".into(),
            ))
        }
        fn media_volume(&self) -> Option<u8> {
            *self.0.lock().unwrap()
        }
    }

    /// The phone's process-wide state back to nothing when a test ends (even a failed one).
    struct Phone;

    impl Drop for Phone {
        fn drop(&mut self) {
            with_held(|h| *h = None);
            *BACKEND.write().unwrap_or_else(|e| e.into_inner()) = None;
            usb_dac::set_dac(None);
            usb_dac::set_settings(DeviceSettings::default());
        }
    }

    /// What UsbAudio.kt reports for the KA17 on `sdk`, its descriptors included (permission given).
    fn ka17_report(sdk: u32, rest: &str) -> String {
        let hex: String = KA17.iter().map(|b| format!("{b:02x}")).collect();
        format!(
            r#"{{"v":1,"sdk":{sdk},"devices":[{{"id":41,"type":11,"name":"FiiO KA17",
            "rates":[44100,48000,88200,96000,176400,192000,352800,384000,705600,768000],
            "encodings":[2,21,22],"channel_counts":[2],"routed":true,
            "usb":{{"vendor_id":10610,"product_id":147,"product":"FiiO KA17","descriptors":"{hex}"}}}}]{rest}}}"#
        )
    }

    /// A KA17 on an Android 13 phone at media volume `media`, the app's USB side with `permission`.
    fn phone(permission: bool, media: Option<u8>) -> (Phone, Arc<FakeApp>) {
        let guard = Phone;
        with_held(|h| *h = None);
        usb_dac::set_settings(DeviceSettings::default());
        usb_dac::set_dac(None);
        let app = FakeApp::new(permission);
        set_backend(app.clone());
        crate::bit_perfect::set_backend(Arc::new(Media(Mutex::new(media))));
        assert!(usb_dac::apply_report(&ka17_report(33, "")));
        (guard, app)
    }

    fn exclusive(mode: PreferredMode) {
        usb_dac::set_settings(DeviceSettings {
            mode,
            ..DeviceSettings::default()
        });
    }

    fn id() -> OutputId {
        OutputId::new("cpal", "phone")
    }

    fn open_ok(fmt: AudioFormat) -> ExclusiveSink {
        match open_phone(id(), fmt) {
            PhoneOpen::Exclusive(s) => s,
            PhoneOpen::Shared { .. } => panic!("shared: {:?}", usb_dac::current().and_then(|d| d.note)),
            PhoneOpen::Gone(w) | PhoneOpen::Failed(w) | PhoneOpen::Unsupported(w) => panic!("{w}"),
            PhoneOpen::BitPerfect(_) => panic!("bit-perfect"),
        }
    }

    #[test]
    fn exclusive_usb_plays_the_ka17_unchanged_and_says_so_once_confirmed() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(60));
        exclusive(PreferredMode::Exclusive);
        assert_eq!(*app.asked.lock().unwrap(), 0, "permission was there: not asked");
        let d = usb_dac::current().unwrap();
        assert_eq!(
            d.modes,
            [DeviceMode::Shared, DeviceMode::Exclusive],
            "Android 13: no bit-perfect"
        );
        // The node plans 96/24 as itself
        let caps = crate::bit_perfect::phone_caps(&shared());
        assert_eq!(caps.quality_ceiling, QualityCeiling::BitPerfect);
        assert!(caps.pcm_rates.contains(&96_000) && caps.pcm_rates.contains(&768_000));
        assert_eq!(caps.pcm_bits, [16, 24, 32]);

        let fmt = AudioFormat::pcm(96_000, 24, 2);
        let mut sink = open_ok(fmt);
        let log = app.dac.state().log.clone();
        // 60 % on Android's curve is -17 dB, less 3: the DAC starts at -20 dB, before the stream
        let level = log
            .iter()
            .position(|l| l == "volume -5120")
            .unwrap_or_else(|| panic!("{log:?}"));
        let stream = log
            .iter()
            .position(|l| l == "alt 1 1")
            .unwrap_or_else(|| panic!("{log:?}"));
        assert!(level < stream, "{log:?}");
        assert!(log.contains(&"set rate 96000".to_string()) && log.contains(&"get rate".to_string()));
        // Not confirmed before the stream runs
        let d = usb_dac::current().unwrap();
        assert_eq!(
            (d.mode, d.confirmed, d.format),
            (DeviceMode::Exclusive, false, Some(fmt))
        );

        let block = AudioBlock::pcm_i32(fmt, vec![0x12_3456, -1].repeat(9_600));
        sink.write(&block).unwrap();
        let d = usb_dac::current().unwrap();
        assert_eq!(
            (d.mode, d.confirmed),
            (DeviceMode::Exclusive, true),
            "read back and running"
        );
        // 24 bit in the 32-bit setting, left-justified: values unchanged
        let first = app.dac.state().out[0].data[..8].to_vec();
        assert_eq!(first, [0x00, 0x56, 0x34, 0x12, 0x00, 0xff, 0xff, 0xff]);
        assert_eq!(sink.frames_written(), 9_600);

        // The zone's volume is the DAC's: a dB range, and the unit follows
        assert_eq!(
            caps_now().hw_volume,
            Some(VolumeKind::Db {
                min: -127.0,
                max: 0.0,
                step: 0.5
            })
        );
        set_volume(Volume::Db(-30.0)).unwrap();
        assert_eq!(app.dac.state().volume, Some(-30 * 256));

        // The next song at 44.1/16: the same DAC, still held, the clock set and read back again
        drop(sink);
        app.dac.state().log.clear();
        let fmt2 = AudioFormat::pcm(44_100, 16, 2);
        let mut sink = open_ok(fmt2);
        let log = app.dac.state().log.clone();
        assert!(
            !log.iter()
                .any(|l| l.starts_with("release") || l.starts_with("claim")),
            "{log:?}"
        );
        assert!(
            log.ends_with(&[
                "alt 1 0".into(),
                "set rate 44100".into(),
                "get rate".into(),
                "alt 1 2".into()
            ]),
            "{log:?}"
        );
        assert!(
            log.contains(&"volume -7680".to_string()),
            "the listener's level stays: {log:?}"
        );
        sink.write(&AudioBlock::pcm_i32(fmt2, vec![0x1234, -2].repeat(4_410)))
            .unwrap();
        assert!(usb_dac::current().unwrap().confirmed);
        assert_eq!(*app.opened.lock().unwrap(), 1, "the DAC opened once");
    }

    /// Android's mixer: 48 kHz, 16 bit, lossless at best.
    fn shared() -> OutputCaps {
        OutputCaps {
            pcm_rates: vec![48_000],
            max_pcm_bits: 16,
            quality_ceiling: QualityCeiling::LosslessUpTo {
                rate_hz: 48_000,
                bits: 16,
            },
            ..OutputCaps::unlimited()
        }
    }

    fn caps_now() -> OutputCaps {
        crate::bit_perfect::phone_caps(&shared())
    }

    #[test]
    fn while_held_the_dac_stays_the_phones_and_an_unplug_pauses() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(100));
        exclusive(PreferredMode::Exclusive);
        let fmt = AudioFormat::pcm(192_000, 24, 2);
        let mut sink = open_ok(fmt);
        let block = AudioBlock::pcm_i32(fmt, vec![0; 2 * 19_200]);
        sink.write(&block).unwrap();
        // Android no longer lists it (its driver let go), the app says it holds it: still the DAC
        let gone = usb_dac::gone_count();
        usb_dac::apply_report(r#"{"v":1,"sdk":33,"devices":[],"held":{"vendor_id":10610,"product_id":147}}"#);
        // Taking it from Android's driver warns "becoming noisy" too: not an unplug while held
        usb_dac::apply_report(
            r#"{"v":1,"sdk":33,"devices":[],"noisy":true,"held":{"vendor_id":10610,"product_id":147}}"#,
        );
        assert_eq!(usb_dac::gone_count(), gone);
        assert_eq!(usb_dac::current().unwrap().name, "FiiO KA17");
        sink.write(&block).unwrap();
        // Unplugged: the next write is Gone (the zone pauses), and the app lets go of it
        app.dac.state().gone = true;
        let e = sink.write(&block).unwrap_err();
        assert!(matches!(e, SinkError::Gone(..)), "{e}");
        assert_eq!(e.gone_reason().as_deref(), Some("FiiO KA17 was unplugged"));
        assert!(!holding());
        assert_eq!(*app.released.lock().unwrap(), 1);
        drop(sink);
        // The app's report without it: gone, as ever
        usb_dac::apply_report(r#"{"v":1,"sdk":33,"devices":[]}"#);
        assert!(usb_dac::current().is_none());
    }

    #[test]
    fn another_mode_gives_the_dac_back_no_louder_than_it_was() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(100));
        app.dac.state().volume = Some(-6 * 256);
        exclusive(PreferredMode::Exclusive);
        let fmt = AudioFormat::pcm(48_000, 16, 2);
        let mut sink = open_ok(fmt);
        // Before: -6 dB on the unit at Android's full volume; the hand-over is -9 dB
        assert_eq!(app.dac.state().volume, Some(-9 * 256));
        set_volume(Volume::Db(0.0)).unwrap();
        // Shared chosen mid-song: the DAC goes back when the song ends, not under it
        exclusive(PreferredMode::Shared);
        assert!(holding());
        sink.write(&AudioBlock::pcm_i32(fmt, vec![0; 2 * 4_800])).unwrap();
        app.dac.state().log.clear();
        drop(sink);
        assert!(!holding());
        assert_eq!(*app.released.lock().unwrap(), 1);
        let log = app.dac.state().log.clone();
        // The unit no louder than before (-6 dB, not the listener's 0 dB), the streaming interface
        // back first, the control interface last
        assert!(log.contains(&"volume -1536".to_string()), "{log:?}");
        assert!(!log.contains(&"volume 0".to_string()), "{log:?}");
        // Until Android lists the DAC again (the app's report still says it held it), nothing
        // opens anywhere: the zone pauses rather than reach the speaker
        usb_dac::apply_report(r#"{"v":1,"sdk":33,"devices":[],"held":{"vendor_id":10610,"product_id":147}}"#);
        match open_phone(id(), fmt) {
            PhoneOpen::Gone(why) => assert!(why.contains("going back to Android"), "{why}"),
            _ => panic!("nothing may open before Android has the DAC again"),
        }
        // Listed again: Android's way (Android 13: shared)
        usb_dac::apply_report(&ka17_report(33, ""));
        assert!(matches!(open_phone(id(), fmt), PhoneOpen::Shared { .. }));
        assert_eq!(*app.opened.lock().unwrap(), 1, "not taken again");
        assert!(
            log.ends_with(&["release 1".into(), "release 0".into()]),
            "{log:?}"
        );
    }

    #[test]
    fn permission_is_asked_only_when_exclusive_usb_is_chosen() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(false, Some(100));
        // Auto never asks, and plays Android's way
        exclusive(PreferredMode::Auto);
        assert_eq!(*app.asked.lock().unwrap(), 0);
        assert!(matches!(
            open_phone(id(), AudioFormat::pcm(48_000, 16, 2)),
            PhoneOpen::Shared { .. }
        ));
        assert_eq!(*app.opened.lock().unwrap(), 0);
        // Chosen: asked once; until it is given, Android plays it and says why
        exclusive(PreferredMode::Exclusive);
        assert_eq!(*app.asked.lock().unwrap(), 1);
        assert!(matches!(
            open_phone(id(), AudioFormat::pcm(48_000, 16, 2)),
            PhoneOpen::Shared { .. }
        ));
        let note = usb_dac::current().unwrap().note.unwrap_or_default();
        assert!(note.contains("needs Android's permission"), "{note}");
        // Given: exclusive
        *app.permission.lock().unwrap() = true;
        drop(open_ok(AudioFormat::pcm(48_000, 16, 2)));
        // Auto on Android 13, with the permission given before: exclusive too, never asking again
        exclusive(PreferredMode::Auto);
        drop(open_ok(AudioFormat::pcm(48_000, 16, 2)));
        assert_eq!(*app.asked.lock().unwrap(), 1);
    }

    #[test]
    fn a_clock_that_wont_take_the_rate_plays_nothing_and_says_why() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(100));
        exclusive(PreferredMode::Exclusive);
        app.dac.state().readback = Some(48_000);
        match open_phone(id(), AudioFormat::pcm(96_000, 24, 2)) {
            PhoneOpen::Failed(why) => assert_eq!(
                why,
                "Exclusive USB could not start: the DAC's clock runs at 48000 Hz, not 96000 Hz"
            ),
            _ => panic!("nothing may play"),
        }
        let d = usb_dac::current().unwrap();
        assert_eq!((d.mode, d.confirmed), (DeviceMode::Exclusive, false));
        assert!(app.dac.state().out.is_empty(), "not a sample sent");
    }
}
