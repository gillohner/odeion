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

/// The longest one TIDAL renewal request may take, its answer read whole: well within the time
/// the vault gives a renewal under its lock (`pm_pubky::vault::CALL_FOR`), so the vault never
/// stops waiting for a renewal that can still answer (and then throws its new tokens away while
/// the next song asks again with the same refresh token). The same here, without the vault.
pub(crate) const RENEW_FOR: std::time::Duration = std::time::Duration::from_secs(40);

/// TIDAL's renewal of `s`, within the phone's limit (a blocking call: off the async runtime).
pub(crate) fn renew_blocking(p: &Phone, s: &full::Session) -> Result<full::Session, full::FullError> {
    let limit =
        std::time::Duration::from_millis(p.tidal_renew_for_ms.load(std::sync::atomic::Ordering::Relaxed));
    full::renew_within(super::stream::blocking_client(), &endpoints(p), s, Some(limit))
}

/// How long a song waits, past the first 8 s, for a renewal of an expired session.
const RENEWAL_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// A renewal's new session the phone couldn't save (its sign-ins file can't be written now): the
/// phone plays it and saves it at the next try, and never renews the session it replaced again
/// (TIDAL may have spent that one's refresh token). Gone once saved, or once the phone holds
/// another sign-in (a sign-out, a new sign-in, one taken from the vault). Lost if the app stops
/// first: a disk that refuses the save keeps the session before it.
#[derive(Clone)]
pub(crate) struct Unsaved {
    /// The access token of the session it renewed, as the phone keeps it.
    from: String,
    session: full::Session,
    /// The vault holds it too (renewed under its lock and written there). Otherwise only the phone
    /// does: it is renewed here, never the vault's copy of the session it replaced.
    in_vault: bool,
}

/// Keeps `session`, renewed from the phone's session with access token `from`, until it can be
/// saved ([`Unsaved`]).
pub(crate) fn hold_unsaved(p: &Phone, from: String, session: full::Session, in_vault: bool) {
    tracing::warn!("phone mode: a renewed TIDAL sign-in couldn't be saved; it is used and saved later");
    *p.tidal_unsaved.lock().unwrap_or_else(|e| e.into_inner()) = Some(Unsaved {
        from,
        session,
        in_vault,
    });
}

/// Whether the phone uses a TIDAL renewal held unsaved that the vault doesn't have (over the
/// session the phone keeps now).
pub(crate) fn held_outside_vault(p: &Phone) -> bool {
    let from = full::load(p.store.as_ref()).session.map(|s| s.access_token);
    p.tidal_unsaved
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|u| !u.in_vault && from.as_deref() == Some(u.from.as_str()))
}

/// Saves a renewal held since its save failed ([`Unsaved`]), over the session it renewed only.
/// Runs with [`Phone::tidal_renewal`] held (the only one that sets it).
fn save_unsaved(p: &Phone) {
    let Some(u) = p.tidal_unsaved.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
        return;
    };
    let saved = p.store.change_if(|d| {
        Ok(full::update_if(d, |st| {
            let over = st.session.as_ref().is_some_and(|s| s.access_token == u.from);
            if over {
                st.session = Some(u.session.clone());
            }
            over
        }))
    });
    match saved {
        // Saved, or the phone holds another sign-in now (that one wins)
        Ok(_) => *p.tidal_unsaved.lock().unwrap_or_else(|e| e.into_inner()) = None,
        Err(e) => tracing::warn!("phone mode: a renewed TIDAL sign-in still can't be saved: {e}"),
    }
}

/// A renewal that couldn't be saved before, saved now if it can be (not while a renewal runs).
pub(crate) fn try_save_unsaved(p: &Phone) {
    if p.tidal_unsaved
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some()
    {
        if let Ok(_one) = p.tidal_renewal.try_lock() {
            save_unsaved(p);
        }
    }
}

/// The TIDAL state as the phone keeps it, with a renewal it couldn't save yet in place of the
/// session that renewal replaced.
fn with_unsaved(p: &Phone, mut st: ModeState) -> ModeState {
    if let Some(u) = p.tidal_unsaved.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        if st.session.as_ref().is_some_and(|s| s.access_token == u.from) {
            st.session = Some(u.session.clone());
        }
    }
    st
}

/// The phone's TIDAL session while it still works (not renewed).
fn current(p: &Phone) -> Option<full::Session> {
    let st = with_unsaved(p, full::load(p.store.as_ref()));
    st.session
        .filter(|s| st.enabled && s.expires_at > pm_sources::now_unix() + 5)
}

fn being_renewed() -> SourceError {
    SourceError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "TIDAL: the sign-in is being renewed; try again in a moment",
    )
}

fn sign_in_first() -> SourceError {
    SourceError::new(
        StatusCode::PRECONDITION_FAILED,
        "Sign in to TIDAL first (Settings › Streaming services)",
    )
}

/// The phone's session as it is (not renewed), when TIDAL is on and signed in.
fn held(p: &Phone) -> Result<full::Session, SourceError> {
    let st = with_unsaved(p, state(p));
    st.session.filter(|_| st.enabled).ok_or_else(sign_in_first)
}

/// Due for renewal: it runs out within a minute (as `full::token` and the vault judge it).
fn due(s: &full::Session) -> bool {
    s.expires_at - now() <= 60
}

/// The person's session when TIDAL can answer for them (renewed first when it is about to run
/// out), else why not.
pub(crate) async fn session(p: &Arc<Phone>) -> Result<full::Session, SourceError> {
    try_save_unsaved(p);
    let s = held(p)?;
    if !due(&s) {
        return Ok(s);
    }
    // The renewal runs to its end on its own task (its new tokens are never dropped); a slow
    // homeserver or TIDAL only stops the song waiting for it (the phone then uses what it has)
    let p2 = p.clone();
    let mut renewal = tokio::spawn(async move { renew(&p2).await });
    let ended = match tokio::time::timeout(std::time::Duration::from_secs(8), &mut renewal).await {
        Ok(r) => r,
        Err(_) => match current(p) {
            Some(s) => {
                tracing::warn!("phone mode: the TIDAL renewal takes long; the current session plays");
                return Ok(s);
            }
            // Expired: the song waits for the renewal, within a limit
            None => match tokio::time::timeout(RENEWAL_WAIT, renewal).await {
                Ok(r) => r,
                Err(_) => return Err(being_renewed()),
            },
        },
    };
    ended.unwrap_or_else(|_| {
        Err(SourceError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "TIDAL: the sign-in check stopped",
        ))
    })
}

/// Renews the phone's TIDAL session, one renewal at a time on the phone ([`Phone::tidal_renewal`]):
/// under the vault's lock where the vault has this sign-in (the person's other devices never renew
/// it at the same time), otherwise, or offline, here (the next sync writes it). A song that found
/// the session due while another renewal ran waits for that one and uses what it got.
///
/// After a vault failure that leaves open whether the sign-in is being renewed elsewhere
/// ([`super::vault::uncertain`]: the homeserver didn't answer, or TIDAL didn't under the vault's
/// lock), the phone never renews here instead: another device may be renewing the same sign-in,
/// and the two would spend each other's tokens. It plays the current session while it works, and
/// the next song tries the vault again. After any other vault failure it renews here. With no
/// Pubky account restored (the homeserver didn't answer when the app started), a sign-in the last
/// sync found in step with the vault is the same: another device may renew it under the vault's
/// lock, so the phone plays its current session until the vault can be asked.
///
/// Renewed here, the new session is saved before it is used; one that can't be saved is held
/// ([`Unsaved`]) and never renewed again meanwhile.
///
/// The whole renewal, here too, runs in the phone's turn at the vault
/// ([`super::vault::vault_turn`]): a vault pass never writes the session being renewed to the
/// vault. Behind a pass that doesn't end in time, the current session plays.
async fn renew(p: &Arc<Phone>) -> Result<full::Session, SourceError> {
    let _one = match p.tidal_renewal.try_lock() {
        Ok(g) => g,
        // Another renewal runs: this one waits for it and uses what it got, never one more after
        // it (songs and searches meanwhile never queue renewals up)
        Err(_) => {
            let _after = p.tidal_renewal.lock().await;
            let s = held(p)?;
            return if due(&s) {
                current(p).ok_or_else(being_renewed)
            } else {
                Ok(s)
            };
        }
    };
    // Renewed just before this one took its turn (or one that couldn't be saved can be now)
    save_unsaved(p);
    let s = held(p)?;
    if !due(&s) {
        return Ok(s);
    }
    // A renewal held unsaved that the vault doesn't have: renewed here (the vault's copy is the
    // session it replaced)
    let outside = held_outside_vault(p);
    let acct = if outside { None } else { p.account().await };
    // The vault's turn, for the whole renewal (through the vault, or here and its save): a vault
    // pass never reads the session being renewed here and writes it to the vault (where another
    // device would renew it again), and none comes between the vault's answer and a renewal here
    // (A held renewal the vault doesn't have is never synced while it is held: no turn needed)
    let _turn = match outside {
        true => None,
        false => match super::vault::vault_turn(p).await {
            Some(t) => Some(t),
            None => {
                tracing::warn!("phone mode: a vault sync runs; TIDAL is renewed after it");
                return current(p).ok_or_else(being_renewed);
            }
        },
    };
    // No account: the vault can't be asked, and the last sync found TIDAL there
    if !outside && acct.is_none() && super::vault::tidal_in_step(p) {
        tracing::warn!(
            "phone mode: TIDAL is in the vault and the homeserver isn't reached yet; not renewed here"
        );
        return current(p).ok_or_else(not_now);
    }
    if let Some(acct) = acct {
        match super::vault::renew_tidal_in_turn(p, &acct).await {
            Ok(super::vault::Renewal::NotInVault) => {}
            // (Or a sign-in or sign-out made here meanwhile: what the phone holds now)
            Ok(super::vault::Renewal::Renewed) => {
                return current(p).ok_or_else(|| held(p).err().unwrap_or_else(being_renewed))
            }
            // Another vault step renews it, or holds the vault: the same, never beside it
            Ok(super::vault::Renewal::Busy) | Err(pm_pubky::vault::VaultStoreError::Busy(_)) => {
                return current(p).ok_or_else(being_renewed);
            }
            // TIDAL said no: switched off with why, as a refusal here
            Ok(super::vault::Renewal::Refused(why)) => {
                full::record(p.store.as_ref(), &Err(full::FullError::Mode(why)));
                return Err(refused(p));
            }
            // TIDAL was asked, and what came of it is unknown here: never asked again beside it
            Ok(super::vault::Renewal::Unclear(why)) => {
                tracing::warn!("phone mode: TIDAL renewal in the vault: {why}");
                return current(p).ok_or_else(not_now);
            }
            Err(e) if super::vault::uncertain(&e) => {
                tracing::warn!("phone mode: TIDAL renewal in the vault: {e}");
                return current(p).ok_or_else(not_now);
            }
            Err(e) => tracing::warn!("phone mode: TIDAL renewal in the vault: {e}; renewed here"),
        }
    }
    // Read again after the vault's turn (a sign-in or sign-out may have come meanwhile): the
    // session to renew, and the one it is saved over (as the phone keeps it; a renewal not saved
    // yet replaces it, not this one), read together
    let st = full::load(p.store.as_ref());
    let Some(from) = st.session.as_ref().map(|k| k.access_token.clone()) else {
        return Err(sign_in_first());
    };
    let st = with_unsaved(p, st);
    let s = st.session.filter(|_| st.enabled).ok_or_else(sign_in_first)?;
    if !due(&s) {
        return Ok(s);
    }
    let p2 = p.clone();
    let r = tokio::task::spawn_blocking(move || renew_blocking(&p2, &s))
        .await
        .map_err(|_| {
            SourceError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "TIDAL: the sign-in check stopped",
            )
        })?;
    match r {
        Ok(fresh) => {
            // Only over the session it renewed (a sign-in or sign-out meanwhile wins), saved
            // before it is used; one that can't be saved is held, never renewed again meanwhile
            let saved = p.store.change_if(|d| {
                Ok(full::update_if(d, |st| {
                    let over = st.session.as_ref().is_some_and(|k| k.access_token == from);
                    if over {
                        st.session = Some(fresh.clone());
                    }
                    over
                }))
            });
            match saved {
                Ok(true) => Ok(fresh),
                Ok(false) => {
                    tracing::info!(
                        "phone mode: a renewed TIDAL session was dropped: the sign-in changed meanwhile"
                    );
                    held(p)
                }
                Err(e) => {
                    tracing::warn!("phone mode: {e}");
                    hold_unsaved(p, from, fresh.clone(), false);
                    Ok(fresh)
                }
            }
        }
        Err(e) => {
            let mode = matches!(e, full::FullError::Mode(_));
            full::record(p.store.as_ref(), &Err(e));
            if mode {
                return Err(refused(p));
            }
            Err(SourceError::new(StatusCode::BAD_GATEWAY, why_not(p)))
        }
    }
}

fn not_now() -> SourceError {
    SourceError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "TIDAL: the sign-in couldn't be renewed through your homeserver just now; try again in a moment",
    )
}

/// What the health note says went wrong (TIDAL's words), else that it didn't answer.
fn why_not(p: &Phone) -> String {
    state(p)
        .health
        .map(|h| h.detail)
        .unwrap_or_else(|| "TIDAL didn't answer".into())
}

/// TIDAL refused the renewal: sign in again.
fn refused(p: &Phone) -> SourceError {
    SourceError::new(StatusCode::PRECONDITION_FAILED, why_not(p))
}

/// Changes the TIDAL state, saved before anything sees it: an error, and nothing changed, when it
/// can't be saved.
fn change(p: &Phone, f: impl FnOnce(&mut ModeState) -> bool) -> Result<bool, SourceError> {
    p.store
        .change_if(|d| Ok(full::update_if(d, f)))
        .map_err(super::store::not_saved)
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

pub(crate) fn sign_out(p: &Phone) -> Result<(), SourceError> {
    change(p, |st| {
        st.session = None;
        st.login = None;
        st.browser = None;
        true
    })
    .map(|_| ())
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
        true
    });
    if let Err(e) = changed {
        return source_err(e);
    }
    if u.quality.is_some() {
        if let Err(e) = super::data::prefs_changed(&p, "tidal") {
            return super::data::not_shared(e);
        }
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
    if let Err(e) = change(&p, |st| {
        st.login = Some(pending.clone());
        true
    }) {
        return source_err(e);
    }
    let p2 = p.clone();
    tokio::spawn(async move { wait_for_approval(p2, c, pending).await });
    Json(view(&p)).into_response()
}

async fn wait_for_approval(p: Arc<Phone>, c: full::Client, pending: full::Pending) {
    while now() < pending.expires_at {
        tokio::time::sleep(std::time::Duration::from_secs(pending.interval)).await;
        // Phone mode left: no more waiting (the code is forgotten below)
        if p.closed() {
            break;
        }
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
        // The sign-in and its new id in one step
        let kept = super::vault::sign_in_here(&p, pm_sources::vault_form::TIDAL_FULL.service, |store| {
            full::update_if(store, |st| {
                if st.login.as_ref().map(|x| &x.device_code) != Some(&pending.device_code) {
                    return false;
                }
                st.session = Some(sess);
                st.login = None;
                st.enabled = true;
                st.health = Some(signed_in());
                true
            })
        });
        match kept {
            Ok(true) => tracing::info!("phone mode: signed in to TIDAL (code sign-in)"),
            Ok(false) => {}
            Err(e) => tracing::warn!("phone mode: the TIDAL sign-in couldn't be saved: {e}"),
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
    if let Err(e) = change(&p, |st| {
        st.browser = Some(pending);
        true
    }) {
        return source_err(e);
    }
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
    // The sign-in and its new id in one step
    let kept = super::vault::sign_in_here(&p, pm_sources::vault_form::TIDAL_FULL.service, |store| {
        full::update_if(store, |st| {
            if st.browser.as_ref() != Some(&pending) {
                return false;
            }
            st.session = Some(sess);
            st.browser = None;
            st.login = None;
            st.enabled = true;
            st.health = Some(signed_in());
            true
        })
    });
    match kept {
        Ok(true) => {}
        Ok(false) => {
            return err(
                StatusCode::CONFLICT,
                "The sign-in changed meanwhile (signed out or started again here). Press Sign in with TIDAL to start again.",
            )
        }
        Err(e) => return source_err(super::store::not_saved(e)),
    }
    tracing::info!("phone mode: signed in to TIDAL (Sign in with TIDAL)");
    Json(view(&p)).into_response()
}

/// `DELETE /v1/sources/tidal/unofficial/login`: forget the session (the switch stays as set).
pub(crate) async fn logout(State(p): State<Arc<Phone>>) -> Response {
    match sign_out(&p) {
        Ok(()) => Json(view(&p)).into_response(),
        Err(e) => source_err(e),
    }
}

/// Whether `store` holds a TIDAL sign-in (for the store's own tests and the vault).
pub fn signed_in_store(store: &dyn SessionStore) -> bool {
    full::load(store).session.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// TIDAL's sign-in service, counting renewals; each answers after `slow`.
    async fn tidal(slow: std::time::Duration) -> (String, Arc<AtomicUsize>) {
        let renewed = Arc::new(AtomicUsize::new(0));
        let r2 = renewed.clone();
        let app = axum::Router::new().route(
            "/v1/oauth2/token",
            axum::routing::post(move || {
                let r = r2.clone();
                async move {
                    let n = r.fetch_add(1, Ordering::SeqCst) + 1;
                    tokio::time::sleep(slow).await;
                    Json(json!({"access_token": format!("fresh-{n}"), "expires_in": 3600}))
                }
            }),
        );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (base, renewed)
    }

    /// Due, and not in the vault (never synced): renewed here.
    fn due_here(p: &Phone) {
        super::super::vault::tidal_due_in_step(p);
        std::fs::remove_file(p.dir().join("vault").join("synced.json")).unwrap();
    }

    fn phone(dir: &std::path::Path, auth_base: String) -> Arc<Phone> {
        Phone::open(super::super::Config {
            dir: dir.to_path_buf(),
            cache: dir.join("streams"),
            version: "test".into(),
            pubky: None,
            tidal: super::super::TidalConf {
                v1_base: format!("{auth_base}/v1"),
                auth_base,
                device: Some(full::Client {
                    id: "device-client".into(),
                    secret: "s".into(),
                    from: "builtin".into(),
                }),
                ..Default::default()
            },
        })
        .unwrap()
    }

    /// The vault's TIDAL copy, made with the phone's device client, due in 30 s.
    fn due_copy(lineage: &str) -> pm_appdata::vault::SignIn {
        super::super::vault::node_copy(
            &pm_sources::vault_form::TIDAL_FULL,
            json!({"access_token": "due", "refresh_token": "rt", "expires_at": pm_sources::now_unix() + 30, "country": "CH"}),
            Some("device-client"),
            lineage,
        )
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn after_a_pass_cut_short_tidal_isnt_renewed_beside_the_vault() {
        let (base, renewed) = tidal(std::time::Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let p = phone(dir.path(), base);
        // A pass takes TIDAL from the vault and is cut off at the next step (Qobuz never answers)
        let vault = super::super::vault::StandIn::default();
        vault.copies.lock().unwrap().insert(
            pm_sources::vault_form::TIDAL_FULL.service.into(),
            due_copy("1000-0123456789abcdef"),
        );
        super::super::vault::qobuz_removal_stalls(&p, &vault);
        let cut = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            super::super::vault::services_here(&p, &vault),
        )
        .await;
        assert!(cut.is_err(), "cut short at Qobuz");
        assert_eq!(current(&p).map(|s| s.access_token).as_deref(), Some("due"));
        // The vault's homeserver doesn't answer now: the vault can't say whether another device
        // is renewing it, so the phone doesn't renew it on its own
        p.set_account(super::super::stalled::refusing().await.account())
            .await;
        let s = session(&p)
            .await
            .expect("the current session, still good for 30 s");
        assert_eq!(s.access_token, "due");
        assert_eq!(
            renewed.load(Ordering::SeqCst),
            0,
            "no renewal here beside the vault"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_take_the_record_missed_isnt_renewed_beside_the_vault() {
        let (base, renewed) = tidal(std::time::Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let p = phone(dir.path(), base);
        // Taken from the vault; the record couldn't be saved after it (the take is under way)
        super::super::vault::taken_unrecorded(&p, &due_copy("1000-0123456789abcdef"));
        assert!(!super::super::vault::record_here(&p)
            .services
            .contains_key("tidal-full"));
        p.set_account(super::super::stalled::refusing().await.account())
            .await;
        let s = session(&p)
            .await
            .expect("the current session, still good for 30 s");
        assert_eq!(s.access_token, "due");
        assert_eq!(
            renewed.load(Ordering::SeqCst),
            0,
            "no renewal here beside the vault"
        );
        // Nor without the homeserver at all (no account restored)
        let dir2 = tempfile::tempdir().unwrap();
        let (base2, renewed2) = tidal(std::time::Duration::ZERO).await;
        let p2 = phone(dir2.path(), base2);
        super::super::vault::taken_unrecorded(&p2, &due_copy("1000-0123456789abcdef"));
        assert_eq!(
            session(&p2).await.map(|s| s.access_token).ok().as_deref(),
            Some("due")
        );
        assert_eq!(renewed2.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_renewal_ends_well_before_the_vault_stops_waiting_for_it() {
        // (Connecting is within it too)
        assert!(RENEW_FOR + std::time::Duration::from_secs(15) <= pm_pubky::vault::CALL_FOR);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_tidal_that_never_answers_doesnt_hold_a_renewal_past_its_limit() {
        // TIDAL takes the request and never answers
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((c, _)) = l.accept().await {
                held.push(c);
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let p = phone(dir.path(), base);
        due_here(&p);
        // (The phone's limit, shortened; the client's own is far longer)
        p.tidal_renew_for_ms
            .store(500, std::sync::atomic::Ordering::Relaxed);
        let started = std::time::Instant::now();
        let r = tokio::time::timeout(std::time::Duration::from_secs(20), session(&p))
            .await
            .expect("the renewal ends by its own limit");
        assert_eq!(
            r.map(|_| ()).unwrap_err().status,
            StatusCode::BAD_GATEWAY,
            "TIDAL didn't answer"
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_yes_from_tidal_that_doesnt_arrive_whole_is_not_a_refusal() {
        // TIDAL answers 200, and its answer is cut off
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut c, _)) = l.accept().await {
                let mut b = [0u8; 4096];
                let _ = c.read(&mut b).await;
                let _ = c
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 200\r\n\r\n{\"access_tok")
                    .await;
                drop(c);
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let p = phone(dir.path(), base);
        due_here(&p);
        let r = session(&p).await.map(|_| ());
        assert_eq!(r.unwrap_err().status, StatusCode::BAD_GATEWAY);
        // Still on: not "sign in again"
        let st = full::load(p.store.as_ref());
        assert!(st.enabled && st.session.is_some(), "{:?}", st.health);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_vault_sync_never_writes_the_session_being_renewed_here() {
        // TIDAL answers after 2 s; the sign-in isn't in the vault: renewed here
        let (base, renewed) = tidal(std::time::Duration::from_secs(2)).await;
        let dir = tempfile::tempdir().unwrap();
        let p = phone(dir.path(), base);
        due_here(&p);
        let p2 = p.clone();
        let renewal = tokio::spawn(async move { renew(&p2).await });
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        // A vault pass starts meanwhile (its turn first, as `vault::pass` takes it), and writes
        // the phone's TIDAL to the vault
        let vault = super::super::vault::StandIn::default();
        let pass = async {
            let _turn = p.vault_lock.lock().await;
            super::super::vault::services_here(&p, &vault).await
        };
        let report = pass.await.unwrap();
        assert!(report.pushed.contains(&"tidal-full"), "{report:?}");
        assert_eq!(renewal.await.unwrap().unwrap().access_token, "fresh-1");
        assert_eq!(renewed.load(Ordering::SeqCst), 1);
        // What went to the vault is the renewed session, never the one TIDAL was renewing (its
        // refresh token spent: another device would renew it again)
        let copy = vault.copies.lock().unwrap()["tidal-full"].clone();
        let travels = pm_sources::vault_form::node_settings(&copy).unwrap();
        let sent =
            pm_sources::vault_form::session_of(pm_sources::vault_form::TIDAL_FULL.carry, &travels).unwrap();
        assert_eq!(sent["access_token"], "fresh-1");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn songs_at_once_wait_for_one_renewal() {
        let (base, renewed) = tidal(std::time::Duration::from_millis(300)).await;
        let dir = tempfile::tempdir().unwrap();
        let p = phone(dir.path(), base);
        // Due, not in the vault, and no Pubky account (offline): renewed here
        due_here(&p);
        let songs: Vec<_> = (0..3)
            .map(|_| {
                let p = p.clone();
                tokio::spawn(async move { session(&p).await })
            })
            .collect();
        for s in songs {
            assert_eq!(s.await.unwrap().unwrap().access_token, "fresh-1");
        }
        assert_eq!(renewed.load(Ordering::SeqCst), 1, "one renewal for all three");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn after_the_vault_failed_the_phone_doesnt_renew_beside_it() {
        let (base, renewed) = tidal(std::time::Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let p = phone(dir.path(), base);
        // In step with the vault, whose homeserver is down: the vault can't say whether another
        // device is renewing it
        super::super::vault::tidal_due_in_step(&p);
        let down = super::super::stalled::refusing().await;
        p.set_account(down.account()).await;
        let s = session(&p)
            .await
            .expect("the current session, still good for 30 s");
        assert_eq!(s.access_token, "due");
        assert_eq!(
            renewed.load(Ordering::SeqCst),
            0,
            "no renewal here beside the vault"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_lasting_vault_problem_doesnt_stop_tidal() {
        let (base, renewed) = tidal(std::time::Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let p = phone(dir.path(), base);
        super::super::vault::tidal_due_in_step(&p);
        p.set_account(super::super::stalled::refusing().await.account())
            .await;
        // The phone's vault key can't be read (not the homeserver: no other device is involved)
        std::fs::write(dir.path().join("vault").join("client.key"), b"not a key").unwrap();
        let s = session(&p).await.expect("renewed here");
        assert_eq!(s.access_token, "fresh-1");
        assert_eq!(renewed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn songs_waiting_on_a_failed_renewal_dont_each_try_again() {
        // TIDAL takes the connection and drops it after a while: a failure on the way
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let tries = Arc::new(AtomicUsize::new(0));
        let t2 = tries.clone();
        tokio::spawn(async move {
            while let Ok((c, _)) = l.accept().await {
                t2.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    drop(c);
                });
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let p = phone(dir.path(), base);
        due_here(&p);
        let songs: Vec<_> = (0..3)
            .map(|_| {
                let p = p.clone();
                tokio::spawn(async move { session(&p).await })
            })
            .collect();
        for s in songs {
            // The current session plays (it still works for 30 s), or the error of the one try
            let _ = s.await.unwrap();
        }
        assert_eq!(tries.load(Ordering::SeqCst), 1, "one try for all three");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn without_the_homeserver_a_sign_in_in_the_vault_isnt_renewed_here() {
        let (base, renewed) = tidal(std::time::Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let p = phone(dir.path(), base);
        // In step with the vault at the last sync; the homeserver didn't answer at start, so no
        // Pubky account: another device may renew it under the vault's lock
        super::super::vault::tidal_due_in_step(&p);
        assert!(p.account().await.is_none());
        let s = session(&p)
            .await
            .expect("the current session, still good for 30 s");
        assert_eq!(s.access_token, "due");
        assert_eq!(renewed.load(Ordering::SeqCst), 0, "no renewal beside the vault");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_renewal_that_cant_be_saved_is_used_and_never_renewed_again() {
        let (base, renewed) = tidal(std::time::Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let p = phone(dir.path(), base);
        due_here(&p);
        // The sign-ins file can't be replaced now (a folder in its place)
        let file = dir.path().join(super::super::store::FILE);
        let kept = std::fs::read(&file).unwrap();
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        // Renewed: the new session plays, and the next songs use it (no second renewal of the
        // session before, whose refresh token TIDAL may have spent)
        for _ in 0..3 {
            assert_eq!(session(&p).await.unwrap().access_token, "fresh-1");
        }
        assert_eq!(renewed.load(Ordering::SeqCst), 1);
        // The disk takes it again: saved at the next song
        std::fs::remove_dir(&file).unwrap();
        std::fs::write(&file, kept).unwrap();
        assert_eq!(session(&p).await.unwrap().access_token, "fresh-1");
        let on_disk = super::super::store::FileStore::open(dir.path()).unwrap();
        assert_eq!(full::load(&on_disk).session.unwrap().access_token, "fresh-1");
        assert!(p.tidal_unsaved.lock().unwrap().is_none());
        assert_eq!(renewed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_sign_in_made_while_the_vault_was_asked_is_not_renewed_over() {
        let (base, renewed) = tidal(std::time::Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let p = phone(dir.path(), base);
        due_here(&p);
        p.set_account(super::super::stalled::refusing().await.account())
            .await;
        // A vault pass holds the vault while the renewal starts; the person signs in again meanwhile
        let pass = p.vault_lock.lock().await;
        let p2 = p.clone();
        let renewal = tokio::spawn(async move { renew(&p2).await });
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        full::update(p.store.as_ref(), |st| {
            st.session = Some(full::Session {
                access_token: "new sign-in".into(),
                expires_at: now() + 3600,
                ..Default::default()
            })
        });
        drop(pass);
        let s = renewal.await.unwrap().unwrap();
        assert_eq!(s.access_token, "new sign-in");
        assert_eq!(
            renewed.load(Ordering::SeqCst),
            0,
            "the old sign-in wasn't renewed"
        );
        assert_eq!(state(&p).session.unwrap().access_token, "new sign-in");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_held_renewal_the_vault_doesnt_have_is_renewed_here() {
        let (base, renewed) = tidal(std::time::Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let p = phone(dir.path(), base);
        // In step with the vault, whose homeserver is down
        super::super::vault::tidal_due_in_step(&p);
        p.set_account(super::super::stalled::refusing().await.account())
            .await;
        // A renewal under the vault's lock that neither the vault nor the file took, held here
        // and due again: only the phone has its refresh token, so it is renewed here (through
        // the vault, the vault's copy of the session before would be renewed again)
        let file = dir.path().join(super::super::store::FILE);
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        let held = full::Session {
            access_token: "held".into(),
            refresh_token: Some("rt-held".into()),
            expires_at: now() + 30,
            ..Default::default()
        };
        hold_unsaved(&p, "due".into(), held, false);
        assert!(held_outside_vault(&p));
        // A vault pass runs meanwhile (stalled on the homeserver): it never syncs TIDAL while
        // this is held, so the renewal doesn't wait for it
        let pass = p.vault_lock.lock().await;
        let s = tokio::time::timeout(std::time::Duration::from_secs(5), renew(&p))
            .await
            .expect("not behind the pass")
            .expect("renewed here");
        drop(pass);
        assert_eq!(s.access_token, "fresh-1");
        assert_eq!(renewed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn leaving_phone_mode_ends_the_wait_for_a_code_sign_in() {
        let (base, renewed) = tidal(std::time::Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let p = phone(dir.path(), base);
        let pending = full::Pending {
            device_code: "dc".into(),
            user_code: "ABCD".into(),
            uri: "link.tidal.com".into(),
            expires_at: now() + 300,
            interval: 1,
        };
        change(&p, |st| {
            st.login = Some(pending.clone());
            true
        })
        .unwrap();
        let c = p.tidal.device.clone().unwrap();
        // Settings › Connect to a node while the code waits (TIDAL would approve it at once)
        p.close();
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            wait_for_approval(p.clone(), c, pending),
        )
        .await
        .expect("the wait ends");
        assert_eq!(renewed.load(Ordering::SeqCst), 0, "TIDAL wasn't asked");
        let st = state(&p);
        assert!(
            st.session.is_none() && st.login.is_none(),
            "nothing signed in, the code forgotten"
        );
    }

    #[tokio::test]
    async fn a_sign_out_that_cant_be_saved_doesnt_happen() {
        let dir = tempfile::tempdir().unwrap();
        let p = phone(dir.path(), "http://127.0.0.1:9".into());
        super::super::vault::tidal_due_in_step(&p);
        // The sign-ins file can't be replaced now (a folder in its place)
        let file = dir.path().join(super::super::store::FILE);
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        let r = logout(State(p.clone())).await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(state(&p).session.is_some(), "still signed in, as the file says");
        let r = put(
            State(p.clone()),
            Json(Update {
                enabled: Some(false),
                quality: None,
                client_id: None,
            }),
        )
        .await;
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(state(&p).enabled, "still on");
    }
}
