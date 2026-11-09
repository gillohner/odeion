//! A zone: one output (groups come with clock sync, §6.5), a queue and a
//! transport, driven by a dedicated audio thread (the private `engine` module).
//!
//! [`Zone`] is the handle the API and the CLI hold: it mutates the queue
//! directly, sends transport commands to the thread, and exposes the
//! latest [`ZoneState`] through a `watch` channel and [`ZoneEvent`]s through
//! a `broadcast` channel.
//!
//! Gapless rule: when a track ends and the next one negotiates the same
//! output format, the sink stays open and the next track's first block
//! follows the last one with nothing in between. A format change drains
//! the sink (the tail plays out) and reopens it.
//!
//! Pause and seek: the [`AudioSink`](crate::AudioSink) trait has no pause,
//! so pausing flushes the sink's buffer and remembers the audible position;
//! resuming seeks the source there. Nothing is resampled or faded.

use crate::engine::{self, Command};
use crate::queue::{Insert, Queue, QueueItem, QueueSnapshot, Repeat};
use crate::sink::{Output, OutputId, SinkError};
use crate::source::SourceFactory;
use pm_model::{AudioFormat, SignalPath, Volume};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use tokio::sync::{broadcast, watch};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ZoneId(pub String);

impl ZoneId {
    pub fn new(s: impl Into<String>) -> Self {
        ZoneId(s.into())
    }
}

impl fmt::Display for ZoneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    #[default]
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NowPlaying {
    pub item: QueueItem,
    pub index: usize,
    /// Audible position: frames handed to the sink minus what it still buffers.
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub source_format: AudioFormat,
    pub output_format: AudioFormat,
    pub signal_path: SignalPath,
    /// Grouped outputs and their drift; empty for a single output.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<crate::sink::MemberStatus>,
    /// The latest in-stream title (internet radio), when the stream sends one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_title: Option<String>,
    /// What is arriving: codec, bitrate, service and tier (the player's quality label).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<crate::source::StreamInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoneState {
    pub id: ZoneId,
    pub name: String,
    pub output: OutputId,
    pub transport: Transport,
    pub now_playing: Option<NowPlaying>,
    pub queue: Vec<QueueItem>,
    pub queue_index: Option<usize>,
    pub queue_version: u64,
    pub repeat: Repeat,
    /// Shuffle mode: the upcoming items are shuffled; off puts them back in their order.
    #[serde(default)]
    pub shuffle: bool,
    /// Keep playing similar music after the last item (implemented by the node).
    #[serde(default)]
    pub radio: bool,
    /// Last error that stopped playback; cleared on the next successful start.
    pub error: Option<String>,
    /// Sequence number of the last command the engine handled (see [`Zone::wait_stopped`]).
    pub acked: u64,
    /// How closely the zone's speakers stay together.
    #[serde(default)]
    pub sync: crate::SyncClass,
    /// Crossfade between tracks in ms; 0 = off.
    #[serde(default)]
    pub crossfade_ms: u64,
    /// The output's level as last read back (after a change, and on the zone's poll); `None`
    /// until a read succeeds, or when the output can't say.
    #[serde(default)]
    pub volume: Option<pm_model::ZoneVolume>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ZoneEvent {
    TrackStarted {
        item: QueueItem,
        index: usize,
        source_format: AudioFormat,
        output_format: AudioFormat,
        signal_path: SignalPath,
        /// The sink stayed open from the previous track (gapless join).
        gapless: bool,
    },
    TrackEnded {
        item: QueueItem,
        /// Codec verification (FLAC MD5) when the codec provides one.
        verify_ok: Option<bool>,
    },
    /// An item could not be opened or planned; the zone moved on.
    Skipped {
        item: QueueItem,
        reason: String,
    },
    Paused {
        position_ms: u64,
    },
    Resumed,
    Seeked {
        position_ms: u64,
    },
    Stopped,
    QueueChanged,
    /// The stream announced a new title (internet radio).
    StreamTitle {
        title: String,
    },
    /// Playback stopped because of this (or, for a seek, the stream carried on).
    Error {
        message: String,
    },
}

/// Handle to a running zone. Cloneable through `Arc`; dropping the last
/// handle stops the audio thread.
pub struct Zone {
    id: ZoneId,
    name: String,
    output: Arc<dyn Output>,
    queue: Arc<Mutex<Queue>>,
    cmds: mpsc::Sender<(u64, Command)>,
    seq: AtomicU64,
    state: watch::Receiver<ZoneState>,
    events: broadcast::Sender<ZoneEvent>,
    crossfade: Arc<AtomicU64>,
    /// A set and its read-back, or a poll's read, one at a time (the device's I/O): a poll that
    /// started before a set can't publish after it.
    volume_io: Arc<tokio::sync::Mutex<()>>,
    /// The level last published, changed only together with the command that publishes it (the
    /// engine's state can lag the commands still queued). Never held across an await.
    volume_published: Arc<std::sync::Mutex<Published>>,
    /// The zone's volume limit, when the node set one ([`Zones::set_volume_limit`]).
    volume_limit: Arc<std::sync::RwLock<Option<VolumeLimit>>>,
}

/// What a zone last said its level was, and how many levels the device announced itself.
#[derive(Default)]
struct Published {
    level: Option<pm_model::ZoneVolume>,
    announced: u64,
}

/// A zone's volume limit in dB for an output (the node's DSP settings, `max_volume_db`), given
/// by the node ([`Zones::set_volume_limit`]): every volume request for the zone, from whichever
/// controller, is held to it in [`Zone::set_volume`].
pub type VolumeLimit = Arc<dyn Fn(&OutputId) -> Option<f32> + Send + Sync>;

/// Apply a volume limit (dB) to an absolute request: a dB request is capped at it, a percent
/// request at the level the output itself sets for that dB (`percent_for`: linear on UPnP and
/// Snapcast, an amplitude on BluOS, MPD and Cast). Up, down and mute pass as they are (a step
/// that lands above the limit is brought back after the read-back).
pub fn cap_volume(v: Volume, limit: Option<f32>, percent_for: impl Fn(f32) -> u8) -> Volume {
    match (v, limit) {
        (Volume::Db(db), Some(max)) if db > max => Volume::Db(max),
        (Volume::Percent(p), Some(max)) => Volume::Percent(p.min(percent_for(max))),
        (v, _) => v,
    }
}

/// One step up in percent, for an output that can't report its level (what the UPnP output
/// steps by).
const PERCENT_STEP: u8 = 3;

/// The request that brings `level` back to `limit` when it is above it.
pub(crate) fn back_to_limit(
    level: pm_model::VolumeLevel,
    limit: f32,
    percent_for: impl Fn(f32) -> u8,
) -> Option<Volume> {
    match level {
        pm_model::VolumeLevel::Db(d) if d > limit => Some(Volume::Db(limit)),
        pm_model::VolumeLevel::Percent(p) if p > percent_for(limit) => {
            Some(Volume::Percent(percent_for(limit)))
        }
        _ => None,
    }
}

impl Zone {
    /// Create a zone on `output` and start its audio thread.
    pub fn new(id: ZoneId, name: &str, output: Arc<dyn Output>, factory: Arc<dyn SourceFactory>) -> Self {
        let queue = Arc::new(Mutex::new(Queue::default()));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (events, _) = broadcast::channel(256);
        let initial = ZoneState {
            id: id.clone(),
            name: name.to_string(),
            output: output.id(),
            transport: Transport::Stopped,
            now_playing: None,
            queue: Vec::new(),
            queue_index: None,
            queue_version: 0,
            repeat: Repeat::Off,
            shuffle: false,
            radio: false,
            error: None,
            acked: 0,
            sync: output.sync(),
            crossfade_ms: 0,
            volume: None,
        };
        let (state_tx, state) = watch::channel(initial);
        let crossfade = Arc::new(AtomicU64::new(0));
        engine::spawn(
            id.clone(),
            output.clone(),
            factory,
            queue.clone(),
            cmd_rx,
            state_tx,
            events.clone(),
            crossfade.clone(),
        );
        Self {
            id,
            name: name.to_string(),
            output,
            queue,
            cmds: cmd_tx,
            seq: AtomicU64::new(0),
            state,
            events,
            crossfade,
            volume_io: Default::default(),
            volume_published: Default::default(),
            volume_limit: Default::default(),
        }
    }

    /// Crossfade between tracks, in ms (0 = off, the default). Songs of the same album
    /// following each other stay gapless; takes effect at the next track end.
    pub fn set_crossfade_ms(&self, ms: u64) {
        self.crossfade.store(ms, Ordering::Relaxed);
        self.send(Command::QueueChanged);
    }

    pub fn crossfade_ms(&self) -> u64 {
        self.crossfade.load(Ordering::Relaxed)
    }

    pub fn id(&self) -> &ZoneId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn output(&self) -> &Arc<dyn Output> {
        &self.output
    }

    /// The latest published state.
    pub fn state(&self) -> ZoneState {
        self.state.borrow().clone()
    }

    /// A receiver that wakes on every state change.
    pub fn watch(&self) -> watch::Receiver<ZoneState> {
        self.state.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ZoneEvent> {
        self.events.subscribe()
    }

    fn send(&self, cmd: Command) -> u64 {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        // A dead engine (panicked audio thread) shows up as a stale state; nothing to do here.
        let _ = self.cmds.send((seq, cmd));
        seq
    }

    fn with_queue<T>(&self, f: impl FnOnce(&mut Queue) -> T) -> T {
        let r = f(&mut self.queue.lock().expect("queue poisoned"));
        self.send(Command::QueueChanged);
        r
    }

    // ---- queue ----

    pub fn enqueue(&self, items: Vec<QueueItem>, at: Insert) -> Vec<u64> {
        self.with_queue(|q| q.add(items, at))
    }

    pub fn remove(&self, id: u64) -> bool {
        self.with_queue(|q| q.remove(id))
    }

    pub fn move_item(&self, id: u64, to_index: usize) -> bool {
        self.with_queue(|q| q.move_item(id, to_index))
    }

    pub fn clear(&self) {
        self.with_queue(|q| q.clear())
    }

    /// Shuffle the upcoming items, the current one keeps playing: shuffle mode goes on, or when
    /// it is on, the upcoming items are shuffled again (`POST …/shuffle`, Subsonic's `shuffle`).
    pub fn shuffle_upcoming(&self) -> bool {
        let seed = seed();
        self.with_queue(|q| {
            if q.shuffle() {
                q.shuffle_upcoming(seed)
            } else {
                q.set_shuffle(true, seed)
            }
        })
    }

    /// Shuffle mode on or off ([`Queue::set_shuffle`]); whether it changed.
    pub fn set_shuffle(&self, on: bool) -> bool {
        let seed = seed();
        self.with_queue(|q| q.set_shuffle(on, seed))
    }

    /// The current item first, every other item shuffled after it ([`Queue::reshuffle`]).
    pub fn reshuffle(&self) {
        let seed = seed();
        self.with_queue(|q| q.reshuffle(seed))
    }

    pub fn shuffle(&self) -> bool {
        self.queue.lock().expect("queue poisoned").shuffle()
    }

    /// Shuffle mode on for a queue just loaded: `index`'s item becomes the first and every other
    /// item is shuffled after it; with no index everything is shuffled and nothing is current.
    pub fn shuffle_from(&self, index: Option<usize>) {
        let seed = seed();
        self.with_queue(|q| {
            q.set_current(index);
            q.reshuffle(seed);
        })
    }

    /// The original order while shuffled, for saving ([`Queue::shuffle_order`]).
    pub fn shuffle_order(&self) -> Option<Vec<usize>> {
        self.queue.lock().expect("queue poisoned").shuffle_order()
    }

    /// What is saved per zone, from one look at the queue ([`Queue::snapshot`]).
    pub fn queue_snapshot(&self) -> QueueSnapshot {
        self.queue.lock().expect("queue poisoned").snapshot()
    }

    /// Shuffle on again from a saved queue ([`Queue::restore_shuffle`]).
    pub fn restore_shuffle(&self, order: &[usize]) {
        self.with_queue(|q| q.restore_shuffle(order))
    }

    pub fn set_repeat(&self, r: Repeat) {
        self.with_queue(|q| q.set_repeat(r))
    }

    pub fn set_radio(&self, on: bool) {
        self.with_queue(|q| q.set_radio(on))
    }

    pub fn radio(&self) -> bool {
        self.queue.lock().expect("queue poisoned").radio()
    }

    pub fn queue(&self) -> Vec<QueueItem> {
        self.queue.lock().expect("queue poisoned").items().to_vec()
    }

    // ---- transport ----

    /// Play: resume when paused, else start the current item (or the first).
    pub fn play(&self) {
        self.send(Command::Play(None));
    }

    /// Start the queue item at `index` from its beginning.
    pub fn play_index(&self, index: usize) {
        self.send(Command::Play(Some(index)));
    }

    pub fn pause(&self) {
        self.send(Command::Pause);
    }

    pub fn toggle_pause(&self) {
        self.send(Command::TogglePause);
    }

    pub fn stop(&self) {
        self.send(Command::Stop);
    }

    pub fn next(&self) {
        self.send(Command::Next);
    }

    /// Previous track, or restart the current one when more than 3 s in.
    pub fn previous(&self) {
        self.send(Command::Previous);
    }

    pub fn seek_ms(&self, ms: u64) {
        self.send(Command::Seek(ms));
    }

    fn published(&self) -> std::sync::MutexGuard<'_, Published> {
        self.volume_published.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Hardware volume on the output; the audio path is untouched. The level is read back into
    /// the state afterwards, so every controller shows what the device took. An output that
    /// can't read its level keeps the last absolute level set (and the mute), so a change still
    /// shows; up and down leave it as it was. A read that fails keeps the last level published
    /// (the poll catches up). Controllers always hear the result, also when it equals what they
    /// had: one that showed the level it asked for learns that the device kept another. A level
    /// the device announced while this ran is newer than this read-back: it stays.
    ///
    /// Every request is held to the zone's volume limit when the node set one
    /// ([`Zones::set_volume_limit`]): an absolute level above it becomes the limit before it
    /// reaches the output ([`cap_volume`]), and a step up that the device took past it is set
    /// back to the limit at once. On an output that can't report its level, a step up becomes
    /// an absolute request instead (the level last published plus one step, at most the
    /// limit), and is refused while no level is known. Unmuting is held to the limit too: a
    /// device that reports its level is set back when it came up above the limit; one that
    /// can't and chooses the level itself ([`Output::unmute_restores_level`] false: AirPlay,
    /// Squeezebox) is given the level last published, held to the limit, and the limit itself
    /// before the unmute while none is known; one that keeps its own level through a mute is
    /// only brought down when the level last published is above the limit, never raised. A
    /// group whose member refuses the unmute is capped all the same, and the refusal reported.
    /// This is the one place every controller's request passes (the pages, announcements, UPnP
    /// controllers, Subsonic clients).
    pub async fn set_volume(&self, v: Volume) -> Result<(), SinkError> {
        let _io = self.volume_io.lock().await;
        let limit = self.volume_limit();
        let mut v = cap_volume(v, limit, |db| self.output.percent_for_db(db));
        if let (Volume::Up, Some(max)) = (v, limit) {
            // A device that can't say where it is can't be checked after the step: the step is
            // made from the level the zone last published, held to the limit
            if !matches!(self.output.volume().await, Ok(Some(_))) {
                let Some(was) = self.published().level else {
                    return Err(SinkError::Device(
                        "the level isn't known, so a step can't be held to the zone's limit; set a level first".into(),
                    ));
                };
                v = match was.level {
                    pm_model::VolumeLevel::Db(d) => {
                        let step = match self.output.caps().hw_volume {
                            Some(pm_model::VolumeKind::Db { step, .. }) => step.max(0.5),
                            _ => 1.0,
                        };
                        Volume::Db((d + step).min(max))
                    }
                    pm_model::VolumeLevel::Percent(p) => Volume::Percent(
                        p.saturating_add(PERCENT_STEP)
                            .min(self.output.percent_for_db(max)),
                    ),
                };
            }
        }
        let seen = self.published().announced;
        if let (Volume::Mute(false), Some(max)) = (v, limit) {
            return self.unmute_within(max, seen).await;
        }
        let percent_for = |db: f32| self.output.percent_for_db(db);
        let (mut next, _) = self.set_and_read(v).await?;
        if let (Volume::Up, Some(max)) = (v, limit) {
            // A step the device took past the limit
            if let Some(back) = next.and_then(|n| back_to_limit(n.level, max, percent_for)) {
                next = self.set_and_read(back).await?.0;
            }
        }
        self.publish_volume(next, seen);
        Ok(())
    }

    /// Unmute held to the zone's limit `max`. A device that chooses its level on unmuting
    /// (AirPlay, Squeezebox) is given the level last published, held to the limit (the limit
    /// itself while none is known) before it unmutes, so it never comes back at full; one that
    /// restores its own level is only unmuted, never raised. A group does this per member
    /// ([`Output::unmute_held`]). After: a device that reports its level is set back when it
    /// came up above the limit; otherwise a level last published above the limit (lowered
    /// since) is brought down to it. The zone shows itself unmuted only when some device came
    /// back; a refusal is returned after the rest was done.
    async fn unmute_within(&self, max: f32, seen: u64) -> Result<(), SinkError> {
        let percent_for = |db: f32| self.output.percent_for_db(db);
        let was = self.published().level;
        let held = match was.map(|l| l.level) {
            Some(pm_model::VolumeLevel::Db(d)) => Volume::Db(d.min(max)),
            Some(pm_model::VolumeLevel::Percent(p)) => Volume::Percent(p.min(percent_for(max))),
            None => match self.output.caps().hw_volume {
                Some(pm_model::VolumeKind::Db { .. }) => Volume::Db(max),
                _ => Volume::Percent(percent_for(max)),
            },
        };
        let chooses = !self.output.unmute_restores_level();
        let r = self.output.unmute_held(held).await;
        if !r.any {
            // Nothing came back: the zone stays as it was shown
            return Err(r
                .error
                .unwrap_or_else(|| SinkError::Device("the unmute went nowhere".into())));
        }
        let mut error = r.error;
        // Down to the limit, never up: per member in a group ([`Output::hold_down`])
        if let Err(e) = self.output.hold_down(max, was.map(|l| l.level)).await {
            error.get_or_insert(e);
        }
        let (mut next, reported) = self.read_after(Volume::Mute(false)).await;
        // A device that can't report: what the zone gave it, held to the limit (a device that
        // keeps its own level and was never given one stays unknown)
        if !reported && (chooses || was.is_some()) {
            next = Some(pm_model::ZoneVolume {
                level: match held {
                    Volume::Db(d) => pm_model::VolumeLevel::Db(d),
                    Volume::Percent(p) => pm_model::VolumeLevel::Percent(p),
                    _ => unreachable!("held is a level"),
                },
                muted: false,
            });
        }
        // Some device came back: the zone shows it, and a member's refusal is said after
        self.publish_volume(next, seen);
        error.map_or(Ok(()), Err)
    }

    /// Publish `next` as the zone's level (none: the last one stays), unless the device announced
    /// a level of its own since `seen` (it is newer).
    fn publish_volume(&self, next: Option<pm_model::ZoneVolume>, seen: u64) {
        let mut p = self.published();
        if p.announced != seen {
            return;
        }
        if next.is_some() {
            p.level = next;
        }
        self.send(Command::Volume(p.level));
    }

    /// The zone's volume limit now: the node's for this zone's output, if any.
    fn volume_limit(&self) -> Option<f32> {
        let limit = self
            .volume_limit
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        limit.and_then(|f| f(&self.output.id()))
    }

    /// Set `v` on the output and read the level back ([`Self::read_after`]).
    async fn set_and_read(&self, v: Volume) -> Result<(Option<pm_model::ZoneVolume>, bool), SinkError> {
        self.output.set_volume(v).await?;
        Ok(self.read_after(v).await)
    }

    /// The level after `v` was set: the device's own reading (and `true`), else the level
    /// implied by `v` over the last one published (`None`: nothing to publish).
    async fn read_after(&self, v: Volume) -> (Option<pm_model::ZoneVolume>, bool) {
        let read = self.output.volume().await;
        match read {
            Ok(Some(read)) => (Some(read), true),
            Err(_) => (None, false),
            Ok(None) => {
                let was = self.published().level;
                let level = match v {
                    Volume::Db(d) => Some(pm_model::VolumeLevel::Db(d)),
                    Volume::Percent(p) => Some(pm_model::VolumeLevel::Percent(p.min(100))),
                    _ => was.map(|w| w.level),
                };
                let muted = match v {
                    Volume::Mute(m) => m,
                    _ => was.is_some_and(|w| w.muted),
                };
                (level.map(|level| pm_model::ZoneVolume { level, muted }), false)
            }
        }
    }

    /// Read the output's level and publish it in the state (the poll). A read that fails, or an
    /// output that can't say, leaves the last value; so does a level the device announced while
    /// the read ran (it is newer).
    pub async fn refresh_volume(&self) -> Option<pm_model::ZoneVolume> {
        let _io = self.volume_io.lock().await;
        let seen = self.published().announced;
        let read = self.output.volume().await;
        let mut p = self.published();
        match read {
            Ok(Some(v)) if p.announced == seen => {
                // Nothing is sent when it is the level last published
                if p.level != Some(v) {
                    p.level = Some(v);
                    self.send(Command::Volume(Some(v)));
                }
                Some(v)
            }
            Ok(_) => p.level,
            Err(_) => None,
        }
    }

    /// A level the device announced itself (UPnP events), through the engine. Always sent: the
    /// state may not show a level announced just before yet, and events must land in order. It
    /// becomes the level last published, so a poll that reads it again sends nothing and one
    /// that reads another level later sends that; a set or a poll in flight leaves it standing.
    pub fn announce_volume(&self, v: pm_model::ZoneVolume) {
        let mut p = self.published();
        p.level = Some(v);
        p.announced += 1;
        self.send(Command::Volume(Some(v)));
    }

    /// Wait until every command sent so far was handled, whatever the
    /// transport ended up as; returns the state published after that.
    pub async fn wait_acked(&self) -> ZoneState {
        let want = self.seq.load(Ordering::SeqCst);
        let mut rx = self.state.clone();
        loop {
            {
                let s = rx.borrow_and_update();
                if s.acked >= want {
                    return s.clone();
                }
            }
            if rx.changed().await.is_err() {
                return self.state();
            }
        }
    }

    /// Wait until every command sent so far was handled and the zone is
    /// stopped (end of queue, explicit stop, or error).
    pub async fn wait_stopped(&self) -> ZoneState {
        let want = self.seq.load(Ordering::SeqCst);
        let mut rx = self.state.clone();
        loop {
            {
                let s = rx.borrow_and_update();
                if s.acked >= want && s.transport == Transport::Stopped {
                    return s.clone();
                }
            }
            if rx.changed().await.is_err() {
                return self.state();
            }
        }
    }
}

impl Drop for Zone {
    fn drop(&mut self) {
        let _ = self.cmds.send((u64::MAX, Command::Shutdown));
    }
}

impl fmt::Debug for Zone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Zone")
            .field("id", &self.id)
            .field("output", &self.output.id())
            .finish()
    }
}

/// All zones of a node.
pub struct Zones {
    factory: Arc<dyn SourceFactory>,
    inner: RwLock<BTreeMap<ZoneId, Arc<Zone>>>,
    /// The node's own addresses, for every zone's queue (`Queue::set_node_origins`).
    node_origins: RwLock<Vec<String>>,
    /// The node's own sign-ins, for every zone's queue (`Queue::set_token_check`).
    token_check: RwLock<Option<crate::queue::TokenCheck>>,
    /// The zones' volume limits, by output (`Zone::set_volume`).
    volume_limit: RwLock<Option<VolumeLimit>>,
}

impl Zones {
    pub fn new(factory: Arc<dyn SourceFactory>) -> Self {
        Self {
            factory,
            inner: RwLock::new(BTreeMap::new()),
            node_origins: RwLock::new(Vec::new()),
            token_check: RwLock::new(None),
            volume_limit: RwLock::new(None),
        }
    }

    /// How a zone learns its volume limit (dB) from its output: every volume request for a
    /// zone, from whichever controller, is held to it. For zones made from now on and those
    /// there are; read at every request, so a changed setting counts at once.
    pub fn set_volume_limit(&self, limit: Option<VolumeLimit>) {
        for z in self.inner.read().expect("zones poisoned").values() {
            *z.volume_limit.write().unwrap_or_else(|e| e.into_inner()) = limit.clone();
        }
        *self.volume_limit.write().expect("zones poisoned") = limit;
    }

    /// How a queue tells the node's own sign-ins (the API token, an owner's session, a listener's
    /// token): a `/v1/` link on any other host loses those on the way into any zone's queue. For
    /// zones made from now on and those there are.
    pub fn set_token_check(&self, check: Option<crate::queue::TokenCheck>) {
        for z in self.inner.read().expect("zones poisoned").values() {
            z.queue
                .lock()
                .expect("queue poisoned")
                .set_token_check(check.clone());
        }
        *self.token_check.write().expect("zones poisoned") = check;
    }

    /// The node's own addresses (`http://host:port`): links there are the node's, and lose their
    /// sign-in on the way into any zone's queue. For zones made from now on and those there are.
    pub fn set_node_origins(&self, origins: Vec<String>) {
        for z in self.inner.read().expect("zones poisoned").values() {
            z.queue
                .lock()
                .expect("queue poisoned")
                .set_node_origins(origins.clone());
        }
        *self.node_origins.write().expect("zones poisoned") = origins;
    }

    /// Create a zone named `name` on `output`. The id is derived from the
    /// name (`Living Room` → `living-room`), made unique with a suffix.
    pub fn create(&self, name: &str, output: Arc<dyn Output>) -> Arc<Zone> {
        let mut map = self.inner.write().expect("zones poisoned");
        let base = slug(name);
        let mut id = ZoneId(base.clone());
        let mut n = 2;
        while map.contains_key(&id) {
            id = ZoneId(format!("{base}-{n}"));
            n += 1;
        }
        let zone = Arc::new(Zone::new(id.clone(), name, output, self.factory.clone()));
        let origins = self.node_origins.read().expect("zones poisoned").clone();
        let check = self.token_check.read().expect("zones poisoned").clone();
        {
            let mut q = zone.queue.lock().expect("queue poisoned");
            q.set_node_origins(origins);
            q.set_token_check(check);
        }
        *zone.volume_limit.write().unwrap_or_else(|e| e.into_inner()) =
            self.volume_limit.read().expect("zones poisoned").clone();
        map.insert(id, zone.clone());
        zone
    }

    pub fn get(&self, id: &ZoneId) -> Option<Arc<Zone>> {
        self.inner.read().expect("zones poisoned").get(id).cloned()
    }

    pub fn for_output(&self, output: &OutputId) -> Option<Arc<Zone>> {
        self.inner
            .read()
            .expect("zones poisoned")
            .values()
            .find(|z| &z.output().id() == output)
            .cloned()
    }

    pub fn remove(&self, id: &ZoneId) -> Option<Arc<Zone>> {
        self.inner.write().expect("zones poisoned").remove(id)
    }

    pub fn list(&self) -> Vec<ZoneState> {
        self.inner
            .read()
            .expect("zones poisoned")
            .values()
            .map(|z| z.state())
            .collect()
    }
}

fn slug(name: &str) -> String {
    let mut s = String::new();
    let mut dash = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash {
            s.push('-');
            dash = true;
        }
    }
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() {
        "zone".into()
    } else {
        s
    }
}

/// A seed for a shuffle: the clock, never the same twice in a row.
fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x5eed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("Living Room"), "living-room");
        assert_eq!(slug("  Küche / DAC #2 "), "k-che-dac-2");
        assert_eq!(slug("***"), "zone");
    }
}
