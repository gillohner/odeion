use crate::sink::{check_block, AudioSink, BufferLevel, Output, OutputId, SinkError};
use async_trait::async_trait;
use pm_model::{AudioBlock, AudioFormat, OutputCaps, Volume, VolumeKind, VolumeLevel, ZoneVolume};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How far a real-time null sink may run ahead of the wall clock.
const AHEAD: Duration = Duration::from_millis(250);

/// Accepts every format and drops the audio. Counts frames so tests and the
/// API can observe progress. [`NullOutput::realtime`] plays at the speed of
/// the music, like a DAC would (soak tests, mock rooms); the default drops
/// audio as fast as it is decoded.
pub struct NullOutput {
    caps: OutputCaps,
    frames: Arc<AtomicU64>,
    opens: AtomicU64,
    realtime: bool,
    /// A pretend hardware volume (the `with_volume` fixtures): the level set last, read back
    /// like a device's.
    level: Option<Mutex<ZoneVolume>>,
}

impl Default for NullOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl NullOutput {
    pub fn new() -> Self {
        Self {
            caps: OutputCaps::unlimited(),
            frames: Arc::new(AtomicU64::new(0)),
            opens: AtomicU64::new(0),
            realtime: false,
            level: None,
        }
    }

    /// A null output with a hardware volume of `kind` (dB or percent), for the pages' and the
    /// API's volume checks: `null:db` and `null:percent`, next to the plain one with none. The
    /// level starts at the top of the scale and keeps what it is set to, mute included.
    pub fn with_volume(kind: VolumeKind) -> Self {
        let mut caps = OutputCaps::unlimited();
        let level = match kind {
            VolumeKind::Db { max, .. } => VolumeLevel::Db(max),
            _ => VolumeLevel::Percent(100),
        };
        caps.hw_volume = Some(kind);
        Self {
            caps,
            level: Some(Mutex::new(ZoneVolume { level, muted: false })),
            ..Self::new()
        }
    }

    /// A null output paced by the wall clock.
    pub fn realtime() -> Self {
        Self {
            realtime: true,
            ..Self::new()
        }
    }

    /// The real-time variant when `PM_NULL_OUTPUT=realtime` is set (tests
    /// that need playback to take as long as the music).
    pub fn from_env() -> Self {
        match std::env::var("PM_NULL_OUTPUT").as_deref() {
            Ok("realtime") => Self::realtime(),
            _ => Self::new(),
        }
    }

    /// Restrict what the output accepts (tests: a "16/44.1-only" device).
    pub fn with_caps(caps: OutputCaps) -> Self {
        Self { caps, ..Self::new() }
    }

    /// The device's own level changed behind the node's back (its remote): what the next read
    /// sees. Only an output made with [`NullOutput::with_volume`] has one.
    pub fn pretend_level(&self, v: ZoneVolume) {
        if let Some(l) = &self.level {
            *l.lock().expect("null volume") = v;
        }
    }

    /// How many sinks were opened on this output (gapless = one per format run).
    pub fn opens_total(&self) -> u64 {
        self.opens.load(Ordering::Relaxed)
    }

    /// Total frames written through every sink opened on this output.
    pub fn frames_total(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl Output for NullOutput {
    fn id(&self) -> OutputId {
        match self.caps.hw_volume {
            Some(VolumeKind::Db { .. }) => OutputId::new("null", "db"),
            Some(_) => OutputId::new("null", "percent"),
            None => OutputId::new("null", ""),
        }
    }

    fn name(&self) -> String {
        match self.caps.hw_volume {
            Some(VolumeKind::Db { .. }) => "Null output (dB volume)".into(),
            Some(_) => "Null output (% volume)".into(),
            None => "Null output".into(),
        }
    }

    async fn set_volume(&self, v: Volume) -> Result<(), SinkError> {
        let Some(level) = &self.level else {
            return Err(SinkError::NoVolume(self.id()));
        };
        let mut l = level.lock().expect("null volume");
        let (min, max, step) = match self.caps.hw_volume {
            Some(VolumeKind::Db { min, max, step }) => (min, max, step.max(0.5)),
            _ => (0.0, 100.0, 3.0),
        };
        let cur = match l.level {
            VolumeLevel::Db(d) => d,
            VolumeLevel::Percent(p) => f32::from(p),
        };
        let next = match v {
            Volume::Db(d) => d.clamp(min, max),
            Volume::Percent(p) => match l.level {
                VolumeLevel::Db(_) => min + (max - min) * f32::from(p.min(100)) / 100.0,
                VolumeLevel::Percent(_) => f32::from(p.min(100)),
            },
            Volume::Up => (cur + step).min(max),
            Volume::Down => (cur - step).max(min),
            Volume::Mute(m) => {
                l.muted = m;
                return Ok(());
            }
        };
        l.level = match l.level {
            VolumeLevel::Db(_) => VolumeLevel::Db(next),
            VolumeLevel::Percent(_) => VolumeLevel::Percent(next.round() as u8),
        };
        Ok(())
    }

    async fn volume(&self) -> Result<Option<ZoneVolume>, SinkError> {
        Ok(self.level.as_ref().map(|l| *l.lock().expect("null volume")))
    }

    fn caps(&self) -> &OutputCaps {
        &self.caps
    }

    async fn open(&self, fmt: AudioFormat) -> Result<Box<dyn AudioSink>, SinkError> {
        fmt.validate().map_err(|e| SinkError::Device(e.to_string()))?;
        if !self.caps.accepts(&fmt) {
            return Err(SinkError::UnsupportedFormat(self.id(), fmt));
        }
        self.opens.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(NullSink {
            fmt,
            written: 0,
            total: self.frames.clone(),
            clock: self.realtime.then(|| (Instant::now(), 0)),
        }))
    }
}

struct NullSink {
    fmt: AudioFormat,
    written: u64,
    total: Arc<AtomicU64>,
    /// Real-time pacing: the instant and frame count playback is measured from.
    clock: Option<(Instant, u64)>,
}

impl AudioSink for NullSink {
    fn format(&self) -> AudioFormat {
        self.fmt
    }

    fn write(&mut self, block: &AudioBlock) -> Result<(), SinkError> {
        check_block(self.fmt, block)?;
        let n = block.frames() as u64;
        self.written += n;
        self.total.fetch_add(n, Ordering::Relaxed);
        if let Some((start, base)) = self.clock {
            let rate = f64::from(self.fmt.rate_hz.max(1));
            let played = Duration::from_secs_f64((self.written - base) as f64 / rate);
            let elapsed = start.elapsed();
            if elapsed > played + AHEAD {
                // Writing resumed after a pause: measure from now.
                self.clock = Some((Instant::now(), self.written));
            } else if played > elapsed + AHEAD {
                std::thread::sleep(played - elapsed - AHEAD);
            }
        }
        Ok(())
    }

    fn buffer_level(&self) -> BufferLevel {
        BufferLevel {
            queued_frames: 0,
            capacity_frames: 0,
        }
    }

    fn flush(&mut self) -> Result<(), SinkError> {
        // After a seek the clock starts again from here.
        if self.clock.is_some() {
            self.clock = Some((Instant::now(), self.written));
        }
        Ok(())
    }

    fn drain(&mut self) -> Result<(), SinkError> {
        // A real-time sink is done when the music would have finished playing.
        if let Some((start, base)) = self.clock {
            let rate = f64::from(self.fmt.rate_hz.max(1));
            let played = Duration::from_secs_f64((self.written - base) as f64 / rate);
            std::thread::sleep(played.saturating_sub(start.elapsed()));
        }
        Ok(())
    }

    fn frames_written(&self) -> u64 {
        self.written
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_realtime_variant_keeps_pace_with_the_music() {
        let out = NullOutput::realtime();
        let fmt = AudioFormat::pcm(8000, 16, 1);
        let mut sink = out.open(fmt).await.unwrap();
        let block = AudioBlock::pcm_i32(fmt, vec![0; 800]); // 100 ms
        let t = Instant::now();
        for _ in 0..6 {
            sink.write(&block).unwrap();
        }
        // 600 ms of audio, up to 250 ms ahead: at least ~350 ms of wall time.
        assert!(t.elapsed() >= Duration::from_millis(300), "{:?}", t.elapsed());
    }

    use pm_model::Samples;

    #[tokio::test]
    async fn counts_frames_and_rejects_mismatch() {
        let out = NullOutput::new();
        let fmt = AudioFormat::pcm(44_100, 16, 2);
        let mut sink = out.open(fmt).await.unwrap();
        sink.write(&AudioBlock::pcm_i32(fmt, vec![0; 8])).unwrap();
        assert_eq!(sink.frames_written(), 4);
        assert_eq!(out.frames_total(), 4);
        let err = sink.write(&AudioBlock {
            format: fmt,
            samples: Samples::PcmF64(vec![0.0; 2]),
        });
        assert!(matches!(err, Err(SinkError::FloatNotAccepted)));
        let err = sink.write(&AudioBlock::pcm_i32(AudioFormat::pcm(48_000, 16, 2), vec![0; 2]));
        assert!(matches!(err, Err(SinkError::FormatMismatch { .. })));
    }
}
