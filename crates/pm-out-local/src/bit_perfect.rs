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
//! * [`live_caps`]: what the phone's output takes then (the DAC's bit-perfect rates and sizes, its
//!   volume through Android's media volume), so the node stops converting for it;
//! * [`confirmation`]: confirmed only when Android read the bit-perfect mixer back with exactly
//!   that format and the track plays through the DAC; otherwise exactly why not;
//! * [`TrackSink`]: the zone's sink on the track, which pauses the zone (via
//!   [`SinkError::Gone`]) when the DAC goes.

use crate::usb_dac::{
    self, Dac, LastOpen, MixerEntry, ENCODING_PCM_16BIT, ENCODING_PCM_24BIT_PACKED, ENCODING_PCM_32BIT,
    MIXER_BEHAVIOR_BIT_PERFECT,
};
use pm_model::{AudioBlock, AudioFormat, OutputCaps, QualityCeiling, SampleKind, Samples, VolumeKind};
use pm_zones::{AudioSink, BufferLevel, DeviceMode, OutputId, PreferredMode, SinkError};
use std::sync::{Arc, RwLock};

/// Android 14: `setPreferredMixerAttributes`.
pub const API_BIT_PERFECT: u32 = 34;

/// Android's player for a bit-perfect stream: the app's `BitPerfect.kt`, over JNI.
pub trait TrackBackend: Send + Sync {
    /// Set the DAC's mixer to `entry`, bit-perfect, then start an `AudioTrack` with the same
    /// attributes and format, preferring the DAC. What Android said comes back with the track.
    fn open(&self, dac_id: i32, entry: &MixerEntry) -> Result<(Box<dyn Track>, OpenReport), String>;
    /// Android's media volume, 0 to 100 (the DAC applies it in bit-perfect mode where it can).
    fn set_volume(&self, percent: u8) -> Result<(), String>;
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
/// would not start.
pub fn parse_open_answer(json: &str) -> Result<(i64, u64, OpenReport), String> {
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
    }
    let a: Answer =
        serde_json::from_str(json).map_err(|e| format!("the bit-perfect player's answer: {e}"))?;
    if !a.ok {
        return Err(a
            .why
            .unwrap_or_else(|| "the bit-perfect player would not start".into()));
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

/// What the phone's output takes now: the DAC's bit-perfect rates and sizes when bit-perfect is
/// offered and not turned off in Settings (with Android's media volume as the zone's volume when
/// "Use the DAC's volume" is on), else `shared`.
pub fn live_caps(dac: Option<&Dac>, settings: pm_zones::DeviceSettings, shared: &OutputCaps) -> OutputCaps {
    let Some(dac) = dac.filter(|d| offer(d).is_ok() && settings.mode != PreferredMode::Shared) else {
        return shared.clone();
    };
    let mut rates: Vec<u32> = bit_perfect_entries(dac).map(|e| e.rate).collect();
    rates.sort_unstable();
    rates.dedup();
    let mut bits: Vec<u8> = [16u8, 24, 32]
        .into_iter()
        .filter(|b| bit_perfect_entries(dac).any(|e| takes(e, *b)))
        .collect();
    bits.sort_unstable();
    let channels = bit_perfect_entries(dac).map(|e| e.channels).max().unwrap_or(2);
    OutputCaps {
        max_pcm_bits: bits.last().copied().unwrap_or(16),
        pcm_rates: rates,
        pcm_bits: bits,
        max_channels: channels.max(1),
        dsd: None,
        hw_volume: settings.dac_volume.then_some(VolumeKind::Percent),
        quality_ceiling: QualityCeiling::BitPerfect,
        ..shared.clone()
    }
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

/// How the phone's output opens `fmt` now.
pub fn plan(fmt: AudioFormat) -> Plan {
    let Some(dac) = usb_dac::current_dac() else {
        return Plan::Shared(None);
    };
    if usb_dac::settings().mode == PreferredMode::Shared {
        return Plan::Shared(Some((dac, "shared, as chosen in Settings".into())));
    }
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

/// Open the bit-perfect player for `fmt` and record what Android confirmed. `Err`: the player
/// could not start (why); the caller plays shared instead.
pub fn open(id: OutputId, dac: &Dac, choice: MixerChoice, fmt: AudioFormat) -> Result<TrackSink, String> {
    let b = backend().ok_or("this app has no bit-perfect player here")?;
    let gone_at_open = usb_dac::gone_count();
    let (track, report) = b.open(dac.id, &choice.entry)?;
    let confirmed = confirmation(&choice.entry, &report, dac.id);
    tracing::info!(format = %fmt, ?confirmed, "bit-perfect player");
    usb_dac::record_open(LastOpen {
        dac_id: dac.id,
        format: fmt,
        // The track plays either way; unconfirmed, Android may convert it: shared, and why
        mode: if confirmed.is_ok() {
            DeviceMode::BitPerfect
        } else {
            DeviceMode::Shared
        },
        confirmed: confirmed.is_ok(),
        note: confirmed.err(),
    });
    Ok(TrackSink {
        id,
        fmt,
        encoding: choice.entry.encoding,
        track,
        dac: dac.device.name.clone(),
        written: 0,
        base_written: 0,
        base_played: 0,
        gone_at_open,
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
    /// The DAC's name, for "FiiO KA17 was unplugged".
    dac: String,
    written: u64,
    /// `written` and the track's played frames at the last flush.
    base_written: u64,
    base_played: u64,
    gone_at_open: u64,
    bytes: Vec<u8>,
}

impl TrackSink {
    fn gone(&self) -> Option<SinkError> {
        (usb_dac::gone_count() != self.gone_at_open).then(|| self.gone_error())
    }

    fn gone_error(&self) -> SinkError {
        SinkError::Gone(self.id.clone(), format!("{} was unplugged", self.dac))
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

    fn flush(&mut self) -> Result<(), SinkError> {
        self.track.pause();
        self.track.flush();
        self.base_written = self.written;
        self.base_played = self.track.played_frames();
        self.track.play();
        Ok(())
    }

    fn drain(&mut self) -> Result<(), SinkError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while self.queued() > 0 && std::time::Instant::now() < deadline {
            if let Some(e) = self.gone() {
                return Err(e);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Ok(())
    }

    /// The track keeps its buffer: the same audio carries on after the pause.
    fn pause(&mut self) -> Result<bool, SinkError> {
        self.track.pause();
        Ok(true)
    }

    fn resume(&mut self) -> Result<(), SinkError> {
        self.ready_to_resume()?;
        self.track.play();
        Ok(())
    }

    fn ready_to_resume(&mut self) -> Result<(), SinkError> {
        match self.gone() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn frames_written(&self) -> u64 {
        self.written
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
        }
    }

    /// A player that records what it was asked and answers as told.
    #[derive(Default)]
    struct Fake {
        report: Mutex<OpenReport>,
        refuse: Mutex<Option<String>>,
        opened: Mutex<Vec<MixerEntry>>,
        bytes: Arc<Mutex<Vec<u8>>>,
        played: Arc<std::sync::atomic::AtomicU64>,
        gone: Arc<std::sync::atomic::AtomicBool>,
        volume: Mutex<Option<u8>>,
    }

    struct FakeTrack {
        bytes: Arc<Mutex<Vec<u8>>>,
        played: Arc<std::sync::atomic::AtomicU64>,
        gone: Arc<std::sync::atomic::AtomicBool>,
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
        fn play(&mut self) {}
        fn flush(&mut self) {}
        fn stop(&mut self) {}
    }

    impl TrackBackend for Fake {
        fn open(&self, _dac_id: i32, entry: &MixerEntry) -> Result<(Box<dyn Track>, OpenReport), String> {
            if let Some(why) = self.refuse.lock().unwrap().clone() {
                return Err(why);
            }
            self.opened.lock().unwrap().push(*entry);
            Ok((
                Box::new(FakeTrack {
                    bytes: self.bytes.clone(),
                    played: self.played.clone(),
                    gone: self.gone.clone(),
                }),
                self.report.lock().unwrap().clone(),
            ))
        }
        fn set_volume(&self, percent: u8) -> Result<(), String> {
            *self.volume.lock().unwrap() = Some(percent);
            Ok(())
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
        let caps = live_caps(Some(&ka17), DeviceSettings::default(), &shared_caps());
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
            caps.hw_volume,
            Some(VolumeKind::Percent),
            "the zone's volume is the DAC's"
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
        // Without the DAC's volume: the node's own volume (no hardware volume offered)
        let no_vol = DeviceSettings {
            dac_volume: false,
            ..DeviceSettings::default()
        };
        assert_eq!(live_caps(Some(&ka17), no_vol, &shared_caps()).hw_volume, None);
        // Shared in Settings, Android 13, no DAC: the shared caps, as today
        let shared_pref = DeviceSettings {
            mode: PreferredMode::Shared,
            ..DeviceSettings::default()
        };
        assert_eq!(live_caps(Some(&ka17), shared_pref, &shared_caps()), shared_caps());
        assert_eq!(
            live_caps(
                Some(&dac(33, ka17_mixer())),
                DeviceSettings::default(),
                &shared_caps()
            ),
            shared_caps()
        );
        assert_eq!(
            live_caps(None, DeviceSettings::default(), &shared_caps()),
            shared_caps()
        );
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
            open(OutputId::new("cpal", "phone"), &d, c, fmt).err().as_deref(),
            Some("another app holds the DAC")
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
        });
        match plan(fmt) {
            Plan::Shared(Some((_, why))) => assert_eq!(why, "shared, as chosen in Settings"),
            other => panic!("{other:?}"),
        }
        usb_dac::set_settings(DeviceSettings::default());
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
            parse_open_answer(r#"{"ok":false,"why":"the DAC is not attached"}"#).unwrap_err(),
            "the DAC is not attached"
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
}
