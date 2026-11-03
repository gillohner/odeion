//! Usage insights (PLAN-node-wide-semantic-diagnostics.md): which pages and buttons people use on
//! this node, which they are shown and pass by, and where they run into trouble, in fixed
//! categories only. It is there to guide changes to the app. It never explains why someone
//! clicked, and a button that is seen and not used is not a judgement on it.
//!
//! - **Off until the owner turns it on** (`PUT /v1/node/usage`); the switch is kept on the node,
//!   so it stays as it was across restarts. Turning it on is the activation: from then on every
//!   signed-in owner and listener browser takes part automatically, with a notice, and each person
//!   can Stop for themselves (`PUT /v1/usage/me {"share": false}`) and Clear what is still linked to
//!   them (`POST /v1/usage/me/clear`). Stop and Clear are separate, as are the owner's Off and
//!   Clear all. Party guests are not covered yet (slice S6): the summary says so.
//! - **No identity in what is kept.** A browser page load gets a random alias, valid for at most
//!   [`ALIAS_MAX`] and [`ALIAS_IDLE`] without events. Which person and credential an alias belongs
//!   to is held in memory only, for the alias's life and [`BINDING_KEEP`] after (for Clear mine);
//!   a restart forgets it. The rows on disk carry the alias, the role (owner or listener) and the
//!   viewport class: no key, token, address or browser.
//! - **A closed schema.** Events are enums only: views, controls, outcomes, coarse durations. No
//!   text anyone typed, no names, titles, URLs, ids of songs or people, no messages or stacks.
//!   Unknown fields refuse the whole batch and refusals never echo the input.
//! - **Signed in for real, checked twice.** Listeners and the owner only, never on a node without an
//!   API token. The credential is checked when a request comes in and again once its body is here,
//!   and a change only lands if none of that person's credentials ended in between (or everyone's:
//!   another identity, a new token, a restore). Every way a credential ends calls
//!   [`Usage::revoke_person`] or [`Usage::revoke_all`] (accounts.rs, people.rs, backup.rs, state.rs).
//! - **Bounded.** Batches of [`MAX_BATCH`] and [`MAX_BODY`]; [`PER_ALIAS_PER_MIN`],
//!   [`PER_PERSON_PER_MIN`] and [`GLOBAL_PER_MIN`]; at most [`MAX_ALIASES`] live aliases; a queue of
//!   [`QUEUE_MAX`] in memory, written by one writer every [`FLUSH_EVERY`]. On disk
//!   (`<data>/usage.sqlite`, 0600, never in a backup): raw events [`RAW_RETENTION`], daily counts
//!   [`AGG_RETENTION`], at most [`MAX_EVENTS`] events and [`MAX_STORE_BYTES`]; over a cap the oldest
//!   day goes first. What was dropped on the way is counted.
//! - **Reading it.** The owner's summary (aggregates only) and export (raw events, renamed per
//!   export) are owner-authenticated; `pm-node usage summary|export` reads the file on the node.
//!   Nothing is sent anywhere by itself.

use crate::accounts::{Caller, Role};
use crate::state::NodeState;
use crate::{ApiError, ApiResult};
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The schema's name and version (export, summary, the client).
pub const SCHEMA: &str = "pm-usage/1";
/// The notice's wording version: a new wording is shown again.
pub const NOTICE_VERSION: u32 = 1;
/// The file, in the node's data folder.
pub const FILE: &str = "usage.sqlite";
/// An alias ends after this long without events…
pub const ALIAS_IDLE: Duration = Duration::from_secs(30 * 60);
/// …and after this long in any case.
pub const ALIAS_MAX: Duration = Duration::from_secs(12 * 3600);
/// Which person an ended alias belonged to is kept this long, in memory, for Clear mine.
pub const BINDING_KEEP: Duration = Duration::from_secs(24 * 3600);
/// At most this many ended aliases are remembered for Clear mine; past it the oldest are forgotten
/// first (their rows simply become unlinked sooner).
pub const ENDED_MAX: usize = 10_000;
pub const RAW_RETENTION_DAYS: u64 = 14;
pub const AGG_RETENTION_DAYS: u64 = 90;
/// Raw events are kept this long.
pub const RAW_RETENTION: Duration = Duration::from_secs(RAW_RETENTION_DAYS * 86_400);
/// Daily counts are kept this long.
pub const AGG_RETENTION: Duration = Duration::from_secs(AGG_RETENTION_DAYS * 86_400);
pub const MAX_EVENTS: u64 = 400_000;
/// Live data in the file (pages in use, not free ones).
pub const MAX_STORE_BYTES: u64 = 64 << 20;
pub const MAX_ALIASES: usize = 256;
/// Live aliases one person may have (their longest idle ends first), so nobody crowds out others.
pub const PER_PERSON_ALIASES: usize = 8;
/// Page loads one person may open a minute.
pub const OPENS_PER_PERSON_PER_MIN: u32 = 30;
/// Ended aliases remembered for Clear mine, per person (their oldest are forgotten first).
pub const PER_PERSON_ENDED: usize = 200;
/// The most a browser may report as dropped in one batch, and as unregistered clicks in one visit.
pub const MAX_REPORTED: u16 = 500;
pub const PER_ALIAS_PER_MIN: u32 = 120;
pub const PER_PERSON_PER_MIN: u32 = 240;
pub const GLOBAL_PER_MIN: u32 = 3000;
pub const MAX_BATCH: usize = 50;
/// The largest events request body.
pub const MAX_BODY: usize = 16 << 10;
/// The largest body of the other requests.
const MAX_SMALL_BODY: usize = 1 << 10;
/// Events waiting for the writer; more are dropped (and counted).
pub const QUEUE_MAX: usize = 10_000;
/// The writer writes this often, or as soon as [`FLUSH_AT`] events wait.
pub const FLUSH_EVERY: Duration = Duration::from_secs(5);
pub const FLUSH_AT: usize = 500;
/// How often what has run out in memory ends when no one sends anything.
pub const HOUSEKEEPING: Duration = Duration::from_secs(60);
/// How often old rows are deleted from the file.
pub const STORE_HOUSEKEEPING: Duration = Duration::from_secs(3600);
/// A control needs this many visits that showed it, from this many aliases, before its use rate
/// says anything (summary).
pub const MIN_EXPOSURES: u64 = 30;
pub const MIN_ALIASES: u64 = 5;
/// Most controls one visit reports as shown, and as used.
pub const MAX_EXPOSED: usize = 64;
pub const MAX_USED: usize = 32;
const MINUTE: Duration = Duration::from_secs(60);
const DAY: u64 = 86_400;

// ---- the schema: enums only ----

/// The app's views (app.js `renderView`), `-` written `_`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
    /// The Log in screen.
    Login,
}

/// What is on top of the view when something happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    Page,
    PlayerBar,
    NowPlaying,
    ZonePop,
    SoundPanel,
    Menu,
    Dialog,
    JobPop,
    Notice,
}

/// How a page was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum How {
    Push,
    Pop,
    Redirect,
    Initial,
}

/// A family of API requests (app.js routes list; a request not on it is not recorded).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
    Other,
}

/// Whether a request reads or changes something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    /// 401/403 from the node
    Refused,
    #[serde(rename = "http_4xx")]
    Http4xx,
    #[serde(rename = "http_5xx")]
    Http5xx,
    /// No answer
    Unreachable,
    /// The page's own code threw while drawing
    Failed,
}

/// How long, in coarse steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ms {
    Lt100,
    Lt300,
    Lt1000,
    Lt3000,
    Lt10000,
    Ge10000,
}

/// A script error's built-in class, nothing of its message or stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    TypeError,
    ReferenceError,
    RangeError,
    SyntaxError,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Viewport {
    Phone,
    Tablet,
    Desktop,
}

/// How a control was used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    Click,
    Key,
    Change,
    Submit,
    Swipe,
    LongPress,
}

/// The app's controls, by what they do (the client's registry maps its buttons onto these; a
/// button not on it is not recorded). Never which song, person, zone, tag or text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    // The player bar
    PlayerNowPlaying,
    PlayerShuffle,
    PlayerPrev,
    PlayerToggle,
    PlayerNext,
    PlayerRepeat,
    PlayerSeek,
    PlayerQuality,
    PlayerVolumeButton,
    PlayerVolume,
    PlayerZone,
    PlayerAlongLeave,
    JobChip,
    // The top bar and the navigation (where to: the event's `to`)
    SearchBox,
    SearchScope,
    SearchRetry,
    SidebarToggle,
    Account,
    ConnectionRetry,
    SideNav,
    BottomNav,
    LibraryTab,
    Back,
    Crumb,
    EmptyAction,
    ViewGo,
    // Song rows
    RowPlay,
    RowNext,
    RowAdd,
    RowMore,
    RowFavourite,
    RowAlbum,
    RowIdentify,
    RowSong,
    RowArtist,
    // An item page's header
    ItemPlay,
    ItemShuffle,
    ItemPlayOn,
    ItemFavourite,
    ItemMore,
    // Menus (app.js `showMenu` keys)
    MenuPlayNext,
    MenuAddQueue,
    MenuFavourite,
    MenuIdentify,
    MenuAddPlaylist,
    MenuStartRadio,
    MenuStreamFrom,
    MenuGoArtist,
    MenuGoAlbum,
    MenuSongPage,
    MenuCopyLink,
    MenuEditTags,
    MenuRemove,
    MenuCopyHome,
    MenuMoveZone,
    MenuPickPlaylist,
    MenuOther,
    // Selections
    SelectAll,
    SelectCancel,
    SelectPlay,
    SelectPlaylist,
    SelectQueue,
    SelectRemove,
    SelectTag,
    // Now playing
    NpShuffle,
    NpPrev,
    NpToggle,
    NpNext,
    NpRepeat,
    NpSeek,
    NpVolume,
    NpZone,
    NpQuality,
    NpClose,
    NpSheetQueue,
    NpSheetLyrics,
    NpSheetDetails,
    NpSheetSources,
    NpSheetTags,
    // Zones and Play to
    ZonePlayTo,
    ZoneMoveHere,
    ZoneSleep,
    ZoneVisibility,
    ZoneGroup,
    // Copy jobs
    JobCancel,
    JobDismiss,
    JobPause,
    JobResume,
    JobRetry,
    // Home
    HomeSeeAll,
    HomeWindow,
    HomeEdit,
    // Settings tabs
    SettingsAccounts,
    SettingsHome,
    SettingsQuality,
    SettingsAppearance,
    SettingsApps,
    SettingsPeople,
    SettingsPlayback,
    SettingsNodeHome,
    SettingsRemote,
    SettingsDiscover,
    SettingsBackup,
    SettingsAbout,
    SettingsHomeserver,
    ThemePick,
    LayoutPick,
    DenseToggle,
    // Signing in and out
    LoginRing,
    LoginQr,
    LoginAgain,
    LoginRetry,
    LoginToken,
    LogOut,
    LinkIdentity,
    // Elsewhere
    AddMusic,
    CopyLink,
    CopyKey,
    TryAgain,
    // Keyboard shortcuts
    KeySearch,
    KeyToggle,
    KeyPrev,
    KeyNext,
    KeyNowPlaying,
    KeyHelp,
    KeyEscape,
    // Usage insights itself
    UsageNoticeDismiss,
    UsageStop,
    UsageShare,
    UsageClear,
    /// A known kind of button with a value outside its list.
    Other,
}

/// What made something harder than it should be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Signal {
    ErrorToast,
    Retry,
    /// Three or more clicks on the same control within a second
    Rage,
    /// A click on something that does nothing
    DeadClick,
    /// Back within 3 s of arriving
    QuickBack,
    StillLoading,
    EmptyState,
    DisabledClick,
    StartFail,
}

/// An error's category: from its status class, never its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Refused,
    NotFound,
    Conflict,
    TooLarge,
    RateLimited,
    ClientOther,
    Server,
    Unreachable,
    Timeout,
    Other,
}

/// One event, as the browser sends it and as the export writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    Nav {
        view: View,
        how: How,
    },
    Load {
        view: View,
        outcome: Outcome,
        ms: Ms,
    },
    Api {
        area: Area,
        class: Class,
        outcome: Outcome,
        ms: Ms,
    },
    Action {
        control: Control,
        view: View,
        surface: Surface,
        via: Via,
        /// Where a navigation control went
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<View>,
    },
    /// One view or overlay from opening to leaving: the controls that were really shown (at
    /// least half on screen for a second, not covered, not disabled, the tab visible) and the
    /// ones used. Shown and not used is not a dislike.
    Visit {
        view: View,
        surface: Surface,
        ms: Ms,
        exposed: Vec<Control>,
        used: Vec<Control>,
        /// Clicks on buttons the registry does not know (counted, nothing else)
        #[serde(default)]
        unregistered: u16,
    },
    Friction {
        signal: Signal,
        view: View,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        control: Option<Control>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        category: Option<Category>,
    },
    Exception {
        class: ErrorClass,
    },
}

impl Event {
    fn kind(&self) -> &'static str {
        match self {
            Event::Nav { .. } => "nav",
            Event::Load { .. } => "load",
            Event::Api { .. } => "api",
            Event::Action { .. } => "action",
            Event::Visit { .. } => "visit",
            Event::Friction { .. } => "friction",
            Event::Exception { .. } => "exception",
        }
    }

    /// Within the schema's own bounds (list lengths, no repeats).
    fn bounded(&self) -> bool {
        match self {
            Event::Visit {
                exposed,
                used,
                unregistered,
                ..
            } => {
                *unregistered <= MAX_REPORTED
                    && exposed.len() <= MAX_EXPOSED
                    && used.len() <= MAX_USED
                    && exposed.iter().collect::<BTreeSet<_>>().len() == exposed.len()
                    && used.iter().collect::<BTreeSet<_>>().len() == used.len()
            }
            _ => true,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Batch {
    alias: String,
    events: Vec<Event>,
    /// Events this browser dropped since its last accepted batch.
    #[serde(default)]
    dropped: u16,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Gate {
    enabled: bool,
    /// The generation the page showed; needed to turn it on.
    #[serde(default)]
    generation: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Choice {
    share: bool,
    /// The choice generation the page showed (`Me::choice`); needed to share again.
    #[serde(default)]
    choice: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoticeSeen {
    version: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Open {
    viewport: Viewport,
    /// The node generation and this person's choice generation the page showed.
    generation: u64,
    choice: u64,
}

// ---- limits ----

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Limits {
    pub max_aliases: usize,
    pub per_person_aliases: usize,
    pub opens_per_person_per_min: u32,
    pub per_alias_per_min: u32,
    pub per_person_per_min: u32,
    pub global_per_min: u32,
    pub max_batch: usize,
    pub max_body_bytes: usize,
    pub queue_max: usize,
    pub max_events: u64,
    pub max_store_bytes: u64,
    pub raw_retention_days: u64,
    pub aggregate_retention_days: u64,
    pub alias_idle_minutes: u64,
    pub alias_max_hours: u64,
    pub min_exposures: u64,
    pub min_aliases: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_aliases: MAX_ALIASES,
            per_person_aliases: PER_PERSON_ALIASES,
            opens_per_person_per_min: OPENS_PER_PERSON_PER_MIN,
            per_alias_per_min: PER_ALIAS_PER_MIN,
            per_person_per_min: PER_PERSON_PER_MIN,
            global_per_min: GLOBAL_PER_MIN,
            max_batch: MAX_BATCH,
            max_body_bytes: MAX_BODY,
            queue_max: QUEUE_MAX,
            max_events: MAX_EVENTS,
            max_store_bytes: MAX_STORE_BYTES,
            raw_retention_days: RAW_RETENTION_DAYS,
            aggregate_retention_days: AGG_RETENTION_DAYS,
            alias_idle_minutes: ALIAS_IDLE.as_secs() / 60,
            alias_max_hours: ALIAS_MAX.as_secs() / 3600,
            min_exposures: MIN_EXPOSURES,
            min_aliases: MIN_ALIASES,
        }
    }
}

/// How long aliases last ([`ALIAS_IDLE`], [`ALIAS_MAX`], [`BINDING_KEEP`]; shorter in tests).
#[derive(Debug, Clone, Copy)]
struct Ttl {
    idle: Duration,
    max: Duration,
    keep: Duration,
}

impl Default for Ttl {
    fn default() -> Self {
        Self {
            idle: ALIAS_IDLE,
            max: ALIAS_MAX,
            keep: BINDING_KEEP,
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

// ---- people, aliases, epochs ----

/// Whose browser it is. The owner is one person whatever credential they use.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Person {
    Owner,
    /// A listener's key (z32, no `pubky`)
    Listener(String),
}

impl Person {
    fn of(c: &Caller) -> Option<Person> {
        match (c.role, &c.key) {
            (Role::Owner, _) => Some(Person::Owner),
            (Role::Listener, Some(k)) => Some(Person::Listener(k.trim_start_matches("pubky").to_string())),
            _ => None,
        }
    }
    fn role(&self) -> &'static str {
        match self {
            Person::Owner => "owner",
            Person::Listener(_) => "listener",
        }
    }
    /// The text a person's stored choices are keyed by (hashed with the file's own key).
    fn label(&self) -> String {
        match self {
            Person::Owner => "owner".into(),
            Person::Listener(k) => format!("listener:{k}"),
        }
    }
}

/// The authorization epochs when a request was let in: everyone's and the caller's own. Taken
/// before the body is read; if either changed meanwhile, the request changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Epoch {
    all: u64,
    person: u64,
}

struct Alias {
    /// A keyed hash of the credential it was opened with (memory only).
    credential: [u8; 32],
    person: Person,
    viewport: Viewport,
    node_generation: u64,
    choice: u64,
    opened: Instant,
    last: Instant,
    window: Window,
}

/// What a person chose (on disk, keyed by a hash of [`Person::label`]).
#[derive(Debug, Clone, Copy, Default)]
struct Pref {
    /// Stopped for themselves (sharing is the default).
    stopped: bool,
    /// Bumped by every Stop and every share again: a page that showed an older one is stale.
    choice: u64,
    notice_seen: u32,
}

/// A row on its way to the file.
#[derive(Clone)]
struct Row {
    unix: u64,
    alias: String,
    role: &'static str,
    viewport: Viewport,
    event: Event,
}

/// What did not make it, by reason (counted per day on disk).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dropped {
    pub evicted: u64,
    pub rate_limited: u64,
    pub queue_full: u64,
    pub by_browsers: u64,
}

impl Dropped {
    fn add(&mut self, o: Dropped) {
        self.evicted += o.evicted;
        self.rate_limited += o.rate_limited;
        self.queue_full += o.queue_full;
        self.by_browsers += o.by_browsers;
    }
    fn is_empty(&self) -> bool {
        *self == Dropped::default()
    }
}

#[derive(Default)]
struct Inner {
    /// Loaded from the file when it is first needed.
    loaded: bool,
    node_on: bool,
    /// Bumped by every On and Off.
    node_generation: u64,
    /// Bumped when every credential may have changed (a restore, another identity, a new token).
    all_epoch: u64,
    /// Bumped when one of a person's credentials ends.
    epochs: HashMap<Person, u64>,
    prefs: HashMap<Person, Pref>,
    aliases: HashMap<String, Alias>,
    /// Ended aliases → whose they were, for Clear mine (memory only).
    ended: HashMap<String, (Person, Instant)>,
    person_windows: HashMap<Person, Window>,
    /// Page loads opened per person, this minute.
    open_windows: HashMap<Person, Window>,
    global: Option<Window>,
    pending: Vec<Row>,
    pending_dropped: Dropped,
    /// Daily session counts waiting for the writer (role, viewport).
    pending_opens: Vec<(u64, &'static str, Viewport)>,
}

/// Why something was refused (each a fixed answer, never the input).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Off,
    /// Turned off or on, or Stop pressed, since the page showed it.
    Stale,
    /// A credential of this person (or everyone's) ended while this was on its way.
    Revoked,
    Gone,
    /// This person stopped usage insights for themselves.
    Stopped,
    TooMany,
    TooLarge,
    RateLimited,
    BadAlias,
    Empty,
    /// Outside the schema's bounds.
    Schema,
    /// The file could not be read or written.
    Storage,
}

impl From<Refusal> for ApiError {
    fn from(r: Refusal) -> Self {
        let (status, why) = match r {
            Refusal::Off => (StatusCode::CONFLICT, "usage insights are off on this node"),
            Refusal::Stale => (
                StatusCode::CONFLICT,
                "usage insights changed since this page showed them; look again",
            ),
            Refusal::Revoked => (
                StatusCode::CONFLICT,
                "a sign-in ended while this was sent; nothing was changed",
            ),
            Refusal::Gone => (StatusCode::GONE, "this usage alias has ended"),
            Refusal::Stopped => (StatusCode::CONFLICT, "you stopped usage insights for yourself"),
            Refusal::TooMany => (StatusCode::TOO_MANY_REQUESTS, "too many browsers at once"),
            Refusal::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "too many events in one batch"),
            Refusal::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "too many events; wait a minute"),
            Refusal::BadAlias => (StatusCode::BAD_REQUEST, "not a usage alias"),
            Refusal::Empty => (StatusCode::BAD_REQUEST, "a batch needs events"),
            Refusal::Schema => (StatusCode::BAD_REQUEST, "that is not in the usage schema"),
            Refusal::Storage => (
                StatusCode::SERVICE_UNAVAILABLE,
                "usage insights can't be stored right now",
            ),
        };
        ApiError::new(status, why)
    }
}

// ---- the file ----

/// `<data>/usage.sqlite`: the switch, people's choices (by hashed key), raw events and daily counts.
struct Store {
    conn: rusqlite::Connection,
    /// Keys the people's hashes, so they mean nothing outside this file.
    salt: [u8; 32],
}

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS pref (person BLOB PRIMARY KEY, stopped INTEGER NOT NULL, choice INTEGER NOT NULL, notice_seen INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS event (seq INTEGER PRIMARY KEY AUTOINCREMENT, day INTEGER NOT NULL, at INTEGER NOT NULL, alias TEXT NOT NULL, role TEXT NOT NULL, viewport TEXT NOT NULL, payload TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS event_day ON event(day);
CREATE INDEX IF NOT EXISTS event_alias ON event(alias);
CREATE TABLE IF NOT EXISTS daily (day INTEGER NOT NULL, metric TEXT NOT NULL, key TEXT NOT NULL, n INTEGER NOT NULL, PRIMARY KEY (day, metric, key));
";

fn storage<E>(_: E) -> Refusal {
    Refusal::Storage
}

impl Store {
    /// Opens the file, making it (0600) when `create` and it is not there yet.
    fn open(path: &FsPath, create: bool) -> Result<Option<Store>, Refusal> {
        if !path.exists() {
            if !create {
                return Ok(None);
            }
            let mut o = std::fs::OpenOptions::new();
            o.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                o.mode(0o600);
            }
            match o.open(path) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(Refusal::Storage),
            }
        }
        let conn = rusqlite::Connection::open(path).map_err(storage)?;
        conn.busy_timeout(Duration::from_secs(5)).map_err(storage)?;
        conn.execute_batch(SCHEMA_SQL).map_err(storage)?;
        let salt = match get_meta(&conn, "salt")? {
            Some(hex) => decode32(&hex).ok_or(Refusal::Storage)?,
            None => {
                let salt: [u8; 32] = rand::random();
                set_meta(&conn, "salt", &hex::encode(salt))?;
                set_meta(&conn, "schema", SCHEMA)?;
                salt
            }
        };
        Ok(Some(Store { conn, salt }))
    }

    /// Opens the file read-only (the `pm-node usage` commands).
    fn open_read_only(path: &FsPath) -> Result<Option<Store>, Refusal> {
        if !path.exists() {
            return Ok(None);
        }
        let conn = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(storage)?;
        let salt = get_meta(&conn, "salt")?
            .and_then(|h| decode32(&h))
            .unwrap_or([0; 32]);
        Ok(Some(Store { conn, salt }))
    }

    fn person_id(&self, p: &Person) -> Vec<u8> {
        blake3::keyed_hash(&self.salt, p.label().as_bytes())
            .as_bytes()
            .to_vec()
    }

    fn pref(&self, p: &Person) -> Result<Pref, Refusal> {
        use rusqlite::OptionalExtension;
        self.conn
            .query_row(
                "SELECT stopped, choice, notice_seen FROM pref WHERE person = ?1",
                [self.person_id(p)],
                |r| {
                    Ok(Pref {
                        stopped: r.get::<_, i64>(0)? != 0,
                        choice: r.get::<_, i64>(1)? as u64,
                        notice_seen: r.get::<_, i64>(2)? as u32,
                    })
                },
            )
            .optional()
            .map(Option::unwrap_or_default)
            .map_err(storage)
    }

    fn set_pref(&self, p: &Person, pref: Pref) -> Result<(), Refusal> {
        self.conn
            .execute(
                "INSERT INTO pref (person, stopped, choice, notice_seen) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(person) DO UPDATE SET stopped = ?2, choice = ?3, notice_seen = ?4",
                rusqlite::params![
                    self.person_id(p),
                    pref.stopped as i64,
                    pref.choice as i64,
                    pref.notice_seen as i64
                ],
            )
            .map(|_| ())
            .map_err(storage)
    }

    fn forget_pref(&self, p: &Person) -> Result<(), Refusal> {
        self.conn
            .execute("DELETE FROM pref WHERE person = ?1", [self.person_id(p)])
            .map(|_| ())
            .map_err(storage)
    }

    fn write(
        &mut self,
        rows: &[Row],
        opens: &[(u64, &'static str, Viewport)],
        dropped: Dropped,
    ) -> Result<(), Refusal> {
        let tx = self.conn.transaction().map_err(storage)?;
        {
            let mut ins = tx
                .prepare_cached(
                    "INSERT INTO event (day, at, alias, role, viewport, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .map_err(storage)?;
            let mut bump = tx
                .prepare_cached(
                    "INSERT INTO daily (day, metric, key, n) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(day, metric, key) DO UPDATE SET n = n + ?4",
                )
                .map_err(storage)?;
            for r in rows {
                let day = r.unix / DAY;
                let payload = serde_json::to_string(&r.event).map_err(storage)?;
                // To the minute only: finer times would make events easier to match to a person
                ins.execute(rusqlite::params![
                    day as i64,
                    (r.unix / 60 * 60) as i64,
                    r.alias,
                    r.role,
                    name(&r.viewport),
                    payload
                ])
                .map_err(storage)?;
                for (metric, key) in daily_keys(&r.event) {
                    bump.execute(rusqlite::params![day as i64, metric, key, 1i64])
                        .map_err(storage)?;
                }
            }
            for (unix, role, viewport) in opens {
                bump.execute(rusqlite::params![
                    (unix / DAY) as i64,
                    "alias",
                    format!("{role}.{}", name(viewport)),
                    1i64
                ])
                .map_err(storage)?;
            }
            let today = (rows.first().map(|r| r.unix).unwrap_or_else(unix_now) / DAY) as i64;
            for (why, n) in [
                ("evicted", dropped.evicted),
                ("rate_limited", dropped.rate_limited),
                ("queue_full", dropped.queue_full),
                ("by_browsers", dropped.by_browsers),
            ] {
                if n > 0 {
                    bump.execute(rusqlite::params![today, "dropped", why, n as i64])
                        .map_err(storage)?;
                }
            }
        }
        tx.commit().map_err(storage)
    }

    /// Deletes what is past its retention, then the oldest days while over a cap; how many raw
    /// events went for a cap is counted as evicted.
    fn housekeep(&mut self, unix: u64, limits: &Limits) -> Result<u64, Refusal> {
        let today = unix / DAY;
        let raw_from = today.saturating_sub(limits.raw_retention_days.saturating_sub(1));
        let agg_from = today.saturating_sub(limits.aggregate_retention_days.saturating_sub(1));
        self.conn
            .execute("DELETE FROM event WHERE day < ?1", [raw_from as i64])
            .map_err(storage)?;
        self.conn
            .execute("DELETE FROM daily WHERE day < ?1", [agg_from as i64])
            .map_err(storage)?;
        let mut evicted = 0u64;
        loop {
            let (count, bytes) = self.size()?;
            if count <= limits.max_events && bytes <= limits.max_store_bytes {
                break;
            }
            let oldest: Option<i64> = self
                .conn
                .query_row("SELECT MIN(day) FROM event", [], |r| r.get(0))
                .map_err(storage)?;
            let Some(oldest) = oldest else { break };
            let newest: i64 = self
                .conn
                .query_row("SELECT MAX(day) FROM event", [], |r| r.get(0))
                .map_err(storage)?;
            let gone = if oldest < newest {
                self.conn
                    .execute("DELETE FROM event WHERE day = ?1", [oldest])
                    .map_err(storage)?
            } else {
                // One day left: its oldest tenth
                let n = (count / 10).max(1) as i64;
                self.conn
                    .execute(
                        "DELETE FROM event WHERE seq IN (SELECT seq FROM event ORDER BY seq LIMIT ?1)",
                        [n],
                    )
                    .map_err(storage)?
            };
            if gone == 0 {
                break;
            }
            evicted += gone as u64;
        }
        if evicted > 0 {
            self.conn
                .execute(
                    "INSERT INTO daily (day, metric, key, n) VALUES (?1, 'dropped', 'evicted', ?2)
                     ON CONFLICT(day, metric, key) DO UPDATE SET n = n + ?2",
                    rusqlite::params![today as i64, evicted as i64],
                )
                .map_err(storage)?;
        }
        // Give space back when most of the file is free
        let (page_count, free): (i64, i64) = (
            self.conn
                .query_row("PRAGMA page_count", [], |r| r.get(0))
                .map_err(storage)?,
            self.conn
                .query_row("PRAGMA freelist_count", [], |r| r.get(0))
                .map_err(storage)?,
        );
        if page_count > 0 && free * 4 > page_count {
            self.conn.execute_batch("VACUUM").map_err(storage)?;
        }
        Ok(evicted)
    }

    /// Raw events kept and the bytes of the file's pages in use.
    fn size(&self) -> Result<(u64, u64), Refusal> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM event", [], |r| r.get(0))
            .map_err(storage)?;
        let pages: i64 = self
            .conn
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .map_err(storage)?;
        let free: i64 = self
            .conn
            .query_row("PRAGMA freelist_count", [], |r| r.get(0))
            .map_err(storage)?;
        let size: i64 = self
            .conn
            .query_row("PRAGMA page_size", [], |r| r.get(0))
            .map_err(storage)?;
        Ok((count as u64, ((pages - free).max(0) * size) as u64))
    }

    fn clear_aliases(&self, aliases: &[String]) -> Result<u64, Refusal> {
        let mut n = 0;
        for a in aliases {
            n += self
                .conn
                .execute("DELETE FROM event WHERE alias = ?1", [a])
                .map_err(storage)? as u64;
        }
        Ok(n)
    }

    fn clear_all(&self) -> Result<(), Refusal> {
        self.conn
            .execute_batch("DELETE FROM event; DELETE FROM daily;")
            .map_err(storage)
    }

    fn rows(&self) -> Result<Vec<StoredRow>, Refusal> {
        let mut st = self
            .conn
            .prepare("SELECT at, alias, role, viewport, payload FROM event ORDER BY seq")
            .map_err(storage)?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)? as u64,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(storage)?;
        let mut out = Vec::new();
        for r in rows {
            let (at, alias, role, viewport, payload) = r.map_err(storage)?;
            // Rows are written from the schema; one that does not read back is skipped
            if let Ok(e) = serde_json::from_str::<Event>(&payload) {
                out.push((at, alias, role, viewport, e));
            }
        }
        Ok(out)
    }

    fn daily(&self) -> Result<Vec<(u64, String, String, u64)>, Refusal> {
        let mut st = self
            .conn
            .prepare("SELECT day, metric, key, n FROM daily ORDER BY day, metric, key")
            .map_err(storage)?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)? as u64,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)? as u64,
                ))
            })
            .map_err(storage)?;
        rows.collect::<Result<_, _>>().map_err(storage)
    }
}

/// A stored event: when (unix seconds, to the minute), alias, role, viewport, the event.
type StoredRow = (u64, String, String, String, Event);

fn get_meta(conn: &rusqlite::Connection, k: &str) -> Result<Option<String>, Refusal> {
    use rusqlite::OptionalExtension;
    conn.query_row("SELECT v FROM meta WHERE k = ?1", [k], |r| r.get(0))
        .optional()
        .map_err(storage)
}

fn set_meta(conn: &rusqlite::Connection, k: &str, v: &str) -> Result<(), Refusal> {
    conn.execute(
        "INSERT INTO meta (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = ?2",
        [k, v],
    )
    .map(|_| ())
    .map_err(storage)
}

fn decode32(h: &str) -> Option<[u8; 32]> {
    hex::decode(h).ok()?.try_into().ok()
}

/// The daily counts an event adds to (metric, key); keys are schema names only.
fn daily_keys(e: &Event) -> Vec<(&'static str, String)> {
    match e {
        Event::Nav { view, .. } => vec![("nav", name(view))],
        Event::Load { view, outcome, .. } => vec![("load", format!("{}.{}", name(view), name(outcome)))],
        Event::Api {
            area, class, outcome, ..
        } => {
            vec![("api", format!("{}.{}.{}", name(area), name(class), name(outcome)))]
        }
        Event::Action { control, .. } => vec![("action", name(control))],
        Event::Visit {
            view,
            surface,
            exposed,
            used,
            ..
        } => {
            let mut v = vec![("visit", format!("{}.{}", name(view), name(surface)))];
            v.extend(exposed.iter().map(|c| ("exposed", name(c))));
            v.extend(used.iter().map(|c| ("used", name(c))));
            v
        }
        Event::Friction { signal, view, .. } => {
            vec![("friction", format!("{}.{}", name(signal), name(view)))]
        }
        Event::Exception { class } => vec![("exception", name(class))],
    }
}

// ---- the node's usage insights ----

/// The switch, the live aliases, the writer's queue and the file.
pub struct Usage {
    inner: Mutex<Inner>,
    /// Taken by the writer and by Clear, so a write in progress never brings back cleared rows.
    /// Lock order: `writing`, then `inner`, then `store`; `inner` is never held while taking
    /// `writing`, and the identity lock is only ever taken inside `inner` ([`Self::revoke_all_across`]).
    writing: Mutex<()>,
    store: Mutex<Option<Store>>,
    path: Option<PathBuf>,
    limits: Limits,
    ttl: Ttl,
    /// Keys the credential hashes (memory only).
    key: [u8; 32],
    wake: tokio::sync::Notify,
    /// Tests: every use of the file fails.
    #[cfg(test)]
    fail_store: std::sync::atomic::AtomicBool,
}

/// What a person sees about usage insights (`GET /v1/usage/me`).
#[derive(Debug, Serialize)]
pub struct Me {
    pub schema: &'static str,
    /// Whether the last change was saved in the file (it applies in memory either way).
    pub saved: bool,
    /// The node's switch.
    pub node_on: bool,
    pub generation: u64,
    /// This person shares (the node's switch decides whether anything is taken).
    pub sharing: bool,
    pub choice: u64,
    /// The notice wording this person has seen (0 = none); [`NOTICE_VERSION`] is current.
    pub notice_seen: u32,
    pub notice_version: u32,
    pub role: &'static str,
    pub limits: Limits,
}

/// The owner's view (`GET /v1/node/usage`).
#[derive(Debug, Serialize)]
pub struct Status {
    pub enabled: bool,
    /// Whether the last change was saved in the file (it applies in memory either way).
    pub saved: bool,
    pub generation: u64,
    pub aliases: usize,
    pub waiting: usize,
    pub events: u64,
    pub bytes: u64,
    pub coverage: Coverage,
    pub limits: Limits,
}

/// Who is covered (honest about what is not yet).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Coverage {
    pub owner: &'static str,
    pub listeners: &'static str,
    pub guests: &'static str,
}

pub const COVERAGE: Coverage = Coverage {
    owner: "instrumented",
    listeners: "instrumented",
    guests: "not instrumented",
};

impl Usage {
    /// The node's usage insights, its file in `data_dir` (not made until something is stored).
    pub fn new(data_dir: &FsPath) -> Self {
        Self::with(Some(data_dir.join(FILE)), Limits::default(), Ttl::default())
    }

    /// With the background tasks: the writer every [`FLUSH_EVERY`], housekeeping every
    /// [`HOUSEKEEPING`] and [`STORE_HOUSEKEEPING`]. They hold a weak reference and end with the node.
    pub fn housekept(data_dir: &FsPath) -> Arc<Self> {
        let u = Arc::new(Self::new(data_dir));
        u.spawn_tasks();
        u
    }

    fn with(path: Option<PathBuf>, limits: Limits, ttl: Ttl) -> Self {
        Self {
            inner: Mutex::default(),
            writing: Mutex::new(()),
            store: Mutex::new(None),
            path,
            limits,
            ttl,
            key: rand::random(),
            wake: tokio::sync::Notify::new(),
            #[cfg(test)]
            fail_store: Default::default(),
        }
    }

    /// Starts the writer and housekeeping (only inside a tokio runtime).
    pub fn spawn_tasks(self: &Arc<Self>) -> Option<tokio::task::JoinHandle<()>> {
        let rt = tokio::runtime::Handle::try_current().ok()?;
        let weak = Arc::downgrade(self);
        Some(rt.spawn(async move {
            let mut flush = tokio::time::interval(FLUSH_EVERY);
            flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut memory = tokio::time::interval(HOUSEKEEPING);
            memory.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut disk = tokio::time::interval(STORE_HOUSEKEEPING);
            disk.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                let woken = {
                    // Held only while waiting (at most one flush interval), so the task still
                    // ends soon after the node does
                    let Some(u) = weak.upgrade() else { break };
                    tokio::select! {
                        _ = flush.tick() => 0,
                        _ = memory.tick() => 1,
                        _ = disk.tick() => 2,
                        _ = u.wake.notified() => 0,
                    }
                };
                let Some(u) = weak.upgrade() else { break };
                let now = Instant::now();
                let unix = unix_now();
                match woken {
                    1 => u.housekeep(now),
                    2 => {
                        let _ = u.flush(unix);
                        let _ = u.housekeep_store(unix);
                    }
                    _ => {
                        let _ = u.flush(unix);
                    }
                }
            }
        }))
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Option<Store>> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn credential_hash(&self, token: &str) -> [u8; 32] {
        *blake3::keyed_hash(&self.key, token.as_bytes()).as_bytes()
    }

    /// The file, opened (and made, when `create`); `None` when there is none yet and not `create`.
    fn with_store<R>(
        &self,
        create: bool,
        f: impl FnOnce(&mut Store) -> Result<R, Refusal>,
    ) -> Result<Option<R>, Refusal> {
        #[cfg(test)]
        if self.fail_store.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(Refusal::Storage);
        }
        let mut st = self.store();
        if st.is_none() {
            let Some(path) = &self.path else {
                return Ok(None);
            };
            *st = Store::open(path, create)?;
        }
        match st.as_mut() {
            Some(s) => f(s).map(Some),
            None => Ok(None),
        }
    }

    /// The switch from the file, the first time it is needed.
    fn load(&self, inner: &mut Inner) -> Result<(), Refusal> {
        if inner.loaded {
            return Ok(());
        }
        let meta = self.with_store(false, |s| {
            Ok((
                get_meta(&s.conn, "on")?.as_deref() == Some("1"),
                get_meta(&s.conn, "generation")?
                    .and_then(|g| g.parse::<u64>().ok())
                    .unwrap_or(0),
            ))
        })?;
        if let Some((on, generation)) = meta {
            inner.node_on = on;
            inner.node_generation = generation;
        }
        inner.loaded = true;
        Ok(())
    }

    fn pref_of(&self, inner: &mut Inner, p: &Person) -> Result<Pref, Refusal> {
        if let Some(pref) = inner.prefs.get(p) {
            return Ok(*pref);
        }
        let pref = self.with_store(false, |s| s.pref(p))?.unwrap_or_default();
        inner.prefs.insert(p.clone(), pref);
        Ok(pref)
    }

    fn save_pref(&self, inner: &mut Inner, p: &Person, pref: Pref) -> Result<(), Refusal> {
        self.with_store(true, |s| s.set_pref(p, pref))?;
        inner.prefs.insert(p.clone(), pref);
        Ok(())
    }

    // ---- epochs and revocation ----

    /// The epochs now, for `p`: take them before reading a request's body.
    pub fn epoch(&self, p: &Person) -> Epoch {
        let inner = self.lock();
        Epoch {
            all: inner.all_epoch,
            person: inner.epochs.get(p).copied().unwrap_or(0),
        }
    }

    /// One of `p`'s credentials ended (Log out, removed, music access off, the oldest of their
    /// sessions replaced): their aliases end and nothing they sent before now changes anything.
    pub fn revoke_person(&self, p: &Person) {
        let mut inner = self.lock();
        *inner.epochs.entry(p.clone()).or_default() += 1;
        Self::end_aliases(&mut inner, |a| &a.person == p, Instant::now());
    }

    /// A listener's credential ended ([`Self::revoke_person`]).
    pub fn revoke_listener(&self, key: &str) {
        self.revoke_person(&Person::Listener(key.trim_start_matches("pubky").to_string()));
    }

    /// One of the owner's credentials ended ([`Self::revoke_person`]).
    pub fn revoke_owner(&self) {
        self.revoke_person(&Person::Owner);
    }

    /// Every credential may have changed (a restore): every alias ends, and nothing let in before
    /// now changes anything.
    pub fn revoke_all(&self) {
        let mut inner = self.lock();
        inner.all_epoch += 1;
        Self::end_aliases(&mut inner, |_| true, Instant::now());
    }

    /// The node takes on another identity (or none): [`Self::revoke_all`], with `swap` putting it
    /// in place under the same hold (lock order: usage, then the identity; `swap` takes no lock
    /// that is ever held while this one is taken).
    pub(crate) fn revoke_all_across<R>(&self, swap: impl FnOnce() -> R) -> R {
        let mut inner = self.lock();
        inner.all_epoch += 1;
        Self::end_aliases(&mut inner, |_| true, Instant::now());
        swap()
    }

    /// A listener removed from the node: their aliases end, what is still linked to them goes
    /// (as Clear mine), and their stored choice goes.
    pub fn forget_listener(&self, key: &str) {
        let p = Person::Listener(key.trim_start_matches("pubky").to_string());
        let _w = self.writing.lock().unwrap_or_else(|e| e.into_inner());
        let theirs: Vec<String> = {
            let mut inner = self.lock();
            *inner.epochs.entry(p.clone()).or_default() += 1;
            Self::end_aliases(&mut inner, |a| a.person == p, Instant::now());
            let theirs: BTreeSet<String> = inner
                .ended
                .iter()
                .filter(|(_, (q, _))| q == &p)
                .map(|(id, _)| id.clone())
                .collect();
            inner.pending.retain(|r| !theirs.contains(&r.alias));
            inner.ended.retain(|_, (q, _)| q != &p);
            inner.prefs.remove(&p);
            theirs.into_iter().collect()
        };
        let _ = self.with_store(false, |s| {
            s.clear_aliases(&theirs)?;
            s.forget_pref(&p)
        });
    }

    fn end_aliases(inner: &mut Inner, which: impl Fn(&Alias) -> bool, now: Instant) {
        let ending: Vec<String> = inner
            .aliases
            .iter()
            .filter(|(_, a)| which(a))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ending {
            if let Some(a) = inner.aliases.remove(&id) {
                inner.ended.insert(id, (a.person, now));
            }
        }
        Self::bound_ended(inner);
    }

    /// Forgets ended aliases past [`PER_PERSON_ENDED`] for one person and [`ENDED_MAX`] in all,
    /// always from whoever has the most (their oldest first): one person opening page after page
    /// only ever pushes out their own.
    fn bound_ended(inner: &mut Inner) {
        loop {
            let mut per: HashMap<&Person, usize> = HashMap::new();
            for (q, _) in inner.ended.values() {
                *per.entry(q).or_default() += 1;
            }
            let Some((most, n)) = per.into_iter().max_by_key(|(_, n)| *n) else {
                break;
            };
            if n <= PER_PERSON_ENDED && inner.ended.len() <= ENDED_MAX {
                break;
            }
            let most = most.clone();
            let oldest = inner
                .ended
                .iter()
                .filter(|(_, (q, _))| *q == most)
                .min_by_key(|(_, (_, at))| *at)
                .map(|(id, _)| id.clone());
            match oldest {
                Some(id) => {
                    inner.ended.remove(&id);
                }
                None => break,
            }
        }
    }

    /// The lock with what has run out ended, if no credential of `p` (or everyone's) ended since `at`.
    fn guard(
        &self,
        at: Epoch,
        p: &Person,
        now: Instant,
    ) -> Result<std::sync::MutexGuard<'_, Inner>, Refusal> {
        let mut inner = self.lock();
        if inner.all_epoch != at.all || inner.epochs.get(p).copied().unwrap_or(0) != at.person {
            return Err(Refusal::Revoked);
        }
        self.expire(&mut inner, now);
        self.load(&mut inner)?;
        Ok(inner)
    }

    /// [`Self::guard`], except that for an Off or a Stop (`always`) a file that can't be read
    /// does not stop it from applying.
    fn guard_off(
        &self,
        at: Epoch,
        p: &Person,
        now: Instant,
        always: bool,
    ) -> Result<std::sync::MutexGuard<'_, Inner>, Refusal> {
        let mut inner = self.lock();
        if inner.all_epoch != at.all || inner.epochs.get(p).copied().unwrap_or(0) != at.person {
            return Err(Refusal::Revoked);
        }
        self.expire(&mut inner, now);
        match self.load(&mut inner) {
            Ok(()) => Ok(inner),
            Err(_) if always => Ok(inner),
            Err(e) => Err(e),
        }
    }

    /// Ends aliases past their time, forgets old bindings, ends rate windows.
    fn expire(&self, inner: &mut Inner, now: Instant) {
        let ttl = self.ttl;
        Self::end_aliases(
            inner,
            |a| now.duration_since(a.last) >= ttl.idle || now.duration_since(a.opened) >= ttl.max,
            now,
        );
        inner
            .ended
            .retain(|_, (_, at)| now.duration_since(*at) < ttl.keep);
        inner
            .person_windows
            .retain(|_, w| now.duration_since(w.start) < MINUTE);
        inner
            .open_windows
            .retain(|_, w| now.duration_since(w.start) < MINUTE);
        if inner
            .global
            .is_some_and(|w| now.duration_since(w.start) >= MINUTE)
        {
            inner.global = None;
        }
    }

    /// What the memory housekeeping does.
    pub fn housekeep(&self, now: Instant) {
        let mut inner = self.lock();
        self.expire(&mut inner, now);
    }

    // ---- the switch and people's choices ----

    fn status_of(&self, inner: &Inner, saved: bool) -> Result<Status, Refusal> {
        // The file's size is informative only: a status never fails for it
        let (events, bytes) = self
            .with_store(false, |s| s.size())
            .ok()
            .flatten()
            .unwrap_or((0, 0));
        Ok(Status {
            enabled: inner.node_on,
            saved,
            generation: inner.node_generation,
            aliases: inner.aliases.len(),
            waiting: inner.pending.len(),
            events,
            bytes,
            coverage: COVERAGE,
            limits: self.limits,
        })
    }

    pub fn status(&self, at: Epoch, now: Instant) -> Result<Status, Refusal> {
        let inner = self.guard(at, &Person::Owner, now)?;
        self.status_of(&inner, true)
    }

    /// The owner's switch. Off always applies at once (in memory, then saved; `saved` says whether
    /// that worked), ends every alias and keeps what is stored. On applies only if `shown` is the
    /// generation now, and only once saved.
    pub fn set_enabled(
        &self,
        at: Epoch,
        on: bool,
        shown: Option<u64>,
        now: Instant,
    ) -> Result<Status, Refusal> {
        let mut inner = self.guard_off(at, &Person::Owner, now, !on)?;
        let mut saved = true;
        if !on {
            let generation = inner.node_generation + 1;
            let was_on = inner.node_on;
            inner.node_on = false;
            inner.node_generation = generation;
            Self::end_aliases(&mut inner, |_| true, now);
            saved = self
                .with_store(was_on, |s| {
                    set_meta(&s.conn, "on", "0")?;
                    set_meta(&s.conn, "generation", &generation.to_string())
                })
                .is_ok();
        } else if shown != Some(inner.node_generation) {
            return Err(Refusal::Stale);
        } else if !inner.node_on {
            let generation = inner.node_generation + 1;
            self.with_store(true, |s| {
                set_meta(&s.conn, "on", "1")?;
                set_meta(&s.conn, "generation", &generation.to_string())?;
                set_meta(&s.conn, "enabled_at", &unix_now().to_string())
            })?;
            inner.node_on = true;
            inner.node_generation = generation;
        }
        self.status_of(&inner, saved)
    }

    pub fn me(&self, at: Epoch, p: &Person, now: Instant) -> Result<Me, Refusal> {
        let mut inner = self.guard(at, p, now)?;
        let pref = self.pref_of(&mut inner, p)?;
        Ok(self.me_of(&inner, p, pref, true))
    }

    fn me_of(&self, inner: &Inner, p: &Person, pref: Pref, saved: bool) -> Me {
        Me {
            schema: SCHEMA,
            saved,
            node_on: inner.node_on,
            generation: inner.node_generation,
            sharing: !pref.stopped,
            choice: pref.choice,
            notice_seen: pref.notice_seen,
            notice_version: NOTICE_VERSION,
            role: p.role(),
            limits: self.limits,
        }
    }

    /// A person's own choice. Stop always applies at once, also when the file can't be written
    /// (`saved` says whether it was): their aliases end and nothing they sent and the writer has
    /// not written yet is written (the writer is held meanwhile). Sharing again applies only if
    /// `shown` is their choice generation now.
    pub fn choose(
        &self,
        at: Epoch,
        p: &Person,
        share: bool,
        shown: Option<u64>,
        now: Instant,
    ) -> Result<Me, Refusal> {
        let _w = self.writing.lock().unwrap_or_else(|e| e.into_inner());
        let mut inner = self.guard_off(at, p, now, !share)?;
        let mut pref = match self.pref_of(&mut inner, p) {
            Ok(pref) => pref,
            // Stop never waits for the file
            Err(_) if !share => inner.prefs.get(p).copied().unwrap_or_default(),
            Err(e) => return Err(e),
        };
        let mut saved = true;
        if !share {
            pref.stopped = true;
            pref.choice += 1;
            inner.prefs.insert(p.clone(), pref);
            Self::end_aliases(&mut inner, |a| &a.person == p, now);
            let ended: BTreeSet<String> = inner
                .ended
                .iter()
                .filter(|(_, (q, _))| q == p)
                .map(|(id, _)| id.clone())
                .collect();
            // What their browsers sent and the writer has not written yet is not written
            inner.pending.retain(|r| !ended.contains(&r.alias));
            saved = self.save_pref(&mut inner, p, pref).is_ok();
        } else if shown != Some(pref.choice) {
            return Err(Refusal::Stale);
        } else if pref.stopped {
            pref.stopped = false;
            pref.choice += 1;
            // Sharing again only once it is saved
            self.save_pref(&mut inner, p, pref)?;
        }
        Ok(self.me_of(&inner, p, pref, saved))
    }

    /// The person has read this wording of the notice.
    pub fn notice_seen(&self, at: Epoch, p: &Person, version: u32, now: Instant) -> Result<Me, Refusal> {
        if version == 0 || version > NOTICE_VERSION {
            return Err(Refusal::Schema);
        }
        let mut inner = self.guard(at, p, now)?;
        let mut pref = self.pref_of(&mut inner, p)?;
        if pref.notice_seen < version {
            pref.notice_seen = version;
            self.save_pref(&mut inner, p, pref)?;
        }
        Ok(self.me_of(&inner, p, pref, true))
    }

    // ---- aliases and events ----

    /// A browser page load of `p` takes part: a fresh random alias, bound in memory to its
    /// credential, the node generation and the person's choice the page showed.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        &self,
        at: Epoch,
        p: &Person,
        token: &str,
        viewport: Viewport,
        generation: u64,
        choice: u64,
        now: Instant,
        unix: u64,
    ) -> Result<String, Refusal> {
        let credential = self.credential_hash(token);
        let mut inner = self.guard(at, p, now)?;
        if !inner.node_on {
            return Err(Refusal::Off);
        }
        if inner.node_generation != generation {
            return Err(Refusal::Stale);
        }
        let pref = self.pref_of(&mut inner, p)?;
        if pref.stopped {
            return Err(Refusal::Stopped);
        }
        if pref.choice != choice {
            return Err(Refusal::Stale);
        }
        let opens = inner
            .open_windows
            .entry(p.clone())
            .or_insert_with(|| Window::new(now));
        if !opens.room(now, 1, self.limits.opens_per_person_per_min) {
            return Err(Refusal::RateLimited);
        }
        opens.count += 1;
        // This person's own longest idle alias ends first when they have too many
        let theirs: Vec<(String, Instant)> = inner
            .aliases
            .iter()
            .filter(|(_, a)| &a.person == p)
            .map(|(id, a)| (id.clone(), a.last))
            .collect();
        if theirs.len() >= self.limits.per_person_aliases {
            if let Some((id, _)) = theirs.into_iter().min_by_key(|(_, last)| *last) {
                if let Some(a) = inner.aliases.remove(&id) {
                    inner.ended.insert(id, (a.person, now));
                }
                Self::bound_ended(&mut inner);
            }
        }
        if inner.aliases.len() >= self.limits.max_aliases {
            // The longest idle goes first
            let oldest = inner
                .aliases
                .iter()
                .min_by_key(|(_, a)| a.last)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                if let Some(a) = inner.aliases.remove(&id) {
                    inner.ended.insert(id, (a.person, now));
                }
                Self::bound_ended(&mut inner);
            } else {
                return Err(Refusal::TooMany);
            }
        }
        let id = format!("{:016x}", rand::random::<u64>());
        let node_generation = inner.node_generation;
        inner.aliases.insert(
            id.clone(),
            Alias {
                credential,
                person: p.clone(),
                viewport,
                node_generation,
                choice,
                opened: now,
                last: now,
                window: Window::new(now),
            },
        );
        inner.pending_opens.push((unix, p.role(), viewport));
        Ok(id)
    }

    /// A browser ends its own alias (page leave); only with the credential it was opened with.
    pub fn close(&self, at: Epoch, p: &Person, id: &str, token: &str, now: Instant) -> Result<(), Refusal> {
        let credential = self.credential_hash(token);
        let mut inner = self.guard(at, p, now)?;
        if inner
            .aliases
            .get(id)
            .is_some_and(|a| a.credential == credential && &a.person == p)
        {
            if let Some(a) = inner.aliases.remove(id) {
                inner.ended.insert(id.to_string(), (a.person, now));
            }
            Self::bound_ended(&mut inner);
        }
        Ok(())
    }

    /// Queues a batch from a live alias of this person, credential, generation and choice.
    #[allow(clippy::too_many_arguments)]
    pub fn ingest(
        &self,
        at: Epoch,
        p: &Person,
        id: &str,
        token: &str,
        events: &[Event],
        dropped: u16,
        now: Instant,
        unix: u64,
    ) -> Result<(), Refusal> {
        if !valid_alias(id) {
            return Err(Refusal::BadAlias);
        }
        if events.is_empty() {
            return Err(Refusal::Empty);
        }
        if events.len() > self.limits.max_batch {
            return Err(Refusal::TooLarge);
        }
        if !events.iter().all(Event::bounded) {
            return Err(Refusal::Schema);
        }
        let credential = self.credential_hash(token);
        let limits = self.limits;
        let mut guard = self.guard(at, p, now)?;
        let inner = &mut *guard;
        if !inner.node_on {
            return Err(Refusal::Off);
        }
        let node_generation = inner.node_generation;
        let pref = match inner.prefs.get(p) {
            Some(pref) => *pref,
            None => return Err(Refusal::Gone),
        };
        let Some(alias) = inner.aliases.get_mut(id).filter(|a| {
            a.credential == credential
                && &a.person == p
                && a.node_generation == node_generation
                && a.choice == pref.choice
                && !pref.stopped
        }) else {
            return Err(Refusal::Gone);
        };
        let n = events.len() as u32;
        let person = inner
            .person_windows
            .entry(p.clone())
            .or_insert_with(|| Window::new(now));
        let global = inner.global.get_or_insert_with(|| Window::new(now));
        if !alias.window.room(now, n, limits.per_alias_per_min)
            || !person.room(now, n, limits.per_person_per_min)
            || !global.room(now, n, limits.global_per_min)
        {
            inner.pending_dropped.add(Dropped {
                rate_limited: u64::from(n),
                ..Dropped::default()
            });
            return Err(Refusal::RateLimited);
        }
        alias.window.count += n;
        person.count += n;
        global.count += n;
        alias.last = now;
        let (viewport, role) = (alias.viewport, p.role());
        let mut lost = Dropped {
            by_browsers: u64::from(dropped.min(MAX_REPORTED)),
            ..Dropped::default()
        };
        for e in events {
            if inner.pending.len() >= limits.queue_max {
                lost.queue_full += 1;
                continue;
            }
            inner.pending.push(Row {
                unix,
                alias: id.to_string(),
                role,
                viewport,
                event: e.clone(),
            });
        }
        inner.pending_dropped.add(lost);
        if inner.pending.len() >= FLUSH_AT {
            self.wake.notify_one();
        }
        Ok(())
    }

    /// Writes what waits (the writer task; tests call it directly).
    pub fn flush(&self, unix: u64) -> Result<usize, Refusal> {
        let _w = self.writing.lock().unwrap_or_else(|e| e.into_inner());
        let (rows, opens, dropped) = {
            let mut inner = self.lock();
            (
                std::mem::take(&mut inner.pending),
                std::mem::take(&mut inner.pending_opens),
                std::mem::take(&mut inner.pending_dropped),
            )
        };
        if rows.is_empty() && opens.is_empty() && dropped.is_empty() {
            return Ok(0);
        }
        let n = rows.len();
        let written = self.with_store(true, |s| {
            s.write(&rows, &opens, dropped)?;
            let (count, bytes) = s.size()?;
            if count > self.limits.max_events || bytes > self.limits.max_store_bytes {
                s.housekeep(unix, &self.limits)?;
            }
            Ok(())
        });
        match written {
            Ok(_) => Ok(n),
            Err(e) => {
                // Not written: counted, never kept waiting without bound
                let mut inner = self.lock();
                inner.pending_dropped.add(Dropped {
                    queue_full: n as u64,
                    ..Dropped::default()
                });
                Err(e)
            }
        }
    }

    /// Deletes stored rows past retention and over the caps (hourly).
    pub fn housekeep_store(&self, unix: u64) -> Result<u64, Refusal> {
        let _w = self.writing.lock().unwrap_or_else(|e| e.into_inner());
        Ok(self
            .with_store(false, |s| s.housekeep(unix, &self.limits))?
            .unwrap_or(0))
    }

    /// Clear mine: what is still linked to `p` (their live aliases and those ended within
    /// [`BINDING_KEEP`]) goes, waiting and stored. Older rows are no longer linked to anyone.
    pub fn clear_mine(&self, at: Epoch, p: &Person, now: Instant) -> Result<u64, Refusal> {
        let _w = self.writing.lock().unwrap_or_else(|e| e.into_inner());
        let mine: Vec<String> = {
            let mut inner = self.guard(at, p, now)?;
            let mut mine: BTreeSet<String> = inner
                .aliases
                .iter()
                .filter(|(_, a)| &a.person == p)
                .map(|(id, _)| id.clone())
                .collect();
            mine.extend(
                inner
                    .ended
                    .iter()
                    .filter(|(_, (q, _))| q == p)
                    .map(|(id, _)| id.clone()),
            );
            inner.pending.retain(|r| !mine.contains(&r.alias));
            mine.into_iter().collect()
        };
        Ok(self.with_store(false, |s| s.clear_aliases(&mine))?.unwrap_or(0))
    }

    /// Clear all (the owner): waiting and stored events and daily counts; the switch stays.
    pub fn clear_all(&self, at: Epoch, now: Instant) -> Result<(), Refusal> {
        let _w = self.writing.lock().unwrap_or_else(|e| e.into_inner());
        {
            let mut inner = self.guard(at, &Person::Owner, now)?;
            inner.pending.clear();
            inner.pending_opens.clear();
            inner.pending_dropped = Dropped::default();
        }
        self.with_store(false, |s| s.clear_all())?;
        Ok(())
    }

    /// The owner's summary (aggregates only), checked under the owner's epoch.
    pub fn summary(
        &self,
        at: Epoch,
        app_version: &str,
        now: Instant,
        unix: u64,
    ) -> Result<serde_json::Value, Refusal> {
        drop(self.guard(at, &Person::Owner, now)?);
        let _ = self.flush(unix);
        let out = {
            let st = self.store();
            match st.as_ref() {
                Some(s) => summarize(s, &self.limits, app_version, unix)?,
                None => empty_summary(&self.limits, app_version, unix),
            }
        };
        // Not delivered if an owner credential ended while it was made
        drop(self.guard(at, &Person::Owner, Instant::now())?);
        Ok(out)
    }

    /// The owner's export: raw events renamed per export, with the summary.
    pub fn export(
        &self,
        at: Epoch,
        app_version: &str,
        now: Instant,
        unix: u64,
    ) -> Result<serde_json::Value, Refusal> {
        drop(self.guard(at, &Person::Owner, now)?);
        let _ = self.flush(unix);
        let out = {
            let st = self.store();
            match st.as_ref() {
                Some(s) => export_of(s, &self.limits, app_version, unix)?,
                None => empty_summary(&self.limits, app_version, unix),
            }
        };
        drop(self.guard(at, &Person::Owner, Instant::now())?);
        Ok(out)
    }

    /// Whether the switch is on (before a body is read).
    fn is_on(&self) -> bool {
        let mut inner = self.lock();
        self.load(&mut inner).is_ok() && inner.node_on
    }

    #[cfg(test)]
    fn held(&self) -> (usize, usize, usize) {
        let inner = self.lock();
        (inner.aliases.len(), inner.ended.len(), inner.pending.len())
    }
}

fn valid_alias(id: &str) -> bool {
    id.len() == 16
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// An enum's name in the schema.
fn name<T: Serialize>(t: &T) -> String {
    serde_json::to_value(t)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

// ---- summary and export ----

fn manifest(limits: &Limits, app_version: &str, unix: u64, contents: &str) -> serde_json::Value {
    serde_json::json!({
        "schema": SCHEMA,
        "app_version": app_version,
        "made_at": unix,
        "limits": limits,
        "coverage": COVERAGE,
        "contents": contents,
        "reading": "Shown and not used is not a dislike. A use rate is only given with at least min_exposures visits from min_aliases aliases; below that it says insufficient. Friction points at places to look at, not conclusions.",
    })
}

fn empty_summary(limits: &Limits, app_version: &str, unix: u64) -> serde_json::Value {
    serde_json::json!({
        "manifest": manifest(limits, app_version, unix, "Nothing stored."),
        "totals": {},
    })
}

/// Aggregates only: counts per view, control, signal and outcome, with how many aliases and
/// visits each one rests on. No aliases, no rows.
fn summarize(s: &Store, limits: &Limits, app_version: &str, unix: u64) -> Result<serde_json::Value, Refusal> {
    let rows = s.rows()?;
    let mut kinds: BTreeMap<&str, u64> = BTreeMap::new();
    let mut roles: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut views: BTreeMap<String, u64> = BTreeMap::new();
    let mut loads: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    let mut api: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    let mut durations: BTreeMap<String, u64> = BTreeMap::new();
    let mut actions: BTreeMap<String, (u64, BTreeSet<String>)> = BTreeMap::new();
    let mut friction: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    let mut exceptions: BTreeMap<String, u64> = BTreeMap::new();
    // control → (visits that showed it, visits that used it, aliases shown to)
    let mut exposure: BTreeMap<String, (u64, u64, BTreeSet<String>)> = BTreeMap::new();
    for (_, alias, role, _, e) in &rows {
        *kinds.entry(e.kind()).or_default() += 1;
        roles.entry(role.clone()).or_default().insert(alias.clone());
        match e {
            Event::Nav { view, .. } => *views.entry(name(view)).or_default() += 1,
            Event::Load { view, outcome, ms } => {
                *loads
                    .entry(name(view))
                    .or_default()
                    .entry(name(outcome))
                    .or_default() += 1;
                *durations.entry(name(ms)).or_default() += 1;
            }
            Event::Api {
                area,
                class,
                outcome,
                ms,
            } => {
                *api.entry(format!("{}.{}", name(area), name(class)))
                    .or_default()
                    .entry(name(outcome))
                    .or_default() += 1;
                *durations.entry(name(ms)).or_default() += 1;
            }
            Event::Action { control, .. } => {
                let a = actions.entry(name(control)).or_default();
                a.0 += 1;
                a.1.insert(alias.clone());
            }
            Event::Visit { exposed, used, .. } => {
                for c in exposed {
                    let x = exposure.entry(name(c)).or_default();
                    x.0 += 1;
                    x.2.insert(alias.clone());
                    if used.contains(c) {
                        x.1 += 1;
                    }
                }
            }
            Event::Friction { signal, view, .. } => {
                *friction
                    .entry(name(signal))
                    .or_default()
                    .entry(name(view))
                    .or_default() += 1;
            }
            Event::Exception { class } => *exceptions.entry(name(class)).or_default() += 1,
        }
    }
    let exposure: BTreeMap<String, serde_json::Value> = exposure
        .into_iter()
        .map(|(c, (shown, used, aliases))| {
            let aliases = aliases.len() as u64;
            let enough = shown >= limits.min_exposures && aliases >= limits.min_aliases;
            (
                c,
                serde_json::json!({
                    "visits_shown": shown,
                    "visits_used": used,
                    "aliases_shown": aliases,
                    "use_rate": if enough { serde_json::json!((used as f64) / (shown as f64)) } else { serde_json::Value::Null },
                    "evidence": if enough { "enough" } else { "insufficient" },
                }),
            )
        })
        .collect();
    let actions: BTreeMap<String, serde_json::Value> = actions
        .into_iter()
        .map(|(c, (n, aliases))| (c, serde_json::json!({"uses": n, "aliases": aliases.len()})))
        .collect();
    let aliases: BTreeMap<String, usize> = roles.into_iter().map(|(r, a)| (r, a.len())).collect();
    let mut daily: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    let mut dropped = Dropped::default();
    for (_, metric, key, n) in s.daily()? {
        *daily
            .entry(metric.clone())
            .or_default()
            .entry(key.clone())
            .or_default() += n;
        if metric == "dropped" {
            match key.as_str() {
                "evicted" => dropped.evicted += n,
                "rate_limited" => dropped.rate_limited += n,
                "queue_full" => dropped.queue_full += n,
                "by_browsers" => dropped.by_browsers += n,
                _ => {}
            }
        }
    }
    Ok(serde_json::json!({
        "manifest": manifest(limits, app_version, unix, "Counts of pages, controls, outcomes and friction from the last raw_retention_days, and daily counts from the last aggregate_retention_days. No aliases, people, text, names or addresses. Not a complete record: see dropped."),
        "window_days": limits.raw_retention_days,
        "totals": kinds,
        "aliases_by_role": aliases,
        "views": views,
        "loads": loads,
        "api": api,
        "durations": durations,
        "actions": actions,
        "exposure": exposure,
        "friction": friction,
        "exceptions": exceptions,
        "dropped": dropped,
        "daily_totals": daily,
    }))
}

/// Raw events, each alias renamed `s1`, `s2`… for this export only, with the summary.
fn export_of(s: &Store, limits: &Limits, app_version: &str, unix: u64) -> Result<serde_json::Value, Refusal> {
    let summary = summarize(s, limits, app_version, unix)?;
    let mut names: HashMap<String, String> = HashMap::new();
    let mut sessions: BTreeMap<String, (String, String, u64)> = BTreeMap::new();
    let mut events = Vec::new();
    for (at, alias, role, viewport, e) in s.rows()? {
        let next = names.len() + 1;
        let n = names.entry(alias).or_insert_with(|| format!("s{next}")).clone();
        sessions
            .entry(n.clone())
            .or_insert_with(|| (role.clone(), viewport.clone(), 0))
            .2 += 1;
        let mut v = serde_json::to_value(&e).map_err(storage)?;
        if let Some(o) = v.as_object_mut() {
            o.insert("at".into(), serde_json::json!(at));
            o.insert("session".into(), serde_json::json!(n));
        }
        events.push(v);
    }
    let sessions: Vec<_> = sessions
        .into_iter()
        .map(|(id, (role, viewport, n))| serde_json::json!({"id": id, "role": role, "viewport": viewport, "events": n}))
        .collect();
    Ok(serde_json::json!({
        "manifest": manifest(limits, app_version, unix, "Raw events of the last raw_retention_days with per-export session names, and the summary. No people, credentials, text, names or addresses."),
        "summary": summary,
        "sessions": sessions,
        "events": events,
    }))
}

/// The summary from a node's data folder, read-only (`pm-node usage summary`).
pub fn summary_of_folder(data_dir: &FsPath, app_version: &str) -> Result<serde_json::Value, String> {
    let limits = Limits::default();
    match Store::open_read_only(&data_dir.join(FILE)).map_err(|_| "can't read the usage file".to_string())? {
        Some(s) => {
            summarize(&s, &limits, app_version, unix_now()).map_err(|_| "can't read the usage file".into())
        }
        None => Ok(empty_summary(&limits, app_version, unix_now())),
    }
}

/// The export from a node's data folder, read-only (`pm-node usage export`).
pub fn export_of_folder(data_dir: &FsPath, app_version: &str) -> Result<serde_json::Value, String> {
    let limits = Limits::default();
    match Store::open_read_only(&data_dir.join(FILE)).map_err(|_| "can't read the usage file".to_string())? {
        Some(s) => {
            export_of(&s, &limits, app_version, unix_now()).map_err(|_| "can't read the usage file".into())
        }
        None => Ok(empty_summary(&limits, app_version, unix_now())),
    }
}

// ---- routes ----

/// The person this request's own credential belongs to, checked now (not the middleware's answer
/// from when the request came in). Never on a node without an API token.
fn credential(
    s: &NodeState,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> Result<(Person, String), ApiError> {
    if !matches!(s.token(), Some(t) if !t.is_empty()) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "usage insights need the node's API token: without one there is no sign-in to check",
        ));
    }
    let unknown = || ApiError::new(StatusCode::UNAUTHORIZED, "missing or wrong bearer token");
    let token = crate::bearer(headers, query)
        .filter(|t| !t.is_empty())
        .ok_or_else(unknown)?;
    match crate::accounts::caller(s, Some(&token)) {
        Some(c) => match Person::of(&c) {
            Some(p) => Ok((p, token)),
            None => Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "usage insights are not for party guests yet",
            )),
        },
        None => Err(unknown()),
    }
}

/// The caller, checked again here; the epochs first, so a revocation from now on is seen.
fn let_in(
    s: &NodeState,
    c: &Caller,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> Result<(Person, Epoch, String), ApiError> {
    let p = Person::of(c).ok_or_else(|| {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "usage insights are not for party guests yet",
        )
    })?;
    let at = s.usage.epoch(&p);
    let (again, token) = credential(s, headers, query)?;
    if again != p {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "missing or wrong bearer token",
        ));
    }
    Ok((p, at, token))
}

/// The same person, once a body has arrived.
fn still(
    s: &NodeState,
    p: &Person,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> Result<String, ApiError> {
    let (again, token) = credential(s, headers, query)?;
    if &again != p {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "missing or wrong bearer token",
        ));
    }
    Ok(token)
}

fn owner_in(
    s: &NodeState,
    c: &Caller,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> Result<Epoch, ApiError> {
    if c.role != Role::Owner {
        return Err(crate::accounts::refused("usage insights are run by the owner"));
    }
    let (p, at, _) = let_in(s, c, headers, query)?;
    if p != Person::Owner {
        return Err(crate::accounts::refused("usage insights are run by the owner"));
    }
    Ok(at)
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
                "that is more than usage insights take in one request",
            ));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// Reads a body into the schema; a refusal never quotes it.
fn parse<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|_| Refusal::Schema.into())
}

type Token = Query<HashMap<String, String>>;

/// `GET /v1/usage/me`
pub(crate) async fn me(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
    Query(query): Token,
) -> ApiResult<Me> {
    let (p, at, _) = let_in(&s, &c, &headers, &query)?;
    Ok(Json(s.usage.me(at, &p, Instant::now())?))
}

/// `PUT /v1/usage/me {"share": bool, "choice": n}`: Stop for me, or share again.
pub(crate) async fn choose(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
    Query(query): Token,
    body: Body,
) -> ApiResult<Me> {
    let (p, at, _) = let_in(&s, &c, &headers, &query)?;
    let body = read(body, MAX_SMALL_BODY).await?;
    still(&s, &p, &headers, &query)?;
    let ch: Choice = parse(&body)?;
    Ok(Json(s.usage.choose(
        at,
        &p,
        ch.share,
        ch.choice,
        Instant::now(),
    )?))
}

/// `POST /v1/usage/me/notice {"version": n}`
pub(crate) async fn notice(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
    Query(query): Token,
    body: Body,
) -> ApiResult<Me> {
    let (p, at, _) = let_in(&s, &c, &headers, &query)?;
    let body = read(body, MAX_SMALL_BODY).await?;
    still(&s, &p, &headers, &query)?;
    let n: NoticeSeen = parse(&body)?;
    Ok(Json(s.usage.notice_seen(at, &p, n.version, Instant::now())?))
}

/// `POST /v1/usage/me/clear`: Clear mine.
pub(crate) async fn clear_mine(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
    Query(query): Token,
) -> ApiResult<serde_json::Value> {
    let (p, at, _) = let_in(&s, &c, &headers, &query)?;
    let gone = s.usage.clear_mine(at, &p, Instant::now())?;
    Ok(Json(serde_json::json!({ "cleared": gone })))
}

/// `POST /v1/usage/sessions {"viewport", "generation", "choice"}`: this page load takes part.
pub(crate) async fn open(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
    Query(query): Token,
    body: Body,
) -> ApiResult<serde_json::Value> {
    let (p, at, _) = let_in(&s, &c, &headers, &query)?;
    let body = read(body, MAX_SMALL_BODY).await?;
    let token = still(&s, &p, &headers, &query)?;
    let o: Open = parse(&body)?;
    let id = s.usage.open(
        at,
        &p,
        &token,
        o.viewport,
        o.generation,
        o.choice,
        Instant::now(),
        unix_now(),
    )?;
    let l = s.usage.limits();
    Ok(Json(serde_json::json!({
        "alias": id,
        "idle_s": s.usage.ttl.idle.as_secs(),
        "expires_in_s": s.usage.ttl.max.as_secs(),
        "max_batch": l.max_batch,
    })))
}

/// `DELETE /v1/usage/sessions/{alias}`: this page load is leaving.
pub(crate) async fn close(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
    Query(query): Token,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let (p, at, token) = let_in(&s, &c, &headers, &query)?;
    if !valid_alias(&id) {
        return Err(Refusal::BadAlias.into());
    }
    s.usage.close(at, &p, &id, &token, Instant::now())?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/usage/events {"alias", "events": […], "dropped": n}`
pub(crate) async fn ingest(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
    Query(query): Token,
    body: Body,
) -> Result<StatusCode, ApiError> {
    let (p, at, _) = let_in(&s, &c, &headers, &query)?;
    // Off is checked before the body is read
    if !s.usage.is_on() {
        return Err(Refusal::Off.into());
    }
    let body = read(body, s.usage.limits().max_body_bytes).await?;
    let token = still(&s, &p, &headers, &query)?;
    let batch: Batch = parse(&body)?;
    s.usage.ingest(
        at,
        &p,
        &batch.alias,
        &token,
        &batch.events,
        batch.dropped,
        Instant::now(),
        unix_now(),
    )?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /v1/node/usage`
pub(crate) async fn status(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
    Query(query): Token,
) -> ApiResult<Status> {
    let at = owner_in(&s, &c, &headers, &query)?;
    Ok(Json(s.usage.status(at, Instant::now())?))
}

/// `PUT /v1/node/usage {"enabled": bool, "generation": n}`: the owner's switch.
pub(crate) async fn set_gate(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
    Query(query): Token,
    body: Body,
) -> ApiResult<Status> {
    let at = owner_in(&s, &c, &headers, &query)?;
    let body = read(body, MAX_SMALL_BODY).await?;
    still(&s, &Person::Owner, &headers, &query)?;
    let g: Gate = parse(&body)?;
    Ok(Json(s.usage.set_enabled(
        at,
        g.enabled,
        g.generation,
        Instant::now(),
    )?))
}

/// `POST /v1/node/usage/clear`: Clear all (the switch stays as it is).
pub(crate) async fn clear_all(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
    Query(query): Token,
) -> Result<StatusCode, ApiError> {
    let at = owner_in(&s, &c, &headers, &query)?;
    s.usage.clear_all(at, Instant::now())?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /v1/node/usage/summary`: aggregates only.
pub(crate) async fn summary(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
    Query(query): Token,
) -> Result<Response, ApiError> {
    let at = owner_in(&s, &c, &headers, &query)?;
    let out = s.usage.summary(at, &s.version, Instant::now(), unix_now())?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(out)).into_response())
}

/// `GET /v1/node/usage/export`: a download for the owner.
pub(crate) async fn export(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
    Query(query): Token,
) -> Result<Response, ApiError> {
    let at = owner_in(&s, &c, &headers, &query)?;
    let out = s.usage.export(at, &s.version, Instant::now(), unix_now())?;
    Ok((
        [
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"odeion-usage.json\"",
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

    const T1: &str = "pml_tokenone";
    const T2: &str = "pml_tokentwo";

    fn l(k: &str) -> Person {
        Person::Listener(k.into())
    }

    fn usage(dir: &FsPath) -> Usage {
        Usage::with(Some(dir.join(FILE)), Limits::default(), Ttl::default())
    }

    fn small(dir: &FsPath, f: impl FnOnce(&mut Limits)) -> Usage {
        let mut limits = Limits::default();
        f(&mut limits);
        Usage::with(
            Some(dir.join(FILE)),
            limits,
            Ttl {
                idle: Duration::from_secs(120),
                max: Duration::from_secs(600),
                keep: Duration::from_secs(1200),
            },
        )
    }

    fn on(u: &Usage, now: Instant) {
        let at = u.epoch(&Person::Owner);
        let g = u.status(at, now).unwrap().generation;
        u.set_enabled(at, true, Some(g), now).unwrap();
    }

    fn open(u: &Usage, p: &Person, token: &str, now: Instant) -> String {
        let at = u.epoch(p);
        let me = u.me(at, p, now).unwrap();
        u.open(
            at,
            p,
            token,
            Viewport::Phone,
            me.generation,
            me.choice,
            now,
            1_000 * DAY,
        )
        .unwrap()
    }

    fn send(u: &Usage, p: &Person, token: &str, id: &str, n: usize, now: Instant) -> Result<(), Refusal> {
        let events = vec![
            Event::Nav {
                view: View::Home,
                how: How::Push
            };
            n
        ];
        u.ingest(u.epoch(p), p, id, token, &events, 0, now, 1_000 * DAY)
    }

    fn stored(u: &Usage) -> u64 {
        u.with_store(false, |s| s.size())
            .unwrap()
            .map(|(c, _)| c)
            .unwrap_or(0)
    }

    #[test]
    fn aliases_end_when_idle_or_old_and_clear_mine_reaches_only_what_is_still_linked() {
        let dir = tempfile::tempdir().unwrap();
        let u = small(dir.path(), |_| {});
        let t0 = Instant::now();
        on(&u, t0);
        let a = open(&u, &l("a"), T1, t0);
        send(&u, &l("a"), T1, &a, 2, t0).unwrap();
        // Idle past its two minutes: ended, its binding kept for Clear mine
        assert_eq!(
            send(&u, &l("a"), T1, &a, 1, t0 + Duration::from_secs(121)),
            Err(Refusal::Gone)
        );
        u.flush(1_000 * DAY).unwrap();
        assert_eq!(stored(&u), 2);
        assert_eq!(
            u.clear_mine(u.epoch(&l("a")), &l("a"), t0 + Duration::from_secs(150))
                .unwrap(),
            2
        );
        // An alias used all along still ends at its maximum
        let b = open(&u, &l("a"), T1, t0 + Duration::from_secs(200));
        let mut t = t0 + Duration::from_secs(200);
        for _ in 0..12 {
            t += Duration::from_secs(50);
            let _ = send(&u, &l("a"), T1, &b, 1, t);
        }
        assert_eq!(
            send(&u, &l("a"), T1, &b, 1, t + Duration::from_secs(50)),
            Err(Refusal::Gone),
            "past 600 s"
        );
        u.flush(1_000 * DAY).unwrap();
        // Once the binding is forgotten, Clear mine can no longer reach those rows
        let later = t + Duration::from_secs(2000);
        u.housekeep(later);
        assert_eq!(
            u.clear_mine(u.epoch(&l("a")), &l("a"), later).unwrap(),
            0,
            "unlinked now"
        );
        assert!(stored(&u) > 0, "they stay until retention");
    }

    #[test]
    fn a_request_let_in_before_its_persons_revocation_lands_nowhere_another_persons_does() {
        let dir = tempfile::tempdir().unwrap();
        let u = usage(dir.path());
        let t0 = Instant::now();
        on(&u, t0);
        let a = open(&u, &l("a"), T1, t0);
        let b = open(&u, &l("b"), T2, t0);
        let (at_a, at_b) = (u.epoch(&l("a")), u.epoch(&l("b")));
        u.revoke_listener("a");
        let ev = [Event::Exception {
            class: ErrorClass::TypeError,
        }];
        assert_eq!(
            u.ingest(at_a, &l("a"), &a, T1, &ev, 0, t0, 0),
            Err(Refusal::Revoked)
        );
        assert_eq!(u.ingest(at_b, &l("b"), &b, T2, &ev, 0, t0, 0), Ok(()));
        for op in [
            u.choose(at_a, &l("a"), false, None, t0).map(|_| ()),
            u.clear_mine(at_a, &l("a"), t0).map(|_| ()),
            u.me(at_a, &l("a"), t0).map(|_| ()),
        ] {
            assert_eq!(op, Err(Refusal::Revoked));
        }
        // Everyone's: a restore or another identity
        let at_b = u.epoch(&l("b"));
        u.revoke_all();
        assert_eq!(
            u.ingest(at_b, &l("b"), &b, T2, &ev, 0, t0, 0),
            Err(Refusal::Revoked)
        );
        assert_eq!(u.held().0, 0, "every alias ended");
        // The owner's side too
        let at = u.epoch(&Person::Owner);
        u.revoke_owner();
        assert_eq!(
            u.set_enabled(at, false, None, t0).map(|_| ()),
            Err(Refusal::Revoked)
        );
        assert_eq!(u.summary(at, "t", t0, 0).map(|_| ()), Err(Refusal::Revoked));
        assert_eq!(u.export(at, "t", t0, 0).map(|_| ()), Err(Refusal::Revoked));
        assert_eq!(u.clear_all(at, t0), Err(Refusal::Revoked));
    }

    #[test]
    fn an_alias_takes_only_the_credential_and_person_it_was_opened_with() {
        let dir = tempfile::tempdir().unwrap();
        let u = usage(dir.path());
        let t0 = Instant::now();
        on(&u, t0);
        let a = open(&u, &l("a"), T1, t0);
        // The same person's other device (another token) and another person with the same token
        assert_eq!(send(&u, &l("a"), T2, &a, 1, t0), Err(Refusal::Gone));
        assert_eq!(send(&u, &l("b"), T1, &a, 1, t0), Err(Refusal::Gone));
        // Closing with another credential leaves it open
        u.close(u.epoch(&l("a")), &l("a"), &a, T2, t0).unwrap();
        assert_eq!(send(&u, &l("a"), T1, &a, 1, t0), Ok(()));
    }

    #[test]
    fn one_person_opening_page_after_page_never_ends_anyone_elses() {
        let dir = tempfile::tempdir().unwrap();
        let u = small(dir.path(), |_| {});
        let t0 = Instant::now();
        on(&u, t0);
        let b = open(&u, &l("b"), T2, t0);
        send(&u, &l("b"), T2, &b, 2, t0).unwrap();
        u.flush(1_000 * DAY).unwrap();
        // b's page closes: still linked to b for Clear mine
        u.close(u.epoch(&l("b")), &l("b"), &b, T2, t0).unwrap();
        let b2 = open(&u, &l("b"), T2, t0);
        // a opens as fast as allowed for a long while
        let mut t = t0;
        let mut refused = 0;
        for minute in 0..9u64 {
            t = t0 + Duration::from_secs(61 * minute);
            // b's page stays in use meanwhile
            send(&u, &l("b"), T2, &b2, 1, t).unwrap();
            for _ in 0..40 {
                let at = u.epoch(&l("a"));
                let me = u.me(at, &l("a"), t).unwrap();
                match u.open(
                    at,
                    &l("a"),
                    T1,
                    Viewport::Phone,
                    me.generation,
                    me.choice,
                    t,
                    1_000 * DAY,
                ) {
                    Ok(_) => {}
                    Err(Refusal::RateLimited) => refused += 1,
                    Err(e) => panic!("{e:?}"),
                }
            }
        }
        assert_eq!(refused, 9 * 10, "30 a minute, per person");
        let live_a = u.lock().aliases.values().filter(|x| x.person == l("a")).count();
        assert!(live_a <= PER_PERSON_ALIASES, "{live_a}");
        let ended_a = u.lock().ended.values().filter(|(q, _)| *q == l("a")).count();
        assert!(ended_a <= PER_PERSON_ENDED, "{ended_a}");
        // b's open alias and b's Clear mine are untouched by it
        assert_eq!(send(&u, &l("b"), T2, &b2, 1, t), Ok(()));
        assert_eq!(u.clear_mine(u.epoch(&l("b")), &l("b"), t).unwrap(), 2);
    }

    #[test]
    fn stop_and_off_apply_at_once_when_the_file_cannot_be_written() {
        use std::sync::atomic::Ordering;
        let dir = tempfile::tempdir().unwrap();
        let u = usage(dir.path());
        let t0 = Instant::now();
        on(&u, t0);
        let a = open(&u, &l("a"), T1, t0);
        let b = open(&u, &l("b"), T2, t0);
        send(&u, &l("a"), T1, &a, 3, t0).unwrap();
        u.fail_store.store(true, Ordering::SeqCst);
        let me = u.choose(u.epoch(&l("a")), &l("a"), false, None, t0).unwrap();
        assert!(!me.sharing && !me.saved, "applied, and says it isn't saved");
        assert_eq!(send(&u, &l("a"), T1, &a, 1, t0), Err(Refusal::Gone));
        assert_eq!(u.held().2, 0, "a's waiting events are dropped");
        let st = u.set_enabled(u.epoch(&Person::Owner), false, None, t0).unwrap();
        assert!(!st.enabled && !st.saved);
        assert_eq!(send(&u, &l("b"), T2, &b, 1, t0), Err(Refusal::Off));
        // Sharing again and On wait for the file
        let at = u.epoch(&l("a"));
        assert_eq!(
            u.choose(at, &l("a"), true, Some(me.choice), t0).map(|_| ()),
            Err(Refusal::Storage)
        );
        assert_eq!(
            u.set_enabled(u.epoch(&Person::Owner), true, Some(st.generation), t0)
                .map(|_| ()),
            Err(Refusal::Storage)
        );
    }

    #[test]
    fn the_identity_is_swapped_while_usage_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let u = usage(dir.path());
        let held = u.revoke_all_across(|| u.inner.try_lock().is_err());
        assert!(held, "no usage change can run across the swap");
    }

    #[test]
    fn the_switch_and_choices_survive_a_restart_aliases_and_bindings_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let t0 = Instant::now();
        let alias = {
            let u = usage(dir.path());
            on(&u, t0);
            u.choose(u.epoch(&l("a")), &l("a"), false, None, t0).unwrap();
            open(&u, &l("b"), T2, t0)
        };
        let u = usage(dir.path());
        let me = u.me(u.epoch(&l("a")), &l("a"), t0).unwrap();
        assert!(me.node_on, "still on");
        assert!(!me.sharing, "a's Stop is kept");
        assert_eq!(
            send(&u, &l("b"), T2, &alias, 1, t0),
            Err(Refusal::Gone),
            "aliases are memory only"
        );
    }

    #[test]
    fn rates_per_alias_person_and_node_and_the_queue_are_bounded_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let u = small(dir.path(), |l| {
            l.per_alias_per_min = 10;
            l.per_person_per_min = 15;
            l.global_per_min = 20;
            l.queue_max = 18;
        });
        let t0 = Instant::now();
        on(&u, t0);
        let a1 = open(&u, &l("a"), T1, t0);
        let a2 = open(&u, &l("a"), T1, t0);
        let b = open(&u, &l("b"), T2, t0);
        send(&u, &l("a"), T1, &a1, 10, t0).unwrap();
        assert_eq!(
            send(&u, &l("a"), T1, &a1, 1, t0),
            Err(Refusal::RateLimited),
            "per alias"
        );
        send(&u, &l("a"), T1, &a2, 5, t0).unwrap();
        assert_eq!(
            send(&u, &l("a"), T1, &a2, 1, t0),
            Err(Refusal::RateLimited),
            "per person"
        );
        send(&u, &l("b"), T2, &b, 5, t0).unwrap();
        assert_eq!(
            send(&u, &l("b"), T2, &b, 1, t0),
            Err(Refusal::RateLimited),
            "the node"
        );
        assert_eq!(u.held().2, 18, "the queue keeps 18 of 20");
        u.flush(1_000 * DAY).unwrap();
        let s = u.summary(u.epoch(&Person::Owner), "t", t0, 1_000 * DAY).unwrap();
        assert_eq!(s["dropped"]["rate_limited"], serde_json::json!(3));
        assert_eq!(s["dropped"]["queue_full"], serde_json::json!(2));
        // A new minute: room again
        send(&u, &l("a"), T1, &a1, 1, t0 + Duration::from_secs(61)).unwrap();
    }

    #[test]
    fn retention_and_caps_roll_the_oldest_day_out_first() {
        let dir = tempfile::tempdir().unwrap();
        let u = small(dir.path(), |l| {
            l.max_events = 25;
            l.raw_retention_days = 3;
        });
        let t0 = Instant::now();
        on(&u, t0);
        let a = open(&u, &l("a"), T1, t0);
        for day in 0..5u64 {
            let ev = vec![
                Event::Nav {
                    view: View::Home,
                    how: How::Push
                };
                10
            ];
            u.ingest(u.epoch(&l("a")), &l("a"), &a, T1, &ev, 0, t0, (1_000 + day) * DAY)
                .unwrap();
            u.flush((1_000 + day) * DAY).unwrap();
        }
        // Over 25: oldest days went as they came
        assert!(stored(&u) <= 25, "{}", stored(&u));
        u.housekeep_store(1_004 * DAY).unwrap();
        let days: Vec<u64> = u
            .with_store(false, |s| s.rows())
            .unwrap()
            .unwrap()
            .iter()
            .map(|r| r.0 / DAY)
            .collect();
        assert!(days.iter().all(|d| *d >= 1_002), "three days kept: {days:?}");
        let s = u.summary(u.epoch(&Person::Owner), "t", t0, 1_004 * DAY).unwrap();
        assert!(s["dropped"]["evicted"].as_u64().unwrap() > 0, "{s}");
    }

    #[test]
    fn the_schema_is_closed_and_bounded() {
        let bad = [
            r#"{"kind":"nav","view":"home"}"#,
            r#"{"kind":"nav","view":"home","how":"push","x":1}"#,
            r#"{"kind":"nav","view":"/albums/12","how":"push"}"#,
            r#"{"kind":"action","control":"row_play","view":"albums","surface":"page","via":"click","label":"Song"}"#,
            r#"{"kind":"action","control":"Row Play","view":"albums","surface":"page","via":"click"}"#,
            r#"{"kind":"api","area":"library","class":"read","outcome":"ok","ms":120}"#,
            r#"{"kind":"exception","class":"type_error","message":"x"}"#,
            r#"{"kind":"friction","signal":"error_toast","view":"home","text":"x"}"#,
            r#"{"kind":"typing","text":"x"}"#,
        ];
        for b in bad {
            assert!(serde_json::from_str::<Event>(b).is_err(), "{b}");
        }
        let dup = Event::Visit {
            view: View::Albums,
            surface: Surface::Page,
            ms: Ms::Lt1000,
            exposed: vec![Control::RowPlay, Control::RowPlay],
            used: vec![],
            unregistered: 0,
        };
        assert!(!dup.bounded());
        let many = Event::Visit {
            view: View::Albums,
            surface: Surface::Page,
            ms: Ms::Lt1000,
            exposed: vec![Control::RowPlay; MAX_EXPOSED + 1],
            used: vec![],
            unregistered: 0,
        };
        assert!(!many.bounded());
        // Daily keys are schema names only
        for (_, key) in daily_keys(&Event::Api {
            area: Area::Library,
            class: Class::Read,
            outcome: Outcome::Http5xx,
            ms: Ms::Lt100,
        }) {
            assert_eq!(key, "library.read.http_5xx");
        }
    }

    #[test]
    fn forgetting_a_listener_takes_their_choice_and_linked_rows() {
        let dir = tempfile::tempdir().unwrap();
        let u = usage(dir.path());
        let t0 = Instant::now();
        on(&u, t0);
        let a = open(&u, &l("a"), T1, t0);
        let b = open(&u, &l("b"), T2, t0);
        send(&u, &l("a"), T1, &a, 3, t0).unwrap();
        send(&u, &l("b"), T2, &b, 2, t0).unwrap();
        u.flush(1_000 * DAY).unwrap();
        // a stopped once: their choice is stored
        u.choose(u.epoch(&l("a")), &l("a"), false, None, t0).unwrap();
        let pref = u.with_store(false, |s| s.pref(&l("a"))).unwrap().unwrap();
        assert!(pref.stopped);
        u.forget_listener("a");
        assert_eq!(stored(&u), 2, "a's linked rows went, b's stay");
        let pref = u.with_store(false, |s| s.pref(&l("a"))).unwrap().unwrap();
        assert!(!pref.stopped && pref.choice == 0, "a's stored choice is gone");
    }
}
