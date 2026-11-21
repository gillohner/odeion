//! TIDAL in phone mode: the full-tracks sign-ins (Sign in with TIDAL, and the code sign-in with
//! this build's device client), the person's state in the phone's store, and the routes the
//! node's TIDAL page uses, in the node's shapes. The steps themselves are `pm_sources`'.

use super::{err, source_err, Phone};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use pm_sources::tidal::full::{self, Approval, Endpoints, Health, ModeState};
use pm_sources::{now_unix as now, SessionStore, SourceError};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

/// What a missing device client says on a phone.
const NO_DEVICE: &str =
    "This build has no TIDAL device client to renew the code sign-in: Sign in with TIDAL instead";

pub(crate) fn state(p: &Phone) -> ModeState {
    full::load(p.store.as_ref())
}

/// Signed in and switched on: TIDAL searches and plays.
pub(crate) fn streams(p: &Phone) -> bool {
    let st = state(p);
    st.enabled && st.session.is_some()
}

pub(crate) fn endpoints(p: &Phone) -> Endpoints<'_> {
    Endpoints {
        auth_base: &p.tidal.auth_base,
        v1_base: &p.tidal.v1_base,
        browser: p.tidal.browser.as_ref(),
        device: p.tidal.device.clone(),
        no_device: NO_DEVICE,
    }
}

/// The person's session when TIDAL can answer for them (renewed first when it is about to run
/// out), else why not.
pub(crate) async fn session(p: &Arc<Phone>) -> Result<full::Session, SourceError> {
    // Renewed under the vault's lock where the vault has this sign-in (the person's other devices
    // never renew it at the same time); otherwise, or offline, here (the next sync writes it)
    if super::vault::tidal_due(p) {
        if let Some(acct) = p.account().await {
            if let Err(e) = super::vault::renew_tidal(p, &acct).await {
                tracing::warn!("phone mode: TIDAL renewal in the vault: {e}");
            }
        }
    }
    let p2 = p.clone();
    let r = tokio::task::spawn_blocking(move || {
        full::token(
            p2.store.as_ref(),
            super::stream::blocking_client(),
            &endpoints(&p2),
        )
    })
    .await
    .map_err(|_| {
        SourceError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "TIDAL: the sign-in check stopped",
        )
    })?;
    match r {
        Ok(Some((sess, _))) => Ok(sess),
        Ok(None) => Err(SourceError::new(
            StatusCode::PRECONDITION_FAILED,
            "Sign in to TIDAL first (Settings › Streaming services)",
        )),
        Err(e) => {
            let mode = matches!(e, full::FullError::Mode(_));
            full::record(p.store.as_ref(), &Err(e));
            let why = state(p)
                .health
                .map(|h| h.detail)
                .unwrap_or_else(|| "TIDAL didn't answer".into());
            Err(SourceError::new(
                if mode {
                    StatusCode::PRECONDITION_FAILED
                } else {
                    StatusCode::BAD_GATEWAY
                },
                why,
            ))
        }
    }
}

fn view(p: &Phone) -> Value {
    let st = state(p);
    serde_json::to_value(full::status(
        &st,
        p.tidal.device.as_ref().map(|c| c.from.clone()),
        p.tidal.browser.is_some(),
    ))
    .unwrap_or(Value::Null)
}

/// The Sources page's row.
pub(crate) fn source_row(p: &Phone) -> Value {
    let on = streams(p);
    let st = state(p);
    let status = match (&st.health, on) {
        (Some(h), false) if !h.ok && st.session.is_some() => format!("error: {}", h.detail),
        (_, true) => "ready".to_string(),
        _ => "needs sign-in".to_string(),
    };
    json!({
        "id": pm_sources::tidal::SOURCE_ID,
        "name": "TIDAL",
        "description": "Your TIDAL plan: search, albums and full tracks on this phone",
        "status": status,
        "enabled": true,
        "features": ["search", "play", "artists"],
        "stage": "unofficial",
        "downloadable": false,
        "sign_in": line(p),
        "healthy": on,
    })
}

fn in_use(st: &ModeState) -> Option<Value> {
    let d = st.session.as_ref().filter(|_| st.enabled)?;
    let (way, label) = match d.client {
        full::ClientKind::Pkce => ("browser", "Sign in with TIDAL"),
        full::ClientKind::Device => ("device", "Sign in with a code"),
    };
    Some(json!({
        "way": way,
        "label": label,
        "user": d.user_id,
        "expires_at": d.expires_at,
        "gives": if d.client == full::ClientKind::Pkce { "full tracks, lossless" } else { "full tracks" },
    }))
}

fn line(p: &Phone) -> String {
    match in_use(&state(p)) {
        Some(u) => format!("{} · full tracks", u["label"].as_str().unwrap_or_default()),
        None => "Not signed in".into(),
    }
}

/// The ways to sign in on a phone (the node's shape): Sign in with TIDAL, and the code sign-in
/// where this build has a device client.
pub(crate) fn sign_in(p: &Phone) -> Value {
    let browser = p.tidal.browser.is_some();
    let device = p.tidal.device.is_some();
    let st = state(p);
    json!({
        "source": "tidal",
        "ways": [
            {
                "id": "browser",
                "label": "Sign in with TIDAL",
                "note": "Full tracks, lossless and hi-res: sign in at TIDAL in your browser, then paste the address of the page you land on. Unofficial: TIDAL has not approved this for other apps and can stop it at any time.",
                "action": "browser",
                "recommended": browser,
                "available": browser,
                "reason": (!browser).then_some(full::NO_BROWSER_CLIENT),
            },
            {
                "id": "device",
                "label": "Sign in with a code",
                "note": "Fallback, which TIDAL may send AAC only (lossy): a code you enter at link.tidal.com.",
                "action": "code",
                "recommended": false,
                "available": device,
                "reason": (!device).then_some("This build has no TIDAL device client."),
            },
        ],
        "in_use": in_use(&st),
        "line": line(p),
        "details": { "unofficial": view(p) },
    })
}

pub(crate) fn sign_out(p: &Phone) {
    full::update(p.store.as_ref(), |st| {
        st.session = None;
        st.login = None;
        st.browser = None;
    });
}

/// `GET /v1/sources/tidal/unofficial`.
pub(crate) async fn status(State(p): State<Arc<Phone>>) -> Json<Value> {
    Json(view(&p))
}

#[derive(Deserialize)]
pub(crate) struct Update {
    enabled: Option<bool>,
    quality: Option<String>,
    client_id: Option<String>,
}

/// `PUT /v1/sources/tidal/unofficial {enabled?, quality?}`.
pub(crate) async fn put(State(p): State<Arc<Phone>>, Json(u): Json<Update>) -> Response {
    if u.client_id.is_some() {
        return err(
            StatusCode::FORBIDDEN,
            "A device client of your own is set up on a node",
        );
    }
    if let Some(q) = &u.quality {
        if q != "max" && q != "high" {
            return err(StatusCode::BAD_REQUEST, "quality is max or high");
        }
    }
    full::update(p.store.as_ref(), |st| {
        if let Some(e) = u.enabled {
            st.enabled = e;
            if e {
                st.health = None;
            }
        }
        if let Some(q) = u.quality.clone() {
            st.quality = q;
        }
    });
    if u.quality.is_some() {
        super::data::prefs_changed(&p);
    }
    Json(view(&p)).into_response()
}

/// `POST /v1/sources/tidal/unofficial/login`: the device code to enter at link.tidal.com; the
/// phone waits for the approval while the app is open.
pub(crate) async fn login(State(p): State<Arc<Phone>>) -> Response {
    let Some(c) = p.tidal.device.clone() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "This build has no TIDAL device client.",
        );
    };
    let pending = match full::device_authorization(&p.http, &p.tidal.auth_base, &c).await {
        Ok(x) => x,
        Err(e) => return source_err(e),
    };
    full::update(p.store.as_ref(), |st| st.login = Some(pending.clone()));
    let p2 = p.clone();
    tokio::spawn(async move { wait_for_approval(p2, c, pending).await });
    Json(view(&p)).into_response()
}

async fn wait_for_approval(p: Arc<Phone>, c: full::Client, pending: full::Pending) {
    while now() < pending.expires_at {
        tokio::time::sleep(std::time::Duration::from_secs(pending.interval)).await;
        if state(&p).login.as_ref().map(|x| &x.device_code) != Some(&pending.device_code) {
            return;
        }
        let sess = match full::poll_approval(&p.http, &p.tidal.auth_base, &c, &pending).await {
            Approval::Waiting => continue,
            Approval::Expired => break,
            Approval::Approved(None) => return,
            Approval::Approved(Some(sess)) => sess,
        };
        let sess = full::fill_session(&p.http, &p.tidal.v1_base, sess).await;
        let kept = full::update_if(p.store.as_ref(), |st| {
            if st.login.as_ref().map(|x| &x.device_code) != Some(&pending.device_code) {
                return false;
            }
            st.session = Some(sess);
            st.login = None;
            st.enabled = true;
            st.health = Some(signed_in());
            true
        });
        if kept {
            tracing::info!("phone mode: signed in to TIDAL (code sign-in)");
        }
        return;
    }
    full::update_if(p.store.as_ref(), |st| {
        let mine = st.login.as_ref().map(|x| &x.device_code) == Some(&pending.device_code);
        if mine {
            st.login = None;
        }
        mine
    });
}

fn signed_in() -> Health {
    Health {
        ok: true,
        detail: "signed in".into(),
        at: now(),
        lossy: false,
    }
}

/// `POST /v1/sources/tidal/unofficial/browser`: TIDAL's sign-in address to open (a new attempt
/// replaces one on its way).
pub(crate) async fn start_browser(State(p): State<Arc<Phone>>) -> Response {
    let Some(c) = &p.tidal.browser else {
        return err(StatusCode::SERVICE_UNAVAILABLE, full::NO_BROWSER_CLIENT);
    };
    let pending = full::new_browser_pending();
    let url = full::authorize_url(p.tidal.login_base.trim_end_matches('/'), &c.id, &pending);
    let expires_at = pending.expires_at;
    full::update(p.store.as_ref(), |st| st.browser = Some(pending));
    Json(json!({ "url": url, "expires_at": expires_at })).into_response()
}

#[derive(Deserialize)]
pub(crate) struct Finish {
    text: String,
}

/// `POST /v1/sources/tidal/unofficial/browser/finish {text}`: the pasted address (or code).
pub(crate) async fn finish_browser(State(p): State<Arc<Phone>>, Json(b): Json<Finish>) -> Response {
    let Some(c) = p.tidal.browser.clone() else {
        return err(StatusCode::SERVICE_UNAVAILABLE, full::NO_BROWSER_CLIENT);
    };
    let Some(pending) = state(&p).browser else {
        return err(
            StatusCode::CONFLICT,
            "No sign-in with TIDAL is on its way here. Press Sign in with TIDAL, sign in, then paste the address.",
        );
    };
    if pending.expires_at <= now() {
        full::update_if(p.store.as_ref(), |st| {
            let mine = st.browser.as_ref() == Some(&pending);
            if mine {
                st.browser = None;
            }
            mine
        });
        return err(
            StatusCode::GONE,
            "That sign-in waited too long and expired. Press Sign in with TIDAL to start again.",
        );
    }
    let code = match full::code_from_paste(&b.text) {
        Ok(c) => c,
        Err(e) => return err(StatusCode::BAD_REQUEST, &e),
    };
    let sess = match full::exchange_browser_code(&p.http, &p.tidal.auth_base, &c, &pending, &code).await {
        Ok(s) => s,
        Err(e) => return source_err(e),
    };
    let sess = full::fill_session(&p.http, &p.tidal.v1_base, sess).await;
    let kept = full::update_if(p.store.as_ref(), |st| {
        if st.browser.as_ref() != Some(&pending) {
            return false;
        }
        st.session = Some(sess);
        st.browser = None;
        st.login = None;
        st.enabled = true;
        st.health = Some(signed_in());
        true
    });
    if !kept {
        return err(
            StatusCode::CONFLICT,
            "The sign-in changed meanwhile (signed out or started again here). Press Sign in with TIDAL to start again.",
        );
    }
    tracing::info!("phone mode: signed in to TIDAL (Sign in with TIDAL)");
    Json(view(&p)).into_response()
}

/// `DELETE /v1/sources/tidal/unofficial/login`: forget the session (the switch stays as set).
pub(crate) async fn logout(State(p): State<Arc<Phone>>) -> Json<Value> {
    sign_out(&p);
    Json(view(&p))
}

/// Whether `store` holds a TIDAL sign-in (for the store's own tests and the vault).
pub fn signed_in_store(store: &dyn SessionStore) -> bool {
    full::load(store).session.is_some()
}
