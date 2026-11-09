//! Qobuz (owner, 2026-10-22: "add qobuz same way as tidal"; STEWARD-DUAL-MODE.md). Qobuz has no
//! open API, so the source is unofficial-only: it uses the private API Qobuz's own players use,
//! opt-in behind a warning, and can stop working at any time.
//!
//! - **App:** the current web player's app ID and signing secret, discovered automatically and
//!   cached for the whole node. `[qobuz] app_id` / `app_secret` and `PM_QOBUZ_APP_ID` / `_SECRET`
//!   are silent admin overrides and suppress discovery.
//! - **Sign-in:** each person's Qobuz email and password, sent once to `user/login`; the node keeps
//!   only the session token Qobuz answers with (a sign-in: a backup carries it only under the
//!   passphrase). Accounts without a streaming plan are refused there.
//! - **Streams:** `track/getFileUrl` (signed with the app secret) answers a plain FLAC URL, up to
//!   24-bit/192 kHz, read straight into the decoder. A sample (30 s) answer is refused, not played.
//! - **Health:** a failure of the mode itself (refused app, ended session) switches it off and the
//!   page says why, with Retry. Qobuz links still open on qobuz.com and match through Pinakes.

use crate::accounts::Who;
use crate::state::NodeState;
use crate::streaming::StreamingService;
use crate::types::*;
use crate::{ApiError, ApiResult};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};

mod auto;

/// Points discovery at a mock web player and forgets what the last discovery found.
#[cfg(test)]
pub(crate) fn set_player(url: &str) {
    auto::set_player(url);
    forget_discovery();
}

#[cfg(test)]
pub(crate) fn forget_discovery() {
    *DISCOVERY.lock().expect("Qobuz discovery") = Discovery::NONE;
}

/// Moves the discovery clock on by `by`, so the last discovery looks that much older.
#[cfg(test)]
pub(crate) fn age_discovery(by: Duration) {
    *CLOCK_SKEW.lock().expect("Qobuz clock") += by;
}

#[cfg(test)]
static CLOCK_SKEW: std::sync::Mutex<Duration> = std::sync::Mutex::new(Duration::ZERO);

pub const SOURCE_ID: &str = "qobuz";
pub const TRACK_URI_PREFIX: &str = "qobuz:track:";
pub const STATE_KEY: &str = "qobuz.unofficial";
pub const APP_KEY: &str = "qobuz.app";
const API_BASE: &str = "https://www.qobuz.com/api.json/0.2";
/// What the page says before the mode is switched on.
pub const WARNING: &str = "This uses Qobuz's private API, which Qobuz has not approved for other apps. It can stop working at any time, and Qobuz could act on the account. It needs a paid Qobuz plan.";

pub fn track_uri(id: &str) -> String {
    format!("{TRACK_URI_PREFIX}{id}")
}

/// `https://open.qobuz.com/{kind}/{id}`: the service's own page.
pub fn page_url(kind: &str, id: &str) -> String {
    format!("https://open.qobuz.com/{kind}/{id}")
}

#[cfg(test)]
fn base_override() -> &'static std::sync::RwLock<Option<String>> {
    static B: std::sync::OnceLock<std::sync::RwLock<Option<String>>> = std::sync::OnceLock::new();
    B.get_or_init(|| std::sync::RwLock::new(None))
}

#[cfg(test)]
pub fn set_api_base(url: &str) {
    *base_override().write().expect("qobuz base") = Some(url.trim_end_matches('/').to_string());
}

#[cfg(test)]
pub(crate) static API_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn api_base() -> String {
    #[cfg(test)]
    if let Some(b) = base_override().read().expect("qobuz base").clone() {
        return b;
    }
    API_BASE.into()
}

fn now() -> i64 {
    pm_library::db::now_unix()
}

fn upstream(e: impl std::fmt::Display) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, format!("Qobuz: {e}"))
}

fn numeric(id: &str) -> Result<&str, ApiError> {
    if id.is_empty() || id.len() > 20 || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ApiError::bad_request("bad Qobuz id"));
    }
    Ok(id)
}

/// Album ids are letters and digits (`0060253780968`, `xu5ypgnw3ugzb`).
fn album_id(id: &str) -> Result<&str, ApiError> {
    if id.is_empty() || id.len() > 40 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(ApiError::bad_request("bad Qobuz album id"));
    }
    Ok(id)
}

// ---- state ---------------------------------------------------------------------------------------

#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Session {
    pub token: String,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// The plan's name as Qobuz labels it (`Studio`, `Sublime`, …).
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    /// The plan streams hi-res (else 16-bit/44.1 kHz at most).
    #[serde(default)]
    pub hires: bool,
    /// Kept only so sessions stored by older versions still load; sign-ins made now leave it empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("token", &if self.token.is_empty() { "" } else { "<set>" })
            .field("user_id", &self.user_id)
            .field("name", &self.name)
            .field("plan", &self.plan)
            .field("country", &self.country)
            .field("hires", &self.hires)
            .field("from", &self.from)
            .finish()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Health {
    pub ok: bool,
    pub detail: String,
    pub at: i64,
}

/// What the node keeps for this source.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModeState {
    #[serde(default)]
    pub enabled: bool,
    /// `max` (up to 24-bit/192 kHz) or `cd` (16-bit/44.1 kHz).
    #[serde(default)]
    pub quality: String,
    #[serde(default)]
    pub session: Option<Session>,
    #[serde(default)]
    pub health: Option<Health>,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct App {
    pub id: String,
    pub secret: String,
    #[serde(default)]
    pub from: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_url: Option<String>,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("id", &self.id)
            .field("secret", &if self.secret.is_empty() { "" } else { "<set>" })
            .field("from", &self.from)
            .field("bundle_url", &self.bundle_url)
            .finish()
    }
}

/// This person's Qobuz: their own sign-in, switch, quality and health.
pub fn load(lib: &pm_library::Library, who: &Who) -> ModeState {
    who.get(lib, STATE_KEY)
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

fn save(lib: &pm_library::Library, who: &Who, st: &ModeState) {
    let _ = who.put(
        lib,
        STATE_KEY,
        Some(&serde_json::to_string(st).expect("state serializes")),
    );
}

pub(crate) fn update(s: &NodeState, who: &Who, f: impl FnOnce(&mut ModeState)) {
    let lib = s.library.lock().expect("library poisoned");
    let mut st = load(&lib, who);
    f(&mut st);
    save(&lib, who, &st);
}

/// [`update`] for an interactive sign-in: the state and the vault's "new sign-in" mark (the
/// owner's only: the vault is theirs) are written under one library guard, so no vault pass sees
/// one without the other.
pub(crate) fn update_signed_in(s: &NodeState, who: &Who, service: &str, f: impl FnOnce(&mut ModeState)) {
    let lib = s.library.lock().expect("library poisoned");
    let mut st = load(&lib, who);
    f(&mut st);
    save(&lib, who, &st);
    if who.is_owner() {
        crate::vault_cache::new_sign_in(&lib, service);
    }
}

fn config_app() -> &'static std::sync::OnceLock<App> {
    static C: std::sync::OnceLock<App> = std::sync::OnceLock::new();
    &C
}

#[cfg(test)]
static TEST_CONFIG_APP: std::sync::RwLock<Option<App>> = std::sync::RwLock::new(None);

#[cfg(test)]
pub(crate) fn set_test_config_app(app: Option<App>) {
    *TEST_CONFIG_APP.write().expect("Qobuz test config") = app;
}

/// An admin override counts only with both values; one alone is ignored with a warning.
fn override_app(id: &str, secret: &str) -> Option<App> {
    let (id, secret) = (id.trim(), secret.trim());
    if id.is_empty() || secret.is_empty() {
        if !id.is_empty() || !secret.is_empty() {
            tracing::warn!("Qobuz: the app ID override needs both app_id and app_secret; ignored");
        }
        return None;
    }
    Some(App {
        id: id.into(),
        secret: secret.into(),
        from: "config".into(),
        bundle_url: None,
    })
}

/// The silent admin override from node.toml (`[qobuz] app_id` / `app_secret`).
pub fn set_config_app(id: &str, secret: &str) {
    if let Some(app) = override_app(id, secret) {
        let _ = config_app().set(app);
    }
}

/// `PM_QOBUZ_APP_ID` / `PM_QOBUZ_APP_SECRET`, read once.
fn env_app() -> Option<App> {
    static E: std::sync::OnceLock<Option<App>> = std::sync::OnceLock::new();
    E.get_or_init(|| {
        override_app(
            &std::env::var("PM_QOBUZ_APP_ID").unwrap_or_default(),
            &std::env::var("PM_QOBUZ_APP_SECRET").unwrap_or_default(),
        )
    })
    .clone()
}

fn configured_app() -> Option<App> {
    #[cfg(test)]
    if let Some(a) = TEST_CONFIG_APP.read().expect("Qobuz test config").clone() {
        return Some(a);
    }
    config_app().get().cloned().or_else(env_app)
}

fn cached_app(lib: &pm_library::Library) -> Option<App> {
    lib.get_setting(APP_KEY)
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str(&v).ok())
        .filter(|a: &App| !a.id.is_empty() && !a.secret.is_empty())
}

fn cache_app(lib: &pm_library::Library, app: &App) -> Result<(), ApiError> {
    lib.set_setting(APP_KEY, &serde_json::to_string(app).expect("app serializes"))?;
    Ok(())
}

/// The app id the node's Qobuz sign-in is made with (`vault_cache`).
pub(crate) fn app_id(s: &NodeState) -> Option<String> {
    app_of(&s.library.lock().expect("library poisoned")).map(|a| a.id)
}

/// The admin override wins; otherwise the last web-player app is reused.
pub fn app_of(lib: &pm_library::Library) -> Option<App> {
    configured_app().or_else(|| cached_app(lib))
}

/// On, signed in and with an app: Qobuz searches and plays for this person.
pub fn streams(s: &NodeState, who: &Who) -> bool {
    let lib = s.library.lock().expect("library poisoned");
    let st = load(&lib, who);
    st.enabled && st.session.is_some() && app_of(&lib).is_some()
}

/// The app and this person's session token, when the source can answer for them.
fn ready(s: &NodeState, who: &Who) -> Result<(App, Session), ApiError> {
    let lib = s.library.lock().expect("library poisoned");
    let st = load(&lib, who);
    let app = app_of(&lib).ok_or_else(|| {
        ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "Sign in to Qobuz again to refresh automatic setup",
        )
    })?;
    if !st.enabled {
        return Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "Qobuz is off: switch it on under Sources › Qobuz",
        ));
    }
    let sess = st
        .session
        .ok_or_else(|| ApiError::new(StatusCode::PRECONDITION_FAILED, not_connected(who)))?;
    Ok((app, sess))
}

/// Why Qobuz can't answer for this person: no sign-in of their own.
pub(crate) fn not_connected(who: &Who) -> String {
    match who {
        Who::Owner => "Sign in to Qobuz first (Sources › Qobuz)".into(),
        Who::Listener(_) => "Connect Qobuz (Settings › You › Services) to play this".into(),
        Who::ListenerOnNode(_) => "This node has no Qobuz sign-in: ask its owner to sign in".into(),
        Who::Nobody => "Qobuz plays with the account of whoever queued it, and this has none".into(),
    }
}

// ---- API -----------------------------------------------------------------------------------------

/// A catalogue call with the asking person's sign-in (never someone else's).
async fn get_json(s: &NodeState, path: &str, query: &[(&str, String)]) -> Result<Value, ApiError> {
    let who = crate::accounts::asking();
    let (app, sess) = ready(s, &who)?;
    let resp = s
        .http_client()
        .get(format!("{}/{path}", api_base()))
        .header(reqwest::header::USER_AGENT, auto::USER_AGENT)
        .header("X-App-Id", &app.id)
        .header("X-User-Auth-Token", &sess.token)
        .query(query)
        .send()
        .await
        .map_err(|e| upstream(e.without_url()))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    match status.as_u16() {
        200..=299 => Ok(v),
        401 => {
            let why = format!("Qobuz ended the session: {}", message(&v));
            mark_down(s, &why);
            Err(ApiError::new(StatusCode::BAD_GATEWAY, why))
        }
        404 => Err(ApiError::not_found("not on Qobuz")),
        _ => Err(upstream(format!("{path} answered {status}: {}", message(&v)))),
    }
}

fn message(v: &Value) -> String {
    v["message"]
        .as_str()
        .or(v["error"].as_str())
        .unwrap_or("")
        .to_string()
}

/// A failure of the mode itself: switch it off and say why.
fn mark_down(s: &NodeState, why: &str) {
    tracing::warn!("Qobuz (unofficial) switched off: {why}");
    update(s, &crate::accounts::asking(), |st| {
        st.enabled = false;
        st.health = Some(Health {
            ok: false,
            detail: why.into(),
            at: now(),
        });
    });
}

fn image_of(v: &Value) -> Option<String> {
    v["image"]["large"]
        .as_str()
        .or(v["image"]["small"].as_str())
        .or(v["image"]["thumbnail"].as_str())
        .map(str::to_string)
}

fn year_of(v: &Value) -> Option<i32> {
    v["release_date_original"]
        .as_str()
        .and_then(|d| d.get(..4))
        .and_then(|y| y.parse().ok())
        .or_else(|| {
            v["released_at"].as_i64().map(|t| {
                // Unix seconds to a year, near enough for a label
                (1970 + t / 31_556_952) as i32
            })
        })
}

fn artist_of(v: &Value) -> Option<String> {
    v["performer"]["name"]
        .as_str()
        .or(v["artist"]["name"].as_str())
        .or(v["album"]["artist"]["name"].as_str())
        .map(str::to_string)
}

fn format_of(v: &Value) -> String {
    let bits = v["maximum_bit_depth"].as_u64().unwrap_or(16);
    let rate = v["maximum_sampling_rate"].as_f64().unwrap_or(44.1);
    if bits > 16 {
        format!("FLAC {bits}-bit/{rate} kHz (Qobuz)")
    } else {
        "FLAC (Qobuz)".into()
    }
}

fn track_hit(t: &Value) -> Option<SourceHit> {
    let id = t["id"].as_u64()?.to_string();
    Some(SourceHit {
        id: format!("track:{id}"),
        title: t["title"].as_str()?.to_string(),
        creator: artist_of(t),
        year: year_of(&t["album"]),
        downloads: None,
        image: image_of(&t["album"]),
        kind: "track".into(),
        duration_ms: t["duration"].as_u64().map(|d| d * 1000),
        library_artist_id: None,
        isrc: t["isrc"].as_str().map(str::to_string),
        upc: None,
        library_track_id: None,
        library_album_id: None,
        uri: Some(track_uri(&id)),
        url: page_url("track", &id),
    })
}

fn album_hit(a: &Value) -> Option<SourceHit> {
    let id = a["id"]
        .as_str()
        .map(str::to_string)
        .or_else(|| a["id"].as_u64().map(|n| n.to_string()))?;
    Some(SourceHit {
        id: format!("album:{id}"),
        title: a["title"].as_str()?.to_string(),
        creator: artist_of(a),
        year: year_of(a),
        downloads: None,
        image: image_of(a),
        kind: "item".into(),
        duration_ms: None,
        library_artist_id: None,
        isrc: None,
        upc: a["upc"].as_str().map(str::to_string),
        library_track_id: None,
        library_album_id: None,
        uri: None,
        url: page_url("album", &id),
    })
}

fn artist_hit(a: &Value) -> Option<SourceHit> {
    let id = a["id"].as_u64()?.to_string();
    Some(SourceHit {
        id: format!("artist:{id}"),
        title: a["name"].as_str()?.to_string(),
        creator: None,
        year: None,
        downloads: None,
        image: image_of(a).or_else(|| a["picture"].as_str().map(str::to_string)),
        kind: "artist".into(),
        duration_ms: None,
        library_artist_id: None,
        isrc: None,
        upc: None,
        library_track_id: None,
        library_album_id: None,
        uri: None,
        url: page_url("artist", &id),
    })
}

fn source_track(t: &Value, album: &Value) -> Option<SourceTrack> {
    let id = t["id"].as_u64()?.to_string();
    Some(SourceTrack {
        title: t["title"].as_str()?.to_string(),
        track_no: t["track_number"].as_u64().map(|n| n as u32),
        duration_ms: t["duration"].as_u64().map(|d| d * 1000),
        format: format_of(if t["maximum_bit_depth"].is_null() {
            album
        } else {
            t
        }),
        url: track_uri(&id),
        artist: artist_of(t),
    })
}

pub struct Qobuz;

#[async_trait::async_trait]
impl StreamingService for Qobuz {
    fn id(&self) -> &'static str {
        SOURCE_ID
    }

    fn unified(&self, s: &NodeState) -> bool {
        streams(s, &crate::accounts::asking())
    }

    fn features(&self) -> &'static [&'static str] {
        &["search", "play", "artists"]
    }

    fn stage(&self) -> &'static str {
        "unofficial"
    }

    fn info(&self, s: &NodeState) -> SourceInfo {
        let lib = s.library.lock().expect("library poisoned");
        let st = load(&lib, &crate::accounts::asking());
        let status = if let Some(h) = st.health.as_ref().filter(|h| !h.ok && !st.enabled) {
            format!("error: {}", h.detail)
        } else if st.session.is_none() {
            "needs sign-in".to_string()
        } else if !st.enabled {
            "off: switch Qobuz (unofficial) on under Sources › Qobuz".to_string()
        } else {
            "signed in".to_string()
        };
        SourceInfo {
            id: SOURCE_ID.into(),
            name: "Qobuz".into(),
            description: "Unofficial: search, albums and full tracks in FLAC up to 24-bit/192 kHz with your Qobuz plan, through Qobuz's private API".into(),
            status,
        }
    }

    async fn search(&self, s: &NodeState, term: &str, limit: u32) -> Result<Vec<SourceHit>, ApiError> {
        let limit = limit.clamp(1, 50);
        let v = get_json(
            s,
            "catalog/search",
            &[("query", term.to_string()), ("limit", limit.to_string())],
        )
        .await?;
        let needle = term.trim().to_lowercase();
        let mut out: Vec<SourceHit> = v["artists"]["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(artist_hit)
            .filter(|a| a.title.to_lowercase() == needle)
            .take(1)
            .collect();
        out.extend(
            v["albums"]["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(album_hit),
        );
        out.extend(
            v["tracks"]["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(track_hit),
        );
        out.truncate(limit as usize * 2);
        Ok(out)
    }

    async fn item(&self, s: &NodeState, id: &str) -> Result<SourceItem, ApiError> {
        if let Some(aid) = id.strip_prefix("album:") {
            let a = get_json(s, "album/get", &[("album_id", album_id(aid)?.to_string())]).await?;
            let tracks = a["tracks"]["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|t| t["streamable"].as_bool() != Some(false))
                .filter_map(|t| source_track(t, &a))
                .collect();
            return Ok(SourceItem {
                source: SOURCE_ID.into(),
                id: id.into(),
                title: a["title"].as_str().unwrap_or_default().to_string(),
                creator: artist_of(&a),
                year: year_of(&a),
                description: a["label"]["name"].as_str().map(|l| format!("Label: {l}")),
                licence: None,
                image: image_of(&a),
                url: page_url("album", aid),
                tracks,
                library_album_id: None,
            });
        }
        if let Some(tid) = id.strip_prefix("track:") {
            let t = get_json(s, "track/get", &[("track_id", numeric(tid)?.to_string())]).await?;
            let album = t["album"].clone();
            return Ok(SourceItem {
                source: SOURCE_ID.into(),
                id: id.into(),
                title: t["title"].as_str().unwrap_or_default().to_string(),
                creator: artist_of(&t),
                year: year_of(&album),
                description: album["title"].as_str().map(|a| format!("From {a}")),
                licence: None,
                image: image_of(&album),
                url: page_url("track", tid),
                tracks: source_track(&t, &album).into_iter().collect(),
                library_album_id: None,
            });
        }
        Err(ApiError::bad_request("a Qobuz item is album:<id> or track:<id>"))
    }

    async fn artist(&self, s: &NodeState, id: &str) -> Result<SourceArtist, ApiError> {
        let aid = numeric(id.strip_prefix("artist:").unwrap_or(id))?.to_string();
        let a = get_json(
            s,
            "artist/get",
            &[
                ("artist_id", aid.clone()),
                ("extra", "albums".into()),
                ("limit", "50".into()),
            ],
        )
        .await?;
        let (albums, catalogue) = artist_releases(&a);
        Ok(SourceArtist {
            source: SOURCE_ID.into(),
            id: format!("artist:{aid}"),
            name: a["name"].as_str().unwrap_or_default().to_string(),
            image: image_of(&a),
            url: page_url("artist", &aid),
            albums,
            library_artist_id: None,
            catalogue: Some(catalogue),
        })
    }
}

/// An artist's releases from one `artist/get` answer (one page of 50: the unofficial API's paging
/// here is not documented, so no more is asked). Honest about what the page holds against Qobuz's
/// own count: more listed than arrived is `truncated`; no count, or rows the node can't use
/// (no id or title), make the list incomplete with the reason; nothing is counted after filtering.
pub(crate) fn artist_releases(a: &Value) -> (Vec<ArtistRelease>, Catalogue) {
    let items = a["albums"]["items"].as_array().cloned().unwrap_or_default();
    let mut albums: Vec<ArtistRelease> = items
        .iter()
        .filter_map(|x| {
            Some(ArtistRelease {
                hit: album_hit(x)?,
                release: crate::releases::qobuz(x),
            })
        })
        .collect();
    let unusable = items.len() - albums.len();
    crate::releases::dedupe(&mut albums);
    crate::releases::sort_releases(&mut albums);
    let total = a["albums"]["total"].as_u64();
    let truncated = total.is_some_and(|t| t > items.len() as u64);
    let error = if unusable > 0 {
        Some(format!(
            "Qobuz sent {} without an id or title",
            crate::plural(unusable as u64, "release")
        ))
    } else if total.is_none() {
        Some("Qobuz didn't say how many releases the artist has".to_string())
    } else {
        None
    };
    let catalogue = Catalogue {
        source: SOURCE_ID.into(),
        complete: error.is_none() && !truncated,
        truncated,
        total,
        error,
    };
    (albums, catalogue)
}

// ---- status, settings, sign-in ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QobuzStatus {
    pub enabled: bool,
    pub signed_in: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    pub hires: bool,
    pub quality: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<Health>,
    pub warning: String,
}

pub(crate) fn view(s: &NodeState, who: &Who) -> QobuzStatus {
    let lib = s.library.lock().expect("library poisoned");
    let st = load(&lib, who);
    QobuzStatus {
        enabled: st.enabled,
        signed_in: st.session.is_some(),
        user: st
            .session
            .as_ref()
            .and_then(|x| x.name.clone().or(x.user_id.clone())),
        plan: st.session.as_ref().and_then(|x| x.plan.clone()),
        hires: st.session.as_ref().is_some_and(|x| x.hires),
        quality: if st.quality.is_empty() {
            "max".into()
        } else {
            st.quality.clone()
        },
        health: st.health.clone(),
        warning: WARNING.into(),
    }
}

/// `GET /v1/sources/qobuz/unofficial`.
pub(crate) async fn get_status(
    State(s): State<Arc<NodeState>>,
    axum::Extension(caller): axum::Extension<crate::accounts::Caller>,
) -> ApiResult<QobuzStatus> {
    Ok(Json(view(&s, &Who::of(&caller))))
}

#[derive(Debug, Deserialize)]
pub struct QobuzUpdate {
    pub enabled: Option<bool>,
    pub quality: Option<String>,
}

/// `PUT /v1/sources/qobuz/unofficial {enabled?, quality?}`.
pub(crate) async fn put(
    State(s): State<Arc<NodeState>>,
    axum::Extension(caller): axum::Extension<crate::accounts::Caller>,
    Json(u): Json<QobuzUpdate>,
) -> ApiResult<QobuzStatus> {
    let who = Who::of(&caller);
    if let Some(q) = &u.quality {
        if q != "max" && q != "cd" && q != "mp3" {
            return Err(ApiError::bad_request("quality is max, cd or mp3"));
        }
    }
    update(&s, &who, |st| {
        if let Some(e) = u.enabled {
            st.enabled = e;
            if e {
                // Switching it on again is the Retry
                st.health = None;
            }
        }
        if let Some(q) = u.quality {
            st.quality = q;
        }
    });
    crate::search::forget_answers(&s);
    Ok(Json(view(&s, &who)))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub ok: bool,
    pub detail: String,
}

/// A track Qobuz has everywhere, for selecting the web player's signing secret.
const CHECK_TRACK: &str = "5966783";

/// Single flight for discovery and for the secret selection with its cache write.
static APP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// How a look at the web player ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// It found candidates.
    Found,
    /// It reached the web player, which is not what discovery expects.
    Changed,
    /// It could not reach the web player.
    Unreachable,
    /// It has not ended: it is running, or it was cut off.
    CutOff,
}

impl Outcome {
    /// How long after its start the next look waits.
    fn wait(self) -> Duration {
        match self {
            Self::Found | Self::Changed => DISCOVERY_EVERY,
            Self::Unreachable | Self::CutOff => RETRY_AFTER,
        }
    }

    /// What a caller is told inside the wait, when no candidates are offered.
    fn fail(self) -> auto::Fail {
        match self {
            Self::Found | Self::Changed => auto::Fail::Changed,
            Self::Unreachable => auto::Fail::Unreachable,
            Self::CutOff => auto::Fail::Interrupted,
        }
    }
}

/// A look at the web player: when it started ([`clock`]) and how it ended.
#[derive(Clone, Copy)]
struct Look {
    at: Duration,
    outcome: Outcome,
}

/// The candidates of the last look that found some.
struct Found {
    /// When that look started ([`clock`]).
    at: Duration,
    apps: Vec<App>,
    /// Qobuz refused them (their app ID, or every secret).
    refused: bool,
}

/// What the web player gave, in memory only; the rules are on [`known_candidates`].
struct Discovery {
    last: Option<Look>,
    found: Option<Found>,
}

impl Discovery {
    const NONE: Self = Self {
        last: None,
        found: None,
    };
}

static DISCOVERY: std::sync::Mutex<Discovery> = std::sync::Mutex::new(Discovery::NONE);
/// After a look that found candidates or saw a changed web player, the next waits this long.
const DISCOVERY_EVERY: Duration = Duration::from_secs(5 * 60);
/// After a look that could not reach Qobuz, or was cut off, the next waits only this long.
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// Found candidates are offered this long after their look started, never longer.
const CANDIDATES_FOR: Duration = Duration::from_secs(10 * 60);
/// A sign-in, and a stream's app refresh, end within this.
const DEADLINE: Duration = Duration::from_secs(60);

/// Time since the first discovery-clock read, for the discovery limits (tests move it on).
fn clock() -> Duration {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let elapsed = START.get_or_init(Instant::now).elapsed();
    #[cfg(test)]
    let elapsed = elapsed + *CLOCK_SKEW.lock().expect("Qobuz clock");
    elapsed
}

/// The web player's candidates: remembered ones, or a new look ([`known_candidates`] has the
/// rules). Call with `APP_LOCK` held, which makes a look single-flight.
async fn candidates(client: &reqwest::Client, fresh: bool) -> Result<Vec<App>, auto::Fail> {
    if let Some(known) = known_candidates(fresh) {
        return known;
    }
    let at = clock();
    let look = |outcome| Some(Look { at, outcome });
    // Until the look ends it counts as cut off: a dropped caller leaves exactly that behind
    DISCOVERY.lock().expect("Qobuz discovery").last = look(Outcome::CutOff);
    let found = auto::discover(client).await;
    let mut d = DISCOVERY.lock().expect("Qobuz discovery");
    d.last = look(match &found {
        Ok(_) => Outcome::Found,
        Err(auto::Fail::Changed) => Outcome::Changed,
        Err(_) => Outcome::Unreachable,
    });
    if let Ok(apps) = &found {
        d.found = Some(Found {
            at,
            apps: apps.clone(),
            refused: false,
        });
    }
    found
}

/// What [`candidates`] answers without a look; `None` when a new look is due.
///
/// - State: the last look (its start and how it ended: found, changed, unreachable, or cut off,
///   which is also how a running look counts), and the last candidates found (when, and whether
///   Qobuz refused them). A look stores how it ended only when it ends.
/// - A new look is due when there was none, or [`DISCOVERY_EVERY`] after the start of one that
///   found candidates or saw a changed web player, or [`RETRY_AFTER`] after one that could not
///   reach Qobuz or was cut off.
/// - Candidates are usable when Qobuz did not refuse them and are younger than [`CANDIDATES_FOR`].
///   A caller that is not `fresh` gets usable candidates whenever there are some, a due look or not.
/// - `fresh` means the known values were refused. Such a caller gets a new look when one is due,
///   and inside the wait only the candidates of the last look, and only when it found them and
///   they are usable.
/// - Otherwise, inside the wait, the last look's own error: could not reach Qobuz, interrupted
///   (cut off), or changed (it saw a changed player, or Qobuz refused what it found). Only that
///   last one, from a look that reached the web player, can switch a person's source off.
fn known_candidates(fresh: bool) -> Option<Result<Vec<App>, auto::Fail>> {
    let d = DISCOVERY.lock().expect("Qobuz discovery");
    let now = clock();
    let usable = d
        .found
        .as_ref()
        .filter(|f| !f.refused && now.saturating_sub(f.at) < CANDIDATES_FOR)
        .map(|f| f.apps.clone());
    if !fresh {
        if let Some(apps) = usable {
            return Some(Ok(apps));
        }
    }
    let last = d.last?;
    if now.saturating_sub(last.at) >= last.outcome.wait() {
        return None;
    }
    Some(match (last.outcome, usable) {
        (Outcome::Found, Some(apps)) => Ok(apps),
        (outcome, _) => Err(outcome.fail()),
    })
}

/// Qobuz refused `apps` (their app ID, or every secret): they are not offered again, and until
/// the next look is due [`candidates`] says so at once instead of probing them again.
fn refuse(apps: &[App]) {
    let mut d = DISCOVERY.lock().expect("Qobuz discovery");
    if let Some(found) = d.found.as_mut().filter(|f| f.apps == apps) {
        found.refused = true;
    }
}

/// [`auto::select_secret`]; candidates whose every secret Qobuz refuses with a 400 about the
/// signature or the app ID are [`refuse`]d. Any other 400 fails only this caller.
async fn select(client: &reqwest::Client, found: &[App], token: &str) -> Result<App, auto::Fail> {
    let chosen = auto::select_secret(client, found, token).await;
    if chosen == Err(auto::Fail::Changed) {
        refuse(found);
    }
    chosen
}

#[derive(Deserialize)]
pub struct Login {
    pub email: String,
    pub password: String,
}

/// `POST /v1/sources/qobuz/unofficial/login {email, password}`: discovers the app when needed,
/// signs in once, and keeps only the session token. Neither password nor its MD5 is stored.
pub(crate) async fn login(
    State(s): State<Arc<NodeState>>,
    axum::Extension(caller): axum::Extension<crate::accounts::Caller>,
    Json(l): Json<Login>,
) -> ApiResult<QobuzStatus> {
    let who = Who::of(&caller);
    if who == Who::Nobody {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "sign in to the node first"));
    }
    if l.email.trim().is_empty() || l.password.is_empty() {
        return Err(ApiError::bad_request("Enter your Qobuz email and password"));
    }
    let hash = hex(&Md5::digest(l.password.as_bytes()));
    let sess = tokio::time::timeout(DEADLINE, sign_in(&s, l.email.trim(), &hash))
        .await
        .map_err(|_| ApiError::from(auto::Fail::Unreachable))??;
    finish_login(&s, &who, sess)
}

/// This person's session, with app values that work for it cached for the node. `APP_LOCK`
/// covers discovery and the secret selection with its cache write, not the person's `user/login`.
///
/// A 400 with the cached values that is not about them is this person's own, or Qobuz retired
/// those values and says so in other words. Either way nothing is refused: the web player's
/// candidates (remembered ones, or a look the limits allow) get one try when the first of them
/// differs from the cached values, and the 400 stands when they do not or when there are none.
async fn sign_in(s: &NodeState, email: &str, hash: &str) -> Result<Session, ApiError> {
    let client = auto::client()?;
    if let Some(app) = configured_app() {
        let sess = authenticate(&client, &app, email, hash)
            .await
            .map_err(LoginFailure::into_configured)?;
        return Ok(sess);
    }
    let cached = cached_app(&s.library.lock().expect("library poisoned"));
    let mut declined = None;
    if let Some(app) = &cached {
        match authenticate(&client, app, email, hash).await {
            Ok(sess) => match auto::secret_works(&client, app, &sess.token).await {
                Ok(true) => return Ok(sess),
                Ok(false) => {}
                Err(auto::Fail::NoStream) => declined = Some((auto::Fail::NoStream, app)),
                Err(f) => return Err(f.into()),
            },
            Err(LoginFailure::BadApp) => {}
            Err(LoginFailure::Declined) => declined = Some((auto::Fail::Declined, app)),
            Err(LoginFailure::Failed(e)) => return Err(e),
        }
    }
    // `fresh` only when Qobuz refused the cached values themselves
    let fresh = cached.is_some() && declined.is_none();
    let found = {
        let _guard = APP_LOCK.lock().await;
        candidates(&client, fresh).await
    };
    let found = match declined {
        None => found?,
        Some((why, old)) => match found {
            // `user/login` reads only the app ID; the stream check also the secret
            Ok(found)
                if found.first().is_some_and(|new| {
                    new.id != old.id || (why == auto::Fail::NoStream && new.secret != old.secret)
                }) =>
            {
                found
            }
            _ => return Err(why.into()),
        },
    };
    let first = found.first().ok_or_else(auto::discovery_failed)?;
    let sess = match authenticate(&client, first, email, hash).await {
        Ok(sess) => sess,
        Err(LoginFailure::BadApp) => {
            refuse(&found);
            return Err(auto::discovery_failed());
        }
        Err(LoginFailure::Declined) => return Err(auto::Fail::Declined.into()),
        Err(LoginFailure::Failed(e)) => return Err(e),
    };
    let _guard = APP_LOCK.lock().await;
    let app = select(&client, &found, &sess.token).await?;
    let lib = s.library.lock().expect("library poisoned");
    if cached_app(&lib).as_ref() != Some(&app) {
        cache_app(&lib, &app)?;
    }
    Ok(sess)
}

enum LoginFailure {
    /// A 400 about the app ID.
    BadApp,
    /// Any other 400: this sign-in's own, as far as anyone can tell.
    Declined,
    Failed(ApiError),
}

impl LoginFailure {
    fn into_configured(self) -> ApiError {
        match self {
            Self::BadApp => upstream("Qobuz refused the configured app ID or secret"),
            Self::Declined => auto::Fail::Declined.into(),
            Self::Failed(e) => e,
        }
    }
}

async fn authenticate(
    client: &reqwest::Client,
    app: &App,
    email: &str,
    hash: &str,
) -> Result<Session, LoginFailure> {
    let response = client
        .get(format!("{}/user/login", api_base()))
        .header("X-App-Id", &app.id)
        .query(&[("email", email), ("password", hash), ("app_id", &app.id)])
        .send()
        .await
        .map_err(|_| LoginFailure::Failed(auto::Fail::Unreachable.into()))?;
    let status = response.status();
    if status == StatusCode::BAD_REQUEST {
        // Only a message about the app ID is the app's; any other 400 is this sign-in's alone
        return Err(if about_app_id(&auto::json_of(response).await) {
            LoginFailure::BadApp
        } else {
            LoginFailure::Declined
        });
    }
    if status == StatusCode::UNAUTHORIZED {
        return Err(LoginFailure::Failed(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "Wrong Qobuz email or password",
        )));
    }
    if !status.is_success() {
        return Err(LoginFailure::Failed(upstream(
            "Qobuz could not sign you in right now. Try again later",
        )));
    }
    let bytes = auto::bounded(response, 64 * 1024).await.map_err(|f| {
        LoginFailure::Failed(match f {
            auto::Fail::Changed => upstream("Qobuz returned an unreadable sign-in response"),
            auto::Fail::Unreachable
            | auto::Fail::Interrupted
            | auto::Fail::Declined
            | auto::Fail::NoStream => f.into(),
        })
    })?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| LoginFailure::Failed(upstream("Qobuz returned an unreadable sign-in response")))?;
    let token = value["user_auth_token"]
        .as_str()
        .filter(|t| !t.is_empty() && t.len() <= 4096)
        .ok_or_else(|| LoginFailure::Failed(upstream("Qobuz returned no session")))?;
    session_from_user(token, &value["user"]).map_err(LoginFailure::Failed)
}

fn finish_login(s: &NodeState, who: &Who, sess: Session) -> ApiResult<QobuzStatus> {
    update_signed_in(s, who, "qobuz", |st| {
        st.session = Some(sess);
        st.enabled = true;
        st.health = None;
    });
    crate::search::forget_answers(s);
    tracing::info!("Qobuz sign-in stored");
    Ok(Json(view(s, who)))
}

fn session_from_user(token: &str, user: &Value) -> Result<Session, ApiError> {
    let params = &user["credential"]["parameters"];
    if params.is_null() || params.as_object().is_some_and(|o| o.is_empty()) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "This Qobuz account is free; full tracks need a paid Qobuz plan",
        ));
    }
    Ok(Session {
        token: token.to_string(),
        user_id: user["id"].as_u64().map(|n| n.to_string()),
        name: user["display_name"]
            .as_str()
            .or(user["login"].as_str())
            .map(str::to_string),
        plan: user["credential"]["label"]
            .as_str()
            .or(user["credential"]["description"].as_str())
            .map(str::to_string),
        country: user["country_code"].as_str().map(str::to_string),
        hires: params["hires_streaming"].as_bool().unwrap_or(false),
        from: None,
    })
}

// ---- one Sign in, the ways side by side (STEWARD-CREDENTIALS-REUSE.md step 3) ----

pub(crate) const WAY_OWN: &str = "own";

/// Qobuz's ways, for `crate::signin`.
pub(crate) struct QobuzSignIn;

impl QobuzSignIn {
    fn in_use(st: &ModeState) -> Option<crate::signin::InUse> {
        let sess = st.session.as_ref()?;
        Some(crate::signin::InUse {
            way: WAY_OWN.into(),
            label: "Your Qobuz account".into(),
            user: sess.name.clone().or_else(|| sess.user_id.clone()),
            expires_at: None,
            gives: Some(if sess.hires { "hi-res" } else { "CD quality" }.into()),
        })
    }
}

#[async_trait::async_trait]
impl crate::signin::SignInSource for QobuzSignIn {
    fn describe(
        &self,
        s: &NodeState,
        who: &Who,
        headers: Option<&axum::http::HeaderMap>,
    ) -> crate::signin::SignIn {
        use crate::signin::{SignIn, Way, NOT_SIGNED_IN};
        let (st, chosen) = {
            let lib = s.library.lock().expect("library poisoned");
            (load(&lib, who), crate::signin::chosen(&lib, who, SOURCE_ID))
        };
        let ways = vec![Way {
            id: WAY_OWN.into(),
            label: "Your Qobuz account".into(),
            note: "Your Qobuz email and password; automatic app setup, and only the session is kept.".into(),
            action: "setup".into(),
            recommended: true,
            advanced: false,
            available: true,
            reason: None,
        }];
        let in_use = Self::in_use(&st);
        let line = match &in_use {
            None => NOT_SIGNED_IN.to_string(),
            Some(u) => format!("{} · {}", u.label, u.gives.clone().unwrap_or_default()),
        };
        SignIn {
            source: SOURCE_ID.into(),
            ways,
            chosen,
            in_use,
            line,
            imports: Vec::new(),
            places: Vec::new(),
            details: if headers.is_some() {
                serde_json::json!({ "unofficial": view(s, who) })
            } else {
                serde_json::Value::Null
            },
        }
    }

    fn sign_out_of(&self, s: &NodeState, who: &Who, _way: &str) -> Result<(), ApiError> {
        update(s, who, |st| st.session = None);
        crate::search::forget_answers(s);
        Ok(())
    }

    async fn import(
        &self,
        _s: &Arc<NodeState>,
        _who: &Who,
        _body: crate::signin::ImportBody,
    ) -> Result<&'static str, ApiError> {
        Err(ApiError::not_found("Qobuz signs in with email and password only"))
    }
}

/// `DELETE /v1/sources/qobuz/unofficial/login`: forget the session (the mode stays as set).
pub(crate) async fn logout(
    State(s): State<Arc<NodeState>>,
    axum::Extension(caller): axum::Extension<crate::accounts::Caller>,
) -> ApiResult<QobuzStatus> {
    let who = Who::of(&caller);
    update(&s, &who, |st| st.session = None);
    crate::search::forget_answers(&s);
    Ok(Json(view(&s, &who)))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ---- playback -------------------------------------------------------------------------------------

/// Whether a failure is the mode's (switch it off), only this track's, or a Qobuz that could not
/// be reached or is busy (neither: try again soon).
#[derive(Debug)]
pub enum StreamError {
    Mode(String),
    Track(String),
    Unavailable(String),
}

/// The signed `track/getFileUrl` request: Qobuz checks
/// `md5("trackgetFileUrlformat_id{f}intentstreamtrack_id{id}{ts}{secret}")`.
fn file_url_query(app: &App, id: &str, format_id: u32, ts: i64) -> Vec<(&'static str, String)> {
    let sig = hex(&Md5::digest(
        format!(
            "trackgetFileUrlformat_id{format_id}intentstreamtrack_id{id}{ts}{}",
            app.secret
        )
        .as_bytes(),
    ));
    vec![
        ("request_ts", ts.to_string()),
        ("request_sig", sig),
        ("track_id", id.to_string()),
        ("format_id", format_id.to_string()),
        ("intent", "stream".into()),
    ]
}

/// FLAC 24-bit up to 192 kHz (27), 16-bit/44.1 kHz (6), or MP3 320 kbps (5).
fn format_id(st: &ModeState) -> u32 {
    if st.quality == "mp3" {
        5
    } else if st.quality == "cd" || st.session.as_ref().is_some_and(|x| !x.hires) {
        6
    } else {
        27
    }
}

fn content_type_for(format: u32) -> &'static str {
    if format == 5 {
        "audio/mpeg"
    } else {
        "audio/flac"
    }
}

/// Qobuz's message is about the app ID.
fn about_app_id(v: &Value) -> bool {
    let m = message(v).to_lowercase();
    m.contains("app_id") || m.contains("app id")
}

/// A 400 about the signature or the app ID: the app values are refused, not the track or the
/// person.
fn app_refused(status: u16, v: &Value) -> bool {
    status == 400 && (about_app_id(v) || message(v).to_lowercase().contains("signature"))
}

/// What `track/getFileUrl` answered, judged: the stream URL, or why not.
fn judge_file_url(status: u16, v: &Value) -> Result<String, StreamError> {
    match status {
        200..=299 => {}
        400 if app_refused(status, v) => {
            return Err(StreamError::Mode(format!(
                "Qobuz refused the app secret: {}",
                message(v)
            )))
        }
        401 => {
            return Err(StreamError::Mode(format!(
                "Qobuz ended the session: {}",
                message(v)
            )))
        }
        403 => {
            return Err(StreamError::Track(format!(
                "Qobuz won't stream this track: {}",
                message(v)
            )))
        }
        404 => return Err(StreamError::Track("Qobuz has no stream for this track".into())),
        408 | 429 | 500..=599 => {
            return Err(StreamError::Unavailable(format!(
                "Qobuz answered {status}: {}",
                message(v)
            )))
        }
        _ => {
            return Err(StreamError::Track(format!(
                "Qobuz answered {status}: {}",
                message(v)
            )))
        }
    }
    if v["sample"].as_bool() == Some(true) {
        return Err(StreamError::Track(
            "Qobuz sent a 30-second sample: the plan may not include this track".into(),
        ));
    }
    v["url"]
        .as_str()
        .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
        .map(str::to_string)
        .ok_or_else(|| StreamError::Mode("Qobuz answered without a stream URL".into()))
}

/// A refresh's failure: only a web player that no longer works is the mode's; a network problem,
/// a timeout or a busy Qobuz is only for now, and a probe 400 that is not about the app values
/// fails only this track.
fn refresh_failed(f: auto::Fail) -> StreamError {
    match f {
        auto::Fail::Changed => StreamError::Mode(f.message().into()),
        auto::Fail::Unreachable | auto::Fail::Interrupted => StreamError::Unavailable(f.message().into()),
        auto::Fail::Declined | auto::Fail::NoStream => {
            StreamError::Track("Qobuz refused this request".into())
        }
    }
}

/// New app values after Qobuz refused `old`'s: another refresh's if one came first, else the web
/// player's candidates (at most one look per [`DISCOVERY_EVERY`]) and the first secret that works.
/// Ends within [`DEADLINE`], waiting for the lock included.
async fn refresh_app(
    library: &std::sync::Mutex<pm_library::Library>,
    old: &App,
    token: &str,
) -> Result<App, StreamError> {
    let refresh = async {
        let _guard = APP_LOCK.lock().await;
        if let Some(current) = {
            let lib = library.lock().map_err(|_| StreamError::Track("library".into()))?;
            app_of(&lib).filter(|app| app != old)
        } {
            return Ok(current);
        }
        let client = auto::client().map_err(refresh_failed)?;
        let found = candidates(&client, true).await.map_err(refresh_failed)?;
        let app = select(&client, &found, token).await.map_err(refresh_failed)?;
        let lib = library.lock().map_err(|_| StreamError::Track("library".into()))?;
        cache_app(&lib, &app).map_err(|e| StreamError::Track(e.message))?;
        Ok(app)
    };
    tokio::time::timeout(DEADLINE, refresh)
        .await
        .unwrap_or_else(|_| Err(refresh_failed(auto::Fail::Unreachable)))
}

/// `track/getFileUrl` for the audio thread: its status and JSON.
fn ask_file_url_blocking(app: &App, token: &str, id: &str, format: u32) -> Result<(u16, Value), StreamError> {
    let resp = crate::sources::http_client()
        .get(format!("{}/track/getFileUrl", api_base()))
        .header(reqwest::header::USER_AGENT, auto::USER_AGENT)
        .header("X-App-Id", &app.id)
        .header("X-User-Auth-Token", token)
        .query(&file_url_query(app, id, format, now()))
        .send()
        .map_err(|e| StreamError::Unavailable(format!("Qobuz: {}", e.without_url())))?;
    let status = resp.status().as_u16();
    Ok((status, resp.json().unwrap_or(Value::Null)))
}

/// The stream URL of a track (the audio thread).
pub fn file_url_blocking(
    library: &std::sync::Mutex<pm_library::Library>,
    id: &str,
    who: &Who,
) -> Result<String, StreamError> {
    let (app, st) = {
        let lib = library.lock().map_err(|_| StreamError::Track("library".into()))?;
        (app_of(&lib), load(&lib, who))
    };
    let app = app.ok_or_else(|| StreamError::Track("no Qobuz app on this node".into()))?;
    let sess = st
        .session
        .clone()
        .filter(|_| st.enabled)
        .ok_or_else(|| StreamError::Track(not_connected(who)))?;
    let format = format_id(&st);
    let (status, v) = ask_file_url_blocking(&app, &sess.token, id, format)?;
    if !app_refused(status, &v) || configured_app().is_some() {
        return judge_file_url(status, &v);
    }
    let app = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| StreamError::Track("Qobuz refresh runtime".into()))?
        .block_on(refresh_app(library, &app, &sess.token))?;
    let (status, v) = ask_file_url_blocking(&app, &sess.token, id, format)?;
    judge_file_url(status, &v)
}

/// `track/getFileUrl`: its status and JSON.
async fn ask_file_url(
    s: &NodeState,
    app: &App,
    token: &str,
    id: &str,
    format: u32,
) -> Result<(u16, Value), StreamError> {
    let resp = s
        .http_client()
        .get(format!("{}/track/getFileUrl", api_base()))
        .header(reqwest::header::USER_AGENT, auto::USER_AGENT)
        .header("X-App-Id", &app.id)
        .header("X-User-Auth-Token", token)
        .query(&file_url_query(app, id, format, now()))
        .send()
        .await
        .map_err(|e| StreamError::Unavailable(format!("Qobuz: {}", e.without_url())))?;
    let status = resp.status().as_u16();
    Ok((status, resp.json().await.unwrap_or(Value::Null)))
}

/// The stream URL of a track; app values Qobuz refuses are refreshed once (unless an admin set them).
async fn file_url(s: &NodeState, who: &Who, id: &str, format: u32) -> Result<String, StreamError> {
    let (app, sess) = ready(s, who).map_err(|e| StreamError::Track(e.message))?;
    let (status, v) = ask_file_url(s, &app, &sess.token, id, format).await?;
    if !app_refused(status, &v) || configured_app().is_some() {
        return judge_file_url(status, &v);
    }
    let app = refresh_app(&s.library, &app, &sess.token).await?;
    let (status, v) = ask_file_url(s, &app, &sess.token, id, format).await?;
    judge_file_url(status, &v)
}

/// Record how the source did: a failure of the mode itself switches it off.
pub fn record(library: &std::sync::Mutex<pm_library::Library>, who: &Who, r: &Result<String, StreamError>) {
    let Ok(lib) = library.lock() else { return };
    let mut st = load(&lib, who);
    match r {
        Ok(_) => {
            st.health = Some(Health {
                ok: true,
                detail: "full tracks are playing".into(),
                at: now(),
            });
        }
        Err(StreamError::Mode(e)) => {
            tracing::warn!("Qobuz (unofficial) switched off: {e}");
            st.enabled = false;
            st.health = Some(Health {
                ok: false,
                detail: e.clone(),
                at: now(),
            });
        }
        Err(StreamError::Track(_) | StreamError::Unavailable(_)) => return,
    }
    save(&lib, who, &st);
}

/// A `qobuz:track:<id>` for a zone.
pub(crate) fn open_track_blocking(
    library: &std::sync::Mutex<pm_library::Library>,
    id: &str,
    who: &Who,
) -> Result<Box<dyn pm_zones::FrameSource>, pm_zones::SourceError> {
    let fail = |reason: String| pm_zones::SourceError::Open {
        uri: track_uri(id),
        reason,
    };
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(fail("bad Qobuz track id".into()));
    }
    let r = file_url_blocking(library, id, who);
    record(library, who, &r);
    let url = r.map_err(|e| match e {
        StreamError::Mode(m) | StreamError::Track(m) | StreamError::Unavailable(m) => fail(m),
    })?;
    crate::sources::open_http_stream(&url).map_err(|_| fail("the Qobuz stream did not open".into()))
}

/// `GET /v1/sources/qobuz/stream/{id}`: the track for browsers, relayed by the node so the
/// session and the app never reach the page.
pub(crate) async fn stream(
    State(s): State<Arc<NodeState>>,
    axum::Extension(caller): axum::Extension<crate::accounts::Caller>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    let id = numeric(&id)?.to_string();
    // With the account the caller plays with (their own, or the node's under its source policy)
    let who = Who::of(&caller).resolved(&s.library.lock().expect("library poisoned"));
    // Use one snapshot for the request and its response type, even if quality changes in flight.
    let format = format_id(&load(&s.library.lock().expect("library poisoned"), &who));
    let r = file_url(&s, &who, &id, format).await;
    record(&s.library, &who, &r);
    let url = r.map_err(|e| match e {
        StreamError::Mode(m) => ApiError::new(StatusCode::BAD_GATEWAY, m),
        StreamError::Track(m) => ApiError::not_found(m),
        StreamError::Unavailable(m) => ApiError::new(StatusCode::SERVICE_UNAVAILABLE, m),
    })?;
    let resp = s
        .http_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| upstream(e.without_url()))?;
    if !resp.status().is_success() {
        return Err(upstream(format!("stream answered {}", resp.status())));
    }
    let len = resp.content_length();
    let mut out = axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, content_type_for(format))
        .header(axum::http::header::CACHE_CONTROL, "private, max-age=600");
    if let Some(n) = len {
        out = out.header(axum::http::header::CONTENT_LENGTH, n);
    }
    out.body(axum::body::Body::from_stream(resp.bytes_stream()))
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_content_type_matches_the_requested_format() {
        assert_eq!(content_type_for(5), "audio/mpeg");
        assert_eq!(content_type_for(6), "audio/flac");
        assert_eq!(content_type_for(27), "audio/flac");
    }

    #[test]
    fn the_file_url_request_is_signed_with_the_app_secret() {
        let app = App {
            id: "100000000".into(),
            secret: "abc123".into(),
            from: "settings".into(),
            bundle_url: None,
        };
        let q = file_url_query(&app, "5966783", 27, 1_700_000_000);
        let sig = &q.iter().find(|(k, _)| *k == "request_sig").unwrap().1;
        let want = hex(&Md5::digest(
            b"trackgetFileUrlformat_id27intentstreamtrack_id59667831700000000abc123",
        ));
        assert_eq!(sig, &want);
        assert!(format!("{app:?}").contains("<set>") && !format!("{app:?}").contains("abc123"));
    }

    #[test]
    fn session_debug_redacts_the_user_token() {
        let session = Session {
            token: "private-user-token".into(),
            name: Some("Listener".into()),
            ..Default::default()
        };
        let shown = format!("{session:?}");
        assert!(shown.contains("<set>") && !shown.contains("private-user-token"));
    }

    #[test]
    fn a_sample_is_refused_and_a_bad_signature_turns_the_mode_off() {
        let ok = judge_file_url(
            200,
            &serde_json::json!({"url": "https://x/a.flac", "format_id": 27}),
        );
        assert_eq!(ok.unwrap(), "https://x/a.flac");
        assert!(matches!(
            judge_file_url(
                200,
                &serde_json::json!({"url": "https://x/a.mp3", "sample": true})
            ),
            Err(StreamError::Track(_))
        ));
        assert!(matches!(
            judge_file_url(
                400,
                &serde_json::json!({"message": "Invalid Request Signature parameter (request_sig)"})
            ),
            Err(StreamError::Mode(_))
        ));
        assert!(matches!(
            judge_file_url(404, &Value::Null),
            Err(StreamError::Track(_))
        ));
    }
}
