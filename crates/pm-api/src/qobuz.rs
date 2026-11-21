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
use serde::Deserialize;
use std::sync::Arc;

// The app values, sign-in, catalogue and stream URLs are shared with the phone (pm-sources).
pub use pm_sources::qobuz::*;

fn now() -> i64 {
    pm_library::db::now_unix()
}

// ---- state ---------------------------------------------------------------------------------------

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

/// The node's [`AppCache`]: its settings, one set of app values for everyone on it.
pub(crate) struct NodeApps<'a>(pub &'a std::sync::Mutex<pm_library::Library>);

impl AppCache for NodeApps<'_> {
    fn cached(&self) -> Option<App> {
        cached_app(&self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn keep(&self, app: &App) -> Result<(), pm_sources::SourceError> {
        cache_app(&self.0.lock().unwrap_or_else(|e| e.into_inner()), app)
            .map_err(|e| pm_sources::SourceError::new(e.status, e.message))
    }
}

/// The app id the node's Qobuz sign-in is made with (`vault_cache`).
pub(crate) fn app_id(s: &NodeState) -> Option<String> {
    app_of(&s.library.lock().expect("library poisoned")).map(|a| a.id)
}

/// The admin override wins; otherwise the last web-player app is reused.
pub fn app_of(lib: &pm_library::Library) -> Option<App> {
    pm_sources::qobuz::configured_app().or_else(|| cached_app(lib))
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

/// What a failed catalogue call answers: an ended session switches the mode off first.
fn failed(s: &NodeState, e: CallError) -> ApiError {
    match e {
        CallError::Ended(why) => {
            mark_down(s, &why);
            ApiError::new(StatusCode::BAD_GATEWAY, why)
        }
        CallError::Other(e) => e.into(),
    }
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
        let (app, sess) = ready(s, &crate::accounts::asking())?;
        pm_sources::qobuz::search(&s.http_client(), &app, &sess.token, term, limit)
            .await
            .map_err(|e| failed(s, e))
    }

    async fn item(&self, s: &NodeState, id: &str) -> Result<SourceItem, ApiError> {
        let (app, sess) = ready(s, &crate::accounts::asking())?;
        pm_sources::qobuz::item(&s.http_client(), &app, &sess.token, id)
            .await
            .map_err(|e| failed(s, e))
    }

    async fn artist(&self, s: &NodeState, id: &str) -> Result<SourceArtist, ApiError> {
        let (app, sess) = ready(s, &crate::accounts::asking())?;
        pm_sources::qobuz::artist(&s.http_client(), &app, &sess.token, id)
            .await
            .map_err(|e| failed(s, e))
    }
}

// ---- status, settings, sign-in ---------------------------------------------------------------------

pub(crate) fn view(s: &NodeState, who: &Who) -> QobuzStatus {
    status(&load(&s.library.lock().expect("library poisoned"), who))
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
    let hash = password_hash(&l.password);
    let sess = tokio::time::timeout(DEADLINE, sign_in(&NodeApps(&s.library), l.email.trim(), &hash))
        .await
        .map_err(|_| ApiError::from(pm_sources::SourceError::from(auto::Fail::Unreachable)))??;
    finish_login(&s, &who, sess)
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

// ---- playback -------------------------------------------------------------------------------------

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
    pm_sources::qobuz::file_url_blocking(
        &NodeApps(library),
        crate::sources::http_client(),
        app,
        &sess.token,
        id,
        format_id(&st),
    )
}

/// The stream URL of a track; app values Qobuz refuses are refreshed once (unless an admin set them).
async fn file_url_for(s: &NodeState, who: &Who, id: &str, format: u32) -> Result<String, StreamError> {
    let (app, sess) = ready(s, who).map_err(|e| StreamError::Track(e.message))?;
    pm_sources::qobuz::file_url(
        &s.http_client(),
        &NodeApps(&s.library),
        app,
        &sess.token,
        id,
        format,
    )
    .await
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
    let r = file_url_for(&s, &who, &id, format).await;
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
        return Err(upstream(format!("stream answered {}", resp.status())).into());
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
