//! Opt-in local diagnostics (first slice): the pages the owner opens in their own browser, how
//! long those pages and the requests they make take and whether they fail, and which class of
//! script error broke a page. Nothing else. The guide is docs/guides/local-diagnostics.md.
//!
//! - **Off by default.** The owner's node switch (`PUT /v1/node/diagnostics`) is a permission, not
//!   consent: each owner browser still has to press Start for itself, and gets a random,
//!   short-lived session id for that page load only (gone on reload, Log out, or a token change).
//!   The switch turns itself off after 24 h, and a restart starts Off (nothing here is persisted).
//! - **The owner, signed in for real.** Every route is under `/node` (listeners and party guests
//!   are refused there already), and every handler checks the request's own credential itself:
//!   a node without an API token (where every direct request is the owner) refuses them all. The
//!   credential is checked again once a request's body has arrived, and a change only lands if no
//!   owner credential was revoked since the request was let in ([`Diagnostics::revoke`]: Log out,
//!   a new token, another identity). Listeners have no consent model, so nothing is taken from them.
//! - **Generations.** Every On and Off starts a new generation. A Start or an On names the
//!   generation the page showed when it was pressed, and is refused if it changed meanwhile: a
//!   Start sent before someone turned diagnostics off and on again never joins the new one. Off
//!   always applies.
//! - **A closed schema.** Events are enums only (no text, URLs, titles, keys, messages, stacks),
//!   unknown fields refuse the whole batch, and refusals never echo the input. Timestamps are the
//!   node's own; the client sends none.
//! - **Bounded, in memory.** A ring of at most [`MAX_EVENTS`] events and [`MAX_BYTES`], kept
//!   [`RETENTION`]; batches of [`MAX_BATCH`] and [`MAX_BODY`]; [`PER_SESSION_PER_MIN`] and
//!   [`GLOBAL_PER_MIN`]; at most [`MAX_SESSIONS`] sessions. A background task looks every
//!   [`HOUSEKEEPING`] while the node runs (and every use looks too), so the switch, sessions and
//!   events end within that much of their time even when no one uses diagnostics. Off ends every
//!   session at once and a batch from before (an older generation) is refused, never replayed.
//!   Clear empties the ring. What was dropped on the way is counted, not hidden: per hour, each
//!   count kept [`RETENTION`] from when the node received it (a browser reports its own drops
//!   with its next batch, up to an hour after them).
//! - **Export** (`GET /v1/node/diagnostics/export`, a manual download) rebuilds the records from
//!   the schema, with per-export pseudonyms (`s1`, `s2`…) instead of session ids, finite aggregates
//!   and a manifest. Never the node's logs, raw errors, tokens or anyone's key.
//!
//! Not in this slice: playback and copy transitions, script errors outside a page being drawn,
//! and anything from listeners.

use crate::accounts::{Caller, Role};
use crate::state::NodeState;
use crate::{ApiError, ApiResult};
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The export's schema name and version.
pub const SCHEMA: &str = "pm-diagnostics/1";
/// The node switch turns itself off after this long.
pub const GATE_TTL: Duration = Duration::from_secs(24 * 3600);
/// A browser's Start lasts at most this long (and only until it reloads).
pub const SESSION_TTL: Duration = Duration::from_secs(3600);
/// Events older than this are dropped.
pub const RETENTION: Duration = Duration::from_secs(72 * 3600);
/// How often the background task ends what has run out, when no one uses diagnostics.
pub const HOUSEKEEPING: Duration = Duration::from_secs(60);
pub const MAX_EVENTS: usize = 50_000;
/// Serialized size of the kept events, as exported.
pub const MAX_BYTES: usize = 50 << 20;
pub const MAX_SESSIONS: usize = 32;
pub const PER_SESSION_PER_MIN: u32 = 120;
pub const GLOBAL_PER_MIN: u32 = 2000;
pub const MAX_BATCH: usize = 32;
/// The largest events request body.
pub const MAX_BODY: usize = 16 << 10;
/// The largest switch or Start body.
const MAX_SMALL_BODY: usize = 1 << 10;
const MINUTE: Duration = Duration::from_secs(60);
/// What was dropped is counted per hour, and each hour's count goes with the events of that hour.
const DROP_BUCKET: Duration = Duration::from_secs(3600);

// ---- the schema: enums only ----

/// The app's views (app.js `renderView`), `-` written `_`. A page outside this list is not
/// recorded at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum View {
    AddMusic,
    Albums,
    Artists,
    Dashboard,
    Downloads,
    Favourites,
    Genre,
    Genres,
    Home,
    HomeSection,
    Identify,
    Item,
    Library,
    Person,
    Playlist,
    Playlists,
    Queue,
    Radio,
    Search,
    Settings,
    Shared,
    Smart,
    SmartEdit,
    SourceArtist,
    SourceItem,
    Sources,
    Tag,
    Tracks,
    Zones,
}

/// Which family of API reads a page made while it loaded (app.js `DIAG_ROUTES`, an explicit list
/// of read routes; a request not on it is not recorded).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Area {
    Library,
    Search,
    Item,
    Playlists,
    Shared,
    Sources,
    Zones,
    Outputs,
    Node,
    Home,
    Me,
    People,
    Radio,
    Tags,
    Favourites,
    Listenbrainz,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    /// 401/403 from the node
    Refused,
    #[serde(rename = "http_4xx")]
    Http4xx,
    #[serde(rename = "http_5xx")]
    Http5xx,
    /// No answer (network down, node unreachable)
    Unreachable,
    /// The view's own code threw while drawing
    Failed,
}

/// How long, in coarse steps (never exact milliseconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ms {
    Lt100,
    Lt300,
    Lt1000,
    Lt3000,
    Lt10000,
    Ge10000,
}

/// The class of a script error that broke a page while it was drawn: the built-in constructor's
/// name, nothing of its message or stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    TypeError,
    ReferenceError,
    RangeError,
    SyntaxError,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Viewport {
    Phone,
    Tablet,
    Desktop,
}

/// One event, as the browser sends it and as the export writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    Nav { view: View },
    Load { view: View, outcome: Outcome, ms: Ms },
    Api { area: Area, outcome: Outcome, ms: Ms },
    Exception { class: ErrorClass },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Batch {
    session: String,
    events: Vec<Event>,
    /// Events this browser dropped since its last accepted batch (its own limits, a lost send).
    #[serde(default)]
    dropped: u16,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Gate {
    enabled: bool,
    /// The generation the page showed (`Status::generation`); needed to turn diagnostics on.
    #[serde(default)]
    generation: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    viewport: Viewport,
    /// The generation the page showed when Start was pressed.
    generation: u64,
}

// ---- the ring ----

/// The limits (the constants above; smaller in tests).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Limits {
    pub max_events: usize,
    pub max_bytes: usize,
    pub retention_hours: u64,
    pub max_sessions: usize,
    pub per_session_per_min: u32,
    pub global_per_min: u32,
    pub max_batch: usize,
    pub max_body_bytes: usize,
    pub session_minutes: u64,
    pub node_switch_hours: u64,
    /// How often idle data is looked at (and ended when its time is up).
    pub cleanup_seconds: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_events: MAX_EVENTS,
            max_bytes: MAX_BYTES,
            retention_hours: RETENTION.as_secs() / 3600,
            max_sessions: MAX_SESSIONS,
            per_session_per_min: PER_SESSION_PER_MIN,
            global_per_min: GLOBAL_PER_MIN,
            max_batch: MAX_BATCH,
            max_body_bytes: MAX_BODY,
            session_minutes: SESSION_TTL.as_secs() / 60,
            node_switch_hours: GATE_TTL.as_secs() / 3600,
            cleanup_seconds: HOUSEKEEPING.as_secs(),
        }
    }
}

/// How long things last ([`GATE_TTL`], [`SESSION_TTL`], [`RETENTION`]; shorter in tests).
#[derive(Debug, Clone, Copy)]
struct Ttl {
    gate: Duration,
    session: Duration,
    retention: Duration,
}

impl Dropped {
    fn add(&mut self, o: Dropped) {
        self.evicted += o.evicted;
        self.rate_limited += o.rate_limited;
        self.by_browsers += o.by_browsers;
    }
}

impl Default for Ttl {
    fn default() -> Self {
        Self {
            gate: GATE_TTL,
            session: SESSION_TTL,
            retention: RETENTION,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Window {
    start: Instant,
    count: u32,
}

impl Window {
    fn new(now: Instant) -> Self {
        Self { start: now, count: 0 }
    }
    fn room(&mut self, now: Instant, n: u32, limit: u32) -> bool {
        if now.duration_since(self.start) >= MINUTE {
            *self = Self::new(now);
        }
        self.count + n <= limit
    }
}

struct Session {
    /// A keyed hash of the credential it started with (memory only, never exported).
    token: [u8; 32],
    generation: u64,
    started: Instant,
    /// The ring's own number for it (exports rename it).
    seq: u64,
    viewport: Viewport,
    window: Window,
}

struct Stored {
    at: Instant,
    unix: u64,
    seq: u64,
    viewport: Viewport,
    bytes: usize,
    event: Event,
}

/// What did not make it into the ring, so an export never reads as complete when it isn't. Kept
/// per hour, [`RETENTION`] from when the node received the count (up to an hour less).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Dropped {
    /// Pushed out by the event or byte cap (oldest first).
    pub evicted: u64,
    /// In batches refused for the per-minute limits.
    pub rate_limited: u64,
    /// Dropped by the browsers themselves (their queue and per-minute limits, a lost send).
    pub by_browsers: u64,
}

#[derive(Default)]
struct Inner {
    /// When the node switch was turned on (`None` = Off).
    on_since: Option<Instant>,
    /// Bumped by every On and Off: sessions from another generation are over.
    generation: u64,
    /// Bumped whenever an owner credential stops working ([`Diagnostics::revoke`]).
    epoch: u64,
    sessions: HashMap<String, Session>,
    next_seq: u64,
    ring: VecDeque<Stored>,
    bytes: usize,
    global: Option<Window>,
    /// What was dropped, per hour (at most retention / 1 h + 1 of them).
    dropped: VecDeque<(Instant, Dropped)>,
}

impl Inner {
    fn note_dropped(&mut self, now: Instant, d: Dropped) {
        match self.dropped.back_mut() {
            Some((since, sum)) if now.duration_since(*since) < DROP_BUCKET => sum.add(d),
            _ => self.dropped.push_back((now, d)),
        }
    }
    fn dropped(&self) -> Dropped {
        let mut all = Dropped::default();
        for (_, d) in &self.dropped {
            all.add(*d);
        }
        all
    }
}

/// Why something was refused (each a fixed answer, never the input).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Off,
    /// Diagnostics were turned off or on since the page showed them.
    Stale,
    /// An owner credential was revoked while this request was on its way.
    Revoked,
    Gone,
    TooMany,
    TooLarge,
    RateLimited,
    BadSession,
    /// A batch without events.
    Empty,
}

impl From<Refusal> for ApiError {
    fn from(r: Refusal) -> Self {
        let (status, why) = match r {
            Refusal::Off => (StatusCode::CONFLICT, "diagnostics are off on this node"),
            Refusal::Stale => (
                StatusCode::CONFLICT,
                "diagnostics were turned off or on since this page showed them; look again",
            ),
            Refusal::Revoked => (
                StatusCode::CONFLICT,
                "a sign-in of the owner ended while this was sent; nothing was changed",
            ),
            Refusal::Gone => (StatusCode::GONE, "this diagnostics session has ended"),
            Refusal::TooMany => (StatusCode::TOO_MANY_REQUESTS, "too many diagnostics sessions"),
            Refusal::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "too many events in one batch"),
            Refusal::RateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                "too many diagnostics events; wait a minute",
            ),
            Refusal::BadSession => (StatusCode::BAD_REQUEST, "not a diagnostics session id"),
            Refusal::Empty => (StatusCode::BAD_REQUEST, "a batch needs events"),
        };
        ApiError::new(status, why)
    }
}

/// The authorization epoch when a request was let in ([`Diagnostics::epoch`]), taken before its
/// body is read: if an owner credential is revoked meanwhile, the request changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Epoch(u64);

/// The node's diagnostics: the switch, the browsers that pressed Start, and the ring.
pub struct Diagnostics {
    inner: Mutex<Inner>,
    limits: Limits,
    ttl: Ttl,
    /// Keys the credential hashes, so they mean nothing outside this process.
    key: [u8; 32],
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self::with_limits(Limits::default())
    }
}

/// A status, for Settings.
#[derive(Debug, Serialize)]
pub struct Status {
    pub enabled: bool,
    /// Changes with every On and Off; a Start or an On names the one the page showed.
    pub generation: u64,
    /// Seconds until the node switch turns itself off.
    pub expires_in_s: Option<u64>,
    pub events: usize,
    pub bytes: usize,
    pub sessions: usize,
    pub dropped: Dropped,
    pub limits: Limits,
}

impl Diagnostics {
    pub fn with_limits(limits: Limits) -> Self {
        Self::with(limits, Ttl::default())
    }

    fn with(limits: Limits, ttl: Ttl) -> Self {
        Self {
            inner: Mutex::default(),
            limits,
            ttl,
            key: rand::random(),
        }
    }

    /// The node's diagnostics, with the background task that ends what has run out every
    /// [`HOUSEKEEPING`] while this exists (started when there is a tokio runtime; it holds a weak
    /// reference, so it ends with the node).
    pub fn housekept() -> Arc<Self> {
        let d = Arc::new(Self::default());
        d.spawn_housekeeping(HOUSEKEEPING);
        d
    }

    /// Starts the housekeeping task, every `every`. `None` outside a tokio runtime (then only use
    /// ends things).
    pub fn spawn_housekeeping(self: &Arc<Self>, every: Duration) -> Option<tokio::task::JoinHandle<()>> {
        let rt = tokio::runtime::Handle::try_current().ok()?;
        let weak = Arc::downgrade(self);
        Some(rt.spawn(async move {
            let mut tick = tokio::time::interval(every);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let Some(d) = weak.upgrade() else { break };
                d.housekeep(Instant::now());
            }
        }))
    }

    /// Ends what has run out (what the housekeeping task does).
    pub fn housekeep(&self, now: Instant) {
        let mut inner = self.lock();
        self.expire(&mut inner, now);
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn token_hash(&self, token: &str) -> [u8; 32] {
        *blake3::keyed_hash(&self.key, token.as_bytes()).as_bytes()
    }

    /// The authorization epoch now: take it before reading a request's body.
    pub fn epoch(&self) -> Epoch {
        Epoch(self.lock().epoch)
    }

    /// An owner credential stopped working (Log out of one browser session): the diagnostics
    /// sessions started with it end, and no request let in before now changes anything.
    pub fn revoke(&self, token: &str) {
        let h = self.token_hash(token);
        let mut inner = self.lock();
        inner.epoch += 1;
        inner.sessions.retain(|_, s| s.token != h);
    }

    /// Every owner credential may have changed (a new API token, another identity): every
    /// diagnostics session ends, and no request let in before now changes anything.
    pub fn revoke_all(&self) {
        let mut inner = self.lock();
        inner.epoch += 1;
        inner.sessions.clear();
    }

    /// The node takes on another identity (or none): [`Self::revoke_all`], with `swap` putting it
    /// in place under the same hold. A change let in under the old identity happened before, or
    /// lands nowhere; one let in from now on was checked against the new one. Whatever `swap`
    /// does, every revocation stands. `swap` takes no lock that is ever held while this one is
    /// taken (lock order: the diagnostics, then the identity).
    pub(crate) fn revoke_all_across<R>(&self, swap: impl FnOnce() -> R) -> R {
        let mut inner = self.lock();
        inner.epoch += 1;
        inner.sessions.clear();
        swap()
    }

    /// Ends what has run out: the switch after 24 h, sessions after an hour, events after 72 h.
    fn expire(&self, inner: &mut Inner, now: Instant) {
        if inner
            .on_since
            .is_some_and(|t| now.duration_since(t) >= self.ttl.gate)
        {
            Self::switch_off(inner);
        }
        // The node-wide rate window is over after its minute
        if inner
            .global
            .is_some_and(|w| now.duration_since(w.start) >= MINUTE)
        {
            inner.global = None;
        }
        let session = self.ttl.session;
        inner
            .sessions
            .retain(|_, s| now.duration_since(s.started) < session);
        while inner
            .ring
            .front()
            .is_some_and(|e| now.duration_since(e.at) >= self.ttl.retention)
        {
            Self::pop(inner);
        }
        while inner
            .dropped
            .front()
            .is_some_and(|(since, _)| now.duration_since(*since) >= self.ttl.retention)
        {
            inner.dropped.pop_front();
        }
    }

    fn pop(inner: &mut Inner) {
        if let Some(e) = inner.ring.pop_front() {
            inner.bytes -= e.bytes;
        }
    }

    fn switch_off(inner: &mut Inner) {
        if inner.on_since.take().is_some() {
            inner.generation += 1;
        }
        inner.sessions.clear();
    }

    /// The lock, with what has run out ended, if no owner credential was revoked since `at`.
    fn guard(&self, at: Epoch, now: Instant) -> Result<std::sync::MutexGuard<'_, Inner>, Refusal> {
        let mut inner = self.lock();
        if inner.epoch != at.0 {
            return Err(Refusal::Revoked);
        }
        self.expire(&mut inner, now);
        Ok(inner)
    }

    fn status_of(&self, inner: &Inner, now: Instant) -> Status {
        Status {
            enabled: inner.on_since.is_some(),
            generation: inner.generation,
            expires_in_s: inner
                .on_since
                .map(|t| self.ttl.gate.saturating_sub(now.duration_since(t)).as_secs()),
            events: inner.ring.len(),
            bytes: inner.bytes,
            sessions: inner.sessions.len(),
            dropped: inner.dropped(),
            limits: self.limits,
        }
    }

    pub fn status(&self, at: Epoch, now: Instant) -> Result<Status, Refusal> {
        let inner = self.guard(at, now)?;
        Ok(self.status_of(&inner, now))
    }

    /// Whether the switch is on (before a body is read: nothing is read while Off).
    fn is_on(&self, now: Instant) -> bool {
        let mut inner = self.lock();
        self.expire(&mut inner, now);
        inner.on_since.is_some()
    }

    /// The node switch. Off always applies and ends every browser's session. On applies only if
    /// `shown` is the generation now (the page showed what is so), and starts a new generation.
    pub fn set_enabled(
        &self,
        at: Epoch,
        on: bool,
        shown: Option<u64>,
        now: Instant,
    ) -> Result<Status, Refusal> {
        let mut inner = self.guard(at, now)?;
        if !on {
            // Every explicit Off is a new generation, also when already off: an On shown before
            // it is refused (turning itself off counts once, in `switch_off`)
            if inner.on_since.is_none() {
                inner.generation += 1;
            }
            Self::switch_off(&mut inner);
        } else if shown != Some(inner.generation) {
            return Err(Refusal::Stale);
        } else if inner.on_since.is_none() {
            inner.generation += 1;
            inner.on_since = Some(now);
        }
        Ok(self.status_of(&inner, now))
    }

    /// A browser's Start: a fresh random id, bound to its credential and to `shown`, the
    /// generation the page showed when Start was pressed (refused if it is not the one now).
    pub fn start(
        &self,
        at: Epoch,
        token: &str,
        viewport: Viewport,
        shown: u64,
        now: Instant,
    ) -> Result<String, Refusal> {
        let token = self.token_hash(token);
        let mut inner = self.guard(at, now)?;
        if inner.on_since.is_none() {
            return Err(Refusal::Off);
        }
        if inner.generation != shown {
            return Err(Refusal::Stale);
        }
        if inner.sessions.len() >= self.limits.max_sessions {
            return Err(Refusal::TooMany);
        }
        let id = format!("{:016x}", rand::random::<u64>());
        inner.next_seq += 1;
        let session = Session {
            token,
            generation: inner.generation,
            started: now,
            seq: inner.next_seq,
            viewport,
            window: Window::new(now),
        };
        inner.sessions.insert(id.clone(), session);
        Ok(id)
    }

    /// A browser's Stop (only the browser that started it, by its credential).
    pub fn stop(&self, at: Epoch, id: &str, token: &str, now: Instant) -> Result<(), Refusal> {
        let token = self.token_hash(token);
        let mut inner = self.guard(at, now)?;
        if inner.sessions.get(id).is_some_and(|s| s.token == token) {
            inner.sessions.remove(id);
        }
        Ok(())
    }

    pub fn clear(&self, at: Epoch, now: Instant) -> Result<(), Refusal> {
        let mut inner = self.guard(at, now)?;
        inner.ring.clear();
        inner.bytes = 0;
        inner.dropped.clear();
        Ok(())
    }

    /// Keeps a batch from a live session of this generation and credential, within the limits.
    #[allow(clippy::too_many_arguments)]
    pub fn ingest(
        &self,
        at: Epoch,
        id: &str,
        token: &str,
        events: &[Event],
        dropped: u16,
        now: Instant,
        unix: u64,
    ) -> Result<(), Refusal> {
        if id.len() != 16
            || !id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(Refusal::BadSession);
        }
        if events.is_empty() {
            return Err(Refusal::Empty);
        }
        let token = self.token_hash(token);
        let limits = self.limits;
        let mut guard = self.guard(at, now)?;
        let inner = &mut *guard;
        if inner.on_since.is_none() {
            return Err(Refusal::Off);
        }
        if events.len() > limits.max_batch {
            return Err(Refusal::TooLarge);
        }
        let generation = inner.generation;
        let Some(session) = inner
            .sessions
            .get_mut(id)
            .filter(|s| s.token == token && s.generation == generation)
        else {
            return Err(Refusal::Gone);
        };
        let n = events.len() as u32;
        let global = inner.global.get_or_insert_with(|| Window::new(now));
        if !session.window.room(now, n, limits.per_session_per_min)
            || !global.room(now, n, limits.global_per_min)
        {
            let d = Dropped {
                rate_limited: u64::from(n),
                ..Dropped::default()
            };
            inner.note_dropped(now, d);
            return Err(Refusal::RateLimited);
        }
        session.window.count += n;
        global.count += n;
        let (seq, viewport) = (session.seq, session.viewport);
        let mut lost = Dropped {
            by_browsers: u64::from(dropped),
            ..Dropped::default()
        };
        for &event in events {
            let bytes = record_size(&event, unix);
            inner.ring.push_back(Stored {
                at: now,
                unix,
                seq,
                viewport,
                bytes,
                event,
            });
            inner.bytes += bytes;
        }
        while inner.ring.len() > limits.max_events || inner.bytes > limits.max_bytes {
            Self::pop(inner);
            lost.evicted += 1;
        }
        if lost != Dropped::default() {
            inner.note_dropped(now, lost);
        }
        Ok(())
    }

    /// The export: rebuilt from the schema, with pseudonyms for the sessions.
    pub fn export(
        &self,
        at: Epoch,
        app_version: &str,
        now: Instant,
        unix: u64,
    ) -> Result<serde_json::Value, Refusal> {
        let inner = self.guard(at, now)?;
        let mut names: BTreeMap<u64, (String, Viewport, usize)> = BTreeMap::new();
        let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
        let mut views: BTreeMap<String, usize> = BTreeMap::new();
        let mut loads: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
        let mut api: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
        let mut durations: BTreeMap<String, usize> = BTreeMap::new();
        let mut exceptions: BTreeMap<String, usize> = BTreeMap::new();
        let mut events = Vec::with_capacity(inner.ring.len());
        for e in &inner.ring {
            let next = names.len() + 1;
            let entry = names
                .entry(e.seq)
                .or_insert_with(|| (format!("s{next}"), e.viewport, 0));
            entry.2 += 1;
            let session = entry.0.clone();
            match e.event {
                Event::Nav { view } => {
                    *kinds.entry("nav").or_default() += 1;
                    *views.entry(name(&view)).or_default() += 1;
                }
                Event::Load { view, outcome, ms } => {
                    *kinds.entry("load").or_default() += 1;
                    *loads
                        .entry(name(&view))
                        .or_default()
                        .entry(name(&outcome))
                        .or_default() += 1;
                    *durations.entry(name(&ms)).or_default() += 1;
                }
                Event::Api { area, outcome, ms } => {
                    *kinds.entry("api").or_default() += 1;
                    *api.entry(name(&area))
                        .or_default()
                        .entry(name(&outcome))
                        .or_default() += 1;
                    *durations.entry(name(&ms)).or_default() += 1;
                }
                Event::Exception { class } => {
                    *kinds.entry("exception").or_default() += 1;
                    *exceptions.entry(name(&class)).or_default() += 1;
                }
            }
            events.push(Record {
                at: e.unix,
                session,
                event: e.event,
            });
        }
        let mut sessions: Vec<_> = names.into_values().collect();
        sessions.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(&b.0)));
        Ok(serde_json::json!({
            "manifest": {
                "schema": SCHEMA,
                "app_version": app_version,
                "exported_at": unix,
                "limits": self.limits,
                "contents": "Pages the owner opened in browsers that pressed Start, how long those pages and the reads they made took and how they ended, and the class of script error that broke a page. No text, addresses, titles, keys, messages or logs. Not a complete record: see dropped.",
                "dropped": inner.dropped(),
            },
            "sessions": sessions.iter().map(|(id, viewport, n)| serde_json::json!({"id": id, "viewport": viewport, "events": n})).collect::<Vec<_>>(),
            "aggregates": {
                "kinds": kinds,
                "views": views,
                "loads": loads,
                "api": api,
                "durations": durations,
                "exceptions": exceptions,
            },
            "events": events,
        }))
    }

    /// What the ring and the session table hold right now, without ending anything first.
    #[cfg(test)]
    fn held(&self) -> (usize, usize, bool) {
        let inner = self.lock();
        (inner.ring.len(), inner.sessions.len(), inner.on_since.is_some())
    }

    #[cfg(test)]
    fn held_drop_hours(&self) -> usize {
        self.lock().dropped.len()
    }
}

/// One exported event.
#[derive(Serialize)]
struct Record {
    /// Unix seconds, the node's clock.
    at: u64,
    session: String,
    #[serde(flatten)]
    event: Event,
}

/// An enum's name in the schema.
fn name<T: Serialize>(t: &T) -> String {
    serde_json::to_value(t)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// The size an event takes in the export (pseudonyms sized for the largest ring).
fn record_size(event: &Event, unix: u64) -> usize {
    serde_json::to_vec(&Record {
        at: unix,
        session: "s50000".into(),
        event: *event,
    })
    .map(|v| v.len() + 1)
    .unwrap_or(256)
}

// ---- routes (all under /v1/node/diagnostics) ----

fn owner(c: &Caller) -> Result<(), ApiError> {
    if c.role == Role::Owner {
        Ok(())
    } else {
        Err(crate::accounts::refused("diagnostics are the owner's"))
    }
}

/// The owner's credential this request carries, checked now (not the middleware's answer from
/// when the request came in). A node without an API token (or an empty one) makes everyone the
/// owner: diagnostics are never open there.
fn credential(
    s: &NodeState,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> Result<String, ApiError> {
    if !matches!(s.token(), Some(t) if !t.is_empty()) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "diagnostics need the node's API token ([api] token): without one there is no owner sign-in to check",
        ));
    }
    let unknown = || ApiError::new(StatusCode::UNAUTHORIZED, "missing or wrong bearer token");
    let token = crate::bearer(headers, query)
        .filter(|t| !t.is_empty())
        .ok_or_else(unknown)?;
    match crate::accounts::caller(s, Some(&token)) {
        Some(c) if c.role == Role::Owner => Ok(token),
        Some(_) => Err(crate::accounts::refused("diagnostics are the owner's")),
        None => Err(unknown()),
    }
}

/// Reads a request body of at most `limit` bytes; a refusal never quotes it.
async fn read(body: Body, limit: usize) -> Result<Vec<u8>, ApiError> {
    use futures_util::StreamExt;
    let mut stream = body.into_data_stream();
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ApiError::bad_request("the request did not arrive whole"))?;
        if out.len() + chunk.len() > limit {
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "that is more than diagnostics take in one request",
            ));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// Reads a request body into the schema; a refusal never quotes it.
fn parse<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|_| ApiError::bad_request("that is not in the diagnostics schema"))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

type Token = Query<HashMap<String, String>>;

/// The owner, checked again here; the epoch first, so a revocation from now on is seen.
fn let_in(
    s: &NodeState,
    c: &Caller,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> Result<(Epoch, String), ApiError> {
    owner(c)?;
    let at = s.diagnostics.epoch();
    let token = credential(s, headers, query)?;
    Ok((at, token))
}

/// `GET /v1/node/diagnostics`
pub(crate) async fn status(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
    Query(query): Token,
) -> ApiResult<Status> {
    let (at, _) = let_in(&s, &c, &headers, &query)?;
    Ok(Json(s.diagnostics.status(at, Instant::now())?))
}

/// `PUT /v1/node/diagnostics {"enabled": bool, "generation": n}`
pub(crate) async fn set_gate(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
    Query(query): Token,
    body: Body,
) -> ApiResult<Status> {
    let (at, _) = let_in(&s, &c, &headers, &query)?;
    let body = read(body, MAX_SMALL_BODY).await?;
    // Again, now that the body is here: a credential revoked meanwhile changes nothing
    credential(&s, &headers, &query)?;
    let gate: Gate = parse(&body)?;
    let st = s
        .diagnostics
        .set_enabled(at, gate.enabled, gate.generation, Instant::now())?;
    Ok(Json(st))
}

/// `POST /v1/node/diagnostics/sessions {"viewport": …, "generation": n}`: this browser's Start.
pub(crate) async fn start(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
    Query(query): Token,
    body: Body,
) -> ApiResult<serde_json::Value> {
    let (at, _) = let_in(&s, &c, &headers, &query)?;
    let body = read(body, MAX_SMALL_BODY).await?;
    let token = credential(&s, &headers, &query)?;
    let start: Start = parse(&body)?;
    let id = s
        .diagnostics
        .start(at, &token, start.viewport, start.generation, Instant::now())?;
    Ok(Json(serde_json::json!({
        "session": id,
        "expires_in_s": s.diagnostics.ttl.session.as_secs(),
        "max_batch": s.diagnostics.limits.max_batch,
    })))
}

/// `DELETE /v1/node/diagnostics/sessions/{id}`: this browser's Stop.
pub(crate) async fn stop(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
    Query(query): Token,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let (at, token) = let_in(&s, &c, &headers, &query)?;
    s.diagnostics.stop(at, &id, &token, Instant::now())?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/node/diagnostics/events {"session", "events": […], "dropped": n}`
pub(crate) async fn ingest(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
    Query(query): Token,
    body: Body,
) -> Result<StatusCode, ApiError> {
    let (at, _) = let_in(&s, &c, &headers, &query)?;
    // Off is checked before the body is read
    if !s.diagnostics.is_on(Instant::now()) {
        return Err(Refusal::Off.into());
    }
    let body = read(body, s.diagnostics.limits.max_body_bytes).await?;
    let token = credential(&s, &headers, &query)?;
    let batch: Batch = parse(&body)?;
    s.diagnostics.ingest(
        at,
        &batch.session,
        &token,
        &batch.events,
        batch.dropped,
        Instant::now(),
        unix_now(),
    )?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/node/diagnostics/clear`: empties the ring (the switch stays as it is).
pub(crate) async fn clear(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
    Query(query): Token,
) -> Result<StatusCode, ApiError> {
    let (at, _) = let_in(&s, &c, &headers, &query)?;
    s.diagnostics.clear(at, Instant::now())?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /v1/node/diagnostics/export`: a download for the owner.
pub(crate) async fn export(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
    Query(query): Token,
) -> Result<Response, ApiError> {
    let (at, _) = let_in(&s, &c, &headers, &query)?;
    let out = s.diagnostics.export(at, &s.version, Instant::now(), unix_now())?;
    Ok((
        [
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"odeion-diagnostics.json\"",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Json(out),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAV: Event = Event::Nav { view: View::Home };
    const T: &str = "tok";

    fn on(d: &Diagnostics, now: Instant) -> String {
        let at = d.epoch();
        let g = d.status(at, now).unwrap().generation;
        d.set_enabled(at, true, Some(g), now).unwrap();
        let g = d.status(at, now).unwrap().generation;
        d.start(at, T, Viewport::Desktop, g, now).unwrap()
    }

    fn put(d: &Diagnostics, id: &str, token: &str, events: &[Event], now: Instant) -> Result<(), Refusal> {
        d.ingest(d.epoch(), id, token, events, 0, now, 0)
    }

    fn generation(d: &Diagnostics, now: Instant) -> u64 {
        d.status(d.epoch(), now).unwrap().generation
    }

    #[test]
    fn the_switch_turns_itself_off_after_a_day_and_ends_sessions() {
        let d = Diagnostics::default();
        let t0 = Instant::now();
        let id = on(&d, t0);
        assert!(
            d.status(d.epoch(), t0 + Duration::from_secs(23 * 3600))
                .unwrap()
                .enabled
        );
        let later = t0 + GATE_TTL;
        assert!(!d.status(d.epoch(), later).unwrap().enabled);
        assert_eq!(put(&d, &id, T, &[NAV], later), Err(Refusal::Off));
        // On again: the old session belongs to an older generation, no replay
        let g = generation(&d, later);
        d.set_enabled(d.epoch(), true, Some(g), later).unwrap();
        assert_eq!(put(&d, &id, T, &[NAV], later), Err(Refusal::Gone));
    }

    #[test]
    fn a_session_lasts_an_hour_and_only_for_its_own_credential() {
        let d = Diagnostics::default();
        let t0 = Instant::now();
        let id = on(&d, t0);
        assert_eq!(put(&d, &id, "other", &[NAV], t0), Err(Refusal::Gone));
        assert_eq!(put(&d, &id, "", &[NAV], t0), Err(Refusal::Gone));
        // Another credential can't stop it either
        d.stop(d.epoch(), &id, "other", t0).unwrap();
        assert_eq!(put(&d, &id, T, &[NAV], t0), Ok(()));
        assert_eq!(put(&d, &id, T, &[NAV], t0 + SESSION_TTL), Err(Refusal::Gone));
    }

    #[test]
    fn on_and_start_name_the_generation_they_were_shown_off_never_needs_to() {
        let d = Diagnostics::default();
        let t0 = Instant::now();
        let at = d.epoch();
        let shown = generation(&d, t0);
        // On without the generation shown, or with an old one: refused
        assert_eq!(d.set_enabled(at, true, None, t0).unwrap_err(), Refusal::Stale);
        assert_eq!(
            d.set_enabled(at, true, Some(shown + 1), t0).unwrap_err(),
            Refusal::Stale
        );
        let st = d.set_enabled(at, true, Some(shown), t0).unwrap();
        assert!(st.enabled && st.generation == shown + 1);
        // A Start shown at the old generation (when it was off): refused
        assert_eq!(d.start(at, T, Viewport::Phone, shown, t0), Err(Refusal::Stale));
        d.start(at, T, Viewport::Phone, st.generation, t0).unwrap();
        // Off then On: a Start shown before is refused; Off with any (or no) generation applies
        d.set_enabled(at, false, None, t0).unwrap();
        let g = generation(&d, t0);
        d.set_enabled(at, true, Some(g), t0).unwrap();
        assert_eq!(
            d.start(at, T, Viewport::Phone, st.generation, t0),
            Err(Refusal::Stale)
        );
        assert!(!d.set_enabled(at, false, Some(0), t0).unwrap().enabled);
        // While Off, a Start is Off whatever it names
        let g = generation(&d, t0);
        assert_eq!(d.start(at, T, Viewport::Phone, g, t0), Err(Refusal::Off));
    }

    #[test]
    fn a_revoked_credential_ends_its_sessions_and_nothing_let_in_before_lands() {
        let d = Diagnostics::default();
        let t0 = Instant::now();
        let id = on(&d, t0);
        let g = generation(&d, t0);
        let other = d.start(d.epoch(), "pmo_other", Viewport::Phone, g, t0).unwrap();
        // A request let in now, whose body is still on the way
        let before = d.epoch();
        d.revoke(T);
        for refused in [
            d.ingest(before, &other, "pmo_other", &[NAV], 0, t0, 0),
            d.start(before, "pmo_other", Viewport::Phone, g, t0).map(|_| ()),
            d.set_enabled(before, false, None, t0).map(|_| ()),
            d.clear(before, t0),
            d.stop(before, &other, "pmo_other", t0),
            d.status(before, t0).map(|_| ()),
            d.export(before, "v", t0, 0).map(|_| ()),
        ] {
            assert_eq!(refused, Err(Refusal::Revoked));
        }
        // Its session is over; another browser's goes on
        assert_eq!(put(&d, &id, T, &[NAV], t0), Err(Refusal::Gone));
        assert_eq!(put(&d, &other, "pmo_other", &[NAV], t0), Ok(()));
        // Every credential: every session
        d.revoke_all();
        assert_eq!(put(&d, &other, "pmo_other", &[NAV], t0), Err(Refusal::Gone));
        assert_eq!(d.held().1, 0);
    }

    fn test_node(dir: &std::path::Path) -> Arc<NodeState> {
        let db = dir.join("lib.sqlite");
        NodeState::new(
            crate::NodeSettings {
                name: "t".into(),
                version: "0".into(),
                db_path: db.clone(),
                scan_folders: vec![],
                art_dir: dir.join("art"),
                mirror_dir: dir.join("hs"),
                pipeline: Default::default(),
                token: Some("api".into()),
                subsonic_user: "pubky".into(),
                pubky: Default::default(),
                public_url: "http://127.0.0.1:7788".into(),
                upnp: false,
                cast: false,
                slimproto_port: 0,
                session_secret_path: None,
                tidal: Default::default(),
                spotify: Default::default(),
                acoustid_key: String::new(),
                jamendo: Default::default(),
            },
            pm_library::Library::open(&db).unwrap(),
            pm_zones::OutputRegistry::new(),
        )
    }

    /// The hooks in accounts.rs and access_token.rs: each way an owner credential ends reaches
    /// the diagnostics.
    #[tokio::test]
    async fn every_way_an_owner_credential_ends_revokes() {
        let dir = tempfile::tempdir().unwrap();
        let s = test_node(dir.path());
        let d = &s.diagnostics;
        let t0 = Instant::now();
        let id = on(d, t0);
        let g = generation(d, t0);
        // Log out of one owner browser session: its diagnostics session ends
        let pmo = "pmo_browser";
        let stored = serde_json::json!([{"hash": blake3::hash(pmo.as_bytes()).to_hex().to_string(), "key": "k", "created_at": 1}]);
        s.library
            .lock()
            .unwrap()
            .set_setting("accounts.owner_sessions", &stored.to_string())
            .unwrap();
        let mine = d.start(d.epoch(), pmo, Viewport::Phone, g, t0).unwrap();
        let before = d.epoch();
        crate::accounts::end_owner_session(&s, pmo).unwrap();
        assert_eq!(d.ingest(before, &id, T, &[NAV], 0, t0, 0), Err(Refusal::Revoked));
        assert_eq!(put(d, &mine, pmo, &[NAV], t0), Err(Refusal::Gone));
        assert_eq!(put(d, &id, T, &[NAV], t0), Ok(()), "another browser's goes on");
        // Already ended: nothing more to revoke
        let before = d.epoch();
        assert!(!crate::accounts::end_owner_session(&s, pmo).unwrap());
        assert_eq!(d.epoch(), before);
        // Every owner browser session (a new token ends them all)
        let before = d.epoch();
        crate::accounts::end_owner_sessions(&s).unwrap();
        assert_ne!(d.epoch(), before);
        assert_eq!(d.held().1, 0);
        // The identity changed: the old one's sessions end
        let id = d.start(d.epoch(), T, Viewport::Phone, g, t0).unwrap();
        s.library
            .lock()
            .unwrap()
            .set_setting(
                "accounts.owner_sessions",
                r#"[{"hash":"00","key":"old","created_at":1}]"#,
            )
            .unwrap();
        let before = d.epoch();
        crate::accounts::keep_owner_sessions_of(&s, "new").unwrap();
        assert_ne!(d.epoch(), before);
        assert_eq!(put(d, &id, T, &[NAV], t0), Err(Refusal::Gone));
        // Kept as they were (nothing ended): no revocation
        let before = d.epoch();
        crate::accounts::keep_owner_sessions_of(&s, "new").unwrap();
        assert_eq!(d.epoch(), before);
    }

    #[test]
    fn at_most_32_sessions() {
        let d = Diagnostics::default();
        let t0 = Instant::now();
        on(&d, t0);
        let g = generation(&d, t0);
        for _ in 1..MAX_SESSIONS {
            d.start(d.epoch(), T, Viewport::Phone, g, t0).unwrap();
        }
        assert_eq!(
            d.start(d.epoch(), T, Viewport::Phone, g, t0),
            Err(Refusal::TooMany)
        );
    }

    #[test]
    fn rates_per_session_and_for_the_node_and_what_they_drop_is_counted() {
        let d = Diagnostics::default();
        let t0 = Instant::now();
        let id = on(&d, t0);
        let batch = [NAV; MAX_BATCH];
        for _ in 0..(PER_SESSION_PER_MIN as usize / MAX_BATCH) {
            put(&d, &id, T, &batch, t0).unwrap();
        }
        // 96 so far: 32 more is over 120
        assert_eq!(put(&d, &id, T, &batch, t0), Err(Refusal::RateLimited));
        assert_eq!(put(&d, &id, T, &batch[..24], t0), Ok(()));
        assert_eq!(put(&d, &id, T, &[NAV], t0), Err(Refusal::RateLimited));
        assert_eq!(d.status(d.epoch(), t0).unwrap().dropped.rate_limited, 33);
        // A minute later there is room again; what the browser dropped itself is counted
        assert_eq!(d.ingest(d.epoch(), &id, T, &batch, 7, t0 + MINUTE, 0), Ok(()));
        assert_eq!(d.status(d.epoch(), t0 + MINUTE).unwrap().dropped.by_browsers, 7);

        let small = Diagnostics::with_limits(Limits {
            global_per_min: 3,
            ..Limits::default()
        });
        let a = on(&small, t0);
        let g = generation(&small, t0);
        let b = small.start(small.epoch(), T, Viewport::Phone, g, t0).unwrap();
        put(&small, &a, T, &[NAV, NAV], t0).unwrap();
        assert_eq!(put(&small, &b, T, &[NAV, NAV], t0), Err(Refusal::RateLimited));
        assert_eq!(put(&small, &b, T, &[NAV], t0), Ok(()));
    }

    #[test]
    fn batches_are_small() {
        let d = Diagnostics::default();
        let t0 = Instant::now();
        let id = on(&d, t0);
        assert_eq!(put(&d, &id, T, &[NAV; 33], t0), Err(Refusal::TooLarge));
        assert_eq!(put(&d, "short", T, &[NAV], t0), Err(Refusal::BadSession));
        assert_eq!(
            put(&d, "0123456789ABCDEF", T, &[NAV], t0),
            Err(Refusal::BadSession)
        );
    }

    #[test]
    fn the_ring_keeps_72_hours_and_its_count_and_byte_caps() {
        let t0 = Instant::now();
        let d = Diagnostics::with_limits(Limits {
            per_session_per_min: u32::MAX,
            global_per_min: u32::MAX,
            max_events: 50,
            ..Limits::default()
        });
        let id = on(&d, t0);
        for _ in 0..4 {
            put(&d, &id, T, &[NAV; 32], t0).unwrap();
        }
        let st = d.status(d.epoch(), t0).unwrap();
        assert_eq!(st.events, 50, "count cap");
        assert_eq!(st.dropped.evicted, 78, "and what it pushed out is counted");
        let one = st.bytes / 50;

        let d = Diagnostics::with_limits(Limits {
            per_session_per_min: u32::MAX,
            global_per_min: u32::MAX,
            max_bytes: one * 10,
            ..Limits::default()
        });
        let id = on(&d, t0);
        put(&d, &id, T, &[NAV; 32], t0).unwrap();
        let st = d.status(d.epoch(), t0).unwrap();
        assert_eq!(st.events, 10, "byte cap");
        assert!(st.bytes <= one * 10);
        // The export really is about that size
        let out = d.export(d.epoch(), "v", t0, 0).unwrap();
        let events = serde_json::to_vec(&out["events"]).unwrap().len();
        assert!(events <= one * 10 + 2, "{events} > {}", one * 10);
        assert_eq!(out["manifest"]["dropped"]["evicted"], 22);

        // 72 h: the switch went off at 24 h, but what was kept stays until it is 72 h old
        let d = Diagnostics::default();
        let id = on(&d, t0);
        put(&d, &id, T, &[NAV], t0).unwrap();
        assert_eq!(
            d.status(d.epoch(), t0 + RETENTION - Duration::from_secs(1))
                .unwrap()
                .events,
            1
        );
        assert_eq!(d.status(d.epoch(), t0 + RETENTION).unwrap().events, 0);
    }

    /// With no one using diagnostics (no status, export or events), housekeeping still ends what
    /// ran out: here [`Diagnostics::housekeep`] alone, at the instants the task would look.
    #[test]
    fn idle_housekeeping_ends_the_switch_sessions_and_old_events() {
        let d = Diagnostics::default();
        let t0 = Instant::now();
        let id = on(&d, t0);
        put(&d, &id, T, &[NAV; 3], t0).unwrap();
        // and something dropped, counted for the same hours as the events
        d.ingest(d.epoch(), &id, T, &[NAV], 5, t0, 0).unwrap();
        assert_eq!(d.held(), (4, 1, true));
        assert_eq!(d.held_drop_hours(), 1);
        d.housekeep(t0 + SESSION_TTL);
        assert_eq!(d.held(), (4, 0, true), "the session ended after an hour");
        d.housekeep(t0 + GATE_TTL);
        assert_eq!(d.held(), (4, 0, false), "the switch went off after a day");
        d.housekeep(t0 + RETENTION - Duration::from_secs(1));
        assert_eq!(d.held().0, 4);
        assert_eq!(d.held_drop_hours(), 1);
        d.housekeep(t0 + RETENTION);
        assert_eq!(d.held(), (0, 0, false), "the events went after 72 h");
        assert_eq!(d.held_drop_hours(), 0, "and so did the count of what was dropped");
    }

    #[test]
    fn what_was_dropped_is_counted_per_hour_and_goes_with_the_events() {
        let d = Diagnostics::with_limits(Limits {
            per_session_per_min: u32::MAX,
            global_per_min: u32::MAX,
            ..Limits::default()
        });
        let t0 = Instant::now();
        let id = on(&d, t0);
        let at = |h: u64, m: u64| t0 + Duration::from_secs(h * 3600 + m * 60);
        d.ingest(d.epoch(), &id, T, &[NAV], 2, at(0, 0), 0).unwrap();
        d.ingest(d.epoch(), &id, T, &[NAV], 3, at(0, 59), 0).unwrap();
        assert_eq!(d.held_drop_hours(), 1, "one hour, one count");
        // The session lasts an hour: another for the next hours
        let g = generation(&d, at(0, 59));
        let id = d.start(d.epoch(), T, Viewport::Phone, g, at(0, 59)).unwrap();
        d.ingest(d.epoch(), &id, T, &[NAV], 4, at(1, 30), 0).unwrap();
        assert_eq!(d.held_drop_hours(), 2);
        let sum = |t| d.status(d.epoch(), t).unwrap().dropped.by_browsers;
        assert_eq!(sum(at(1, 30)), 9);
        // 72 h after the first hour began, its count goes (never later than its events would)
        assert_eq!(sum(at(72, 0)), 4);
        assert_eq!(sum(at(73, 30)), 0);
        assert_eq!(d.held_drop_hours(), 0);
    }

    /// The real background task: it ends events while nothing else touches the ring, and it ends
    /// itself once the diagnostics are gone (it holds them weakly).
    #[tokio::test]
    async fn the_housekeeping_task_runs_while_idle_and_ends_with_the_node() {
        let d = Arc::new(Diagnostics::with(
            Limits::default(),
            Ttl {
                gate: Duration::from_secs(3600),
                session: Duration::from_secs(3600),
                retention: Duration::from_millis(150),
            },
        ));
        let t0 = Instant::now();
        let id = on(&d, t0);
        put(&d, &id, T, &[NAV; 2], t0).unwrap();
        let task = d.spawn_housekeeping(Duration::from_millis(20)).unwrap();
        assert_eq!(d.held().0, 2);
        let mut gone = false;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if d.held().0 == 0 {
                gone = true;
                break;
            }
        }
        assert!(gone, "idle events were not ended");
        assert!(!task.is_finished());
        drop(d);
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the task ends with the diagnostics")
            .unwrap();
    }

    #[test]
    fn clear_is_not_off() {
        let d = Diagnostics::default();
        let t0 = Instant::now();
        let id = on(&d, t0);
        put(&d, &id, T, &[NAV], t0).unwrap();
        d.clear(d.epoch(), t0).unwrap();
        let st = d.status(d.epoch(), t0).unwrap();
        assert!(st.enabled && st.events == 0 && st.bytes == 0);
        assert_eq!(put(&d, &id, T, &[NAV], t0), Ok(()));
        // and Off keeps what was taken until Clear or 72 h
        d.set_enabled(d.epoch(), false, None, t0).unwrap();
        assert_eq!(d.status(d.epoch(), t0).unwrap().events, 1);
    }

    #[test]
    fn the_schema_is_closed() {
        for bad in [
            r#"{"kind":"nav","view":"home","x":1}"#,
            r#"{"kind":"nav","view":"https://example.com"}"#,
            r#"{"kind":"nav","view":"other"}"#,
            r#"{"kind":"api","area":"other","outcome":"ok","ms":"lt100"}"#,
            r#"{"kind":"api","area":"library","method":"get","outcome":"ok","ms":"lt300"}"#,
            r#"{"kind":"api","area":"library","outcome":"ok","ms":153}"#,
            r#"{"kind":"exception","class":"type_error","stack":"at foo.js:1"}"#,
            r#"{"kind":"exception","class":"unhandled_rejection"}"#,
            r#"{"kind":"log","line":"anything"}"#,
            r#"{"view":"home"}"#,
        ] {
            assert!(serde_json::from_str::<Event>(bad).is_err(), "{bad}");
        }
        assert!(serde_json::from_str::<Batch>(r#"{"session":"a","events":[],"at":1}"#).is_err());
        assert!(serde_json::from_str::<Batch>(r#"{"session":"a","events":[],"dropped":70000}"#).is_err());
        assert!(serde_json::from_str::<Start>(r#"{"viewport":"phone"}"#).is_err());
        let ok: Event =
            serde_json::from_str(r#"{"kind":"load","view":"add_music","outcome":"http_5xx","ms":"lt3000"}"#)
                .unwrap();
        assert_eq!(
            ok,
            Event::Load {
                view: View::AddMusic,
                outcome: Outcome::Http5xx,
                ms: Ms::Lt3000
            }
        );
    }

    #[test]
    fn listeners_and_guests_are_refused_by_path_as_well() {
        for path in [
            "/node/diagnostics",
            "/node/diagnostics/export",
            "/node/diagnostics/events",
            "/node/diagnostics/sessions/0123456789abcdef",
        ] {
            assert!(crate::party::guest_refusal(&axum::http::Method::GET, path).is_some());
            assert!(owner(&Caller {
                role: Role::Listener,
                key: Some("k".into())
            })
            .is_err());
            assert!(owner(&Caller {
                role: Role::Guest,
                key: None
            })
            .is_err());
        }
    }

    /// Minting a 21st owner browser session evicts the oldest: a credential ending like any
    /// other. A request let in with it (its epoch taken) changes nothing, its diagnostics session
    /// ends, and a write that fails after the eviction doesn't skip that.
    #[tokio::test]
    async fn evicting_the_oldest_owner_session_revokes_even_when_a_later_write_fails() {
        let dir = tempfile::tempdir().unwrap();
        let s = test_node(dir.path());
        let d = &s.diagnostics;
        let t0 = Instant::now();
        on(d, t0);
        let g = generation(d, t0);
        let e0 = d.epoch();
        let tokens: Vec<String> = (0..20)
            .map(|_| crate::accounts::mint_owner_if(&s, "k", || true).unwrap())
            .collect();
        assert_eq!(d.epoch(), e0, "up to 20, nothing ends");
        let oldest = d.start(d.epoch(), &tokens[0], Viewport::Phone, g, t0).unwrap();
        // Let in with the oldest credential and checked; the diagnostics lock not yet taken
        let before = d.epoch();
        crate::accounts::mint_owner_if(&s, "k", || true).unwrap();
        assert_eq!(
            d.ingest(before, &oldest, &tokens[0], &[NAV], 0, t0, 0),
            Err(Refusal::Revoked)
        );
        assert_eq!(put(d, &oldest, &tokens[0], &[NAV], t0), Err(Refusal::Gone));
        // The next eviction, whose write of the token mark fails afterwards: revoked all the same.
        // Its session starts after the first eviction, so it is live until this one
        let next = d.start(d.epoch(), &tokens[1], Viewport::Phone, g, t0).unwrap();
        assert_eq!(put(d, &next, &tokens[1], &[NAV], t0), Ok(()), "live before");
        rusqlite::Connection::open(dir.path().join("lib.sqlite"))
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_mark_i BEFORE INSERT ON settings WHEN NEW.key = 'accounts.owner_sessions_token'
                 BEGIN SELECT RAISE(ABORT, 'disk I/O error'); END;
                 CREATE TRIGGER fail_mark_u BEFORE UPDATE ON settings WHEN NEW.key = 'accounts.owner_sessions_token'
                 BEGIN SELECT RAISE(ABORT, 'disk I/O error'); END;",
            )
            .unwrap();
        let before = d.epoch();
        assert!(crate::accounts::mint_owner_if(&s, "k", || true).is_err());
        assert_ne!(d.epoch(), before, "the failed write did not skip the revocation");
        assert_eq!(
            d.ingest(before, &next, &tokens[1], &[NAV], 0, t0, 0),
            Err(Refusal::Revoked)
        );
        assert_eq!(put(d, &next, &tokens[1], &[NAV], t0), Err(Refusal::Gone));
        // The eviction itself was saved before the mark failed: that credential is gone for good
        let kept: Vec<crate::accounts::PersonToken> = serde_json::from_str(
            &s.library
                .lock()
                .unwrap()
                .get_setting("accounts.owner_sessions")
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        let hash = |t: &str| blake3::hash(t.as_bytes()).to_hex().to_string();
        assert_eq!(kept.len(), 20);
        assert!(!kept.iter().any(|k| k.hash == hash(&tokens[1])));
        assert!(kept.iter().any(|k| k.hash == hash(&tokens[2])));
    }

    /// Every explicit Off starts a new generation, also when already off, so an On shown before
    /// it is refused; the switch turning itself off (or staying off) doesn't keep counting.
    #[test]
    fn every_explicit_off_is_a_new_generation_expiry_is_not() {
        let d = Diagnostics::default();
        let t0 = Instant::now();
        let shown = generation(&d, t0);
        d.set_enabled(d.epoch(), false, None, t0).unwrap();
        assert_ne!(generation(&d, t0), shown);
        assert_eq!(
            d.set_enabled(d.epoch(), true, Some(shown), t0).unwrap_err(),
            Refusal::Stale
        );
        // Turned on, then off by itself after a day: one new generation, then no more
        let g = generation(&d, t0);
        d.set_enabled(d.epoch(), true, Some(g), t0).unwrap();
        let later = t0 + GATE_TTL;
        d.housekeep(later);
        let off = generation(&d, later);
        assert_eq!(off, g + 2);
        d.housekeep(later + HOUSEKEEPING);
        d.housekeep(later + 2 * HOUSEKEEPING);
        assert_eq!(generation(&d, later + 2 * HOUSEKEEPING), off);
    }

    /// A batch has events: an empty one (with or without a drop count) is refused, unrated
    #[test]
    fn an_empty_batch_is_refused() {
        let d = Diagnostics::default();
        let t0 = Instant::now();
        let id = on(&d, t0);
        assert_eq!(d.ingest(d.epoch(), &id, T, &[], 0, t0, 0), Err(Refusal::Empty));
        assert_eq!(d.ingest(d.epoch(), &id, T, &[], 500, t0, 0), Err(Refusal::Empty));
        assert_eq!(d.status(d.epoch(), t0).unwrap().dropped, Dropped::default());
    }

    /// A batch refused for the rates is counted once, as rate-limited; the browser then reports
    /// only its own earlier drops with the next batch, and the total is exact.
    #[test]
    fn a_rate_limited_batch_then_an_accepted_one_count_exactly() {
        let d = Diagnostics::default();
        let t0 = Instant::now();
        let id = on(&d, t0);
        for _ in 0..3 {
            put(&d, &id, T, &[NAV; MAX_BATCH], t0).unwrap();
        }
        // 96 kept; 32 more (with 3 the browser dropped earlier) is over 120
        assert_eq!(
            d.ingest(d.epoch(), &id, T, &[NAV; MAX_BATCH], 3, t0, 0),
            Err(Refusal::RateLimited)
        );
        // A minute later: the 3 again, with new events
        d.ingest(d.epoch(), &id, T, &[NAV; 2], 3, t0 + MINUTE, 0).unwrap();
        let dr = d.status(d.epoch(), t0 + MINUTE).unwrap().dropped;
        assert_eq!((dr.rate_limited, dr.by_browsers, dr.evicted), (32, 3, 0));
    }

    /// The node-wide rate window is usage too: idle housekeeping ends it after its minute
    #[test]
    fn idle_housekeeping_ends_the_rate_window() {
        let d = Diagnostics::default();
        let t0 = Instant::now();
        let id = on(&d, t0);
        put(&d, &id, T, &[NAV], t0).unwrap();
        assert!(d.lock().global.is_some());
        d.housekeep(t0 + MINUTE - Duration::from_secs(1));
        assert!(d.lock().global.is_some());
        d.housekeep(t0 + MINUTE);
        assert!(d.lock().global.is_none(), "the window went without an ingest");
    }

    /// A Pubky identity, wholly offline: a grant its own key signed, held by a client that reaches
    /// nothing (no DHT; one relay on the local discard port, never asked).
    fn identity() -> Arc<crate::sync::PubkySync> {
        use base64::Engine as _;
        use pubky::{Capability, ClientId, GrantClaims, GrantId, Keypair, GRANT_JWS_TYP};
        let client = pubky::PubkyHttpClient::builder()
            .pkarr(|p| {
                p.no_default_network()
                    .relays(&["http://127.0.0.1:9/"])
                    .expect("a relay url")
            })
            .build()
            .unwrap();
        let (user, pop) = (Keypair::random(), Keypair::random());
        let now = unix_now();
        let grant = GrantClaims {
            iss: user.public_key(),
            client_id: ClientId::new("node.pubky.music").unwrap(),
            caps: vec![Capability::root()],
            cnf: pop.public_key(),
            jti: GrantId::generate(),
            iat: now,
            exp: now + 3600,
        }
        .sign(&user, GRANT_JWS_TYP);
        let stored = format!(
            "pubky-grant-credential-v1:{}:{}:{grant}",
            Keypair::random().public_key().z32(),
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(pop.secret()),
        );
        let session = pubky::PubkySession::from_grant_credential(
            client.clone(),
            pubky::GrantCredential::from_shared_secret(&stored).unwrap(),
        );
        let hs = pm_pubky::homeserver::NodeHomeserver::from_pubky(pubky::Pubky::with_client(client));
        Arc::new(crate::sync::PubkySync::from_session(hs, session))
    }

    /// Writes of the setting `key` fail from now on (a full disk, say).
    fn fail_writes(dir: &std::path::Path, key: &str) {
        rusqlite::Connection::open(dir.join("lib.sqlite"))
            .unwrap()
            .execute_batch(&format!(
                "CREATE TRIGGER fail_i BEFORE INSERT ON settings WHEN NEW.key = '{key}'
                 BEGIN SELECT RAISE(ABORT, 'disk I/O error'); END;
                 CREATE TRIGGER fail_u BEFORE UPDATE ON settings WHEN NEW.key = '{key}'
                 BEGIN SELECT RAISE(ABORT, 'disk I/O error'); END;"
            ))
            .unwrap();
    }

    /// The node takes on another identity (`set_sync`: a log in as someone else) while a request
    /// is let in with the old one's browser session, its credential checked by the route's own
    /// check and the diagnostics lock not yet taken. It changes and exports nothing, and its
    /// diagnostics session ends: also when ending the old identity's sessions in the database
    /// fails, with no browser session at all, and on to no identity.
    #[tokio::test]
    async fn another_identity_revokes_what_the_old_one_let_in_even_when_its_cleanup_fails() {
        let dir = tempfile::tempdir().unwrap();
        let s = test_node(dir.path());
        let d = &s.diagnostics;
        let t0 = Instant::now();
        on(d, t0);
        let g = generation(d, t0);
        let a = identity();
        s.set_sync(Some(a.clone()));
        let pmo = crate::accounts::mint_owner(&s, a.z32()).unwrap();
        let mine = d.start(d.epoch(), &pmo, Viewport::Phone, g, t0).unwrap();
        assert_eq!(put(d, &mine, &pmo, &[NAV], t0), Ok(()), "live before");
        let me = Caller {
            role: Role::Owner,
            key: Some(a.z32().trim_start_matches("pubky").to_string()),
        };
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, format!("Bearer {pmo}").parse().unwrap());
        let (at, token) = let_in(&s, &me, &headers, &HashMap::new()).unwrap();
        assert_eq!(token, pmo);
        // Another identity, and ending the old one's sessions in the database fails
        fail_writes(dir.path(), "accounts.owner_sessions");
        let b = identity();
        s.set_sync(Some(b.clone()));
        assert!(s.sync_handle().is_some_and(|h| Arc::ptr_eq(&h, &b)));
        let kept = s
            .library
            .lock()
            .unwrap()
            .get_setting("accounts.owner_sessions")
            .unwrap()
            .unwrap();
        assert!(
            kept.contains(a.z32().trim_start_matches("pubky")),
            "the cleanup really failed"
        );
        assert!(
            crate::accounts::caller(&s, Some(&pmo)).is_none(),
            "a new request with it is refused"
        );
        // What was let in before lands nowhere: no events, switch, clearing or export
        assert_eq!(d.ingest(at, &mine, &pmo, &[NAV], 0, t0, 0), Err(Refusal::Revoked));
        assert_eq!(
            d.set_enabled(at, false, None, t0).map(|_| ()),
            Err(Refusal::Revoked)
        );
        assert_eq!(d.clear(at, t0), Err(Refusal::Revoked));
        assert_eq!(d.export(at, "0", t0, 0).map(|_| ()), Err(Refusal::Revoked));
        assert_eq!(put(d, &mine, &pmo, &[NAV], t0), Err(Refusal::Gone));
        assert_eq!(d.held(), (1, 0, true), "the event from before stays, still On");
        // No browser session in the database at all (the API token's diagnostics), then no
        // identity: revoked all the same
        let api = d.start(d.epoch(), "api", Viewport::Phone, g, t0).unwrap();
        let at = d.epoch();
        s.set_sync(Some(identity()));
        assert_eq!(d.export(at, "0", t0, 0).map(|_| ()), Err(Refusal::Revoked));
        assert_eq!(put(d, &api, "api", &[NAV], t0), Err(Refusal::Gone));
        let api = d.start(d.epoch(), "api", Viewport::Phone, g, t0).unwrap();
        let at = d.epoch();
        s.set_sync(None);
        assert_eq!(d.export(at, "0", t0, 0).map(|_| ()), Err(Refusal::Revoked));
        assert_eq!(put(d, &api, "api", &[NAV], t0), Err(Refusal::Gone));
    }

    /// The identity is never swapped under a diagnostics change in progress (one past its epoch
    /// check, holding the diagnostics lock): that change finishes under the identity it was let in
    /// with, and the swap, revoking, comes after it.
    #[tokio::test]
    async fn the_identity_never_changes_under_a_diagnostics_change_in_progress() {
        let dir = tempfile::tempdir().unwrap();
        let s = test_node(dir.path());
        let (a, b) = (identity(), identity());
        s.set_sync(Some(a.clone()));
        let in_progress = s.diagnostics.lock();
        let epoch = in_progress.epoch;
        let (asked, swapping) = std::sync::mpsc::channel();
        let swap = {
            let s = s.clone();
            std::thread::spawn(move || {
                asked.send(()).unwrap();
                s.set_sync(Some(b));
            })
        };
        swapping.recv().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            s.sync_handle().is_some_and(|h| Arc::ptr_eq(&h, &a)),
            "swapped while a diagnostics change was in progress"
        );
        drop(in_progress);
        swap.join().unwrap();
        assert!(!s.sync_handle().is_some_and(|h| Arc::ptr_eq(&h, &a)));
        assert_ne!(s.diagnostics.epoch(), Epoch(epoch));
    }
}
