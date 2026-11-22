//! Qobuz in phone mode: email and password once (only the session is kept), the web player's app
//! values found and kept the way the node does it, and the node's Qobuz routes in its shapes.

use super::{err, source_err, Phone};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use pm_sources::qobuz::{self, App, Health, ModeState, Session};
use pm_sources::{SessionStore, SourceError};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub(crate) fn state(p: &Phone) -> ModeState {
    p.store
        .get(qobuz::STATE_KEY)
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

pub(crate) fn update(p: &Phone, f: impl FnOnce(&mut ModeState)) {
    update_in(p.store.as_ref(), f)
}

/// [`update`], saved before anything sees it: an error, and nothing changed, when it can't be
/// saved (what the page asked for).
fn change(p: &Phone, f: impl FnOnce(&mut ModeState)) -> Result<(), SourceError> {
    p.store
        .change_if(|d| {
            update_in(d, f);
            Ok(true)
        })
        .map(|_| ())
        .map_err(super::store::not_saved)
}

/// [`update`] on `store` (the phone's, or a draft of it).
pub(crate) fn update_in(store: &dyn pm_sources::SessionStore, f: impl FnOnce(&mut ModeState)) {
    let mut f = Some(f);
    store.update(qobuz::STATE_KEY, &mut |now| {
        let mut st: ModeState = now
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default();
        (f.take()?)(&mut st);
        Some(serde_json::to_string(&st).expect("state serializes"))
    });
}

/// On, signed in and with an app: Qobuz searches and plays.
pub(crate) fn streams(p: &Phone) -> bool {
    let st = state(p);
    st.enabled && st.session.is_some() && qobuz::app_of(p.store.as_ref()).is_some()
}

/// The app and the person's session, when Qobuz can answer for them.
pub(crate) fn ready(p: &Phone) -> Result<(App, Session), SourceError> {
    let st = state(p);
    let app = qobuz::app_of(p.store.as_ref()).ok_or_else(|| {
        SourceError::new(
            StatusCode::PRECONDITION_FAILED,
            "Sign in to Qobuz again to refresh automatic setup",
        )
    })?;
    if !st.enabled {
        return Err(SourceError::new(
            StatusCode::PRECONDITION_FAILED,
            "Qobuz is off: switch it on in Settings › Streaming services",
        ));
    }
    let sess = st.session.ok_or_else(|| {
        SourceError::new(
            StatusCode::PRECONDITION_FAILED,
            "Sign in to Qobuz first (Settings › Streaming services)",
        )
    })?;
    Ok((app, sess))
}

/// A failure of the mode itself: switch it off and say why (as the node does).
pub(crate) fn mark_down(p: &Phone, why: &str) {
    tracing::warn!("phone mode: Qobuz switched off: {why}");
    update(p, |st| {
        st.enabled = false;
        st.health = Some(Health {
            ok: false,
            detail: why.into(),
            at: pm_sources::now_unix(),
        });
    });
}

/// What a failed catalogue call answers.
pub(crate) fn failed(p: &Phone, e: qobuz::CallError) -> SourceError {
    match e {
        qobuz::CallError::Ended(why) => {
            mark_down(p, &why);
            SourceError::new(StatusCode::BAD_GATEWAY, why)
        }
        qobuz::CallError::Other(e) => e,
    }
}

fn view(p: &Phone) -> Value {
    serde_json::to_value(qobuz::status(&state(p))).unwrap_or(Value::Null)
}

/// The Sources page's row.
pub(crate) fn source_row(p: &Phone) -> Value {
    let st = state(p);
    let status = if let Some(h) = st.health.as_ref().filter(|h| !h.ok && !st.enabled) {
        format!("error: {}", h.detail)
    } else if st.session.is_none() {
        "needs sign-in".to_string()
    } else if !st.enabled {
        "off: switch Qobuz on in Settings › Streaming services".to_string()
    } else {
        "signed in".to_string()
    };
    json!({
        "id": qobuz::SOURCE_ID,
        "name": "Qobuz",
        "description": "Unofficial: search, albums and full tracks in FLAC up to 24-bit/192 kHz with your Qobuz plan",
        "status": status,
        "enabled": true,
        "features": ["search", "play", "artists"],
        "stage": "unofficial",
        "downloadable": false,
        "sign_in": line(&st),
        "healthy": streams(p),
    })
}

fn in_use(st: &ModeState) -> Option<Value> {
    let s = st.session.as_ref()?;
    Some(json!({
        "way": "own",
        "label": "Your Qobuz account",
        "user": s.name.clone().or_else(|| s.user_id.clone()),
        "gives": if s.hires { "hi-res" } else { "CD quality" },
    }))
}

fn line(st: &ModeState) -> String {
    match in_use(st) {
        Some(u) => format!(
            "{} · {}",
            u["label"].as_str().unwrap_or_default(),
            u["gives"].as_str().unwrap_or_default()
        ),
        None => "Not signed in".into(),
    }
}

/// The one way to sign in (the node's shape).
pub(crate) fn sign_in(p: &Phone) -> Value {
    let st = state(p);
    json!({
        "source": "qobuz",
        "ways": [{
            "id": "own",
            "label": "Your Qobuz account",
            "note": "Your Qobuz email and password; automatic app setup, and only the session is kept.",
            "action": "setup",
            "recommended": true,
            "available": true,
        }],
        "in_use": in_use(&st),
        "line": line(&st),
        "details": { "unofficial": view(p) },
    })
}

pub(crate) fn sign_out(p: &Phone) -> Result<(), SourceError> {
    change(p, |st| st.session = None)
}

/// `GET /v1/sources/qobuz/unofficial`.
pub(crate) async fn status(State(p): State<Arc<Phone>>) -> Json<Value> {
    Json(view(&p))
}

#[derive(Deserialize)]
pub(crate) struct Update {
    enabled: Option<bool>,
    quality: Option<String>,
}

/// `PUT /v1/sources/qobuz/unofficial {enabled?, quality?}`.
pub(crate) async fn put(State(p): State<Arc<Phone>>, Json(u): Json<Update>) -> Response {
    if let Some(q) = &u.quality {
        if q != "max" && q != "cd" && q != "mp3" {
            return err(StatusCode::BAD_REQUEST, "quality is max, cd or mp3");
        }
    }
    let changed = change(&p, |st| {
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
    if let Err(e) = changed {
        return source_err(e);
    }
    if u.quality.is_some() {
        if let Err(e) = super::data::prefs_changed(&p, "qobuz") {
            return super::data::not_shared(e);
        }
    }
    Json(view(&p)).into_response()
}

/// `POST /v1/sources/qobuz/unofficial/login {email, password}`: finds the app values when
/// needed, signs in once and keeps only the session. Neither the password nor its MD5 is kept.
pub(crate) async fn login(State(p): State<Arc<Phone>>, Json(l): Json<qobuz::Login>) -> Response {
    if l.email.trim().is_empty() || l.password.is_empty() {
        return err(StatusCode::BAD_REQUEST, "Enter your Qobuz email and password");
    }
    let hash = qobuz::password_hash(&l.password);
    let signed = tokio::time::timeout(
        qobuz::DEADLINE,
        qobuz::sign_in(p.store.as_ref(), l.email.trim(), &hash),
    )
    .await;
    let sess = match signed {
        Err(_) => return source_err(qobuz::auto::Fail::Unreachable.into()),
        Ok(Err(e)) => return source_err(e),
        Ok(Ok(s)) => s,
    };
    // The sign-in and its new id in one step
    let kept = super::vault::sign_in_here(&p, pm_sources::vault_form::QOBUZ.service, |store| {
        update_in(store, |st| {
            st.session = Some(sess);
            st.enabled = true;
            st.health = None;
        });
        true
    });
    if let Err(e) = kept {
        return source_err(super::store::not_saved(e));
    }
    tracing::info!("phone mode: signed in to Qobuz");
    Json(view(&p)).into_response()
}

/// `DELETE /v1/sources/qobuz/unofficial/login`: forget the session.
pub(crate) async fn logout(State(p): State<Arc<Phone>>) -> Response {
    match sign_out(&p) {
        Ok(()) => Json(view(&p)).into_response(),
        Err(e) => source_err(e),
    }
}
