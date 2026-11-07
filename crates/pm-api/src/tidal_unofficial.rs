//! TIDAL "Full tracks (unofficial)" (owner, 2026-10-22; STEWARD-TIDAL-FULL.md, the first
//! source of the dual-mode pattern in STEWARD-DUAL-MODE.md). The official app (Open API,
//! third-party tier) only gets 30 s previews. This opt-in mode signs in the way TIDAL's own TV
//! and desktop apps do (OAuth device flow: a code entered at link.tidal.com) and asks TIDAL's
//! private playback endpoint for the full stream. TIDAL has not approved it: it can stop working
//! at any time, and it needs a paid TIDAL plan.
//!
//! - **Sign-ins** (owner, 2026-11-04), each person their own:
//!   - **Sign in with TIDAL** (the browser sign-in, Recommended): OAuth 2 with PKCE the way
//!     python-tidal's `login_pkce` does it. The node builds the login.tidal.com address, the
//!     person signs in in their browser, lands on a tidal.com page and pastes its address back;
//!     the node takes the code for tokens and renews them with the same client. Its client is
//!     built in ([`crate::project_apps::tidal_pkce_client`]); nobody enters one. It is the only
//!     sign-in TIDAL serves lossless and hi-res FLAC to.
//!   - **The code sign-in** (fallback): OAuth device flow, a code entered at link.tidal.com, with
//!     a device client: the owner's (Settings, else `[tidal] unofficial_client_id` /
//!     `unofficial_client_secret`, else `PM_TIDAL_UNOFFICIAL_CLIENT_*`), else the build's
//!     ([`crate::project_apps::tidal_device_client`]). TIDAL may send it AAC only.
//!
//!   A session records which client made it ([`ClientKind`]; one stored before this has none and
//!   is a device session), so it is renewed with that one.
//! - **Streams:** `playbackinfopostpaywall` answers with a BTS manifest (direct URLs) or a DASH
//!   manifest; both are read as one stream into the node's usual decoder. Anything encrypted is
//!   refused, never decrypted.
//! - **Health:** a failure of the mode itself (revoked client, changed answer) switches it off,
//!   playback falls back to the official preview, and the TIDAL page says why, with Retry. Each
//!   opened track logs what TIDAL answered (quality, manifest, codecs, depth and rate; never a URL
//!   or token); AAC for HI_RES_LOSSLESS is asked again as LOSSLESS, and a sign-in that still gets
//!   AAC says so on the TIDAL page, with what to do.

use crate::accounts::Who;
use crate::state::NodeState;
use crate::{ApiError, ApiResult};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

pub const STATE_KEY: &str = "tidal.unofficial";
pub const CLIENT_KEY: &str = "tidal.unofficial_client";
const SCOPE: &str = "r_usr w_usr w_sub";
/// The browser sign-in's scope, exactly as python-tidal's `pkce_get_auth_token` sends it.
const PKCE_SCOPE: &str = "r_usr+w_usr+w_sub";
/// Where TIDAL sends the browser after the browser sign-in (python-tidal's `pkce_uri_redirect`):
/// a tidal.com page whose address carries the code.
pub const PKCE_REDIRECT: &str = "https://tidal.com/android/login/auth";
/// How long a browser sign-in waits for its pasted address.
const BROWSER_WAIT_SECS: i64 = 600;
/// The health line of a code sign-in that gets AAC where lossless was asked for.
pub const AAC_ONLY_DEVICE: &str =
    "TIDAL sends this sign-in AAC only (lossy). Sign in with TIDAL again for lossless";
/// The same for a device session imported from another app or pasted (it wasn't signed in here).
pub const AAC_ONLY_IMPORTED: &str =
    "TIDAL sends this sign-in AAC only (lossy). Sign in with TIDAL for lossless";
/// The same on a build without the browser client: what is possible then.
pub const AAC_ONLY_NO_BROWSER: &str = "TIDAL sends this sign-in AAC only (lossy). Lossless needs Sign in with TIDAL, which this build doesn't have (a release build carries it)";
/// The health line of a browser sign-in whose last track still came as AAC.
pub const AAC_TRACK: &str = "TIDAL sent the last track as AAC (lossy): it may have no lossless version";
/// What the page says before the mode is switched on.
pub const WARNING: &str = "This uses TIDAL's private API, which TIDAL has not approved for other apps. It can stop working at any time, and TIDAL could act on the account. It needs a paid TIDAL plan.";
/// TIDAL's answer when the client comes from its developer portal, in plain words.
pub const NOT_A_DEVICE_CLIENT: &str = "This client can't use the code sign-in. Apps from TIDAL's developer portal are not device clients; enter a TIDAL device client instead.";

fn v1_base(s: &NodeState) -> String {
    s.tidal
        .read()
        .expect("tidal endpoints")
        .v1_base
        .trim_end_matches('/')
        .to_string()
}

fn auth_base(s: &NodeState) -> String {
    s.tidal
        .read()
        .expect("tidal endpoints")
        .auth_base
        .trim_end_matches('/')
        .to_string()
}

/// A urlencoded form body (reqwest is built without its form feature here).
fn form(pairs: &[(&str, &str)]) -> String {
    let mut ser = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        ser.append_pair(k, v);
    }
    ser.finish()
}

fn now() -> i64 {
    pm_library::db::now_unix()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Session {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    pub expires_at: i64,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub session_id: Option<String>,
    /// The app this sign-in was imported from (`tidal-dl-ng`, `pasted`, …); none for the node's
    /// own device sign-in (`crate::imports`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The client that made it, and renews it. Absent in a session stored before there were two:
    /// a device session.
    #[serde(default, skip_serializing_if = "ClientKind::is_device")]
    pub client: ClientKind,
    /// The client an imported file says made it, when it names one (only for the import's
    /// "renews here" check; never stored).
    #[serde(skip)]
    pub(crate) named_client: Option<crate::imports::ClientTag>,
}

/// Which client a session belongs to (TIDAL renews a session only with the client that made it).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClientKind {
    /// The code sign-in's device client.
    #[default]
    Device,
    /// The browser sign-in's client (OAuth with PKCE).
    Pkce,
}

impl ClientKind {
    pub fn is_device(&self) -> bool {
        *self == ClientKind::Device
    }

    fn words(self) -> &'static str {
        match self {
            ClientKind::Device => "code sign-in",
            ClientKind::Pkce => "browser sign-in",
        }
    }
}

/// A browser sign-in on its way: what the token request needs once the address is pasted back.
/// Short-lived and per person; never shown.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BrowserPending {
    pub code_verifier: String,
    pub client_unique_key: String,
    pub expires_at: i64,
}

impl std::fmt::Debug for BrowserPending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserPending")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// The browser sign-in's client (built in; tests set their own). Its Debug names no value.
#[derive(Clone, PartialEq, Eq)]
pub struct BrowserClient {
    pub id: String,
    pub secret: String,
}

impl std::fmt::Debug for BrowserClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BrowserClient { .. }")
    }
}

impl BrowserClient {
    /// This build's, if it carries one.
    pub fn built_in() -> Option<Self> {
        crate::project_apps::tidal_pkce_client().map(|(id, secret)| Self { id, secret })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Pending {
    pub device_code: String,
    pub user_code: String,
    pub uri: String,
    pub expires_at: i64,
    pub interval: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Health {
    pub ok: bool,
    pub detail: String,
    pub at: i64,
    /// Working, but TIDAL sent AAC (lossy) where lossless was asked for.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lossy: bool,
}

/// What the node keeps for this mode (the session is a sign-in: a backup carries it only under
/// the owner's passphrase).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModeState {
    #[serde(default)]
    pub enabled: bool,
    /// `max` (up to 24-bit/192 kHz) or `high` (16-bit/44.1 kHz).
    #[serde(default)]
    pub quality: String,
    #[serde(default)]
    pub session: Option<Session>,
    #[serde(default)]
    pub login: Option<Pending>,
    /// A browser sign-in waiting for its pasted address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<BrowserPending>,
    #[serde(default)]
    pub health: Option<Health>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Client {
    pub id: String,
    pub secret: String,
    /// `settings`, `config` or `builtin` (this build's).
    pub from: String,
}

/// This person's full-tracks mode: their own sign-in, switch, quality and health.
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

pub(crate) fn update(s: &NodeState, who: &Who, f: impl FnOnce(&mut ModeState)) -> ModeState {
    let lib = s.library.lock().expect("library poisoned");
    let mut st = load(&lib, who);
    f(&mut st);
    save(&lib, who, &st);
    st
}

/// [`update`] for an interactive sign-in: the state and the vault's "new sign-in" mark (the
/// owner's only: the vault is theirs) are written under one library guard, so no vault pass sees
/// one without the other.
pub(crate) fn update_signed_in(s: &NodeState, who: &Who, service: &str, f: impl FnOnce(&mut ModeState)) {
    update_signed_in_if(s, who, service, |_| true, f);
}

/// [`update_signed_in`] only while `still` holds for the state, checked under the same guard;
/// whether it was written.
pub(crate) fn update_signed_in_if(
    s: &NodeState,
    who: &Who,
    service: &str,
    still: impl FnOnce(&ModeState) -> bool,
    f: impl FnOnce(&mut ModeState),
) -> bool {
    let lib = s.library.lock().expect("library poisoned");
    let mut st = load(&lib, who);
    if !still(&st) {
        return false;
    }
    f(&mut st);
    save(&lib, who, &st);
    if who.is_owner() {
        crate::vault_cache::new_sign_in(&lib, service);
    }
    true
}

fn config_client() -> &'static std::sync::OnceLock<Client> {
    static C: std::sync::OnceLock<Client> = std::sync::OnceLock::new();
    &C
}

/// The device client from node.toml (`[tidal] unofficial_client_id` / `unofficial_client_secret`),
/// set once at start-up; Settings override it.
pub fn set_config_client(id: &str, secret: &str) {
    if !id.trim().is_empty() {
        let _ = config_client().set(Client {
            id: id.trim().into(),
            secret: secret.trim().into(),
            from: "config".into(),
        });
    }
}

/// The device client id the code sign-in is made with (`vault_cache`): the app a device session
/// is carried in the vault under, unchanged from before there were two kinds.
pub(crate) fn app_id(s: &NodeState) -> Option<String> {
    client_of(&s.library.lock().expect("library poisoned")).map(|c| c.id)
}

/// The app a full-tracks session is carried in the vault under, by the client that made it: a
/// device session the device client's id ([`app_id`]), a browser session `pkce:` and the browser
/// client's id. None when this node has no client that renews it, so it never takes one it
/// would have to switch off.
pub(crate) fn app_of_session(s: &NodeState, session: &serde_json::Map<String, Value>) -> Option<String> {
    let kind = match session.get("client") {
        None | Some(Value::Null) => ClientKind::Device,
        Some(v) => serde_json::from_value(v.clone()).ok()?,
    };
    match kind {
        ClientKind::Device => app_id(s),
        ClientKind::Pkce => browser_client(s).map(|c| format!("pkce:{}", c.id)),
    }
}

/// The device client for the code sign-in: the owner's in Settings, else from node.toml, else the
/// `PM_TIDAL_UNOFFICIAL_CLIENT_ID` / `_SECRET` environment, else this build's (owner, 2026-11-04).
/// A client of the owner's always wins and is never swapped for the build's after a failure.
pub fn client_of(lib: &pm_library::Library) -> Option<Client> {
    let kept: Option<Client> = lib
        .get_setting(CLIENT_KEY)
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str(&v).ok());
    if let Some(c) = kept.filter(|c| !c.id.is_empty()) {
        return Some(c);
    }
    if let Some(c) = config_client().get() {
        return Some(c.clone());
    }
    let id = std::env::var("PM_TIDAL_UNOFFICIAL_CLIENT_ID").unwrap_or_default();
    let secret = std::env::var("PM_TIDAL_UNOFFICIAL_CLIENT_SECRET").unwrap_or_default();
    if !id.trim().is_empty() {
        return Some(Client {
            id: id.trim().into(),
            secret: secret.trim().into(),
            from: "config".into(),
        });
    }
    crate::project_apps::tidal_device_client().map(|(id, secret)| Client {
        id,
        secret,
        from: "builtin".into(),
    })
}

/// Whether the mode is on and signed in, so TIDAL tracks can play in full.
pub fn streams(s: &NodeState, who: &Who) -> bool {
    let st = load(&s.library.lock().expect("library poisoned"), who);
    st.enabled && st.session.is_some()
}

fn client(s: &NodeState) -> Result<Client, String> {
    client_of(&s.library.lock().expect("library poisoned"))
        .ok_or_else(|| "Set a TIDAL device client id and secret first (Sources › TIDAL › Full tracks)".into())
}

fn session_from(v: &Value, previous: Option<&Session>) -> Option<Session> {
    Some(Session {
        access_token: v["access_token"].as_str()?.to_string(),
        refresh_token: v["refresh_token"]
            .as_str()
            .map(str::to_string)
            .or_else(|| previous.and_then(|p| p.refresh_token.clone())),
        expires_at: now() + v["expires_in"].as_i64().unwrap_or(3600),
        user_id: previous.and_then(|p| p.user_id.clone()),
        country: previous.map(|p| p.country.clone()).unwrap_or_default(),
        session_id: previous.and_then(|p| p.session_id.clone()),
        from: previous.and_then(|p| p.from.clone()),
        client: previous.map(|p| p.client).unwrap_or_default(),
        named_client: None,
    })
}

/// Who the session is: TIDAL's `/sessions` (user, country, session id).
async fn fill_session(s: &NodeState, mut sess: Session) -> Session {
    if let Ok(r) = s
        .http_client()
        .get(format!("{}/sessions", v1_base(s)))
        .bearer_auth(&sess.access_token)
        .send()
        .await
    {
        if let Ok(v) = r.json::<Value>().await {
            sess.user_id = v["userId"].as_i64().map(|u| u.to_string()).or(sess.user_id);
            if let Some(c) = v["countryCode"].as_str() {
                sess.country = c.to_string();
            }
            sess.session_id = v["sessionId"].as_str().map(str::to_string).or(sess.session_id);
        }
    }
    sess
}

// ---- status and settings -----------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnofficialStatus {
    pub service: String,
    pub enabled: bool,
    pub signed_in: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// The app the sign-in was imported from, when it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub quality: String,
    /// A sign-in waiting for the code to be entered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<Health>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_from: Option<String>,
    /// The client the session in use belongs to: `pkce` (Sign in with TIDAL) or `device`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_client: Option<String>,
    /// This build can sign in with TIDAL in the browser (it carries that client).
    pub browser_available: bool,
    /// A browser sign-in waiting for its pasted address, until then (Unix seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_pending: Option<i64>,
    pub warning: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingView {
    pub user_code: String,
    pub uri: String,
    pub expires_at: i64,
}

pub(crate) fn view(s: &NodeState, who: &Who) -> UnofficialStatus {
    let browser_available = browser_client(s).is_some();
    let lib = s.library.lock().expect("library poisoned");
    let st = load(&lib, who);
    let client_from = client_of(&lib).map(|c| c.from);
    UnofficialStatus {
        service: "tidal".into(),
        enabled: st.enabled,
        signed_in: st.session.is_some(),
        user: st.session.as_ref().and_then(|x| x.user_id.clone()),
        from: st.session.as_ref().and_then(|x| x.from.clone()),
        quality: if st.quality.is_empty() {
            "max".into()
        } else {
            st.quality.clone()
        },
        pending: st
            .login
            .as_ref()
            .filter(|p| p.expires_at > now())
            .map(|p| PendingView {
                user_code: p.user_code.clone(),
                uri: p.uri.clone(),
                expires_at: p.expires_at,
            }),
        health: st.health.clone(),
        client_from,
        session_client: st.session.as_ref().map(|x| {
            match x.client {
                ClientKind::Pkce => "pkce",
                ClientKind::Device => "device",
            }
            .to_string()
        }),
        browser_available,
        browser_pending: st.browser.as_ref().map(|b| b.expires_at).filter(|e| *e > now()),
        warning: WARNING.into(),
    }
}

/// `GET /v1/sources/tidal/unofficial`.
pub(crate) async fn get_status(
    State(s): State<Arc<NodeState>>,
    axum::Extension(caller): axum::Extension<crate::accounts::Caller>,
) -> ApiResult<UnofficialStatus> {
    let who = Who::of(&caller);
    Ok(Json(view(&s, &who)))
}

#[derive(Debug, Deserialize)]
pub struct UnofficialUpdate {
    pub enabled: Option<bool>,
    pub quality: Option<String>,
    /// Replace the device client (both; an empty id forgets it and goes back to node.toml's).
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

/// `PUT /v1/sources/tidal/unofficial {enabled?, quality?, client_id?, client_secret?}`.
pub(crate) async fn put(
    State(s): State<Arc<NodeState>>,
    axum::Extension(caller): axum::Extension<crate::accounts::Caller>,
    Json(u): Json<UnofficialUpdate>,
) -> ApiResult<UnofficialStatus> {
    let who = Who::of(&caller);
    // The device client is the node's, set by its owner; the switch and quality are each person's
    if u.client_id.is_some() && !who.is_owner() {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "only the node's owner sets the TIDAL device client",
        ));
    }
    if let Some(q) = &u.quality {
        if q != "max" && q != "high" {
            return Err(ApiError::bad_request("quality is max or high"));
        }
    }
    if let Some(id) = &u.client_id {
        let lib = s.library.lock().expect("library poisoned");
        if id.trim().is_empty() {
            lib.delete_setting(CLIENT_KEY)?;
        } else {
            let c = Client {
                id: id.trim().into(),
                secret: u.client_secret.clone().unwrap_or_default().trim().into(),
                from: "settings".into(),
            };
            lib.set_setting(CLIENT_KEY, &serde_json::to_string(&c).expect("client serializes"))?;
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
    Ok(Json(view(&s, &who)))
}

#[derive(Deserialize)]
pub struct ClientCheck {
    pub client_id: String,
    pub client_secret: String,
}

/// `POST /v1/sources/tidal/unofficial/client/check {client_id, client_secret}`: asks TIDAL whether
/// this is a device client it knows, and whether the secret goes with it, without saving anything
/// and without an account: a device-code request (the ID), then one token request for that code
/// (the secret; "authorization_pending" means TIDAL took the secret and waits for a person). The
/// code is never shown, so nobody can approve it. Always 200 with `{ok, detail}`.
pub(crate) async fn check_client(
    State(s): State<Arc<NodeState>>,
    Json(c): Json<ClientCheck>,
) -> ApiResult<crate::qobuz::CheckResult> {
    let answer = |ok: bool, detail: String| Ok(Json(crate::qobuz::CheckResult { ok, detail }));
    let (id, secret) = (c.client_id.trim(), c.client_secret.trim());
    if id.is_empty() || secret.is_empty() {
        return answer(false, "Enter the device client's ID and secret.".into());
    }
    let post = |path: &str, body: String| {
        s.http_client()
            .post(format!("{}{path}", auth_base(&s)))
            .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body)
            .send()
    };
    let resp = match post(
        "/v1/oauth2/device_authorization",
        form(&[("client_id", id), ("scope", SCOPE)]),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return answer(false, format!("Couldn't reach TIDAL: {}", e.without_url())),
    };
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    let detail_of = |v: &Value| {
        v["error_description"]
            .as_str()
            .or(v["error"].as_str())
            .unwrap_or("")
            .to_string()
    };
    if !status.is_success() {
        let d = detail_of(&v);
        if d.to_ascii_lowercase().contains("limited input device") {
            return answer(false, NOT_A_DEVICE_CLIENT.into());
        }
        // Busy or down is not a verdict on the client
        if status.as_u16() == 429 || status.is_server_error() {
            return answer(
                false,
                format!("TIDAL couldn't answer right now ({status}). Try again in a moment."),
            );
        }
        return answer(
            false,
            format!(
                "TIDAL doesn't know this client ID ({status}{})",
                if d.is_empty() {
                    String::new()
                } else {
                    format!(": {d}")
                }
            ),
        );
    }
    let Some(device_code) = v["deviceCode"].as_str().filter(|d| !d.is_empty()) else {
        return answer(false, "TIDAL sent no device code".into());
    };
    let resp = match post(
        "/v1/oauth2/token",
        form(&[
            ("client_id", id),
            ("client_secret", secret),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("scope", SCOPE),
        ]),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return answer(false, format!("Couldn't reach TIDAL: {}", e.without_url())),
    };
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    Ok(Json(judge_client_check(status.as_u16(), &v)))
}

/// What TIDAL's answer to the token request of the client check means.
fn judge_client_check(status: u16, v: &Value) -> crate::qobuz::CheckResult {
    let err = v["error"].as_str().unwrap_or("");
    let (ok, detail) = match (status, err) {
        (_, "authorization_pending" | "slow_down") | (200..=299, _) => (
            true,
            "OK: TIDAL accepts this device client. Sign in with a code next.".to_string(),
        ),
        (401, _) | (_, "invalid_client" | "unauthorized_client") => (
            false,
            "TIDAL knows this client ID, but the secret doesn't go with it. Check the secret.".to_string(),
        ),
        _ => (
            false,
            format!(
                "TIDAL answered {status}{}",
                v["error_description"]
                    .as_str()
                    .or(v["error"].as_str())
                    .map(|d| format!(": {d}"))
                    .unwrap_or_default()
            ),
        ),
    };
    crate::qobuz::CheckResult { ok, detail }
}

/// `POST /v1/sources/tidal/unofficial/login`: the device code to enter at link.tidal.com; the
/// node waits for the approval in the background.
pub(crate) async fn login(
    State(s): State<Arc<NodeState>>,
    axum::Extension(caller): axum::Extension<crate::accounts::Caller>,
) -> ApiResult<UnofficialStatus> {
    let who = Who::of(&caller);
    if who == Who::Nobody {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "sign in to the node first"));
    }
    let c = client(&s).map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e))?;
    let resp = s
        .http_client()
        .post(format!("{}/v1/oauth2/device_authorization", auth_base(&s)))
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(form(&[("client_id", c.id.as_str()), ("scope", SCOPE)]))
        .send()
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, format!("TIDAL: {e}")))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let detail = v["error_description"]
            .as_str()
            .or(v["error"].as_str())
            .unwrap_or("");
        // An app from TIDAL's developer portal is not a device client: TIDAL answers 400
        // "Client is not a Limited Input Device client" to the device-code request.
        if detail.to_ascii_lowercase().contains("limited input device") {
            return Err(ApiError::bad_request(NOT_A_DEVICE_CLIENT));
        }
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            format!("TIDAL refused the sign-in ({status}): {detail}"),
        ));
    }
    let uri = v["verificationUriComplete"]
        .as_str()
        .or(v["verificationUri"].as_str())
        .unwrap_or("link.tidal.com")
        .to_string();
    let pending = Pending {
        device_code: v["deviceCode"].as_str().unwrap_or_default().to_string(),
        user_code: v["userCode"].as_str().unwrap_or_default().to_string(),
        uri: if uri.starts_with("http") {
            uri
        } else {
            format!("https://{uri}")
        },
        expires_at: now() + v["expiresIn"].as_i64().unwrap_or(300),
        interval: v["interval"].as_u64().unwrap_or(2).clamp(1, 30),
    };
    if pending.device_code.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "TIDAL sent no device code",
        ));
    }
    update(&s, &who, |st| st.login = Some(pending.clone()));
    let st2 = s.clone();
    let who2 = who.clone();
    tokio::spawn(async move { wait_for_approval(&st2, &who2, c, pending).await });
    Ok(Json(view(&s, &who)))
}

/// Waits for the code to be approved; the sign-in is kept for `who`, who asked for the code.
async fn wait_for_approval(s: &Arc<NodeState>, who: &Who, c: Client, p: Pending) {
    while now() < p.expires_at {
        tokio::time::sleep(std::time::Duration::from_secs(p.interval)).await;
        // A newer sign-in replaced this one
        let current = load(&s.library.lock().expect("library poisoned"), who).login;
        if current.as_ref().map(|x| &x.device_code) != Some(&p.device_code) {
            return;
        }
        let Ok(resp) = s
            .http_client()
            .post(format!("{}/v1/oauth2/token", auth_base(s)))
            .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(form(&[
                ("client_id", c.id.as_str()),
                ("client_secret", c.secret.as_str()),
                ("device_code", p.device_code.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("scope", SCOPE),
            ]))
            .send()
            .await
        else {
            continue;
        };
        let ok = resp.status().is_success();
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if ok {
            if let Some(sess) = session_from(&v, None) {
                let sess = fill_session(s, sess).await;
                // Only while this code is still the sign-in on the way: signed out, replaced or
                // removed (People) while TIDAL was asked, it isn't kept
                let still =
                    |st: &ModeState| st.login.as_ref().map(|x| &x.device_code) == Some(&p.device_code);
                let kept = update_signed_in_if(s, who, "tidal-full", still, |st| {
                    st.session = Some(sess);
                    st.login = None;
                    st.enabled = true;
                    st.health = Some(Health {
                        ok: true,
                        detail: "signed in".into(),
                        at: now(),
                        lossy: false,
                    });
                });
                if kept {
                    tracing::info!("TIDAL full tracks (unofficial): signed in");
                }
            }
            return;
        }
        if v["error"].as_str() == Some("expired_token") {
            break;
        }
    }
    update(s, who, |st| {
        if st.login.as_ref().map(|x| &x.device_code) == Some(&p.device_code) {
            st.login = None;
        }
    });
}

// ---- the browser sign-in ("Sign in with TIDAL", OAuth 2 with PKCE) -------------------------

/// The browser sign-in's client: this build's (tests set their own on the endpoints).
pub(crate) fn browser_client(s: &NodeState) -> Option<BrowserClient> {
    s.tidal.read().expect("tidal endpoints").browser_client.clone()
}

fn login_base(s: &NodeState) -> String {
    s.tidal
        .read()
        .expect("tidal endpoints")
        .login_base
        .trim_end_matches('/')
        .to_string()
}

/// Why a build can't sign in with TIDAL in the browser.
pub const NO_BROWSER_CLIENT: &str = "This build has no client for Sign in with TIDAL (a release build carries it). Use the code sign-in instead.";

/// PKCE's S256 challenge of a verifier: base64url, no padding, of its SHA-256.
pub(crate) fn code_challenge(verifier: &str) -> String {
    use sha2::Digest;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()))
}

/// A new attempt: python-tidal's verifier (32 random bytes, base64url without padding) and
/// client unique key (64 random bits in hex).
fn new_browser_pending() -> BrowserPending {
    use rand::RngCore;
    let mut rng = rand::thread_rng();
    let mut bytes = [0u8; 32];
    rng.fill_bytes(&mut bytes);
    BrowserPending {
        code_verifier: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes),
        client_unique_key: format!("{:02x}", rng.next_u64()),
        expires_at: now() + BROWSER_WAIT_SECS,
    }
}

/// The login.tidal.com address for an attempt, with python-tidal's `pkce_login_url` parameters
/// in its order.
pub(crate) fn authorize_url(login_base: &str, client_id: &str, p: &BrowserPending) -> String {
    let challenge = code_challenge(&p.code_verifier);
    format!(
        "{login_base}/authorize?{}",
        form(&[
            ("response_type", "code"),
            ("redirect_uri", PKCE_REDIRECT),
            ("client_id", client_id),
            ("lang", "EN"),
            ("appMode", "android"),
            ("client_unique_key", p.client_unique_key.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("restrict_signup", "true"),
        ])
    )
}

/// The sign-in code from what was pasted back: the address of the tidal.com page TIDAL showed
/// (https, on tidal.com or one of its subdomains, checked before anything is read from it), or the
/// bare code. The error is for the person, in plain words.
pub(crate) fn code_from_paste(text: &str) -> Result<String, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("Paste the address of the TIDAL page you landed on after signing in.".into());
    }
    let looks_like_address = t.contains("://") || t.contains('/') || t.contains('?');
    if !looks_like_address {
        // A bare code: what TIDAL issues is URL-safe
        let fine = t.len() <= 2048
            && t.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~'));
        return if fine {
            Ok(t.to_string())
        } else {
            Err("That is neither a TIDAL address nor a sign-in code. Paste the whole address of the TIDAL page you landed on.".into())
        };
    }
    let with_scheme = if t.contains("://") {
        t.to_string()
    } else {
        format!("https://{t}")
    };
    let url = url::Url::parse(&with_scheme).map_err(|_| {
        "That address can't be read. Paste the whole address of the TIDAL page you landed on.".to_string()
    })?;
    if url.scheme() != "https" {
        return Err("That address doesn't start with https://. Paste the address of the TIDAL page exactly as your browser shows it.".into());
    }
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host != "tidal.com" && !host.ends_with(".tidal.com") {
        return Err(format!(
            "That isn't a TIDAL address ({}). Paste the address of the tidal.com page you landed on after signing in.",
            if host.is_empty() { "no host" } else { host.as_str() }
        ));
    }
    let q = |k: &str| {
        url.query_pairs()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.into_owned())
    };
    if let Some(e) = q("error") {
        let why = q("error_description").unwrap_or(e);
        return Err(format!(
            "TIDAL didn't sign you in: {}. Press Sign in with TIDAL to try again.",
            why.chars().take(200).collect::<String>()
        ));
    }
    match q("code").map(|c| c.trim().to_string()).filter(|c| !c.is_empty()) {
        Some(code) => Ok(code),
        None => Err("That TIDAL address has no sign-in code in it. Sign in first; the page TIDAL shows afterwards carries it (its address has ?code=…).".into()),
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BrowserStart {
    /// Open this in a new tab: TIDAL's sign-in.
    pub url: String,
    /// Paste the address back before then (Unix seconds).
    pub expires_at: i64,
}

/// `POST /v1/sources/tidal/unofficial/browser`: starts this person's browser sign-in (a new
/// attempt replaces one on its way) and answers TIDAL's sign-in address to open.
pub(crate) async fn start_browser(
    State(s): State<Arc<NodeState>>,
    axum::Extension(caller): axum::Extension<crate::accounts::Caller>,
) -> ApiResult<BrowserStart> {
    let who = Who::of(&caller);
    if who == Who::Nobody {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "sign in to the node first"));
    }
    let Some(c) = browser_client(&s) else {
        return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, NO_BROWSER_CLIENT));
    };
    let pending = new_browser_pending();
    let url = authorize_url(&login_base(&s), &c.id, &pending);
    let expires_at = pending.expires_at;
    update(&s, &who, |st| st.browser = Some(pending));
    Ok(Json(BrowserStart { url, expires_at }))
}

#[derive(Debug, Deserialize)]
pub struct BrowserFinish {
    /// The pasted address of the page TIDAL landed on, or the bare code.
    pub text: String,
}

/// `POST /v1/sources/tidal/unofficial/browser/finish {text}`: the pasted address (or code) of the
/// browser sign-in; the node takes the code for tokens and keeps them as this person's full-tracks
/// session (renewed with the browser sign-in's client).
pub(crate) async fn finish_browser(
    State(s): State<Arc<NodeState>>,
    axum::Extension(caller): axum::Extension<crate::accounts::Caller>,
    Json(b): Json<BrowserFinish>,
) -> ApiResult<UnofficialStatus> {
    let who = Who::of(&caller);
    if who == Who::Nobody {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "sign in to the node first"));
    }
    let Some(c) = browser_client(&s) else {
        return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, NO_BROWSER_CLIENT));
    };
    let pending = load(&s.library.lock().expect("library poisoned"), &who).browser;
    let Some(p) = pending else {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "No sign-in with TIDAL is on its way here. Press Sign in with TIDAL, sign in, then paste the address.",
        ));
    };
    if p.expires_at <= now() {
        update(&s, &who, |st| {
            if st.browser.as_ref() == Some(&p) {
                st.browser = None;
            }
        });
        return Err(ApiError::new(
            StatusCode::GONE,
            "That sign-in waited too long and expired. Press Sign in with TIDAL to start again.",
        ));
    }
    let code = code_from_paste(&b.text).map_err(ApiError::bad_request)?;
    // python-tidal's `pkce_get_auth_token`: no client secret in the exchange
    let resp = s
        .http_client()
        .post(format!("{}/v1/oauth2/token", auth_base(&s)))
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(form(&[
            ("code", code.as_str()),
            ("client_id", c.id.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", PKCE_REDIRECT),
            ("scope", PKCE_SCOPE),
            ("code_verifier", p.code_verifier.as_str()),
            ("client_unique_key", p.client_unique_key.as_str()),
        ]))
        .send()
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                format!("Couldn't reach TIDAL: {}", e.without_url()),
            )
        })?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        if status.as_u16() == 429 || status.is_server_error() {
            return Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                format!("TIDAL couldn't answer right now ({status}). Paste the address again in a moment."),
            ));
        }
        let why = v["error_description"]
            .as_str()
            .or(v["error"].as_str())
            .unwrap_or("")
            .chars()
            .take(200)
            .collect::<String>();
        return Err(ApiError::bad_request(format!(
            "TIDAL didn't accept that sign-in code ({status}{}). A code works once and only for a few minutes: press Sign in with TIDAL to get a new one.",
            if why.is_empty() { String::new() } else { format!(": {why}") }
        )));
    }
    let Some(mut sess) = session_from(&v, None) else {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "TIDAL's answer had no access token",
        ));
    };
    sess.client = ClientKind::Pkce;
    // The token answer names the user; /sessions adds the session id and country
    sess.user_id = v["user_id"]
        .as_i64()
        .or(v["user"]["userId"].as_i64())
        .map(|u| u.to_string());
    if let Some(cc) = v["user"]["countryCode"].as_str() {
        sess.country = cc.to_string();
    }
    let sess = fill_session(&s, sess).await;
    // Only while this attempt is still the one on its way (signed out, restarted or removed
    // (People) while TIDAL was asked: not kept)
    let still = |st: &ModeState| st.browser.as_ref() == Some(&p);
    let kept = update_signed_in_if(&s, &who, "tidal-full", still, |st| {
        st.session = Some(sess);
        st.browser = None;
        st.login = None;
        st.enabled = true;
        st.health = Some(Health {
            ok: true,
            detail: "signed in".into(),
            at: now(),
            lossy: false,
        });
    });
    if !kept {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "The sign-in changed meanwhile (signed out or started again here). Press Sign in with TIDAL to start again.",
        ));
    }
    let _ = who.put(
        &s.library.lock().expect("library poisoned"),
        &crate::signin::way_key(crate::tidal::SOURCE_ID),
        Some(crate::tidal::WAY_BROWSER),
    );
    tracing::info!("TIDAL full tracks: signed in with TIDAL (browser sign-in)");
    Ok(Json(view(&s, &who)))
}

/// `DELETE /v1/sources/tidal/unofficial/login`: forget the session (the mode stays as set).
pub(crate) async fn logout(
    State(s): State<Arc<NodeState>>,
    axum::Extension(caller): axum::Extension<crate::accounts::Caller>,
) -> ApiResult<UnofficialStatus> {
    let who = Who::of(&caller);
    update(&s, &who, |st| {
        st.session = None;
        st.login = None;
        st.browser = None;
    });
    Ok(Json(view(&s, &who)))
}

// ---- playback (the audio thread and the browser proxy) -----------------------------------------

/// A full stream: the reader, the extension the decoder probes with, and the media type.
pub struct FullStream {
    pub reader: Box<dyn std::io::Read + Send>,
    /// A segmented (DASH) stream's segments, so a zone can seek in it; `reader` reads them all.
    pub segments: Option<crate::segmented::Segments>,
    pub ext: &'static str,
    pub mime: &'static str,
    /// TIDAL sent AAC (or another lossy codec) where lossless was asked for.
    pub lossy: bool,
    /// The sign-in it was asked with.
    pub client: ClientKind,
    /// The sign-in was imported from another app or pasted (not made on this node).
    pub imported: bool,
    /// This build has the browser client (Sign in with TIDAL is possible).
    pub browser: bool,
    /// What TIDAL answered.
    pub answer: Answer,
}

impl FullStream {
    /// The health line for how it played: lossless asked for (Max or High) and AAC came means
    /// the code sign-in for a device session (Sign in with TIDAL is the way to lossless, where
    /// this build has it), and a track with no lossless version for a browser one.
    pub fn health_line(&self) -> &'static str {
        match (self.lossy, self.client) {
            (false, _) => "full tracks are playing",
            (true, ClientKind::Pkce) => AAC_TRACK,
            (true, ClientKind::Device) if !self.browser => AAC_ONLY_NO_BROWSER,
            (true, ClientKind::Device) if self.imported => AAC_ONLY_IMPORTED,
            (true, ClientKind::Device) => AAC_ONLY_DEVICE,
        }
    }
}

/// Whether a failure is the mode's (switch it off) or only this track's.
#[derive(Debug)]
pub enum FullError {
    Mode(String),
    Track(String),
}

fn token_blocking(
    library: &std::sync::Mutex<pm_library::Library>,
    auth: &str,
    client: &reqwest::blocking::Client,
    who: &Who,
    browser: Option<&BrowserClient>,
) -> Result<Option<(Session, String)>, FullError> {
    let st = {
        let lib = library.lock().map_err(|_| FullError::Track("library".into()))?;
        load(&lib, who)
    };
    if !st.enabled {
        return Ok(None);
    }
    let Some(sess) = st.session.clone() else {
        return Ok(None);
    };
    let quality = if st.quality == "high" {
        "LOSSLESS"
    } else {
        "HI_RES_LOSSLESS"
    }
    .to_string();
    if sess.expires_at - now() > 60 {
        return Ok(Some((sess, quality)));
    }
    let Some(refresh) = sess.refresh_token.clone() else {
        return Err(FullError::Mode("the session ran out: sign in again".into()));
    };
    // Renewed with the client that made it: python-tidal's `token_refresh` (the PKCE pair for a
    // browser sign-in, no scope), the device client with the scope for a code sign-in
    let body = match sess.client {
        ClientKind::Pkce => {
            let c = browser.ok_or_else(|| {
                FullError::Mode(
                    "this build has no client for Sign in with TIDAL, so it can't renew that sign-in: sign in again with the code sign-in"
                        .into(),
                )
            })?;
            form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh.as_str()),
                ("client_id", c.id.as_str()),
                ("client_secret", c.secret.as_str()),
            ])
        }
        ClientKind::Device => {
            let c: Client = library
                .lock()
                .ok()
                .and_then(|l| client_of(&l))
                .ok_or_else(|| {
                    FullError::Mode(
                        "no TIDAL device client on this node to renew the code sign-in: Sign in with TIDAL, or add one under Sources › TIDAL › Sign in › Advanced"
                            .into(),
                    )
                })?;
            form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh.as_str()),
                ("client_id", c.id.as_str()),
                ("client_secret", c.secret.as_str()),
                ("scope", SCOPE),
            ])
        }
    };
    let resp = client
        .post(format!("{auth}/v1/oauth2/token"))
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .map_err(|e| FullError::Track(format!("renewing the session: {}", e.without_url())))?;
    let status = resp.status();
    let v: Value = resp.json().unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(FullError::Mode(format!(
            "TIDAL did not renew the session ({status}): {}",
            v["error_description"]
                .as_str()
                .or(v["error"].as_str())
                .unwrap_or("")
        )));
    }
    let fresh =
        session_from(&v, Some(&sess)).ok_or_else(|| FullError::Mode("renewal without a token".into()))?;
    if let Ok(lib) = library.lock() {
        let mut st = load(&lib, who);
        // Only over the session this renewal started from (the vault may have changed it
        // meanwhile); otherwise the session there now is the one to use
        if st
            .session
            .as_ref()
            .is_some_and(|c| c.access_token == sess.access_token)
        {
            st.session = Some(fresh.clone());
            save(&lib, who, &st);
        } else {
            tracing::info!("a renewed TIDAL full-tracks session was dropped: the sign-in changed meanwhile");
            return Ok(st.session.map(|c| (c, quality)));
        }
    }
    Ok(Some((fresh, quality)))
}

/// What TIDAL answered for a track, as the log and the health line say it: never a URL or token.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Answer {
    /// The `audioquality` asked for.
    pub asked: String,
    /// `audioQuality` (`HI_RES_LOSSLESS`, `LOSSLESS`, `HIGH`, `LOW`).
    pub audio_quality: String,
    /// `manifestMimeType`.
    pub manifest: String,
    /// The BTS manifest's `codecs`, or the DASH representation's `codecs` attribute.
    pub codecs: String,
    pub bit_depth: Option<u64>,
    pub sample_rate: Option<u64>,
}

impl Answer {
    /// AAC (or another lossy codec) instead of FLAC.
    pub fn lossy(&self) -> bool {
        let c = self.codecs.to_ascii_lowercase();
        if c.is_empty() {
            return matches!(self.audio_quality.as_str(), "HIGH" | "LOW");
        }
        !(c.starts_with("flac") || c.starts_with("alac") || c.starts_with("mqa"))
    }
}

impl std::fmt::Display for Answer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let or = |v: &str| {
            if v.is_empty() {
                "-".to_string()
            } else {
                v.to_string()
            }
        };
        write!(
            f,
            "audioQuality={} manifestMimeType={} codecs={}",
            or(&self.audio_quality),
            or(&self.manifest),
            or(&self.codecs)
        )?;
        if let Some(b) = self.bit_depth {
            write!(f, " bitDepth={b}")?;
        }
        if let Some(r) = self.sample_rate {
            write!(f, " sampleRate={r}")?;
        }
        Ok(())
    }
}

/// Where the audio of one answer is.
enum Media {
    /// One file (BTS): its URL, the decoder's extension, the media type.
    File(String, &'static str, &'static str),
    /// DASH: the initialization segment, then the media segments, with their times.
    Segments(crate::segmented::Segments),
}

/// Asks the private playback endpoint for one track at one quality (python-tidal's
/// `Track.get_stream`, with its Android web view's User-Agent and client version).
fn playback_info(
    client: &reqwest::blocking::Client,
    v1: &str,
    sess: &Session,
    id: &str,
    quality: &str,
) -> Result<Value, FullError> {
    let mut q: Vec<(&str, String)> = vec![
        ("playbackmode", "STREAM".into()),
        ("audioquality", quality.into()),
        ("assetpresentation", "FULL".into()),
        ("countryCode", sess.country.clone()),
    ];
    if let Some(sid) = &sess.session_id {
        q.push(("sessionId", sid.clone()));
    }
    let resp = client
        .get(format!("{v1}/tracks/{id}/playbackinfopostpaywall"))
        .bearer_auth(&sess.access_token)
        .header(reqwest::header::USER_AGENT, PLAYBACK_USER_AGENT)
        .header("x-tidal-client-version", PLAYBACK_CLIENT_VERSION)
        .query(&q)
        .send()
        .map_err(|e| FullError::Track(format!("playback info: {}", e.without_url())))?;
    let status = resp.status();
    let v: Value = resp.json().unwrap_or(Value::Null);
    match status.as_u16() {
        200..=299 => Ok(v),
        404 => Err(FullError::Track("TIDAL has no stream for this track".into())),
        401 | 403 => Err(FullError::Mode(format!(
            "TIDAL refused the private playback ({status}): {}",
            v["userMessage"].as_str().or(v["error"].as_str()).unwrap_or("")
        ))),
        _ => Err(FullError::Track(format!("playback info answered {status}"))),
    }
}

/// python-tidal's request headers for the private API (`tidalapi/request.py`).
const PLAYBACK_USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 12; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/91.0.4472.114 Safari/537.36";
const PLAYBACK_CLIENT_VERSION: &str = "2025.7.16";

/// What one playback-info answer says and where its audio is. An encrypted stream is refused,
/// never decrypted.
fn read_answer(v: &Value, asked: &str) -> Result<(Answer, Media), FullError> {
    if v["assetPresentation"].as_str().is_some_and(|p| p != "FULL") {
        return Err(FullError::Track(
            "TIDAL sent a preview: the plan may not include this track".into(),
        ));
    }
    let mut answer = Answer {
        asked: asked.into(),
        audio_quality: v["audioQuality"].as_str().unwrap_or_default().into(),
        manifest: v["manifestMimeType"].as_str().unwrap_or_default().into(),
        codecs: String::new(),
        bit_depth: v["bitDepth"].as_u64(),
        sample_rate: v["sampleRate"].as_u64(),
    };
    let manifest = base64::engine::general_purpose::STANDARD
        .decode(v["manifest"].as_str().unwrap_or_default())
        .map_err(|_| FullError::Mode("playback info without a readable manifest".into()))?;
    let manifest = String::from_utf8_lossy(&manifest).into_owned();
    if answer.manifest == "application/vnd.tidal.bts" {
        let m: Value = serde_json::from_str(&manifest)
            .map_err(|_| FullError::Mode("BTS manifest is not JSON".into()))?;
        answer.codecs = m["codecs"].as_str().unwrap_or_default().to_string();
        let enc = m["encryptionType"].as_str().unwrap_or("NONE");
        if enc != "NONE" {
            // Never decrypted: no DRM circumvention
            return Err(FullError::Track(format!(
                "TIDAL sends this track encrypted ({enc})"
            )));
        }
        let url = m["urls"][0]
            .as_str()
            .ok_or_else(|| FullError::Mode("BTS manifest without a URL".into()))?;
        let (ext, mime) = if answer.codecs.to_ascii_lowercase().starts_with("flac") {
            ("flac", "audio/flac")
        } else {
            ("m4a", "audio/mp4")
        };
        return Ok((answer, Media::File(url.to_string(), ext, mime)));
    }
    if answer.manifest == "application/dash+xml" {
        if manifest.contains("ContentProtection") {
            return Err(FullError::Track("TIDAL sends this track encrypted".into()));
        }
        let d = crate::dash::parse(&manifest).map_err(FullError::Mode)?;
        let segments = d.segments();
        answer.codecs = d.codecs;
        if answer.sample_rate.is_none() {
            answer.sample_rate = d.sample_rate.map(u64::from);
        }
        return Ok((answer, Media::Segments(segments)));
    }
    Err(FullError::Mode(format!(
        "unknown manifest type {:?}",
        answer.manifest
    )))
}

/// Open the full track through the private playback endpoint. `Ok(None)`: the mode is off or
/// not signed in (use the official path). AAC where HI_RES_LOSSLESS was asked for is asked once
/// more as LOSSLESS before it is played; one line in the log says what TIDAL answered.
pub fn open_blocking(
    library: &std::sync::Mutex<pm_library::Library>,
    ep: &crate::tidal::TidalEndpoints,
    id: &str,
    who: &Who,
) -> Result<Option<FullStream>, FullError> {
    let client = crate::sources::http_client();
    let auth = ep.auth_base.trim_end_matches('/');
    let v1 = ep.v1_base.trim_end_matches('/');
    let Some((sess, quality)) = token_blocking(library, auth, client, who, ep.browser_client.as_ref())?
    else {
        return Ok(None);
    };
    let v = playback_info(client, v1, &sess, id, &quality)?;
    let (mut answer, mut media) = read_answer(&v, &quality)?;
    let mut first = None;
    if quality == "HI_RES_LOSSLESS" && answer.lossy() {
        let again =
            playback_info(client, v1, &sess, id, "LOSSLESS").and_then(|v| read_answer(&v, "LOSSLESS"));
        match again {
            Ok((a, m)) => {
                first = Some(format!("{answer}"));
                (answer, media) = (a, m);
            }
            Err(e) => {
                tracing::info!(
                    "TIDAL full track {id}: asking LOSSLESS after AAC failed ({}); playing the AAC answer",
                    match &e {
                        FullError::Mode(x) | FullError::Track(x) => x.as_str(),
                    }
                );
            }
        }
    }
    tracing::info!(
        "TIDAL full track {id} ({}): asked {}, TIDAL answered {answer}{}",
        sess.client.words(),
        answer.asked,
        first
            .map(|f| format!(" (HI_RES_LOSSLESS had answered {f})"))
            .unwrap_or_default()
    );
    let reader: Box<dyn std::io::Read + Send> = match &media {
        Media::File(url, ..) => Box::new(
            client
                .get(url)
                .send()
                .and_then(|r| r.error_for_status())
                .map_err(|e| FullError::Track(format!("stream: {}", e.without_url())))?,
        ),
        Media::Segments(segs) => Box::new(crate::segmented::SegmentReader::new(client, segs.urls())),
    };
    let (ext, mime, segments) = match media {
        Media::File(_, ext, mime) => (ext, mime, None),
        Media::Segments(segs) => ("m4a", "audio/mp4", Some(segs)),
    };
    Ok(Some(FullStream {
        reader,
        segments,
        ext,
        mime,
        lossy: answer.lossy(),
        client: sess.client,
        imported: sess.from.is_some(),
        browser: ep.browser_client.is_some(),
        answer,
    }))
}

/// Record how the mode did: a failure of the mode itself switches it off (the page says why;
/// Retry switches it on again).
pub fn record(
    library: &std::sync::Mutex<pm_library::Library>,
    who: &Who,
    r: &Result<Option<FullStream>, FullError>,
) {
    let Ok(lib) = library.lock() else { return };
    let mut st = load(&lib, who);
    match r {
        Ok(Some(f)) => {
            st.health = Some(Health {
                ok: true,
                detail: f.health_line().into(),
                at: now(),
                lossy: f.lossy,
            });
        }
        Err(FullError::Mode(e)) => {
            tracing::warn!("TIDAL full tracks (unofficial) switched off: {e}");
            st.enabled = false;
            st.health = Some(Health {
                ok: false,
                detail: e.clone(),
                at: now(),
                lossy: false,
            });
        }
        _ => return,
    }
    save(&lib, who, &st);
}
