//! Cross-platform local output through `cpal` (CoreAudio on macOS, WASAPI on
//! Windows, AAudio on Android, ALSA elsewhere), for the app's built-in player
//! where there is no `hw:` device of ours. It runs in the operating system's shared mode:
//! the OS mixer may resample to the device rate and mixes in float, so the
//! capability ceiling is "lossless up to the device rate", never bit-perfect.
//! Samples are handed over as 32-bit float, which is lossless for the 16- and
//! 24-bit PCM the pipeline produces.
//!
//! On Android the app offers one output: the one Android routes to (the
//! speaker, wired or Bluetooth headphones, a USB DAC), followed as the user
//! plugs things in. Android's mixer runs at 48 kHz on nearly every phone, so
//! the output takes 44.1 and 48 kHz and the node converts anything higher. A
//! USB DAC Android routes to is described by the app (`usb_dac`) and reported
//! as the output's device, still in shared mode.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use pm_model::{AudioBlock, AudioFormat, OutputCaps, QualityCeiling, SampleKind, Samples, Volume};
use pm_zones::{AudioSink, BufferLevel, Output, OutputId, SinkError};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const STANDARD_RATES: [u32; 9] = [
    44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000, 768_000,
];
/// How much audio the sink keeps queued for the callback.
const QUEUE_SECONDS: f64 = 0.5;

pub struct CpalOutput {
    device_name: String,
    /// `None`: the host's default output, followed as the system reroutes.
    device_id: Option<cpal::DeviceId>,
    caps: OutputCaps,
}

/// Android's mixer rate on nearly every phone (`PROPERTY_OUTPUT_SAMPLE_RATE`).
#[cfg(target_os = "android")]
const ANDROID_RATES: [u32; 2] = [44_100, 48_000];

impl CpalOutput {
    /// The system's default output device, if any.
    pub fn default_device() -> Option<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device()?;
        let mut out = Self::from_device(&device)?;
        out.device_id = None;
        Some(out)
    }

    /// Every output device the host lists (on Android: the routed output).
    pub fn enumerate() -> Vec<Self> {
        #[cfg(target_os = "android")]
        {
            Self::default_device().map(Self::android).into_iter().collect()
        }
        #[cfg(not(target_os = "android"))]
        {
            let host = cpal::default_host();
            host.output_devices()
                .map(|it| it.filter_map(|d| Self::from_device(&d)).collect())
                .unwrap_or_default()
        }
    }

    #[cfg(target_os = "android")]
    fn android(mut self) -> Self {
        self.device_name = "Speaker or headphones".into();
        self.caps.pcm_rates = ANDROID_RATES.to_vec();
        self.caps.quality_ceiling = QualityCeiling::LosslessUpTo {
            rate_hz: 48_000,
            bits: 24,
        };
        self
    }

    fn from_device(device: &cpal::Device) -> Option<Self> {
        let device_name = device.to_string();
        let device_id = device.id().ok();
        let configs: Vec<cpal::SupportedStreamConfigRange> =
            device.supported_output_configs().ok()?.collect();
        if configs.is_empty() {
            return None;
        }
        let mut pcm_rates: Vec<u32> = STANDARD_RATES
            .into_iter()
            .filter(|r| {
                configs
                    .iter()
                    .any(|c| c.min_sample_rate() <= *r && *r <= c.max_sample_rate() && c.channels() >= 2)
            })
            .collect();
        if pcm_rates.is_empty() {
            pcm_rates = vec![device
                .default_output_config()
                .map(|c| c.sample_rate())
                .unwrap_or(48_000)];
        }
        let max_rate = pcm_rates.iter().copied().max().unwrap_or(48_000);
        let max_channels = configs.iter().map(|c| c.channels()).max().unwrap_or(2).min(8) as u8;
        Some(Self {
            device_name,
            device_id,
            caps: OutputCaps {
                pcm_rates,
                max_pcm_bits: 24,
                pcm_bits: vec![16, 24],
                max_channels,
                dsd: None,
                hw_volume: None,
                standby: false,
                source_switch: false,
                gapless: true,
                groupable: false,
                fixed_latency_ms: None,
                quality_ceiling: QualityCeiling::LosslessUpTo {
                    rate_hz: max_rate,
                    bits: 24,
                },
            },
        })
    }
}

fn host_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "CoreAudio"
    } else if cfg!(target_os = "windows") {
        "WASAPI"
    } else if cfg!(target_os = "android") {
        "Android audio"
    } else {
        "system audio"
    }
}

#[async_trait::async_trait]
impl Output for CpalOutput {
    fn id(&self) -> OutputId {
        OutputId::new("cpal", &self.device_name)
    }

    fn name(&self) -> String {
        format!("{} ({}, shared mode)", self.device_name, host_label())
    }

    /// On Android, the USB DAC the app reported (`usb_dac`), played in shared mode. The name and
    /// id stay the phone's: a zone is named once, from its output's name, and a DAC comes and goes.
    fn device(&self) -> Option<pm_zones::OutputDevice> {
        #[cfg(target_os = "android")]
        return crate::usb_dac::current();
        #[cfg(not(target_os = "android"))]
        None
    }

    fn caps(&self) -> &OutputCaps {
        &self.caps
    }

    /// On Android with a USB DAC that plays bit-perfect: the DAC's rates and sizes (the node stops
    /// converting to 44.1/48), its volume through Android's media volume.
    fn live_caps(&self) -> OutputCaps {
        #[cfg(target_os = "android")]
        return crate::bit_perfect::phone_caps(&self.caps);
        #[cfg(not(target_os = "android"))]
        self.caps.clone()
    }

    /// The phone's DAC settings (bit-perfect or shared, the DAC's volume), kept by the app.
    async fn configure_device(&self, settings: pm_zones::DeviceSettings) -> Result<(), SinkError> {
        #[cfg(target_os = "android")]
        {
            crate::usb_dac::set_settings(settings);
            Ok(())
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = settings;
            Err(SinkError::Unavailable(
                self.id(),
                "it has no device settings".into(),
            ))
        }
    }

    async fn open(&self, fmt: AudioFormat) -> Result<Box<dyn AudioSink>, SinkError> {
        // A phone with a USB DAC: bit-perfect where Android allows it for this format; otherwise
        // shared, and the device says why. The bit-perfect player failing to start is no error:
        // Android's shared path plays it at its own rate then (Android converts, the device says so)
        #[cfg(target_os = "android")]
        let any_rate = match crate::bit_perfect::plan(fmt) {
            crate::bit_perfect::Plan::BitPerfect(dac, choice, warning) => {
                match crate::bit_perfect::open(self.id(), &dac, choice, warning, fmt) {
                    Ok(s) => return Ok(Box::new(s)),
                    Err(why) => {
                        tracing::warn!(format = %fmt, "no bit-perfect player: {why}");
                        crate::bit_perfect::record_shared(&dac, fmt, why);
                        true
                    }
                }
            }
            // Planned for the DAC's caps but played shared (a mono track, Settings changed since):
            // Android's mixer takes the track's rate, never a stopped zone
            crate::bit_perfect::Plan::Shared(Some((dac, why))) => {
                crate::bit_perfect::record_shared(&dac, fmt, why);
                true
            }
            crate::bit_perfect::Plan::Shared(None) => false,
        };
        #[cfg(not(target_os = "android"))]
        let any_rate = false;
        if fmt.kind != SampleKind::Pcm
            || (!any_rate && !self.caps.pcm_rates.contains(&fmt.rate_hz))
            || fmt.channels < 1
        {
            return Err(SinkError::UnsupportedFormat(self.id(), fmt));
        }
        CpalSink::open(&self.device_name, self.device_id.clone(), self.id(), fmt)
            .map(|s| Box::new(s) as Box<dyn AudioSink>)
    }

    /// No volume of its own: Android's (the phone's buttons) in shared mode; in bit-perfect mode
    /// the level is the DAC's. The zone's software volume (DSP) still works, and the signal path
    /// then says it changes the samples.
    async fn set_volume(&self, _v: Volume) -> Result<(), SinkError> {
        Err(SinkError::NoVolume(self.id()))
    }
}

struct Shared {
    queue: Mutex<VecDeque<f32>>,
    /// Frames the callback has consumed (what the user hears, roughly).
    played: AtomicU64,
    underruns: AtomicU64,
    stop: AtomicBool,
    /// The stream ended by itself (its device went away: Android disconnects it).
    dead: AtomicBool,
}

/// The `cpal::Stream` is not `Send` on every platform, so it lives on its own
/// thread; the sink talks to it through the shared queue.
pub struct CpalSink {
    id: OutputId,
    fmt: AudioFormat,
    shared: Arc<Shared>,
    capacity_frames: u64,
    written: u64,
    thread: Option<std::thread::JoinHandle<()>>,
    /// Opened with a phone's USB DAC attached: its name and the count of DACs gone then. When it
    /// goes, the sink stops (the zone pauses) instead of playing on from the phone's speaker.
    dac_at_open: Option<(String, u64)>,
    /// The phone's settings-or-DAC generation at open (a change closes it at the next song).
    generation: u64,
}

impl CpalSink {
    fn open(
        device_name: &str,
        device_id: Option<cpal::DeviceId>,
        id: OutputId,
        fmt: AudioFormat,
    ) -> Result<Self, SinkError> {
        let shared = Arc::new(Shared {
            queue: Mutex::new(VecDeque::new()),
            played: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            dead: AtomicBool::new(false),
        });
        let capacity_frames = (fmt.rate_hz as f64 * QUEUE_SECONDS) as u64;
        // A phone's USB DAC at open: once it goes, the callback plays silence at once
        let dac_at_open =
            crate::usb_dac::current_dac().map(|d| (d.device.name, crate::usb_dac::gone_count()));
        let gone_at = dac_at_open.as_ref().map(|(_, g)| *g);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let name = device_name.to_string();
        let channels = fmt.channels as usize;
        let s2 = shared.clone();
        let thread = std::thread::Builder::new()
            .name("cpal-output".into())
            .spawn(move || {
                let host = cpal::default_host();
                let device = match &device_id {
                    Some(id) => host.device_by_id(id),
                    None => host.default_output_device(),
                };
                let Some(device) = device else {
                    let _ = ready_tx.send(Err(format!("output device {name} is gone")));
                    return;
                };
                let config = cpal::StreamConfig {
                    channels: channels as u16,
                    sample_rate: fmt.rate_hz,
                    buffer_size: cpal::BufferSize::Default,
                };
                let s3 = s2.clone();
                let stream = device.build_output_stream(
                    config,
                    move |out: &mut [f32], _| {
                        if gone_at.is_some_and(|g| crate::usb_dac::gone_count_now() != g) {
                            out.fill(0.0);
                            return;
                        }
                        let mut q = s3.queue.lock().unwrap_or_else(|p| p.into_inner());
                        let have = q.len().min(out.len());
                        for (i, v) in out.iter_mut().enumerate() {
                            *v = if i < have {
                                q.pop_front().unwrap_or(0.0)
                            } else {
                                0.0
                            };
                        }
                        if have < out.len() && have > 0 {
                            s3.underruns.fetch_add(1, Ordering::Relaxed);
                        }
                        s3.played
                            .fetch_add((have / channels.max(1)) as u64, Ordering::Relaxed);
                    },
                    {
                        let s4 = s2.clone();
                        move |e| {
                            tracing::warn!("cpal stream error: {e}");
                            s4.dead.store(true, Ordering::SeqCst);
                        }
                    },
                    None,
                );
                let stream = match stream {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e.to_string()));
                        return;
                    }
                };
                if let Err(e) = stream.play() {
                    let _ = ready_tx.send(Err(e.to_string()));
                    return;
                }
                let _ = ready_tx.send(Ok(()));
                while !s2.stop.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                drop(stream);
            })
            .map_err(|e| SinkError::Device(e.to_string()))?;
        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(SinkError::Unavailable(id, e)),
            Err(_) => return Err(SinkError::Unavailable(id, "audio thread ended".into())),
        }
        Ok(Self {
            id,
            fmt,
            shared,
            capacity_frames,
            written: 0,
            thread: Some(thread),
            dac_at_open,
            generation: crate::usb_dac::generation(),
        })
    }

    fn queued_frames(&self) -> u64 {
        let q = self.shared.queue.lock().unwrap_or_else(|p| p.into_inner());
        (q.len() / self.fmt.channels.max(1) as usize) as u64
    }

    /// The phone's DAC this sink opened on went away, or its stream ended by itself with a DAC.
    fn gone(&self) -> Option<SinkError> {
        let (name, at) = self.dac_at_open.as_ref()?;
        (crate::usb_dac::gone_count() != *at || self.shared.dead.load(Ordering::SeqCst)).then(|| {
            let why = crate::usb_dac::gone_reason();
            SinkError::Gone(
                self.id.clone(),
                if why.is_empty() {
                    format!("{name} was unplugged")
                } else {
                    why
                },
            )
        })
    }

    /// Wait until the queue has room for `frames` more; never past a DAC that went away (its
    /// stream stops taking audio, and the wait would never end).
    fn wait_for_room(&self, frames: u64) -> Result<(), SinkError> {
        while self.queued_frames() + frames > self.capacity_frames * 2 {
            if self.shared.stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            if let Some(e) = self.gone() {
                return Err(e);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Ok(())
    }
}

impl AudioSink for CpalSink {
    fn format(&self) -> AudioFormat {
        self.fmt
    }

    fn write(&mut self, block: &AudioBlock) -> Result<(), SinkError> {
        if block.format != self.fmt {
            return Err(SinkError::FormatMismatch {
                expected: self.fmt,
                found: block.format,
            });
        }
        if let Some(e) = self.gone() {
            self.flush()?;
            return Err(e);
        }
        let scale = 1.0 / (1u64 << (self.fmt.bits.saturating_sub(1).min(31))) as f32;
        let samples: Vec<f32> = match &block.samples {
            Samples::PcmI32(s) => s.iter().map(|&v| v as f32 * scale).collect(),
            Samples::PcmF64(s) => s.iter().map(|&v| v as f32).collect(),
            Samples::Dsd(_) => return Err(SinkError::UnsupportedFormat(self.id.clone(), block.format)),
        };
        if let Err(e) = self.wait_for_room(block.frames() as u64) {
            self.flush()?;
            return Err(e);
        }
        self.shared
            .queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .extend(samples);
        self.written += block.frames() as u64;
        Ok(())
    }

    fn buffer_level(&self) -> BufferLevel {
        BufferLevel {
            queued_frames: self.queued_frames(),
            capacity_frames: self.capacity_frames * 2,
        }
    }

    fn flush(&mut self) -> Result<(), SinkError> {
        self.shared
            .queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
        Ok(())
    }

    fn drain(&mut self) -> Result<(), SinkError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while self.queued_frames() > 0 && std::time::Instant::now() < deadline {
            if let Some(e) = self.gone() {
                self.flush()?;
                return Err(e);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // Let the device play out what the OS still buffers.
        std::thread::sleep(std::time::Duration::from_millis(50));
        Ok(())
    }

    fn frames_written(&self) -> u64 {
        self.written
    }

    /// A phone's DAC settings or its DAC changed since it opened: the next song opens afresh.
    fn reopen_at_track(&self) -> bool {
        crate::usb_dac::generation() != self.generation
    }
}

impl Drop for CpalSink {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let underruns = self.shared.underruns.load(Ordering::Relaxed);
        if underruns > 0 {
            tracing::debug!(output = %self.id, underruns, "cpal sink closed");
        }
    }
}
