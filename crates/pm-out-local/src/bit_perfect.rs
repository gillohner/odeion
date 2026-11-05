//! Bit-perfect playback to a phone's USB DAC (Android 14 and later).
//!
//! Android passes a stream to a USB DAC unchanged when the app sets the DAC's mixer to
//! bit-perfect for that exact format (`AudioManager.setPreferredMixerAttributes` with
//! `MIXER_BEHAVIOR_BIT_PERFECT`, for `USAGE_MEDIA`) and plays an `AudioTrack` with the same
//! attributes and the same `AudioFormat`. The app's Kotlin side does that (`BitPerfect.kt`, behind
//! [`TrackBackend`]); this module decides, without a JVM, so it is tested on every host:
//!
//! * [`offer`]: whether the phone and DAC allow it at all, or why not;
//! * [`choose`]: the DAC's mixer entry for a track's format (16 bit as 16; 24 bit packed, else in
//!   32 with the samples left-justified, their values unchanged; 32 as 32), or why not;
//! * [`volume_handover`]: never louder. Android applies no volume of its own in bit-perfect mode,
//!   so Auto plays bit-perfect only while Android's media volume is at full (nothing to lose);
//!   below that it stays shared and says why. Bit-perfect chosen in Settings plays below full
//!   volume only with the listener's agreement to full level, given when they chose it, for that
//!   DAC; without it, shared and why;
//! * [`live_caps`]: what the phone's output takes then (rates and sizes the DAC lists together),
//!   so the node stops converting for it;
//! * [`confirmation`]: confirmed only when Android read the bit-perfect mixer back with exactly
//!   that format and the track plays through the DAC; a track routed elsewhere is not used at all;
//! * [`TrackSink`]: the zone's sink on the track: it withdraws the confirmation when Android's
//!   mixer changes under it, and pauses the zone (via [`SinkError::Gone`]) when the DAC goes.

use crate::usb_dac::{
    self, Dac, LastOpen, MixerEntry, ENCODING_PCM_16BIT, ENCODING_PCM_24BIT_PACKED, ENCODING_PCM_32BIT,
    MIXER_BEHAVIOR_BIT_PERFECT,
};
use pm_model::{AudioBlock, AudioFormat, OutputCaps, QualityCeiling, SampleKind, Samples};
use pm_zones::{AudioSink, BufferLevel, DeviceMode, OutputId, PreferredMode, SinkError};
use std::sync::{Arc, RwLock};

/// Android 14: `setPreferredMixerAttributes`.
pub const API_BIT_PERFECT: u32 = 34;

/// Android's player for a bit-perfect stream: the app's `BitPerfect.kt`, over JNI.
pub trait TrackBackend: Send + Sync {
    /// Set the DAC's mixer to `entry`, bit-perfect, then start an `AudioTrack` with the same
    /// attributes and format, preferring the DAC. What Android said comes back with the track.
    /// `Err`: [`OpenError::RouteLost`] when the DAC is not there (any more), else `Refused`.
    fn open(&self, dac_id: i32, entry: &MixerEntry) -> Result<(Box<dyn Track>, OpenReport), OpenError>;
    /// Android's media volume, 0 to 100 (`None`: it could not be read).
    fn media_volume(&self) -> Option<u8>;
}

/// One `AudioTrack`.
pub trait Track: Send {
    /// Write whole frames in the track's encoding, blocking until they are in.
    fn write(&mut self, bytes: &[u8]) -> Result<(), TrackError>;
    /// Frames the DAC has played (`getPlaybackHeadPosition`, unwrapped).
    fn played_frames(&self) -> u64;
    /// The track's buffer, in frames.
    fn buffer_frames(&self) -> u64;
    fn pause(&mut self);
    fn play(&mut self);
    /// Discard what is queued (paused first).
    fn flush(&mut self);
    /// Stop for good. The app clears the mixer attributes once no track has played for a while.
    fn stop(&mut self);
    /// Still bit-perfect as confirmed at open, or why not any more (Android's mixer for the DAC
    /// was changed or cleared, or the track was moved).
    fn still_confirmed(&self) -> Result<(), String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackError {
    /// The DAC went away (unplugged; Android moved the track elsewhere and the app stopped it).
    Gone,
    Other(String),
}

/// What Android said once the track started.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenReport {
    /// `setPreferredMixerAttributes` returned true.
    pub set_ok: bool,
    /// `getPreferredMixerAttributes` read back (`None`: nothing set).
    pub readback: Option<MixerEntry>,
    /// The device the track plays through (`getRoutedDevice`).
    pub routed_id: Option<i32>,
    pub routed_name: Option<String>,
}

/// `BitPerfect.open`'s answer: the track's handle and buffer, and what Android said; or why it
/// would not start (`gone`: the DAC is not attached: [`OpenError::RouteLost`]).
pub fn parse_open_answer(json: &str) -> Result<(i64, u64, OpenReport), OpenError> {
    #[derive(serde::Deserialize)]
    struct Answer {
        ok: bool,
        #[serde(default)]
        why: Option<String>,
        #[serde(default)]
        handle: i64,
        #[serde(default)]
        buffer_frames: u64,
        #[serde(default)]
        set_ok: bool,
        #[serde(default)]
        readback: Option<MixerEntry>,
        #[serde(default)]
        routed_id: Option<i32>,
        #[serde(default)]
        routed_name: Option<String>,
        #[serde(default)]
        gone: bool,
    }
    let a: Answer = serde_json::from_str(json)
        .map_err(|e| OpenError::Refused(format!("the bit-perfect player's answer: {e}")))?;
    if !a.ok {
        let why = a
            .why
            .unwrap_or_else(|| "the bit-perfect player would not start".into());
        return Err(if a.gone {
            OpenError::RouteLost(why)
        } else {
            OpenError::Refused(why)
        });
    }
    Ok((
        a.handle,
        a.buffer_frames,
        OpenReport {
            set_ok: a.set_ok,
            readback: a.readback,
            routed_id: a.routed_id,
            routed_name: a.routed_name,
        },
    ))
}

static BACKEND: RwLock<Option<Arc<dyn TrackBackend>>> = RwLock::new(None);

/// The app's player (set once at start, on Android 14 and later).
pub fn set_backend(b: Arc<dyn TrackBackend>) {
    *BACKEND.write().unwrap_or_else(|e| e.into_inner()) = Some(b);
}

pub fn backend() -> Option<Arc<dyn TrackBackend>> {
    BACKEND.read().unwrap_or_else(|e| e.into_inner()).clone()
}

fn khz(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        format!("{}", rate / 1000)
    } else {
        format!("{:.1}", f64::from(rate) / 1000.0)
    }
}

fn bit_perfect_entries(dac: &Dac) -> impl Iterator<Item = &MixerEntry> {
    dac.mixer
        .iter()
        .filter(|e| e.behavior == MIXER_BEHAVIOR_BIT_PERFECT)
}

/// Whether the phone and DAC allow bit-perfect at all, or why not.
pub fn offer(dac: &Dac) -> Result<(), String> {
    if dac.sdk != 0 && dac.sdk < API_BIT_PERFECT {
        return Err(format!(
            "Android {}: bit-perfect needs Android 14",
            crate::usb_dac::android_version(dac.sdk)
        ));
    }
    if bit_perfect_entries(dac).next().is_none() {
        return Err("Android offers no bit-perfect mode for this DAC".into());
    }
    if backend().is_none() {
        return Err("this app has no bit-perfect player here".into());
    }
    Ok(())
}

/// The mixer entry a track's format plays bit-perfect with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MixerChoice {
    pub entry: MixerEntry,
}

fn takes(e: &MixerEntry, bits: u8) -> bool {
    matches!(
        (bits, e.encoding),
        (16, ENCODING_PCM_16BIT)
            | (24, ENCODING_PCM_24BIT_PACKED)
            | (24, ENCODING_PCM_32BIT)
            | (32, ENCODING_PCM_32BIT)
    )
}

/// The DAC's bit-perfect entry for `fmt`, or why there is none.
pub fn choose(dac: &Dac, fmt: AudioFormat) -> Result<MixerChoice, String> {
    offer(dac)?;
    if fmt.kind != SampleKind::Pcm {
        return Err("DSD plays through Android's mixer".into());
    }
    let at_rate: Vec<&MixerEntry> = bit_perfect_entries(dac)
        .filter(|e| e.rate == fmt.rate_hz)
        .collect();
    if at_rate.is_empty() {
        return Err(format!(
            "the DAC does not list {} kHz for bit-perfect",
            khz(fmt.rate_hz)
        ));
    }
    let with_channels: Vec<&MixerEntry> = at_rate
        .into_iter()
        .filter(|e| e.channels == fmt.channels)
        .collect();
    if with_channels.is_empty() {
        return Err(format!(
            "the DAC does not take {} channel{} bit-perfect",
            fmt.channels,
            if fmt.channels == 1 { "" } else { "s" }
        ));
    }
    // 24 bit: packed where the DAC takes it, else in 32 bit with the values unchanged
    let order: &[i32] = match fmt.bits {
        16 => &[ENCODING_PCM_16BIT],
        24 => &[ENCODING_PCM_24BIT_PACKED, ENCODING_PCM_32BIT],
        32 => &[ENCODING_PCM_32BIT],
        _ => &[],
    };
    order
        .iter()
        .find_map(|enc| with_channels.iter().find(|e| e.encoding == *enc))
        .map(|e| MixerChoice { entry: **e })
        .ok_or_else(|| format!("the DAC does not take {}-bit bit-perfect", fmt.bits))
}

/// Whether bit-perfect may start without the music getting louder. Android applies no volume of
/// its own in bit-perfect mode: the level is the DAC's (or the amplifier's). So it plays
/// bit-perfect only while Android's media volume is at full, where nothing is lost, or, for
/// Bit-perfect chosen in Settings, when the listener agreed to full level for this DAC
/// (`full_level_ok`; then `Ok(Some(warning))`). Otherwise it stays shared, and `Err` says why.
pub fn volume_handover(
    mode: PreferredMode,
    media_percent: Option<u8>,
    full_level_ok: bool,
) -> Result<Option<String>, String> {
    if media_percent == Some(100) {
        return Ok(None);
    }
    let at = media_percent.map(|p| format!(" ({p}%)")).unwrap_or_default();
    match (mode, full_level_ok) {
        (PreferredMode::BitPerfect, true) => Ok(Some(format!(
            "Bit-perfect plays at full level: Android's volume{at} does not apply. Set the level on the DAC or amplifier"
        ))),
        (PreferredMode::BitPerfect, false) => Err(format!(
            "Bit-perfect plays at full level, and Android's volume{at} would no longer apply: confirm Bit-perfect in Settings to use it"
        )),
        _ => Err(match media_percent {
            Some(p) => format!(
                "Android's volume is at {p}%, and bit-perfect plays at full level: turn Android's volume up to full and set the level on the DAC, or choose Bit-perfect in Settings"
            ),
            None => "Android's volume could not be read, and bit-perfect plays at full level".into(),
        }),
    }
}

/// **The one gate for rule (c), "never louder without consent":** whether bit-perfect (full
/// level) may be entered or resumed on the phone's DAC now: [`volume_handover`] on the live
/// settings, Android's media volume now and the consent recorded for this DAC. Every open, resume,
/// re-start after a flush (Android's mixer set again after an idle clear) and same-format next
/// song asks here; `Err` is why it plays shared instead.
pub fn entry_gate() -> Result<Option<String>, String> {
    let mode = usb_dac::settings().mode;
    if mode == PreferredMode::Shared {
        return Err("shared, as chosen in Settings".into());
    }
    let media = backend().and_then(|b| b.media_volume());
    volume_handover(mode, media, usb_dac::full_level_consent())
}

/// The rates and sizes the DAC lists together, in stereo: the largest set of sizes every one of
/// whose rates the DAC takes with every one of those sizes (a node plans rate and size apart, so it
/// must never pick a pair the DAC doesn't list). More rates first, then the larger size.
pub fn rectangle(dac: &Dac) -> (Vec<u32>, Vec<u8>) {
    let stereo: Vec<&MixerEntry> = bit_perfect_entries(dac).filter(|e| e.channels == 2).collect();
    let mut rates: Vec<u32> = stereo.iter().map(|e| e.rate).collect();
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
                    .all(|b| stereo.iter().any(|e| e.rate == *rate && takes(e, *b)))
            })
            .collect();
        let better = r.len() > best.0.len()
            || (r.len() == best.0.len()
                && !r.is_empty()
                && (set.last() > best.1.last() || (set.last() == best.1.last() && set.len() > best.1.len())));
        if better {
            best = (r, set.to_vec());
        }
    }
    best
}

/// What the phone's output takes now: the rates and sizes the DAC lists together for bit-perfect,
/// when bit-perfect would be used (offered, not turned off in Settings, and not louder than now:
/// [`volume_handover`]); else `shared`. Bit-perfect has no volume of its own here: the level is
/// the DAC's.
pub fn live_caps(
    dac: Option<&Dac>,
    settings: pm_zones::DeviceSettings,
    full_level_ok: bool,
    media_percent: Option<u8>,
    shared: &OutputCaps,
) -> OutputCaps {
    let Some(dac) = dac.filter(|d| plays_bit_perfect(d, settings, full_level_ok, media_percent)) else {
        return shared.clone();
    };
    let (rates, bits) = rectangle(dac);
    if rates.is_empty() {
        return shared.clone();
    }
    OutputCaps {
        max_pcm_bits: bits.last().copied().unwrap_or(16),
        pcm_rates: rates,
        pcm_bits: bits,
        max_channels: 2,
        dsd: None,
        hw_volume: None,
        quality_ceiling: QualityCeiling::BitPerfect,
        ..shared.clone()
    }
}

/// Whether the phone plays this DAC bit-perfect now (offered, not Shared in Settings, never louder).
fn plays_bit_perfect(
    dac: &Dac,
    settings: pm_zones::DeviceSettings,
    full_level_ok: bool,
    media: Option<u8>,
) -> bool {
    offer(dac).is_ok()
        && settings.mode != PreferredMode::Shared
        && volume_handover(settings.mode, media, full_level_ok).is_ok()
}

/// Every format the DAC takes bit-perfect: each listed entry's rate and channels, with each sample
/// size it carries (24 bit also in a 32-bit entry, left-justified).
pub fn formats(dac: &Dac) -> Vec<pm_zones::DeviceFormat> {
    let mut v: Vec<pm_zones::DeviceFormat> = Vec::new();
    for e in bit_perfect_entries(dac) {
        for bits in [16u8, 24, 32] {
            let f = pm_zones::DeviceFormat {
                rate_hz: e.rate,
                bits,
                channels: e.channels,
            };
            if takes(e, bits) && !v.contains(&f) {
                v.push(f);
            }
        }
    }
    v
}

/// For the device the phone reports: the exact formats it plays bit-perfect now (none when it
/// plays shared), and Android's media volume.
pub fn view_extras(
    dac: &Dac,
    settings: pm_zones::DeviceSettings,
    full_level_ok: bool,
) -> (Vec<pm_zones::DeviceFormat>, Option<u8>) {
    let media = backend().and_then(|b| b.media_volume());
    let f = if crate::exclusive::plays_exclusive(dac, settings, full_level_ok, media) {
        crate::exclusive::formats(dac)
    } else if plays_bit_perfect(dac, settings, full_level_ok, media) {
        formats(dac)
    } else {
        Vec::new()
    };
    (f, media)
}

/// [`live_caps`] for the phone's output now (or Exclusive USB's, when it plays that way), with
/// the formats the DAC then takes unchanged.
fn phone_caps_and_formats(shared: &OutputCaps) -> (OutputCaps, Vec<pm_zones::DeviceFormat>) {
    let dac = usb_dac::current_dac();
    let media = dac.as_ref().and(backend()).and_then(|b| b.media_volume());
    let settings = usb_dac::settings();
    let consent = usb_dac::full_level_consent();
    if let Some(d) = dac.as_ref() {
        if let Some(c) = crate::exclusive::live_caps(d, settings, consent, media, shared) {
            return (c, crate::exclusive::formats(d));
        }
    }
    let caps = live_caps(dac.as_ref(), settings, consent, media, shared);
    let f = match dac.as_ref() {
        Some(d) if caps.quality_ceiling == QualityCeiling::BitPerfect => formats(d),
        _ => Vec::new(),
    };
    (caps, f)
}

/// [`live_caps`] for the phone's output now.
pub fn phone_caps(shared: &OutputCaps) -> OutputCaps {
    phone_caps_and_formats(shared).0
}

/// [`phone_caps`] for one source: its own format when the DAC lists it (unchanged), else the
/// closest listed one.
pub fn phone_caps_for(shared: &OutputCaps, src: AudioFormat) -> OutputCaps {
    let (caps, f) = phone_caps_and_formats(shared);
    if caps.quality_ceiling != QualityCeiling::BitPerfect || f.is_empty() {
        return caps;
    }
    pm_zones::caps_for_source(&f, src, &caps)
}

/// Confirmed only when Android read the bit-perfect mixer back with exactly the format asked for
/// and the track plays through the DAC; otherwise exactly why not.
pub fn confirmation(chosen: &MixerEntry, report: &OpenReport, dac_id: i32) -> Result<(), String> {
    let Some(rb) = report.readback.filter(|_| report.set_ok) else {
        return Err("Android refused bit-perfect for this format".into());
    };
    if rb.behavior != MIXER_BEHAVIOR_BIT_PERFECT
        || (rb.encoding, rb.rate, rb.channel_mask) != (chosen.encoding, chosen.rate, chosen.channel_mask)
    {
        return Err("Android set its mixer to another format: not bit-perfect".into());
    }
    match report.routed_id {
        Some(id) if id == dac_id => Ok(()),
        Some(_) => Err(format!(
            "playing through {}, not the DAC",
            report.routed_name.as_deref().unwrap_or("another device")
        )),
        None => Err("Android did not say it plays through the DAC".into()),
    }
}

/// How the phone's output opens a format.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// Through the bit-perfect player, on this DAC with this entry; with a warning to keep with it.
    BitPerfect(Dac, MixerChoice, Option<String>),
    /// Through Android's mixer; with a DAC, why not bit-perfect.
    Shared(Option<(Dac, String)>),
}

/// How the phone's output opens `fmt` now.
pub fn plan(fmt: AudioFormat) -> Plan {
    let Some(dac) = usb_dac::current_dac() else {
        return Plan::Shared(None);
    };
    let mode = usb_dac::settings().mode;
    if mode == PreferredMode::Shared {
        return Plan::Shared(Some((dac, "shared, as chosen in Settings".into())));
    }
    let choice = match choose(&dac, fmt) {
        Ok(c) => c,
        Err(why) => return Plan::Shared(Some((dac, why))),
    };
    match entry_gate() {
        Ok(warning) => Plan::BitPerfect(dac, choice, warning),
        Err(why) => Plan::Shared(Some((dac, why))),
    }
}

/// Record a shared open on `dac`, with why it is not bit-perfect.
pub fn record_shared(dac: &Dac, fmt: AudioFormat, why: String) {
    usb_dac::record_open(LastOpen {
        dac_id: dac.id,
        format: fmt,
        mode: DeviceMode::Shared,
        confirmed: false,
        note: Some(why),
    });
}

/// Why the bit-perfect player gave no track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// The player would not start (another app holds the DAC, a format Android won't play): the
    /// phone plays shared, through the same DAC.
    Refused(String),
    /// Android plays the track anywhere but the DAC (it went, or is going, or media moved): nothing
    /// may open elsewhere, the zone pauses.
    RouteLost(String),
}

impl OpenError {
    pub fn why(&self) -> &str {
        match self {
            OpenError::Refused(w) | OpenError::RouteLost(w) => w,
        }
    }
}

/// What the phone's output opens for a format.
pub enum PhoneOpen {
    /// The bit-perfect player.
    BitPerfect(TrackSink),
    /// The app's own USB driver (Exclusive USB).
    Exclusive(crate::exclusive::ExclusiveSink),
    /// The DAC can't take this format unchanged in Exclusive USB: the zone plans another.
    Unsupported(String),
    /// Exclusive USB holds the DAC but would not start: nothing plays (never another device).
    Failed(String),
    /// Android's shared path; `any_rate`: at the format's own rate even if the shared caps don't
    /// list it (Android converts), because it was planned for the DAC; `opened_for`: the DAC it
    /// is for (the sink stops, never plays elsewhere, once that DAC isn't Android's route).
    Shared {
        any_rate: bool,
        opened_for: Option<usb_dac::OpenedFor>,
    },
    /// The DAC went, or Android plays elsewhere: the zone pauses, nothing opens (no speaker).
    Gone(String),
}

/// The exclusive driver's open, as the phone's output opens it; `Err`: Android plays it (why).
fn open_exclusive(
    id: OutputId,
    dac: &Dac,
    fmt: AudioFormat,
    warning: Option<String>,
) -> Result<PhoneOpen, String> {
    use crate::exclusive::ExclusiveError as E;
    match crate::exclusive::open(id, dac, fmt, warning) {
        Ok(s) => Ok(PhoneOpen::Exclusive(s)),
        Err(E::Gone(why)) => {
            usb_dac::gone_seen();
            Ok(PhoneOpen::Gone(why))
        }
        Err(E::Unsupported(why)) => Ok(PhoneOpen::Unsupported(why)),
        Err(E::Failed(why)) => {
            tracing::warn!(format = %fmt, "Exclusive USB: {why}");
            usb_dac::record_open(LastOpen {
                dac_id: dac.id,
                format: fmt,
                mode: DeviceMode::Exclusive,
                confirmed: false,
                note: Some(why.clone()),
            });
            Ok(PhoneOpen::Failed(why))
        }
        Err(E::Refused(why)) => Err(why),
    }
}

/// How long a DAC the app gave back may take to reappear in Android's list.
const GIVE_BACK_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// The DAC Android plays again, after the app gave it back: `None` while it doesn't (yet).
fn dac_back_from_exclusive() -> Option<Dac> {
    let until = std::time::Instant::now() + GIVE_BACK_WAIT;
    loop {
        match usb_dac::current_dac() {
            // Still the app's, or given back and not listed by Android again yet
            Some(d) if d.held => {}
            other => return other,
        }
        if std::time::Instant::now() >= until {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// How the phone's output opens `fmt` now: the whole decision, apart from cpal. The DAC it is
/// for is fixed first: nothing opened for it may reach another device (rule (b), [`usb_dac::may_reach`]).
pub fn open_phone(id: OutputId, fmt: AudioFormat) -> PhoneOpen {
    let mut at_open = usb_dac::opened_for_now();
    // An unplug no sink reported yet (between one song's sink and the next): pause, once
    if at_open.is_none() {
        if let Some(why) = usb_dac::take_gone_unseen() {
            return PhoneOpen::Gone(why);
        }
    }
    let gone = |why: String| {
        usb_dac::gone_seen();
        PhoneOpen::Gone(why)
    };
    let mut why_not = None;
    if let Some(dac) = usb_dac::current_dac() {
        let media = backend().and_then(|b| b.media_volume());
        // Exclusive USB when chosen; when chosen but not possible, why (Android plays it)
        why_not =
            match crate::exclusive::decide(&dac, usb_dac::settings(), usb_dac::full_level_consent(), media) {
                Some(Ok(warning)) => match open_exclusive(id.clone(), &dac, fmt, warning) {
                    Ok(o) => return o,
                    Err(why) => {
                        tracing::warn!(format = %fmt, "no Exclusive USB: {why}");
                        Some((dac.clone(), why))
                    }
                },
                Some(Err(why)) => Some((dac.clone(), why)),
                None => None,
            };
        // Android's paths, once Android has the DAC back (never while the app holds it); Android
        // lists it under a new id then: the DAC this open is for is fixed again
        crate::exclusive::release();
        if dac.held || usb_dac::current_dac().is_some_and(|d| d.held) {
            if dac_back_from_exclusive().is_none() {
                return gone(format!(
                    "{} is going back to Android: press Play again in a moment",
                    dac.device.name
                ));
            }
            at_open = usb_dac::opened_for_now();
        }
    }
    let shared = |any_rate| PhoneOpen::Shared {
        any_rate,
        opened_for: at_open.clone(),
    };
    if let Some((before, why)) = why_not {
        let dac = usb_dac::current_dac().unwrap_or(before);
        record_shared(&dac, fmt, why);
        return shared(true);
    }
    match plan(fmt) {
        Plan::BitPerfect(dac, choice, warning) => match open(id.clone(), &dac, choice, warning, fmt) {
            Ok(s) => PhoneOpen::BitPerfect(s),
            Err(OpenError::RouteLost(why)) => {
                tracing::warn!(format = %fmt, "the DAC is not where Android plays: {why}");
                record_shared(&dac, fmt, why.clone());
                gone(format!(
                    "{} is not where Android plays now ({why})",
                    dac.device.name
                ))
            }
            Err(OpenError::Refused(why)) => {
                // Refused for a DAC that went meanwhile: never the default route
                if let Err(lost) = usb_dac::may_reach(at_open.as_ref()) {
                    return gone(lost);
                }
                tracing::warn!(format = %fmt, "no bit-perfect player: {why}");
                // Auto: Exclusive USB instead, where permission was given before (never asks)
                if usb_dac::settings().mode == PreferredMode::Auto {
                    let media = backend().and_then(|b| b.media_volume());
                    if let Some(warning) = crate::exclusive::auto_fallback(&dac, media) {
                        if let Ok(o) = open_exclusive(id, &dac, fmt, warning) {
                            return o;
                        }
                    }
                }
                record_shared(&dac, fmt, why);
                shared(true)
            }
        },
        // Planned for the DAC's caps but played shared (a mono track, Settings changed since):
        // Android's mixer takes the track's rate, never a stopped zone
        Plan::Shared(Some((dac, why))) => {
            record_shared(&dac, fmt, why);
            shared(true)
        }
        // No DAC now; one at the start of this open went meanwhile: pause
        Plan::Shared(None) => match usb_dac::may_reach(at_open.as_ref()) {
            Ok(()) => shared(false),
            Err(lost) => gone(lost),
        },
    }
}

/// Open the bit-perfect player for `fmt` and record what Android confirmed. `Err`: no track (why,
/// and whether the DAC is still where Android plays).
pub fn open(
    id: OutputId,
    dac: &Dac,
    choice: MixerChoice,
    warning: Option<String>,
    fmt: AudioFormat,
) -> Result<TrackSink, OpenError> {
    let b = backend().ok_or_else(|| OpenError::Refused("this app has no bit-perfect player here".into()))?;
    let opened_for = usb_dac::OpenedFor {
        dac_id: dac.id,
        name: dac.device.name.clone(),
        gone_at: usb_dac::gone_count(),
    };
    let generation = usb_dac::generation();
    let (mut track, report) = b.open(dac.id, &choice.entry)?;
    let confirmed = confirmation(&choice.entry, &report, dac.id);
    tracing::info!(format = %fmt, ?confirmed, "bit-perfect player");
    // A track Android plays anywhere but the DAC is not used at all: the caller plays shared,
    // where Android routes media (no sample of this track reaches another device)
    if report.routed_id != Some(dac.id) {
        track.stop();
        return Err(OpenError::RouteLost(confirmed.err().unwrap_or_else(|| {
            "Android did not say it plays through the DAC".into()
        })));
    }
    let confirmed_ok = confirmed.is_ok();
    usb_dac::record_open(LastOpen {
        dac_id: dac.id,
        format: fmt,
        // The track plays either way; unconfirmed, Android may convert it: shared, and why
        mode: if confirmed_ok {
            DeviceMode::BitPerfect
        } else {
            DeviceMode::Shared
        },
        confirmed: confirmed_ok,
        note: confirmed.err().or(warning),
    });
    Ok(TrackSink {
        id,
        fmt,
        encoding: choice.entry.encoding,
        track,
        dac: dac.device.name.clone(),
        dac_id: dac.id,
        confirmed: confirmed_ok,
        // The first write checks at once
        checked: std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(1))
            .unwrap_or_else(std::time::Instant::now),
        written: 0,
        base_written: 0,
        base_played: 0,
        _in_use: usb_dac::DacUse::new(Some(&opened_for)),
        opened_for,
        entry_checked: std::sync::Mutex::new(None),
        generation,
        paused: false,
        bytes: Vec::new(),
    })
}

/// `samples` (right-aligned `bits`-bit values) in `encoding`, little-endian, values unchanged.
pub fn encode(bits: u8, encoding: i32, samples: &[i32], out: &mut Vec<u8>) {
    out.clear();
    match encoding {
        ENCODING_PCM_16BIT => samples
            .iter()
            .for_each(|s| out.extend_from_slice(&(*s as i16).to_le_bytes())),
        ENCODING_PCM_24BIT_PACKED => samples
            .iter()
            .for_each(|s| out.extend_from_slice(&s.to_le_bytes()[..3])),
        // 32 bit: 24-bit values left-justified (the low byte zero), 32-bit as they are
        _ => {
            let shift = 32u32.saturating_sub(u32::from(bits));
            samples
                .iter()
                .for_each(|s| out.extend_from_slice(&s.wrapping_shl(shift).to_le_bytes()))
        }
    }
}

/// The zone's sink on the bit-perfect player.
pub struct TrackSink {
    id: OutputId,
    fmt: AudioFormat,
    encoding: i32,
    track: Box<dyn Track>,
    /// The DAC's name and Android's id for it.
    dac: String,
    dac_id: i32,
    /// Confirmed bit-perfect, as far as the last check knows.
    confirmed: bool,
    checked: std::time::Instant,
    written: u64,
    /// `written` and the track's played frames at the last flush.
    base_written: u64,
    base_played: u64,
    /// The DAC it was opened for: every write, drain, flush and resume asks [`usb_dac::may_reach`].
    opened_for: usb_dac::OpenedFor,
    _in_use: usb_dac::DacUse,
    /// [`entry_gate`] at most once a second for [`AudioSink::reopen_at_track`] (asked often).
    entry_checked: std::sync::Mutex<Option<(std::time::Instant, bool)>>,
    /// The settings-or-DAC generation at open: a change closes the sink at the next song.
    generation: u64,
    /// Paused (by a pause or a flush): it plays again with the next write or a resume.
    paused: bool,
    bytes: Vec<u8>,
}

impl TrackSink {
    /// Rule (b): its DAC still Android's route, none gone since; else Gone (and the zone knows).
    fn gone(&self) -> Option<SinkError> {
        let why = usb_dac::may_reach(Some(&self.opened_for)).err()?;
        usb_dac::gone_seen();
        Some(SinkError::Gone(self.id.clone(), why))
    }

    fn gone_error(&self) -> SinkError {
        // The reason the app gave when it saw this DAC go; the track saying so first: unplugged
        self.gone().unwrap_or_else(|| {
            usb_dac::gone_seen();
            SinkError::Gone(self.id.clone(), format!("{} was unplugged", self.dac))
        })
    }

    /// Rule (c), cached for a second: may this sink carry bit-perfect into the next song?
    fn entry_allowed(&self) -> bool {
        let mut c = self.entry_checked.lock().unwrap_or_else(|e| e.into_inner());
        match *c {
            Some((at, ok)) if at.elapsed() < std::time::Duration::from_secs(1) => ok,
            _ => {
                let ok = entry_gate().is_ok();
                *c = Some((std::time::Instant::now(), ok));
                ok
            }
        }
    }

    /// Once a second while confirmed: is it still? If not, the device says shared, and why.
    fn recheck(&mut self) {
        if !self.confirmed || self.checked.elapsed() < std::time::Duration::from_secs(1) {
            return;
        }
        self.checked = std::time::Instant::now();
        if let Err(why) = self.track.still_confirmed() {
            self.confirmed = false;
            tracing::warn!(format = %self.fmt, "no longer bit-perfect: {why}");
            usb_dac::withdraw_confirmation(self.dac_id, self.fmt, why);
        }
    }

    fn queued(&self) -> u64 {
        (self.written - self.base_written)
            .saturating_sub(self.track.played_frames().saturating_sub(self.base_played))
    }
}

impl AudioSink for TrackSink {
    fn format(&self) -> AudioFormat {
        self.fmt
    }

    fn write(&mut self, block: &AudioBlock) -> Result<(), SinkError> {
        pm_zones::sink::check_block(self.fmt, block)?;
        if let Some(e) = self.gone() {
            return Err(e);
        }
        let Samples::PcmI32(samples) = &block.samples else {
            return Err(SinkError::FloatNotAccepted);
        };
        if self.paused {
            // Starting again (after a flush, an idle clear): never into full level without consent.
            // Callers reopen first ([`AudioSink::reopen_now`]); one that didn't is paused here
            if let Err(why) = entry_gate() {
                usb_dac::notify_change();
                return Err(SinkError::Gone(self.id.clone(), why));
            }
            self.track.play();
            self.paused = false;
        }
        self.recheck();
        let mut bytes = std::mem::take(&mut self.bytes);
        encode(self.fmt.bits, self.encoding, samples, &mut bytes);
        let r = self.track.write(&bytes);
        self.bytes = bytes;
        match r {
            Ok(()) => {
                self.written += block.frames() as u64;
                Ok(())
            }
            Err(TrackError::Gone) => Err(self.gone_error()),
            Err(TrackError::Other(e)) => Err(SinkError::Device(e)),
        }
    }

    fn buffer_level(&self) -> BufferLevel {
        BufferLevel {
            queued_frames: self.queued(),
            capacity_frames: self.track.buffer_frames(),
        }
    }

    /// Discard the queue; the track stays paused until the next write (a paused zone keeps no
    /// track playing, so the app can clear the mixer after a while).
    fn flush(&mut self) -> Result<(), SinkError> {
        self.track.pause();
        self.track.flush();
        self.paused = true;
        self.base_written = self.written;
        self.base_played = self.track.played_frames();
        // Emptied, and its DAC gone: said, so a song boundary pauses instead of opening elsewhere
        match self.gone() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn drain(&mut self) -> Result<(), SinkError> {
        // Asked before and after, also when nothing is queued (an unplug at a song boundary)
        if let Some(e) = self.gone() {
            return Err(e);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while self.queued() > 0 && std::time::Instant::now() < deadline {
            if let Some(e) = self.gone() {
                return Err(e);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        match self.gone() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// The track keeps its buffer: the same audio carries on after the pause.
    fn pause(&mut self) -> Result<bool, SinkError> {
        self.track.pause();
        self.paused = true;
        Ok(true)
    }

    fn resume(&mut self) -> Result<(), SinkError> {
        self.ready_to_resume()?;
        self.track.play();
        self.paused = false;
        Ok(())
    }

    /// Rule (b), then rule (c): its DAC still there, and full level still allowed now (Android's
    /// volume may have gone down during the pause). Not allowed: an error that opens the output
    /// again, planned as it is now (shared).
    fn ready_to_resume(&mut self) -> Result<(), SinkError> {
        if let Some(e) = self.gone() {
            return Err(e);
        }
        entry_gate()
            .map(|_| ())
            .map_err(|why| SinkError::Unavailable(self.id.clone(), why))
    }

    fn frames_written(&self) -> u64 {
        self.written
    }

    /// The settings or the DAC changed since it opened, or full level is no longer allowed (rule
    /// (c): Android's volume went down): the next song opens afresh, planned as it is now.
    fn reopen_at_track(&self) -> bool {
        usb_dac::generation() != self.generation || !self.entry_allowed()
    }

    /// Paused (by a pause or a flush) with full level no longer allowed: the next write would set
    /// Android's mixer bit-perfect again. The caller opens the output again instead (shared).
    fn reopen_now(&self) -> bool {
        self.paused && entry_gate().is_err()
    }
}

impl Drop for TrackSink {
    fn drop(&mut self) {
        self.track.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usb_dac::{
        ENCODING_PCM_16BIT as P16, ENCODING_PCM_24BIT_PACKED as P24, ENCODING_PCM_32BIT as P32,
    };
    use pm_zones::{DeviceSettings, OutputDevice};
    use std::sync::Mutex;

    /// The tests that touch the process-wide DAC and player run one at a time (with usb_dac's).
    use crate::PHONE_TESTS as SERIAL;

    const STEREO: i32 = 12; // AudioFormat.CHANNEL_OUT_STEREO

    fn bp(encoding: i32, rate: u32) -> MixerEntry {
        MixerEntry {
            encoding,
            rate,
            channel_mask: STEREO,
            channels: 2,
            behavior: MIXER_BEHAVIOR_BIT_PERFECT,
        }
    }

    /// What a KA17-like DAC lists on Android 14: bit-perfect 16/24-packed/32 at the usual rates,
    /// plus the default (mixing) behavior entries Android also lists.
    fn ka17_mixer() -> Vec<MixerEntry> {
        let mut v = Vec::new();
        for rate in [44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000] {
            for enc in [P16, P24, P32] {
                v.push(bp(enc, rate));
                v.push(MixerEntry {
                    behavior: 0,
                    ..bp(enc, rate)
                });
            }
        }
        v
    }

    fn dac(sdk: u32, mixer: Vec<MixerEntry>) -> Dac {
        Dac {
            device: OutputDevice {
                name: "FiiO KA17".into(),
                ..Default::default()
            },
            id: 41,
            mixer,
            sdk,
            descriptors: None,
            held: false,
        }
    }

    /// A player that records what it was asked and answers as told.
    #[derive(Default)]
    struct Fake {
        report: Mutex<OpenReport>,
        refuse: Mutex<Option<String>>,
        /// The DAC goes while the player opens: the app sees it unplugged (typed) or only refuses
        unplug_on_open: Mutex<Option<bool>>,
        opened: Mutex<Vec<MixerEntry>>,
        bytes: Arc<Mutex<Vec<u8>>>,
        played: Arc<std::sync::atomic::AtomicU64>,
        gone: Arc<std::sync::atomic::AtomicBool>,
        /// Android's media volume (100 unless a test says otherwise)
        volume: Mutex<Option<u8>>,
        /// Why the track is no longer bit-perfect (Android's mixer changed under it)
        unconfirmed: Arc<Mutex<Option<String>>>,
        stopped: Arc<std::sync::atomic::AtomicU64>,
        plays: Arc<std::sync::atomic::AtomicU64>,
    }

    struct FakeTrack {
        bytes: Arc<Mutex<Vec<u8>>>,
        played: Arc<std::sync::atomic::AtomicU64>,
        gone: Arc<std::sync::atomic::AtomicBool>,
        unconfirmed: Arc<Mutex<Option<String>>>,
        stopped: Arc<std::sync::atomic::AtomicU64>,
        plays: Arc<std::sync::atomic::AtomicU64>,
    }

    impl Track for FakeTrack {
        fn write(&mut self, bytes: &[u8]) -> Result<(), TrackError> {
            if self.gone.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(TrackError::Gone);
            }
            self.bytes.lock().unwrap().extend_from_slice(bytes);
            Ok(())
        }
        fn played_frames(&self) -> u64 {
            self.played.load(std::sync::atomic::Ordering::SeqCst)
        }
        fn buffer_frames(&self) -> u64 {
            9600
        }
        fn pause(&mut self) {}
        fn play(&mut self) {
            self.plays.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        fn flush(&mut self) {}
        fn stop(&mut self) {
            self.stopped.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        fn still_confirmed(&self) -> Result<(), String> {
            match self.unconfirmed.lock().unwrap().clone() {
                Some(why) => Err(why),
                None => Ok(()),
            }
        }
    }

    impl TrackBackend for Fake {
        fn open(&self, _dac_id: i32, entry: &MixerEntry) -> Result<(Box<dyn Track>, OpenReport), OpenError> {
            if let Some(typed) = *self.unplug_on_open.lock().unwrap() {
                usb_dac::apply_report(r#"{"v":1,"sdk":34,"devices":[]}"#);
                return Err(if typed {
                    OpenError::RouteLost("the DAC is not attached".into())
                } else {
                    OpenError::Refused(
                        "another app holds the DAC, or Android would not play this format".into(),
                    )
                });
            }
            if let Some(why) = self.refuse.lock().unwrap().clone() {
                return Err(OpenError::Refused(why));
            }
            self.opened.lock().unwrap().push(*entry);
            Ok((
                Box::new(FakeTrack {
                    bytes: self.bytes.clone(),
                    played: self.played.clone(),
                    gone: self.gone.clone(),
                    unconfirmed: self.unconfirmed.clone(),
                    stopped: self.stopped.clone(),
                    plays: self.plays.clone(),
                }),
                self.report.lock().unwrap().clone(),
            ))
        }
        fn media_volume(&self) -> Option<u8> {
            *self.volume.lock().unwrap()
        }
    }

    /// Android's answer when all went well: the mixer read back, the track on the DAC.
    fn confirmed_report(e: MixerEntry) -> OpenReport {
        OpenReport {
            set_ok: true,
            readback: Some(e),
            routed_id: Some(41),
            routed_name: Some("FiiO KA17".into()),
        }
    }

    fn with_player() -> Arc<Fake> {
        let f = Arc::new(Fake::default());
        *f.volume.lock().unwrap() = Some(100);
        set_backend(f.clone());
        f
    }

    #[test]
    fn the_mixer_entry_for_each_format() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        with_player();
        let ka17 = dac(34, ka17_mixer());
        assert_eq!(
            choose(&ka17, AudioFormat::pcm(44_100, 16, 2)).unwrap().entry,
            bp(P16, 44_100)
        );
        assert_eq!(
            choose(&ka17, AudioFormat::pcm(96_000, 24, 2)).unwrap().entry,
            bp(P24, 96_000)
        );
        assert_eq!(
            choose(&ka17, AudioFormat::pcm(192_000, 24, 2)).unwrap().entry,
            bp(P24, 192_000)
        );
        assert_eq!(
            choose(&ka17, AudioFormat::pcm(384_000, 32, 2)).unwrap().entry,
            bp(P32, 384_000)
        );
        // A DAC without packed 24: 24 bit in 32, values unchanged
        let no_packed = dac(
            34,
            ka17_mixer().into_iter().filter(|e| e.encoding != P24).collect(),
        );
        assert_eq!(
            choose(&no_packed, AudioFormat::pcm(96_000, 24, 2)).unwrap().entry,
            bp(P32, 96_000)
        );
        // Why not, exactly
        assert_eq!(
            choose(&ka17, AudioFormat::pcm(705_600, 24, 2)).unwrap_err(),
            "the DAC does not list 705.6 kHz for bit-perfect"
        );
        assert_eq!(
            choose(&ka17, AudioFormat::pcm(96_000, 24, 1)).unwrap_err(),
            "the DAC does not take 1 channel bit-perfect"
        );
        assert_eq!(
            choose(&ka17, AudioFormat::dsd(2_822_400, 2)).unwrap_err(),
            "DSD plays through Android's mixer"
        );
        // A phone that lists only 48 kHz, 16 bit
        let phone48 = dac(34, vec![bp(P16, 48_000)]);
        assert!(choose(&phone48, AudioFormat::pcm(48_000, 16, 2)).is_ok());
        assert_eq!(
            choose(&phone48, AudioFormat::pcm(44_100, 16, 2)).unwrap_err(),
            "the DAC does not list 44.1 kHz for bit-perfect"
        );
        assert_eq!(
            choose(&phone48, AudioFormat::pcm(48_000, 24, 2)).unwrap_err(),
            "the DAC does not take 24-bit bit-perfect"
        );
        // Android 13, and a DAC whose list has no bit-perfect behavior
        assert_eq!(
            choose(&dac(33, ka17_mixer()), AudioFormat::pcm(96_000, 24, 2)).unwrap_err(),
            "Android 13: bit-perfect needs Android 14"
        );
        let mixing_only = dac(34, ka17_mixer().into_iter().filter(|e| e.behavior == 0).collect());
        assert_eq!(
            offer(&mixing_only).unwrap_err(),
            "Android offers no bit-perfect mode for this DAC"
        );
    }

    #[test]
    fn confirmed_only_with_the_same_format_read_back_and_the_dac_routed() {
        let e = bp(P24, 96_000);
        assert_eq!(confirmation(&e, &confirmed_report(e), 41), Ok(()));
        let refused = OpenReport {
            set_ok: false,
            ..confirmed_report(e)
        };
        assert_eq!(
            confirmation(&e, &refused, 41).unwrap_err(),
            "Android refused bit-perfect for this format"
        );
        let nothing = OpenReport {
            readback: None,
            ..confirmed_report(e)
        };
        assert_eq!(
            confirmation(&e, &nothing, 41).unwrap_err(),
            "Android refused bit-perfect for this format"
        );
        for rb in [
            bp(P24, 48_000),
            bp(P32, 96_000),
            MixerEntry { behavior: 0, ..e },
            MixerEntry { channel_mask: 4, ..e },
        ] {
            let other = OpenReport {
                readback: Some(rb),
                ..confirmed_report(e)
            };
            assert_eq!(
                confirmation(&e, &other, 41).unwrap_err(),
                "Android set its mixer to another format: not bit-perfect",
                "{rb:?}"
            );
        }
        let bluetooth = OpenReport {
            routed_id: Some(7),
            routed_name: Some("WH-1000XM5".into()),
            ..confirmed_report(e)
        };
        assert_eq!(
            confirmation(&e, &bluetooth, 41).unwrap_err(),
            "playing through WH-1000XM5, not the DAC"
        );
        let unknown = OpenReport {
            routed_id: None,
            ..confirmed_report(e)
        };
        assert!(confirmation(&e, &unknown, 41).is_err());
    }

    fn shared_caps() -> OutputCaps {
        OutputCaps {
            pcm_rates: vec![44_100, 48_000],
            max_pcm_bits: 24,
            pcm_bits: vec![16, 24],
            max_channels: 2,
            quality_ceiling: QualityCeiling::LosslessUpTo {
                rate_hz: 48_000,
                bits: 24,
            },
            ..OutputCaps::unlimited()
        }
    }

    #[test]
    fn the_node_stops_converting_for_a_bit_perfect_dac() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        with_player();
        let ka17 = dac(34, ka17_mixer());
        let caps = live_caps(
            Some(&ka17),
            DeviceSettings::default(),
            false,
            Some(100),
            &shared_caps(),
        );
        assert_eq!(
            caps.pcm_rates,
            [44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000]
        );
        assert_eq!(
            (caps.pcm_bits.as_slice(), caps.max_pcm_bits),
            (&[16, 24, 32][..], 32)
        );
        assert_eq!(caps.quality_ceiling, QualityCeiling::BitPerfect);
        assert_eq!(
            caps.hw_volume, None,
            "no volume of its own: the level is the DAC's"
        );
        // The node's plan: a 96/24 and a 192/24 track reach the DAC unchanged
        for src in [AudioFormat::pcm(96_000, 24, 2), AudioFormat::pcm(192_000, 24, 2)] {
            let plan = pm_audio::Plan::negotiate(src, &caps, &pm_audio::PipelineConfig::default()).unwrap();
            assert_eq!(plan.output_format(), src, "{src}: no resample, no requantize");
            assert!(
                plan.signal_path()
                    .stages
                    .iter()
                    .all(|s| s.quality == pm_model::Quality::BitPerfect),
                "{src}: {:?}",
                plan.signal_path().stages
            );
            // Shared, as before: converted to 48 kHz
            let shared =
                pm_audio::Plan::negotiate(src, &shared_caps(), &pm_audio::PipelineConfig::default()).unwrap();
            assert_eq!(shared.output_format().rate_hz, 48_000);
        }
        // Auto with Android's volume turned down: shared caps (never louder); Bit-perfect chosen: the DAC's
        assert_eq!(
            live_caps(
                Some(&ka17),
                DeviceSettings::default(),
                false,
                Some(40),
                &shared_caps()
            ),
            shared_caps()
        );
        assert_eq!(
            live_caps(
                Some(&ka17),
                DeviceSettings::default(),
                false,
                None,
                &shared_caps()
            ),
            shared_caps()
        );
        let chosen = DeviceSettings {
            mode: PreferredMode::BitPerfect,
            ..DeviceSettings::default()
        };
        // Bit-perfect chosen below full volume: only with the listener's agreement to full level
        assert_eq!(
            live_caps(Some(&ka17), chosen, false, Some(40), &shared_caps()),
            shared_caps()
        );
        assert_eq!(
            live_caps(Some(&ka17), chosen, true, Some(40), &shared_caps()),
            caps
        );
        // Shared in Settings, Android 13, no DAC: the shared caps, as today
        let shared_pref = DeviceSettings {
            mode: PreferredMode::Shared,
            ..DeviceSettings::default()
        };
        assert_eq!(
            live_caps(Some(&ka17), shared_pref, false, Some(100), &shared_caps()),
            shared_caps()
        );
        assert_eq!(
            live_caps(
                Some(&dac(33, ka17_mixer())),
                DeviceSettings::default(),
                false,
                Some(100),
                &shared_caps()
            ),
            shared_caps()
        );
        assert_eq!(
            live_caps(None, DeviceSettings::default(), false, Some(100), &shared_caps()),
            shared_caps()
        );
    }

    /// Never louder: Android applies no volume in bit-perfect mode, so Auto plays it only from
    /// full volume; Bit-perfect chosen plays anyway, with a warning to set the level on the DAC.
    #[test]
    fn the_volume_hand_over_is_never_louder() {
        use PreferredMode::{Auto, BitPerfect, Unknown};
        // At full volume nothing is lost: bit-perfect, Auto or chosen
        assert_eq!(volume_handover(Auto, Some(100), false), Ok(None));
        assert_eq!(volume_handover(BitPerfect, Some(100), false), Ok(None));
        // Below full, Auto stays shared, whatever was agreed
        for ok in [false, true] {
            let auto = volume_handover(Auto, Some(40), ok).unwrap_err();
            assert!(
                auto.starts_with("Android's volume is at 40%, and bit-perfect plays at full level"),
                "{auto}"
            );
            assert!(volume_handover(Auto, None, ok).is_err(), "unknown volume: shared");
            assert!(volume_handover(Unknown, Some(10), ok).is_err());
        }
        // Bit-perfect chosen below full: only with the agreement to full level, and still warned
        assert_eq!(
            volume_handover(BitPerfect, Some(40), false),
            Err("Bit-perfect plays at full level, and Android's volume (40%) would no longer apply: confirm Bit-perfect in Settings to use it".into())
        );
        assert!(volume_handover(BitPerfect, None, false).is_err());
        assert_eq!(
            volume_handover(BitPerfect, Some(40), true),
            Ok(Some(
                "Bit-perfect plays at full level: Android's volume (40%) does not apply. Set the level on the DAC or amplifier".into()
            ))
        );
    }

    /// The node picks a rate and a size apart: the caps only offer pairs the DAC lists together,
    /// and anything else is converted to one (a 384 kHz track on a 192 kHz DAC plays at 192).
    #[test]
    fn caps_offer_only_what_the_dac_lists_together() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        with_player();
        // 16 bit only at 48 kHz, 32 bit only at 192 kHz
        let odd = dac(34, vec![bp(P16, 48_000), bp(P32, 192_000)]);
        // (24 bit plays in the 32-bit entry, left-justified)
        assert_eq!(rectangle(&odd), (vec![192_000], vec![24, 32]));
        // 24-bit packed everywhere, 16 bit only at 44.1/48
        let mut mixer: Vec<MixerEntry> = [44_100, 48_000, 96_000, 192_000]
            .iter()
            .map(|r| bp(P24, *r))
            .collect();
        mixer.extend([bp(P16, 44_100), bp(P16, 48_000)]);
        let grid = dac(34, mixer);
        assert_eq!(
            rectangle(&grid),
            (vec![44_100, 48_000, 96_000, 192_000], vec![24])
        );
        // Every pair the node can plan from these caps opens bit-perfect
        for d in [&odd, &grid, &dac(34, ka17_mixer())] {
            let caps = live_caps(
                Some(d),
                DeviceSettings::default(),
                false,
                Some(100),
                &shared_caps(),
            );
            for src in [
                AudioFormat::pcm(44_100, 16, 2),
                AudioFormat::pcm(96_000, 24, 2),
                AudioFormat::pcm(352_800, 24, 2),
                AudioFormat::pcm(384_000, 16, 2),
                AudioFormat::pcm(768_000, 32, 2),
            ] {
                let plan =
                    pm_audio::Plan::negotiate(src, &caps, &pm_audio::PipelineConfig::default()).unwrap();
                let out = plan.output_format();
                assert!(
                    choose(d, out).is_ok(),
                    "{src} planned as {out}, which the DAC does not list"
                );
            }
        }
        // A 192 kHz DAC: 352.8 and 384 kHz tracks are converted to a rate it lists
        let k192 = dac(
            34,
            ka17_mixer().into_iter().filter(|e| e.rate <= 192_000).collect(),
        );
        let caps = live_caps(
            Some(&k192),
            DeviceSettings::default(),
            false,
            Some(100),
            &shared_caps(),
        );
        for src in [AudioFormat::pcm(352_800, 24, 2), AudioFormat::pcm(384_000, 24, 2)] {
            let out = pm_audio::Plan::negotiate(src, &caps, &pm_audio::PipelineConfig::default())
                .unwrap()
                .output_format();
            assert!(
                out.rate_hz <= 192_000 && choose(&k192, out).is_ok(),
                "{src} → {out}"
            );
        }
    }

    #[test]
    fn samples_reach_the_track_with_their_values_unchanged() {
        let mut out = Vec::new();
        encode(16, P16, &[1, -2, 0x7fff, -0x8000], &mut out);
        assert_eq!(out, [1, 0, 0xfe, 0xff, 0xff, 0x7f, 0x00, 0x80]);
        encode(24, P24, &[0x123456, -1, -0x800000], &mut out);
        assert_eq!(out, [0x56, 0x34, 0x12, 0xff, 0xff, 0xff, 0x00, 0x00, 0x80]);
        // 24 in 32: left-justified, the low byte zero; read back as 32-bit >> 8 it is the same value
        encode(24, P32, &[0x123456, -0x800000], &mut out);
        assert_eq!(out, [0x00, 0x56, 0x34, 0x12, 0x00, 0x00, 0x00, 0x80]);
        encode(32, P32, &[i32::MIN, 7], &mut out);
        assert_eq!(out, [0, 0, 0, 0x80, 7, 0, 0, 0]);
    }

    fn ka17_report_with(mixer: &[MixerEntry]) -> String {
        let m = serde_json::to_string(mixer).unwrap();
        format!(
            r#"{{"v":1,"sdk":34,"devices":[{{"id":41,"type":11,"name":"FiiO KA17","rates":[44100,96000,192000],
            "encodings":[2,21,22],"channel_counts":[2],"routed":true,"mixer":{m}}}]}}"#
        )
    }

    #[test]
    fn an_open_is_confirmed_or_says_why_and_an_unplug_is_gone() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let player = with_player();
        usb_dac::set_settings(DeviceSettings::default());
        usb_dac::set_dac(None);
        usb_dac::apply_report(&ka17_report_with(&ka17_mixer()));
        let fmt = AudioFormat::pcm(96_000, 24, 2);
        let v = usb_dac::current().unwrap();
        assert_eq!(v.modes, [DeviceMode::Shared, DeviceMode::BitPerfect]);
        assert_eq!(
            (v.mode, v.confirmed),
            (DeviceMode::Shared, false),
            "nothing played yet"
        );

        // Confirmed
        *player.report.lock().unwrap() = confirmed_report(bp(P24, 96_000));
        let Plan::BitPerfect(d, c, w) = plan(fmt) else {
            panic!("bit-perfect expected")
        };
        let mut sink = open(OutputId::new("cpal", "phone"), &d, c, w, fmt).unwrap();
        assert_eq!(
            *player.opened.lock().unwrap(),
            [bp(P24, 96_000)],
            "the same format to the mixer"
        );
        let v = usb_dac::current().unwrap();
        assert_eq!(
            (v.mode, v.confirmed, v.format, v.note.clone()),
            (DeviceMode::BitPerfect, true, Some(fmt), None)
        );
        let block = AudioBlock::pcm_i32(fmt, vec![0x123456, -1, 5, 6]);
        sink.write(&block).unwrap();
        assert_eq!(
            *player.bytes.lock().unwrap(),
            [0x56, 0x34, 0x12, 0xff, 0xff, 0xff, 5, 0, 0, 6, 0, 0]
        );
        assert_eq!(sink.frames_written(), 2);
        assert_eq!(sink.buffer_level().queued_frames, 2);
        player.played.store(2, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(sink.buffer_level().queued_frames, 0);

        // Unplugged mid-track: the next write is Gone (the zone pauses), never anything else
        usb_dac::apply_report(r#"{"v":1,"sdk":34,"devices":[]}"#);
        let e = sink.write(&block).unwrap_err();
        assert!(matches!(e, SinkError::Gone(..)), "{e}");
        assert_eq!(e.gone_reason().as_deref(), Some("FiiO KA17 was unplugged"));
        assert!(sink.resume().is_err(), "nothing resumes on a DAC that is gone");
        drop(sink);
        // The track itself said so first (Android moved it): also Gone
        usb_dac::apply_report(&ka17_report_with(&ka17_mixer()));
        let Plan::BitPerfect(d, c, w) = plan(fmt) else {
            panic!()
        };
        let mut sink = open(OutputId::new("cpal", "phone"), &d, c, w, fmt).unwrap();
        player.gone.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(sink.write(&block), Err(SinkError::Gone(..))));
        player.gone.store(false, std::sync::atomic::Ordering::SeqCst);
        drop(sink);

        // Android refused, or read back another format: it plays, shared, and says why
        *player.report.lock().unwrap() = OpenReport {
            set_ok: false,
            ..confirmed_report(bp(P24, 96_000))
        };
        let Plan::BitPerfect(d, c, w) = plan(fmt) else {
            panic!()
        };
        drop(open(OutputId::new("cpal", "phone"), &d, c, w, fmt).unwrap());
        let v = usb_dac::current().unwrap();
        assert_eq!(
            (v.mode, v.confirmed, v.note.as_deref()),
            (
                DeviceMode::Shared,
                false,
                Some("Android refused bit-perfect for this format")
            )
        );
        // The player could not start at all (another app holds the DAC): the caller plays shared
        *player.refuse.lock().unwrap() = Some("another app holds the DAC".into());
        let Plan::BitPerfect(d, c, w) = plan(fmt) else {
            panic!()
        };
        assert_eq!(
            open(OutputId::new("cpal", "phone"), &d, c, w, fmt).err(),
            Some(OpenError::Refused("another app holds the DAC".into()))
        );
        *player.refuse.lock().unwrap() = None;

        // Formats the DAC doesn't list, and Shared in Settings: shared, with why
        match plan(AudioFormat::pcm(705_600, 24, 2)) {
            Plan::Shared(Some((_, why))) => {
                assert_eq!(why, "the DAC does not list 705.6 kHz for bit-perfect")
            }
            other => panic!("{other:?}"),
        }
        usb_dac::set_settings(DeviceSettings {
            mode: PreferredMode::Shared,
            dac_volume: true,
            full_level_ok: false,
        });
        match plan(fmt) {
            Plan::Shared(Some((_, why))) => assert_eq!(why, "shared, as chosen in Settings"),
            other => panic!("{other:?}"),
        }
        usb_dac::set_settings(DeviceSettings::default());

        // Android plays the track anywhere but the DAC: not used at all, stopped, and why
        let stops = player.stopped.load(std::sync::atomic::Ordering::SeqCst);
        for (routed, why) in [
            (Some(7), "playing through WH-1000XM5, not the DAC"),
            (None, "Android did not say it plays through the DAC"),
        ] {
            *player.report.lock().unwrap() = OpenReport {
                routed_id: routed,
                routed_name: routed.map(|_| "WH-1000XM5".into()),
                ..confirmed_report(bp(P24, 96_000))
            };
            let Plan::BitPerfect(d, c, w) = plan(fmt) else {
                panic!()
            };
            assert_eq!(
                open(OutputId::new("cpal", "phone"), &d, c, w, fmt).err(),
                Some(OpenError::RouteLost(why.into()))
            );
        }
        assert_eq!(
            player.stopped.load(std::sync::atomic::Ordering::SeqCst),
            stops + 2,
            "both tracks stopped"
        );

        // Confirmed, then Android's mixer changes under it: withdrawn at the next write, and the
        // app is told (it registers again, so the node learns it)
        let told = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let t2 = told.clone();
        usb_dac::on_change(move || {
            t2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        *player.report.lock().unwrap() = confirmed_report(bp(P24, 96_000));
        let Plan::BitPerfect(d, c, w) = plan(fmt) else {
            panic!()
        };
        let mut sink = open(OutputId::new("cpal", "phone"), &d, c, w, fmt).unwrap();
        assert!(usb_dac::current().unwrap().confirmed);
        *player.unconfirmed.lock().unwrap() = Some("Android's mixer for the DAC was changed".into());
        sink.write(&block).unwrap();
        let v = usb_dac::current().unwrap();
        assert_eq!(
            (v.mode, v.confirmed, v.note.as_deref()),
            (
                DeviceMode::Shared,
                false,
                Some("Android's mixer for the DAC was changed")
            )
        );
        assert_eq!(told.load(std::sync::atomic::Ordering::SeqCst), 1);
        *player.unconfirmed.lock().unwrap() = None;
        // A flush (the zone paused or seeked) leaves the track paused until the next write
        let plays = player.plays.load(std::sync::atomic::Ordering::SeqCst);
        sink.flush().unwrap();
        assert_eq!(
            player.plays.load(std::sync::atomic::Ordering::SeqCst),
            plays,
            "nothing plays while paused"
        );
        sink.write(&block).unwrap();
        assert_eq!(player.plays.load(std::sync::atomic::Ordering::SeqCst), plays + 1);
        // A settings change: the next song opens afresh (the change applies there)
        assert!(!sink.reopen_at_track());
        usb_dac::set_settings(DeviceSettings {
            mode: PreferredMode::BitPerfect,
            dac_volume: true,
            full_level_ok: false,
        });
        assert!(sink.reopen_at_track());
        drop(sink);

        // Never louder: Auto with Android's volume down plays shared; Bit-perfect chosen plays
        // bit-perfect with the warning to set the level on the DAC
        *player.volume.lock().unwrap() = Some(40);
        usb_dac::set_settings(DeviceSettings::default());
        match plan(fmt) {
            Plan::Shared(Some((_, why))) => assert!(why.starts_with("Android's volume is at 40%"), "{why}"),
            other => panic!("{other:?}"),
        }
        // Bit-perfect chosen without the agreement to full level: shared, and why
        usb_dac::set_settings(DeviceSettings {
            mode: PreferredMode::BitPerfect,
            ..DeviceSettings::default()
        });
        match plan(fmt) {
            Plan::Shared(Some((_, why))) => assert!(why.contains("confirm Bit-perfect in Settings"), "{why}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            usb_dac::current().unwrap().media_volume,
            Some(40),
            "the app sees Android's volume"
        );
        assert!(
            usb_dac::current().unwrap().formats.is_empty(),
            "nothing bit-perfect offered without it"
        );
        // ... and with it: bit-perfect, warned
        usb_dac::set_settings(DeviceSettings {
            mode: PreferredMode::BitPerfect,
            full_level_ok: true,
            ..DeviceSettings::default()
        });
        assert!(usb_dac::full_level_consent());
        let Plan::BitPerfect(d, c, w) = plan(fmt) else {
            panic!()
        };
        drop(open(OutputId::new("cpal", "phone"), &d, c, w, fmt).unwrap());
        let v = usb_dac::current().unwrap();
        assert!(v.confirmed);
        assert_eq!(
            v.note.as_deref(),
            Some("Bit-perfect plays at full level: Android's volume (40%) does not apply. Set the level on the DAC or amplifier")
        );
        // The agreement is for that DAC only, and goes with any other mode
        usb_dac::set_dac(None);
        usb_dac::apply_report(&ka17_report_with(&ka17_mixer()).replace("FiiO KA17", "Other DAC"));
        assert!(!usb_dac::full_level_consent(), "another DAC: asked again");
        usb_dac::set_dac(None);
        usb_dac::apply_report(&ka17_report_with(&ka17_mixer()));
        usb_dac::set_settings(DeviceSettings::default());
        assert!(!usb_dac::settings().full_level_ok);
        *player.volume.lock().unwrap() = Some(100);
        usb_dac::set_settings(DeviceSettings::default());

        // Media moved to Bluetooth with the DAC still in: Gone, and said so
        let Plan::BitPerfect(d, c, w) = plan(fmt) else {
            panic!()
        };
        let mut sink = open(OutputId::new("cpal", "phone"), &d, c, w, fmt).unwrap();
        usb_dac::apply_report(
            &ka17_report_with(&ka17_mixer()).replace(r#""routed":true"#, r#""routed":false"#),
        );
        let e = sink.write(&block).unwrap_err();
        assert_eq!(
            e.gone_reason().as_deref(),
            Some("FiiO KA17 is not where Android plays now")
        );
        drop(sink);
        usb_dac::on_change(|| {});

        usb_dac::set_dac(None);
        assert_eq!(plan(fmt), Plan::Shared(None), "no DAC: the phone as today");
    }

    #[test]
    fn settings_are_kept_where_the_app_says() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("dac.json");
        usb_dac::keep_settings_in(file.clone());
        let s = DeviceSettings {
            mode: PreferredMode::Shared,
            dac_volume: false,
            full_level_ok: false,
        };
        assert!(usb_dac::set_settings(s));
        assert!(!usb_dac::set_settings(s), "the same again is no change");
        usb_dac::set_settings(DeviceSettings::default());
        usb_dac::set_settings(s);
        // The app starts again: read back
        usb_dac::set_settings(DeviceSettings::default());
        std::fs::write(&file, serde_json::to_vec(&s).unwrap()).unwrap();
        usb_dac::keep_settings_in(file);
        assert_eq!(usb_dac::settings(), s);
        usb_dac::set_settings(DeviceSettings::default());
    }

    #[test]
    fn the_players_answer_and_its_contract_with_bitperfect_kt() {
        let (handle, buffer, r) = parse_open_answer(
            r#"{"ok":true,"handle":3,"buffer_frames":9600,"set_ok":true,
            "readback":{"encoding":21,"rate":96000,"channel_mask":12,"channels":2,"behavior":1},
            "routed_id":41,"routed_name":"FiiO KA17"}"#,
        )
        .unwrap();
        assert_eq!((handle, buffer), (3, 9600));
        assert_eq!(r, confirmed_report(bp(P24, 96_000)));
        let (_, _, r) = parse_open_answer(
            r#"{"ok":true,"handle":4,"set_ok":false,"readback":null,"routed_id":null,"routed_name":null}"#,
        )
        .unwrap();
        assert_eq!((r.set_ok, r.readback, r.routed_id), (false, None, None));
        assert_eq!(
            parse_open_answer(r#"{"ok":false,"why":"another app holds the DAC"}"#).unwrap_err(),
            OpenError::Refused("another app holds the DAC".into())
        );
        // The DAC not attached is typed: the zone pauses, never the default route
        assert_eq!(
            parse_open_answer(r#"{"ok":false,"why":"the DAC is not attached","gone":true}"#).unwrap_err(),
            OpenError::RouteLost("the DAC is not attached".into())
        );
        assert!(parse_open_answer("garbage").is_err());
        // BitPerfect.kt writes exactly the keys read here
        let kt = include_str!("../../../tools/release/android/BitPerfect.kt");
        let written: std::collections::BTreeSet<&str> = kt
            .match_indices(".put(\"")
            .filter_map(|(at, m)| kt[at + m.len()..].split('"').next())
            .collect();
        let read = [
            "ok",
            "why",
            "gone",
            "handle",
            "buffer_frames",
            "set_ok",
            "readback",
            "routed_id",
            "routed_name",
            "encoding",
            "rate",
            "channel_mask",
            "channels",
            "behavior",
        ];
        assert_eq!(
            written,
            read.into_iter().collect(),
            "BitPerfect.kt writes {written:?}"
        );
    }

    /// A track Android plays anywhere but the DAC at open (it is being unplugged): the zone pauses
    /// and nothing opens; a refusal on the same DAC plays shared through it.
    #[test]
    fn a_lost_route_pauses_and_a_refusal_plays_shared() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let player = with_player();
        usb_dac::set_settings(DeviceSettings::default());
        usb_dac::set_dac(None);
        usb_dac::apply_report(&ka17_report_with(&ka17_mixer()));
        let fmt = AudioFormat::pcm(96_000, 24, 2);
        *player.report.lock().unwrap() = OpenReport {
            routed_id: Some(2),
            routed_name: Some("Pixel 8 speaker".into()),
            ..confirmed_report(bp(P24, 96_000))
        };
        match open_phone(OutputId::new("cpal", "phone"), fmt) {
            PhoneOpen::Gone(why) => assert!(
                why.starts_with("FiiO KA17 is not where Android plays now"),
                "{why}"
            ),
            _ => panic!("must not fall back to the default route"),
        }
        *player.report.lock().unwrap() = confirmed_report(bp(P24, 96_000));
        *player.refuse.lock().unwrap() = Some("another app holds the DAC".into());
        assert!(matches!(
            open_phone(OutputId::new("cpal", "phone"), fmt),
            PhoneOpen::Shared {
                any_rate: true,
                opened_for: Some(_)
            }
        ));
        *player.refuse.lock().unwrap() = None;
        assert!(matches!(
            open_phone(OutputId::new("cpal", "phone"), fmt),
            PhoneOpen::BitPerfect(_)
        ));
        // Unplugged moments after that song's sink closed: the next open pauses, once
        usb_dac::set_dac(None);
        assert!(matches!(
            open_phone(OutputId::new("cpal", "phone"), fmt),
            PhoneOpen::Gone(_)
        ));
        // Then the listener's Play: the phone's own output (no DAC to be for)
        assert!(matches!(
            open_phone(OutputId::new("cpal", "phone"), fmt),
            PhoneOpen::Shared {
                any_rate: false,
                opened_for: None
            }
        ));
    }

    /// Codex H1 (round 3): the DAC goes while the player opens. Whether the app says so (typed)
    /// or only refuses, nothing opens on the default route: the zone pauses.
    #[test]
    fn a_dac_gone_while_the_player_opens_pauses() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let player = with_player();
        usb_dac::set_settings(DeviceSettings::default());
        let fmt = AudioFormat::pcm(96_000, 24, 2);
        for typed in [true, false] {
            usb_dac::set_dac(None);
            usb_dac::apply_report(&ka17_report_with(&ka17_mixer()));
            *player.unplug_on_open.lock().unwrap() = Some(typed);
            match open_phone(OutputId::new("cpal", "phone"), fmt) {
                PhoneOpen::Gone(why) => assert!(why.contains("FiiO KA17"), "{why}"),
                PhoneOpen::Shared { .. } => panic!("typed {typed}: opened the default route"),
                _ => panic!("typed {typed}: no track expected"),
            }
            *player.unplug_on_open.lock().unwrap() = None;
        }
        // The pause was the zone's news: the listener's next Play opens the phone's own output
        assert!(matches!(
            open_phone(OutputId::new("cpal", "phone"), fmt),
            PhoneOpen::Shared { opened_for: None, .. }
        ));
    }

    /// Android's "becoming noisy" warning: the DAC is gone at once, before its removal arrives.
    #[test]
    fn a_noisy_warning_is_a_dac_gone() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        with_player();
        usb_dac::set_dac(None);
        usb_dac::apply_report(&ka17_report_with(&ka17_mixer()));
        let g = usb_dac::gone_count_now();
        // The warning comes with the DAC still listed (its removal follows): gone all the same
        let noisy = ka17_report_with(&ka17_mixer()).replace(r#"{"v":1,"#, r#"{"v":1,"noisy":true,"#);
        assert!(noisy.contains(r#""noisy":true"#));
        assert!(usb_dac::apply_report(&noisy));
        assert_eq!(
            usb_dac::gone_count_now(),
            g + 1,
            "the shared stream goes silent at once"
        );
        assert_eq!(usb_dac::current(), None);
        assert_eq!(usb_dac::gone_reason(), "FiiO KA17 was unplugged");
    }

    /// 96/24 and 192/24 reach the DAC unchanged when it lists them, even where the largest grid of
    /// its formats is 16 bit: the device lists them, and the node plans each source with them.
    #[test]
    fn the_main_goal_96_24_and_192_24_stay_unchanged() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        with_player();
        usb_dac::set_settings(DeviceSettings::default());
        // 16 bit at every rate, packed 24 only at 96 and 192: the largest grid is 16 bit, and a
        // node planning from it alone would requantize 96/24 and 192/24 to 16
        let mut mixer: Vec<MixerEntry> = [44_100, 48_000, 96_000, 192_000]
            .iter()
            .map(|r| bp(P16, *r))
            .collect();
        mixer.extend([bp(P24, 96_000), bp(P24, 192_000)]);
        usb_dac::set_dac(None);
        usb_dac::apply_report(&ka17_report_with(&mixer));
        let listed = usb_dac::current().unwrap().formats;
        for src in [
            AudioFormat::pcm(96_000, 24, 2),
            AudioFormat::pcm(192_000, 24, 2),
            AudioFormat::pcm(44_100, 16, 2),
        ] {
            assert!(
                listed.contains(&pm_zones::DeviceFormat {
                    rate_hz: src.rate_hz,
                    bits: src.bits,
                    channels: 2
                }),
                "{src} listed"
            );
            let caps = phone_caps_for(&shared_caps(), src);
            let plan = pm_audio::Plan::negotiate(src, &caps, &pm_audio::PipelineConfig::default()).unwrap();
            assert_eq!(plan.output_format(), src, "{src}: unchanged");
            assert!(choose(&usb_dac::current_dac().unwrap(), src).is_ok());
        }
        usb_dac::set_dac(None);
    }

    /// Claude H1 / Codex H2 (round 3): Auto played bit-perfect at 100%; during the pause Android's
    /// volume went to 40%. Resuming, the first write after a flush (an idle clear), and the next
    /// song in the same format all go back through the entry gate: none re-enters full level.
    #[test]
    fn resuming_after_the_volume_went_down_never_reenters_full_level() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let player = with_player();
        usb_dac::set_settings(DeviceSettings::default());
        usb_dac::set_dac(None);
        usb_dac::apply_report(&ka17_report_with(&ka17_mixer()));
        let fmt = AudioFormat::pcm(96_000, 24, 2);
        *player.report.lock().unwrap() = confirmed_report(bp(P24, 96_000));
        let PhoneOpen::BitPerfect(mut sink) = open_phone(OutputId::new("cpal", "phone"), fmt) else {
            panic!("bit-perfect at 100%")
        };
        let block = AudioBlock::pcm_i32(fmt, vec![1, 2, 3, 4]);
        sink.write(&block).unwrap();
        assert!(!sink.reopen_at_track() && !sink.reopen_now());
        assert!(sink.pause().unwrap(), "held by the track");
        *player.volume.lock().unwrap() = Some(40);
        let plays = player.plays.load(std::sync::atomic::Ordering::SeqCst);
        // Resume: refused, not Gone: the zone opens the output again, planned as now (shared)
        let e = sink.resume().unwrap_err();
        assert!(e.gone_reason().is_none() && e.to_string().contains("40%"), "{e}");
        assert_eq!(
            player.plays.load(std::sync::atomic::Ordering::SeqCst),
            plays,
            "the track did not play"
        );
        // A flush (the node's pause, an idle clear), then a write: whoever writes reopens first
        sink.flush().unwrap();
        assert!(sink.reopen_now());
        // And one that didn't: paused, never played
        assert!(sink.write(&block).is_err());
        assert_eq!(player.plays.load(std::sync::atomic::Ordering::SeqCst), plays);
        // The same format next song: afresh, planned as now
        std::thread::sleep(std::time::Duration::from_millis(1_050));
        assert!(sink.reopen_at_track());
        drop(sink);
        assert!(matches!(
            open_phone(OutputId::new("cpal", "phone"), fmt),
            PhoneOpen::Shared { any_rate: true, .. }
        ));
        // Back at 100%: bit-perfect again; explicit Bit-perfect with consent at 40%: allowed
        *player.volume.lock().unwrap() = Some(100);
        assert!(entry_gate().is_ok());
        *player.volume.lock().unwrap() = Some(40);
        usb_dac::set_settings(DeviceSettings {
            mode: PreferredMode::BitPerfect,
            full_level_ok: true,
            ..DeviceSettings::default()
        });
        assert!(entry_gate().unwrap().is_some(), "allowed, with the warning");
        usb_dac::set_settings(DeviceSettings::default());
        assert!(entry_gate().is_err(), "Auto at 40%: never");
    }

    /// Codex H3 (round 3): a sink with nothing queued whose DAC went still says Gone, from drain
    /// and from flush: a song boundary pauses instead of opening the next song elsewhere.
    #[test]
    fn an_empty_sink_on_a_dac_that_went_says_gone() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let player = with_player();
        usb_dac::set_settings(DeviceSettings::default());
        usb_dac::set_dac(None);
        usb_dac::apply_report(&ka17_report_with(&ka17_mixer()));
        let fmt = AudioFormat::pcm(96_000, 24, 2);
        *player.report.lock().unwrap() = confirmed_report(bp(P24, 96_000));
        for flush in [false, true] {
            let PhoneOpen::BitPerfect(mut sink) = open_phone(OutputId::new("cpal", "phone"), fmt) else {
                panic!()
            };
            assert_eq!(sink.buffer_level().queued_frames, 0, "nothing queued");
            usb_dac::apply_report(r#"{"v":1,"sdk":34,"devices":[]}"#);
            let r = if flush { sink.flush() } else { sink.drain() };
            let e = r.unwrap_err();
            assert_eq!(
                e.gone_reason().as_deref(),
                Some("FiiO KA17 was unplugged"),
                "flush {flush}"
            );
            drop(sink);
            usb_dac::apply_report(&ka17_report_with(&ka17_mixer()));
        }
    }
}
