//! A listener's own Bookmarks on their own homeserver (TASK-listener-personal-library-1104, ADR
//! 0016 step 5); their Collections are in [`crate::personal_collections`]. The owner's stay what
//! they were: rows in the library, pushed by the owner's sync.
//!
//! - **The grant.** A listener approves the node in Ring a second time, for
//!   [`PERSONAL_CAPABILITIES`] only (their bookmarks and collections folders: no vault, no files).
//!   The node keeps that session only when the approver is the listener asking, in
//!   `<data>/grants/<z32>.session` (created 0600, never in a backup), and it goes when they
//!   disconnect or People › Remove removes them. It is never used for anyone else.
//! - **The store.** Their homeserver holds their bookmarks; the node keeps a copy
//!   (`person.<z32>.bookmarks`) so a page answers at once and a change made while the homeserver
//!   is away waits there (`pending`), written at the next read or write. A homeserver that
//!   refuses the session (revoked in Ring) marks the grant lost: writes answer 409
//!   `needs_connect` until they approve the node again, reads answer from the copy.
//! - **Never the owner's.** Nothing here reads or writes the owner's tables or the owner's sync;
//!   a listener without a grant gets an empty list and 409, never someone else's data.

use crate::accounts::PERSON_PREFIX;
use crate::state::NodeState;
use crate::types::{BookmarkChanged, PersonalState};
use crate::ApiError;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use pm_pubky::homeserver::{HomeserverError, LoginFlow, NodeHomeserver};
use pubky::PubkySession;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// What a listener approves so the node may keep their bookmarks and collections on their own
/// homeserver: those folders, nothing else.
pub const PERSONAL_CAPABILITIES: &str =
    "/priv/social/v1/bookmarks/:rw,/priv/social/v1/posts/:rw,/pub/social/v1/posts/:rw";

/// How long one call to a listener's homeserver may take before the node answers from its copy.
const CALL_LIMIT: Duration = Duration::from_secs(8);
/// How long a Ring approval for the grant may wait.
const CONNECT_TTL: Duration = Duration::from_secs(600);

/// Why a listener's homeserver didn't do what the node asked.
#[derive(Debug, Clone)]
pub enum PersonalError {
    /// The node has no grant from them.
    NoGrant,
    /// The homeserver refused the session (revoked in Ring, or not the folders it needs).
    Refused(String),
    /// It didn't answer, or not in time.
    Unreachable(String),
    /// It answered that it will never take this change (a 4xx other than a refusal of the
    /// session): the change is dropped, not tried again.
    Rejected(String),
}

/// A listener's homeserver as the node reaches it through their grant. The node's own is
/// [`Grants`]; tests stand in a fake ([`NodeState::set_personal_homeserver`]).
#[async_trait::async_trait]
pub trait PersonalHomeserver: Send + Sync {
    /// Whether the node holds a grant from `key`.
    fn connected(&self, key: &str) -> bool;
    /// Forgets the grant of `key` (their homeserver keeps everything).
    async fn disconnect(&self, key: &str);
    /// Every bookmark of `key`: the stored URI and when it was made (Unix microseconds).
    /// `known` is what the node has already: those aren't downloaded again.
    async fn bookmarks(
        &self,
        key: &str,
        known: &[(String, i64)],
    ) -> Result<Vec<(String, i64)>, PersonalError>;
    async fn put_bookmark(&self, key: &str, uri: &str, created_at: i64) -> Result<(), PersonalError>;
    /// Removes the bookmark of `uri`; one that isn't there is no error.
    async fn delete_bookmark(&self, key: &str, uri: &str) -> Result<(), PersonalError>;
    /// Every file under `dir` (one of the granted folders), at any depth; a folder that isn't
    /// there is empty.
    async fn list(&self, key: &str, dir: &str) -> Result<Vec<String>, PersonalError>;
    /// The file at `path`; `None` when it isn't there.
    async fn read(&self, key: &str, path: &str) -> Result<Option<Vec<u8>>, PersonalError>;
    async fn write(&self, key: &str, path: &str, body: &serde_json::Value) -> Result<(), PersonalError>;
    /// Removes the file at `path`; one that isn't there is no error.
    async fn remove(&self, key: &str, path: &str) -> Result<(), PersonalError>;
}

/// Where grants are kept: `<data>/grants/`, next to the library.
fn grants_dir(s: &NodeState) -> PathBuf {
    s.db_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("grants")
}

/// The file of one person's grant. A key is z32 (letters and digits); anything else has no file.
fn grant_file(dir: &std::path::Path, key: &str) -> Option<PathBuf> {
    let key = key.trim_start_matches("pubky");
    (!key.is_empty() && key.len() <= 64 && key.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then(|| dir.join(format!("{key}.session")))
}

/// Restored sessions, by key: a grant is read from its file once.
fn sessions() -> &'static Mutex<HashMap<String, PubkySession>> {
    static S: OnceLock<Mutex<HashMap<String, PubkySession>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

// What a homeserver error means comes from the error itself, never from its text, where a URL
// or an id can hold any digits.

/// The HTTP status a homeserver answered with, when it answered.
fn status_of(e: &HomeserverError) -> Option<u16> {
    use pubky::errors::{Error as Pe, RequestError};
    match e {
        HomeserverError::Pubky(Pe::Request(RequestError::Server { status, .. })) => Some(status.as_u16()),
        _ => None,
    }
}

fn is_auth_error(e: &HomeserverError) -> bool {
    matches!(e, HomeserverError::Pubky(pubky::errors::Error::Authentication(_)))
}

/// A session that could not be restored or refreshed: the grant is gone (expired, revoked, the
/// user or the grant unknown to the homeserver: any 4xx but "slow down", or an auth error), or
/// the homeserver is away.
fn classify_grant(e: &HomeserverError) -> PersonalError {
    let msg = e.to_string();
    match status_of(e) {
        Some(408 | 429) => PersonalError::Unreachable(msg),
        Some(400..=499) => PersonalError::Refused(msg),
        _ if is_auth_error(e) => PersonalError::Refused(msg),
        _ => PersonalError::Unreachable(msg),
    }
}

/// A read or a write that failed. Only what the homeserver says it will never take (400, 413,
/// 415, 422) is dropped; any other 4xx is the grant's (a grant or user gone answers 404): lost,
/// approve again, and the change keeps waiting.
fn classify_call(e: &HomeserverError) -> PersonalError {
    let msg = e.to_string();
    match status_of(e) {
        Some(400 | 413 | 415 | 422) => PersonalError::Rejected(msg),
        Some(408 | 429) => PersonalError::Unreachable(msg),
        Some(400..=499) => PersonalError::Refused(msg),
        _ if is_auth_error(e) => PersonalError::Refused(msg),
        _ => PersonalError::Unreachable(msg),
    }
}

/// The homeserver says there is nothing there.
fn is_not_found(e: &HomeserverError) -> bool {
    status_of(e) == Some(404)
}

/// The node's own reach into listeners' homeservers: their grant files and the node's client.
pub struct Grants {
    dir: PathBuf,
    hs: Option<NodeHomeserver>,
}

impl Grants {
    fn new(s: &NodeState) -> Self {
        Self {
            dir: grants_dir(s),
            hs: s.homeserver().ok(),
        }
    }

    fn client(&self) -> Result<&NodeHomeserver, PersonalError> {
        self.hs
            .as_ref()
            .ok_or_else(|| PersonalError::Unreachable("the node's Pubky client didn't start".into()))
    }

    /// Their session, and whether it was restored from the grant just now.
    async fn session(&self, key: &str) -> Result<(PubkySession, bool), PersonalError> {
        if let Some(s) = sessions().lock().expect("sessions").get(key) {
            return Ok((s.clone(), false));
        }
        let file = grant_file(&self.dir, key).ok_or(PersonalError::NoGrant)?;
        let secret = std::fs::read_to_string(&file).map_err(|_| PersonalError::NoGrant)?;
        let session = tokio::time::timeout(CALL_LIMIT, self.client()?.restore_session(secret.trim()))
            .await
            .map_err(|_| PersonalError::Unreachable("the homeserver took too long".into()))?
            .map_err(|e| self.noted(key, classify_grant(&e)))?;
        sessions()
            .lock()
            .expect("sessions")
            .insert(key.to_string(), session.clone());
        Ok((session, true))
    }

    /// A call to their homeserver, time-limited; a refusal forgets the restored session.
    async fn limited<T>(
        &self,
        key: &str,
        f: impl std::future::Future<Output = Result<T, HomeserverError>>,
    ) -> Result<T, PersonalError> {
        match tokio::time::timeout(CALL_LIMIT, f).await {
            Err(_) => Err(PersonalError::Unreachable("the homeserver took too long".into())),
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(self.noted(key, classify_call(&e))),
        }
    }

    /// A refusal forgets the restored session (the next call reads the grant again).
    fn noted(&self, key: &str, e: PersonalError) -> PersonalError {
        if matches!(e, PersonalError::Refused(_)) {
            sessions().lock().expect("sessions").remove(key);
        }
        e
    }

    /// A call about one file, time-limited: `None` when the file isn't there. On a session
    /// restored from the grant just now a 404 is the file's; on an older one it may be the grant
    /// gone, so the session is restored again (a grant that is gone fails there) and the call
    /// asked once more.
    async fn maybe_there<T, F, Fut>(&self, key: &str, f: F) -> Result<Option<T>, PersonalError>
    where
        F: Fn(PubkySession) -> Fut,
        Fut: std::future::Future<Output = Result<T, HomeserverError>>,
    {
        let (session, fresh) = self.session(key).await?;
        match tokio::time::timeout(CALL_LIMIT, f(session)).await {
            Err(_) => Err(PersonalError::Unreachable("the homeserver took too long".into())),
            Ok(Ok(v)) => Ok(Some(v)),
            Ok(Err(e)) if is_not_found(&e) && fresh => Ok(None),
            Ok(Err(e)) if is_not_found(&e) => {
                sessions().lock().expect("sessions").remove(key);
                let (session, _) = self.session(key).await?;
                match tokio::time::timeout(CALL_LIMIT, f(session)).await {
                    Err(_) => Err(PersonalError::Unreachable("the homeserver took too long".into())),
                    Ok(Ok(v)) => Ok(Some(v)),
                    Ok(Err(e)) if is_not_found(&e) => Ok(None),
                    Ok(Err(e)) => Err(self.noted(key, classify_call(&e))),
                }
            }
            Ok(Err(e)) => Err(self.noted(key, classify_call(&e))),
        }
    }
}

/// How a path of theirs is reached: `/pub/` publicly, anything else with their session.
fn visibility_of<'a>(
    path: &str,
    session: &'a PubkySession,
) -> (pm_pubky::Visibility, Option<&'a PubkySession>) {
    if path.starts_with("/pub/") {
        (pm_pubky::Visibility::Public, None)
    } else {
        (pm_pubky::Visibility::Private, Some(session))
    }
}

#[async_trait::async_trait]
impl PersonalHomeserver for Grants {
    fn connected(&self, key: &str) -> bool {
        grant_file(&self.dir, key).is_some_and(|f| f.is_file())
    }

    async fn disconnect(&self, key: &str) {
        sessions().lock().expect("sessions").remove(key);
        if let Some(f) = grant_file(&self.dir, key) {
            if let Err(e) = std::fs::remove_file(&f) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(path = %f.display(), "cannot remove a grant: {e}");
                }
            }
        }
    }

    async fn bookmarks(
        &self,
        key: &str,
        known: &[(String, i64)],
    ) -> Result<Vec<(String, i64)>, PersonalError> {
        let (session, _) = self.session(key).await?;
        let hs = self.client()?;
        // A listing, and only the bookmarks the node doesn't know yet (as the owner's sync)
        let known: HashMap<String, (String, i64)> = known
            .iter()
            .filter_map(|(uri, at)| {
                let path = pm_pubky::v1::bookmark_path(uri).ok()?;
                Some((path.rsplit('/').next()?.to_string(), (uri.clone(), *at)))
            })
            .collect();
        let all = self
            .limited(key, hs.read_v1_bookmarks_except(&session, &known))
            .await?;
        Ok(all.into_iter().map(|(_, uri, at)| (uri, at)).collect())
    }

    async fn put_bookmark(&self, key: &str, uri: &str, created_at: i64) -> Result<(), PersonalError> {
        let (object, path) =
            pm_pubky::v1::bookmark_of(uri, created_at).map_err(PersonalError::Unreachable)?;
        let (session, _) = self.session(key).await?;
        let hs = self.client()?;
        self.limited(key, hs.write_object(&session, &path, &object)).await
    }

    async fn delete_bookmark(&self, key: &str, uri: &str) -> Result<(), PersonalError> {
        let path = pm_pubky::v1::bookmark_path(uri).map_err(PersonalError::Unreachable)?;
        self.remove(key, &path).await
    }

    async fn list(&self, key: &str, dir: &str) -> Result<Vec<String>, PersonalError> {
        let (session, _) = self.session(key).await?;
        let hs = self.client()?;
        let (visibility, session) = visibility_of(dir, &session);
        self.limited(key, hs.list_tree(key, dir, visibility, session))
            .await
    }

    async fn read(&self, key: &str, path: &str) -> Result<Option<Vec<u8>>, PersonalError> {
        let hs = self.client()?;
        self.maybe_there(key, |session| async move {
            let (visibility, session) = visibility_of(path, &session);
            hs.get_bytes(key, path, visibility, session).await
        })
        .await
    }

    async fn write(&self, key: &str, path: &str, body: &serde_json::Value) -> Result<(), PersonalError> {
        let (session, _) = self.session(key).await?;
        let hs = self.client()?;
        self.limited(key, hs.write_object(&session, path, body)).await
    }

    async fn remove(&self, key: &str, path: &str) -> Result<(), PersonalError> {
        let hs = self.client()?;
        self.maybe_there(key, |session| async move { hs.delete(&session, path).await })
            .await
            .map(|_| ())
    }
}

/// The listener's homeserver as this node reaches it: a test's stand-in, else the grants.
pub(crate) fn backend(s: &NodeState) -> Arc<dyn PersonalHomeserver> {
    s.personal_override()
        .unwrap_or_else(|| Arc::new(Grants::new(s)) as Arc<dyn PersonalHomeserver>)
}

// ---- the node's copy -------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Copy {
    /// Their bookmarks as the node last knew them, its own changes included.
    #[serde(default)]
    items: Vec<Item>,
    /// Changes not on their homeserver yet, oldest first.
    #[serde(default)]
    pending: Vec<Change>,
    /// Their homeserver refused the grant: nothing is tried until they approve the node again.
    #[serde(default)]
    lost: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Item {
    uri: String,
    at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Change {
    uri: String,
    on: bool,
    at: i64,
}

fn copy_key(key: &str) -> String {
    format!("{PERSON_PREFIX}{}.bookmarks", key.trim_start_matches("pubky"))
}

fn load(s: &NodeState, key: &str) -> Copy {
    s.library
        .lock()
        .expect("library poisoned")
        .get_setting(&copy_key(key))
        .ok()
        .flatten()
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default()
}

/// One listener's copy changes one request at a time: two devices, or a read writing what waited
/// while a heart comes in, would otherwise each save their own and lose the other's.
type PersonLocks = Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>;

pub(crate) fn person_lock(key: &str) -> Arc<tokio::sync::Mutex<()>> {
    static L: OnceLock<PersonLocks> = OnceLock::new();
    L.get_or_init(Default::default)
        .lock()
        .expect("person locks")
        .entry(key.trim_start_matches("pubky").to_string())
        .or_default()
        .clone()
}

fn save(s: &NodeState, key: &str, c: &Copy) -> Result<(), ApiError> {
    // Removed (People › Remove) while this request waited on their homeserver: nothing of
    // theirs comes back
    if !crate::people::may_use_music(s, key) {
        return Ok(());
    }
    s.library
        .lock()
        .expect("library poisoned")
        .set_setting(&copy_key(key), &serde_json::to_string(c).expect("serializes"))?;
    Ok(())
}

/// Whether their homeserver refused the grant (approve the node again).
pub(crate) fn is_lost(s: &NodeState, key: &str) -> bool {
    load(s, key).lost
}

/// Their homeserver refused the grant: nothing is tried until they approve the node again.
pub(crate) fn mark_lost(s: &NodeState, key: &str, why: &str) -> Result<(), ApiError> {
    tracing::info!(%key, "a listener's homeserver refused the node: {why}");
    let mut c = load(s, key);
    c.lost = true;
    save(s, key, &c)
}

impl Copy {
    fn apply(&mut self, uri: &str, on: bool, at: i64) {
        self.items.retain(|i| i.uri != uri);
        if on {
            self.items.push(Item {
                uri: uri.to_string(),
                at,
            });
        }
    }
}

/// The answer a listener gets when the node can't write for them: connect (again) first.
pub(crate) fn needs_connect(lost: bool) -> Response {
    let error = if lost {
        "Your homeserver no longer lets this node in: approve it again (Settings › You › My homeserver)"
    } else {
        "Connect your homeserver first (Settings › You › My homeserver): your bookmarks and collections are kept there"
    };
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({"error": error, "needs_connect": true, "lost": lost})),
    )
        .into_response()
}

/// A listener's bookmarks, newest from their homeserver when it answers (writing what waited
/// first), else the node's copy, with where they stand.
pub(crate) async fn bookmark_uris(
    s: &NodeState,
    key: &str,
) -> Result<(Vec<String>, PersonalState), ApiError> {
    let b = backend(s);
    let lock = person_lock(key);
    let _one = lock.lock().await;
    let connected = b.connected(key);
    // Not connected (or disconnected): theirs aren't known here; the copy waits for a reconnect
    if !connected {
        return Ok((Vec::new(), PersonalState::default()));
    }
    let mut c = load(s, key);
    let mut stale = false;
    if !c.lost {
        while let Some(ch) = c.pending.first().cloned() {
            let r = if ch.on {
                b.put_bookmark(key, &ch.uri, ch.at).await
            } else {
                b.delete_bookmark(key, &ch.uri).await
            };
            match r {
                Ok(()) => {
                    c.pending.remove(0);
                }
                Err(PersonalError::Refused(m)) => {
                    tracing::info!(%key, "a listener's homeserver refused the node: {m}");
                    c.lost = true;
                    break;
                }
                // Never taken: dropped, and the rest go on
                Err(PersonalError::Rejected(m)) => {
                    tracing::warn!(%key, uri = %ch.uri, "a listener's homeserver won't take a waiting change; dropped: {m}");
                    c.pending.remove(0);
                }
                Err(_) => {
                    stale = true;
                    break;
                }
            }
        }
        if !c.lost && !stale {
            let known: Vec<(String, i64)> = c.items.iter().map(|i| (i.uri.clone(), i.at)).collect();
            match b.bookmarks(key, &known).await {
                Ok(list) => {
                    c.items = list.into_iter().map(|(uri, at)| Item { uri, at }).collect();
                }
                Err(PersonalError::Refused(m)) => {
                    tracing::info!(%key, "a listener's homeserver refused the node: {m}");
                    c.lost = true;
                }
                Err(_) => stale = true,
            }
        }
        save(s, key, &c)?;
    }
    let mut items = c.items.clone();
    items.sort_by_key(|i| std::cmp::Reverse(i.at));
    let state = PersonalState {
        connected,
        lost: c.lost,
        pending: c.pending.len(),
        stale,
    };
    Ok((items.into_iter().map(|i| i.uri).collect(), state))
}

/// Bookmarks (`on`) or un-bookmarks `uri` (its stored form) for listener `key`. `at` (Unix
/// seconds) is when a device made the change offline: a bookmark keeps that time, and a clear
/// older than the bookmark the node knows leaves it (it was bookmarked again since).
pub(crate) async fn set_bookmark(
    s: &NodeState,
    key: &str,
    uri: String,
    on: bool,
    at: Option<i64>,
) -> Result<Response, ApiError> {
    let b = backend(s);
    let lock = person_lock(key);
    let _one = lock.lock().await;
    if !b.connected(key) {
        return Ok(needs_connect(false));
    }
    let mut c = load(s, key);
    if c.lost {
        return Ok(needs_connect(true));
    }
    let now = pm_pubky::ids::now_micros();
    let at = at.map_or(now, |a| a.clamp(0, now / 1_000_000).saturating_mul(1_000_000));
    if !on
        && c.items
            .iter()
            .any(|i| i.uri == uri && i.at >= at.saturating_add(1_000_000))
    {
        return Ok(Json(BookmarkChanged::new(uri, true)).into_response());
    }
    let r = if on {
        b.put_bookmark(key, &uri, at).await
    } else {
        b.delete_bookmark(key, &uri).await
    };
    let pending = match r {
        // Written: whatever still waited for this one is older and goes
        Ok(()) => {
            c.pending.retain(|p| p.uri != uri);
            false
        }
        Err(PersonalError::NoGrant) => return Ok(needs_connect(false)),
        Err(PersonalError::Rejected(m)) => {
            return Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                format!("Your homeserver won't keep this bookmark: {m}"),
            ))
        }
        Err(PersonalError::Refused(m)) => {
            tracing::info!(%key, "a listener's homeserver refused the node: {m}");
            c.lost = true;
            save(s, key, &c)?;
            return Ok(needs_connect(true));
        }
        Err(PersonalError::Unreachable(m)) => {
            tracing::info!(%key, "a listener's homeserver didn't answer; the change waits: {m}");
            c.pending.retain(|p| p.uri != uri);
            c.pending.push(Change {
                uri: uri.clone(),
                on,
                at,
            });
            true
        }
    };
    c.apply(&uri, on, at);
    save(s, key, &c)?;
    let mut answer = BookmarkChanged::new(uri, on);
    answer.pending = pending;
    Ok(Json(answer).into_response())
}

// ---- connecting a listener's homeserver ------------------------------------------------------

/// Whether a Ring approval may become `key`'s grant: approved by them, for every folder.
pub(crate) fn may_keep(key: &str, approver: &str, granted: &[String]) -> Result<(), &'static str> {
    if approver.trim_start_matches("pubky") != key.trim_start_matches("pubky") {
        return Err("approved by another identity than yours");
    }
    if !pm_pubky::homeserver::missing_capabilities(granted, PERSONAL_CAPABILITIES).is_empty() {
        return Err("not approved for your bookmarks and collections folders");
    }
    Ok(())
}

/// A Ring approval in progress. Its poll runs in a task of its own: after an approval it goes
/// on to exchange it with the listener's homeserver, and a request that gives up (or goes
/// away) must not cancel that, or the approval is lost.
type Polled = (Box<LoginFlow>, Result<Option<PubkySession>, HomeserverError>);
enum Asking {
    /// Nobody is asking it now (boxed: a flow is large, the other states a pointer or nothing).
    Waiting(Box<LoginFlow>),
    /// Being asked; a poll picks up its answer.
    Asked(tokio::task::JoinHandle<Polled>),
    /// A poll of theirs holds it this moment.
    Busy,
}

/// Ring approvals in progress: id → (where it stands, started, the listener who started it).
type Connects = tokio::sync::Mutex<HashMap<String, (Asking, Instant, String)>>;

fn connects() -> &'static Connects {
    static C: OnceLock<Connects> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn listener_key(c: &crate::accounts::Caller) -> Result<String, ApiError> {
    match (c.role, &c.key) {
        (crate::accounts::Role::Listener, Some(k)) => Ok(k.trim_start_matches("pubky").to_string()),
        _ => Err(ApiError::bad_request(
            "this is for listeners: the node's own homeserver is set in Settings › Pubky",
        )),
    }
}

/// `GET /v1/me/homeserver`: `{connected, lost, pending}` for a listener.
pub(crate) async fn status(
    axum::extract::State(s): axum::extract::State<Arc<NodeState>>,
    axum::Extension(c): axum::Extension<crate::accounts::Caller>,
) -> Result<Json<PersonalState>, ApiError> {
    let key = listener_key(&c)?;
    let connected = backend(&s).connected(&key);
    let copy = load(&s, &key);
    Ok(Json(PersonalState {
        connected,
        lost: connected && copy.lost,
        pending: copy.pending.len() + crate::personal_collections::pending(&s, &key),
        stale: false,
    }))
}

/// `DELETE /v1/me/homeserver`: the node forgets the listener's grant; their homeserver and the
/// node's copy keep everything.
pub(crate) async fn disconnect(
    axum::extract::State(s): axum::extract::State<Arc<NodeState>>,
    axum::Extension(c): axum::Extension<crate::accounts::Caller>,
) -> Result<Json<PersonalState>, ApiError> {
    let key = listener_key(&c)?;
    let lock = person_lock(&key);
    let _one = lock.lock().await;
    backend(&s).disconnect(&key).await;
    let copy = load(&s, &key);
    Ok(Json(PersonalState {
        connected: false,
        lost: false,
        pending: copy.pending.len() + crate::personal_collections::pending(&s, &key),
        stale: false,
    }))
}

/// `POST /v1/me/homeserver/connect`: a Ring approval of [`PERSONAL_CAPABILITIES`] for the
/// listener asking. `{id, url, qr_svg}`; poll it with `GET …/connect/poll?id=`.
pub(crate) async fn connect_start(
    axum::extract::State(s): axum::extract::State<Arc<NodeState>>,
    axum::Extension(c): axum::Extension<crate::accounts::Caller>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let key = listener_key(&c)?;
    let hs = s
        .homeserver()
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;
    let flow = hs
        .start_login_with("pubky-music-listener", PERSONAL_CAPABILITIES)
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;
    let url = flow.url().to_string();
    let id = crate::tidal::random_urlsafe(24);
    let mut all = connects().lock().await;
    all.retain(|_, (_, at, _)| at.elapsed() < CONNECT_TTL);
    // One at a time per listener: a new one replaces theirs
    all.retain(|_, (_, _, who)| who != &key);
    all.insert(id.clone(), (Asking::Waiting(Box::new(flow)), Instant::now(), key));
    Ok(Json(
        serde_json::json!({"id": id, "qr_svg": crate::qr_svg(&url), "url": url}),
    ))
}

/// `GET /v1/me/homeserver/connect/poll?id=`: `pending`, `ok` (the grant is kept), `denied`
/// (approved by someone else, or not for every folder: nothing kept) or `expired`.
pub(crate) async fn connect_poll(
    axum::extract::State(s): axum::extract::State<Arc<NodeState>>,
    axum::Extension(c): axum::Extension<crate::accounts::Caller>,
    axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let key = listener_key(&c)?;
    let id = q.get("id").cloned().unwrap_or_default();
    let pending = || Ok(Json(serde_json::json!({"state": "pending"})));
    // Only the listener who started it may finish it. The shared map is held only to look: one
    // slow homeserver holds up nobody else's
    let mut handle = {
        let mut all = connects().lock().await;
        let Some((state, started, who)) = all.get_mut(&id) else {
            return Ok(Json(serde_json::json!({"state": "expired"})));
        };
        if who != &key {
            return Ok(Json(serde_json::json!({"state": "expired"})));
        }
        if started.elapsed() > CONNECT_TTL {
            all.remove(&id);
            return Ok(Json(serde_json::json!({"state": "expired"})));
        }
        match std::mem::replace(state, Asking::Busy) {
            // Another tab of theirs is asking it this moment
            Asking::Busy => return pending(),
            Asking::Waiting(flow) => tokio::spawn(async move {
                let r = flow.poll().await;
                (flow, r)
            }),
            Asking::Asked(h) => h,
        }
    };
    let set = |id: String, st: Asking| async move {
        if let Some(e) = connects().lock().await.get_mut(&id) {
            e.0 = st;
        }
    };
    let session = match tokio::time::timeout(Duration::from_secs(5), &mut handle).await {
        // Still asking (an approval being exchanged): the next poll takes its answer
        Err(_) => {
            set(id, Asking::Asked(handle)).await;
            return pending();
        }
        Ok(Ok((flow, Ok(None)))) => {
            set(id, Asking::Waiting(flow)).await;
            return pending();
        }
        Ok(Ok((_, Ok(Some(session))))) => {
            connects().lock().await.remove(&id);
            session
        }
        Ok(Ok((_, Err(e)))) => {
            connects().lock().await.remove(&id);
            return Err(ApiError::new(StatusCode::BAD_GATEWAY, e.to_string()));
        }
        Ok(Err(e)) => {
            connects().lock().await.remove(&id);
            return Err(ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
        }
    };
    let approver = session.public_key().to_string();
    let granted = pm_pubky::homeserver::session_capabilities(&session);
    if let Err(why) = may_keep(&key, &approver, &granted) {
        tracing::warn!(%key, %approver, "a listener's homeserver grant was refused: {why}");
        return Ok(Json(serde_json::json!({"state": "denied", "reason": why})));
    }
    let Some(secret) = pm_pubky::homeserver::export_session_secret(&session).await else {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "Ring approved, but the session can't be kept: try again",
        ));
    };
    let lock = person_lock(&key);
    let _one = lock.lock().await;
    // Their music access went meanwhile: nothing is kept
    if !crate::people::may_use_music(&s, &key) {
        return Ok(Json(
            serde_json::json!({"state": "denied", "reason": "you have no music access on this node any more"}),
        ));
    }
    let file = grant_file(&grants_dir(&s), &key).ok_or_else(|| ApiError::bad_request("not a Pubky key"))?;
    let _ = std::fs::remove_file(&file);
    crate::imports::write_private(&file, secret.as_bytes()).map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot keep the grant: {e}"),
        )
    })?;
    sessions().lock().expect("sessions").insert(key.clone(), session);
    // Approved again: the node tries their homeserver again
    let mut copy = load(&s, &key);
    copy.lost = false;
    save(&s, &key, &copy)?;
    tracing::info!(%key, "a listener connected their homeserver");
    Ok(Json(serde_json::json!({"state": "ok"})))
}

/// People › Remove: the grant of `key` goes (in the background: the store is async).
pub(crate) fn forget(s: &NodeState, key: &str) {
    let b = backend(s);
    let key = key.trim_start_matches("pubky").to_string();
    sessions().lock().expect("sessions").remove(&key);
    match tokio::runtime::Handle::try_current() {
        Ok(h) => {
            h.spawn(async move {
                let lock = person_lock(&key);
                let _one = lock.lock().await;
                b.disconnect(&key).await;
            });
        }
        Err(_) => {
            if let Some(f) = grant_file(&grants_dir(s), &key) {
                let _ = std::fs::remove_file(f);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grant_is_kept_only_from_its_listener_for_every_folder() {
        let all: Vec<String> = PERSONAL_CAPABILITIES.split(',').map(String::from).collect();
        let k = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";
        assert!(may_keep(k, k, &all).is_ok());
        assert!(
            may_keep(k, &format!("pubky{k}"), &all).is_ok(),
            "the same key, either spelling"
        );
        assert!(may_keep(k, "pxnu33x7jtpx9ar1ytsi4yxbp6a5o36gwhffs8zoxmbuptici1jy", &all).is_err());
        assert!(
            may_keep(k, k, &all[..1]).is_err(),
            "bookmarks alone is not enough"
        );
        assert!(may_keep(k, k, &["/pub/pubky.app/:r".to_string()]).is_err());
    }

    #[test]
    fn what_a_homeserver_error_means_for_the_grant() {
        use pubky::errors::{AuthError, Error as Pe, RequestError};
        let server = |c: u16| {
            HomeserverError::Pubky(Pe::Request(RequestError::Server {
                status: reqwest::StatusCode::from_u16(c).unwrap(),
                message: String::new(),
            }))
        };
        // A call: only what will never be taken is dropped; a grant or user gone (404, 409, …) is
        // lost (approve again), and the change keeps waiting
        for c in [401, 403, 404, 405, 409] {
            assert!(
                matches!(classify_call(&server(c)), PersonalError::Refused(_)),
                "{c}"
            );
        }
        for c in [400, 413, 415, 422] {
            assert!(
                matches!(classify_call(&server(c)), PersonalError::Rejected(_)),
                "{c}"
            );
        }
        for c in [408, 429, 500, 503] {
            assert!(
                matches!(classify_call(&server(c)), PersonalError::Unreachable(_)),
                "{c}"
            );
        }
        // The grant: unknown user or grant (404), a broken grant (400), revoked: lost, never dropped
        for c in [400, 401, 403, 404] {
            assert!(
                matches!(classify_grant(&server(c)), PersonalError::Refused(_)),
                "{c}"
            );
        }
        for c in [408, 429, 502] {
            assert!(
                matches!(classify_grant(&server(c)), PersonalError::Unreachable(_)),
                "{c}"
            );
        }
        assert!(is_not_found(&server(404)) && !is_not_found(&server(403)));
        // An expired grant: lost (approve again), never "waiting" for good
        let expired = HomeserverError::Pubky(Pe::Authentication(AuthError::Validation(
            "stored grant credential has expired".into(),
        )));
        assert!(matches!(classify_grant(&expired), PersonalError::Refused(_)));
        assert!(matches!(classify_call(&expired), PersonalError::Refused(_)));
        // Digits in a URL decide nothing: a homeserver on port 4031, a path with 403 in it
        let net = HomeserverError::Transport(
            "error sending request for url (http://hs.example:4031/pub/403/404)".into(),
        );
        assert!(matches!(classify_call(&net), PersonalError::Unreachable(_)));
        assert!(matches!(classify_grant(&net), PersonalError::Unreachable(_)));
        assert!(!is_not_found(&net));
        assert!(matches!(
            classify_call(&HomeserverError::Timeout),
            PersonalError::Unreachable(_)
        ));
    }

    #[test]
    fn a_grant_file_is_named_by_a_key_only() {
        let d = std::path::Path::new("/data/grants");
        assert_eq!(grant_file(d, "pubkyabc123").unwrap(), d.join("abc123.session"));
        for bad in ["", "../x", "a/b", "a.b", "abc def"] {
            assert!(grant_file(d, bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn a_kept_grant_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let f = grant_file(dir.path(), "abc").unwrap();
        crate::imports::write_private(&f, b"secret").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }
}
