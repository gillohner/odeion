//! Who may do what on the node, decided in one place.
//!
//! A request comes from a [`Caller`]: the owner, a listener, an app the owner gave an app password,
//! or a party guest ([`Role`]). Every
//! `/v1` route behind the sign-in check names, per method, the one [`Capability`] it needs
//! ([`ROUTES`]). The check in front of the handlers asks [`decide`] and nothing else. A route that
//! names no capability is the owner's alone, and the `every_route_names_its_capability` test fails
//! until it names one. The routes that take no sign-in are a fixed list ([`OPEN`]), held by a test
//! too. Subsonic (`/rest`, the API token or an app password as its password) and the UPnP renderer
//! (LAN peers only) are surfaces of their own, outside both.
//!
//! | Role | Has |
//! | :-- | :-- |
//! | owner | everything |
//! | listener | [`Capability::Artwork`], [`Capability::Session`], [`Capability::Library`], [`Capability::ZoneList`], [`Capability::ZoneUse`] on the zones the owner opened, [`Capability::Collections`], [`Capability::Own`], [`Capability::OwnPlay`], [`Capability::OwnSignInState`], and [`Capability::OwnSignIn`] unless the node plays everyone through its own sign-ins |
//! | app | [`Capability::Artwork`], [`Capability::Session`], [`Capability::Library`], [`Capability::ZoneList`], [`Capability::ZoneUse`] on every zone, [`Capability::ZoneGroup`], [`Capability::Collections`] and [`Capability::OwnPlay`], all as the owner's |
//! | party guest | [`Capability::Artwork`], [`Capability::PartyGuest`] |
//!
//! Handlers that take a zone from the body instead of the path ask
//! [`crate::accounts::may_use_zone`]; handlers that answer differently per person ask
//! [`Caller::can`].

use crate::accounts::{Caller, Role, SourcePolicy};
use crate::state::NodeState;
use axum::http::Method;
use axum::routing::MethodRouter;
use axum::Router;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// What a request needs to be allowed. One per route and method ([`ROUTES`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    // ---- everyone with any sign-in, a party guest included ----
    /// Album covers.
    Artwork,
    /// A party guest's own calls: the party they joined, its search and its queue.
    PartyGuest,

    // ---- the owner and listeners ----
    /// Who am I, sign out, the node's name and health, the event stream.
    Session,
    /// Browse, search and play from the shared library and the public catalogues.
    Library,
    /// The list of zones (each caller sees the ones they may use).
    ZoneList,
    /// See and control one zone: queue, transport, volume, sleep. For a listener only a zone the
    /// owner opened to listeners.
    ZoneUse,
    /// The caller's bookmarks and collections, and the order they play in.
    Collections,
    /// The caller's other own things: preferences, devices and downloads, presence, problem
    /// reports, their usage choice, their sign-ins on this node and their own homeserver link.
    Own,
    /// Play with the caller's own streaming sign-ins.
    OwnPlay,
    /// See the caller's own streaming sign-ins, and sign out of them.
    OwnSignInState,
    /// Sign in to the caller's own streaming services. Not for a listener while the node plays
    /// everyone through its own sign-ins ([`SourcePolicy::Node`]).
    OwnSignIn,

    // ---- the owner ----
    /// The owner's own music data, which is not kept per person yet: tags, reviews, comments,
    /// history, smart collections, Home, radio stations, podcasts, follows, imports, and browsing a
    /// streaming service with the node's sign-ins.
    OwnerMusic,
    /// Add, remove and tune zones and speakers (outputs, DSP, crossfade).
    ZoneManage,
    /// See the speakers, and group them into a zone and ungroup it (what a home automation does).
    ZoneGroup,
    /// Change the library: folders, shares and drives, scans, uploads, edits, hiding and deleting
    /// songs, identification, homeserver files, storage.
    LibraryManage,
    /// The node's settings and its service apps (app ids, keys, helper programs, which sources are on).
    Settings,
    /// The node's own sign-ins at services and media servers.
    ServiceLogins,
    /// People: invites, music access, the zones open to listeners.
    People,
    /// The node's API token.
    Tokens,
    /// The sources vault: its devices and keys.
    Vault,
    /// Back up and restore.
    Backup,
    /// The node's Pubky identity and its homeserver.
    Identity,
    /// Reaching the node from outside (the tunnel).
    RemoteAccess,
    /// Start and end parties.
    Party,
    /// Usage insights across the node, and the speaker discovery report.
    Diagnostics,
}

impl Capability {
    /// Every capability, in the order of the enum.
    pub const ALL: &'static [Capability] = &[
        Capability::Artwork,
        Capability::PartyGuest,
        Capability::Session,
        Capability::Library,
        Capability::ZoneList,
        Capability::ZoneUse,
        Capability::Collections,
        Capability::Own,
        Capability::OwnPlay,
        Capability::OwnSignInState,
        Capability::OwnSignIn,
        Capability::OwnerMusic,
        Capability::ZoneManage,
        Capability::ZoneGroup,
        Capability::LibraryManage,
        Capability::Settings,
        Capability::ServiceLogins,
        Capability::People,
        Capability::Tokens,
        Capability::Vault,
        Capability::Backup,
        Capability::Identity,
        Capability::RemoteAccess,
        Capability::Party,
        Capability::Diagnostics,
    ];

    /// Why a listener is refused a capability their role does not have.
    fn owners(self) -> &'static str {
        match self {
            Capability::ZoneManage => "only the owner adds, removes or tunes zones",
            Capability::OwnerMusic => {
                "that is the owner's: a listener's own are their bookmarks and collections"
            }
            _ => OWNERS,
        }
    }
}

/// The refusal for what is the owner's alone.
const OWNERS: &str = "that is the owner's";
/// The refusal a party guest gets for everything else.
const GUESTS: &str = "a party guest can only add songs to the party";
/// The refusal an app password gets for everything else.
const APPS: &str = "an app password plays, controls zones and browses; for this, sign in as the owner";

impl Role {
    /// Whether this role has `cap` at all. A listener's [`Capability::ZoneUse`] and
    /// [`Capability::OwnSignIn`] also depend on the node's settings: [`decide`].
    pub const fn has(self, cap: Capability) -> bool {
        use Capability::*;
        match self {
            Role::Owner => true,
            Role::Listener => matches!(
                cap,
                Artwork
                    | Session
                    | Library
                    | ZoneList
                    | ZoneUse
                    | Collections
                    | Own
                    | OwnPlay
                    | OwnSignInState
                    | OwnSignIn
            ),
            // What a home automation or a Subsonic app needs, and nothing that changes the node's
            // settings or speakers, its people, its sign-ins or the library
            Role::App => matches!(
                cap,
                Artwork | Session | Library | ZoneList | ZoneUse | ZoneGroup | Collections | OwnPlay
            ),
            Role::Guest => matches!(cap, Artwork | PartyGuest),
        }
    }
}

impl Caller {
    /// Whether this caller's role has `cap` ([`Role::has`]).
    pub const fn can(&self, cap: Capability) -> bool {
        self.role.has(cap)
    }
}

/// What a request's path names that a decision depends on.
#[derive(Debug, Default, Clone, Copy)]
pub struct At<'a> {
    /// The zone of a `/zones/{id}/…` route.
    pub zone: Option<&'a str>,
    /// The source of a `/sources/{source}/…` route.
    pub source: Option<&'a str>,
    /// The route names a source (`{source}`), whether or not the request's could be read.
    pub names_source: bool,
}

impl<'a> At<'a> {
    /// From a matched route (`/zones/{id}/queue`) and its path parameters.
    pub fn of(route: &str, params: impl Iterator<Item = (&'a str, &'a str)>) -> Self {
        let route = route.strip_prefix("/v1").unwrap_or(route);
        let zone_route = route == "/zones/{id}" || route.starts_with("/zones/{id}/");
        let mut at = At {
            names_source: route.contains("/{source}"),
            ..At::default()
        };
        for (k, v) in params {
            match k {
                "id" if zone_route => at.zone = Some(v),
                "source" => at.source = Some(v),
                _ => {}
            }
        }
        at
    }
}

/// Whether `caller` may do what needs `cap`, here. `None` = yes; `Some(why)` = no. `cap` is
/// `None` for a route that names no capability: the owner's alone.
pub fn decide(s: &NodeState, caller: &Caller, cap: Option<Capability>, at: &At<'_>) -> Option<&'static str> {
    if caller.role == Role::Owner {
        return None;
    }
    let Some(cap) = cap else {
        return Some(OWNERS);
    };
    if !caller.role.has(cap) {
        return Some(match caller.role {
            Role::Guest => GUESTS,
            Role::App => APPS,
            _ => cap.owners(),
        });
    }
    if caller.role != Role::Listener {
        return None;
    }
    let own_source = matches!(
        cap,
        Capability::OwnPlay | Capability::OwnSignInState | Capability::OwnSignIn
    );
    match cap {
        // A zone's state, queue and controls: only the zones open to listeners
        Capability::ZoneUse => match at.zone {
            Some(zone) => crate::accounts::zone_refusal(s, zone),
            None => Some(OWNERS),
        },
        // A source with one node-wide sign-in (or none) is the owner's, and so is a request
        // whose source can't be read
        _ if own_source && at.source.is_some_and(|x| !PER_PERSON.contains(&x)) => Some(OWNERS),
        _ if own_source && at.names_source && at.source.is_none() => Some(OWNERS),
        // The node plays through its own sign-ins: a listener can't sign in to their own (what
        // they had stays; their own status, signing out, plays and streams go on)
        Capability::OwnSignIn if node_sign_ins_only(s) => Some(crate::accounts::NODE_SOURCES_ONLY),
        _ => None,
    }
}

/// The sources each person signs in to on their own. Every other source has one sign-in for the
/// whole node, the owner's, or none: a new source is the owner's until it is named here.
pub const PER_PERSON: &[&str] = &["tidal", "qobuz", "spotify"];

/// What `caller` may do on this node now, for `GET /v1/me`: their role's capabilities, without
/// [`Capability::OwnSignIn`] while the node plays everyone through its own sign-ins, and with
/// [`Capability::PartyGuest`] only for a guest (those calls answer nobody else).
/// [`Capability::ZoneUse`] is listed for a listener whatever zones are open: `/v1/me` names those.
pub fn can(s: &NodeState, caller: &Caller) -> Vec<Capability> {
    let node_only = caller.role == Role::Listener && node_sign_ins_only(s);
    Capability::ALL
        .iter()
        .copied()
        .filter(|cap| caller.role.has(*cap))
        .filter(|cap| !(node_only && *cap == Capability::OwnSignIn))
        .filter(|cap| *cap != Capability::PartyGuest || caller.role == Role::Guest)
        .collect()
}

/// Whether the node plays everyone's music through its own sign-ins ([`SourcePolicy::Node`]).
fn node_sign_ins_only(s: &NodeState) -> bool {
    crate::accounts::source_policy(&s.library.lock().expect("library poisoned")) == SourcePolicy::Node
}

/// A route's method, as [`ROUTES`] lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verb {
    Get,
    Post,
    Put,
    Delete,
    Patch,
}

impl Verb {
    pub const ALL: [Verb; 5] = [Verb::Get, Verb::Post, Verb::Put, Verb::Delete, Verb::Patch];

    /// The verb a request's method is listed under (`HEAD` is answered by the `GET` handler).
    pub fn of(m: &Method) -> Option<Verb> {
        Some(match *m {
            Method::GET | Method::HEAD => Verb::Get,
            Method::POST => Verb::Post,
            Method::PUT => Verb::Put,
            Method::DELETE => Verb::Delete,
            Method::PATCH => Verb::Patch,
            _ => return None,
        })
    }

    pub fn method(self) -> Method {
        match self {
            Verb::Get => Method::GET,
            Verb::Post => Method::POST,
            Verb::Put => Method::PUT,
            Verb::Delete => Method::DELETE,
            Verb::Patch => Method::PATCH,
        }
    }
}

fn table() -> &'static HashMap<(Verb, &'static str), Capability> {
    static TABLE: OnceLock<HashMap<(Verb, &'static str), Capability>> = OnceLock::new();
    TABLE.get_or_init(|| {
        ROUTES
            .iter()
            .flat_map(|(cap, routes)| routes.iter().map(move |r| (*r, *cap)))
            .collect()
    })
}

/// The capability a matched route needs for this method (`route` as the router matched it, with
/// or without the `/v1` it is nested under). `None`: the route names none.
pub fn required(method: &Method, route: &str) -> Option<Capability> {
    let route = route.strip_prefix("/v1").unwrap_or(route);
    table().get(&(Verb::of(method)?, route)).copied()
}

/// The listed route a path would match (`/zones/kitchen/queue` → `/zones/{id}/queue`), with its
/// parameters: for callers that hold a path instead of the router's match (tests, mostly). As the
/// router does, a fixed segment wins over a parameter, whatever methods the route has
/// (`GET /playlists/import` is the import route, which has no `GET`: [`required`] then names no
/// capability). Unlike the router it reads the path as written: no percent-decoding.
pub fn route_of(path: &str) -> Option<(&'static str, Vec<(&'static str, String)>)> {
    let path = path.strip_prefix("/v1").unwrap_or(path);
    let segs: Vec<&str> = path.split('/').collect();
    let mut best: Option<(&'static str, Vec<bool>)> = None;
    for (_, route) in table().keys() {
        let rsegs: Vec<&str> = route.split('/').collect();
        if rsegs.len() != segs.len() {
            continue;
        }
        let fixed: Vec<bool> = rsegs.iter().map(|r| !r.starts_with('{')).collect();
        let fits = rsegs
            .iter()
            .zip(&segs)
            .zip(&fixed)
            .all(|((r, p), f)| if *f { r == p } else { !p.is_empty() });
        if fits && best.as_ref().is_none_or(|(_, b)| fixed > *b) {
            best = Some((*route, fixed));
        }
    }
    let (route, _) = best?;
    let params = route
        .split('/')
        .zip(&segs)
        .filter_map(|(r, p)| {
            let name = r.strip_prefix('{')?.strip_suffix('}')?;
            Some((name, (*p).to_string()))
        })
        .collect();
    Some((route, params))
}

/// [`decide`] for a path instead of the router's match. A path no route lists is refused to
/// everyone but the owner.
pub fn decide_path(s: &NodeState, caller: &Caller, method: &Method, path: &str) -> Option<&'static str> {
    match route_of(path) {
        Some((route, params)) => {
            let at = At::of(route, params.iter().map(|(k, v)| (*k, v.as_str())));
            decide(s, caller, required(method, route), &at)
        }
        None => decide(s, caller, None, &At::default()),
    }
}

/// Whether a party guest may make this request, by its path. `None` = yes; `Some(why)` = no.
pub fn guest_refusal(method: &Method, path: &str) -> Option<&'static str> {
    match route_of(path).and_then(|(route, _)| required(method, route)) {
        Some(cap) if Role::Guest.has(cap) => None,
        Some(_) => Some(GUESTS),
        None => Some(OWNERS),
    }
}

/// The routes reached without a sign-in, each with why. They sit outside the sign-in check, so
/// no capability applies: what each one hands out is its own handler's to guard.
pub const OPEN: &[(&str, &str)] = &[
    (
        "/tidal/callback",
        "OAuth redirect target: needs the state of a sign-in someone started",
    ),
    ("/spotify/callback", "OAuth redirect target"),
    ("/lastfm/callback", "OAuth redirect target"),
    ("/drives/{provider}/callback", "OAuth redirect target"),
    ("/jamendo/callback", "OAuth redirect target"),
    (
        "/v1/auth/pubky/start",
        "starts a Pubky Ring sign-in: this is how a token is got",
    ),
    (
        "/v1/auth/pubky/poll",
        "the sign-in's answer, to whoever started it",
    ),
    (
        "/v1/auth/pubky/owner/avatar",
        "the owner's picture on the Log in screen",
    ),
    ("/party/{code}", "a party's page, by its link"),
    ("/v1/party/join", "joining a party with its link's code"),
    (
        pm_discovery::DESCRIPTION_PATH,
        "LAN discovery: names the node, nothing more",
    ),
    (
        "/v1/outputs/pull/{name}/{file}",
        "a renderer's stream, by the stream's own key",
    ),
    (
        "/v1/upnp/events/{token}",
        "a renderer's volume events, by the token the node gave it; LAN only",
    ),
    (
        "/v1/art/{id}",
        "an album cover for a TV screen, by that cover's key",
    ),
    ("/mb/ws/2/{*path}", "the MusicBrainz caching proxy"),
];

/// Routes as they are added, so that a test can walk them (`every_route_names_its_capability`,
/// `the_routes_without_a_sign_in_are_the_listed_ones`).
pub(crate) struct Routes {
    router: Router<Arc<NodeState>>,
    paths: Vec<&'static str>,
}

impl Routes {
    pub(crate) fn new() -> Self {
        Self {
            router: Router::new(),
            paths: Vec::new(),
        }
    }

    /// As `axum::Router::route`.
    pub(crate) fn route(mut self, path: &'static str, methods: MethodRouter<Arc<NodeState>>) -> Self {
        self.paths.push(path);
        self.router = self.router.route(path, methods);
        self
    }

    /// The router of routes that take no sign-in ([`OPEN`]).
    pub(crate) fn into_open_router(self) -> Router<Arc<NodeState>> {
        self.router
    }

    /// The paths added so far.
    #[cfg(test)]
    pub(crate) fn paths(&self) -> &[&'static str] {
        &self.paths
    }

    /// The router, and the paths added to it.
    pub(crate) fn into_parts(self) -> (Router<Arc<NodeState>>, Vec<&'static str>) {
        // A path the table doesn't know stays the owner's alone; say so where an operator looks
        static LISTED: OnceLock<std::collections::HashSet<&'static str>> = OnceLock::new();
        let listed = LISTED.get_or_init(|| table().keys().map(|(_, r)| *r).collect());
        for p in &self.paths {
            if !listed.contains(p) {
                tracing::error!("route {p} names no capability (access.rs): only the owner can use it");
            }
        }
        (self.router, self.paths)
    }
}

use Capability::*;
use Verb::*;

/// Every `/v1` route and method, under the capability it needs.
pub static ROUTES: &[(Capability, &[(Verb, &str)])] = &[
    (Artwork, &[(Get, "/library/albums/{id}/cover")]),
    (
        PartyGuest,
        &[
            (Get, "/party/guest"),
            (Get, "/party/guest/search"),
            (Post, "/party/guest/queue"),
        ],
    ),
    (
        Session,
        &[
            (Get, "/info"),
            (Get, "/health"),
            (Get, "/me"),
            (Delete, "/me/token"),
            (Get, "/events"),
        ],
    ),
    (
        Library,
        &[
            (Get, "/library/stats"),
            (Get, "/library/tracks"),
            (Get, "/library/tracks/{id}"),
            (Get, "/library/tracks/{id}/lyrics"),
            (Get, "/lyrics"),
            (Get, "/library/tracks/{id}/versions"),
            (Get, "/library/albums/{id}/versions"),
            (Get, "/library/tracks/{id}/file"),
            (Get, "/library/tracks/{id}/download"),
            (Get, "/library/search"),
            (Get, "/library/artists"),
            (Get, "/library/artists/{id}"),
            (Get, "/library/artists/{id}/tracks"),
            (Get, "/library/artists/{id}/info"),
            (Get, "/library/artists/{id}/image"),
            (Get, "/library/genres"),
            (Get, "/library/genres/{id}"),
            (Get, "/library/albums"),
            (Get, "/library/albums/formats"),
            (Get, "/library/albums/{id}/tracks"),
            (Get, "/shared/resolve"),
            (Get, "/shared/{user}/playlists"),
            (Get, "/shared/{user}/playlists/{id}"),
            (Get, "/shared/{user}/collections"),
            (Get, "/shared/{user}/collections/{id}"),
            (Get, "/discover/trends"),
            (Get, "/discover/charts"),
            (Get, "/discover/search"),
            (Get, "/discover/item"),
            (Get, "/discover/play"),
            (Get, "/discover/contents"),
            (Get, "/search"),
            (Get, "/search/tags"),
            (Get, "/item/{kind}/{id}"),
            (Get, "/item/{kind}/{id}/extras"),
            (Get, "/discover/uri"),
            (Get, "/discover/tag/{label}"),
            (Get, "/discover/friends/{viewer}"),
            (Get, "/library/artists/{id}/similar"),
            (Get, "/radio/search"),
            (Get, "/podcasts/search"),
            (Get, "/radio/curated"),
            (Get, "/radio/countries"),
            (Get, "/radio/tags"),
            (Get, "/profiles"),
            (Get, "/profiles/{id}"),
            (Get, "/profiles/{id}/avatar"),
        ],
    ),
    (ZoneList, &[(Get, "/zones")]),
    (
        ZoneUse,
        &[
            (Get, "/zones/{id}"),
            (Get, "/zones/{id}/queue"),
            (Post, "/zones/{id}/queue"),
            (Delete, "/zones/{id}/queue"),
            (Delete, "/zones/{id}/queue/{item}"),
            (Put, "/zones/{id}/queue/{item}"),
            (Post, "/zones/{id}/transport"),
            (Post, "/zones/{id}/announce"),
            (Post, "/zones/{id}/volume"),
            (Get, "/zones/{id}/volume"),
            (Put, "/zones/{id}/repeat"),
            (Put, "/zones/{id}/radio"),
            (Post, "/zones/{id}/shuffle"),
            (Put, "/zones/{id}/shuffle"),
            (Post, "/zones/{id}/transfer"),
            (Post, "/zones/{id}/load"),
            (Get, "/zones/{id}/sleep"),
            (Put, "/zones/{id}/sleep"),
            (Get, "/zones/{id}/dsp"),
        ],
    ),
    (
        Collections,
        &[
            (Get, "/collections"),
            (Post, "/collections"),
            (Get, "/collections/{id}"),
            (Put, "/collections/{id}"),
            (Delete, "/collections/{id}"),
            (Post, "/collections/{id}/items"),
            (Delete, "/collections/{id}/items/{index}"),
            (Post, "/collections/{id}/play"),
            (Get, "/playlists"),
            (Post, "/playlists"),
            (Get, "/bookmarks"),
            (Put, "/bookmarks"),
            (Delete, "/bookmarks"),
            (Get, "/favourites"),
            (Put, "/favourites"),
            (Delete, "/favourites"),
            (Get, "/playlists/{id}"),
            (Put, "/playlists/{id}"),
            (Delete, "/playlists/{id}"),
            (Post, "/playlists/{id}/items"),
            (Delete, "/playlists/{id}/items/{index}"),
            (Post, "/playlists/{id}/play"),
            (Get, "/play-order"),
            (Put, "/play-order"),
        ],
    ),
    (
        Own,
        &[
            (Get, "/me/sign-ins"),
            (Delete, "/me/sign-ins"),
            (Delete, "/me/sign-ins/{id}"),
            // (making a collection public posts it under the caller's name: not for an app)
            (Put, "/collections/{id}/visibility"),
            (Put, "/playlists/{id}/visibility"),
            (Get, "/me/prefs"),
            (Put, "/me/prefs"),
            (Get, "/me/presence"),
            (Put, "/me/presence"),
            (Get, "/presence"),
            (Post, "/presence/along"),
            (Delete, "/presence/along"),
            (Get, "/me/first-steps"),
            (Get, "/me/homeserver"),
            (Delete, "/me/homeserver"),
            (Post, "/me/homeserver/connect"),
            (Get, "/me/homeserver/connect/poll"),
            (Get, "/usage/me"),
            (Put, "/usage/me"),
            (Post, "/usage/me/notice"),
            (Post, "/usage/me/clear"),
            (Post, "/usage/sessions"),
            (Post, "/usage/sessions/close"),
            (Post, "/usage/events"),
            (Get, "/me/services"),
            (Get, "/me/sources"),
            (Put, "/me/sources"),
            (Post, "/feedback"),
            (Get, "/feedback/mine"),
            (Get, "/devices"),
            (Put, "/devices/{id}"),
            (Delete, "/devices/{id}"),
            (Get, "/devices/{id}/downloads"),
            (Put, "/devices/{id}/downloads"),
        ],
    ),
    (
        OwnPlay,
        &[
            (Get, "/sources/tidal/stream/{id}"),
            (Get, "/sources/qobuz/stream/{id}"),
            (Get, "/sources/spotify/devices"),
            (Post, "/sources/spotify/play"),
        ],
    ),
    (
        OwnSignInState,
        &[
            (Delete, "/sources/qobuz/unofficial/login"),
            (Delete, "/sources/tidal/unofficial/login"),
            (Get, "/sources/{source}/signin"),
            (Delete, "/sources/{source}/signin"),
            (Get, "/sources/tidal/status"),
            (Get, "/sources/tidal/unofficial"),
            (Delete, "/sources/tidal/session"),
            (Get, "/sources/qobuz/unofficial"),
            (Get, "/sources/spotify/status"),
            (Delete, "/sources/spotify/session"),
        ],
    ),
    (
        OwnSignIn,
        &[
            (Put, "/sources/{source}/signin"),
            (Post, "/sources/{source}/signin/import"),
            (Get, "/sources/tidal/login"),
            (Put, "/sources/tidal/unofficial"),
            (Post, "/sources/tidal/unofficial/login"),
            (Post, "/sources/tidal/unofficial/browser"),
            (Post, "/sources/tidal/unofficial/browser/finish"),
            (Post, "/sources/tidal/callback"),
            (Post, "/sources/spotify/callback"),
            (Put, "/sources/qobuz/unofficial"),
            (Post, "/sources/qobuz/unofficial/login"),
            (Get, "/sources/spotify/login"),
        ],
    ),
    (
        OwnerMusic,
        &[
            (Get, "/me/music"),
            (Get, "/library/new-releases"),
            (Post, "/library/new-releases/refresh"),
            (Get, "/library/history"),
            (Post, "/library/history"),
            (Get, "/library/home"),
            (Delete, "/collections/{id}/local/{index}"),
            (Get, "/collections/{id}/play-from"),
            (Put, "/collections/{id}/order"),
            (Put, "/collections/{id}/items/{index}/source"),
            (Post, "/playlists/from-queue"),
            (Post, "/playlists/import"),
            (Post, "/playlists/import/file"),
            (Post, "/import/roon"),
            (Get, "/activity"),
            (Post, "/activity"),
            (Get, "/comments"),
            (Post, "/comments"),
            (Delete, "/comments"),
            (Post, "/posts"),
            (Get, "/tags"),
            (Put, "/tags"),
            (Delete, "/tags"),
            (Post, "/tags/bulk"),
            (Get, "/reviews"),
            (Put, "/reviews"),
            (Delete, "/reviews"),
            (Delete, "/playlists/{id}/local/{index}"),
            (Get, "/playlists/{id}/play-from"),
            (Put, "/playlists/{id}/order"),
            (Put, "/playlists/{id}/items/{index}/source"),
            (Post, "/shared/{user}/playlists/{id}/copy"),
            (Post, "/shared/{user}/collections/{id}/duplicate"),
            (Get, "/cards"),
            (Post, "/discover/describe"),
            (Get, "/home/location"),
            (Put, "/home/location"),
            (Get, "/home/layout"),
            (Put, "/home/layout"),
            (Get, "/smart"),
            (Post, "/smart"),
            (Post, "/smart/preview"),
            (Put, "/smart/{id}"),
            (Delete, "/smart/{id}"),
            (Get, "/smart/{id}/tracks"),
            (Get, "/home/{section}"),
            (Get, "/sources"),
            (Get, "/sources/{source}/search"),
            (Get, "/sources/{source}/items/{id}"),
            (Get, "/sources/{source}/artists/{id}"),
            (Get, "/sources/tidal/collection"),
            (Get, "/sources/deezer/stream/{id}"),
            (Get, "/sources/jamendo/stream/{id}"),
            (Get, "/sources/bandcamp/stream/{key}"),
            (Get, "/sources/bandcamp/collection"),
            (Post, "/sources/bandcamp/collection/{id}/download"),
            (Get, "/sources/bandcamp/downloads"),
            (Get, "/sources/soundcloud/stream/{id}"),
            (Get, "/sources/soundcloud/likes"),
            (Post, "/sources/soundcloud/likes/import"),
            (Get, "/sources/soundcloud/playlists"),
            (Post, "/sources/soundcloud/playlists/{id}/import"),
            (Get, "/sources/subsonic/stream/{id}"),
            (Get, "/sources/jellyfin/stream/{id}"),
            (Post, "/listenbrainz/import"),
            (Get, "/listenbrainz/recommendations"),
            (Get, "/listenbrainz/playlists"),
            (Post, "/listenbrainz/playlists/{id}/import"),
            (Get, "/sources/jellyfin/cover/{id}"),
            (Get, "/sources/plex/stream/{id}"),
            (Get, "/sources/plex/cover/{id}/{ver}"),
            (Post, "/sources/audiobookshelf/play"),
            (Get, "/sources/audiobookshelf/continue"),
            (Get, "/sources/audiobookshelf/stream/{item}/{ino}"),
            (Get, "/sources/audiobookshelf/cover/{item}"),
            (Get, "/sources/jellyfin/playlists"),
            (Post, "/sources/jellyfin/playlists/{id}/import"),
            (Get, "/sources/subsonic/cover/{id}"),
            (Get, "/sources/subsonic/playlists"),
            (Post, "/sources/subsonic/playlists/{id}/import"),
            (Get, "/sources/jamendo/status"),
            (Post, "/sources/jamendo/collection/import"),
            (Get, "/sources/spotify/collection"),
            (Post, "/sources/spotify/collection/import"),
            (Get, "/sources/spotify/playlists"),
            (Post, "/sources/spotify/playlists/{id}/import"),
            (Post, "/sources/tidal/collection/import"),
            (Get, "/sources/tidal/playlists"),
            (Post, "/sources/tidal/playlists/{id}/import"),
            (Get, "/radio/stations"),
            (Put, "/radio/stations"),
            (Delete, "/radio/stations/{id}"),
            (Post, "/radio/play"),
            (Get, "/podcasts/subscriptions"),
            (Post, "/podcasts/subscriptions"),
            (Get, "/podcasts/subscriptions/{id}"),
            (Delete, "/podcasts/subscriptions/{id}"),
            (Get, "/podcasts/new"),
            (Post, "/podcasts/refresh"),
            (Get, "/podcasts/gpodder"),
            (Put, "/podcasts/gpodder"),
            (Delete, "/podcasts/gpodder"),
            (Post, "/podcasts/gpodder/sync"),
            (Post, "/podcasts/play"),
            (Put, "/podcasts/progress/{id}"),
            (Put, "/profiles/{id}/follow"),
            (Delete, "/profiles/{id}/follow"),
            (Get, "/me/listens"),
            (Delete, "/me/listens"),
            (Get, "/me/listens/stats"),
            (Get, "/me/listens/settings"),
            (Put, "/me/listens/settings"),
            (Post, "/me/listens/visibility"),
        ],
    ),
    (
        ZoneGroup,
        &[(Get, "/outputs"), (Post, "/zones"), (Delete, "/zones/{id}")],
    ),
    (
        ZoneManage,
        &[
            (Post, "/outputs/rescan"),
            (Post, "/outputs/upnp"),
            (Delete, "/outputs/upnp/{address}"),
            (Post, "/outputs/endpoints"),
            (Delete, "/outputs/endpoints/{address}"),
            (Post, "/outputs/pull"),
            (Put, "/outputs/{id}/device"),
            (Get, "/outputs/pull/{name}/url"),
            (Put, "/zones/{id}/crossfade"),
            (Put, "/zones/{id}/dsp"),
            (Delete, "/zones/{id}/dsp"),
        ],
    ),
    (
        LibraryManage,
        &[
            (Get, "/library/tracks/{id}/edit"),
            (Delete, "/library/tracks/{id}"),
            (Put, "/library/tracks/{id}/edit"),
            (Delete, "/library/tracks/{id}/edit"),
            (Put, "/library/albums/{id}/edit"),
            (Get, "/library/hidden"),
            (Post, "/library/hidden/restore"),
            (Post, "/library/folders/move"),
            (Get, "/library/tree"),
            (Get, "/library/loudness"),
            (Post, "/library/loudness/measure"),
            (Get, "/library/matches"),
            (Get, "/library/folders"),
            (Post, "/library/folders"),
            (Delete, "/library/folders"),
            (Get, "/library/folders/suggest"),
            (Post, "/library/scan"),
            (Get, "/library/identify"),
            (Post, "/library/identify"),
            (Put, "/library/identify/settings"),
            (Post, "/library/identify/key/check"),
            (Post, "/library/identify/write"),
            (Get, "/library/identify/fix"),
            (Get, "/library/tag-fixes"),
            (Post, "/library/tag-fixes"),
            (Get, "/library/suggestions"),
            (Get, "/library/suggestions/albums"),
            (Post, "/library/suggestions/confirm"),
            (Post, "/library/suggestions/skip"),
            (Post, "/library/suggestions/unskip"),
            (Get, "/library/tracks/{id}/candidates"),
            (Post, "/library/tracks/{id}/match"),
            (Post, "/library/suggestions/{id}/accept"),
            (Delete, "/library/suggestions/{id}"),
            (Get, "/storage"),
            (Post, "/storage/save"),
            (Get, "/library/uploads"),
            (Post, "/library/uploads"),
            (Get, "/library/uploads/{id}"),
            (Put, "/library/uploads/{id}"),
            (Delete, "/library/uploads/{id}"),
            (Post, "/library/imports"),
            (Get, "/library/webdav"),
            (Post, "/library/webdav"),
            (Get, "/library/drives"),
            (Post, "/library/drives"),
            (Post, "/library/drives/{provider}/login"),
            (Get, "/library/drives/{provider}/folders"),
            (Delete, "/library/drives/{provider}/account"),
            (Post, "/library/drives/roots/{id}/scan"),
            (Delete, "/library/drives/roots/{id}"),
            (Delete, "/library/webdav/{id}"),
            (Post, "/library/webdav/{id}/scan"),
            (Get, "/pubky/files"),
            (Post, "/pubky/files"),
            (Delete, "/pubky/files"),
            (Post, "/pubky/files/mirror"),
            (Post, "/pubky/files/upload"),
            (Get, "/pubky/files/upload/{id}"),
            (Post, "/pubky/files/upload/{id}/pause"),
            (Post, "/pubky/files/upload/{id}/resume"),
            (Post, "/pubky/files/upload/{id}/cancel"),
            (Post, "/pubky/files/upload/{id}/retry"),
            (Get, "/pubky/files/jobs"),
            (Delete, "/pubky/files/jobs/last"),
            (Get, "/pubky/files/jobs/{id}"),
            (Post, "/pubky/files/jobs/{id}/cancel"),
        ],
    ),
    (
        Settings,
        &[
            (Get, "/node"),
            (Get, "/node/apps"),
            (Put, "/node/apps/{id}"),
            (Get, "/node/settings"),
            (Put, "/node/settings"),
            (Put, "/sources/{source}/enabled"),
            (Get, "/sources/{source}/quality"),
            (Put, "/sources/{source}/quality"),
            (Post, "/sources/tidal/unofficial/client/check"),
            (Get, "/sources/youtube/key"),
            (Put, "/sources/youtube/key"),
            (Get, "/sources/youtube/unofficial"),
            (Put, "/sources/youtube/unofficial"),
            (Post, "/sources/youtube/unofficial/ytdlp"),
            (Delete, "/sources/youtube/unofficial/ytdlp"),
            (Post, "/sources/spotify/soloist/binary"),
            (Delete, "/sources/spotify/soloist/binary"),
            (Put, "/sources/spotify/app"),
        ],
    ),
    (
        ServiceLogins,
        &[
            (Get, "/sources/bandcamp/unofficial"),
            (Put, "/sources/bandcamp/unofficial"),
            (Get, "/sources/spotify/soloist"),
            (Put, "/sources/spotify/soloist"),
            (Post, "/sources/spotify/soloist/control"),
            (Get, "/sources/spotify/librespot"),
            (Put, "/sources/spotify/librespot"),
            (Get, "/sources/soundcloud/unofficial"),
            (Put, "/sources/soundcloud/unofficial"),
            (Get, "/sources/subsonic/server"),
            (Put, "/sources/subsonic/server"),
            (Delete, "/sources/subsonic/server"),
            (Get, "/sources/jellyfin/server"),
            (Put, "/sources/jellyfin/server"),
            (Delete, "/sources/jellyfin/server"),
            (Get, "/listenbrainz"),
            (Put, "/listenbrainz"),
            (Delete, "/listenbrainz"),
            (Get, "/lastfm"),
            (Put, "/lastfm"),
            (Delete, "/lastfm"),
            (Get, "/lastfm/login"),
            (Get, "/sources/plex/server"),
            (Put, "/sources/plex/server"),
            (Delete, "/sources/plex/server"),
            (Post, "/sources/plex/pin"),
            (Get, "/sources/plex/pin/{id}"),
            (Get, "/sources/audiobookshelf/server"),
            (Put, "/sources/audiobookshelf/server"),
            (Delete, "/sources/audiobookshelf/server"),
            (Post, "/sources/jamendo/login"),
            (Post, "/sources/jamendo/logout"),
        ],
    ),
    (
        People,
        &[
            (Delete, "/people/{key}/sign-ins"),
            (Delete, "/people/{key}/sign-ins/{id}"),
            (Get, "/people/zones"),
            (Put, "/people/zones"),
            (Get, "/people"),
            (Post, "/people/invites"),
            (Delete, "/people/invites/{id}"),
            (Patch, "/people/{key}"),
            (Delete, "/people/{key}"),
        ],
    ),
    (
        Tokens,
        &[
            (Post, "/node/token"),
            (Get, "/node/app-passwords"),
            (Post, "/node/app-passwords"),
            (Delete, "/node/app-passwords/{id}"),
        ],
    ),
    (
        Vault,
        &[
            (Get, "/vault"),
            (Post, "/vault/register"),
            (Post, "/vault/sync"),
            (Post, "/vault/devices/{key}/sign-out"),
            (Post, "/vault/sign-out-everywhere"),
            (Post, "/vault/new-key"),
        ],
    ),
    (
        Backup,
        &[
            (Post, "/node/settings/backup"),
            (Post, "/node/settings/restore"),
            (Get, "/backup"),
            (Post, "/backup"),
            (Post, "/restore"),
            (Get, "/backup/schedule"),
            (Put, "/backup/schedule"),
            (Post, "/backup/schedule/run"),
        ],
    ),
    (
        Identity,
        &[
            (Get, "/homeserver/bundled"),
            (Post, "/homeserver/bundled/owner-signup"),
            (Post, "/auth/pubky/renewal"),
            (Get, "/pubky"),
            (Post, "/pubky/sync"),
            (Post, "/pubky/login"),
            (Get, "/pubky/login"),
        ],
    ),
    (
        RemoteAccess,
        &[
            (Get, "/homeserver/bundled/tunnel"),
            (Put, "/homeserver/bundled/tunnel"),
            (Delete, "/homeserver/bundled/tunnel"),
            (Get, "/remote/tunnel"),
            (Put, "/remote/tunnel"),
            (Delete, "/remote/tunnel"),
        ],
    ),
    (
        Party,
        &[(Get, "/party"), (Post, "/party"), (Delete, "/party/{id}")],
    ),
    (
        Diagnostics,
        &[
            (Get, "/node/usage"),
            (Put, "/node/usage"),
            (Post, "/node/usage/clear"),
            (Get, "/node/usage/summary"),
            (Get, "/node/usage/export"),
            (Get, "/outputs/discovery"),
        ],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::collections::{BTreeSet, HashSet};
    use tower::ServiceExt;

    const LISTENER: Caller = Caller {
        role: Role::Listener,
        key: None,
    };
    const GUEST: Caller = Caller {
        role: Role::Guest,
        key: None,
    };
    const APP: Caller = Caller {
        role: Role::App,
        key: None,
    };

    /// A route with every parameter filled in (`pick` names the value for a parameter).
    fn filled(route: &str, pick: &dyn Fn(&str) -> String) -> String {
        route
            .split('/')
            .map(
                |seg| match seg.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                    Some(name) => pick(name),
                    None => seg.to_string(),
                },
            )
            .collect::<Vec<_>>()
            .join("/")
    }

    /// The router's own `/v1` routes and methods are exactly the ones the table lists: a new
    /// route, or a new method on one, fails here until it names its capability, and a line for a
    /// route that is gone fails too.
    #[tokio::test]
    async fn every_route_names_its_capability() {
        let dir = tempfile::tempdir().unwrap();
        let s = crate::tests::state(dir.path(), Some("s3cret")).await;
        let (routes, paths) = crate::v1_routes().into_parts();
        // The routes alone, without the sign-in check in front: no handler runs below
        let app = routes.with_state(s);
        // Each path once (the router itself refuses a path added twice)
        assert_eq!(
            paths.iter().collect::<HashSet<_>>().len(),
            paths.len(),
            "a /v1 path is added twice"
        );
        // What the router serves: asked with a method no route has, each path answers 405 and
        // names the methods it does have (`Allow`)
        let probe = Method::from_bytes(b"PROBE").unwrap();
        let mut served = BTreeSet::new();
        for path in &paths {
            let uri = filled(path, &|_| "1".into());
            let req = Request::builder()
                .method(probe.clone())
                .uri(&uri)
                .body(Body::empty())
                .unwrap();
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED, "{uri}");
            let allow = resp
                .headers()
                .get(axum::http::header::ALLOW)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_else(|| panic!("{uri} names no methods"))
                .to_string();
            for name in allow.split(',').map(str::trim) {
                let method = Method::from_bytes(name.as_bytes()).unwrap();
                // HEAD is the GET handler's
                if method == Method::HEAD {
                    continue;
                }
                let verb = Verb::of(&method)
                    .unwrap_or_else(|| panic!("{name} {path}: a method the table has no word for"));
                served.insert((format!("{verb:?}"), path.to_string()));
            }
        }
        let mut listed = BTreeSet::new();
        for (cap, routes) in ROUTES {
            for (verb, route) in *routes {
                assert!(
                    listed.insert((format!("{verb:?}"), route.to_string())),
                    "{verb:?} {route} is listed twice (again under {cap:?})"
                );
            }
        }
        let unlisted: Vec<_> = served.difference(&listed).collect();
        assert!(
            unlisted.is_empty(),
            "routes without a capability: add each to access::ROUTES under the capability it needs: {unlisted:?}"
        );
        let gone: Vec<_> = listed.difference(&served).collect();
        assert!(
            gone.is_empty(),
            "access::ROUTES lists routes the router doesn't have: {gone:?}"
        );
        assert_eq!(listed.len(), table().len());
        // Every capability is one of ALL, and each is used
        let used: HashSet<Capability> = ROUTES.iter().map(|(cap, _)| *cap).collect();
        assert_eq!(used.len(), ROUTES.len(), "a capability has two blocks in ROUTES");
        for cap in Capability::ALL {
            assert!(used.contains(cap), "{cap:?} is on no route");
        }
        assert_eq!(used.len(), Capability::ALL.len());
    }

    /// What a decision reads from the path is there on every route that needs it.
    #[test]
    fn routes_that_depend_on_the_path_name_what_they_depend_on() {
        for (cap, routes) in ROUTES {
            for (_, route) in *routes {
                let zone = *route == "/zones/{id}" || route.starts_with("/zones/{id}/");
                match cap {
                    Capability::ZoneUse => assert!(zone, "{route} needs a zone in its path"),
                    // A zone route that isn't ZoneUse is the owner's (adding, removing, tuning), or
                    // ungrouping one
                    _ if zone => assert!(
                        matches!(cap, Capability::ZoneManage | Capability::ZoneGroup),
                        "{route}"
                    ),
                    _ => {}
                }
                if route.contains("{source}") {
                    assert!(
                        route.starts_with("/sources/{source}/"),
                        "{route}: a source parameter elsewhere isn't read"
                    );
                }
            }
        }
    }

    /// The owner has every capability; a listener, an app password and a guest exactly the ones
    /// the module's table says; a route without a capability is the owner's alone.
    #[tokio::test]
    async fn roles_have_what_the_table_says() {
        use Capability::*;
        let dir = tempfile::tempdir().unwrap();
        let s = crate::tests::state(dir.path(), Some("s3cret")).await;
        let owner = Caller {
            role: Role::Owner,
            key: None,
        };
        let listener: HashSet<Capability> = [
            Artwork,
            Session,
            Library,
            ZoneList,
            ZoneUse,
            Collections,
            Own,
            OwnPlay,
            OwnSignInState,
            OwnSignIn,
        ]
        .into();
        let app: HashSet<Capability> = [
            Artwork,
            Session,
            Library,
            ZoneList,
            ZoneUse,
            ZoneGroup,
            Collections,
            OwnPlay,
        ]
        .into();
        let guest: HashSet<Capability> = [Artwork, PartyGuest].into();
        for cap in Capability::ALL {
            assert!(Role::Owner.has(*cap) && owner.can(*cap), "{cap:?}");
            assert_eq!(Role::Listener.has(*cap), listener.contains(cap), "{cap:?}");
            assert_eq!(Role::App.has(*cap), app.contains(cap), "{cap:?}");
            assert_eq!(Role::Guest.has(*cap), guest.contains(cap), "{cap:?}");
            assert_eq!(decide(&s, &owner, Some(*cap), &At::default()), None);
            if !guest.contains(cap) {
                assert_eq!(decide(&s, &GUEST, Some(*cap), &At::default()), Some(GUESTS));
            }
            // An app password on any zone, with any source: it plays as the owner
            let anywhere = At {
                zone: Some("any-zone"),
                source: Some("plex"),
                names_source: true,
            };
            let want = (!app.contains(cap)).then_some(APPS);
            assert_eq!(decide(&s, &APP, Some(*cap), &anywhere), want, "{cap:?}");
        }
        assert_eq!(decide(&s, &owner, None, &At::default()), None);
        assert_eq!(decide(&s, &LISTENER, None, &At::default()), Some(OWNERS));
        assert_eq!(decide(&s, &APP, None, &At::default()), Some(OWNERS));
        assert_eq!(decide(&s, &GUEST, None, &At::default()), Some(OWNERS));
        // Nothing an app password may do changes the node, its people or anyone's sign-ins
        for cap in [
            Settings,
            People,
            Tokens,
            ServiceLogins,
            Own,
            OwnSignIn,
            OwnSignInState,
            LibraryManage,
            ZoneManage,
            Backup,
            Vault,
            Identity,
            RemoteAccess,
            OwnerMusic,
            Diagnostics,
            Party,
        ] {
            assert!(!Role::App.has(cap), "{cap:?}");
        }
        // A path no route lists, and a method a route doesn't have
        assert_eq!(decide_path(&s, &owner, &Method::GET, "/v1/nothing/here"), None);
        assert!(decide_path(&s, &LISTENER, &Method::GET, "/v1/nothing/here").is_some());
        assert!(decide_path(&s, &LISTENER, &Method::DELETE, "/v1/library/albums").is_some());
        assert!(decide_path(&s, &LISTENER, &Method::OPTIONS, "/v1/library/albums").is_some());
    }

    /// `docs/access.md` says what the code does: one row per capability, in the enum's order,
    /// and each role's column is filled exactly where [`Role::has`] says yes.
    #[test]
    fn the_access_doc_matches_the_roles() {
        let doc = include_str!("../../../docs/access.md");
        let rows: Vec<Vec<&str>> = doc
            .lines()
            .skip_while(|l| *l != "## What each role can do")
            .skip(1)
            .take_while(|l| !l.starts_with("## "))
            .filter(|l| l.starts_with("| `"))
            .map(|l| l.trim_matches('|').split('|').map(str::trim).collect())
            .collect();
        let named: Vec<&str> = rows.iter().map(|r| r[0].trim_matches('`')).collect();
        let caps: Vec<String> = Capability::ALL
            .iter()
            .map(|c| serde_json::to_value(c).unwrap().as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            named, caps,
            "docs/access.md lists the capabilities of access.rs, in its order"
        );
        for (row, cap) in rows.iter().zip(Capability::ALL) {
            assert_eq!(
                row.len(),
                6,
                "{cap:?}: capability, what it covers, owner, listener, app, guest"
            );
            for (cell, role) in row[2..]
                .iter()
                .zip([Role::Owner, Role::Listener, Role::App, Role::Guest])
            {
                assert_eq!(!cell.is_empty(), role.has(*cap), "{cap:?} for {role:?}: `{cell}`");
            }
        }
        // And it names every route that takes no sign-in
        for (path, _) in OPEN {
            assert!(
                doc.contains(&format!("`{path}`")),
                "docs/access.md doesn't name {path}"
            );
        }
    }

    /// The routes that take no sign-in are the listed ones ([`OPEN`]): one added without a line
    /// there fails here, as a `/v1` route without a capability does.
    #[test]
    fn the_routes_without_a_sign_in_are_the_listed_ones() {
        let open = crate::open_routes();
        let served: BTreeSet<&str> = open.paths().iter().copied().collect();
        assert_eq!(served.len(), open.paths().len(), "an open path is added twice");
        let listed: BTreeSet<&str> = OPEN.iter().map(|(path, _)| *path).collect();
        assert_eq!(listed.len(), OPEN.len(), "an open path is listed twice");
        assert_eq!(
            served, listed,
            "a route outside the sign-in check: list it in access::OPEN with why it needs no sign-in, \
             or add it to v1_routes() with a capability"
        );
        // None of them is also a route behind the check
        for (path, why) in OPEN {
            assert!(!why.is_empty(), "{path}");
            if let Some(route) = path.strip_prefix("/v1") {
                assert!(
                    table().keys().all(|(_, listed)| *listed != route),
                    "{path} is open and also names a capability"
                );
            }
        }
    }

    /// The sources each person signs in to on their own are the ones the sign-in code says
    /// ([`PER_PERSON`] and `SignInSource::per_person`), and nothing else passes for one.
    #[test]
    fn the_per_person_sources_are_the_listed_ones() {
        for id in [
            "tidal",
            "qobuz",
            "spotify",
            "youtube",
            "lastfm",
            "deezer",
            "soundcloud",
            "bandcamp",
            "jamendo",
            "subsonic",
            "jellyfin",
            "plex",
            "audiobookshelf",
            "librespot",
            "",
            "TIDAL",
            "tidal ",
        ] {
            assert_eq!(crate::signin::per_person(id), PER_PERSON.contains(&id), "{id:?}");
        }
    }

    #[test]
    fn a_path_finds_its_route_as_the_router_does() {
        let route = |p: &str| route_of(p).map(|(r, _)| r);
        assert_eq!(route("/v1/zones/kitchen/queue"), Some("/zones/{id}/queue"));
        assert_eq!(route("/zones/kitchen/queue"), Some("/zones/{id}/queue"));
        // A fixed segment wins over a parameter, whatever the method: the import route has no
        // GET, so a GET there needs a capability no one names (the owner gets the router's 405)
        assert_eq!(route("/playlists/import"), Some("/playlists/import"));
        assert_eq!(required(&Method::GET, "/playlists/import"), None);
        assert!(required(&Method::POST, "/playlists/import").is_some());
        assert_eq!(route("/playlists/7"), Some("/playlists/{id}"));
        assert_eq!(route("/sources/tidal/status"), Some("/sources/tidal/status"));
        assert_eq!(route("/sources/tidal/signin"), Some("/sources/{source}/signin"));
        assert_eq!(route("/collections/"), None);
        assert_eq!(route("/collectionsx"), None);
        assert_eq!(route("/zones/a/b/c/d"), None);
        let (_, params) = route_of("/v1/sources/qobuz/signin").unwrap();
        assert_eq!(params, vec![("source", "qobuz".to_string())]);
        let at = At::of(
            "/v1/zones/{id}/queue/{item}",
            [("id", "kitchen"), ("item", "3")].into_iter(),
        );
        assert_eq!(
            (at.zone, at.source, at.names_source),
            (Some("kitchen"), None, false)
        );
        // A source route whose source could not be read still says it names one: refused
        let at = At::of("/v1/sources/{source}/signin", std::iter::empty());
        assert_eq!((at.source, at.names_source), (None, true));
        for cap in [
            Capability::OwnPlay,
            Capability::OwnSignInState,
            Capability::OwnSignIn,
        ] {
            assert!(Role::Listener.has(cap));
        }
        // `{id}` is a zone only on a zone's own routes
        assert_eq!(
            At::of("/library/albums/{id}/cover", [("id", "7")].into_iter()).zone,
            None
        );
    }

    /// The table gives listeners and party guests what the path lists gave them, for every route
    /// and method, under each source policy and zone list, with the owner's zone list set, unset
    /// and unreadable. The four places where it differs on purpose are named in the loop.
    #[tokio::test]
    async fn listeners_and_guests_reach_what_they_did() {
        let dir = tempfile::tempdir().unwrap();
        let s = crate::tests::state(dir.path(), Some("s3cret")).await;
        let set = |key: &str, value: Option<&str>| {
            let lib = s.library.lock().unwrap();
            match value {
                Some(v) => lib.set_setting(key, v).unwrap(),
                None => {
                    lib.delete_setting(key).unwrap();
                }
            }
        };
        let mut compared = 0;
        for policy in [None, Some("node"), Some("own"), Some("choice")] {
            set(crate::accounts::SOURCE_POLICY_KEY, policy);
            for zones in [None, Some(r#"["open"]"#), Some("not a list")] {
                set("accounts.listener_zones", zones);
                for (cap, routes) in ROUTES {
                    for (verb, route) in *routes {
                        let method = verb.method();
                        // Each zone and source where the route names one
                        let zone_ids: &[&str] = if route.starts_with("/zones/{id}") {
                            &["open", "shut"]
                        } else {
                            &["-"]
                        };
                        let source_ids: &[&str] = if route.contains("{source}") {
                            &["tidal", "qobuz", "spotify", "deezer", "youtube", "lastfm"]
                        } else {
                            &["-"]
                        };
                        for zone in zone_ids {
                            for source in source_ids {
                                let path = filled(route, &|name| match name {
                                    "source" => source.to_string(),
                                    "id" if route.starts_with("/zones/") => zone.to_string(),
                                    _ => "1".to_string(),
                                });
                                let ctx =
                                    format!("{verb:?} {path} ({cap:?}; policy {policy:?}, zones {zones:?})");
                                let was = before::listener_refusal(&s, &method, &path);
                                let now = decide_path(&s, &LISTENER, &method, &path);
                                // Where the table differs from the path lists on purpose:
                                // the owner's own files and a song's saved edit passed the lists
                                // and were refused by their handlers; they are refused here now
                                let closed = matches!(
                                    (verb, *route),
                                    (Verb::Get, "/me/music") | (Verb::Get, "/library/tracks/{id}/edit")
                                );
                                // Signing out of their own stays possible while the node plays
                                // through its own sign-ins: these two were left out of that
                                let sign_out = policy == Some("node")
                                    && matches!(
                                        (verb, *route),
                                        (Verb::Delete, "/sources/tidal/unofficial/login")
                                            | (Verb::Delete, "/sources/qobuz/unofficial/login")
                                    );
                                if closed {
                                    assert!(was.is_none() && now.is_some(), "listener: {ctx}");
                                } else if sign_out {
                                    assert_eq!(was, Some(crate::accounts::NODE_SOURCES_ONLY), "{ctx}");
                                    assert_eq!(now, None, "listener: {ctx}");
                                } else {
                                    assert_eq!(
                                        was.is_some(),
                                        now.is_some(),
                                        "listener: {ctx}: was {was:?}, now {now:?}"
                                    );
                                    // The refusals that say what to do about them keep their words
                                    if was.is_some_and(|w| {
                                        w == crate::accounts::NODE_SOURCES_ONLY || w.contains("zone")
                                    }) {
                                        assert_eq!(was, now, "listener: {ctx}");
                                    }
                                }
                                let was = before::guest_refusal(&method, &path);
                                let now = guest_refusal(&method, &path);
                                assert_eq!(was.is_some(), now.is_some(), "guest: {ctx}");
                                assert_eq!(now, decide_path(&s, &GUEST, &method, &path), "guest: {ctx}");
                                compared += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(compared > 12 * 445, "{compared}");
    }

    /// The rules as they were before this module: path prefix lists read on the request's raw
    /// path (`accounts::listener_refusal`, `personal_collections::is_own_route`,
    /// `party::guest_refusal`), kept word for word as the reference the table is checked against
    /// (`listeners_and_guests_reach_what_they_did`).
    mod before {
        use crate::accounts::SourcePolicy;
        use crate::state::NodeState;
        use axum::http::Method;

        /// Routes a listener may never use (reads included): the owner's settings and data.
        const OWNER_ONLY: &[&str] = &[
            // The owner's listen history (under /me, which a listener may otherwise use)
            "/me/listens",
            "/people",
            "/vault",
            "/party",
            "/remote",
            "/homeserver",
            "/node",
            "/pubky",
            "/backup",
            "/storage",
            "/library/webdav",
            "/library/uploads",
            "/library/imports",
            "/library/folders",
            "/library/tree",
            "/library/scan",
            "/library/tag-fixes",
            "/outputs",
        ];

        /// What a listener may read: the shared library, public catalogues and directories, their
        /// zones, who they are and their own bookmarks (bookmarks.rs answers each person's own).
        /// Everything else (the owner's bookmarks, playlists, tags, reviews, history, podcast
        /// subscriptions and streaming accounts) stays the owner's.
        const LISTENER_READS: &[&str] = &[
            "/me",
            "/bookmarks",
            // The same, by its older name (the page and the offline app's sync still use it)
            "/favourites",
            "/events",
            "/info",
            "/status",
            "/health",
            "/library/albums",
            "/library/artists",
            "/library/tracks",
            "/library/genres",
            "/library/search",
            "/library/stats",
            "/lyrics",
            "/search",
            "/item",
            "/catalog/search",
            "/discover",
            "/shared",
            "/profiles",
            "/mb",
            "/radio/countries",
            "/radio/tags",
            "/radio/search",
            "/radio/curated",
            "/podcasts/search",
            "/zones",
            "/devices",
            "/presence",
        ];

        /// A listener's own streaming sign-ins and plays (TASK-per-person-service-accounts): each of these
        /// acts on the caller's own account, reads included; the apps and clients behind them stay the
        /// owner's (the handlers refuse those fields, and [`LISTENER_OWN_EXCEPTIONS`] those routes).
        const LISTENER_OWN: &[&str] = &[
            // Their own problem reports (feedback.rs keeps each person to theirs)
            "/feedback",
            // Usage insights: their own choice, aliases and events (usage.rs checks the person itself)
            "/usage",
            "/play-order",
            "/sources/tidal/signin",
            "/sources/qobuz/signin",
            "/sources/spotify/signin",
            "/sources/tidal/login",
            "/sources/tidal/status",
            "/sources/tidal/session",
            "/sources/tidal/callback",
            "/sources/tidal/unofficial",
            "/sources/tidal/stream",
            "/sources/qobuz/unofficial",
            "/sources/qobuz/stream",
            "/sources/spotify/login",
            "/sources/spotify/status",
            "/sources/spotify/session",
            "/sources/spotify/callback",
            "/sources/spotify/devices",
            "/sources/spotify/play",
        ];

        /// Inside [`LISTENER_OWN`], what goes on under every source policy: plays, streams and the play order.
        const LISTENER_OWN_ALWAYS: &[&str] = &[
            // Problem reports aren't a sign-in either: a listener reports and sees their own whatever
            // plays through whose sign-ins (feedback.rs keeps each person to theirs)
            "/feedback",
            // Usage insights aren't a sign-in: a listener's own choice, Stop for me and Clear mine work
            // whatever plays through whose sign-ins (usage.rs checks the person)
            "/usage",
            "/play-order",
            "/sources/tidal/stream",
            "/sources/qobuz/stream",
            "/sources/spotify/devices",
            "/sources/spotify/play",
        ];

        /// Inside [`LISTENER_OWN`], a listener's own sign-in state, which they may still read (GET) when the
        /// node plays through its own sign-ins.
        const LISTENER_OWN_STATUS: &[&str] = &[
            "/sources/tidal/signin",
            "/sources/qobuz/signin",
            "/sources/spotify/signin",
            "/sources/tidal/status",
            "/sources/spotify/status",
            "/sources/tidal/session",
            "/sources/spotify/session",
            "/sources/tidal/unofficial",
            "/sources/qobuz/unofficial",
        ];

        /// Inside [`LISTENER_OWN`], still the owner's: checking the node's TIDAL device client.
        const LISTENER_OWN_EXCEPTIONS: &[&str] = &["/sources/tidal/unofficial/client"];

        /// Inside [`LISTENER_READS`], still the owner's: identification candidates and suggestions.
        const LISTENER_READ_EXCEPTIONS: &[&str] = &["/candidates", "/suggestions"];

        fn under(path: &str, prefixes: &[&str]) -> bool {
            prefixes
                .iter()
                .any(|p| path == *p || path.starts_with(&format!("{p}/")))
        }

        /// Whether a listener may make this request. `None` = yes; `Some(why)` = no.
        pub(super) fn listener_refusal(s: &NodeState, method: &Method, path: &str) -> Option<&'static str> {
            // Inside the `/v1` router the prefix is gone; take either
            let path = path.strip_prefix("/v1").unwrap_or(path);
            if under(path, OWNER_ONLY) {
                return Some("that is the owner's");
            }
            // Their own streaming accounts
            if under(path, LISTENER_OWN) {
                if under(path, LISTENER_OWN_EXCEPTIONS) {
                    return Some("that is the owner's");
                }
                // The node plays through its own sign-ins: a listener can't sign in to their own (what
                // they had stays; reads of their own status, plays and streams go on)
                if !under(path, LISTENER_OWN_ALWAYS)
                    && !(*method == Method::GET && LISTENER_OWN_STATUS.contains(&path))
                    // Signing out of their own stays possible
                    && !(*method == Method::DELETE && LISTENER_OWN_STATUS.contains(&path))
                    && crate::accounts::source_policy(&s.library.lock().expect("library poisoned")) == SourcePolicy::Node
                {
                    return Some(crate::accounts::NODE_SOURCES_ONLY);
                }
                return None;
            }
            // Their own collections, on their own homeserver (personal_collections.rs answers each
            // person's); the rest of /collections and /playlists stays the owner's
            if is_own_route(method, path) {
                return None;
            }
            if *method == Method::GET || *method == Method::HEAD {
                if !under(path, LISTENER_READS) || LISTENER_READ_EXCEPTIONS.iter().any(|x| path.contains(x)) {
                    return Some("that is the owner's");
                }
                // A zone's state and queue: only the zones open to listeners (the list itself is fine)
                if let Some(rest) = path.strip_prefix("/zones/") {
                    return crate::accounts::zone_refusal(s, rest.split('/').next().unwrap_or(rest));
                }
                return None;
            }
            // Playing on a zone the owner allows: its queue, transport, volume and the like
            if let Some(rest) = path.strip_prefix("/zones/") {
                let (zone, what) = rest.split_once('/').unwrap_or((rest, ""));
                if what.is_empty() || what.starts_with("dsp") || what == "crossfade" {
                    return Some("only the owner adds, removes or tunes zones");
                }
                return crate::accounts::zone_refusal(s, zone);
            }
            // The page asks who is signed in, and a listener may sign out
            if path == "/me" || path.starts_with("/me/") {
                return None;
            }
            // Their own phones and laptops and what those keep offline (devices.rs checks the owner)
            if path.starts_with("/devices/") {
                return None;
            }
            // Listening along (presence.rs checks the zones)
            if path == "/presence/along" {
                return None;
            }
            // Their own bookmarks, on their own homeserver (bookmarks.rs answers each person's)
            if path == "/bookmarks" || path == "/favourites" {
                return None;
            }
            Some("that is the owner's: a listener's own are their bookmarks and collections")
        }

        /// Whether a listener's request is one of the routes above, for their own collections
        /// (accounts.rs lets exactly these through).
        fn is_own_route(method: &axum::http::Method, path: &str) -> bool {
            use axum::http::Method;
            let Some(rest) = path
                .strip_prefix("/collections")
                .or_else(|| path.strip_prefix("/playlists"))
            else {
                return false;
            };
            let parts: Vec<&str> = rest.split('/').skip(1).collect();
            match (method, parts.as_slice()) {
                (&Method::GET | &Method::POST, []) => rest.is_empty(),
                (&Method::GET | &Method::PUT | &Method::DELETE, [id]) => {
                    !id.is_empty() && !RESERVED.contains(id)
                }
                (&Method::POST, [id, "items"]) | (&Method::POST, [id, "play"]) => !id.is_empty(),
                (&Method::DELETE, [id, "items", index]) => !id.is_empty() && index.parse::<usize>().is_ok(),
                (&Method::PUT, [id, "visibility"]) => !id.is_empty(),
                _ => false,
            }
        }

        /// Names under `/playlists/` that aren't collections.
        const RESERVED: &[&str] = &["from-queue", "import"];

        /// Whether a guest may make this request: only the party routes and covers.
        pub(super) fn guest_refusal(method: &axum::http::Method, path: &str) -> Option<&'static str> {
            let path = path.strip_prefix("/v1").unwrap_or(path);
            if path == "/party/guest" || path.starts_with("/party/guest/") {
                return None;
            }
            let is_cover = path
                .strip_prefix("/library/albums/")
                .and_then(|r| r.strip_suffix("/cover"))
                .is_some_and(|id| id.parse::<i64>().is_ok());
            if is_cover && (*method == axum::http::Method::GET || *method == axum::http::Method::HEAD) {
                return None;
            }
            Some("a party guest can only add songs to the party")
        }
    }
}
