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
use crate::person_store::PersonStore;
use crate::state::NodeState;
use crate::{ApiError, ApiResult};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use pm_sources::form;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

// The sign-ins, renewal and playback themselves are shared with the phone (pm-sources).
pub use pm_sources::tidal::full::{
    authorize_url, code_challenge, code_from_paste, exchange_browser_code, judge_client_check,
    new_browser_pending, read_answer, session_from, Answer, Approval, BrowserClient, BrowserPending, Client,
    ClientKind, FullError, FullStream, Health, Media, ModeState, Pending, PendingView, Session,
    UnofficialStatus, AAC_ONLY_DEVICE, AAC_ONLY_IMPORTED, AAC_ONLY_NO_BROWSER, AAC_TRACK, CLIENT_KEY,
    NOT_A_DEVICE_CLIENT, NO_BROWSER_CLIENT, PKCE_REDIRECT, PKCE_SCOPE, SCOPE, STATE_KEY, WARNING,
};

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

fn now() -> i64 {
    pm_library::db::now_unix()
}

/// This person's full-tracks mode: their own sign-in, switch, quality and health.
pub fn load(lib: &pm_library::Library, who: &Who) -> ModeState {
    who.get(lib, STATE_KEY)
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

fn save(lib: &pm_library::Library, who: &Who, st: &ModeState) {
    let _ = try_save(lib, who, st);
}

/// [`save`], saying when the state was not kept (someone no longer listed, or a sign-in with an
/// account that is someone else's here: accounts.rs).
fn try_save(lib: &pm_library::Library, who: &Who, st: &ModeState) -> Result<(), ApiError> {
    who.put(
        lib,
        STATE_KEY,
        Some(&serde_json::to_string(st).expect("state serializes")),
    )
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
pub(crate) fn update_signed_in(
    s: &NodeState,
    who: &Who,
    service: &str,
    f: impl FnOnce(&mut ModeState),
) -> Result<(), ApiError> {
    update_signed_in_if(s, who, service, |_| true, f).map(|_| ())
}

/// [`update_signed_in`] only while `still` holds for the state, checked under the same guard;
/// whether it was written, or why the sign-in was not kept.
pub(crate) fn update_signed_in_if(
    s: &NodeState,
    who: &Who,
    service: &str,
    still: impl FnOnce(&ModeState) -> bool,
    f: impl FnOnce(&mut ModeState),
) -> Result<bool, ApiError> {
    let lib = s.library.lock().expect("library poisoned");
    let mut st = load(&lib, who);
    if !still(&st) {
        return Ok(false);
    }
    f(&mut st);
    try_save(&lib, who, &st)?;
    if who.is_owner() {
        crate::vault_cache::new_sign_in(&lib, service);
    }
    Ok(true)
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

/// A sign-in of someone other than the owner, before it is stored: whose account it is must be
/// known, and it must be nobody else's on this node ([`Who::account_free`]). `asked` says the
/// account in `sess` is TIDAL's own answer. A pasted sign-in's is not: TIDAL is asked, and only
/// when it does not answer (a sign-in that has run out and is renewed at its first play) the
/// account is the one the sign-in names itself; the renewal then holds it to TIDAL's answer
/// ([`renewed_for`]). The owner's sign-ins pass as they are.
pub(crate) async fn held_to_its_person(
    s: &NodeState,
    who: &Who,
    mut sess: Session,
    asked: bool,
) -> Result<Session, ApiError> {
    if who.is_owner() {
        return Ok(sess);
    }
    if !asked {
        // TIDAL's answer first; else the account the pasted token names itself; what the paste
        // says next to it only when neither does
        let stated = sess.user_id.take();
        sess = fill_session(s, sess).await;
        sess.unconfirmed = sess.user_id.is_none();
        sess.user_id = sess
            .user_id
            .or_else(|| token_account(&sess.access_token))
            .or(stated);
    }
    // (an answer that names nobody next to its token: the token names the account)
    sess.user_id = sess
        .user_id
        .as_deref()
        .and_then(crate::accounts::account_id)
        .or_else(|| token_account(&sess.access_token));
    let Some(account) = sess.user_id.as_deref() else {
        return Err(if asked {
            crate::accounts::account_unknown("TIDAL")
        } else {
            ApiError::bad_request(
                "This sign-in doesn't say whose TIDAL account it is, and TIDAL didn't either (it may have run out). Use Sign in with TIDAL instead.",
            )
        });
    };
    who.account_free(&s.library.lock().expect("library poisoned"), STATE_KEY, account)?;
    Ok(sess)
}

/// Ends every unconfirmed pasted sign-in (`Session::unconfirmed`) of someone other than `me` that
/// claims `account`: a sign-in TIDAL vouched for holds it now. Its health line says why. Written
/// directly, not through `Who::put` (nothing is added that needs checking).
pub(crate) fn end_claims_on(lib: &pm_library::Library, me: &Who, account: &str) -> Result<(), ApiError> {
    let listed = crate::people::listed_keys(lib)?;
    let holders = std::iter::once(Who::Owner).chain(listed.into_iter().map(Who::Listener));
    for who in holders.filter(|w| w.own() != me.own()) {
        let mut st = load(lib, &who);
        let claims = st.session.as_ref().is_some_and(|c| {
            c.unconfirmed
                && c.user_id
                    .as_deref()
                    .and_then(crate::accounts::account_id)
                    .as_deref()
                    == Some(account)
        });
        if !claims {
            continue;
        }
        st.session = None;
        st.health = Some(Health {
            ok: false,
            detail: "This TIDAL account was signed in on this node by someone else, with TIDAL's own \
                     confirmation. Sign in with your own account."
                .into(),
            at: now(),
            lossy: false,
        });
        let Some(key) = who.setting(STATE_KEY) else {
            continue;
        };
        lib.set_setting(&key, &serde_json::to_string(&st).expect("state serializes"))?;
    }
    Ok(())
}

/// The account a TIDAL token names itself (its `uid` claim).
fn token_account(access_token: &str) -> Option<String> {
    match &crate::tidal::jwt_claims(access_token)["uid"] {
        Value::Number(n) => crate::accounts::account_id(&n.to_string()),
        Value::String(s) => crate::accounts::account_id(s),
        _ => None,
    }
}

/// The account a renewal's answer names, when it is another one than the session was kept
/// under (a pasted sign-in whose renewal belongs to a different account): for someone other
/// than the owner it must be nobody else's on this node, as at sign-in. `Err` is why not.
pub(crate) fn renewed_for(
    lib: &pm_library::Library,
    who: &Who,
    answer: &Value,
    kept: &Session,
) -> Result<Option<String>, (StatusCode, String)> {
    // (TIDAL writes the id as a number; text is read too, and both the same way)
    let id = |v: &Value| match v {
        Value::Number(n) => crate::accounts::account_id(&n.to_string()),
        Value::String(s) => crate::accounts::account_id(s),
        _ => None,
    };
    // (and when the answer names none next to its token, the token does)
    let named = id(&answer["user_id"])
        .or_else(|| id(&answer["user"]["userId"]))
        .or_else(|| answer["access_token"].as_str().and_then(token_account));
    let was = kept.user_id.as_deref().and_then(crate::accounts::account_id);
    match named {
        // (one only the paste vouched for is checked now, the same account or not)
        Some(account) if kept.unconfirmed || was.as_deref() != Some(account.as_str()) => {
            if !who.is_owner() {
                who.account_free(lib, STATE_KEY, &account)
                    .map_err(|e| (e.status, e.message))?;
            }
            Ok(Some(account))
        }
        _ => Ok(None),
    }
}

/// Who the session is: TIDAL's `/sessions` (user, country, session id).
async fn fill_session(s: &NodeState, sess: Session) -> Session {
    pm_sources::tidal::full::fill_session(&s.http_client(), &v1_base(s), sess).await
}

// ---- status and settings -----------------------------------------------------------------------

pub(crate) fn view(s: &NodeState, who: &Who) -> UnofficialStatus {
    let browser_available = browser_client(s).is_some();
    let lib = s.library.lock().expect("library poisoned");
    let st = load(&lib, who);
    let client_from = client_of(&lib).map(|c| c.from);
    pm_sources::tidal::full::status(&st, client_from, browser_available)
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
) -> ApiResult<crate::types::CheckResult> {
    let answer = |ok: bool, detail: String| Ok(Json(crate::types::CheckResult { ok, detail }));
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
    let pending = pm_sources::tidal::full::device_authorization(&s.http_client(), &auth_base(&s), &c).await?;
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
        let sess = match pm_sources::tidal::full::poll_approval(&s.http_client(), &auth_base(s), &c, &p).await
        {
            Approval::Waiting => continue,
            Approval::Expired => break,
            Approval::Approved(None) => return,
            Approval::Approved(Some(sess)) => sess,
        };
        let sess = fill_session(s, sess).await;
        // Someone else's account on this node, or one TIDAL did not name, is not kept
        // for a person (accounts.rs): the code is given up, and the page says why
        let sess = match held_to_its_person(s, who, sess, true).await {
            Ok(sess) => sess,
            Err(e) => {
                update(s, who, |st| {
                    if st.login.as_ref().map(|x| &x.device_code) == Some(&p.device_code) {
                        st.login = None;
                        st.health = Some(Health {
                            ok: false,
                            detail: e.message.clone(),
                            at: now(),
                            lossy: false,
                        });
                    }
                });
                return;
            }
        };
        // Only while this code is still the sign-in on the way: signed out, replaced or
        // removed (People) while TIDAL was asked, it isn't kept
        let still = |st: &ModeState| st.login.as_ref().map(|x| &x.device_code) == Some(&p.device_code);
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
        match kept {
            Ok(true) => tracing::info!("TIDAL full tracks (unofficial): signed in"),
            Ok(false) => {}
            // Not kept after all (the account became someone else's meanwhile, or
            // this person is no longer listed): the code is given up, with why
            Err(e) => {
                update(s, who, |st| {
                    if st.login.as_ref().map(|x| &x.device_code) == Some(&p.device_code) {
                        st.login = None;
                        st.health = Some(Health {
                            ok: false,
                            detail: e.message.clone(),
                            at: now(),
                            lossy: false,
                        });
                    }
                });
            }
        }
        return;
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
    let sess = exchange_browser_code(&s.http_client(), &auth_base(&s), &c, &p, &code).await?;
    let sess = fill_session(&s, sess).await;
    // Someone else's account on this node, or one TIDAL did not name, is not kept for a person
    let sess = held_to_its_person(&s, &who, sess, true).await?;
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
    })?;
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

/// What a missing device client says when a code sign-in needs renewing.
const NO_DEVICE_CLIENT: &str = "no TIDAL device client on this node to renew the code sign-in: Sign in with TIDAL, or add one under Sources › TIDAL › Sign in › Advanced";

/// This person's session and the quality to ask for, renewed first when it is about to run out
/// (or is a paste TIDAL never vouched for). The renewal is pm-sources' (the phone's too); what the
/// node adds is its per-person rule: the account a renewal names is held to it as at sign-in
/// ([`renewed_for`]), checked and written under one library guard. `Ok(None)`: the mode is off or
/// not signed in.
fn token_blocking(
    library: &pm_library::LibraryLock,
    client: &reqwest::blocking::Client,
    ep: &pm_sources::tidal::full::Endpoints<'_>,
    who: &Who,
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
    let quality = pm_sources::tidal::full::quality_of(&st).to_string();
    // (a paste TIDAL never vouched for is renewed, and so checked, before its first play)
    if sess.expires_at - now() > 60 && !sess.unconfirmed {
        return Ok(Some((sess, quality)));
    }
    let (mut fresh, v) = pm_sources::tidal::full::renew_answer(client, ep, &sess, None)?;
    if let Ok(lib) = library.lock() {
        let mut st = load(&lib, who);
        // The renewal names another account than the sign-in was kept under: held to the same
        // rule as a sign-in (accounts.rs). Someone else's is not kept: the sign-in ends here
        match renewed_for(&lib, who, &v, &sess) {
            Ok(Some(account)) => {
                fresh.user_id = Some(account);
                fresh.unconfirmed = false;
            }
            Ok(None) => {}
            // (a store that can't be read now ends nothing: the renewal fails and is tried again)
            // (as a failed renewal: full tracks stay on, the next play tries again)
            Err((status, why)) if status != StatusCode::CONFLICT => return Err(FullError::Track(why)),
            Err((_, why)) => {
                if st
                    .session
                    .as_ref()
                    .is_some_and(|c| c.access_token == sess.access_token)
                {
                    st.session = None;
                    st.health = Some(Health {
                        ok: false,
                        detail: why.clone(),
                        at: now(),
                        lossy: false,
                    });
                    save(&lib, who, &st);
                }
                return Err(FullError::Mode(why));
            }
        }
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
            // (one TIDAL never vouched for plays only once a renewal has checked it)
            return Ok(st.session.filter(|c| !c.unconfirmed).map(|c| (c, quality)));
        }
    }
    // Still only what the paste claimed (TIDAL's renewal named no account): it doesn't play
    if fresh.unconfirmed {
        return Err(FullError::Mode(
            "TIDAL didn't say whose account this pasted sign-in is, so it doesn't play here: use Sign in with TIDAL"
                .into(),
        ));
    }
    Ok(Some((fresh, quality)))
}

/// Open the full track through the private playback endpoint (`pm_sources::tidal::full::open_as`)
/// with this person's sign-in, renewed by `token_blocking` first when it needs it. `Ok(None)`:
/// the mode is off or not signed in (use the official path).
pub fn open_blocking(
    library: &pm_library::LibraryLock,
    ep: &crate::tidal::TidalEndpoints,
    id: &str,
    who: &Who,
) -> Result<Option<FullStream>, FullError> {
    let device = library.lock().ok().and_then(|l| client_of(&l));
    let endpoints = pm_sources::tidal::full::Endpoints {
        auth_base: &ep.auth_base,
        v1_base: &ep.v1_base,
        browser: ep.browser_client.as_ref(),
        device,
        no_device: NO_DEVICE_CLIENT,
    };
    let client = crate::sources::http_client();
    let Some((sess, quality)) = token_blocking(library, client, &endpoints, who)? else {
        return Ok(None);
    };
    pm_sources::tidal::full::open_as(client, &endpoints, &sess, &quality, id).map(Some)
}

/// Record how the mode did: a failure of the mode itself switches it off (the page says why;
/// Retry switches it on again).
pub fn record(library: &pm_library::LibraryLock, who: &Who, r: &Result<Option<FullStream>, FullError>) {
    pm_sources::tidal::full::record(&PersonStore::new(library, who), r);
}
