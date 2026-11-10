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
//! * the volume: Android scales nothing in a bit-perfect stream; it hands its media volume to the
//!   USB DAC's own hardware volume (AOSP's `BitPerfectThread` passes the track's volume to the
//!   HAL, whose USB `setVolume` sets the card's feature unit). So the zone's volume stays
//!   Android's media volume at any level: bit-perfect never waits for full volume. Before the
//!   first bit-perfect track on a DAC the app lowers that volume, while Android routes media to
//!   that DAC alone, to the DAC's last level, else [`usb_dac::MEDIA_START_PERCENT`], never raising
//!   it ([`TrackBackend::set_media_volume_on`]). A DAC without a volume of its own has a fixed
//!   zone volume (set on the DAC or amplifier);
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
use pm_zones::{AudioSink, BufferLevel, DeviceMode, OutputId, SinkError};
use std::sync::{Arc, RwLock};

/// Android 14: `setPreferredMixerAttributes`.
pub const API_BIT_PERFECT: u32 = 34;

/// Android's player for a bit-perfect stream: the app's `BitPerfect.kt`, over JNI.
pub trait TrackBackend: Send + Sync {
    /// Set the DAC's mixer to `entry`, bit-perfect, then start an `AudioTrack` with the same
    /// attributes and format, preferring the DAC. What Android said comes back with the track.
    /// `Err`: [`OpenError::RouteLost`] when the DAC is not there (any more), else `Refused`.
    fn open(&self, dac_id: i32, entry: &MixerEntry) -> Result<(Box<dyn Track>, OpenReport), OpenError>;
    /// Lower Android's media volume to `percent` (never raise it), only while Android routes
    /// media to the DAC it knows as `dac_id` and nothing else (`getAudioDevicesForAttributes`):
    /// the media volume on that DAC as read back (0 to 100), or why there is no start level.
    /// Never another route's (the speaker's) volume.
    fn set_media_volume_on(&self, dac_id: i32, percent: u8) -> Result<u8, MediaRefused>;
}

/// Why Android's media volume is no start level for a bit-perfect track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaRefused {
    /// Media doesn't go to that DAC alone (another route, or several): nothing was set.
    Route,
    /// Android keeps the media volume fixed, or didn't take the level (it read back higher).
    Level,
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

/// Before the first bit-perfect track on `dac` since it was plugged in: Android's media volume (in
/// bit-perfect mode the DAC's own volume) down to the level the app last played it at, else
/// [`usb_dac::MEDIA_START_PERCENT`] (never up: lower already, it stays); set only while Android
/// routes media to that DAC alone. `Err` when it could not be set there: nothing plays then (the
/// route isn't the DAC's alone: [`OpenError::RouteLost`]; the level isn't known to be low:
/// [`OpenError::NoStartLevel`], with what to do).
fn media_start(b: &dyn TrackBackend, dac: &Dac) -> Result<(), OpenError> {
    if usb_dac::media_start_done(dac.id) {
        return Ok(());
    }
    let level = usb_dac::media_start_level(dac.device.usb);
    let now = match b.set_media_volume_on(dac.id, level) {
        Ok(now) => now,
        Err(MediaRefused::Route) => {
            return Err(OpenError::RouteLost(format!(
                "Android's media volume could not be set for {}: media isn't on it alone",
                dac.device.name
            )))
        }
        Err(MediaRefused::Level) => {
            return Err(OpenError::NoStartLevel(format!(
                "Android won't set the media volume for {} (it is fixed, or this phone ignores it), so the first sound can't be made quiet and nothing plays. Allow Exclusive USB in Settings › This phone: the app then sets the DAC's own volume",
                dac.device.name
            )))
        }
    };
    tracing::info!(dac = %dac.device.name, level, now, "bit-perfect: Android's media volume before the first track");
    usb_dac::media_start_set(dac.id);
    if let Some(u) = dac.device.usb {
        usb_dac::save_media(u, now);
    }
    Ok(())
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
/// when bit-perfect is offered; else `shared`. The zone's volume stays Android's media volume
/// (Android hands it to the DAC's own volume), except on a DAC known to have no volume of its own:
/// none then (the DAC or amplifier sets the level).
pub fn live_caps(dac: Option<&Dac>, shared: &OutputCaps) -> OutputCaps {
    let Some(dac) = dac.filter(|d| offer(d).is_ok()) else {
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
        hw_volume: if crate::exclusive::own_volume(dac) == Some(false) {
            None
        } else {
            shared.hw_volume
        },
        quality_ceiling: QualityCeiling::BitPerfect,
        ..shared.clone()
    }
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

/// For the device the phone reports: the exact formats it plays unchanged now, in Exclusive USB
/// or bit-perfect (none when it plays shared).
pub fn view_extras(dac: &Dac) -> Vec<pm_zones::DeviceFormat> {
    if crate::exclusive::plays_exclusive(dac) {
        crate::exclusive::formats(dac)
    } else if offer(dac).is_ok() {
        formats(dac)
    } else {
        Vec::new()
    }
}

/// [`live_caps`] for the phone's output now (or Exclusive USB's, when it plays that way), with
/// the formats the DAC then takes unchanged.
fn phone_caps_and_formats(shared: &OutputCaps) -> (OutputCaps, Vec<pm_zones::DeviceFormat>) {
    let dac = usb_dac::current_dac();
    if let Some(d) = dac.as_ref() {
        if let Some(c) = crate::exclusive::live_caps(d, shared) {
            return (c, crate::exclusive::formats(d));
        }
    }
    let caps = live_caps(dac.as_ref(), shared);
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
    /// Through the bit-perfect player, on this DAC with this entry.
    BitPerfect(Dac, MixerChoice),
    /// Through Android's mixer; with a DAC, why not bit-perfect.
    Shared(Option<(Dac, String)>),
}

/// How the phone's output opens `fmt` now: bit-perfect whenever the DAC takes the format so, at
/// any media volume.
pub fn plan(fmt: AudioFormat) -> Plan {
    let Some(dac) = usb_dac::current_dac() else {
        return Plan::Shared(None);
    };
    match choose(&dac, fmt) {
        Ok(c) => Plan::BitPerfect(dac, c),
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
    /// Android's media volume can't be made the DAC's quiet start level (fixed, or ignored): the
    /// zone pauses, and why says what to do.
    NoStartLevel(String),
}

impl OpenError {
    pub fn why(&self) -> &str {
        match self {
            OpenError::Refused(w) | OpenError::RouteLost(w) | OpenError::NoStartLevel(w) => w,
        }
    }
}

/// What the phone's output opens for a format.
pub enum PhoneOpen {
    /// The bit-perfect player.
    BitPerfect(TrackSink),
    /// The app's own USB driver (Exclusive USB).
    Exclusive(Box<crate::exclusive::ExclusiveSink>),
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
fn open_exclusive(id: OutputId, dac: &Dac, fmt: AudioFormat) -> Result<PhoneOpen, String> {
    use crate::exclusive::ExclusiveError as E;
    match crate::exclusive::open(id, dac, fmt) {
        Ok(s) => Ok(PhoneOpen::Exclusive(Box::new(s))),
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

/// How the phone's output opens `fmt` now: the whole decision, apart from cpal. One order, at any
/// media volume: Exclusive USB where it opens, else Android's bit-perfect, else shared (only where
/// neither is possible, and the device says why). The DAC it is for is fixed first: nothing opened
/// for it may reach another device (rule (b), [`usb_dac::may_reach`]).
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
    // Android's permission for Exclusive USB is missing: said with bit-perfect's reason when it
    // plays shared (the listener can give it)
    let mut exclusive_why: Option<String> = None;
    // Exclusive USB was tried first for this open and didn't play: bit-perfect is next
    let mut tried_exclusive = false;
    if let Some(dac) = usb_dac::current_dac() {
        match crate::exclusive::decide(&dac) {
            Some(Ok(())) => match open_exclusive(id.clone(), &dac, fmt) {
                // An Exclusive USB that took the DAC and won't start plays nothing now (never
                // another path mid-take); from the next song Android plays this DAC (bit-perfect,
                // then shared), unless the listener muted the zone on it (Android's paths don't
                // know that mute: they would play)
                Ok(PhoneOpen::Failed(why)) if !crate::exclusive::listener_muted(&dac) => {
                    crate::exclusive::auto_failed(&dac);
                    return PhoneOpen::Failed(format!(
                        "{why}. Android plays it (bit-perfect where it can) from the next song"
                    ));
                }
                Ok(o) => return o,
                Err(why) => {
                    tracing::info!(format = %fmt, "no Exclusive USB ({why}); bit-perfect next");
                    tried_exclusive = true;
                }
            },
            Some(Err(why)) => {
                exclusive_why = Some(why);
                tried_exclusive = true;
            }
            None => {}
        }
        // Android's paths only once Android has the DAC back: never while the app holds it, and
        // after the app gave it back (or touched it in a failed attempt) only once Android lists
        // it again, under a new id: the DAC this open is for is fixed again then
        crate::exclusive::release();
        if crate::exclusive::holding() {
            return gone(format!(
                "{} is going back to Android: press Play again in a moment",
                dac.device.name
            ));
        }
        // Waited for a give-back: the DAC this open is for is the one Android lists now (a new
        // id); none listed: Gone. Not waited: the DAC fixed at the start stays (it went meanwhile:
        // the gate says so, never the default route)
        match crate::exclusive::wait_android_back() {
            Err(why) => return gone(why),
            Ok(true) => match usb_dac::opened_for_now() {
                Some(f) => at_open = Some(f),
                None => return gone(format!("{} went while going back to Android", dac.device.name)),
            },
            Ok(false) => {}
        }
    }
    let shared = |any_rate| PhoneOpen::Shared {
        any_rate,
        opened_for: at_open.clone(),
    };
    // Shared through the DAC: bit-perfect's reason, and Exclusive USB's when it had one
    let shared_why = |why: String| match exclusive_why.as_deref() {
        Some(ex) => usb_dac::sentences(&why, ex),
        None => why,
    };
    match plan(fmt) {
        Plan::BitPerfect(dac, choice) => match open(id.clone(), &dac, choice, fmt) {
            Ok(s) => PhoneOpen::BitPerfect(s),
            Err(OpenError::NoStartLevel(why)) => {
                tracing::warn!(format = %fmt, "no start level for bit-perfect: {why}");
                record_shared(&dac, fmt, why.clone());
                gone(why)
            }
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
                // Exclusive USB instead, where permission was given before (never asks), unless it
                // was tried first for this open
                if !tried_exclusive && crate::exclusive::auto_fallback(&dac) {
                    if let Ok(o) = open_exclusive(id, &dac, fmt) {
                        return o;
                    }
                    // The attempt may have taken the DAC from Android: shared only once
                    // Android lists it again, for the DAC as Android lists it then
                    let opened_for = match crate::exclusive::wait_android_back() {
                        Err(why) => return gone(why),
                        Ok(true) => match usb_dac::opened_for_now() {
                            Some(f) => Some(f),
                            None => {
                                return gone(format!("{} went while going back to Android", dac.device.name))
                            }
                        },
                        Ok(false) => at_open.clone(),
                    };
                    record_shared(&dac, fmt, why);
                    return PhoneOpen::Shared {
                        any_rate: true,
                        opened_for,
                    };
                }
                record_shared(&dac, fmt, shared_why(why));
                shared(true)
            }
        },
        // Planned for the DAC's caps but played shared (a mono track, a format it doesn't list):
        // Android's mixer takes the track's rate, never a stopped zone
        Plan::Shared(Some((dac, why))) => {
            record_shared(&dac, fmt, shared_why(why));
            shared(true)
        }
        // No DAC now; one at the start of this open went meanwhile: pause
        Plan::Shared(None) => match usb_dac::may_reach(at_open.as_ref()) {
            Ok(()) => shared(false),
            Err(lost) => gone(lost),
        },
    }
}

/// Open the bit-perfect player for `fmt` and record what Android confirmed: the first track on a
/// DAC sets its start level first (`media_start`). `Err`: no track (why, and whether the DAC is
/// still where Android plays).
pub fn open(id: OutputId, dac: &Dac, choice: MixerChoice, fmt: AudioFormat) -> Result<TrackSink, OpenError> {
    let b = backend().ok_or_else(|| OpenError::Refused("this app has no bit-perfect player here".into()))?;
    media_start(&*b, dac)?;
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
        note: confirmed.err(),
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
        gate: usb_dac::DacGate::new(Some(opened_for)),
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
    gate: usb_dac::DacGate,
    /// The DAC generation at open: a change closes the sink at the next song.
    generation: u64,
    /// Paused (by a pause or a flush): it plays again with the next write or a resume.
    paused: bool,
    bytes: Vec<u8>,
}

impl TrackSink {
    /// Rule (b): its DAC still Android's route, none gone since; else Gone (and the zone knows).
    /// Rule (b): its DAC still Android's route, none gone since; else Gone. The loss stays kept
    /// for the next open until the zone heard it ([`AudioSink::gone_reported`]).
    fn gone(&self) -> Option<SinkError> {
        self.gate.gone(&self.id)
    }

    fn gone_error(&self) -> SinkError {
        // The reason the app gave when it saw this DAC go; the track saying so first: unplugged
        self.gone()
            .unwrap_or_else(|| SinkError::Gone(self.id.clone(), format!("{} was unplugged", self.dac)))
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
            // Starting again (after a flush, an idle clear)
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

    /// Rule (b): its DAC still there.
    fn ready_to_resume(&mut self) -> Result<(), SinkError> {
        match self.gone() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn frames_written(&self) -> u64 {
        self.written
    }

    /// The DAC changed (or Android's permission for Exclusive USB came) since it opened: the next
    /// song opens afresh, planned as it is now.
    fn reopen_at_track(&self) -> bool {
        usb_dac::generation() != self.generation
    }

    /// The zone heard this sink's Gone: the loss need not be kept for the next open.
    fn gone_reported(&mut self) {
        self.gate.reported();
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
    use pm_model::VolumeKind;
    use pm_zones::{DeviceSettings, OutputDevice, PreferredMode};
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
        /// Where Android routes media now: its id for that device (the DAC's, 41, unless a test
        /// says otherwise; 2: the speaker)
        route: Mutex<Option<i32>>,
        /// Every media volume the app asked for, with the DAC it was for
        media_sets: Mutex<Vec<(i32, u8)>>,
        /// Android's media volume on the route now (100 unless a test says otherwise)
        media: Mutex<u8>,
        /// Setting the media volume finds the DAC unplugged (it goes while it is set)
        unplug_on_media: Mutex<bool>,
        /// Android keeps the media volume fixed (or ignores a set)
        fixed: Mutex<bool>,
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
        fn set_media_volume_on(&self, dac_id: i32, percent: u8) -> Result<u8, MediaRefused> {
            if std::mem::take(&mut *self.unplug_on_media.lock().unwrap()) {
                usb_dac::apply_report(r#"{"v":1,"sdk":34,"devices":[]}"#);
                return Err(MediaRefused::Route);
            }
            if *self.route.lock().unwrap() != Some(dac_id) {
                return Err(MediaRefused::Route);
            }
            if *self.fixed.lock().unwrap() {
                return Err(MediaRefused::Level);
            }
            self.media_sets.lock().unwrap().push((dac_id, percent));
            // Lowered, never raised
            let mut m = self.media.lock().unwrap();
            *m = (*m).min(percent);
            Ok(*m)
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
        *f.route.lock().unwrap() = Some(41);
        *f.media.lock().unwrap() = 100;
        usb_dac::forget_levels();
        set_backend(f.clone());
        f
    }

    /// `report` with the KA17's USB ids, and its descriptors when given (Android's permission).
    fn with_usb(report: &str, descriptors: Option<&[u8]>) -> String {
        let d = descriptors
            .map(|raw| {
                let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
                format!(r#","descriptors":"{hex}""#)
            })
            .unwrap_or_default();
        report.replacen(
            r#""routed":true"#,
            &format!(r#""routed":true,"usb":{{"vendor_id":10610,"product_id":147{d}}}"#),
            1,
        )
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
        let caps = live_caps(Some(&ka17), &shared_caps());
        assert_eq!(
            caps.pcm_rates,
            [44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000]
        );
        assert_eq!(
            (caps.pcm_bits.as_slice(), caps.max_pcm_bits),
            (&[16, 24, 32][..], 32)
        );
        assert_eq!(caps.quality_ceiling, QualityCeiling::BitPerfect);
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
        // On the phone, whose zone volume is Android's media volume: it stays that at any level
        // (Android hands it to the DAC's own volume, no gain in the samples)
        let mut phone = shared_caps();
        phone.hw_volume = Some(VolumeKind::Percent);
        assert_eq!(
            live_caps(Some(&ka17), &phone).hw_volume,
            Some(VolumeKind::Percent)
        );
        // A DAC known to have no volume of its own: none (fixed; set on the DAC or amplifier),
        // still bit-perfect
        let mut no_unit = ka17.clone();
        no_unit.descriptors = Some(crate::exclusive::tests::paths_dac(false, false));
        let fixed = live_caps(Some(&no_unit), &phone);
        assert_eq!(
            (fixed.hw_volume, fixed.quality_ceiling),
            (None, QualityCeiling::BitPerfect)
        );
        // Android 13, no DAC: the shared caps, as today
        assert_eq!(
            live_caps(Some(&dac(33, ka17_mixer())), &shared_caps()),
            shared_caps()
        );
        assert_eq!(live_caps(None, &shared_caps()), shared_caps());
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
            let caps = live_caps(Some(d), &shared_caps());
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
        let caps = live_caps(Some(&k192), &shared_caps());
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
        let Plan::BitPerfect(d, c) = plan(fmt) else {
            panic!("bit-perfect expected")
        };
        let mut sink = open(OutputId::new("cpal", "phone"), &d, c, fmt).unwrap();
        assert_eq!(
            *player.opened.lock().unwrap(),
            [bp(P24, 96_000)],
            "the same format to the mixer"
        );
        let v = usb_dac::current().unwrap();
        assert_eq!(
            (v.mode, v.confirmed, v.format, v.note.as_deref()),
            (
                DeviceMode::BitPerfect,
                true,
                Some(fmt),
                Some(usb_dac::FOLLOW_VOLUME)
            ),
            "whether it has a volume of its own isn't known: what to do if it doesn't follow"
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
        let Plan::BitPerfect(d, c) = plan(fmt) else {
            panic!()
        };
        let mut sink = open(OutputId::new("cpal", "phone"), &d, c, fmt).unwrap();
        player.gone.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(sink.write(&block), Err(SinkError::Gone(..))));
        player.gone.store(false, std::sync::atomic::Ordering::SeqCst);
        drop(sink);

        // Android refused, or read back another format: it plays, shared, and says why
        *player.report.lock().unwrap() = OpenReport {
            set_ok: false,
            ..confirmed_report(bp(P24, 96_000))
        };
        let Plan::BitPerfect(d, c) = plan(fmt) else {
            panic!()
        };
        drop(open(OutputId::new("cpal", "phone"), &d, c, fmt).unwrap());
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
        let Plan::BitPerfect(d, c) = plan(fmt) else {
            panic!()
        };
        assert_eq!(
            open(OutputId::new("cpal", "phone"), &d, c, fmt).err(),
            Some(OpenError::Refused("another app holds the DAC".into()))
        );
        *player.refuse.lock().unwrap() = None;

        // Formats the DAC doesn't list: shared, with why
        match plan(AudioFormat::pcm(705_600, 24, 2)) {
            Plan::Shared(Some((_, why))) => {
                assert_eq!(why, "the DAC does not list 705.6 kHz for bit-perfect")
            }
            other => panic!("{other:?}"),
        }

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
            let Plan::BitPerfect(d, c) = plan(fmt) else {
                panic!()
            };
            assert_eq!(
                open(OutputId::new("cpal", "phone"), &d, c, fmt).err(),
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
        let Plan::BitPerfect(d, c) = plan(fmt) else {
            panic!()
        };
        let mut sink = open(OutputId::new("cpal", "phone"), &d, c, fmt).unwrap();
        assert!(usb_dac::current().unwrap().confirmed);
        // (Opening bit-perfect after shared was news too: counted from here)
        told.store(0, std::sync::atomic::Ordering::SeqCst);
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
        // Android's permission for Exclusive USB came (the DAC's descriptors with it): the next
        // song opens afresh, planned as it is then
        assert!(!sink.reopen_at_track());
        usb_dac::apply_report(&with_usb(
            &ka17_report_with(&ka17_mixer()),
            Some(crate::exclusive::tests::KA17),
        ));
        assert!(sink.reopen_at_track());
        drop(sink);

        // Media moved to Bluetooth with the DAC still in: Gone, and said so
        let Plan::BitPerfect(d, c) = plan(fmt) else {
            panic!()
        };
        let mut sink = open(OutputId::new("cpal", "phone"), &d, c, fmt).unwrap();
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

    /// Settings an earlier app saved (Shared, the agreement to full level) read as the one
    /// behaviour: bit-perfect at 40 % media volume, and the device reports no settings; a UI
    /// sending the earlier body changes nothing either.
    #[test]
    fn an_old_settings_file_reads_as_auto() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let player = with_player();
        *player.report.lock().unwrap() = confirmed_report(bp(P24, 96_000));
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("usb-dac.json");
        std::fs::write(
            &file,
            r#"{"mode":"shared","dac_volume":false,"full_level_ok":true}"#,
        )
        .unwrap();
        usb_dac::keep_settings_in(file);
        usb_dac::set_dac(None);
        usb_dac::apply_report(
            &ka17_report_with(&ka17_mixer()).replace(r#""sdk":34"#, r#""sdk":34,"media_volume":40"#),
        );
        assert_eq!(usb_dac::current().unwrap().settings, None);
        let fmt = AudioFormat::pcm(96_000, 24, 2);
        assert!(matches!(
            open_phone(OutputId::new("cpal", "phone"), fmt),
            PhoneOpen::BitPerfect(_)
        ));
        usb_dac::set_settings(DeviceSettings {
            mode: PreferredMode::Shared,
            dac_volume: false,
            full_level_ok: true,
        });
        assert!(matches!(plan(fmt), Plan::BitPerfect(..)));
        assert_eq!(usb_dac::current().unwrap().settings, None);
        usb_dac::forget_levels();
        usb_dac::set_dac(None);
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
        // The start level is set only while Android routes media to that very DAC alone (by its
        // id), checked before anything is set: never the speaker's volume; and only lowered
        let set = kt
            .split("fun setMediaVolumeOn(dacId: Int, percent: Int): Int {")
            .nth(1)
            .and_then(|r| r.split("\n        }\n").next())
            .expect("setMediaVolumeOn");
        let (route, sets) = (
            set.find("getAudioDevicesForAttributes(media).singleOrNull()?.id != dacId"),
            set.find("setStreamVolume"),
        );
        assert!(route.is_some() && sets.is_some() && route < sets, "{set}");
        assert!(
            set.contains("if (now > target) am.setStreamVolume"),
            "lowered only: {set}"
        );
        // ... and read back before it counts (a fixed volume, or a set Android ignored: none)
        assert!(
            set.contains("if (am.isVolumeFixed) return@runCatching -2"),
            "{set}"
        );
        assert!(
            set.rfind("getStreamVolume(") > sets && set.contains("if (v > target) return@runCatching -2"),
            "read back: {set}"
        );
        assert!(
            !kt.contains("mediaVolumeDbOnUsb"),
            "no hand-over reading any more"
        );
    }

    /// A track Android plays anywhere but the DAC at open (it is being unplugged): the zone pauses
    /// and nothing opens; a refusal on the same DAC plays shared through it.
    #[test]
    fn a_lost_route_pauses_and_a_refusal_plays_shared() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let player = with_player();
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

    /// Codex H3 (round 3): a sink with nothing queued whose DAC went still says Gone, from drain
    /// and from flush: a song boundary pauses instead of opening the next song elsewhere.
    #[test]
    fn an_empty_sink_on_a_dac_that_went_says_gone() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let player = with_player();
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

    /// Claude L3 (round 4): an unplug kept for the next open counts only as long as it can be a
    /// song boundary; a Play long after is the listener's and opens the phone's own output.
    #[test]
    fn an_old_unplug_does_not_pause_a_later_play() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let player = with_player();
        usb_dac::set_dac(None);
        usb_dac::apply_report(&ka17_report_with(&ka17_mixer()));
        let fmt = AudioFormat::pcm(96_000, 24, 2);
        *player.report.lock().unwrap() = confirmed_report(bp(P24, 96_000));
        drop(open_phone(OutputId::new("cpal", "phone"), fmt));
        usb_dac::set_dac(None);
        usb_dac::age_gone_unseen(std::time::Duration::from_secs(3600));
        assert!(matches!(
            open_phone(OutputId::new("cpal", "phone"), fmt),
            PhoneOpen::Shared { opened_for: None, .. }
        ));
    }

    // ---- The phone's output through its endpoint, as the node reaches it ----

    /// Android's shared path, as the phone's cpal sink does it apart from cpal itself: the same
    /// gate ([`usb_dac::DacGate`]); what reaches "Android" is recorded, with the DAC it was for.
    struct SharedStandIn {
        id: OutputId,
        fmt: AudioFormat,
        gate: usb_dac::DacGate,
        heard: Arc<Mutex<Vec<i32>>>,
        written: u64,
    }

    impl AudioSink for SharedStandIn {
        fn format(&self) -> AudioFormat {
            self.fmt
        }
        fn write(&mut self, block: &AudioBlock) -> Result<(), SinkError> {
            if let Some(e) = self.gate.gone(&self.id) {
                return Err(e);
            }
            if let Samples::PcmI32(v) = &block.samples {
                self.heard.lock().unwrap().extend_from_slice(v);
            }
            self.written += block.frames() as u64;
            Ok(())
        }
        fn buffer_level(&self) -> BufferLevel {
            BufferLevel {
                queued_frames: 0,
                capacity_frames: 4800,
            }
        }
        fn flush(&mut self) -> Result<(), SinkError> {
            self.gate.gone(&self.id).map_or(Ok(()), Err)
        }
        fn drain(&mut self) -> Result<(), SinkError> {
            self.gate.gone(&self.id).map_or(Ok(()), Err)
        }
        fn frames_written(&self) -> u64 {
            self.written
        }
        fn gone_reported(&mut self) {
            self.gate.reported();
        }
    }

    /// The phone's output: its opens go through [`open_phone`], the whole decision.
    #[derive(Default)]
    struct PhoneOut {
        /// Each shared open, with the DAC it was for (`None`: the phone's own speaker)
        shared_opens: Mutex<Vec<Option<i32>>>,
        heard: Arc<Mutex<Vec<i32>>>,
        /// Reports what the phone's output takes now and its DAC, as the app's output does
        live: bool,
    }

    #[async_trait::async_trait]
    impl pm_zones::Output for PhoneOut {
        fn id(&self) -> OutputId {
            OutputId::new("cpal", "phone")
        }
        fn name(&self) -> String {
            "phone".into()
        }
        fn caps(&self) -> &OutputCaps {
            static CAPS: std::sync::OnceLock<OutputCaps> = std::sync::OnceLock::new();
            CAPS.get_or_init(|| OutputCaps {
                pcm_rates: vec![48_000],
                max_pcm_bits: 16,
                quality_ceiling: QualityCeiling::LosslessUpTo {
                    rate_hz: 48_000,
                    bits: 16,
                },
                ..OutputCaps::unlimited()
            })
        }
        fn live_caps(&self) -> OutputCaps {
            if self.live {
                phone_caps(self.caps())
            } else {
                OutputCaps::unlimited()
            }
        }
        fn device(&self) -> Option<pm_zones::OutputDevice> {
            self.live.then(usb_dac::current).flatten()
        }
        async fn open(&self, fmt: AudioFormat) -> Result<Box<dyn AudioSink>, SinkError> {
            match open_phone(self.id(), fmt) {
                PhoneOpen::BitPerfect(s) => Ok(Box::new(s)),
                PhoneOpen::Exclusive(s) => Ok(s),
                PhoneOpen::Unsupported(_) => Err(SinkError::UnsupportedFormat(self.id(), fmt)),
                PhoneOpen::Failed(why) => Err(SinkError::Unavailable(self.id(), why)),
                PhoneOpen::Gone(why) => Err(SinkError::Gone(self.id(), why)),
                PhoneOpen::Shared { opened_for, .. } => {
                    self.shared_opens
                        .lock()
                        .unwrap()
                        .push(opened_for.as_ref().map(|f| f.dac_id));
                    Ok(Box::new(SharedStandIn {
                        id: self.id(),
                        fmt,
                        gate: usb_dac::DacGate::new(opened_for),
                        heard: self.heard.clone(),
                        written: 0,
                    }))
                }
            }
        }
    }

    /// The phone's endpoint serving [`PhoneOut`], and the node's output on it.
    fn through_endpoint() -> (Arc<PhoneOut>, pm_endpoint::Serving, pm_out_net::NetOutput) {
        let phone = Arc::new(PhoneOut::default());
        let registry = pm_zones::OutputRegistry::new();
        registry.register(phone.clone());
        let key = pm_stream::Key::random();
        let serving = pm_endpoint::Endpoint::new("Pixel 8", registry, key.clone())
            .spawn(std::net::TcpListener::bind("127.0.0.1:0").unwrap())
            .unwrap();
        let cell = pm_out_net::KeyCell::new(key);
        let out = pm_out_net::NetOutput::connect(serving.addr(), &cell)
            .unwrap()
            .remove(0);
        (phone, serving, out)
    }

    fn net_open(out: &pm_out_net::NetOutput, fmt: AudioFormat) -> Result<Box<dyn AudioSink>, SinkError> {
        use pm_zones::Output;
        pm_stream::block_on(out.open(fmt))
    }

    /// Codex H1 (round 5): a song boundary through the endpoint: the old song drained, the DAC
    /// unplugged, the node closes the stream (a Gone found then reaches nobody), the next song
    /// opens in another format: it must say Gone (the zone pauses), and nothing opens on the
    /// phone's speaker. Both playback paths: bit-perfect and Android's shared path.
    #[test]
    fn a_close_after_an_unplug_never_lets_the_next_song_reach_the_speaker() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        for shared in [false, true] {
            let player = with_player();
            // Shared: the bit-perfect player would not start (another app holds it): Android's
            // mixer, through the DAC
            *player.refuse.lock().unwrap() = shared.then(|| "another app holds the DAC".to_string());
            usb_dac::set_dac(None);
            usb_dac::apply_report(&ka17_report_with(&ka17_mixer()));
            *player.report.lock().unwrap() = confirmed_report(bp(P24, 96_000));
            player
                .played
                .store(u64::MAX / 2, std::sync::atomic::Ordering::SeqCst);
            let (phone, _serving, out) = through_endpoint();
            let fmt = AudioFormat::pcm(96_000, 24, 2);
            let mut sink = net_open(&out, fmt).unwrap();
            sink.write(&AudioBlock::pcm_i32(fmt, vec![1; 2 * 480])).unwrap();
            sink.drain().unwrap();
            // Unplugged now, then the stream closes
            usb_dac::apply_report(r#"{"v":1,"sdk":34,"devices":[]}"#);
            drop(sink);
            std::thread::sleep(std::time::Duration::from_millis(300));
            let speaker_before = phone
                .shared_opens
                .lock()
                .unwrap()
                .iter()
                .filter(|o| o.is_none())
                .count();
            let e = net_open(&out, AudioFormat::pcm(48_000, 16, 2))
                .err()
                .unwrap_or_else(|| panic!("shared {shared}: the next song opened"));
            assert!(e.gone_reason().is_some(), "shared {shared}: {e}");
            let speaker = phone
                .shared_opens
                .lock()
                .unwrap()
                .iter()
                .filter(|o| o.is_none())
                .count();
            assert_eq!(speaker, speaker_before, "shared {shared}: the speaker was opened");
            // The zone heard it: the listener's next Play opens the phone's own output
            assert!(net_open(&out, AudioFormat::pcm(48_000, 16, 2)).is_ok());
        }
    }

    /// A report that changes only Android's volume is no change (the app doesn't register again
    /// at every step): the phone's output stays bit-perfect at any level, with or without a DAC.
    #[test]
    fn a_volume_only_report_is_no_change() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _player = with_player();
        usb_dac::set_dac(None);
        let at = |v: i32| {
            ka17_report_with(&ka17_mixer()).replace(r#""sdk":34"#, &format!(r#""sdk":34,"media_volume":{v}"#))
        };
        assert!(usb_dac::apply_report(&at(100)), "the DAC arrives");
        let phone = PhoneOut {
            live: true,
            ..PhoneOut::default()
        };
        use pm_zones::Output;
        for v in [40, 0, -1, 100] {
            assert!(!usb_dac::apply_report(&at(v)), "{v}: no change");
            assert_eq!(
                phone.live_caps().quality_ceiling,
                QualityCeiling::BitPerfect,
                "{v}"
            );
        }
        // Without a DAC: no change either
        let none = |v: u8| format!(r#"{{"v":1,"sdk":34,"devices":[],"media_volume":{v}}}"#);
        usb_dac::apply_report(&none(50));
        assert!(!usb_dac::apply_report(&none(40)), "no DAC: no change");
        usb_dac::apply_report(&ka17_report_with(&ka17_mixer()));
    }

    /// One behaviour: bit-perfect whatever Android's media volume is (40 %, 0 %, unknown), never
    /// shared because of it; a pause, a volume change and a resume carry on bit-perfect, and the
    /// zone's volume stays Android's media volume (Android hands it to the DAC's own volume).
    #[test]
    fn bit_perfect_plays_at_any_media_volume() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let player = with_player();
        *player.report.lock().unwrap() = confirmed_report(bp(P24, 96_000));
        let fmt = AudioFormat::pcm(96_000, 24, 2);
        let mut phone = shared_caps();
        phone.hw_volume = Some(VolumeKind::Percent);
        let at = |v: i32| {
            ka17_report_with(&ka17_mixer()).replace(r#""sdk":34"#, &format!(r#""sdk":34,"media_volume":{v}"#))
        };
        for (v, then) in [(40, 0), (0, 100), (-1, 40)] {
            usb_dac::set_dac(None);
            usb_dac::apply_report(&at(v));
            let caps = phone_caps(&phone);
            assert_eq!(caps.quality_ceiling, QualityCeiling::BitPerfect, "{v}");
            assert_eq!(caps.hw_volume, Some(VolumeKind::Percent), "{v}");
            assert!(matches!(plan(fmt), Plan::BitPerfect(..)), "{v}");
            let PhoneOpen::BitPerfect(mut sink) = open_phone(OutputId::new("cpal", "phone"), fmt) else {
                panic!("{v}: bit-perfect")
            };
            let d = usb_dac::current().unwrap();
            assert_eq!(
                (d.mode, d.confirmed, d.note.as_deref()),
                (DeviceMode::BitPerfect, true, Some(usb_dac::FOLLOW_VOLUME)),
                "{v}"
            );
            let block = AudioBlock::pcm_i32(fmt, vec![1, 2, 3, 4]);
            sink.write(&block).unwrap();
            assert!(sink.pause().unwrap());
            // The volume keys during the pause: no change, and it resumes bit-perfect
            assert!(!usb_dac::apply_report(&at(then)), "{v} then {then}");
            sink.resume().unwrap();
            sink.flush().unwrap();
            sink.write(&block).unwrap();
            assert!(!sink.reopen_now() && !sink.reopen_at_track(), "{v}");
            assert!(usb_dac::current().unwrap().confirmed, "{v}");
            drop(sink);
        }
        usb_dac::set_dac(None);
    }

    /// Android's bit-perfect path: before the first track on a DAC, Android's media volume (the
    /// DAC's own volume then) goes down to the DAC's last level, else 20 %, never up, and only
    /// while Android routes media to that DAC: never the speaker's (nothing plays then). The level
    /// the listener ends up with is kept for that DAC (not a report from just before the set):
    /// plugged in again, it starts there.
    #[test]
    fn the_start_level_is_set_only_on_the_dacs_route() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let player = with_player();
        *player.report.lock().unwrap() = confirmed_report(bp(P24, 96_000));
        let fmt = AudioFormat::pcm(96_000, 24, 2);
        let ids = pm_zones::UsbIds {
            vendor_id: 0x2972,
            product_id: 0x0093,
        };
        let report = |v: u8| {
            with_usb(&ka17_report_with(&ka17_mixer()), None)
                .replace(r#""sdk":34"#, &format!(r#""sdk":34,"media_volume":{v}"#))
        };
        let phone = || OutputId::new("cpal", "phone");
        usb_dac::set_dac(None);
        // Android's volume before anything played on the DAC: not its level
        usb_dac::apply_report(&report(100));
        assert_eq!(usb_dac::saved_level(ids).media, None);
        // Media goes to the speaker (Android's id 2): nothing is set, no track, the zone pauses
        *player.route.lock().unwrap() = Some(2);
        match open_phone(phone(), fmt) {
            PhoneOpen::Gone(why) => assert!(why.contains("FiiO KA17"), "{why}"),
            _ => panic!("no track without the start level"),
        }
        assert!(
            player.media_sets.lock().unwrap().is_empty(),
            "the speaker's volume untouched"
        );
        assert!(player.opened.lock().unwrap().is_empty(), "no track");
        // On the DAC's route: 20 % before the first track, once
        *player.route.lock().unwrap() = Some(41);
        for _ in 0..2 {
            let PhoneOpen::BitPerfect(sink) = open_phone(phone(), fmt) else {
                panic!("bit-perfect")
            };
            drop(sink);
        }
        assert_eq!(*player.media_sets.lock().unwrap(), [(41, 20)]);
        assert_eq!(*player.media.lock().unwrap(), 20, "from 100 % down to 20 %");
        assert_eq!(usb_dac::saved_level(ids).media, Some(20));
        // A report built just before the set (Android's old 100 %): not the DAC's level
        usb_dac::apply_report(&report(100));
        assert_eq!(usb_dac::saved_level(ids).media, Some(20), "a stale report");
        // The listener turns it to 35 %: kept for this DAC
        usb_dac::age_media_start(std::time::Duration::from_secs(3));
        usb_dac::apply_report(&report(35));
        assert_eq!(usb_dac::saved_level(ids).media, Some(35));
        // Unplugged and plugged in again (Android's new id 42): it starts at 35 %
        usb_dac::set_dac(None);
        *player.media.lock().unwrap() = 60;
        usb_dac::apply_report(&report(60).replace(r#""id":41"#, r#""id":42"#));
        *player.route.lock().unwrap() = Some(42);
        *player.report.lock().unwrap() = OpenReport {
            routed_id: Some(42),
            ..confirmed_report(bp(P24, 96_000))
        };
        let PhoneOpen::BitPerfect(sink) = open_phone(phone(), fmt) else {
            panic!("bit-perfect")
        };
        drop(sink);
        assert_eq!(player.media_sets.lock().unwrap().last(), Some(&(42, 35)));
        assert_eq!(*player.media.lock().unwrap(), 35);
        drop(open_phone(phone(), fmt));
        // Never raised: Android had the DAC lower than its last level (5 %): it stays at 5 %
        usb_dac::set_dac(None);
        *player.media.lock().unwrap() = 5;
        usb_dac::apply_report(&report(5).replace(r#""id":41"#, r#""id":43"#));
        *player.route.lock().unwrap() = Some(43);
        *player.report.lock().unwrap() = OpenReport {
            routed_id: Some(43),
            ..confirmed_report(bp(P24, 96_000))
        };
        let PhoneOpen::BitPerfect(sink) = open_phone(phone(), fmt) else {
            panic!("bit-perfect")
        };
        drop(sink);
        assert_eq!(player.media_sets.lock().unwrap().last(), Some(&(43, 35)));
        assert_eq!(*player.media.lock().unwrap(), 5, "not raised to 35 %");
        assert_eq!(usb_dac::saved_level(ids).media, Some(5));
        usb_dac::forget_levels();
        usb_dac::set_dac(None);
    }

    /// Android keeps the media volume fixed (or ignores the set): no quiet start level, so nothing
    /// plays, the zone pauses, and the note says what to do (Exclusive USB sets the DAC itself).
    #[test]
    fn a_fixed_media_volume_pauses_and_says_what_to_do() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let player = with_player();
        *player.report.lock().unwrap() = confirmed_report(bp(P24, 96_000));
        *player.fixed.lock().unwrap() = true;
        usb_dac::set_dac(None);
        usb_dac::apply_report(&ka17_report_with(&ka17_mixer()));
        match open_phone(OutputId::new("cpal", "phone"), AudioFormat::pcm(96_000, 24, 2)) {
            PhoneOpen::Gone(why) => assert!(
                why.contains("can't be made quiet") && why.contains("Allow Exclusive USB"),
                "{why}"
            ),
            _ => panic!("nothing may play without a start level"),
        }
        assert!(player.opened.lock().unwrap().is_empty(), "no track");
        let d = usb_dac::current().unwrap();
        assert_eq!(d.mode, DeviceMode::Shared);
        assert!(d.note.unwrap_or_default().contains("Allow Exclusive USB"));
        usb_dac::set_dac(None);
    }

    /// The DAC goes while its start level is set (the app sees it unplugged, Android says media
    /// isn't on it): nothing opens anywhere, the zone pauses; never the phone's speaker.
    #[test]
    fn an_unplug_while_the_start_level_is_set_never_opens_the_speaker() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let player = with_player();
        *player.report.lock().unwrap() = confirmed_report(bp(P24, 96_000));
        usb_dac::set_dac(None);
        usb_dac::apply_report(&ka17_report_with(&ka17_mixer()));
        *player.unplug_on_media.lock().unwrap() = true;
        match open_phone(OutputId::new("cpal", "phone"), AudioFormat::pcm(96_000, 24, 2)) {
            PhoneOpen::Gone(why) => assert!(why.contains("FiiO KA17"), "{why}"),
            PhoneOpen::Shared { opened_for, .. } => panic!("shared, for {opened_for:?}"),
            _ => panic!("no track expected"),
        }
        assert!(player.opened.lock().unwrap().is_empty(), "no track");
        assert!(player.media_sets.lock().unwrap().is_empty(), "nothing set");
    }

    /// A DAC without a volume of its own (no feature unit on its playback path) stays
    /// bit-perfect; the zone's volume is fixed and the note says plainly to set it on the DAC.
    /// With a unit: Android's media volume. Unknown (no descriptors: no permission yet): nothing
    /// is claimed either way.
    #[test]
    fn a_dac_without_a_volume_of_its_own_stays_bit_perfect_with_a_fixed_volume() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let player = with_player();
        *player.report.lock().unwrap() = confirmed_report(bp(P24, 96_000));
        let fmt = AudioFormat::pcm(96_000, 24, 2);
        let mut phone = shared_caps();
        phone.hw_volume = Some(VolumeKind::Percent);
        let base = ka17_report_with(&ka17_mixer());
        let no_unit = crate::exclusive::tests::paths_dac(false, false);
        let ka17 = crate::exclusive::tests::KA17.to_vec();
        for (descriptors, own, hw, note) in [
            (Some(no_unit), Some(false), None, Some(usb_dac::NO_OWN_VOLUME)),
            (Some(ka17), Some(true), Some(VolumeKind::Percent), None),
            (
                None,
                None,
                Some(VolumeKind::Percent),
                Some(usb_dac::FOLLOW_VOLUME),
            ),
        ] {
            usb_dac::set_dac(None);
            usb_dac::apply_report(&with_usb(&base, descriptors.as_deref()));
            let d = usb_dac::current().unwrap();
            assert_eq!(
                (d.own_volume, d.note.as_deref()),
                (own, note),
                "{own:?}: before the first song"
            );
            let caps = phone_caps(&phone);
            assert_eq!(
                (caps.quality_ceiling, caps.hw_volume),
                (QualityCeiling::BitPerfect, hw),
                "{own:?}"
            );
            // The phone's session volume follows: fixed without a unit, else Android's
            assert_eq!(crate::media_volume::phone_kind(), hw, "{own:?}");
            let PhoneOpen::BitPerfect(sink) = open_phone(OutputId::new("cpal", "phone"), fmt) else {
                panic!("{own:?}: bit-perfect")
            };
            let d = usb_dac::current().unwrap();
            assert_eq!(
                (d.mode, d.confirmed, d.note.as_deref()),
                (DeviceMode::BitPerfect, true, note),
                "{own:?}"
            );
            drop(sink);
        }
        usb_dac::set_dac(None);
    }
}
