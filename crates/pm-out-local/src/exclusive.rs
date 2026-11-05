//! Exclusive USB: the app drives the phone's USB DAC itself, with its own driver
//! ([`pm_usb_audio::driver`]) on the USB device Android hands over (`UsbExclusive.kt`), for phones
//! below Android 14 or where Android refuses bit-perfect.
//!
//! * The listener chooses it (Settings > This phone, "Exclusive USB"): the app asks Android's
//!   permission for the DAC then, and only then ([`settings_changed`]). Auto uses it by itself
//!   only where bit-perfect isn't offered or was refused, and only once that permission exists:
//!   it never asks. Once Auto holds the DAC it keeps it while the zone plays.
//! * The DAC is held from the first song until the mode changes, the DAC goes or the app closes
//!   ([`release`]): between songs it stays claimed, so Android's driver doesn't come and go. A
//!   paused zone keeps no sink on it ([`ExclusiveSink`]'s pause), so closing the app gives it back.
//!   After a crash, [`recover`] gives back what was left without Android's driver.
//! * Never louder: Android's media volume is read before the DAC is taken (while Android still
//!   plays it), and the DAC's feature unit on the playback path starts at what that and the unit's
//!   own level and mute amounted to ([`pm_usb_audio::volume::handover`]), muted when that is below
//!   what it can do. Nothing plays until the DAC read that back ([`driver::apply_level`]). It goes
//!   back no louder than it was. A DAC without a unit plays at full level: as with bit-perfect,
//!   Auto uses it only at Android's full volume, Exclusive USB below that only with the listener's
//!   agreement ([`volume_rule`]).
//! * Confirmed only when the playback path's clock read back exactly the track's rate and its
//!   packets arrive; withdrawn when they stop arriving.
//! * Nothing reaches another device: after the app gave a DAC back (or an attempt to take it
//!   failed), Android's paths wait until Android lists it again ([`wait_android_back`]), else the
//!   zone pauses.
//!
//! Everything here but the JNI backend runs on any host, against a fake DAC.

use crate::usb_dac::{self, Dac, LastOpen};
use pm_model::{
    AudioBlock, AudioFormat, OutputCaps, QualityCeiling, SampleKind, Samples, Volume, VolumeKind,
};
use pm_usb_audio::driver::{self, Driver, DriverError, Plan, UsbError};
use pm_usb_audio::quirks::{self, Quirks};
use pm_usb_audio::schedule::Speed;
use pm_usb_audio::volume::{self as vol, Level, VolumeRange};
use pm_usb_audio::{Uac, UsbAudioDevice};
use pm_zones::{AudioSink, BufferLevel, DeviceFormat, DeviceMode, OutputId, PreferredMode, SinkError};
use std::sync::{Arc, Mutex, RwLock};

pub use pm_usb_audio::driver::Transport;
pub use pm_zones::UsbIds;

/// An opened DAC: the transport on (a duplicate of) its file descriptor, and its raw descriptors.
pub type Opened = (Arc<dyn Transport>, Vec<u8>);

/// Why the app would not open the DAC; `partial`: it opened it but could not take every audio
/// interface (another app holds one): the ones taken are given back to Android's driver.
pub struct OpenRefused {
    pub why: String,
    pub partial: Option<Opened>,
}

/// The app's side (`UsbExclusive.kt`, over JNI): Android's permission and the open device, the
/// DAC found by its USB ids.
pub trait UsbBackend: Send + Sync {
    /// The app may open the DAC (Android's permission was given). Never asks.
    fn has_permission(&self, usb: UsbIds) -> bool;
    /// Ask for Android's permission (its dialog): only when the listener chose Exclusive USB.
    fn request_permission(&self, usb: UsbIds);
    /// Open the DAC with its audio interfaces taken from Android's driver.
    fn open(&self, usb: UsbIds) -> Result<Opened, OpenRefused>;
    /// Close the connection (the driver gave the interfaces back first).
    fn release(&self);
    /// The claim this app recorded (it was killed holding a DAC: Android's driver never got it
    /// back), whether it holds one now, and every attached USB device by name. Facts only: what
    /// is given back is decided here ([`recovery_target`]).
    fn claims(&self) -> Claims {
        Claims::default()
    }
    /// Open exactly the device Android names so (its path), with its audio interfaces taken.
    fn open_detached(&self, device_name: &str) -> Result<Opened, OpenRefused> {
        Err(OpenRefused {
            why: format!("{device_name} can't be opened by name here"),
            partial: None,
        })
    }
    /// Forget the recorded claim (its device is gone, or is another one now).
    fn forget_claim(&self) {}
}

/// A USB device by Android's name for it (its path under /dev/bus/usb), which tells it from an
/// identical DAC, and its ids.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct UsbDeviceRef {
    pub device_name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    /// It has an audio interface.
    #[serde(default)]
    pub audio: bool,
}

/// `UsbExclusive.detached`'s answer ([`UsbBackend::claims`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct Claims {
    /// The device this app claimed and never released, as recorded then.
    #[serde(default)]
    pub recorded: Option<UsbDeviceRef>,
    /// The app holds a DAC now.
    #[serde(default)]
    pub held: bool,
    /// Every USB device attached now.
    #[serde(default)]
    pub attached: Vec<UsbDeviceRef>,
}

/// The device recovery gives back to Android's driver: exactly the one recorded, attached under
/// the same name with the same ids, an audio device, while the app holds none. Never another
/// device, an identical DAC under another name included (that one is Android's, playing).
pub fn recovery_target(c: &Claims) -> Option<&UsbDeviceRef> {
    if c.held {
        return None;
    }
    let rec = c.recorded.as_ref()?;
    c.attached.iter().find(|d| {
        d.device_name == rec.device_name
            && (d.vendor_id, d.product_id) == (rec.vendor_id, rec.product_id)
            && d.audio
    })
}

/// `UsbExclusive.open`'s answer: the file descriptor and the raw descriptors, or why not (with
/// them when it opened the DAC but could not take every interface).
#[allow(clippy::type_complexity)]
pub fn parse_open_answer(json: &str) -> Result<(i32, Vec<u8>), (String, Option<(i32, Vec<u8>)>)> {
    #[derive(serde::Deserialize)]
    struct Answer {
        ok: bool,
        #[serde(default)]
        why: Option<String>,
        #[serde(default = "no_fd")]
        fd: i32,
        #[serde(default)]
        descriptors: String,
    }
    fn no_fd() -> i32 {
        -1
    }
    let a: Answer =
        serde_json::from_str(json).map_err(|e| (format!("the USB driver's answer: {e}"), None))?;
    // A descriptor handed over is ours whatever else the answer says: always returned (to be
    // given back and closed), with no descriptors when they can't be read (every interface then)
    let raw = usb_dac::unhex(&a.descriptors);
    let readable = raw.is_some();
    let opened = (a.fd >= 0).then(|| (a.fd, raw.unwrap_or_default()));
    if !a.ok {
        let why = a.why.unwrap_or_else(|| "Android would not open the DAC".into());
        return Err((why, opened));
    }
    match opened {
        None => Err(("Android gave no file descriptor for the DAC".into(), None)),
        Some(o) if !readable => Err(("the DAC's descriptors could not be read".into(), Some(o))),
        Some(o) => Ok(o),
    }
}

/// `UsbExclusive.detached`'s answer ([`Claims`]); nothing when it can't be read.
pub fn parse_claims(json: &str) -> Claims {
    serde_json::from_str(json).unwrap_or_default()
}

/// The kernel's usbfs on the file descriptor the app handed over: a duplicate it made under its
/// own lock and gave up, so it is ours (closed with the transport) and no unplug can close it
/// before this takes it.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn usbfs(fd: i32) -> Arc<dyn Transport> {
    Arc::new(pm_usb_audio::usbfs::Usbfs::take(fd))
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

/// The feature unit the driver uses: on the playback path, with its range and whether it mutes.
#[derive(Clone)]
struct Unit {
    id: u8,
    ac: u8,
    range: VolumeRange,
    can_mute: bool,
}

/// A feature unit the driver may use, with its level and mute before the take and now.
#[derive(Clone)]
struct UnitState {
    unit: Unit,
    before: Option<Level>,
    level: Option<Level>,
    /// The driver set (or tried to set) it: given back at the end.
    touched: bool,
    /// The listener set its level (on this path): its own level from then on, not the hand-over.
    chosen: bool,
}

/// The DAC while the app holds it.
struct Held {
    dac_id: i32,
    dac_name: String,
    t: Arc<dyn Transport>,
    dev: UsbAudioDevice,
    /// Each playback clock's rates (its `RANGE`).
    clock_rates: Vec<(u8, Vec<u32>)>,
    speed: Speed,
    quirks: Quirks,
    /// Every playback path's feature unit (each path its own; never a microphone's).
    units: Vec<UnitState>,
    /// The unit of the setting playing (or last played): the zone's volume moves it.
    active: Option<u8>,
    /// A setting was opened since the take (`active` then says its path's unit, or none).
    opened: bool,
    /// The listener muted it (the zone's mute), not the hand-over: every path plays muted, and it
    /// goes back muted.
    listener_muted: bool,
    /// The level the listener chose (dB), on whichever path played then: every path plays at it
    /// (muted when it is below what that path's unit goes), and it goes back no louder.
    listener_db: Option<f32>,
    /// Android's own attenuation for the DAC (dB), read before the DAC was taken, while Android
    /// still played it (`None`: not known: the hand-over mutes).
    media_db_at_take: Option<f32>,
    /// The driver, between songs and while paused (a sink has it while it plays).
    driver: Option<Driver>,
    /// A sink plays on it now.
    in_use: bool,
    /// Give it back once the sink closes.
    release_pending: bool,
}

impl Held {
    fn state(&mut self, id: u8) -> Option<&mut UnitState> {
        self.units.iter_mut().find(|u| u.unit.id == id)
    }

    /// The unit the zone's volume moves: the playing setting's, else the first playback path's.
    /// None once a path without a unit of its own played: the zone's volume can't move it.
    fn shown(&self) -> Option<&UnitState> {
        match self.active {
            Some(a) => self.units.iter().find(|u| u.unit.id == a),
            None if self.opened => None,
            None => self.units.first(),
        }
    }
}

static HELD: Mutex<Option<Held>> = Mutex::new(None);

fn with_held<T>(f: impl FnOnce(&mut Option<Held>) -> T) -> T {
    f(&mut HELD.lock().unwrap_or_else(|e| e.into_inner()))
}

/// The DAC the app gave back (or tried to take), and Android's listing count then: Android's
/// paths wait until it lists the DAC again ([`wait_android_back`]).
static GIVEN_BACK: Mutex<Option<(String, u64)>> = Mutex::new(None);

fn mark_given_back(name: &str) {
    *GIVEN_BACK.lock().unwrap_or_else(|e| e.into_inner()) = Some((name.to_string(), usb_dac::listed_seq()));
}

/// How long a DAC the app gave back may take to reappear in Android's list.
const GIVE_BACK_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// Before anything opens on Android's paths: when the app gave a DAC back (or touched it in an
/// attempt that failed), wait until Android lists a DAC again; `Err` (why, for a pause) when it
/// doesn't within a few seconds. Nothing opens on the default route meanwhile.
pub fn wait_android_back() -> Result<bool, String> {
    let Some((name, seq)) = GIVEN_BACK.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
        return Ok(false);
    };
    let until = std::time::Instant::now() + GIVE_BACK_WAIT;
    while usb_dac::listed_seq() == seq {
        if std::time::Instant::now() >= until {
            return Err(format!(
                "{name} is going back to Android: press Play again in a moment"
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    *GIVEN_BACK.lock().unwrap_or_else(|e| e.into_inner()) = None;
    Ok(true)
}

/// Android's own attenuation for the DAC (dB) for the hand-over, read before anything touches the
/// DAC: `None` (the hand-over then mutes) when Android can't say it for the DAC's route, or while
/// a DAC the app gave back may not be Android's route yet (its reading may be the speaker's).
pub fn handover_reading() -> Option<f32> {
    {
        let mut given = GIVEN_BACK.lock().unwrap_or_else(|e| e.into_inner());
        match given.as_ref() {
            // Android listed it again since: its route is its own again
            Some((_, seq)) if *seq != usb_dac::listed_seq() => *given = None,
            Some(_) => return None,
            None => {}
        }
    }
    crate::bit_perfect::backend().and_then(|b| b.media_db_on_dac())
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
            None => "Android's volume for the DAC is not known (media plays elsewhere, or Android can't say), and this DAC has no volume of its own".into(),
        }),
    }
}

/// Android's media volume as it applies to the DAC (rule (c) for a path that plays at full
/// level): it counts only together with a reading taken while the DAC was Android's media route
/// (`media_db`, Android's own attenuation there: [`handover_reading`] before the take, the take's
/// while the app holds it), and full only when that reading is full too (0 dB). Any other reading
/// (another route's volume, Bluetooth or the speaker, a stale or unknown one) is unknown: full
/// level then needs the listener's consent, Auto doesn't play it.
pub fn media_on_dac(media: Option<u8>, media_db: Option<f32>) -> Option<u8> {
    let db = media_db.filter(|d| d.is_finite())?;
    match media {
        Some(100) if db >= -0.01 => Some(100),
        Some(100) => None,
        other => other,
    }
}

/// Android's own attenuation for the DAC that counts for [`media_on_dac`]: the take's while the
/// app holds it (Android's route is elsewhere then), else now's (only on the DAC's own route).
fn dac_reading(dac_id: i32) -> Option<f32> {
    let held = with_held(|h| {
        h.as_ref()
            .filter(|h| h.dac_id == dac_id)
            .map(|h| h.media_db_at_take)
    });
    match held {
        Some(at_take) => at_take,
        None => handover_reading(),
    }
}

/// Rule (c) for a playback path without a volume of its own (it plays at full level): Android at
/// full volume for the DAC itself ([`media_on_dac`]), else the listener's consent ([`volume_rule`]);
/// never while the listener muted the zone, and with the consent only when they turned it down.
fn full_level_rule(
    mode: PreferredMode,
    media_on_dac: Option<u8>,
    consent: bool,
    listener_muted: bool,
    listener_db: Option<f32>,
) -> Result<Option<String>, String> {
    if listener_muted {
        return Err("The zone is muted, and this format plays on a path of the DAC without a volume of its own (at full level): unmute the zone to play it".into());
    }
    let turned_down = listener_db.is_some_and(|db| db < -0.01);
    let media = media_on_dac.filter(|p| !(turned_down && *p == 100));
    volume_rule(mode, false, media, consent)
}

/// The DAC's descriptors as the app reported them (with an earlier permission), parsed.
fn reported(dac: &Dac) -> Option<UsbAudioDevice> {
    dac.descriptors
        .as_deref()
        .map(pm_usb_audio::parse)
        .filter(|d| d.uac.is_some())
}

/// The bus speed assumed before the DAC is held (UAC2 DACs run at high speed).
fn assumed_speed(d: &UsbAudioDevice) -> Speed {
    if d.uac == Some(Uac::V2) {
        Speed::High
    } else {
        Speed::Full
    }
}

/// Whether the DAC has a feature unit the driver uses (on its playback path, UAC2, no quirk
/// against it), as far as is known now. The same unit a UI calls "its own volume".
fn has_unit(dac: &Dac) -> bool {
    if let Some(h) = with_held(|h| {
        h.as_ref()
            .filter(|h| h.dac_id == dac.id)
            .map(|h| h.shown().is_some())
    }) {
        return h;
    }
    let ids = dac.device.usb.map(|u| (u.vendor_id, u.product_id));
    let quirk = ids.is_some_and(|(v, p)| quirks::lookup(v, p).no_hardware_volume);
    !quirk && reported(dac).is_some_and(|d| d.exclusive_unit().is_some())
}

/// Every format the DAC plays unchanged in Exclusive USB: from the held DAC (its clock's ranges,
/// its bus speed) or, before, from its reported descriptors and the rates Android lists for it.
pub fn formats(dac: &Dac) -> Vec<DeviceFormat> {
    let held = with_held(|h| {
        h.as_ref()
            .filter(|h| h.dac_id == dac.id)
            .map(|h| pm_usb_audio::stream::formats(&h.dev, &h.clock_rates, h.speed))
    });
    let list = match held {
        Some(l) => l,
        None => match reported(dac) {
            Some(d) => {
                // Before the take: each playback clock with the rates Android lists for the DAC
                let clocks: Vec<(u8, Vec<u32>)> = d
                    .playback_clocks()
                    .into_iter()
                    .map(|c| (c, dac.device.rates.clone()))
                    .collect();
                pm_usb_audio::stream::formats(&d, &clocks, assumed_speed(&d))
            }
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
            .and_then(|h| h.shown())
            .map(|u| VolumeKind::Db {
                min: vol::cur_to_db(u.unit.range.min()),
                max: vol::cur_to_db(u.unit.range.max()),
                step: u.unit.range.step_db(),
            })
    })
}

/// Whether the phone plays this DAC exclusive now: `None` when it isn't chosen (or offered);
/// `Some(Ok(warning))` when it plays exclusive; `Some(Err(why))` when it was chosen but can't
/// (no permission yet, or louder than now). Auto keeps a DAC it holds (no giving back and taking
/// again at every format change).
pub fn decide(
    dac: &Dac,
    settings: pm_zones::DeviceSettings,
    full_level_ok: bool,
    media: Option<u8>,
) -> Option<Result<Option<String>, String>> {
    let b = backend()?;
    offer(dac).ok()?;
    let held = holds(dac.id);
    let permission = held || b.has_permission(ids(dac)?);
    let keep = settings.mode == PreferredMode::Auto && held;
    if !keep && !chosen(settings.mode, crate::bit_perfect::offer(dac).is_ok(), permission) {
        return None;
    }
    if !permission {
        return Some(Err(
            "Exclusive USB needs Android's permission for the DAC: allow it when Android asks (choose Exclusive USB again in Settings to ask)".into(),
        ));
    }
    let media = media_on_dac(media, dac_reading(dac.id));
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
    let media = media_on_dac(media, dac_reading(dac.id));
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

/// The listener set the settings (changed or not): choosing Exclusive USB asks Android's
/// permission for the DAC when it isn't given (the only place it is asked; choosing it again asks
/// again); a mode that doesn't use it gives the DAC back.
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

/// Give the DAC back to Android: now when no song plays on it (a paused zone keeps none), else
/// when that sink closes.
pub fn release() {
    let now = with_held(|h| match h.as_mut() {
        Some(x) if x.in_use => {
            x.release_pending = true;
            None
        }
        _ => h.take(),
    });
    if let Some(h) = now {
        give_back(h, true);
    }
}

/// The DAC went or another took its place: let go of it at once (its sink stops with Gone).
pub fn dac_changed(dac_id: Option<i32>) {
    let gone = with_held(|h| match h.as_ref() {
        Some(x) if Some(x.dac_id) != dac_id => h.take(),
        _ => None,
    });
    if let Some(h) = gone {
        give_back(h, false);
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

/// Back to Android: the unit no louder than before, the interfaces to Android's driver, the
/// connection closed. `attached`: the DAC is still there (Android's paths wait for it to be
/// listed again).
fn give_back(mut h: Held, attached: bool) {
    // Each unit the driver set goes back as it was before the take (also one whose hand-over
    // failed half-way), not the hand-over's attenuation: but no louder than the listener's own
    // level, and muted when the listener muted
    for st in h.units.iter().filter(|u| u.touched) {
        let Some(back) = st.before.or(st.level) else {
            continue;
        };
        let mut back = cap(vol::giveback(st.before, back), h.listener_db, &st.unit.range);
        back.mute |= h.listener_muted;
        // A unit that can't mute: at its quietest instead (its level is still given back)
        if back.mute && !st.unit.can_mute {
            back = Level {
                cur: st.unit.range.min(),
                mute: false,
            };
        }
        let _ = driver::apply_level(&*h.t, st.unit.id, st.unit.ac, back, st.unit.can_mute);
    }
    if let Some(mut d) = h.driver.take() {
        d.stop();
    }
    hand_back(&*h.t, &h.dev);
    tracing::info!(dac = %h.dac_name, "Exclusive USB: the DAC goes back to Android");
    if attached {
        mark_given_back(&h.dac_name);
    }
    if let Some(b) = backend() {
        b.release();
    }
}

/// The held DAC's stream failed for good: back to Android now (its sink stopped its driver), not
/// kept claimed with nothing playing.
fn give_back_failed(dac_id: i32) {
    let h = with_held(|h| match h.as_ref() {
        Some(x) if x.dac_id == dac_id => h.take(),
        _ => None,
    });
    if let Some(h) = h {
        give_back(h, true);
    }
}

/// After the app was killed while it held a DAC, Android's driver was never given it back: give
/// back every audio device the app may open that Android lists no audio device for. At start.
pub fn recover() {
    let Some(b) = backend() else { return };
    if holding() {
        return;
    }
    let claims = b.claims();
    if claims.held {
        return;
    }
    // Exactly the device recorded, by its name: never an identical DAC Android plays now
    let Some(usb) = recovery_target(&claims).cloned() else {
        // Unplugged since (a replug gives Android a fresh device), or another device now
        if claims.recorded.is_some() {
            b.forget_claim();
        }
        return;
    };
    let (t, raw) = match b.open_detached(&usb.device_name) {
        Ok(o) => o,
        Err(OpenRefused { partial: Some(o), .. }) => o,
        Err(_) => return,
    };
    let dev = pm_usb_audio::parse(&raw);
    hand_back(&*t, &dev);
    b.release();
    tracing::info!(
        ?usb,
        "Exclusive USB: a DAC left without Android's driver goes back to it"
    );
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

/// Take the DAC over (or keep it): its descriptors, every playback path's clock rates and feature
/// unit (its range, level and mute before), and `media_db`, Android's attenuation for the DAC
/// read before the DAC was taken.
fn acquire(dac: &Dac, media_db: Option<f32>) -> Result<(), ExclusiveError> {
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
    let (t, raw) = match b.open(usb) {
        Ok(o) => o,
        Err(OpenRefused { why, partial }) => {
            // Some interfaces were taken before one was refused: those go back too
            if let Some((t, raw)) = partial {
                hand_back(&*t, &pm_usb_audio::parse(&raw));
                b.release();
                mark_given_back(&dac.device.name);
            }
            return Err(ExclusiveError::Refused(why));
        }
    };
    let dev = pm_usb_audio::parse(&raw);
    // Not one it drives, or gone while it was read: back to Android
    let let_go = |e: ExclusiveError| {
        hand_back(&*t, &dev);
        b.release();
        if !matches!(e, ExclusiveError::Gone(_)) {
            mark_given_back(&dac.device.name);
        }
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
    type Read = (Vec<(u8, Vec<u32>)>, Vec<UnitState>);
    let read = || -> Result<Read, UsbError> {
        // Each playback path's clock, as its terminal names it
        let mut clock_rates = Vec::new();
        if uac == Uac::V2 {
            for c in dev.playback_clocks() {
                let mut r = driver::clock_rates(&*t, c, ac)?;
                if r.is_empty() {
                    r = dac.device.rates.clone();
                }
                clock_rates.push((c, r));
            }
        }
        // Each playback path's feature unit (never a capture path's), its range, level and mute
        let mut units: Vec<UnitState> = Vec::new();
        if uac == Uac::V2 && !quirks.no_hardware_volume {
            let ids = dev
                .streams
                .iter()
                .filter(|s| s.endpoint.as_ref().is_some_and(|e| e.address & 0x80 == 0))
                .filter_map(|s| dev.playback_unit(s.terminal_link));
            for hv in ids {
                if units.iter().any(|u| u.unit.id == hv.unit_id) {
                    continue;
                }
                let Some(range) = driver::volume_range(&*t, hv.unit_id, ac)? else {
                    continue;
                };
                let cur = driver::get_volume(&*t, hv.unit_id, ac)?;
                let mute = if hv.mute {
                    driver::get_mute(&*t, hv.unit_id, ac)?
                } else {
                    Some(false)
                };
                units.push(UnitState {
                    unit: Unit {
                        id: hv.unit_id,
                        ac,
                        range,
                        can_mute: hv.mute,
                    },
                    // Both known, or the level before is unknown (the hand-over then mutes)
                    before: cur.zip(mute).map(|(cur, mute)| Level { cur, mute }),
                    level: None,
                    touched: false,
                    chosen: false,
                });
            }
        }
        Ok((clock_rates, units))
    };
    let (clock_rates, units) = match read() {
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
    tracing::info!(dac = %dac.device.name, ?uac, ?speed, clocks = ?clock_rates, units = ?units.iter().map(|u| (u.unit.id, u.before)).collect::<Vec<_>>(), ?media_db, "Exclusive USB: the app holds the DAC");
    with_held(|h| {
        *h = Some(Held {
            dac_id: dac.id,
            dac_name: dac.device.name.clone(),
            t,
            dev,
            clock_rates,
            speed,
            quirks,
            units,
            active: None,
            opened: false,
            listener_muted: false,
            listener_db: None,
            media_db_at_take: media_db,
            driver: None,
            in_use: false,
            release_pending: false,
        })
    });
    // What the output takes is now exactly known: the node learns it
    usb_dac::notify_change();
    Ok(())
}

/// Open the exclusive driver for `fmt` on `dac`, with `warning` to keep (full level). `media`:
/// Android's media volume (percent, for the full-level rule) and `media_db` its attenuation for
/// the DAC ([`handover_reading`]), both read by the caller before anything touched the DAC.
pub fn open(
    id: OutputId,
    dac: &Dac,
    fmt: AudioFormat,
    warning: Option<String>,
    media: Option<u8>,
    media_db: Option<f32>,
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
    acquire(dac, media_db)?;
    // Read before the held DAC's lock (the phone's state is never asked under it)
    let settings = usb_dac::settings();
    let consent = usb_dac::full_level_consent();
    let mut note = warning;
    let taken = with_held(|h| {
        let h = h.as_mut().filter(|h| h.dac_id == dac.id)?;
        let setting = pm_usb_audio::stream::choose(
            &h.dev,
            &h.clock_rates,
            fmt.rate_hz,
            fmt.bits,
            fmt.channels,
            h.speed,
        );
        // The setting's own path decides the volume: its unit, or (none) the full-level rule
        let unit_id = setting
            .as_ref()
            .ok()
            .and_then(|s| s.unit.as_ref())
            .map(|u| u.unit_id)
            .filter(|id| h.units.iter().any(|u| u.unit.id == *id));
        if setting.is_ok() && unit_id.is_none() {
            let on_dac = media_on_dac(media, h.media_db_at_take);
            let rule = full_level_rule(settings.mode, on_dac, consent, h.listener_muted, h.listener_db);
            if let Err(why) = rule {
                return Some(Err(ExclusiveError::Failed(why)));
            }
        }
        let plan = match setting {
            Ok(setting) => Plan {
                uac: h.dev.uac.unwrap_or(Uac::V2),
                control_interface: h.dev.control_interface.unwrap_or(0),
                setting,
                rate: fmt.rate_hz,
                speed: h.speed,
                quirks: h.quirks,
            },
            Err(why) => return Some(Err(ExclusiveError::Unsupported(why))),
        };
        h.opened = true;
        // The level before the first sample: the hand-over once (from Android's volume read
        // before the DAC was taken), then the listener's; in place only once the DAC read it back
        h.active = unit_id;
        if let Some(uid) = unit_id {
            let (media_db, listener_db, listener_muted) =
                (h.media_db_at_take, h.listener_db, h.listener_muted);
            let t = h.t.clone();
            let st = h.state(uid).expect("filtered on a known unit");
            let u = st.unit.clone();
            // Whichever path plays: its own level (the hand-over, or the listener's on it), no
            // louder than the listener's level chosen on any path, muted with the listener's mute
            let own = st
                .level
                .filter(|_| st.chosen)
                .unwrap_or_else(|| vol::handover(media_db, st.before, &u.range));
            let level = path_level(own, listener_db, listener_muted, &u.range);
            st.level = Some(level);
            // Touched before it is set: given back as it was even if this fails half-way
            st.touched = true;
            match driver::apply_level(&*t, u.id, u.ac, level, u.can_mute) {
                Ok(()) => {}
                Err(UsbError::Gone) => {
                    return Some(Err(ExclusiveError::Gone(format!("{} was unplugged", h.dac_name))))
                }
                Err(UsbError::Other(why)) => {
                    st.level = None;
                    if level.mute && !u.can_mute {
                        return Some(Err(ExclusiveError::Failed(
                            "This format plays on a path of the DAC whose volume can't be muted, and it would have to start muted (the zone is muted, or quieter than this path goes): nothing plays".into(),
                        )));
                    }
                    return Some(Err(ExclusiveError::Failed(format!(
                        "Exclusive USB could not set the DAC's volume ({why}): nothing plays"
                    ))));
                }
            }
            if level.mute && !h.listener_muted && note.is_none() {
                note = Some(
                    "The DAC starts muted (Android's own level for it wasn't known, or is below what the DAC's volume goes): raise the zone's volume to hear it".into(),
                );
            }
        }
        h.in_use = true;
        Some(Ok((h.t.clone(), h.driver.take(), plan)))
    });
    let (t, kept, plan) = match taken {
        Some(Ok(x)) => x,
        Some(Err(ExclusiveError::Failed(why))) => {
            // Its volume can't be trusted: back to Android, nothing plays here
            release();
            return Err(ExclusiveError::Failed(why));
        }
        Some(Err(ExclusiveError::Unsupported(why))) => {
            // Not playable here (the clock won't take the rate): back to Android, not kept claimed
            release();
            return Err(ExclusiveError::Unsupported(why));
        }
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
                other => {
                    // Not started: back to Android (nothing plays here, and it keeps nothing)
                    release();
                    ExclusiveError::Failed(format!("Exclusive USB could not start: {other}"))
                }
            });
        }
    };
    tracing::info!(format = %fmt, readback = ?driver.readback(), "Exclusive USB");
    usb_dac::record_open(LastOpen {
        dac_id: dac.id,
        format: fmt,
        mode: DeviceMode::Exclusive,
        confirmed: false,
        note,
    });
    Ok(ExclusiveSink {
        id,
        fmt,
        subslot: plan.setting.subslot_bytes,
        plan,
        driver: Some(driver),
        dac: dac.device.name.clone(),
        dac_id: dac.id,
        confirmed: false,
        gate: usb_dac::DacGate::new(Some(opened_for)),
        flushed: false,
        generation,
        written: 0,
        bytes: Vec::new(),
    })
}

/// A playback path's level before its first sample: its `own` (the hand-over from Android's level
/// for it, or the level the listener set on it), never louder than the listener's level chosen
/// on any path (muted when this unit can't go that low), muted whenever the listener muted.
fn path_level(own: Level, listener_db: Option<f32>, listener_muted: bool, range: &VolumeRange) -> Level {
    let mut level = cap(own, listener_db, range);
    level.mute |= listener_muted;
    level
}

/// `level` no louder than the listener's `db`: muted, at the minimum, when the unit can't go that
/// low.
fn cap(mut level: Level, listener_db: Option<f32>, range: &VolumeRange) -> Level {
    if let Some(db) = listener_db {
        match range.at_or_below((f64::from(db) * 256.0).floor() as i32) {
            Some(cur) => level.cur = level.cur.min(cur),
            None => {
                level.cur = range.min();
                level.mute = true;
            }
        }
    }
    level
}

/// Rule (c) for Exclusive USB: whether the held DAC may (go on) play(ing) now. With a feature unit
/// the hand-over keeps it never louder: always. Without one it plays at full level:
/// [`volume_rule`] on the live settings, Android's volume now and the consent for this DAC.
pub fn entry_gate(dac_id: i32) -> Result<Option<String>, String> {
    // The playing setting's path: with a unit the hand-over keeps it never louder
    let held = with_held(|h| {
        h.as_ref()
            .filter(|h| h.dac_id == dac_id && h.active.is_none())
            .map(|h| (h.media_db_at_take, h.listener_muted, h.listener_db))
    });
    let Some((media_db, listener_muted, listener_db)) = held else {
        return Ok(None);
    };
    let media = media_on_dac(crate::bit_perfect::media_now(), media_db);
    full_level_rule(
        usb_dac::settings().mode,
        media,
        usb_dac::full_level_consent(),
        listener_muted,
        listener_db,
    )
}

/// Set the zone's volume on the held DAC's feature unit: the listener's level (a hand-over mute
/// ends with it), or the listener's mute.
pub fn set_volume(v: Volume) -> Result<(), String> {
    with_held(|h| {
        let h = h.as_mut().ok_or("no DAC is held")?;
        let uid = h
            .shown()
            .map(|u| u.unit.id)
            .ok_or("the DAC has no volume of its own")?;
        let listener_muted = h.listener_muted;
        let t = h.t.clone();
        let st = h.state(uid).expect("shown");
        let u = st.unit.clone();
        st.touched = true;
        st.chosen = true;
        let now = st.level.unwrap_or(Level {
            cur: u.range.min(),
            mute: true,
        });
        let err = |e: UsbError| format!("{e:?}");
        let cur = match v {
            Volume::Mute(m) => {
                if !u.can_mute {
                    return Err("the DAC can't be muted".into());
                }
                driver::set_mute(&*t, u.id, u.ac, m).map_err(err)?;
                st.level = Some(Level { mute: m, ..now });
                h.listener_muted = m;
                // Heard at this level now (a hand-over mute ended): every path plays at it
                if !m && h.listener_db.is_none() {
                    h.listener_db = Some(vol::cur_to_db(now.cur));
                }
                return Ok(());
            }
            Volume::Db(db) => vol::db_to_cur(db, &u.range),
            Volume::Percent(p) => {
                let (lo, hi) = (vol::cur_to_db(u.range.min()), vol::cur_to_db(u.range.max()));
                vol::db_to_cur(lo + (hi - lo) * f32::from(p.min(100)) / 100.0, &u.range)
            }
            Volume::Up => vol::db_to_cur(vol::cur_to_db(now.cur) + 2.0, &u.range),
            Volume::Down => vol::db_to_cur(vol::cur_to_db(now.cur) - 2.0, &u.range),
        };
        driver::set_volume(&*t, u.id, u.ac, cur).map_err(err)?;
        // A level the listener chose ends a hand-over mute; their own mute stays
        let mute = now.mute && listener_muted;
        if now.mute && !mute {
            driver::set_mute(&*t, u.id, u.ac, false).map_err(err)?;
        }
        st.level = Some(Level { cur, mute });
        // The listener's level, for every path (below this unit's minimum: as asked)
        h.listener_db = Some(match v {
            Volume::Db(db) => db.min(vol::cur_to_db(cur)),
            _ => vol::cur_to_db(cur),
        });
        Ok(())
    })
}

/// The zone's sink on the exclusive driver.
pub struct ExclusiveSink {
    id: OutputId,
    fmt: AudioFormat,
    subslot: u8,
    /// What it plays with (to start again after a pause).
    plan: Plan,
    /// The driver while it plays; with the held DAC while paused.
    driver: Option<Driver>,
    dac: String,
    dac_id: i32,
    confirmed: bool,
    /// The DAC it was opened for (rule (b), [`usb_dac::may_reach`]).
    gate: usb_dac::DacGate,
    /// Flushed (a pause, a seek) and not written since: the next write starts it again.
    flushed: bool,
    generation: u64,
    written: u64,
    bytes: Vec<u8>,
}

impl ExclusiveSink {
    /// Rule (b): its DAC still the phone's (held), none gone since; else Gone (kept for the next
    /// open until the zone heard it).
    fn gone(&self) -> Option<SinkError> {
        self.gate.gone(&self.id)
    }

    fn map(&self, e: DriverError) -> SinkError {
        match e {
            DriverError::Gone => {
                dac_changed(None);
                SinkError::Gone(self.id.clone(), format!("{} was unplugged", self.dac))
            }
            other => SinkError::Device(other.to_string()),
        }
    }

    /// The stream failed for good (the clock won't take the rate again, the DAC stopped taking
    /// packets, ...): no longer confirmed (said first); the stream stops and the DAC goes back to
    /// Android at once, so nothing is left claimed with nothing playing. Unplugged: let go.
    fn fail(&mut self, e: DriverError) -> SinkError {
        self.withdraw("the stream to the DAC failed: not confirmed");
        if matches!(e, DriverError::Gone) {
            return self.map(e);
        }
        if let Some(mut d) = self.driver.take() {
            d.stop();
        }
        give_back_failed(self.dac_id);
        tracing::warn!(dac = %self.dac, "Exclusive USB failed: {e}");
        SinkError::Device(e.to_string())
    }

    /// Its driver; after a pause, taken back from the held DAC and started again (Gone when the
    /// app gave the DAC back meanwhile).
    fn driver(&mut self) -> Result<&mut Driver, SinkError> {
        if let Some(e) = self.gone() {
            return Err(e);
        }
        if self.driver.is_none() {
            let dac_id = self.dac_id;
            let kept = with_held(|h| match h.as_mut() {
                Some(x) if x.dac_id == dac_id && !x.release_pending => {
                    x.in_use = true;
                    x.driver.take()
                }
                _ => None,
            });
            let Some(mut d) = kept else {
                return Err(SinkError::Gone(
                    self.id.clone(),
                    format!("{} went back to Android", self.dac),
                ));
            };
            // Started again: nothing confirmed until the clock and its packets say so again
            self.withdraw("started again after a pause: not confirmed yet");
            if let Err(e) = d.retune(self.plan.rate, self.plan.setting.clone()) {
                d.stop();
                return Err(self.fail(e));
            }
            self.driver = Some(d);
        }
        Ok(self.driver.as_mut().expect("just set"))
    }

    /// No longer confirmed (published to the node at once), with why.
    fn withdraw(&mut self, why: &str) {
        if self.confirmed {
            self.confirmed = false;
            usb_dac::unconfirm_exclusive(self.dac_id, self.fmt, why.into());
        }
    }

    /// Confirmed once the clock read back the rate and its packets arrive; withdrawn when they
    /// stop arriving. The node learns either.
    fn check_confirmed(&mut self) {
        let now = self.driver.as_ref().is_some_and(|d| d.confirmed());
        if now && !self.confirmed {
            self.confirmed = true;
            usb_dac::confirm_open(self.dac_id, self.fmt);
        } else if !now && self.confirmed {
            self.confirmed = false;
            usb_dac::unconfirm_exclusive(
                self.dac_id,
                self.fmt,
                "the DAC is not taking every packet: not confirmed".into(),
            );
        }
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
            // The stream failed: what was confirmed isn't any more (said before the error goes
            // on), and the DAC goes back to Android
            Err(e) => Err(self.fail(e)),
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
        self.flushed = true;
        if self.driver.is_some() {
            // A flush (a seek) that fails: not confirmed, said; the DAC stays held (the next write
            // says whether the stream failed for good)
            let r = self.driver()?.flush();
            r.map_err(|e| {
                self.withdraw("the stream to the DAC failed: not confirmed");
                self.map(e)
            })?;
        }
        // Emptied, and its DAC gone: said (a song boundary pauses)
        match self.gone() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn drain(&mut self) -> Result<(), SinkError> {
        if self.driver.is_none() {
            return match self.gone() {
                Some(e) => Err(e),
                None => Ok(()),
            };
        }
        let r = self.driver()?.drain();
        r.map_err(|e| self.fail(e))?;
        // Packets that failed while it drained (fewer than stop it) withdraw the confirmation
        self.check_confirmed();
        Ok(())
    }

    /// Pause: the stream stops and its driver rests with the held DAC, so no sink holds it while
    /// paused (closing the app gives it back). The next write starts it again.
    fn pause(&mut self) -> Result<bool, SinkError> {
        self.flush()?;
        // Nothing plays: confirmed again once it does
        self.withdraw("paused: confirmed again once it plays");
        if let Some(mut d) = self.driver.take() {
            if let Err(e) = d.idle() {
                d.stop();
                return Err(self.fail(e));
            }
            let dac_id = self.dac_id;
            let give = with_held(|h| match h.as_mut() {
                Some(x) if x.dac_id == dac_id => {
                    x.in_use = false;
                    x.driver = Some(d);
                    if x.release_pending {
                        h.take()
                    } else {
                        None
                    }
                }
                _ => None,
            });
            if let Some(h) = give {
                give_back(h, true);
            }
        }
        Ok(false)
    }

    fn frames_written(&self) -> u64 {
        self.written
    }

    /// The zone heard this sink's Gone: the loss need not be kept for the next open.
    fn gone_reported(&mut self) {
        self.gate.reported();
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
        // Nothing plays on it any more
        self.withdraw("closed: nothing plays");
        let mut d = self.driver.take();
        if let Some(mut x) = d.take() {
            // Gone or broken: nothing to keep; broken, the DAC goes back to Android at once
            match x.idle() {
                Ok(()) => d = Some(x),
                Err(DriverError::Gone) => {
                    x.stop();
                    dac_changed(None);
                }
                Err(_) => {
                    x.stop();
                    give_back_failed(self.dac_id);
                }
            }
        }
        let dac_id = self.dac_id;
        let give = with_held(|h| match h.as_mut() {
            Some(x) if x.dac_id == dac_id => {
                x.in_use = false;
                if d.is_some() {
                    x.driver = d.take();
                }
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
            give_back(h, true);
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
        /// Android's media volume (shared with the test's bit-perfect backend)
        pub media: Arc<Mutex<Option<u8>>>,
        /// Android's own attenuation for the DAC's route, in dB (`None`: it can't say)
        pub media_db: Arc<Mutex<Option<f32>>>,
        /// Taking the DAC moves Android's route: its dB reading then is this (the speaker's)
        pub media_db_on_open: Mutex<Option<f32>>,
        /// Reading Android's volume finds the DAC unplugged (it goes while it is read)
        pub unplug_on_read: Arc<std::sync::atomic::AtomicBool>,
        /// The descriptors the app reads from the device (the KA17's unless set)
        pub raw: Mutex<Option<Vec<u8>>>,
        /// Taking the DAC moves Android's route: its volume then reads this (the speaker's)
        pub media_on_open: Mutex<Option<u8>>,
        /// The app opens another USB device than asked
        pub other_device: Mutex<bool>,
        /// The app refuses (why), having taken some interfaces (true) or none
        pub refuse: Mutex<Option<(String, bool)>>,
        /// What the app says for recovery: its recorded claim, held, the attached devices
        pub claims: Mutex<Claims>,
        /// Attached devices by name (opened by name: recovery), each its own fake DAC
        pub named: Mutex<Vec<(String, FakeDac)>>,
        /// The devices opened by name (recovery)
        pub opened_names: Mutex<Vec<String>>,
        /// Times the recorded claim was forgotten
        pub forgot: Mutex<u32>,
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
                s.muted = Some(false);
                s.feedback = Some(((12u32) << 16).to_le_bytes().to_vec());
            }
            Arc::new(FakeApp {
                dac,
                permission: Mutex::new(permission),
                asked: Mutex::new(0),
                released: Mutex::new(0),
                opened: Mutex::new(0),
                media: Arc::new(Mutex::new(Some(100))),
                media_db: Arc::new(Mutex::new(None)),
                media_db_on_open: Mutex::new(None),
                unplug_on_read: Arc::default(),
                raw: Mutex::new(None),
                media_on_open: Mutex::new(None),
                other_device: Mutex::new(false),
                refuse: Mutex::new(None),
                claims: Mutex::new(Claims::default()),
                named: Mutex::new(Vec::new()),
                opened_names: Mutex::new(Vec::new()),
                forgot: Mutex::new(0),
            })
        }
    }

    impl UsbBackend for FakeApp {
        fn has_permission(&self, usb: UsbIds) -> bool {
            assert_eq!((usb.vendor_id, usb.product_id), (0x2972, 0x0093), "the KA17");
            *self.permission.lock().unwrap_or_else(|e| e.into_inner())
        }
        fn request_permission(&self, _usb: UsbIds) {
            *self.asked.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        }
        fn open(&self, _usb: UsbIds) -> Result<Opened, OpenRefused> {
            *self.opened.lock().unwrap_or_else(|e| e.into_inner()) += 1;
            self.the_device()
        }
        fn open_detached(&self, device_name: &str) -> Result<Opened, OpenRefused> {
            self.opened_names
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(device_name.to_string());
            // Exactly the device with that name (as Android's deviceList[name])
            let named = self.named.lock().unwrap_or_else(|e| e.into_inner());
            match named.iter().find(|(n, _)| n == device_name) {
                Some((_, dac)) => Ok((Arc::new(dac.clone()), KA17.to_vec())),
                None => Err(OpenRefused {
                    why: format!("{device_name} is not attached"),
                    partial: None,
                }),
            }
        }
        fn claims(&self) -> Claims {
            self.claims.lock().unwrap_or_else(|e| e.into_inner()).clone()
        }
        fn forget_claim(&self) {
            *self.forgot.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        }
        fn release(&self) {
            *self.released.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        }
    }

    impl FakeApp {
        /// The device as the app opens it (or refuses it).
        fn the_device(&self) -> Result<Opened, OpenRefused> {
            if let Some(v) = *self.media_on_open.lock().unwrap_or_else(|e| e.into_inner()) {
                *self.media.lock().unwrap_or_else(|e| e.into_inner()) = Some(v);
            }
            if let Some(v) = *self.media_db_on_open.lock().unwrap_or_else(|e| e.into_inner()) {
                *self.media_db.lock().unwrap_or_else(|e| e.into_inner()) = Some(v);
            }
            let custom = self.raw.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let raw = if let Some(r) = custom {
                r
            } else if *self.other_device.lock().unwrap_or_else(|e| e.into_inner()) {
                let mut r = KA17.to_vec();
                r[8] = 0x99; // another vendor
                r
            } else {
                KA17.to_vec()
            };
            let t: Arc<dyn Transport> = Arc::new(self.dac.clone());
            if let Some((why, partial)) = self.refuse.lock().unwrap_or_else(|e| e.into_inner()).clone() {
                return Err(OpenRefused {
                    why,
                    partial: partial.then_some((t, raw)),
                });
            }
            Ok((t, raw))
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
            Err(("another app holds the DAC".into(), None))
        );
        // Opened, but not every interface taken: the device comes with it, to be given back
        assert_eq!(
            parse_open_answer(
                r#"{"ok":false,"why":"another app holds the DAC","fd":7,"descriptors":"1201"}"#
            ),
            Err(("another app holds the DAC".into(), Some((7, vec![0x12, 0x01]))))
        );
        assert!(parse_open_answer(r#"{"ok":true,"fd":-1,"descriptors":""}"#).is_err());
        // Claude L3 (round 3 of #306): descriptors that can't be read still hand the descriptor
        // back (given back and closed, never leaked)
        assert_eq!(
            parse_open_answer(r#"{"ok":true,"fd":3,"descriptors":"zz"}"#),
            Err((
                "the DAC's descriptors could not be read".into(),
                Some((3, vec![]))
            ))
        );
        assert_eq!(
            parse_open_answer(r#"{"ok":false,"why":"no","fd":5,"descriptors":"z"}"#),
            Err(("no".into(), Some((5, vec![]))))
        );
        assert!(parse_open_answer("not json").is_err());
    }

    /// UsbExclusive.kt writes the keys read here: its open answer ([`parse_open_answer`]) and the
    /// DAC it holds (UsbAudio's report, `held`).
    #[test]
    fn the_detached_list() {
        let c = parse_claims(
            r#"{"recorded":{"device_name":"/dev/bus/usb/001/005","vendor_id":10610,"product_id":147},
            "held":false,"attached":[{"device_name":"/dev/bus/usb/001/005","vendor_id":10610,
            "product_id":147,"audio":true}]}"#,
        );
        let ka17 = UsbDeviceRef {
            device_name: "/dev/bus/usb/001/005".into(),
            vendor_id: 0x2972,
            product_id: 0x0093,
            audio: false,
        };
        assert_eq!(c.recorded, Some(ka17.clone()));
        assert_eq!(c.attached, [UsbDeviceRef { audio: true, ..ka17 }]);
        assert!(!c.held);
        assert_eq!(parse_claims("not json"), Claims::default());
        assert_eq!(parse_claims("{}"), Claims::default());
    }

    #[test]
    fn the_kotlin_side_writes_the_keys_read_here() {
        let kt = include_str!("../../../tools/release/android/UsbExclusive.kt");
        let written: std::collections::BTreeSet<&str> = kt
            .match_indices(".put(\"")
            .filter_map(|(at, m)| kt[at + m.len()..].split('"').next())
            .collect();
        let read: std::collections::BTreeSet<&str> = [
            "ok",
            "why",
            "fd",
            "descriptors",
            "vendor_id",
            "product_id",
            "device_name",
            "recorded",
            "held",
            "attached",
            "audio",
        ]
        .into_iter()
        .collect();
        assert_eq!(written, read);
        // The calls the JNI side makes, with the signatures it uses
        for f in [
            "fun hasPermission(vendor: Int, product: Int): Boolean",
            "fun requestPermission(vendor: Int, product: Int)",
            "fun open(vendor: Int, product: Int): String",
            "fun openDetached(name: String): String",
            "fun release()",
            "fun detached(): String",
            "fun forgetClaim()",
        ] {
            assert!(kt.contains(f), "{f}");
        }
        assert_eq!(kt.matches("@JvmStatic").count(), 7);
        // openDetached selects nothing itself: a lookup by the name the Rust side chose
        let od = kt
            .split("fun openDetached(name: String): String {")
            .nth(1)
            .and_then(|r| r.split("\n        }\n").next())
            .expect("openDetached");
        assert!(od.contains("um.deviceList[name]"), "{od}");
        assert!(!od.contains("find(") && !od.contains("vendorId"), "{od}");
    }

    #[test]
    fn the_ka17s_rates_and_sizes_listed_together() {
        let dev = pm_usb_audio::parse(KA17);
        let rates = [44_100u32, 48_000, 96_000, 192_000, 768_000];
        let f: Vec<DeviceFormat> =
            pm_usb_audio::stream::formats(&dev, &[(0x29, rates.to_vec())], Speed::High)
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

    /// Android's media volume, for the hand-over; its bit-perfect player refuses (Android 13, or
    /// refused on 14).
    struct Media {
        volume: Arc<Mutex<Option<u8>>>,
        db: Arc<Mutex<Option<f32>>>,
        unplug: Arc<std::sync::atomic::AtomicBool>,
    }

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
            if self.unplug.swap(false, std::sync::atomic::Ordering::SeqCst) {
                usb_dac::apply_report(r#"{"v":1,"sdk":33,"devices":[]}"#);
            }
            *self.volume.lock().unwrap_or_else(|e| e.into_inner())
        }
        fn media_db_on_dac(&self) -> Option<f32> {
            *self.db.lock().unwrap_or_else(|e| e.into_inner())
        }
    }

    /// The phone's process-wide state back to nothing when a test ends (even a failed one).
    struct Phone;

    impl Drop for Phone {
        fn drop(&mut self) {
            with_held(|h| *h = None);
            *GIVEN_BACK.lock().unwrap_or_else(|e| e.into_inner()) = None;
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
        *app.media.lock().unwrap() = media;
        set_backend(app.clone());
        crate::bit_perfect::set_backend(Arc::new(Media {
            volume: app.media.clone(),
            db: app.media_db.clone(),
            unplug: app.unplug_on_read.clone(),
        }));
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
            PhoneOpen::Exclusive(s) => *s,
            PhoneOpen::Shared { .. } => panic!("shared: {:?}", usb_dac::current().and_then(|d| d.note)),
            PhoneOpen::Gone(w) | PhoneOpen::Failed(w) | PhoneOpen::Unsupported(w) => panic!("{w}"),
            PhoneOpen::BitPerfect(_) => panic!("bit-perfect"),
        }
    }

    #[test]
    fn exclusive_usb_plays_the_ka17_unchanged_and_says_so_once_confirmed() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(60));
        // Android's own level for the DAC at 60 %: -20 dB
        *app.media_db.lock().unwrap() = Some(-20.0);
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
        // Android played the DAC at -20 dB: the DAC starts there, before the stream
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

        let block = AudioBlock::pcm_i32(fmt, [0x12_3456, -1].repeat(9_600));
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
        sink.write(&AudioBlock::pcm_i32(fmt2, [0x1234, -2].repeat(4_410)))
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
        *app.media_db.lock().unwrap() = Some(-3.0);
        app.dac.state().volume = Some(-6 * 256);
        exclusive(PreferredMode::Exclusive);
        let fmt = AudioFormat::pcm(48_000, 16, 2);
        let mut sink = open_ok(fmt);
        // Before: -6 dB on the unit, Android at -3 dB on top: the hand-over is -9 dB
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
        // Codex M6 (round 2 of #306): not started, so back to Android
        assert!(!holding());
        assert_eq!(*app.released.lock().unwrap(), 1);
        let log = app.dac.state().log.clone();
        assert!(
            log.ends_with(&["release 1".into(), "release 0".into()]),
            "{log:?}"
        );
    }

    fn volume_of(app: &FakeApp) -> (Option<i16>, Option<bool>) {
        let st = app.dac.state();
        (st.volume, st.muted)
    }

    /// Claude H1 / Codex H3 (round 1 of #306): Android's volume is read before the DAC is taken.
    /// Taken, Android's route moves (to the speaker, at 80 %), and its volume reads that: the
    /// hand-over still uses the DAC's 20 %.
    #[test]
    fn the_hand_over_uses_androids_volume_from_before_the_dac_was_taken() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(20));
        *app.media_db.lock().unwrap() = Some(-43.0);
        *app.media_on_open.lock().unwrap() = Some(80);
        *app.media_db_on_open.lock().unwrap() = Some(-11.5);
        exclusive(PreferredMode::Exclusive);
        drop(open_ok(AudioFormat::pcm(48_000, 16, 2)));
        // Android's level for the DAC before the take: -43 dB (the speaker's after: -11.5 dB)
        assert_eq!(volume_of(&app), (Some(-43 * 256), Some(false)));
    }

    /// Codex H1 (round 1 of #306): a level the DAC refuses or ignores plays nothing, and the DAC
    /// goes back to Android.
    #[test]
    fn a_volume_the_dac_wont_take_plays_nothing() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        for stuck in [false, true] {
            let (_p, app) = phone(true, Some(60));
            if stuck {
                app.dac.state().volume_stuck = Some(0);
            } else {
                app.dac.state().refuse_volume = true;
            }
            exclusive(PreferredMode::Exclusive);
            match open_phone(id(), AudioFormat::pcm(48_000, 16, 2)) {
                PhoneOpen::Failed(why) => assert!(why.contains("volume"), "{why}"),
                _ => panic!("stuck {stuck}: nothing may play"),
            }
            assert!(app.dac.state().out.is_empty(), "not a sample sent");
            assert_eq!(*app.released.lock().unwrap(), 1, "back to Android");
        }
    }

    /// Codex H2 / Claude M3 (round 1 of #306): Android quieter than the DAC's unit goes starts it
    /// muted (and says how to hear it); a DAC muted before stays muted, and goes back muted.
    #[test]
    fn quieter_than_the_dac_goes_starts_muted_and_a_mute_is_kept() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(20));
        *app.media_db.lock().unwrap() = Some(-43.0);
        {
            let mut s = app.dac.state();
            let mut v = 1u16.to_le_bytes().to_vec();
            v.extend((-20i16 * 256).to_le_bytes());
            v.extend(0i16.to_le_bytes());
            v.extend(256u16.to_le_bytes());
            s.volume_range = Some(v);
        }
        exclusive(PreferredMode::Exclusive);
        drop(open_ok(AudioFormat::pcm(48_000, 16, 2)));
        assert_eq!(
            volume_of(&app),
            (Some(-20 * 256), Some(true)),
            "below -20 dB: muted"
        );
        let note = usb_dac::current().unwrap().note.unwrap_or_default();
        assert!(note.contains("starts muted"), "{note}");
        // The listener raises the zone's volume: heard
        set_volume(Volume::Db(-10.0)).unwrap();
        assert_eq!(volume_of(&app), (Some(-10 * 256), Some(false)));
        drop(_p);

        // Muted while Android played it: muted, and given back muted
        let (_p, app) = phone(true, Some(100));
        *app.media_db.lock().unwrap() = Some(0.0);
        app.dac.state().muted = Some(true);
        exclusive(PreferredMode::Exclusive);
        drop(open_ok(AudioFormat::pcm(48_000, 16, 2)));
        assert_eq!(volume_of(&app).1, Some(true));
        exclusive(PreferredMode::Shared);
        assert_eq!(volume_of(&app).1, Some(true), "given back muted");
    }

    /// Codex H5 (round 1 of #306): an attempt to take the DAC that fails after touching it never
    /// lets Android's path open before Android lists the DAC again (the speaker meanwhile).
    #[test]
    fn a_failed_take_over_never_opens_the_speaker() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(100));
        *app.other_device.lock().unwrap() = true;
        exclusive(PreferredMode::Exclusive);
        match open_phone(id(), AudioFormat::pcm(48_000, 16, 2)) {
            PhoneOpen::Gone(why) => assert!(why.contains("going back to Android"), "{why}"),
            PhoneOpen::Shared { .. } => panic!("opened Android's path before it had the DAC back"),
            _ => panic!("nothing else"),
        }
        assert_eq!(*app.released.lock().unwrap(), 1);
        // Shared chosen, Android lists it again: its path opens, for the DAC listed now
        exclusive(PreferredMode::Shared);
        usb_dac::apply_report(&ka17_report(33, ""));
        assert!(matches!(
            open_phone(id(), AudioFormat::pcm(48_000, 16, 2)),
            PhoneOpen::Shared {
                opened_for: Some(_),
                ..
            }
        ));
    }

    /// Claude M5 (round 1 of #306): paused, no sink holds the DAC: closing the app gives it back
    /// at once (the next write is Gone); not closed, the next write starts it again.
    #[test]
    fn a_paused_zone_holds_no_sink_on_the_dac() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(100));
        exclusive(PreferredMode::Exclusive);
        let fmt = AudioFormat::pcm(96_000, 24, 2);
        let block = AudioBlock::pcm_i32(fmt, vec![0; 2 * 9_600]);
        let mut sink = open_ok(fmt);
        sink.write(&block).unwrap();
        assert!(!sink.pause().unwrap());
        // Played on after the pause: started again, on the same DAC, not opened again
        sink.write(&block).unwrap();
        assert_eq!(*app.opened.lock().unwrap(), 1);
        assert!(!sink.pause().unwrap());
        // The app closes while paused: back to Android at once
        release();
        assert_eq!(*app.released.lock().unwrap(), 1);
        assert!(!holding());
        let e = sink.write(&block).unwrap_err();
        assert!(matches!(e, SinkError::Gone(..)), "{e}");
    }

    /// Claude M5 / Codex M6 (round 1 of #306): a DAC left without Android's driver by a killed app
    /// goes back to it at the next start; one taken only in part goes back at once.
    #[test]
    fn what_a_killed_app_or_a_partial_take_left_goes_back() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(100));
        let ka17 = UsbDeviceRef {
            device_name: "/dev/bus/usb/001/004".into(),
            vendor_id: 0x2972,
            product_id: 0x0093,
            audio: true,
        };
        *app.claims.lock().unwrap() = Claims {
            recorded: Some(ka17.clone()),
            held: false,
            attached: vec![ka17],
        };
        app.named
            .lock()
            .unwrap()
            .push(("/dev/bus/usb/001/004".into(), app.dac.clone()));
        recover();
        // Codex M6 (round 3 of #306): exactly the device recorded, by its name, never one found
        // by its ids (an identical DAC Android plays now)
        assert_eq!(*app.opened_names.lock().unwrap(), ["/dev/bus/usb/001/004"]);
        assert_eq!(*app.opened.lock().unwrap(), 0, "never opened by its ids");
        let log = app.dac.state().log.clone();
        assert!(
            log.ends_with(&["release 1".into(), "release 0".into()]),
            "{log:?}"
        );
        assert_eq!(*app.released.lock().unwrap(), 1);
        // Another app holds one of its interfaces: the ones taken go back, Android plays it
        app.dac.state().log.clear();
        *app.refuse.lock().unwrap() = Some(("another app holds the DAC".into(), true));
        exclusive(PreferredMode::Exclusive);
        assert!(matches!(
            open_phone(id(), AudioFormat::pcm(48_000, 16, 2)),
            PhoneOpen::Gone(_) | PhoneOpen::Shared { .. }
        ));
        let log = app.dac.state().log.clone();
        assert!(
            log.ends_with(&["release 1".into(), "release 0".into()]),
            "{log:?}"
        );
        assert_eq!(*app.released.lock().unwrap(), 2);
    }

    /// Claude M7 (round 1 of #306): Auto on Android 14 with bit-perfect refused keeps the DAC it
    /// took, song after song (no giving back and taking again at each one).
    #[test]
    fn auto_keeps_the_dac_it_took() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(100));
        let mixer = r#","mixer":[{"encoding":2,"rate":48000,"channel_mask":12,"channels":2,"behavior":1},{"encoding":22,"rate":96000,"channel_mask":12,"channels":2,"behavior":1}]"#;
        let report = ka17_report(34, "").replace(r#""routed":true,"#, &format!(r#""routed":true{mixer},"#));
        usb_dac::apply_report(&report);
        assert!(crate::bit_perfect::offer(&usb_dac::current_dac().unwrap()).is_ok());
        exclusive(PreferredMode::Auto);
        drop(open_ok(AudioFormat::pcm(48_000, 16, 2)));
        drop(open_ok(AudioFormat::pcm(96_000, 24, 2)));
        drop(open_ok(AudioFormat::pcm(48_000, 16, 2)));
        assert_eq!(*app.opened.lock().unwrap(), 1, "taken once");
        assert_eq!(*app.released.lock().unwrap(), 0, "never given back meanwhile");
    }

    /// Codex M8 (round 1 of #306): once its packets stop arriving, a confirmed open is no longer
    /// confirmed (the node learns it).
    #[test]
    fn confirmation_is_withdrawn_when_packets_fail() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(100));
        exclusive(PreferredMode::Exclusive);
        let fmt = AudioFormat::pcm(96_000, 24, 2);
        let mut sink = open_ok(fmt);
        sink.write(&AudioBlock::pcm_i32(fmt, vec![0; 2 * 9_600])).unwrap();
        assert!(usb_dac::current().unwrap().confirmed);
        app.dac.state().packet_status = -71;
        let _ = sink.write(&AudioBlock::pcm_i32(fmt, vec![0; 2 * 384]));
        let d = usb_dac::current().unwrap();
        assert_eq!((d.mode, d.confirmed), (DeviceMode::Exclusive, false));
    }

    /// A UAC2 DAC (the KA17's ids) with a microphone path listed first, then two playback paths:
    /// interface 1, 16 bit, terminal 4 → unit 5 → output 6, clock 0x11; interface 2, 32 bit,
    /// terminal 7 → unit 0x0b → output 8, clock 0x12. The microphone: terminal 1 → unit 2 →
    /// USB streaming output 3, clock 0x10.
    fn two_path_dac() -> Vec<u8> {
        paths_dac(true, true)
    }

    /// [`two_path_dac`], each playback path with its unit (5 on the 16-bit one, 0x0b on the 32-bit
    /// one) or without one (its terminal straight to the output: full level).
    fn paths_dac(unit16: bool, unit32: bool) -> Vec<u8> {
        let mut b = vec![
            18, 1, 0x00, 0x02, 0xef, 2, 1, 64, 0x72, 0x29, 0x93, 0x00, 0x00, 0x01, 1, 2, 3, 1,
        ];
        let fu = |id: u8, src: u8| -> Vec<u8> {
            vec![18, 0x24, 6, id, src, 0x0f, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        };
        let it = |id: u8, ty: u16, clock: u8| -> Vec<u8> {
            let t = ty.to_le_bytes();
            vec![17, 0x24, 2, id, t[0], t[1], 0, clock, 2, 0, 0, 0, 0, 0, 0, 0, 0]
        };
        let ot = |id: u8, ty: u16, src: u8, clock: u8| -> Vec<u8> {
            let t = ty.to_le_bytes();
            vec![12, 0x24, 3, id, t[0], t[1], 0, src, clock, 0, 0, 0]
        };
        let mut ac: Vec<u8> = Vec::new();
        for c in [0x10u8, 0x11, 0x12] {
            ac.extend([8, 0x24, 0x0a, c, 0x03, 0x07, 0, 0]);
        }
        ac.extend(it(1, 0x0201, 0x10));
        ac.extend(fu(2, 1));
        ac.extend(ot(3, 0x0101, 2, 0x10));
        ac.extend(it(4, 0x0101, 0x11));
        if unit16 {
            ac.extend(fu(5, 4));
        }
        ac.extend(ot(6, 0x0302, if unit16 { 5 } else { 4 }, 0x11));
        ac.extend(it(7, 0x0101, 0x12));
        if unit32 {
            ac.extend(fu(0x0b, 7));
        }
        ac.extend(ot(8, 0x0301, if unit32 { 0x0b } else { 7 }, 0x12));
        let ac_len = (9 + ac.len()) as u16;
        let mut cfg: Vec<u8> = vec![9, 4, 0, 0, 0, 1, 1, 0x20, 0, 9, 0x24, 1, 0x00, 0x02, 4];
        cfg.extend(ac_len.to_le_bytes());
        cfg.push(0);
        cfg.extend(ac);
        for (iface, link, subslot, bits) in [(1u8, 4u8, 2u8, 16u8), (2, 7, 4, 32)] {
            cfg.extend([9, 4, iface, 0, 0, 1, 2, 0x20, 0]);
            cfg.extend([9, 4, iface, 1, 1, 1, 2, 0x20, 0]);
            cfg.extend([16, 0x24, 1, link, 0, 1, 1, 0, 0, 0, 2, 3, 0, 0, 0, 0]);
            cfg.extend([6, 0x24, 2, 1, subslot, bits]);
            // Isochronous OUT, adaptive, 1024 bytes a microframe
            cfg.extend([7, 5, iface, 0x09, 0x00, 0x04, 1]);
            cfg.extend([8, 0x25, 1, 0, 0, 0, 0, 0]);
        }
        let total = (9 + cfg.len()) as u16;
        let mut head = vec![9, 2];
        head.extend(total.to_le_bytes());
        head.extend([3, 1, 0, 0x80, 50]);
        head.extend(cfg);
        b.extend(head);
        b
    }

    /// Codex H2 / Claude M5 (round 2 of #306): each format's own playback path decides the volume:
    /// 192/24 plays on interface 2 and only its unit (0x0b) is set; 48/16 on interface 1, unit 5;
    /// the microphone's unit (2) never.
    #[test]
    fn the_volume_is_the_playing_paths_own() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(100));
        *app.raw.lock().unwrap() = Some(two_path_dac());
        *app.media_db.lock().unwrap() = Some(-10.0);
        exclusive(PreferredMode::Exclusive);
        let units = |app: &FakeApp| -> Vec<u8> {
            let mut u: Vec<u8> = app.dac.state().unit_writes.iter().map(|(u, _)| *u).collect();
            u.dedup();
            u
        };
        let s = open_ok(AudioFormat::pcm(192_000, 24, 2));
        assert_eq!(units(&app), [0x0b], "the 32-bit path's own unit");
        assert_eq!(app.dac.state().unit_writes, [(0x0b, -10 * 256)]);
        drop(s);
        app.dac.state().unit_writes.clear();
        drop(open_ok(AudioFormat::pcm(48_000, 16, 2)));
        assert_eq!(units(&app), [0x05], "the 16-bit path's own unit");
        assert!(!app.dac.state().log.iter().any(|l| l.contains("volume 2")));
    }

    /// Codex H3 / Claude M1 (round 2 of #306): the hand-over is Android's own level for the DAC
    /// (getStreamVolumeDb), not an assumed curve; without that reading, or within the give-back
    /// window (the reading may be the speaker's), the DAC starts muted at its minimum.
    #[test]
    fn the_hand_over_is_androids_own_level_for_the_dac_or_muted() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(50));
        *app.media_db.lock().unwrap() = Some(-30.0);
        exclusive(PreferredMode::Exclusive);
        drop(open_ok(AudioFormat::pcm(48_000, 16, 2)));
        assert_eq!(
            volume_of(&app),
            (Some(-30 * 256), Some(false)),
            "Android's -30 dB"
        );
        drop(_p);

        // No reading (Android 8, or media not on the DAC)
        let (_p, app) = phone(true, Some(50));
        exclusive(PreferredMode::Exclusive);
        drop(open_ok(AudioFormat::pcm(48_000, 16, 2)));
        assert_eq!(
            volume_of(&app),
            (Some(-127 * 256), Some(true)),
            "muted at its minimum"
        );
        // Given back: exactly as it was before (0 dB, not muted), not the hand-over's mute
        exclusive(PreferredMode::Shared);
        assert_eq!(volume_of(&app), (Some(0), Some(false)));
        drop(_p);

        // Moments after a give-back the reading may be the speaker's: muted
        let (_p, app) = phone(true, Some(50));
        *app.media_db.lock().unwrap() = Some(-5.0);
        mark_given_back("FiiO KA17");
        exclusive(PreferredMode::Exclusive);
        drop(open_ok(AudioFormat::pcm(48_000, 16, 2)));
        assert_eq!(volume_of(&app), (Some(-127 * 256), Some(true)));
    }

    /// Codex H1 (round 2 of #306): the DAC goes while Android's volume is read for the open:
    /// nothing opens on the default route (the guard fixed at the start stays).
    #[test]
    fn an_unplug_while_the_volume_is_read_never_opens_the_speaker() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(100));
        exclusive(PreferredMode::Shared);
        app.unplug_on_read
            .store(true, std::sync::atomic::Ordering::SeqCst);
        match open_phone(id(), AudioFormat::pcm(48_000, 16, 2)) {
            PhoneOpen::Gone(why) => assert!(why.contains("FiiO KA17"), "{why}"),
            PhoneOpen::Shared { opened_for, .. } => panic!("shared, for {opened_for:?}"),
            _ => panic!("nothing else"),
        }
    }

    /// Codex M5 (round 2 of #306): a confirmed stream that fails for good is no longer confirmed,
    /// published before the error goes on.
    #[test]
    fn a_failed_stream_is_no_longer_confirmed() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(100));
        exclusive(PreferredMode::Exclusive);
        let fmt = AudioFormat::pcm(96_000, 24, 2);
        let mut sink = open_ok(fmt);
        sink.write(&AudioBlock::pcm_i32(fmt, vec![0; 2 * 9_600])).unwrap();
        assert!(usb_dac::current().unwrap().confirmed);
        app.dac.state().stall = true;
        app.dac.state().packet_status = -71;
        app.dac.state().stall = false;
        let e = sink.write(&AudioBlock::pcm_i32(fmt, vec![0; 2 * 96_000]));
        assert!(e.is_err());
        assert!(!usb_dac::current().unwrap().confirmed);
    }

    /// Codex M2 / Claude M2 (round 3 of #306): the listener's level and mute, chosen on one
    /// playback path, carry to the next path a format plays on: never louder, mute kept.
    #[test]
    fn the_listeners_level_and_mute_carry_to_every_path() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(100));
        *app.raw.lock().unwrap() = Some(two_path_dac());
        *app.media_db.lock().unwrap() = Some(-10.0);
        exclusive(PreferredMode::Exclusive);
        drop(open_ok(AudioFormat::pcm(192_000, 24, 2)));
        assert_eq!(app.dac.state().unit_writes, [(0x0b, -10 * 256)]);
        // The listener turns it down on the 32-bit path; 48/16 plays on the 16-bit path at that
        set_volume(Volume::Db(-50.0)).unwrap();
        app.dac.state().unit_writes.clear();
        drop(open_ok(AudioFormat::pcm(48_000, 16, 2)));
        assert_eq!(
            app.dac.state().unit_writes,
            [(0x05, -50 * 256)],
            "the listener's -50 dB, not the hand-over's -10 dB"
        );
        assert_eq!(app.dac.state().muted, Some(false));
        // Muted on the 16-bit path: 192/24 plays muted on the 32-bit path
        set_volume(Volume::Mute(true)).unwrap();
        app.dac.state().log.clear();
        app.dac.state().unit_writes.clear();
        drop(open_ok(AudioFormat::pcm(192_000, 24, 2)));
        assert_eq!(app.dac.state().unit_writes, [(0x0b, -50 * 256)]);
        let log = app.dac.state().log.clone();
        assert!(log.contains(&"mute true".to_string()), "{log:?}");
        assert!(!log.contains(&"mute false".to_string()), "{log:?}");
        assert_eq!(app.dac.state().muted, Some(true));
        // No note that the hand-over muted it: the listener did
        let note = usb_dac::current().unwrap().note.unwrap_or_default();
        assert!(!note.contains("starts muted"), "{note}");
    }

    /// A level below what the unit of the next path goes plays that path muted, never at its
    /// (louder) minimum; a path's own level stays when it is the quieter.
    #[test]
    fn a_listeners_level_below_a_paths_range_plays_it_muted() {
        let quiet = VolumeRange::single(-20 * 256, 0, 256);
        let at = |db: i16, mute: bool| Level { cur: db * 256, mute };
        assert_eq!(
            path_level(at(0, false), Some(-30.0), false, &quiet),
            at(-20, true)
        );
        assert_eq!(
            path_level(at(0, false), Some(-10.0), false, &quiet),
            at(-10, false)
        );
        // Its own level quieter than the listener's: its own (never louder than Android played it)
        assert_eq!(
            path_level(at(-15, false), Some(-10.0), false, &quiet),
            at(-15, false)
        );
        // Muted before (its own): muted
        assert_eq!(
            path_level(at(-15, true), Some(-10.0), false, &quiet),
            at(-15, true)
        );
        // Muted by the listener: muted, whatever the level
        assert!(path_level(at(0, false), Some(-10.0), true, &quiet).mute);
        // Nothing chosen: its own
        assert_eq!(path_level(at(-5, false), None, false, &quiet), at(-5, false));
    }

    /// Codex M3 (round 3 of #306): the give-back drops the hand-over's attenuation but keeps the
    /// listener's: their mute, and no louder than their own level.
    #[test]
    fn the_give_back_keeps_the_listeners_mute_and_level() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(100));
        *app.media_db.lock().unwrap() = Some(0.0);
        exclusive(PreferredMode::Exclusive);
        drop(open_ok(AudioFormat::pcm(48_000, 16, 2)));
        assert_eq!(volume_of(&app), (Some(0), Some(false)));
        set_volume(Volume::Mute(true)).unwrap();
        exclusive(PreferredMode::Shared);
        assert!(!holding());
        assert_eq!(volume_of(&app), (Some(0), Some(true)), "given back muted");
        drop(_p);

        // Turned down by the listener: given back at their level, not the 0 dB from before
        let (_p, app) = phone(true, Some(100));
        *app.media_db.lock().unwrap() = Some(0.0);
        exclusive(PreferredMode::Exclusive);
        drop(open_ok(AudioFormat::pcm(48_000, 16, 2)));
        set_volume(Volume::Db(-20.0)).unwrap();
        exclusive(PreferredMode::Shared);
        assert_eq!(volume_of(&app), (Some(-20 * 256), Some(false)));
    }

    /// Codex M4 (round 3 of #306): the clock won't take the rate when it starts again after a
    /// pause: the DAC goes back to Android at once, nothing left claimed.
    #[test]
    fn a_failed_start_after_a_pause_gives_the_dac_back() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(100));
        exclusive(PreferredMode::Exclusive);
        let fmt = AudioFormat::pcm(96_000, 24, 2);
        let block = AudioBlock::pcm_i32(fmt, vec![0; 2 * 9_600]);
        let mut sink = open_ok(fmt);
        sink.write(&block).unwrap();
        assert!(!sink.pause().unwrap());
        app.dac.state().readback = Some(48_000);
        app.dac.state().log.clear();
        let e = sink.write(&block).unwrap_err();
        assert!(matches!(e, SinkError::Device(_)), "{e}");
        assert!(!holding(), "given back");
        assert_eq!(*app.released.lock().unwrap(), 1);
        let log = app.dac.state().log.clone();
        assert!(
            log.ends_with(&["release 1".into(), "release 0".into()]),
            "{log:?}"
        );
        drop(sink);
        assert_eq!(*app.released.lock().unwrap(), 1, "once");
    }

    /// Codex M4 (round 3 of #306): a DAC that stops taking packets for good goes back to Android
    /// at once.
    #[test]
    fn a_stream_that_fails_for_good_gives_the_dac_back() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(100));
        exclusive(PreferredMode::Exclusive);
        let fmt = AudioFormat::pcm(96_000, 24, 2);
        let mut sink = open_ok(fmt);
        sink.write(&AudioBlock::pcm_i32(fmt, vec![0; 2 * 9_600])).unwrap();
        app.dac.state().packet_status = -71;
        let e = sink
            .write(&AudioBlock::pcm_i32(fmt, vec![0; 2 * 96_000]))
            .unwrap_err();
        assert!(matches!(e, SinkError::Device(_)), "{e}");
        assert!(!holding(), "given back");
        assert_eq!(*app.released.lock().unwrap(), 1);
        drop(sink);
        assert_eq!(*app.released.lock().unwrap(), 1, "once");
    }

    /// Codex M5 (round 3 of #306): packets that fail while it drains (fewer than stop it) withdraw
    /// the confirmation; so do a pause and closing the sink.
    #[test]
    fn confirmation_follows_drain_pause_and_close() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(100));
        exclusive(PreferredMode::Exclusive);
        let fmt = AudioFormat::pcm(96_000, 24, 2);
        let confirmed = || usb_dac::current().unwrap().confirmed;
        let mut sink = open_ok(fmt);
        sink.write(&AudioBlock::pcm_i32(fmt, vec![0; 2 * 9_600])).unwrap();
        sink.drain().unwrap();
        // Not a whole transfer: what is left is sent by the drain (the fake's transfers finish
        // within write)
        sink.write(&AudioBlock::pcm_i32(fmt, vec![0; 2 * 390])).unwrap();
        assert!(confirmed());
        app.dac.state().packet_status = -71;
        sink.drain().expect("a few failed packets don't stop it");
        assert!(!confirmed(), "failed packets while it drained");
        app.dac.state().packet_status = 0;

        sink.write(&AudioBlock::pcm_i32(fmt, vec![0; 2 * 9_600])).unwrap();
        assert!(confirmed());
        assert!(!sink.pause().unwrap());
        assert!(!confirmed(), "paused");

        sink.write(&AudioBlock::pcm_i32(fmt, vec![0; 2 * 9_600])).unwrap();
        assert!(confirmed());
        drop(sink);
        assert!(!confirmed(), "closed");
    }

    /// Claude L5 (round 3 of #306): once Android lists the DAC given back again, its reading is
    /// the DAC's own again.
    #[test]
    fn the_hand_over_reads_android_again_once_it_lists_the_dac() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(50));
        *app.media_db.lock().unwrap() = Some(-5.0);
        mark_given_back("FiiO KA17");
        assert_eq!(handover_reading(), None);
        usb_dac::apply_report(&ka17_report(33, ""));
        assert_eq!(handover_reading(), Some(-5.0));
    }

    /// Codex H1 / Claude M2 (round 3 of #306): BitPerfect.kt reads Android's level only when media
    /// plays to the USB DAC and nothing else: never an attached DAC's type with another route's
    /// volume (Android 9 to 12: the stream's own output bits, else unknown, so muted).
    #[test]
    fn the_kotlin_side_reads_the_level_only_on_the_dacs_route() {
        let kt = include_str!("../../../tools/release/android/BitPerfect.kt");
        let f = kt
            .split("fun mediaVolumeDbOnUsb()")
            .nth(1)
            .and_then(|r| r.split("/// Devices went away").next())
            .expect("mediaVolumeDbOnUsb");
        assert!(
            !f.contains("GET_DEVICES_OUTPUTS"),
            "an attached output is not the route"
        );
        assert!(f.contains("routed.singleOrNull()?.takeIf { it in usb }"));
        assert!(f.contains("\"getDevicesForStream\""));
        assert!(f.contains("0x4000 -> AudioDeviceInfo.TYPE_USB_DEVICE"));
        assert!(f.contains("0x4000000 -> AudioDeviceInfo.TYPE_USB_HEADSET"));
        assert!(f.contains("else -> null"));
    }

    /// The fake DAC's units with levels and ranges of their own: (unit, level before in dB, muted
    /// before, minimum in dB; 1 dB steps up to 0 dB).
    fn own_units(app: &FakeApp, units: &[(u8, i16, bool, i16)]) {
        let mut s = app.dac.state();
        for &(id, db, muted, min) in units {
            let mut range = 1u16.to_le_bytes().to_vec();
            range.extend((min * 256).to_le_bytes());
            range.extend(0i16.to_le_bytes());
            range.extend(256u16.to_le_bytes());
            s.units.insert(
                id,
                pm_usb_audio::driver::fake::UnitFake {
                    volume: db * 256,
                    muted,
                    range,
                },
            );
        }
    }

    /// A unit's level and mute on the fake DAC.
    fn unit(app: &FakeApp, id: u8) -> (i16, bool) {
        let s = app.dac.state();
        (s.units[&id].volume, s.units[&id].muted)
    }

    /// Codex H1 (round 4 of #306): Android's media volume counts for full level only with a
    /// reading taken on the DAC's own route, full there too.
    #[test]
    fn only_a_reading_on_the_dacs_own_route_counts_as_full() {
        assert_eq!(media_on_dac(Some(100), Some(0.0)), Some(100));
        // Bluetooth or the speaker at 100 %: the DAC's reading unknown
        assert_eq!(media_on_dac(Some(100), None), None);
        assert_eq!(media_on_dac(Some(100), Some(f32::NAN)), None);
        // Full elsewhere, not on the DAC
        assert_eq!(media_on_dac(Some(100), Some(-6.0)), None);
        assert_eq!(media_on_dac(Some(40), Some(-20.0)), Some(40));
        assert_eq!(media_on_dac(None, Some(0.0)), None);
    }

    /// Codex H1 (round 4 of #306): media plays on Bluetooth at 100 % while the DAC is plugged in
    /// (Android's reading for the DAC unknown): a path without a volume of its own never plays at
    /// full level; once the reading is the DAC's own, at full, it does.
    #[test]
    fn another_routes_full_volume_never_plays_at_full_level() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let fmt = AudioFormat::pcm(48_000, 16, 2);
        // A DAC whose every path plays at full level, as Android reports it
        let report = |raw: &[u8]| {
            let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
            let ka17: String = KA17.iter().map(|b| format!("{b:02x}")).collect();
            ka17_report(33, "").replace(&ka17, &hex)
        };
        for mode in [PreferredMode::Auto, PreferredMode::Exclusive] {
            let (_p, app) = phone(true, Some(100));
            let raw = paths_dac(false, false);
            *app.raw.lock().unwrap() = Some(raw.clone());
            usb_dac::apply_report(&report(&raw));
            exclusive(mode);
            let dac = usb_dac::current_dac().unwrap();
            assert!(
                !plays_exclusive(&dac, usb_dac::settings(), false, Some(100)),
                "{mode:?}: Bluetooth's 100 % says nothing about the DAC"
            );
            match open_phone(id(), fmt) {
                PhoneOpen::Exclusive(_) | PhoneOpen::BitPerfect(_) => {
                    panic!("{mode:?}: full level on another route's volume")
                }
                _ => {}
            }
            assert!(app.dac.state().out.is_empty(), "{mode:?}: not a sample sent");
            assert!(!holding());
            drop(_p);

            // Android plays the DAC itself at full volume: full level
            let (_p, app) = phone(true, Some(100));
            *app.raw.lock().unwrap() = Some(raw.clone());
            *app.media_db.lock().unwrap() = Some(0.0);
            usb_dac::apply_report(&report(&raw));
            exclusive(mode);
            let mut s = open_ok(fmt);
            s.write(&AudioBlock::pcm_i32(fmt, vec![1; 2 * 4_800])).unwrap();
            drop(s);
        }
        // Held, a path without a unit after one with: the take's reading decides, not now's
        let (_p, app) = phone(true, Some(100));
        *app.raw.lock().unwrap() = Some(paths_dac(false, true));
        exclusive(PreferredMode::Auto);
        drop(open_ok(AudioFormat::pcm(192_000, 24, 2)));
        // Android's route is the speaker now (the app holds the DAC), full there
        *app.media_db.lock().unwrap() = Some(0.0);
        assert!(matches!(open_phone(id(), fmt), PhoneOpen::Failed(_)));
    }

    /// Claude M1 (round 4 of #306): a path the listener hasn't set starts at its own hand-over
    /// (Android's level for it), no louder than the listener's level chosen on another path; a
    /// path muted before starts muted. One the listener set keeps their level.
    #[test]
    fn a_new_path_is_no_louder_than_android_played_it() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(100));
        *app.raw.lock().unwrap() = Some(two_path_dac());
        // Before the take: the 32-bit path's unit at 0 dB, the 16-bit one's at -20 dB, muted
        own_units(&app, &[(0x0b, 0, false, -127), (5, -20, true, -127)]);
        *app.media_db.lock().unwrap() = Some(-10.0);
        exclusive(PreferredMode::Exclusive);
        drop(open_ok(AudioFormat::pcm(192_000, 24, 2)));
        assert_eq!(unit(&app, 0x0b), (-10 * 256, false));
        // One step down on it: -12 dB
        set_volume(Volume::Down).unwrap();
        assert_eq!(unit(&app, 0x0b), (-12 * 256, false));
        // The 16-bit path: Android played it at -30 dB, muted: so it starts, not at -12 dB
        drop(open_ok(AudioFormat::pcm(48_000, 16, 2)));
        assert_eq!(unit(&app, 5), (-30 * 256, true));
        // The listener sets it: theirs from then on
        set_volume(Volume::Db(-25.0)).unwrap();
        assert_eq!(unit(&app, 5), (-25 * 256, false));
        drop(open_ok(AudioFormat::pcm(48_000, 16, 2)));
        assert_eq!(unit(&app, 5), (-25 * 256, false));
        // Back on the 32-bit path: its own -12 dB, capped at the listener's -25 dB
        drop(open_ok(AudioFormat::pcm(192_000, 24, 2)));
        assert_eq!(unit(&app, 0x0b), (-25 * 256, false));
    }

    /// Codex M2 (round 4 of #306): a path without a volume of its own can't honour the listener's
    /// mute or level: muted, it never plays; turned down, only with the full-level consent; while
    /// it plays, the zone's volume can't be set on another path's unit.
    #[test]
    fn a_path_without_a_volume_keeps_the_listeners_mute_and_level() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mixed = || paths_dac(false, true);
        let (fast, slow) = (AudioFormat::pcm(192_000, 24, 2), AudioFormat::pcm(48_000, 16, 2));
        // Muted: the 16-bit path (full level) doesn't play
        let (_p, app) = phone(true, Some(100));
        *app.raw.lock().unwrap() = Some(mixed());
        *app.media_db.lock().unwrap() = Some(0.0);
        exclusive(PreferredMode::Exclusive);
        drop(open_ok(fast));
        set_volume(Volume::Mute(true)).unwrap();
        app.dac.state().out.clear();
        match open_phone(id(), slow) {
            PhoneOpen::Failed(why) => assert!(why.contains("muted"), "{why}"),
            _ => panic!("a muted zone played at full level"),
        }
        assert!(app.dac.state().out.is_empty(), "not a sample sent");
        drop(_p);

        // Turned down: full level needs the consent
        let (_p, app) = phone(true, Some(100));
        *app.raw.lock().unwrap() = Some(mixed());
        *app.media_db.lock().unwrap() = Some(0.0);
        exclusive(PreferredMode::Exclusive);
        drop(open_ok(fast));
        set_volume(Volume::Db(-30.0)).unwrap();
        match open_phone(id(), slow) {
            PhoneOpen::Failed(why) => assert!(why.contains("confirm Exclusive USB"), "{why}"),
            _ => panic!("played at full level after the listener turned it down"),
        }
        drop(_p);

        // Neither: it plays at full level (Android at full for the DAC); meanwhile the zone's
        // volume has nothing to move
        let (_p, app) = phone(true, Some(100));
        *app.raw.lock().unwrap() = Some(mixed());
        *app.media_db.lock().unwrap() = Some(0.0);
        exclusive(PreferredMode::Exclusive);
        let s = open_ok(slow);
        assert!(set_volume(Volume::Db(-10.0)).is_err());
        drop(s);
    }

    /// Codex M3 / Claude L4 (round 4 of #306): given back, a path whose minimum is above the
    /// listener's level stays muted; the others no louder than the listener's level.
    #[test]
    fn the_give_back_keeps_a_path_muted_that_cant_go_as_low() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(100));
        *app.raw.lock().unwrap() = Some(two_path_dac());
        own_units(&app, &[(0x0b, 0, false, -127), (5, 0, false, -20)]);
        *app.media_db.lock().unwrap() = Some(0.0);
        exclusive(PreferredMode::Exclusive);
        drop(open_ok(AudioFormat::pcm(192_000, 24, 2)));
        set_volume(Volume::Db(-50.0)).unwrap();
        drop(open_ok(AudioFormat::pcm(48_000, 16, 2)));
        assert_eq!(unit(&app, 5), (-20 * 256, true), "can't go to -50 dB: muted");
        exclusive(PreferredMode::Shared);
        assert!(!holding());
        assert_eq!(unit(&app, 5), (-20 * 256, true), "given back muted");
        assert_eq!(unit(&app, 0x0b), (-50 * 256, false));
    }

    /// Codex M4 (round 4 of #306): a format the DAC's clock turns out not to take (Android listed
    /// it) gives the DAC back to Android, not kept claimed.
    #[test]
    fn an_unsupported_format_gives_the_dac_back() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(100));
        {
            let mut s = app.dac.state();
            let mut b = 1u16.to_le_bytes().to_vec();
            b.extend(48_000u32.to_le_bytes());
            b.extend(48_000u32.to_le_bytes());
            b.extend(0u32.to_le_bytes());
            s.rate_ranges = Some(b);
        }
        exclusive(PreferredMode::Exclusive);
        match open_phone(id(), AudioFormat::pcm(96_000, 24, 2)) {
            PhoneOpen::Unsupported(_) => {}
            _ => panic!("the clock doesn't take 96 kHz"),
        }
        assert!(!holding(), "given back");
        assert_eq!(*app.released.lock().unwrap(), 1);
        assert!(app.dac.state().out.is_empty());
    }

    /// Codex M5 (round 4 of #306): two identical DACs attached, the recorded one under its own
    /// name: recovery opens exactly that one, never its twin (Android's, playing); the recorded
    /// one gone or another device now: nothing is opened, the claim is forgotten.
    #[test]
    fn recovery_opens_only_the_recorded_dac_never_its_twin() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_p, app) = phone(true, Some(100));
        let dev = |name: &str| UsbDeviceRef {
            device_name: name.into(),
            vendor_id: 0x2972,
            product_id: 0x0093,
            audio: true,
        };
        let (twin_name, ours) = ("/dev/bus/usb/001/004", "/dev/bus/usb/001/005");
        let twin = FakeDac::high_speed();
        *app.named.lock().unwrap() = vec![(twin_name.into(), twin.clone()), (ours.into(), app.dac.clone())];
        // The twin is listed first: a choice by the USB ids would take it
        *app.claims.lock().unwrap() = Claims {
            recorded: Some(dev(ours)),
            held: false,
            attached: vec![dev(twin_name), dev(ours)],
        };
        recover();
        assert_eq!(*app.opened_names.lock().unwrap(), [ours]);
        assert!(twin.state().log.is_empty(), "the twin untouched");
        let log = app.dac.state().log.clone();
        assert!(
            log.ends_with(&["release 1".into(), "release 0".into()]),
            "{log:?}"
        );
        assert_eq!(*app.released.lock().unwrap(), 1);
        assert_eq!(*app.opened.lock().unwrap(), 0, "never opened by its ids");

        // Unplugged since, the twin still there: nothing opened, the claim forgotten
        app.opened_names.lock().unwrap().clear();
        app.claims.lock().unwrap().attached = vec![dev(twin_name)];
        recover();
        assert!(app.opened_names.lock().unwrap().is_empty());
        assert!(twin.state().log.is_empty());
        assert_eq!(*app.forgot.lock().unwrap(), 1);

        // Another device under that name now: nothing opened
        app.claims.lock().unwrap().attached = vec![
            dev(twin_name),
            UsbDeviceRef {
                product_id: 1,
                ..dev(ours)
            },
        ];
        recover();
        assert!(app.opened_names.lock().unwrap().is_empty());
        assert_eq!(*app.forgot.lock().unwrap(), 2);

        // The app holds a DAC now: nothing
        *app.claims.lock().unwrap() = Claims {
            recorded: Some(dev(ours)),
            held: true,
            attached: vec![dev(twin_name), dev(ours)],
        };
        recover();
        assert!(app.opened_names.lock().unwrap().is_empty());
        assert_eq!(*app.forgot.lock().unwrap(), 2);
    }

    /// Claude L3 (round 4 of #306): a drain or a pause that fails for good gives the DAC back.
    #[test]
    fn a_drain_or_pause_that_fails_for_good_gives_the_dac_back() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let fmt = AudioFormat::pcm(96_000, 24, 2);
        let (_p, app) = phone(true, Some(100));
        exclusive(PreferredMode::Exclusive);
        let mut sink = open_ok(fmt);
        sink.write(&AudioBlock::pcm_i32(fmt, vec![0; 2 * 9_600])).unwrap();
        // The DAC stops taking audio: what is left (less than a transfer) never finishes
        app.dac.state().stall = true;
        sink.write(&AudioBlock::pcm_i32(fmt, vec![0; 2 * 6])).unwrap();
        assert!(matches!(sink.drain(), Err(SinkError::Device(_))));
        assert!(!holding(), "given back");
        assert_eq!(*app.released.lock().unwrap(), 1);
        drop(sink);
        drop(_p);

        let (_p, app) = phone(true, Some(100));
        exclusive(PreferredMode::Exclusive);
        let mut sink = open_ok(fmt);
        sink.write(&AudioBlock::pcm_i32(fmt, vec![0; 2 * 9_600])).unwrap();
        app.dac.state().refuse_alt = true;
        assert!(matches!(sink.pause(), Err(SinkError::Device(_))));
        assert!(!holding(), "given back");
        assert_eq!(*app.released.lock().unwrap(), 1);
    }
}
