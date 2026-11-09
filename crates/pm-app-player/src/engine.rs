//! The page's player, played here: a `pm_zones::Zone` on this device's output, with the page's
//! URLs as its queue. The page keeps its own queue, repeat and shuffle; it hands this player what
//! plays now and what comes next ([`AppPlayer::load`], [`AppPlayer::ahead`]), and hears what the
//! `<audio>` element would say through [`Status`]: playing or paused, where, and `ended` counted
//! up at the end of each song. When the next song was handed ahead, the zone joins it without a
//! gap and the status already names it when `ended` goes up.
//!
//! The output is taken through the endpoint's [`OutputOwner`]: this player opening it stops the
//! node's session on it, and the node opening it stops this player ([`Status::error`] says so).

use crate::source::UrlSources;
use pm_endpoint::owner::{Claim, Holder, OutputOwner, Stop};
use pm_model::{AudioBlock, AudioFormat, OutputCaps, SignalPath, Volume};
use pm_zones::{
    AudioSink, BufferLevel, Insert, Output, OutputDevice, OutputId, QueueItem, SinkError, StreamInfo,
    Transport, Zone, ZoneEvent, ZoneId, ZoneState,
};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// Why this player stopped when the node took its output.
pub const TAKEN_BY_NODE: &str = "the node plays on this device's zone now";
/// How long an open waits for the node's session to let go of the output.
const TAKE_WAIT: Duration = Duration::from_secs(3);
/// Position-only changes reach the page at most this often.
const POSITION_EVERY: Duration = Duration::from_millis(500);

/// What the page's `<audio>`-like engine reads.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    /// What plays (or is paused) now: the URL the page loaded or handed ahead.
    pub url: Option<String>,
    pub transport: Transport,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    /// Songs that came to their end so far (the page's `ended`).
    pub ended: u64,
    /// Of those, the ones the zone went on from to the song handed ahead (it already plays it,
    /// `url` names it): the page's next `src` is that song, not a new one to open.
    pub advanced: u64,
    /// The last thing that went wrong, until the next start.
    pub error: Option<String>,
    /// The URL could not be played here (opened or decoded): the page plays it itself.
    pub refused: Option<String>,
    /// The node took the output for its own zone: paused here, [`Status::error`] says so.
    pub taken: bool,
    /// The stream broke mid-song: paused where it broke, [`Status::error`] says why (the page's
    /// mid-song retry opens it there again).
    pub broke: bool,
    /// The format and path of what plays, as a zone reports them.
    pub now_playing: Option<Playing>,
    /// What plays is the copy downloaded to this device (no stream).
    pub from_device: bool,
    pub output: OutputId,
    /// The hardware behind the output (a USB DAC), as the node's output list has it.
    pub device: Option<OutputDevice>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Playing {
    pub source_format: AudioFormat,
    pub output_format: AudioFormat,
    pub signal_path: SignalPath,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<StreamInfo>,
}

/// Called with the status whenever it changes.
pub type Notify = Arc<dyn Fn(&Status) + Send + Sync>;

/// Why [`AppPlayer::load`] didn't start a song.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoadError {
    /// Something the app can't play itself (the page plays it in `<audio>`); otherwise it may
    /// pass (the page tries again, once, as with `<audio>`).
    pub refused: bool,
    /// Another load or a stop came after it: nothing to say to the listener.
    pub overtaken: bool,
    pub message: String,
}

impl LoadError {
    fn overtaken() -> Self {
        Self {
            refused: false,
            overtaken: true,
            message: "another song or a stop came first".into(),
        }
    }

    fn failed(message: impl Into<String>) -> Self {
        Self {
            refused: false,
            overtaken: false,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Where the page and the zone stand.
#[derive(Debug, Default, Clone)]
struct Page {
    url: Option<String>,
    /// The zone's queue id of `url`.
    current: Option<u64>,
    ahead: Option<(String, u64)>,
    /// What the page last said comes next (applied once no load is under way).
    want_ahead: Option<String>,
    ended: u64,
    advanced: u64,
    error: Option<String>,
    refused: Option<String>,
    /// The node took the output: paused here at this position (a play loads it again there).
    taken_at: Option<u64>,
    /// Raised by every load and stop: a load that finds it raised since it began was overtaken
    /// (another song, or a stop) and leaves the zone alone.
    generation: u64,
    /// A load is opening its URL (the zone still holds the previous song).
    loading: bool,
    /// Pause pressed while a load opened: the song starts paused.
    hold: bool,
    /// The current song's stream broke (a decode error): the zone's next TrackEnded is no end.
    breaking: bool,
    /// Where the current song was last heard to be (the zone may already name the next one when
    /// the page learns the stream broke).
    heard_at: u64,
    /// It broke here: paused at this position (a play opens it again there; the page's retry).
    broke_at: Option<u64>,
}

/// What the page should hear of a zone event, and whether the zone must stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Heard {
    tell: bool,
    stop: bool,
}

impl From<bool> for Heard {
    fn from(tell: bool) -> Self {
        Heard { tell, stop: false }
    }
}

impl Page {
    /// What the zone says plays now (its id and position): where the current song is.
    fn note(&mut self, playing: Option<(u64, u64)>) {
        if let Some((id, at)) = playing {
            if Some(id) == self.current {
                self.heard_at = at;
            }
        }
    }

    /// The zone said `ev`.
    fn on(&mut self, ev: &ZoneEvent) -> Heard {
        match ev {
            // The stream broke (the zone ends the song on a decode error, and would go on to the
            // song handed ahead): not its end. The TrackEnded that follows the error is swallowed
            ZoneEvent::TrackEnded { item, .. } if Some(item.id) == self.current && self.breaking => {
                self.breaking = false;
                true.into()
            }
            ZoneEvent::TrackEnded { item, .. } if Some(item.id) == self.current => {
                self.ended += 1;
                // Handed ahead: the zone goes on with it, and the page hears both at once
                if let Some((url, id)) = self.ahead.take() {
                    self.url = Some(url);
                    self.current = Some(id);
                    self.advanced += 1;
                    self.heard_at = 0;
                    // (a load under way keeps what the page said comes after its song)
                    if !self.loading {
                        self.want_ahead = None;
                    }
                }
                true.into()
            }
            ZoneEvent::TrackStarted { item, .. } if Some(item.id) == self.current => {
                self.error = None;
                self.refused = None;
                self.breaking = false;
                true.into()
            }
            // The song handed ahead didn't open: something the app can't play is the page's to
            // play; anything else is an error (the page's retry runs)
            ZoneEvent::Skipped { item, reason } if Some(item.id) == self.current => {
                if crate::source::is_refusal(reason) {
                    self.refused = Some(reason.clone());
                } else {
                    self.error = Some(reason.clone());
                }
                true.into()
            }
            ZoneEvent::Skipped { item, .. } if self.ahead.as_ref().is_some_and(|(_, id)| *id == item.id) => {
                // The next one can't play here: the page opens it when its turn comes
                self.ahead = None;
                false.into()
            }
            ZoneEvent::Error { message } => {
                if self.taken_at.is_some() {
                    return true.into();
                }
                self.error = Some(message.clone());
                // The stream broke (the zone says "decode: …" and then ends the song): paused
                // where it was last heard, at once, and the zone stops there
                if message.starts_with("decode:") && self.current.is_some() && self.broke_at.is_none() {
                    self.breaking = true;
                    self.broke_at = Some(self.heard_at);
                    return Heard {
                        tell: true,
                        stop: true,
                    };
                }
                true.into()
            }
            ZoneEvent::Stopped => true.into(),
            // Playing again: an old reason (a DAC that was unplugged) no longer applies
            ZoneEvent::Resumed => {
                if self.taken_at.is_none() && self.broke_at.is_none() {
                    self.error = None;
                }
                true.into()
            }
            ZoneEvent::Paused { .. } | ZoneEvent::Seeked { .. } => true.into(),
            _ => false.into(),
        }
    }

    fn status(&self, z: Option<&ZoneState>, output: &dyn Output) -> Status {
        let np = z.and_then(|z| z.now_playing.as_ref());
        let mine = np.filter(|np| Some(np.item.id) == self.current);
        let (transport, position_ms) = match (self.taken_at.or(self.broke_at), z, mine) {
            (Some(at), _, _) => (Transport::Paused, at),
            (None, Some(z), Some(np)) => (z.transport, np.position_ms),
            _ => (Transport::Stopped, 0),
        };
        Status {
            url: self.url.clone(),
            transport,
            position_ms,
            duration_ms: mine.and_then(|np| np.duration_ms),
            ended: self.ended,
            advanced: self.advanced,
            error: self.error.clone(),
            refused: self.refused.clone(),
            taken: self.taken_at.is_some(),
            broke: self.broke_at.is_some(),
            now_playing: mine.map(|np| Playing {
                source_format: np.source_format,
                output_format: np.output_format,
                signal_path: np.signal_path.clone(),
                format: np.format.clone(),
            }),
            from_device: false,
            output: output.id(),
            device: output.device(),
        }
    }
}

struct Inner {
    output: Arc<dyn Output>,
    owner: OutputOwner,
    sources: Arc<UrlSources>,
    zone: Mutex<Option<Arc<Zone>>>,
    page: Mutex<Page>,
    notify: Mutex<Option<Notify>>,
    last: Mutex<Option<(Status, Instant)>>,
}

/// The app's own player on one output. Cloning shares it.
#[derive(Clone)]
pub struct AppPlayer {
    inner: Arc<Inner>,
}

impl AppPlayer {
    /// A player on `output`, taken through `owner` (the endpoint's, so the node's zone on the
    /// same output and this player never play at once).
    pub fn new(output: Arc<dyn Output>, owner: OutputOwner) -> Self {
        Self {
            inner: Arc::new(Inner {
                output,
                owner,
                sources: Arc::new(UrlSources::new()),
                zone: Mutex::default(),
                page: Mutex::default(),
                notify: Mutex::default(),
                last: Mutex::default(),
            }),
        }
    }

    /// Where each change goes (the page).
    pub fn on_change(&self, f: Notify) {
        *self.inner.notify.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }

    /// The URL opener (files kept on this device go there).
    pub fn sources(&self) -> &UrlSources {
        &self.inner.sources
    }

    pub fn status(&self) -> Status {
        let z = self.zone_state();
        let mut st = self.page().status(z.as_ref(), &*self.inner.output);
        st.from_device = st.now_playing.is_some()
            && st
                .url
                .as_deref()
                .is_some_and(|u| self.inner.sources.from_device(u));
        st
    }

    fn page(&self) -> std::sync::MutexGuard<'_, Page> {
        self.inner.page.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn zone_state(&self) -> Option<ZoneState> {
        self.inner
            .zone
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|z| z.state())
    }

    /// The zone, made on first use with its watcher.
    fn zone(&self) -> Arc<Zone> {
        let mut slot = self.inner.zone.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(z) = slot.as_ref() {
            return z.clone();
        }
        let weak = Arc::downgrade(&self.inner);
        let owned: Arc<dyn Output> = Arc::new(Owned {
            inner: self.inner.output.clone(),
            owner: self.inner.owner.clone(),
            player: weak.clone(),
        });
        let zone = Arc::new(Zone::new(
            ZoneId::new("app"),
            &self.inner.output.name(),
            owned,
            self.inner.sources.clone(),
        ));
        *slot = Some(zone.clone());
        drop(slot);
        watch(weak, &zone);
        zone
    }

    /// Play `url` from `at_ms`, with `ahead` to follow it without a gap. Opens it first (network
    /// I/O: call from a blocking thread). `Err` leaves the zone as it was: [`LoadError::refused`]
    /// when it is something the app can't play itself (the page plays it), else why it didn't open
    /// (the node busy, the network gone; or another load or a stop overtook this one).
    pub fn load(&self, url: &str, at_ms: u64, ahead: Option<&str>) -> Result<(), LoadError> {
        let mine = {
            let mut p = self.page();
            p.generation += 1;
            p.loading = true;
            p.hold = false;
            p.want_ahead = ahead.map(str::to_string);
            p.generation
        };
        // (a decoder that panics on a strange file is a failure like another)
        let opened =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.inner.sources.open_url(url)))
                .unwrap_or_else(|_| Err("the app's decoder stopped on this stream".to_string()));
        let mut p = self.page();
        if p.generation != mine {
            return Err(LoadError::overtaken());
        }
        p.loading = false;
        let source = match opened {
            Ok(s) => s,
            Err(message) if crate::source::is_refusal(&message) => {
                tracing::info!("the app's player leaves this one to the page: {message}");
                return Err(LoadError {
                    refused: true,
                    overtaken: false,
                    message,
                });
            }
            Err(message) => {
                // The page shows this song now: what played before stops (the page tries again)
                tracing::info!("the app's player: the song didn't open: {message}");
                p.url = None;
                p.current = None;
                p.ahead = None;
                p.want_ahead = None;
                p.taken_at = None;
                p.broke_at = None;
                p.breaking = false;
                if let Some(z) = self.inner.zone.lock().unwrap_or_else(|e| e.into_inner()).clone() {
                    z.stop();
                    z.clear();
                }
                drop(p);
                self.changed(true);
                return Err(LoadError::failed(message));
            }
        };
        // The queue changes under the page's lock, so no other load or stop comes in between
        let zone = self.zone();
        self.inner.sources.ready(url, source);
        p.error = None;
        p.refused = None;
        p.taken_at = None;
        p.broke_at = None;
        p.breaking = false;
        p.heard_at = at_ms;
        zone.clear();
        let ahead = p.want_ahead.clone();
        let mut items = vec![QueueItem::new(url)];
        items.extend(ahead.as_deref().map(QueueItem::new));
        let ids = zone.enqueue(items, Insert::End);
        p.url = Some(url.to_string());
        p.current = ids.first().copied();
        p.ahead = ahead.zip(ids.get(1).copied());
        zone.play_index(0);
        if at_ms > 1_000 {
            zone.seek_ms(at_ms);
        }
        if std::mem::take(&mut p.hold) {
            zone.pause();
        }
        drop(p);
        self.changed(true);
        Ok(())
    }

    /// What comes after what plays now (`None`: nothing, the end of the page's queue). During a
    /// load it waits for it.
    pub fn ahead(&self, url: Option<&str>) {
        let mut p = self.page();
        p.want_ahead = url.map(str::to_string);
        if p.loading {
            return;
        }
        let Some(zone) = self.inner.zone.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
            return;
        };
        if p.ahead.as_ref().map(|(u, _)| u.as_str()) == url {
            return;
        }
        if let Some((_, id)) = p.ahead.take() {
            zone.remove(id);
        }
        if let (Some(url), Some(_)) = (url, p.current) {
            let ids = zone.enqueue(vec![QueueItem::new(url)], Insert::End);
            p.ahead = ids.first().map(|id| (url.to_string(), *id));
        }
    }

    /// Carry on: from the pause; after the node took the output, or once the zone stopped, from
    /// where it was (again through [`Self::load`], so call it from a blocking thread too).
    pub fn play(&self) -> Result<(), LoadError> {
        // Play after a pause pressed while a song opened: it starts playing after all
        self.page().hold = false;
        let (url, at, ahead, taken) = {
            let p = self.page();
            let at = p.taken_at.or(p.broke_at);
            (
                p.url.clone(),
                at,
                p.want_ahead
                    .clone()
                    .or_else(|| p.ahead.as_ref().map(|a| a.0.clone())),
                at.is_some(),
            )
        };
        let Some(url) = url else {
            return Err(LoadError::failed("nothing to play"));
        };
        let st = self.zone_state();
        let stopped = st.as_ref().is_none_or(|s| s.transport == Transport::Stopped);
        if taken || stopped {
            return self.load(&url, at.unwrap_or(0), ahead.as_deref());
        }
        self.zone().play();
        Ok(())
    }

    pub fn pause(&self) {
        {
            let mut p = self.page();
            if p.loading {
                p.hold = true;
            }
        }
        if let Some(z) = self.inner.zone.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            z.pause();
        }
    }

    pub fn seek(&self, ms: u64) {
        // Paused where the node took it or the stream broke: the play opens it here
        let parked = {
            let mut g = self.page();
            let p = &mut *g;
            match p.taken_at.as_mut().or(p.broke_at.as_mut()) {
                Some(at) => {
                    *at = ms;
                    true
                }
                None => false,
            }
        };
        if parked {
            self.changed(true);
            return;
        }
        if let Some(z) = self.inner.zone.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            z.seek_ms(ms);
        }
    }

    /// Stop and let the output go (the page plays something else, or on another zone). A load
    /// still opening its URL doesn't start afterwards.
    pub fn stop(&self) {
        {
            let mut p = self.page();
            p.generation += 1;
            p.loading = false;
            p.hold = false;
            p.want_ahead = None;
            p.ahead = None;
            p.broke_at = None;
            p.breaking = false;
            p.taken_at = None;
            p.url = None;
            p.current = None;
        }
        if let Some(z) = self.inner.zone.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            z.stop();
            z.clear();
        }
        self.changed(true);
    }

    /// The node took the output: paused here where it was.
    fn taken(&self) {
        let at = self.status().position_ms;
        {
            let mut p = self.page();
            if p.url.is_none() || p.taken_at.is_some() {
                return;
            }
            p.taken_at = Some(at);
            p.error = Some(TAKEN_BY_NODE.into());
        }
        if let Some(z) = self.inner.zone.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            z.stop();
        }
        self.changed(true);
    }

    /// Tell the page, now (`urgent`) or when the position alone moved and it is time.
    fn changed(&self, urgent: bool) {
        let st = self.status();
        let mut last = self.inner.last.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((was, at)) = last.as_ref() {
            if *was == st {
                return;
            }
            let moved_only = Status {
                position_ms: was.position_ms,
                ..st.clone()
            } == *was;
            if moved_only && !urgent && at.elapsed() < POSITION_EVERY {
                return;
            }
        }
        *last = Some((st.clone(), Instant::now()));
        drop(last);
        let f = self
            .inner
            .notify
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(f) = f {
            f(&st);
        }
    }
}

/// Follow the zone's events and state for the page, until the player is gone.
fn watch(player: Weak<Inner>, zone: &Zone) {
    let mut events = zone.subscribe();
    let mut state = zone.watch();
    let spawned = std::thread::Builder::new()
        .name("app-player".into())
        .spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
            else {
                return;
            };
            rt.block_on(async move {
                loop {
                    // The position moves only while it plays: no wake-ups otherwise
                    let ticking = state.borrow().transport == Transport::Playing;
                    let tick = async {
                        if ticking {
                            tokio::time::sleep(POSITION_EVERY).await
                        } else {
                            std::future::pending::<()>().await
                        }
                    };
                    // Events first: the zone sends them before it publishes the state that follows
                    // them (a break's error before the state that names the next song)
                    let urgent = tokio::select! {
                        biased;
                        ev = events.recv() => match ev {
                            Ok(ev) => {
                                let Some(inner) = player.upgrade() else { return };
                                let p = AppPlayer { inner };
                                let playing = state.borrow().now_playing.as_ref().map(|np| (np.item.id, np.position_ms));
                                let mut pg = p.page();
                                pg.note(playing);
                                let heard = pg.on(&ev);
                                // (under the page's lock: no load comes in between)
                                if heard.stop {
                                    let z = p.inner.zone.lock().unwrap_or_else(|e| e.into_inner()).clone();
                                    if let Some(z) = z {
                                        z.stop();
                                    }
                                }
                                drop(pg);
                                Some((p, heard.tell))
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => None,
                            Err(_) => return,
                        },
                        ch = state.changed() => {
                            if ch.is_err() {
                                return;
                            }
                            player.upgrade().map(|inner| {
                                let p = AppPlayer { inner };
                                let playing = state.borrow().now_playing.as_ref().map(|np| (np.item.id, np.position_ms));
                                p.page().note(playing);
                                (p, false)
                            })
                        }
                        // A position that moves with nothing else changing still reaches the page
                        _ = tick => player.upgrade().map(|inner| (AppPlayer { inner }, false)),
                    };
                    match urgent {
                        Some((p, urgent)) => p.changed(urgent),
                        None if player.strong_count() == 0 => return,
                        None => {}
                    }
                }
            });
        });
    if let Err(e) = spawned {
        tracing::warn!("the app's player can't follow its zone: {e}");
    }
}

/// The device's output as this player opens it: taken from the node first, and given up when
/// the node takes it back (the sink's next write says so and the zone stops).
struct Owned {
    inner: Arc<dyn Output>,
    owner: OutputOwner,
    player: Weak<Inner>,
}

#[async_trait::async_trait]
impl Output for Owned {
    fn id(&self) -> OutputId {
        self.inner.id()
    }
    fn name(&self) -> String {
        self.inner.name()
    }
    fn caps(&self) -> &OutputCaps {
        self.inner.caps()
    }
    fn model(&self) -> Option<String> {
        self.inner.model()
    }
    fn live_caps(&self) -> OutputCaps {
        self.inner.live_caps()
    }
    fn live_caps_for(&self, src: AudioFormat) -> OutputCaps {
        self.inner.live_caps_for(src)
    }
    fn device(&self) -> Option<OutputDevice> {
        self.inner.device()
    }
    async fn open(&self, fmt: AudioFormat) -> Result<Box<dyn AudioSink>, SinkError> {
        let lost = Arc::new(AtomicBool::new(false));
        let stop: Stop = {
            let (lost, player) = (lost.clone(), self.player.clone());
            Arc::new(move || {
                lost.store(true, Ordering::SeqCst);
                if let Some(inner) = player.upgrade() {
                    // Not on the caller's thread: the zone's stop waits for nothing, but the
                    // page's notify may
                    std::thread::spawn(move || AppPlayer { inner }.taken());
                }
            })
        };
        let claim = self
            .owner
            .take(&self.id(), Holder::App, stop, TAKE_WAIT)
            .map_err(|why| SinkError::Unavailable(self.id(), why))?;
        let sink = self.inner.open(fmt).await?;
        Ok(Box::new(OwnedSink {
            inner: sink,
            id: self.id(),
            lost,
            _claim: claim,
        }))
    }
    async fn set_volume(&self, v: Volume) -> Result<(), SinkError> {
        self.inner.set_volume(v).await
    }
}

struct OwnedSink {
    inner: Box<dyn AudioSink>,
    id: OutputId,
    lost: Arc<AtomicBool>,
    _claim: Claim,
}

impl OwnedSink {
    fn check(&self) -> Result<(), SinkError> {
        if self.lost.load(Ordering::SeqCst) {
            return Err(SinkError::Gone(self.id.clone(), TAKEN_BY_NODE.into()));
        }
        Ok(())
    }
}

impl AudioSink for OwnedSink {
    fn format(&self) -> AudioFormat {
        self.inner.format()
    }
    fn write(&mut self, block: &AudioBlock) -> Result<(), SinkError> {
        self.check()?;
        self.inner.write(block)
    }
    fn buffer_level(&self) -> BufferLevel {
        self.inner.buffer_level()
    }
    fn flush(&mut self) -> Result<(), SinkError> {
        self.inner.flush()
    }
    fn drain(&mut self) -> Result<(), SinkError> {
        self.inner.drain()
    }
    fn pause(&mut self) -> Result<bool, SinkError> {
        self.inner.pause()
    }
    fn resume(&mut self) -> Result<(), SinkError> {
        self.check()?;
        self.inner.resume()
    }
    fn ready_to_resume(&mut self) -> Result<(), SinkError> {
        self.check()?;
        self.inner.ready_to_resume()
    }
    fn frames_written(&self) -> u64 {
        self.inner.frames_written()
    }
    fn reopen_at_track(&self) -> bool {
        self.inner.reopen_at_track()
    }
    fn gone_reported(&mut self) {
        self.inner.gone_reported()
    }
    fn reopen_now(&self) -> bool {
        self.inner.reopen_now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::range::tests::{serve, Mode};
    use pm_model::Samples;
    use pm_zones::outputs::{read_wav, NullOutput, WavFileOutput};

    fn wav_bytes(rate: u32, bits: u16, frames: usize, seed: i64) -> (Vec<u8>, Vec<i32>) {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: rate,
            bits_per_sample: bits,
            sample_format: hound::SampleFormat::Int,
        };
        let mut cur = std::io::Cursor::new(Vec::new());
        let mut samples = Vec::new();
        {
            let mut w = hound::WavWriter::new(&mut cur, spec).unwrap();
            let fs = 1i64 << (bits - 1);
            for i in 0..frames as i64 * 2 {
                let s = ((i * 7919 + seed * 104_729) % (2 * fs) - fs) as i32;
                samples.push(s);
                if bits == 16 {
                    w.write_sample(s as i16).unwrap();
                } else {
                    w.write_sample(s).unwrap();
                }
            }
            w.finalize().unwrap();
        }
        (cur.into_inner(), samples)
    }

    fn wait(p: &AppPlayer, what: &str, ok: impl Fn(&Status) -> bool) -> Status {
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            let st = p.status();
            if ok(&st) {
                return st;
            }
            assert!(Instant::now() < until, "{what}: {st:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Every status the page was told, in order.
    fn recorder(p: &AppPlayer) -> Arc<Mutex<Vec<Status>>> {
        let seen: Arc<Mutex<Vec<Status>>> = Arc::default();
        let s2 = seen.clone();
        p.on_change(Arc::new(move |st| s2.lock().unwrap().push(st.clone())));
        seen
    }

    #[test]
    fn a_flac_and_a_wav_over_http_reach_the_output_sample_for_sample() {
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../pm-audio/tests/fixtures");
        let flac = fixtures.join("63_predictor_overflow_check_24-bit.flac");
        let want_flac = pm_audio::Decoder::open(&flac).unwrap().decode_all().unwrap();
        let (wav, want_wav) = wav_bytes(96_000, 24, 20_000, 3);
        for (body, ct, want) in [
            (
                std::fs::read(&flac).unwrap(),
                "audio/flac",
                want_flac.samples.clone(),
            ),
            (wav, "audio/wav", Samples::PcmI32(want_wav)),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let out = dir.path().join("out.wav");
            let p = AppPlayer::new(Arc::new(WavFileOutput::new(&out)), OutputOwner::new());
            let (url, _) = serve(body, ct, Mode::default());
            p.load(&url, 0, None).unwrap();
            let st = wait(&p, "the end", |s| {
                s.ended == 1 && s.transport == Transport::Stopped
            });
            assert_eq!(st.url.as_deref(), Some(url.as_str()));
            assert!(st.error.is_none(), "{st:?}");
            p.stop();
            assert_eq!(read_wav(&out).unwrap().samples, want, "{ct}");
        }
    }

    /// The next song handed ahead joins without a gap: one sink, both songs in it back to back,
    /// and the page hears the end of the first together with the second's URL.
    #[test]
    fn the_song_handed_ahead_follows_gaplessly() {
        let (a, mut want) = wav_bytes(44_100, 16, 30_000, 1);
        let (b, wb) = wav_bytes(44_100, 16, 20_000, 2);
        want.extend(wb);
        let (ua, _) = serve(a, "audio/wav", Mode::default());
        let (ub, _) = serve(b, "audio/wav", Mode::default());
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.wav");
        let p = AppPlayer::new(Arc::new(WavFileOutput::new(&out)), OutputOwner::new());
        let seen = recorder(&p);
        p.load(&ua, 0, Some(&ub)).unwrap();
        wait(&p, "both ended", |s| {
            s.ended == 2 && s.transport == Transport::Stopped
        });
        p.stop();
        assert_eq!(read_wav(&out).unwrap().samples, Samples::PcmI32(want));
        let first_end = seen
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.ended == 1)
            .cloned()
            .unwrap();
        assert_eq!(first_end.url.as_deref(), Some(ub.as_str()), "{first_end:?}");
    }

    /// A song kept on this device plays from its copy (no request reaches the node), and the
    /// status says so; one without a copy streams.
    #[test]
    fn a_downloaded_song_plays_from_the_device() {
        let dir = tempfile::tempdir().unwrap();
        let (wav, want) = wav_bytes(44_100, 16, 10_000, 4);
        std::fs::write(dir.path().join("3.wav"), &wav).unwrap();
        let (url, asked) = serve(wav.clone(), "audio/wav", Mode::default());
        let out_path = dir.path().join("out.wav");
        let p = AppPlayer::new(Arc::new(WavFileOutput::new(&out_path)), OutputOwner::new());
        let kept = dir.path().join("3.wav");
        let u2 = url.clone();
        p.sources()
            .set_local(Some(Box::new(move |u: &str| (u == u2).then(|| kept.clone()))));
        let seen = recorder(&p);
        p.load(&url, 0, None).unwrap();
        wait(&p, "the end", |s| s.ended == 1);
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .any(|s| s.from_device && s.now_playing.is_some()),
            "said it plays from the device"
        );
        p.stop();
        assert!(asked.lock().unwrap().is_empty(), "nothing asked of the node");
        assert_eq!(read_wav(&out_path).unwrap().samples, Samples::PcmI32(want));
        // Without a copy: the stream
        p.sources().set_local(None);
        p.load(&url, 0, None).unwrap();
        wait(&p, "the end of the stream", |s| s.ended == 2);
        assert!(!p.sources().from_device(&url));
        assert!(!asked.lock().unwrap().is_empty(), "streamed from the node");
        p.stop();
    }

    #[test]
    fn what_cant_be_decoded_here_is_left_to_the_page() {
        let p = AppPlayer::new(Arc::new(NullOutput::new()), OutputOwner::new());
        let (html, _) = serve(b"<html>busy</html>".to_vec(), "text/html", Mode::default());
        let e = p.load(&html, 0, None).unwrap_err();
        assert!(e.refused && e.message.contains("instead of audio"), "{e:?}");
        let (hls, _) = serve(
            b"#EXTM3U\n".to_vec(),
            "application/vnd.apple.mpegurl",
            Mode::default(),
        );
        let e = p.load(&hls, 0, None).unwrap_err();
        assert!(e.refused && e.message.contains("playlist"), "{e:?}");
        let (junk, _) = serve(vec![7u8; 4096], "application/octet-stream", Mode::default());
        let e = p.load(&junk, 0, None).unwrap_err();
        assert!(e.refused, "{e:?}");
        // A node that doesn't answer is no refusal: the page tries again
        let e = p
            .load("http://127.0.0.1:9/v1/library/tracks/1/file", 0, None)
            .unwrap_err();
        assert!(!e.refused, "{e:?}");
        assert_eq!(p.status().url, None, "nothing changed");
    }

    #[test]
    fn pause_play_and_seek_move_the_zone() {
        let (a, _) = wav_bytes(48_000, 16, 48_000 * 4, 1);
        let (ua, _) = serve(a, "audio/wav", Mode::default());
        let p = AppPlayer::new(Arc::new(NullOutput::realtime()), OutputOwner::new());
        p.load(&ua, 2_000, None).unwrap();
        let st = wait(&p, "playing past the seek", |s| {
            s.transport == Transport::Playing && s.position_ms >= 2_000
        });
        assert_eq!(st.duration_ms, Some(4_000));
        assert_eq!(
            st.now_playing.unwrap().output_format,
            AudioFormat::pcm(48_000, 16, 2)
        );
        p.pause();
        let at = wait(&p, "paused", |s| s.transport == Transport::Paused).position_ms;
        p.seek(500);
        wait(&p, "seeked while paused", |s| {
            s.position_ms == 500 && s.transport == Transport::Paused
        });
        p.play().unwrap();
        wait(&p, "playing again", |s| s.transport == Transport::Playing);
        assert!(at >= 2_000);
        p.stop();
        let st = wait(&p, "stopped", |s| s.transport == Transport::Stopped);
        assert_eq!(st.ended, 0, "a stop is no end");
        assert_eq!(st.url, None);
    }

    /// One output, one stream: the node's session takes it from this player (paused here with
    /// the reason, where it was), and a play here takes it back (the node's session is stopped).
    #[test]
    fn the_node_and_the_app_hand_the_output_over() {
        let (a, _) = wav_bytes(48_000, 16, 48_000 * 6, 1);
        let (ua, _) = serve(a, "audio/wav", Mode::default());
        let owner = OutputOwner::new();
        let out = Arc::new(NullOutput::realtime());
        let id = out.id();
        let p = AppPlayer::new(out, owner.clone());
        p.load(&ua, 0, None).unwrap();
        wait(&p, "playing", |s| {
            s.transport == Transport::Playing && s.position_ms > 300
        });
        assert_eq!(owner.holder(&id), Some(Holder::App));
        let node_stopped = Arc::new(AtomicBool::new(false));
        let ns = node_stopped.clone();
        let node = owner
            .take(
                &id,
                Holder::Node,
                Arc::new(move || ns.store(true, Ordering::SeqCst)),
                Duration::from_secs(5),
            )
            .unwrap();
        let st = wait(&p, "taken", |s| s.error.as_deref() == Some(TAKEN_BY_NODE));
        assert_eq!(st.transport, Transport::Paused);
        assert!(st.position_ms > 300, "{st:?}");
        assert_eq!(owner.holder(&id), Some(Holder::Node));
        // Play here again: the node's session is told to stop and lets go
        let slot = Arc::new(Mutex::new(Some(node)));
        let s2 = slot.clone();
        let release = std::thread::spawn(move || {
            while !node_stopped.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(10));
            }
            s2.lock().unwrap().take();
        });
        p.play().unwrap();
        let st = wait(&p, "playing here again", |s| {
            s.transport == Transport::Playing && s.position_ms > st.position_ms
        });
        assert!(st.error.is_none(), "{st:?}");
        release.join().unwrap();
        assert_eq!(owner.holder(&id), Some(Holder::App));
        p.stop();
        wait(&p, "let go", |_| owner.holder(&id).is_none());
    }

    /// A song tapped while another still opens (a far node): the later one plays, the earlier one
    /// gives up; a stop while one opens leaves the zone silent.
    #[test]
    fn a_later_load_or_a_stop_overtakes_one_still_opening() {
        let slow = Mode {
            delay_ms: 600,
            ..Mode::default()
        };
        let (a, _) = wav_bytes(48_000, 16, 48_000 * 3, 1);
        let (b, _) = wav_bytes(48_000, 16, 48_000 * 3, 2);
        let (ua, _) = serve(a.clone(), "audio/wav", slow);
        let (ub, _) = serve(b, "audio/wav", Mode::default());
        let p = AppPlayer::new(Arc::new(NullOutput::realtime()), OutputOwner::new());
        let p2 = p.clone();
        let ua2 = ua.clone();
        let first = std::thread::spawn(move || p2.load(&ua2, 0, None));
        std::thread::sleep(Duration::from_millis(100));
        p.load(&ub, 0, None).unwrap();
        let e = first.join().unwrap().unwrap_err();
        assert!(!e.refused, "{e:?}");
        let st = wait(&p, "the later song plays", |s| s.transport == Transport::Playing);
        assert_eq!(st.url.as_deref(), Some(ub.as_str()));
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            p.status().url.as_deref(),
            Some(ub.as_str()),
            "still the later one"
        );
        // A stop while a song opens
        let (uslow, _) = serve(a, "audio/wav", slow);
        let p3 = p.clone();
        let opening = std::thread::spawn(move || p3.load(&uslow, 0, None));
        std::thread::sleep(Duration::from_millis(100));
        p.stop();
        assert!(opening.join().unwrap().is_err());
        std::thread::sleep(Duration::from_millis(300));
        let st = p.status();
        assert_eq!((st.transport, st.url), (Transport::Stopped, None));
    }

    /// Repeat one hands the same song ahead: its end says the zone went on (`advanced`), so the
    /// page keeps it playing instead of opening it again.
    #[test]
    fn the_same_song_handed_ahead_says_the_zone_went_on() {
        let (a, _) = wav_bytes(44_100, 16, 20_000, 1);
        let (ua, _) = serve(a, "audio/wav", Mode::default());
        let p = AppPlayer::new(Arc::new(NullOutput::new()), OutputOwner::new());
        p.load(&ua, 0, Some(&ua)).unwrap();
        let st = wait(&p, "the end of the first", |s| s.ended >= 1);
        assert_eq!((st.advanced, st.url.as_deref()), (1, Some(ua.as_str())));
    }

    impl Page {
        fn on_test(&mut self, ev: &ZoneEvent) -> bool {
            self.on(ev).tell
        }
    }

    /// The break as the page learns it when the zone is already on the next song: paused where
    /// the song was last heard, at once, and the end that follows is swallowed.
    #[test]
    fn a_break_is_paused_where_the_song_was_last_heard() {
        let item = |id| QueueItem {
            id,
            ..QueueItem::new("u")
        };
        let mut pg = Page {
            url: Some("a".into()),
            current: Some(1),
            ahead: Some(("b".into(), 2)),
            ..Page::default()
        };
        pg.note(Some((1, 61_000)));
        // The zone has already published the next song (at 0) when the error is handled
        pg.note(Some((2, 0)));
        let h = pg.on(&ZoneEvent::Error {
            message: "decode: the stream broke off".into(),
        });
        assert!(h.tell && h.stop, "{h:?}");
        assert_eq!(pg.broke_at, Some(61_000));
        let out = NullOutput::new();
        let st = pg.status(None, &out);
        assert_eq!(
            (st.transport, st.position_ms, st.broke),
            (Transport::Paused, 61_000, true)
        );
        let h = pg.on(&ZoneEvent::TrackEnded {
            item: item(1),
            verify_ok: None,
        });
        assert!(!h.stop);
        assert_eq!(
            (pg.ended, pg.advanced, pg.url.as_deref()),
            (0, 0, Some("a")),
            "no end, not on to b"
        );
    }

    /// A stream that breaks for good mid-song (the link gone): no end, the zone doesn't go on to
    /// the song handed ahead, and the page sees it paused where it broke, with the error (its
    /// mid-song retry opens it there again).
    #[test]
    fn a_stream_that_breaks_mid_song_is_no_end() {
        let (a, _) = wav_bytes(44_100, 16, 44_100 * 5, 1);
        let (b, _) = wav_bytes(44_100, 16, 44_100, 2);
        let broken = Mode {
            break_at: Some(a.len() * 7 / 10),
            break_always: true,
            ..Mode::default()
        };
        let (ua, _) = serve(a, "audio/wav", broken);
        let (ub, _) = serve(b, "audio/wav", Mode::default());
        let p = AppPlayer::new(Arc::new(NullOutput::realtime()), OutputOwner::new());
        p.load(&ua, 0, Some(&ub)).unwrap();
        let st = wait(&p, "broke", |s| {
            s.error.is_some() && s.transport == Transport::Paused
        });
        assert_eq!((st.ended, st.advanced), (0, 0), "{st:?}");
        assert_eq!(st.url.as_deref(), Some(ua.as_str()), "not on to the next song");
        assert!(st.position_ms > 0, "where it broke: {st:?}");
        std::thread::sleep(Duration::from_millis(300));
        let z = p.zone_state().unwrap();
        assert_eq!(z.transport, Transport::Stopped, "the zone stopped: {z:?}");
    }

    #[test]
    fn the_page_hears_ends_only_for_its_own_songs() {
        let item = |id| QueueItem {
            id,
            ..QueueItem::new("u")
        };
        let mut pg = Page {
            url: Some("a".into()),
            current: Some(1),
            ahead: Some(("b".into(), 2)),
            ..Page::default()
        };
        assert!(pg.on_test(&ZoneEvent::TrackEnded {
            item: item(1),
            verify_ok: None
        }));
        assert_eq!((pg.ended, pg.url.as_deref(), pg.current), (1, Some("b"), Some(2)));
        assert!(pg.ahead.is_none());
        // An old item's end changes nothing
        assert!(!pg.on_test(&ZoneEvent::TrackEnded {
            item: item(1),
            verify_ok: None
        }));
        assert_eq!(pg.ended, 1);
        // The adopted one refused by the opener: the page plays it itself
        assert!(pg.on_test(&ZoneEvent::Skipped {
            item: item(2),
            reason: format!("{}unsupported codec", crate::source::CANT_PLAY_HERE)
        }));
        assert!(pg.refused.as_deref().is_some_and(crate::source::is_refusal));
    }
}
