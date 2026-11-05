use async_trait::async_trait;
use pm_model::{AudioBlock, AudioFormat, OutputCaps, Volume};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

/// Stable identifier for an output: `<protocol>:<device>`, e.g.
/// `alsa:hw:0,0`, `openhome:uuid:…`, `wav:/tmp/out.wav`, `null`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OutputId(pub String);

impl OutputId {
    pub fn new(protocol: &str, device: &str) -> Self {
        if device.is_empty() {
            OutputId(protocol.to_string())
        } else {
            OutputId(format!("{protocol}:{device}"))
        }
    }

    pub fn protocol(&self) -> &str {
        self.0.split(':').next().unwrap_or(&self.0)
    }
}

impl fmt::Display for OutputId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a client sees when listing outputs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputInfo {
    pub id: OutputId,
    pub name: String,
    pub protocol: String,
    pub caps: OutputCaps,
    /// The device as its maker names it (`Klimax DSM`, `HomePod`), when the protocol tells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The hardware behind it, when the output knows: a USB DAC on a phone. Read leniently: a
    /// device a newer app describes in a way this node does not understand is `None`, never an
    /// error (it travels in every endpoint `Welcome`, and the output must keep working).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient_device"
    )]
    pub device: Option<OutputDevice>,
}

/// The hardware an output plays through, as it describes itself: a USB DAC on a phone. What the
/// output does with it now is in [`OutputInfo::caps`]; this is what the device itself takes, and
/// how the app drives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OutputDevice {
    /// "FiiO KA17".
    pub name: String,
    #[serde(default)]
    pub kind: DeviceKind,
    /// One line for a picker: "FiiO KA17 · PCM up to 768 kHz / 32 bit · DSD · hardware volume".
    #[serde(default)]
    pub summary: String,
    /// PCM rates the device takes, ascending (empty while unknown).
    #[serde(default)]
    pub rates: Vec<u32>,
    /// Integer PCM sample sizes it takes, ascending (16, 24, 32).
    #[serde(default)]
    pub bits: Vec<u8>,
    /// The highest of `rates` (0 while unknown).
    #[serde(default)]
    pub max_rate: u32,
    /// The highest of `bits` (0 while unknown).
    #[serde(default)]
    pub max_bits: u8,
    /// Most channels it takes (0 while unknown).
    #[serde(default)]
    pub channels: u8,
    /// DSD, when the device says how it takes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dsd: Option<DsdKind>,
    /// It has its own volume (a USB feature unit).
    #[serde(default)]
    pub hardware_volume: bool,
    /// How the app drives it now.
    #[serde(default)]
    pub mode: DeviceMode,
    /// The modes this device and phone allow.
    #[serde(default)]
    pub modes: Vec<DeviceMode>,
    /// The system confirmed `mode` for the format playing now (Android read the bit-perfect mixer
    /// back for that device and format). Never true for [`DeviceMode::Shared`]; a UI says
    /// "bit-perfect" only when this is true.
    #[serde(default)]
    pub confirmed: bool,
    /// Why it is not bit-perfect, or anything else the listener should know.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// USB ids, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usb: Option<UsbIds>,
    /// The format it was last opened in (what `mode` and `confirmed` are about).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<AudioFormat>,
    /// The listener's choices for it (`PUT /v1/outputs/{id}/device`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<DeviceSettings>,
}

/// The listener's choices for an [`OutputDevice`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceSettings {
    /// The mode to use: `auto` takes bit-perfect where the phone allows it, else shared.
    #[serde(default)]
    pub mode: PreferredMode,
    /// The zone's volume sets the device's own (Android's media volume, applied by the DAC in
    /// bit-perfect mode). Off: the node's software volume, which changes the samples.
    #[serde(default = "yes")]
    pub dac_volume: bool,
}

fn yes() -> bool {
    true
}

impl Default for DeviceSettings {
    fn default() -> Self {
        Self {
            mode: PreferredMode::Auto,
            dac_volume: true,
        }
    }
}

/// [`DeviceSettings::mode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PreferredMode {
    #[default]
    Auto,
    BitPerfect,
    Shared,
    /// A choice this node does not know yet: as `auto`.
    #[serde(other)]
    Unknown,
}

/// What kind of hardware an [`OutputDevice`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    /// A USB DAC (Android's `TYPE_USB_DEVICE`).
    #[default]
    UsbDac,
    /// A USB headset (Android's `TYPE_USB_HEADSET`).
    UsbHeadset,
    /// A kind this node does not know yet (from a newer app).
    #[serde(other)]
    Unknown,
}

/// How an app drives an [`OutputDevice`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DeviceMode {
    /// Through the system mixer, which may resample and mix in other sounds.
    #[default]
    Shared,
    /// The system passes the stream to the device unchanged (Android 14's bit-perfect mixer).
    BitPerfect,
    /// The app drives the device itself (its own USB audio driver).
    Exclusive,
    /// A mode this node does not know yet (from a newer app): shown as shared, never as more.
    #[serde(other)]
    Unknown,
}

/// How a device takes DSD.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DsdKind {
    /// As raw DSD (UAC2 `TYPE_I_RAW_DATA`).
    Native,
    /// Packed in PCM frames (DoP).
    Dop,
    /// A way this node does not know yet (from a newer app).
    #[serde(other)]
    Unknown,
}

/// `OutputInfo.device`, or `None` when it does not read as an [`OutputDevice`].
fn lenient_device<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<OutputDevice>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Lenient {
        Device(OutputDevice),
        Other(serde::de::IgnoredAny),
    }
    Ok(match Option::<Lenient>::deserialize(d)? {
        Some(Lenient::Device(dev)) => Some(dev),
        _ => None,
    })
}

/// A USB device's vendor and product ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsbIds {
    pub vendor_id: u16,
    pub product_id: u16,
}

#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    #[error("output {0} does not accept format {1}")]
    UnsupportedFormat(OutputId, AudioFormat),
    #[error("block format {found} does not match the open format {expected}")]
    FormatMismatch {
        expected: AudioFormat,
        found: AudioFormat,
    },
    #[error("float blocks must be converted to integer PCM before an output")]
    FloatNotAccepted,
    #[error("output {0} is not available: {1}")]
    Unavailable(OutputId, String),
    #[error("output {0} has no volume control")]
    NoVolume(OutputId),
    #[error("output {0} has no standby control")]
    NoStandby(OutputId),
    #[error("device error: {0}")]
    Device(String),
    /// The device the output played through went away mid-stream (a USB DAC unplugged from a
    /// phone): the zone pauses where it was instead of failing, and plays on no other device.
    /// Its text carries [`GONE`], so it survives an endpoint's error message.
    #[error("output {0}: device gone: {1}")]
    Gone(OutputId, String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// The words [`SinkError::Gone`] puts in its text, so a node can tell it from an endpoint's
/// error message.
pub const GONE: &str = "device gone: ";

impl SinkError {
    /// [`SinkError::Gone`], or an error whose text says a device went away (an endpoint's).
    pub fn gone_reason(&self) -> Option<String> {
        match self {
            SinkError::Gone(_, why) => Some(why.clone()),
            other => {
                let text = other.to_string();
                text.find(GONE)
                    .map(|at| text[at + GONE.len()..].trim().to_string())
            }
        }
    }
}

/// How full the sink's buffer is; the zone paces itself on this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BufferLevel {
    pub queued_frames: u64,
    pub capacity_frames: u64,
}

impl BufferLevel {
    pub fn fill(&self) -> f64 {
        if self.capacity_frames == 0 {
            0.0
        } else {
            self.queued_frames as f64 / self.capacity_frames as f64
        }
    }
}

/// A clock an output exposes, for grouping and drift correction (§6.5).
pub trait ClockSource: Send + Sync {
    /// Frames this clock has consumed since the sink opened.
    fn position_frames(&self) -> u64;
    /// The clock's rate in frames per second as the device reports it.
    fn rate_hz(&self) -> u32;
    /// Fixed latency between a frame being accepted and being audible.
    fn latency(&self) -> Duration;
}

/// One member of a grouped sink, as reported to the zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberStatus {
    pub output: OutputId,
    /// Audible position relative to the group's leader; positive = ahead.
    pub drift_ms: i64,
    pub level: BufferLevel,
    /// Net frames inserted (positive) or dropped (negative) to keep this
    /// member in time with the leader; 0 when nothing was corrected.
    #[serde(default)]
    pub slipped_frames: i64,
}

/// An open output. Methods block; call them from the zone's audio thread.
pub trait AudioSink: Send {
    /// The negotiated format. Every block written must match it exactly.
    fn format(&self) -> AudioFormat;

    /// Push one block. Blocks until the sink has room for it.
    fn write(&mut self, block: &AudioBlock) -> Result<(), SinkError>;

    fn buffer_level(&self) -> BufferLevel;

    /// Discard queued audio (stop / seek).
    fn flush(&mut self) -> Result<(), SinkError>;

    /// Play out queued audio and return when it is audible (end of track
    /// without gapless, or clean stop).
    fn drain(&mut self) -> Result<(), SinkError>;

    /// Pause: silence now. The default discards the queued audio ([`Self::flush`]) and
    /// returns false. A sink whose device buffers seconds of a stream it fetches (a UPnP
    /// renderer, a BluOS player) tells the device to pause instead, so it stops at once and
    /// keeps its buffer, and returns true: the zone then calls [`Self::resume`] before
    /// anything plays on it, and closes it instead when a new track or a seek makes that
    /// buffer stale. The zone writes nothing while paused.
    fn pause(&mut self) -> Result<bool, SinkError> {
        self.flush().map(|()| false)
    }

    /// Carry on after a [`Self::pause`] that returned true. An error means the sink cannot
    /// carry on (the device dropped the stream during a long pause): the zone opens the output
    /// again where the listener last heard it.
    fn resume(&mut self) -> Result<(), SinkError> {
        Ok(())
    }

    /// Whether [`Self::resume`] can carry on from what the device held, without resuming yet: an
    /// error when it would fail (the device was stopped for the pause, or dropped the stream). A
    /// group asks every member first, so none plays before another turns out to need a fresh
    /// start.
    fn ready_to_resume(&mut self) -> Result<(), SinkError> {
        Ok(())
    }

    /// Frames written so far, for position reporting.
    fn frames_written(&self) -> u64;

    /// How many more frames the sink's stream can carry, when it has a fixed size (a renderer's
    /// stream offered as a file of fixed length); `None` when there is no limit. The zone starts
    /// a track that doesn't fit on a fresh sink instead of joining it to this one.
    fn room_frames(&self) -> Option<u64> {
        None
    }

    /// Members of a grouped sink and their drift; empty for a single output.
    fn members(&self) -> Vec<MemberStatus> {
        Vec::new()
    }

    /// Synced group start (Slimproto): from now on (and after every flush) buffer without
    /// playing until `start_at`. Call before the first write. False when the sink cannot; it
    /// then starts as soon as it has audio, as before.
    fn hold(&mut self) -> bool {
        false
    }

    /// A held sink has buffered enough to start on time.
    fn ready(&self) -> bool {
        true
    }

    /// Start a held sink at `at` (the group passes the same instant to every member).
    fn start_at(&mut self, _at: std::time::Instant) -> Result<(), SinkError> {
        Ok(())
    }

    /// Move the playback point now, for drift correction: `frames` > 0 holds it back (the sink
    /// is ahead), < 0 skips ahead. For sinks whose device queues seconds of audio, where frames
    /// slipped into the stream would be heard only after that queue. False when not supported.
    fn nudge(&mut self, _frames: i64) -> bool {
        false
    }
}

/// How closely a zone's speakers stay together (the Zones view says it plainly).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SyncClass {
    /// One device.
    #[default]
    Single,
    /// Sample-accurate: the node's own endpoints (ADR 0009), Slimproto players, a Snapcast group,
    /// a Sonos group, a Cast group (each kept in sync by its own protocol).
    Tight,
    /// Started together, drifting apart within a second or so: devices on different protocols, or
    /// on a protocol without a shared clock (AirPlay 1, DLNA, single Cast devices).
    Loose,
}

/// What a renderer with a screen shows for a track (Cast, DLNA TVs, Sonos).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackMeta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// Cover art the device can fetch itself (an `http(s)` URL, no API token).
    pub image: Option<String>,
}

impl TrackMeta {
    /// The queue item's own fields (art only when it is a URL).
    pub fn from_item(item: &crate::QueueItem) -> Self {
        Self {
            title: item.title.clone(),
            artist: item.artist.clone(),
            album: item.album.clone(),
            image: item.image.clone().filter(|u| u.starts_with("http")),
        }
    }
}

/// Turns a queue item into what a renderer shows; the node fills artist, album and art from its
/// library.
pub type Describe = Arc<dyn Fn(&crate::QueueItem) -> TrackMeta + Send + Sync>;

/// One place audio can go (handoff §6.4).
#[async_trait]
pub trait Output: Send + Sync {
    fn id(&self) -> OutputId;
    fn name(&self) -> String;
    fn caps(&self) -> &OutputCaps;

    /// The device's maker and model in words, when the protocol tells (UPnP, AirPlay, Cast,
    /// BluOS); a UI shows it instead of the id.
    fn model(&self) -> Option<String> {
        None
    }

    fn info(&self) -> OutputInfo {
        OutputInfo {
            id: self.id(),
            name: self.name(),
            protocol: self.id().protocol().to_string(),
            caps: self.live_caps(),
            model: self.model(),
            device: self.device(),
        }
    }

    /// What the output takes now, when that changes while it is registered (a phone's output with
    /// a USB DAC in bit-perfect mode takes the DAC's rates): the zone plans each track with it.
    /// [`Output::caps`] stays what it was when the output was found.
    fn live_caps(&self) -> OutputCaps {
        self.caps().clone()
    }

    /// Apply the listener's choices for the hardware behind the output (see [`DeviceSettings`]).
    async fn configure_device(&self, _settings: DeviceSettings) -> Result<(), SinkError> {
        Err(SinkError::Unavailable(
            self.id(),
            "it has no device settings".into(),
        ))
    }

    /// The hardware behind this output, when it knows (see [`OutputDevice`]).
    fn device(&self) -> Option<OutputDevice> {
        None
    }

    /// Open the output in exactly `fmt`. Implementations must not convert:
    /// if the device cannot take `fmt` as-is, return
    /// [`SinkError::UnsupportedFormat`] so the pipeline negotiates and records
    /// the conversion in the signal path.
    async fn open(&self, fmt: AudioFormat) -> Result<Box<dyn AudioSink>, SinkError>;

    async fn set_volume(&self, _v: Volume) -> Result<(), SinkError> {
        Err(SinkError::NoVolume(self.id()))
    }

    async fn standby(&self, _on: bool) -> Result<(), SinkError> {
        Err(SinkError::NoStandby(self.id()))
    }

    /// The output's clock, if it exposes one.
    fn clock(&self) -> Option<Arc<dyn ClockSource>> {
        None
    }

    /// How closely this output's speakers stay together.
    fn sync(&self) -> SyncClass {
        SyncClass::Single
    }

    /// The track about to play (before its sink opens, when one opens) or `None` when the zone
    /// stops. Outputs that show metadata (a TV, a Cast screen) keep it; the default ignores it.
    fn now_playing(&self, _item: Option<&crate::QueueItem>) {}
}

/// Reject a block the sink cannot take: wrong format or still float.
pub fn check_block(expected: AudioFormat, block: &AudioBlock) -> Result<(), SinkError> {
    if block.is_float() {
        return Err(SinkError::FloatNotAccepted);
    }
    if block.format != expected {
        return Err(SinkError::FormatMismatch {
            expected,
            found: block.format,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An output's info as a Welcome carries it, with `device` as given.
    fn info_with(device: serde_json::Value) -> serde_json::Value {
        let mut v = serde_json::to_value(crate::outputs::NullOutput::new().info()).unwrap();
        v["device"] = device;
        v
    }

    #[test]
    fn a_device_from_a_newer_app_never_breaks_the_output() {
        // Values this node does not know: read as Unknown (shown as shared), the rest as sent
        let v = info_with(
            serde_json::json!({"name": "FiiO KA17", "kind": "usb_c_hub", "mode": "hyper_perfect",
            "modes": ["shared", "hyper_perfect"], "confirmed": true, "dsd": "dsd_over_usb4", "max_rate": 768000,
            "future_field": {"anything": [1, 2]}}),
        );
        let info: OutputInfo = serde_json::from_value(v).unwrap();
        let d = info.device.unwrap();
        assert_eq!(
            (d.kind, d.mode, d.dsd),
            (DeviceKind::Unknown, DeviceMode::Unknown, Some(DsdKind::Unknown))
        );
        assert_eq!(d.modes, [DeviceMode::Shared, DeviceMode::Unknown]);
        assert_eq!((d.name.as_str(), d.max_rate), ("FiiO KA17", 768_000));
        // A device this node can't read at all: no device, and the output itself still reads
        for odd in [
            serde_json::json!({"name": "FiiO KA17", "rates": "many"}),
            serde_json::json!({"mode": 3}),
            serde_json::json!("a string"),
            serde_json::json!([1, 2]),
            serde_json::Value::Null,
        ] {
            let info: OutputInfo = serde_json::from_value(info_with(odd.clone())).unwrap();
            assert_eq!(info.device, None, "{odd}");
            assert_eq!(info.id, OutputId::new("null", ""));
        }
        // Absent: none, as from an older app
        let mut v = info_with(serde_json::Value::Null);
        v.as_object_mut().unwrap().remove("device");
        assert_eq!(serde_json::from_value::<OutputInfo>(v).unwrap().device, None);
    }
}
