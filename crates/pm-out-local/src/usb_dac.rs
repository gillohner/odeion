//! USB DACs on an Android phone, as the app's Kotlin side reports them
//! (`tools/release/android/UsbAudio.kt`): what is attached, by name, with the rates, sample sizes
//! and channels Android lists for it, and, when the app already has permission for the USB
//! device, its raw descriptors (parsed by [`pm_usb_audio`]).
//!
//! Nothing here talks to the JVM, so it is built and tested on every host: the app hands the
//! report over as text ([`parse_poll`], [`parse_report`]), [`describe`] turns it into the
//! [`OutputDevice`] the phone's output reports, and [`apply_report`] keeps it for that output.
//!
//! On Android 14 and later the report also lists the DAC's mixer attributes
//! (`getSupportedMixerAttributes`), from which [`crate::bit_perfect`] plays it bit-perfect; the
//! description itself ([`describe`]) always says shared, and [`current`] says what the last open
//! got, confirmed or not, with why.

use pm_zones::{DeviceKind, DeviceMode, DeviceSettings, DsdKind, OutputDevice, UsbIds};
use serde::Deserialize;
use std::sync::Mutex;

/// `AudioDeviceInfo.TYPE_USB_DEVICE`.
pub const TYPE_USB_DEVICE: i32 = 11;
/// `AudioDeviceInfo.TYPE_USB_HEADSET`.
pub const TYPE_USB_HEADSET: i32 = 22;
/// `AudioFormat.ENCODING_PCM_16BIT`.
pub const ENCODING_PCM_16BIT: i32 = 2;
/// `AudioFormat.ENCODING_PCM_24BIT_PACKED` (API 31).
pub const ENCODING_PCM_24BIT_PACKED: i32 = 21;
/// `AudioFormat.ENCODING_PCM_32BIT` (API 31).
pub const ENCODING_PCM_32BIT: i32 = 22;
/// `AudioFormat.ENCODING_DSD` (API 34).
pub const ENCODING_DSD: i32 = 31;

/// The largest report accepted, in bytes (raw descriptors as hex are a few KB).
pub const MAX_REPORT: usize = 256 * 1024;
/// At most this many devices, rates, encodings or channel counts are read from a report.
const MAX_ITEMS: usize = 64;
/// Raw descriptors larger than this (decoded) are ignored.
const MAX_DESCRIPTORS: usize = 64 * 1024;
const MAX_NAME: usize = 64;

/// What `UsbAudio.kt` reports: Android's SDK level and the USB audio sinks attached.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Report {
    /// The report's format version (1).
    #[serde(default)]
    pub v: u32,
    /// `Build.VERSION.SDK_INT`.
    #[serde(default)]
    pub sdk: u32,
    #[serde(default)]
    pub devices: Vec<ReportDevice>,
    /// Android's warning that audio is about to move off the device it plays on (a USB DAC
    /// being unplugged), sent before the device's removal: the DAC counts as gone at once.
    #[serde(default)]
    pub noisy: bool,
    /// Android's media volume, 0 to 100 (-1: unknown); the app reports again at every change.
    #[serde(default)]
    pub media_volume: Option<i32>,
}

/// One output `AudioDeviceInfo`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ReportDevice {
    /// `getId()`.
    #[serde(default)]
    pub id: i32,
    /// `getType()`.
    #[serde(default, rename = "type")]
    pub kind: i32,
    /// `getProductName()`.
    #[serde(default)]
    pub name: String,
    /// `getSampleRates()`: empty means any rate.
    #[serde(default)]
    pub rates: Vec<u32>,
    /// `getEncodings()`.
    #[serde(default)]
    pub encodings: Vec<i32>,
    /// `getChannelCounts()`.
    #[serde(default)]
    pub channel_counts: Vec<u32>,
    /// Android routes media there now (`getAudioDevicesForAttributes`, API 33); `None` before.
    #[serde(default)]
    pub routed: Option<bool>,
    /// The matching `UsbDevice`, when one was found.
    #[serde(default)]
    pub usb: Option<ReportUsb>,
    /// `AudioManager.getSupportedMixerAttributes(device)` (API 34): the formats Android can set
    /// its mixer to for this device, and how.
    #[serde(default)]
    pub mixer: Vec<MixerEntry>,
}

/// One `AudioMixerAttributes`: its `AudioFormat` and mixer behavior.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, serde::Serialize)]
pub struct MixerEntry {
    /// `AudioFormat.getEncoding()`.
    #[serde(default)]
    pub encoding: i32,
    /// `AudioFormat.getSampleRate()`.
    #[serde(default)]
    pub rate: u32,
    /// `AudioFormat.getChannelMask()`.
    #[serde(default)]
    pub channel_mask: i32,
    /// `AudioFormat.getChannelCount()`.
    #[serde(default)]
    pub channels: u8,
    /// `getMixerBehavior()`: [`MIXER_BEHAVIOR_BIT_PERFECT`] or default.
    #[serde(default)]
    pub behavior: i32,
}

/// `AudioMixerAttributes.MIXER_BEHAVIOR_BIT_PERFECT`.
pub const MIXER_BEHAVIOR_BIT_PERFECT: i32 = 1;

/// The `UsbDevice` behind an audio device.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ReportUsb {
    #[serde(default)]
    pub vendor_id: u16,
    #[serde(default)]
    pub product_id: u16,
    #[serde(default)]
    pub manufacturer: Option<String>,
    #[serde(default)]
    pub product: Option<String>,
    /// `UsbDeviceConnection.getRawDescriptors()` as hex, only when the app already had permission.
    #[serde(default)]
    pub descriptors: Option<String>,
}

/// `UsbAudio.next()`'s answer: `"<version>\n<report>"`.
pub fn parse_poll(s: &str) -> Option<(u64, &str)> {
    let (v, rest) = s.split_once('\n')?;
    Some((v.trim().parse().ok()?, rest))
}

/// A report from its JSON, with every list cut to a sane length.
pub fn parse_report(json: &str) -> Result<Report, String> {
    if json.len() > MAX_REPORT {
        return Err(format!("a USB audio report of {} bytes", json.len()));
    }
    let mut r: Report = serde_json::from_str(json).map_err(|e| format!("a USB audio report: {e}"))?;
    r.devices.truncate(MAX_ITEMS);
    for d in &mut r.devices {
        d.rates.truncate(MAX_ITEMS);
        d.encodings.truncate(MAX_ITEMS);
        d.channel_counts.truncate(MAX_ITEMS);
        d.mixer.truncate(4 * MAX_ITEMS);
    }
    Ok(r)
}

fn is_usb_sink(d: &ReportDevice) -> bool {
    d.kind == TYPE_USB_DEVICE || d.kind == TYPE_USB_HEADSET
}

/// Printable, trimmed, at most [`MAX_NAME`] characters.
fn clean(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    s.chars().take(MAX_NAME).collect::<String>().trim().to_string()
}

/// The name people know it by: Android's product name without the "USB-Audio - " some phones put
/// first, else the USB product string, with the maker in front when neither says it.
pub fn display_name(d: &ReportDevice) -> String {
    let mut name = clean(&d.name);
    for prefix in ["USB-Audio - ", "USB Audio - ", "USB-Audio-"] {
        if let Some(rest) = name.strip_prefix(prefix) {
            name = rest.trim().to_string();
        }
    }
    let usb = d.usb.as_ref();
    if name.is_empty() {
        name = usb
            .and_then(|u| u.product.as_deref())
            .map(clean)
            .unwrap_or_default();
    }
    if let Some(maker) = usb.and_then(|u| u.manufacturer.as_deref()).map(clean) {
        if !maker.is_empty() && !name.to_lowercase().contains(&maker.to_lowercase()) {
            name = if name.is_empty() {
                maker
            } else {
                format!("{maker} {name}")
            };
        }
    }
    if name.is_empty() {
        name = "USB audio device".into();
    }
    name.chars().take(MAX_NAME).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) || s.len() / 2 > MAX_DESCRIPTORS {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// The descriptors' summary, when they are there, parse as an audio device, and are the same
/// USB device (vendor and product id).
fn descriptors_summary(d: &ReportDevice, name: &str) -> Option<pm_usb_audio::Summary> {
    let usb = d.usb.as_ref()?;
    let raw = unhex(usb.descriptors.as_deref()?)?;
    let parsed = pm_usb_audio::parse(&raw);
    if parsed.uac.is_none() || (parsed.vendor_id, parsed.product_id) != (usb.vendor_id, usb.product_id) {
        return None;
    }
    Some(parsed.summary(name, &[]))
}

/// The Android major version for an SDK level, for words people know ("Android 13").
pub(crate) fn android_version(sdk: u32) -> String {
    match sdk {
        36.. => (sdk - 20).to_string(),
        35 => "15".into(),
        34 => "14".into(),
        33 => "13".into(),
        32 => "12L".into(),
        31 => "12".into(),
        30 => "11".into(),
        29 => "10".into(),
        28 => "9".into(),
        26 | 27 => "8".into(),
        _ => format!("API {sdk}"),
    }
}

/// The USB DAC the phone plays through, from a report: the USB sink Android routes media to; the
/// first one only when Android does not say where media goes (before API 33). `None` when no USB
/// audio sink is attached, or media goes elsewhere (Bluetooth, the speaker).
pub fn describe(report: &Report) -> Option<OutputDevice> {
    describe_device(report, pick(report)?)
}

/// The USB sink [`describe`] is about.
fn pick(report: &Report) -> Option<&ReportDevice> {
    let sinks: Vec<&ReportDevice> = report.devices.iter().filter(|d| is_usb_sink(d)).collect();
    // With Android's routing (API 33+) only the sink media goes to; without it, the first one
    Some(if sinks.iter().any(|d| d.routed.is_some()) {
        *sinks.iter().find(|d| d.routed == Some(true))?
    } else {
        *sinks.first()?
    })
}

/// The DAC the phone plays through: its description, Android's id for it, its mixer attributes
/// and the phone's Android.
#[derive(Debug, Clone, PartialEq)]
pub struct Dac {
    pub device: OutputDevice,
    /// `AudioDeviceInfo.getId()`.
    pub id: i32,
    pub mixer: Vec<MixerEntry>,
    pub sdk: u32,
}

/// The [`Dac`] of a report, when there is one.
pub fn dac(report: &Report) -> Option<Dac> {
    let d = pick(report)?;
    Some(Dac {
        device: describe_device(report, d)?,
        id: d.id,
        mixer: d.mixer.clone(),
        sdk: report.sdk,
    })
}

fn describe_device(report: &Report, d: &ReportDevice) -> Option<OutputDevice> {
    let name = display_name(d);
    let desc = descriptors_summary(d, &name);
    let mut rates: Vec<u32> = d
        .rates
        .iter()
        .copied()
        .filter(|r| (8_000..=1_536_000).contains(r))
        .collect();
    if rates.is_empty() {
        // Android lists none for "any rate": the descriptors say which
        rates = desc.as_ref().map(|s| s.rates.clone()).unwrap_or_default();
    }
    rates.sort_unstable();
    rates.dedup();
    let mut bits: Vec<u8> = d
        .encodings
        .iter()
        .filter_map(|e| match *e {
            ENCODING_PCM_16BIT => Some(16),
            ENCODING_PCM_24BIT_PACKED => Some(24),
            ENCODING_PCM_32BIT => Some(32),
            _ => None,
        })
        .collect();
    if bits.is_empty() {
        bits.extend(desc.as_ref().map(|s| s.max_bits).filter(|b| *b > 0));
    }
    bits.sort_unstable();
    bits.dedup();
    let channels = d
        .channel_counts
        .iter()
        .copied()
        .max()
        .unwrap_or(0)
        .min(u32::from(u8::MAX)) as u8;
    let dsd = (d.encodings.contains(&ENCODING_DSD) || desc.as_ref().is_some_and(|s| s.dsd))
        .then_some(DsdKind::Native);
    let hardware_volume = desc.as_ref().is_some_and(|s| s.hardware_volume);
    let summary = pm_usb_audio::Summary {
        name: name.clone(),
        uac: desc.as_ref().and_then(|s| s.uac),
        rates: rates.clone(),
        max_bits: bits.last().copied().unwrap_or(0),
        dsd: dsd.is_some(),
        hardware_volume,
    }
    .line();
    let note = if report.sdk == 0 || report.sdk >= 34 {
        None
    } else {
        Some(format!(
            "Bit-perfect playback needs Android 14 or later; this phone has Android {}.",
            android_version(report.sdk)
        ))
    };
    Some(OutputDevice {
        name,
        kind: if d.kind == TYPE_USB_HEADSET {
            DeviceKind::UsbHeadset
        } else {
            DeviceKind::UsbDac
        },
        summary,
        max_rate: rates.last().copied().unwrap_or(0),
        max_bits: bits.last().copied().unwrap_or(0),
        rates,
        bits,
        channels,
        dsd,
        hardware_volume,
        // Android's mixer still plays it: shared, and nothing confirmed
        mode: DeviceMode::Shared,
        modes: vec![DeviceMode::Shared],
        confirmed: false,
        note,
        usb: d
            .usb
            .as_ref()
            .filter(|u| u.vendor_id != 0 || u.product_id != 0)
            .map(|u| UsbIds {
                vendor_id: u.vendor_id,
                product_id: u.product_id,
            }),
        format: None,
        settings: None,
        formats: Vec::new(),
        media_volume: None,
    })
}

/// What the phone's output knows about its DAC: the DAC, the listener's settings, what the last
/// open got, and how often a DAC went away.
#[derive(Default)]
struct Phone {
    dac: Option<Dac>,
    settings: DeviceSettings,
    last: Option<LastOpen>,
    /// Counts DACs that went away: a sink opened on a DAC stops when it changes.
    gone: u64,
    /// Why the last one went ("FiiO KA17 was unplugged", or media moved elsewhere).
    gone_why: String,
    /// Where the settings are kept (the app's data folder), once known.
    settings_file: Option<std::path::PathBuf>,
    /// The DAC the listener's agreement to full level (`full_level_ok`) was given for.
    consent_dac: Option<String>,
    /// Sinks open for a DAC now, and when the last one closed: an unplug then is one no sink
    /// may have seen yet.
    sinks_open: u32,
    last_close: Option<std::time::Instant>,
    /// A DAC went while a sink was open for it or moments after one closed, and nothing has
    /// reported it yet: the next open pauses instead of opening the default route.
    gone_unseen: Option<(String, std::time::Instant)>,
    /// Android's media volume as the app last reported it.
    media: Option<u8>,
}

/// [`gone_count`] without the lock, for the audio callback.
static GONE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Counts changes to the settings or the DAC: a sink opened before one is closed at the next song,
/// so the change applies from there (`AudioSink::reopen_at_track`).
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Called when what the phone's output reports changed outside a report from the app (a
/// confirmation withdrawn): the app registers its player again, so the node learns it.
static ON_CHANGE: Mutex<Option<Box<dyn Fn() + Send>>> = Mutex::new(None);

/// What to call when the output's device changed by itself (the app's re-registration).
pub fn on_change(f: impl Fn() + Send + 'static) {
    *ON_CHANGE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
}

pub(crate) fn notify_change() {
    if let Some(f) = ON_CHANGE.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        f();
    }
}

/// The settings-or-DAC generation: it changes with the settings or the DAC, and a sink opened
/// before a change is closed at the next song.
pub fn generation() -> u64 {
    GENERATION.load(std::sync::atomic::Ordering::SeqCst)
}

/// [`gone_count`], lock-free.
pub fn gone_count_now() -> u64 {
    GONE.load(std::sync::atomic::Ordering::SeqCst)
}

/// Why the last DAC went.
pub fn gone_reason() -> String {
    with_phone(|p| {
        if p.gone_why.is_empty() {
            "the USB DAC was unplugged".into()
        } else {
            p.gone_why.clone()
        }
    })
}

/// An unplug this soon after the last sink for the DAC closed counts as one the zone hasn't seen
/// (it lands between one song's sink closing and the next one opening).
const UNSEEN_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);

/// The DAC a sink (or an open) is for, as it was then: rule (b)'s reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedFor {
    pub dac_id: i32,
    pub name: String,
    /// The count of DACs gone then.
    pub gone_at: u64,
}

/// The phone's DAC now, as something to open for (`None`: no DAC).
pub fn opened_for_now() -> Option<OpenedFor> {
    with_phone(|p| {
        p.dac.as_ref().map(|d| OpenedFor {
            dac_id: d.id,
            name: d.device.name.clone(),
            gone_at: p.gone,
        })
    })
}

/// **The one gate for rule (b), "nothing reaches another device":** audio opened for `f` may
/// reach a device only while that DAC is still the one Android plays media to (attached, the
/// routed one: the report names no other) and no DAC went since. Opened without a DAC (`None`):
/// always. Every write, drain, flush, resume and open of the phone's sinks asks here (the audio
/// callback asks [`may_reach_now`]); `Err` is why, for [`pm_zones::SinkError::Gone`].
pub fn may_reach(f: Option<&OpenedFor>) -> Result<(), String> {
    let Some(f) = f else { return Ok(()) };
    with_phone(|p| {
        if p.gone != f.gone_at {
            return Err(if p.gone_why.is_empty() {
                format!("{} was unplugged", f.name)
            } else {
                p.gone_why.clone()
            });
        }
        match p.dac.as_ref() {
            Some(d) if d.id == f.dac_id => Ok(()),
            Some(_) | None => Err(format!("{} is not where Android plays now", f.name)),
        }
    })
}

/// [`may_reach`] without a lock, for the audio callback: no DAC went since `gone_at` (a DAC
/// replaced or no longer routed counts as gone too).
pub fn may_reach_now(gone_at: u64) -> bool {
    GONE.load(std::sync::atomic::Ordering::SeqCst) == gone_at
}

/// An unplug no sink has reported yet (see [`DacUse`]): taken once, by the next open, which
/// pauses the zone with it instead of opening the default route.
pub fn take_gone_unseen() -> Option<String> {
    // Only as long as it can be the boundary it was kept for: a Play hours later is the listener's
    with_phone(|p| p.gone_unseen.take())
        .filter(|(_, at)| at.elapsed() < UNSEEN_WINDOW)
        .map(|(why, _)| why)
}

/// Make the kept unplug `by` older (tests).
#[cfg(test)]
pub(crate) fn age_gone_unseen(by: std::time::Duration) {
    with_phone(|p| {
        if let Some((_, at)) = p.gone_unseen.as_mut() {
            *at = at.checked_sub(by).unwrap_or(*at);
        }
    });
}

/// A sink or an open reported the DAC gone: the zone knows (the next open is the listener's).
pub fn gone_seen() {
    with_phone(|p| p.gone_unseen = None);
}

/// Held by a sink opened for a DAC while it lives: an unplug while one is open, or moments after
/// it closed, is one the zone must still hear about ([`take_gone_unseen`]).
#[derive(Debug)]
pub struct DacUse(bool);

impl DacUse {
    pub fn new(f: Option<&OpenedFor>) -> Self {
        if f.is_some() {
            with_phone(|p| p.sinks_open += 1);
        }
        DacUse(f.is_some())
    }
}

impl Drop for DacUse {
    fn drop(&mut self) {
        if self.0 {
            with_phone(|p| {
                p.sinks_open = p.sinks_open.saturating_sub(1);
                let now = std::time::Instant::now();
                p.last_close = Some(now);
                // A loss nobody heard is the next open's from here (the boundary is now)
                if let Some((_, at)) = p.gone_unseen.as_mut() {
                    *at = now;
                }
            });
        }
    }
}

/// **The gate a sink holds for the DAC it was opened for** (rule (b)): every sink of the phone's
/// output (bit-perfect, shared) asks it before anything reaches a device, and holds its
/// [`DacUse`]. A Gone it says is kept for the next open until the zone heard it
/// ([`DacGate::reported`]); a Gone nobody heard (a stream closed with it) stays kept, so nothing
/// opens afterwards except on the DAC.
#[derive(Debug)]
pub struct DacGate {
    opened_for: Option<OpenedFor>,
    _use: DacUse,
}

impl DacGate {
    /// For the DAC `f` (`None`: opened without one; the gate is always open).
    pub fn new(f: Option<OpenedFor>) -> Self {
        DacGate {
            _use: DacUse::new(f.as_ref()),
            opened_for: f,
        }
    }

    pub fn opened_for(&self) -> Option<&OpenedFor> {
        self.opened_for.as_ref()
    }

    /// Gone (with why) when its DAC isn't Android's route any more ([`may_reach`]).
    pub fn gone(&self, id: &pm_zones::OutputId) -> Option<pm_zones::SinkError> {
        let why = may_reach(self.opened_for.as_ref()).err()?;
        Some(pm_zones::SinkError::Gone(id.clone(), why))
    }

    /// The zone heard this sink's Gone: the next open is the listener's.
    pub fn reported(&self) {
        gone_seen();
    }
}

/// Android's media volume as the app last reported it (`None`: not reported, or unknown).
pub fn reported_media_volume() -> Option<u8> {
    with_phone(|p| p.media)
}

/// What an open of the phone's output got, on which DAC.
#[derive(Debug, Clone, PartialEq)]
pub struct LastOpen {
    pub dac_id: i32,
    pub format: pm_model::AudioFormat,
    pub mode: DeviceMode,
    pub confirmed: bool,
    /// Why it is not bit-perfect, when it is not.
    pub note: Option<String>,
}

static PHONE: Mutex<Option<Phone>> = Mutex::new(None);

fn with_phone<T>(f: impl FnOnce(&mut Phone) -> T) -> T {
    let mut p = PHONE.lock().unwrap_or_else(|e| e.into_inner());
    f(p.get_or_insert_with(Phone::default))
}

/// The phone's DAC as the output reports it: described, with the modes it allows, the settings,
/// and what the last open got (confirmed bit-perfect, or shared with why).
pub fn current() -> Option<OutputDevice> {
    with_phone(|p| view(p))
}

fn view(p: &Phone) -> Option<OutputDevice> {
    let d = p.dac.as_ref()?;
    let mut dev = d.device.clone();
    let offer = crate::bit_perfect::offer(d);
    dev.modes = if offer.is_ok() {
        vec![DeviceMode::Shared, DeviceMode::BitPerfect]
    } else {
        vec![DeviceMode::Shared]
    };
    dev.settings = Some(p.settings);
    // What bit-perfect would play now, and Android's volume (it plays at full level)
    let (formats, media) = crate::bit_perfect::view_extras(d, p.settings, consent(p), p.media);
    dev.formats = formats;
    dev.media_volume = media;
    match p.last.as_ref().filter(|l| l.dac_id == d.id) {
        Some(l) => {
            dev.mode = l.mode;
            dev.confirmed = l.confirmed && l.mode != DeviceMode::Shared;
            dev.note = l.note.clone();
            dev.format = Some(l.format);
        }
        // Nothing played on it yet: why it can't be bit-perfect, when it can't
        None => {
            if let Err(why) = offer {
                dev.note = dev.note.or(Some(why));
            }
        }
    }
    Some(dev)
}

/// Whether the listener agreed to full level for the DAC attached now.
fn consent(p: &Phone) -> bool {
    p.settings.full_level_ok && p.dac.as_ref().map(|d| &d.device.name) == p.consent_dac.as_ref()
}

/// Whether the listener agreed to full level (Bit-perfect below full volume) for the DAC attached now.
pub fn full_level_consent() -> bool {
    with_phone(|p| consent(p))
}

/// The DAC itself, for the bit-perfect player.
pub fn current_dac() -> Option<Dac> {
    with_phone(|p| p.dac.clone())
}

/// The listener's settings for the phone's DAC.
pub fn settings() -> DeviceSettings {
    with_phone(|p| p.settings)
}

/// Keep the listener's settings (and save them when the app said where). Returns whether they
/// changed.
pub fn set_settings(s: DeviceSettings) -> bool {
    let mut s = s;
    // The agreement to full level goes with Bit-perfect, for the DAC attached now
    if s.mode != pm_zones::PreferredMode::BitPerfect {
        s.full_level_ok = false;
    }
    let (changed, file) = with_phone(|p| {
        if s.full_level_ok {
            match p.dac.as_ref() {
                Some(d) => p.consent_dac = Some(d.device.name.clone()),
                None => s.full_level_ok = false,
            }
        } else {
            p.consent_dac = None;
        }
        let changed = p.settings != s;
        p.settings = s;
        (changed, p.settings_file.clone())
    });
    if changed {
        GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    if let (true, Some(f)) = (changed, file) {
        if let Err(e) = std::fs::write(&f, serde_json::to_vec(&s).expect("serializes")) {
            tracing::warn!(path = %f.display(), "cannot save the DAC settings: {e}");
        }
    }
    changed
}

/// Where the settings are kept: read them from there now, save them there from now on.
pub fn keep_settings_in(path: std::path::PathBuf) {
    let saved = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<DeviceSettings>(&b).ok());
    with_phone(|p| {
        if let Some(mut s) = saved {
            // An agreement to full level is for the DAC it was given with: asked again after a start
            s.full_level_ok = false;
            p.settings = s;
        }
        p.settings_file = Some(path);
    });
}

/// What an open got (the bit-perfect player or the shared path records it). When the mode or the
/// confirmation changes (a sink replaced internally, bit-perfect falling back to shared), the
/// node is told at once ([`on_change`]): the UI never keeps claiming what no longer plays.
pub fn record_open(last: LastOpen) {
    let changed = with_phone(|p| {
        let was = p
            .last
            .as_ref()
            .filter(|l| l.dac_id == last.dac_id)
            .map(|l| (l.mode, l.confirmed));
        let changed = was.is_some_and(|w| w != (last.mode, last.confirmed));
        p.last = Some(last);
        changed
    });
    if changed {
        notify_change();
    }
}

/// The open on `dac_id` in `format` is no longer what was confirmed (Android's mixer for the DAC
/// was changed or cleared, the track moved): shared from now on, with why. Tells the app, so the
/// node learns it. Returns whether that changed anything.
pub fn withdraw_confirmation(dac_id: i32, format: pm_model::AudioFormat, why: String) -> bool {
    let changed = with_phone(|p| match p.last.as_mut() {
        Some(l) if l.dac_id == dac_id && l.format == format && l.confirmed => {
            l.confirmed = false;
            l.mode = DeviceMode::Shared;
            l.note = Some(why);
            true
        }
        _ => false,
    });
    if changed {
        notify_change();
    }
    changed
}

/// How many DACs went away so far: a sink opened on one stops when this changes.
pub fn gone_count() -> u64 {
    with_phone(|p| p.gone)
}

/// Keep `dac` as the phone's DAC. Returns whether what the output reports changed (the app then
/// registers its player again, so the node and the UI learn it).
pub fn set_dac(dac: Option<Dac>) -> bool {
    set_dac_because(dac, None)
}

/// [`set_dac`], with why the DAC went when it goes (`None`: it was unplugged).
fn set_dac_because(dac: Option<Dac>, why: Option<String>) -> bool {
    with_phone(|p| {
        let before = view(p);
        let other =
            p.dac.as_ref().map(|d| (d.id, &d.device.name)) != dac.as_ref().map(|d| (d.id, &d.device.name));
        if other && p.dac.is_some() {
            // The DAC an open sink plays on went away (or another took its place)
            p.gone += 1;
            GONE.store(p.gone, std::sync::atomic::Ordering::SeqCst);
            let name = p.dac.as_ref().map(|d| d.device.name.clone()).unwrap_or_default();
            p.gone_why = why.unwrap_or_else(|| format!("{name} was unplugged"));
            let armed = p.sinks_open > 0 || p.last_close.is_some_and(|t| t.elapsed() < UNSEEN_WINDOW);
            if armed {
                p.gone_unseen = Some((p.gone_why.clone(), std::time::Instant::now()));
            }
        }
        if other {
            GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            p.last = None;
            // The next DAC's reason, when it goes, is its own; an earlier unplug is moot now
            if dac.is_some() {
                p.gone_why.clear();
                p.gone_unseen = None;
            }
        }
        p.dac = dac;
        view(p) != before
    })
}

/// A report from the app, kept as the phone's DAC: a report that can't be read (too large,
/// malformed) means no DAC, never the last one seen. Returns whether the DAC changed.
pub fn apply_report(json: &str) -> bool {
    let parsed = parse_report(json);
    let mut media_changed = false;
    if let Ok(r) = parsed.as_ref() {
        let v = r
            .media_volume
            .and_then(|v| u8::try_from(v).ok())
            .filter(|v| *v <= 100);
        // Compared before it is kept: a volume-only report is a change too (the node re-reads
        // the caps: Auto at 100 % plays bit-perfect again, below it shared)
        media_changed = with_phone(|p| std::mem::replace(&mut p.media, v) != v);
    }
    let (dac, why) = match parsed {
        // Android is about to move audio off the DAC: gone now, before its removal arrives
        Ok(r) if r.noisy => (None, None),
        Ok(r) => {
            let dac = dac(&r);
            // The DAC still attached, but media goes elsewhere (Bluetooth, the speaker)
            let old = current_dac();
            let why = old
                .filter(|o| dac.is_none() && r.devices.iter().any(|d| d.id == o.id && is_usb_sink(d)))
                .map(|o| format!("{} is not where Android plays now", o.device.name));
            (dac, why)
        }
        Err(e) => {
            tracing::warn!("USB DAC detection: {e}");
            (None, None)
        }
    };
    tracing::info!(dac = ?dac.as_ref().map(|d| &d.device.summary), "USB audio devices changed");
    set_dac_because(dac, why) || media_changed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests that set the phone's DAC (one process-wide value) run one at a time.
    use crate::PHONE_TESTS as SERIAL;

    /// What UsbAudio.kt reports for a FiiO KA17 on an Android 14 phone (rates and encodings as
    /// Android lists them for the KA17's UAC2 interface).
    const KA17: &str = r#"{"v":1,"sdk":34,"devices":[
        {"id":3,"type":2,"name":"Pixel 8","rates":[48000],"encodings":[2],"channel_counts":[2]},
        {"id":41,"type":11,"name":"FiiO KA17","rates":[44100,48000,88200,96000,176400,192000,352800,384000,705600,768000],
         "encodings":[2,21,22,4],"channel_counts":[2],"routed":true,
         "usb":{"vendor_id":10610,"product_id":147,"manufacturer":"FiiO","product":"FiiO KA17"}}]}"#;

    #[test]
    fn the_ka17_is_described_by_name_with_rates_and_sizes() {
        let d = describe(&parse_report(KA17).unwrap()).unwrap();
        assert_eq!(d.name, "FiiO KA17");
        assert_eq!(d.kind, DeviceKind::UsbDac);
        assert_eq!(d.rates.first(), Some(&44_100));
        assert_eq!((d.max_rate, d.max_bits), (768_000, 32));
        assert_eq!(d.bits, [16, 24, 32]);
        assert_eq!(d.channels, 2);
        assert_eq!(d.summary, "FiiO KA17 · PCM up to 768 kHz / 32 bit");
        assert_eq!(
            d.usb,
            Some(UsbIds {
                vendor_id: 0x2972,
                product_id: 0x0093
            })
        );
        assert_eq!(d.note, None);
    }

    #[test]
    fn it_is_never_called_bit_perfect_while_android_mixes() {
        for sdk in [29, 33, 34, 35] {
            let json = KA17.replace("\"sdk\":34", &format!("\"sdk\":{sdk}"));
            let d = describe(&parse_report(&json).unwrap()).unwrap();
            assert_eq!((d.mode, d.confirmed), (DeviceMode::Shared, false), "sdk {sdk}");
            assert_eq!(d.modes, [DeviceMode::Shared]);
            let text = serde_json::to_string(&d).unwrap();
            assert!(!text.contains("bit_perfect"), "{text}");
        }
        // Below Android 14 the note says why bit-perfect isn't possible
        let d = describe(&parse_report(&KA17.replace("\"sdk\":34", "\"sdk\":33")).unwrap()).unwrap();
        assert_eq!(
            d.note.as_deref(),
            Some("Bit-perfect playback needs Android 14 or later; this phone has Android 13.")
        );
    }

    #[test]
    fn the_wire_shape_is_what_the_ui_reads() {
        let d = describe(&parse_report(KA17).unwrap()).unwrap();
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["kind"], "usb_dac");
        assert_eq!(v["mode"], "shared");
        assert_eq!(v["modes"], serde_json::json!(["shared"]));
        assert_eq!(v["confirmed"], false);
        assert_eq!(v["max_rate"], 768_000);
        assert_eq!(v["usb"]["vendor_id"], 0x2972);
        assert!(v.get("dsd").is_none() && v.get("note").is_none(), "{v}");
        // And it reads back
        let back: OutputDevice = serde_json::from_value(v).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn with_its_descriptors_it_also_tells_dsd_and_hardware_volume() {
        let raw = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../pm-usb-audio/tests/fixtures/fiio-ka17.bin"
        ))
        .unwrap();
        let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        let json = KA17.replace(
            r#""product":"FiiO KA17"}"#,
            &format!(r#""product":"FiiO KA17","descriptors":"{hex}"}}"#),
        );
        let d = describe(&parse_report(&json).unwrap()).unwrap();
        assert_eq!((d.dsd, d.hardware_volume), (Some(DsdKind::Native), true));
        assert_eq!(
            d.summary,
            "FiiO KA17 · PCM up to 768 kHz / 32 bit · DSD · hardware volume"
        );
        // Android listing no rates ("any rate"): the descriptors' clock can't be asked here, so none
        let any = json.replace(
            "[44100,48000,88200,96000,176400,192000,352800,384000,705600,768000]",
            "[]",
        );
        let d = describe(&parse_report(&any).unwrap()).unwrap();
        assert!(d.rates.is_empty() && d.max_rate == 0, "{:?}", d.rates);
        // Descriptors of another device (ids differ) are not used
        let other = json.replace("\"product_id\":147", "\"product_id\":148");
        let d = describe(&parse_report(&other).unwrap()).unwrap();
        assert_eq!((d.dsd, d.hardware_volume), (None, false));
        // Broken hex is ignored, not an error
        let broken = json.replace(&hex[..4], "zz");
        assert!(!describe(&parse_report(&broken).unwrap()).unwrap().hardware_volume);
    }

    #[test]
    fn names_people_know() {
        let dev = |name: &str, maker: Option<&str>, product: Option<&str>| ReportDevice {
            name: name.into(),
            kind: TYPE_USB_DEVICE,
            usb: Some(ReportUsb {
                manufacturer: maker.map(Into::into),
                product: product.map(Into::into),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            display_name(&dev("USB-Audio - KA17", Some("FiiO"), None)),
            "FiiO KA17"
        );
        assert_eq!(display_name(&dev("FiiO KA17", Some("FiiO"), None)), "FiiO KA17");
        assert_eq!(
            display_name(&dev(
                "",
                Some("Apple, Inc."),
                Some("USB-C to 3.5mm Headphone Jack Adapter")
            )),
            "Apple, Inc. USB-C to 3.5mm Headphone Jack Adapter"
        );
        assert_eq!(display_name(&dev("  KA17\u{0}\n ", None, None)), "KA17");
        assert_eq!(display_name(&dev("", None, None)), "USB audio device");
        assert_eq!(
            display_name(&dev(&"x".repeat(500), None, None)).chars().count(),
            MAX_NAME
        );
    }

    #[test]
    fn the_routed_usb_sink_wins_and_no_usb_sink_means_none() {
        let r = parse_report(
            r#"{"sdk":34,"devices":[
            {"id":1,"type":22,"name":"Headset","rates":[48000],"encodings":[2],"routed":false},
            {"id":2,"type":11,"name":"DAC","rates":[96000,44100,96000],"encodings":[21],"routed":true}]}"#,
        )
        .unwrap();
        let d = describe(&r).unwrap();
        assert_eq!(
            (d.name.as_str(), d.rates.as_slice(), d.bits.as_slice()),
            ("DAC", &[44_100, 96_000][..], &[24][..])
        );
        // Android routes media elsewhere (Bluetooth, the speaker): no DAC, though one is attached
        let elsewhere = r#"{"sdk":34,"devices":[
            {"type":22,"name":"Headset","routed":false},{"type":11,"name":"DAC","routed":false}]}"#;
        assert_eq!(describe(&parse_report(elsewhere).unwrap()), None);
        // Without routing information (Android 12): the first USB sink, a headset here
        let r =
            parse_report(r#"{"sdk":31,"devices":[{"type":22,"name":"Headset"},{"type":11,"name":"DAC"}]}"#)
                .unwrap();
        assert_eq!(describe(&r).unwrap().kind, DeviceKind::UsbHeadset);
        // The speaker, wired and Bluetooth headphones are not USB DACs
        let r =
            parse_report(r#"{"sdk":34,"devices":[{"type":2,"name":"Speaker"},{"type":8,"name":"Buds"}]}"#)
                .unwrap();
        assert_eq!(describe(&r), None);
        assert_eq!(
            describe(&parse_report(r#"{"v":1,"sdk":34,"devices":[]}"#).unwrap()),
            None
        );
    }

    #[test]
    fn hostile_reports_are_refused_or_cut_never_a_panic() {
        assert!(parse_report("").is_err());
        assert!(parse_report("not json").is_err());
        assert!(parse_report(r#"{"devices":[{"type":"eleven"}]}"#).is_err());
        assert!(parse_report(&" ".repeat(MAX_REPORT + 1)).is_err());
        // Unknown fields are fine, missing ones default
        let r = parse_report(r#"{"devices":[{"type":11,"future":true}],"later":1}"#).unwrap();
        assert_eq!(describe(&r).unwrap().name, "USB audio device");
        // Long lists are cut; absurd rates dropped
        let rates: Vec<String> = (0..1000).map(|i| (i * 1000).to_string()).collect();
        let json = format!(
            r#"{{"devices":[{{"type":11,"rates":[{},4000000000]}}]}}"#,
            rates.join(",")
        );
        let r = parse_report(&json).unwrap();
        assert_eq!(r.devices[0].rates.len(), MAX_ITEMS);
        let d = describe(&r).unwrap();
        assert!(d.rates.iter().all(|r| (8_000..=1_536_000).contains(r)));
        // A huge channel count saturates
        let r = parse_report(r#"{"devices":[{"type":11,"channel_counts":[4294967295]}]}"#).unwrap();
        assert_eq!(describe(&r).unwrap().channels, 255);
    }

    #[test]
    fn polls() {
        assert_eq!(parse_poll("7\n{\"v\":1}"), Some((7, "{\"v\":1}")));
        assert_eq!(parse_poll("7"), None);
        assert_eq!(parse_poll("x\n{}"), None);
        assert_eq!(parse_poll(""), None);
    }

    #[test]
    fn the_current_dac() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let d = dac(&parse_report(KA17).unwrap());
        set_dac(None);
        assert!(set_dac(d.clone()));
        assert!(!set_dac(d.clone()), "the same DAC again is no change");
        let v = current().unwrap();
        assert_eq!(v.name, "FiiO KA17");
        assert_eq!(
            v.modes,
            [DeviceMode::Shared],
            "no mixer attributes listed: shared only"
        );
        assert_eq!(v.settings, Some(DeviceSettings::default()));
        let g = gone_count();
        assert!(set_dac(None));
        assert_eq!(current(), None);
        assert_eq!(gone_count(), g + 1, "a DAC went away");
    }

    /// The Kotlin side (tools/release/android/UsbAudio.kt) writes every key read here, and nothing
    /// this side would drop: the report's contract, checked without a JVM.
    #[test]
    fn the_kotlin_side_writes_the_keys_read_here() {
        let kt = include_str!("../../../tools/release/android/UsbAudio.kt");
        let written: std::collections::BTreeSet<&str> = kt
            .match_indices(".put(\"")
            .filter_map(|(at, m)| kt[at + m.len()..].split('"').next())
            .collect();
        let read = [
            "v",
            "sdk",
            "devices",
            "id",
            "type",
            "name",
            "rates",
            "encodings",
            "channel_counts",
            "routed",
            "usb",
            "vendor_id",
            "product_id",
            "manufacturer",
            "product",
            "descriptors",
            "mixer",
            "encoding",
            "rate",
            "channel_mask",
            "channels",
            "behavior",
            "noisy",
            "media_volume",
        ];
        assert_eq!(
            written,
            read.into_iter().collect(),
            "UsbAudio.kt writes {written:?}"
        );
        // And a report shaped like the one it builds parses
        let r = parse_report(
            r#"{"v":1,"sdk":35,"devices":[{"id":41,"type":11,"name":"FiiO KA17","rates":[],"encodings":[],
            "channel_counts":[],"routed":true,"usb":{"vendor_id":10610,"product_id":147,"manufacturer":"FiiO",
            "product":"FiiO KA17","descriptors":"12"}}]}"#,
        )
        .unwrap();
        assert_eq!(
            r.devices[0].usb.as_ref().unwrap().descriptors.as_deref(),
            Some("12")
        );
    }

    /// A report that can't be read clears the DAC: the app then shows none rather than the last.
    #[test]
    fn a_report_that_cannot_be_read_means_no_dac() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_dac(None);
        assert!(apply_report(KA17));
        assert_eq!(current().map(|d| d.name), Some("FiiO KA17".into()));
        assert!(!apply_report(KA17), "the same report again is no change");
        // Too large (two DACs with long descriptors), or broken: none, and that is a change
        let huge = format!(r#"{{"v":1,"devices":[],"pad":"{}"}}"#, "x".repeat(MAX_REPORT));
        assert!(apply_report(&huge));
        assert_eq!(current(), None);
        assert!(apply_report(KA17));
        assert!(apply_report("{\"devices\": [ broken"));
        assert_eq!(current(), None);
        assert!(!apply_report("also broken"), "still none: no change");
    }
}
