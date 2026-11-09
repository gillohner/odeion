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
    /// The exact formats it takes bit-perfect now, when it plays bit-perfect: the node plans each
    /// source with these ([`caps_for_source`]), so a listed format reaches it unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formats: Vec<DeviceFormat>,
    /// The phone's media volume, 0 to 100, when it matters (bit-perfect plays at full level).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_volume: Option<u8>,
}

/// One format a device takes bit-perfect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceFormat {
    pub rate_hz: u32,
    pub bits: u8,
    pub channels: u8,
}

/// The caps to plan `src` with on an output whose device lists the exact formats it takes: the
/// source's own format when listed (it reaches the device unchanged), else the closest listed one
/// (the same rate before another; no fewer bits before fewer; the same rate family; the nearest
/// rate). `caps` as they are when nothing is listed for the source's channels.
pub fn caps_for_source(formats: &[DeviceFormat], src: AudioFormat, caps: &OutputCaps) -> OutputCaps {
    if src.kind != pm_model::SampleKind::Pcm {
        return caps.clone();
    }
    let family = |r: u32| r.is_multiple_of(44_100);
    let best = formats
        .iter()
        .filter(|f| f.channels == src.channels)
        .min_by_key(|f| {
            (
                f.rate_hz != src.rate_hz,
                f.bits < src.bits,
                family(f.rate_hz) != family(src.rate_hz),
                f.rate_hz.abs_diff(src.rate_hz),
                f.bits.abs_diff(src.bits),
            )
        });
    match best {
        Some(f) => OutputCaps {
            pcm_rates: vec![f.rate_hz],
            pcm_bits: vec![f.bits],
            max_pcm_bits: f.bits,
            max_channels: caps.max_channels.max(f.channels),
            ..caps.clone()
        },
        None => caps.clone(),
    }
}

/// The listener's choices for an [`OutputDevice`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceSettings {
    /// The mode to use: `auto` takes bit-perfect where the phone allows it, else shared.
    #[serde(default)]
    pub mode: PreferredMode,
    /// Reserved: kept so earlier apps' settings still read. The phone's DAC has no zone volume of
    /// its own (in bit-perfect mode the level is the DAC's).
    #[serde(default = "yes")]
    pub dac_volume: bool,
    /// The listener agreed that Bit-perfect plays at full level below Android's full volume (asked
    /// when they chose it); for the DAC attached then only.
    #[serde(default)]
    pub full_level_ok: bool,
}

fn yes() -> bool {
    true
}

impl Default for DeviceSettings {
    fn default() -> Self {
        Self {
            mode: PreferredMode::Auto,
            dac_volume: true,
            full_level_ok: false,
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
    /// The app drives the DAC itself (its own USB driver; Android's permission is asked).
    Exclusive,
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

    /// The output's settings changed since this sink opened (a phone's DAC switched between
    /// bit-perfect and shared): the zone closes it at the next song and opens afresh, so the change
    /// applies there even when the format stays the same.
    fn reopen_at_track(&self) -> bool {
        false
    }

    /// The Gone this sink returned reached the zone (it pauses there with the reason). A sink
    /// that remembers a device's loss for the next open (a phone's DAC: the next open pauses
    /// instead of opening the speaker) may forget it now; a Gone nobody heard (a stream closed
    /// with it) is kept.
    fn gone_reported(&mut self) {}

    /// The sink may not carry on as it is with the next write: a phone's DAC paused in
    /// bit-perfect mode whose listener turned Android's volume down since (writing would put it
    /// back at full level without consent). Whoever writes opens the output again in the same
    /// format first (planned as it is now: shared) and writes there.
    fn reopen_now(&self) -> bool {
        false
    }

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

/// The highest percent level at or below `db` as an amplitude (`10^(dB/20)`), as BluOS, MPD and
/// Cast map it. Rounded down: it caps a request at a volume limit.
pub fn amplitude_percent(db: f32) -> u8 {
    (10f64.powf(f64::from(db.clamp(-80.0, 0.0)) / 20.0) * 100.0).floor() as u8
}

/// What [`Output::unmute_held`] did.
#[derive(Debug)]
pub struct Unmuted {
    /// Some device came back: one, a member of a group, a player of a native group that unmuted
    /// before another refused, or a device whose level write already unmuted it.
    pub any: bool,
    /// The first refusal.
    pub error: Option<SinkError>,
    /// Every device that came back is at or under the limit as far as is known (by its reading,
    /// or by a level write that went through). False when a correction it needed was refused:
    /// the zone then keeps showing the level it had, so the next unmute corrects again.
    pub held: bool,
}

impl Unmuted {
    /// Nothing came back.
    pub fn nothing(e: SinkError) -> Self {
        Self {
            any: false,
            error: Some(e),
            held: true,
        }
    }

    /// One device's outcome folded into a group's: the group came back when any member did; the
    /// first refusal is kept; held when every member that came back is.
    pub fn and(mut self, m: Unmuted) -> Self {
        self.any |= m.any;
        self.held &= m.held || !m.any;
        if self.error.is_none() {
            self.error = m.error;
        }
        self
    }
}

/// The zone's last level, asked for when it is needed (a level the device announced since
/// replaces the one the unmute began with).
pub type LastLevel<'a> = Option<&'a (dyn Fn() -> Option<pm_model::VolumeLevel> + Send + Sync)>;

/// [`Output::unmute_held`] for each of a group's `players` in turn (a node group's members, or the
/// players of a native Sonos, HEOS or BluOS group), each by its own level: none is given the
/// group's (a player may have been turned down on its own); `last`, the group's, only tells
/// whether one that can't report may be above the limit.
pub async fn unmute_each(players: &[&dyn Output], level: Volume, max: f32, last: LastLevel<'_>) -> Unmuted {
    let mut out = Unmuted {
        any: false,
        error: None,
        held: true,
    };
    for p in players {
        out = out.and(p.unmute_held(level, max, last, false).await);
    }
    out
}

/// One place audio can go (handoff §6.4).

#[async_trait]
pub trait Output: Send + Sync {
    fn id(&self) -> OutputId;
    fn name(&self) -> String;
    fn caps(&self) -> &OutputCaps;
    /// Whether unmuting brings the device back to the level it had before the mute (a mute
    /// switch: most outputs). `false` where the device has no mute of its own and unmuting is a
    /// level the output chooses (AirPlay receivers, Squeezebox players): the zone then holds that
    /// level to its limit itself.
    fn unmute_restores_level(&self) -> bool {
        true
    }

    /// Whether a level write alone makes the device heard (AirPlay, Squeezebox: mute is a level),
    /// so a level that went through unmuted it whatever the unmute after it says.
    fn level_write_unmutes(&self) -> bool {
        false
    }

    /// Unmute under the zone's limit `max`, never raising the device. `last` is the zone's level;
    /// `sole`: the zone is the device's only controller (a single device), so `last` is its
    /// level; a group's member may have been set apart, so `last` never sets it.
    ///
    /// One that chooses its level on unmuting gets `level` (the zone's level held to the limit)
    /// first. One that restores its own is brought down to the limit while still muted when its
    /// reading is above it. A member that can't be read while the group's level is above the
    /// limit (lowered since) may be above it too: it stays muted, and that is the refusal. After
    /// the unmute, whether or not the device said yes (a native group may have unmuted a player
    /// before another refused), it is brought down again when its reading, else (`sole`) `last`
    /// asked for now, is above the limit.
    async fn unmute_held(&self, level: Volume, max: f32, last: LastLevel<'_>, sole: bool) -> Unmuted {
        let back = |l| crate::zone::back_to_limit(l, max, |db| self.percent_for_db(db));
        if !self.unmute_restores_level() {
            if let Err(e) = self.set_volume(level).await {
                return Unmuted::nothing(e);
            }
            return match self.set_volume(Volume::Mute(false)).await {
                Ok(()) => Unmuted {
                    any: true,
                    error: None,
                    held: true,
                },
                // The level write already brought it back, at `level`
                Err(e) if self.level_write_unmutes() => Unmuted {
                    any: true,
                    error: Some(e),
                    held: true,
                },
                Err(e) => Unmuted::nothing(e),
            };
        }
        match self.volume().await {
            Ok(Some(v)) => {
                if let Some(b) = back(v.level) {
                    if let Err(e) = self.set_volume(b).await {
                        return Unmuted::nothing(e);
                    }
                }
            }
            _ if !sole && last.and_then(|f| f()).and_then(back).is_some() => {
                return Unmuted::nothing(SinkError::Device(format!(
                    "{}: its level can't be read, so it stays muted under the volume limit",
                    self.name()
                )));
            }
            _ => {}
        }
        let unmuted = self.set_volume(Volume::Mute(false)).await;
        let after = self.volume().await.ok().flatten();
        let any = match (&unmuted, after) {
            (Ok(()), _) => true,
            (Err(_), Some(v)) => !v.muted,
            (Err(_), None) => false,
        };
        let now = after
            .map(|v| v.level)
            .or_else(|| last.filter(|_| sole).and_then(|f| f()));
        let fix = match now.and_then(back) {
            Some(b) => self.set_volume(b).await.err(),
            None => None,
        };
        Unmuted {
            any,
            held: fix.is_none(),
            error: unmuted.err().or(fix),
        }
    }

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

    /// What the output takes now for `src` in particular: [`Output::live_caps`], narrowed to the
    /// closest exact format when the device lists them (see [`caps_for_source`]).
    fn live_caps_for(&self, _src: AudioFormat) -> OutputCaps {
        self.live_caps()
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

    /// The highest percent level this output plays at or below `db` (a zone's volume limit caps
    /// percent requests with it, so it rounds down). The default: linear over the output's own dB
    /// range when it reports a plausible one (ALSA's mixer, an endpoint's output; a mixer whose
    /// lowest step is mute reports about -99999 dB, which says nothing), else over -60…0 dB (UPnP,
    /// Snapcast). Outputs that map dB as an amplitude (BluOS, MPD, Cast) use
    /// [`amplitude_percent`]; Squeezebox (a squared gain) and AirPlay (-30…0 dB) their own; a
    /// group its strictest member's.
    fn percent_for_db(&self, db: f32) -> u8 {
        match self.caps().hw_volume {
            Some(pm_model::VolumeKind::Db { min, max, .. }) if max > min && min > -200.0 => {
                ((db.clamp(min, max) - min) / (max - min) * 100.0).floor() as u8
            }
            _ => ((db.clamp(-60.0, 0.0) + 60.0) / 60.0 * 100.0).floor() as u8,
        }
    }

    /// The level the output reports now, with its mute; `None` when it can't be read (the
    /// pages say "level unknown" rather than guess).
    async fn volume(&self) -> Result<Option<pm_model::ZoneVolume>, SinkError> {
        Ok(None)
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

    /// A source the device lists plays as it is; another plays as the closest listed format.
    #[test]
    fn each_source_is_planned_with_the_closest_listed_format() {
        let f = |rate_hz, bits| DeviceFormat {
            rate_hz,
            bits,
            channels: 2,
        };
        let caps = OutputCaps::unlimited();
        let pick = |formats: &[DeviceFormat], src: AudioFormat| {
            let c = caps_for_source(formats, src, &caps);
            (c.pcm_rates[0], c.pcm_bits[0])
        };
        // KA17-like: 16 bit at 44.1/48 only; 24 (in 32) and 32 up to 768 kHz
        let mut ka17 = vec![f(44_100, 16), f(48_000, 16)];
        for r in [
            44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000, 705_600, 768_000,
        ] {
            ka17.extend([f(r, 24), f(r, 32)]);
        }
        assert_eq!(
            pick(&ka17, AudioFormat::pcm(96_000, 24, 2)),
            (96_000, 24),
            "96/24 unchanged"
        );
        assert_eq!(
            pick(&ka17, AudioFormat::pcm(192_000, 24, 2)),
            (192_000, 24),
            "192/24 unchanged"
        );
        assert_eq!(pick(&ka17, AudioFormat::pcm(44_100, 16, 2)), (44_100, 16));
        assert_eq!(
            pick(&ka17, AudioFormat::pcm(96_000, 16, 2)),
            (96_000, 24),
            "padded, not resampled"
        );
        // 16 bit everywhere, 24 only at 96/192: 96/24 and 192/24 still unchanged
        let mut mixed: Vec<DeviceFormat> = [44_100, 48_000, 96_000, 192_000]
            .iter()
            .map(|r| f(*r, 16))
            .collect();
        mixed.extend([f(96_000, 24), f(192_000, 24)]);
        assert_eq!(pick(&mixed, AudioFormat::pcm(96_000, 24, 2)), (96_000, 24));
        assert_eq!(pick(&mixed, AudioFormat::pcm(192_000, 24, 2)), (192_000, 24));
        assert_eq!(
            pick(&mixed, AudioFormat::pcm(44_100, 24, 2)),
            (44_100, 16),
            "same rate before more bits"
        );
        // A rate it doesn't list: the nearest of its family, no fewer bits
        let k192: Vec<DeviceFormat> = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000]
            .iter()
            .map(|r| f(*r, 24))
            .collect();
        assert_eq!(pick(&k192, AudioFormat::pcm(352_800, 24, 2)), (176_400, 24));
        assert_eq!(pick(&k192, AudioFormat::pcm(384_000, 24, 2)), (192_000, 24));
        // Nothing for its channels, or nothing listed: the caps as they are
        assert_eq!(
            caps_for_source(&k192, AudioFormat::pcm(96_000, 24, 1), &caps),
            caps
        );
        assert_eq!(caps_for_source(&[], AudioFormat::pcm(96_000, 24, 2), &caps), caps);
    }

    /// The listener's modes on the wire, Exclusive USB among them; one from a newer app is Unknown
    /// (taken as auto).
    #[test]
    fn the_preferred_modes_on_the_wire() {
        for (text, mode) in [
            ("auto", PreferredMode::Auto),
            ("bit_perfect", PreferredMode::BitPerfect),
            ("exclusive", PreferredMode::Exclusive),
            ("shared", PreferredMode::Shared),
            ("hyper_perfect", PreferredMode::Unknown),
        ] {
            let s: DeviceSettings = serde_json::from_value(serde_json::json!({ "mode": text })).unwrap();
            assert_eq!(s.mode, mode, "{text}");
            if mode != PreferredMode::Unknown {
                assert_eq!(serde_json::to_value(mode).unwrap(), text);
            }
        }
    }
}
