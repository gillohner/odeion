//! A listener's own Bookmarks on their own homeserver (TASK-listener-personal-library-1104, ADR
//! 0016 step 5). The owner's stay what they were: rows in the library, pushed by the owner's sync.
//!
//! - **The grant.** A listener approves the node in Ring a second time, for
//!   [`PERSONAL_CAPABILITIES`] only (their bookmarks folder: no posts, no vault, no files).
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
use pm_pubky::homeserver::{HomeserverError, NodeHomeserver};
use pubky::PubkySession;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// What a listener approves so the node may keep their bookmarks on their own homeserver: that
/// folder, nothing else (not their posts: nothing here writes them).
pub const PERSONAL_CAPABILITIES: &str = "/priv/social/v1/bookmarks/:rw";

/// How long one call to a listener's homeserver may take before the node answers from its copy.
const CALL_LIMIT: Duration = Duration::from_secs(8);
/// How long a Ring approval for the grant may wait.
const CONNECT_TTL: Duration = Duration::from_secs(600);

tokio::task_local! {
    /// Captured before token authentication, including time spent waiting before a handler.
    pub(crate) static AUTHENTICATED_ACCESS: u64;
}

fn request_started(s: &NodeState) -> u64 {
    AUTHENTICATED_ACCESS
        .try_with(|revision| *revision)
        .unwrap_or_else(|_| crate::people::access_started(s))
}

fn require_access(s: &NodeState, key: &str, started: u64) -> Result<(), ApiError> {
    crate::people::access_current(s, key.trim_start_matches("pubky"), started)?;
    if !crate::people::may_use_music(s, key) {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "music access has ended"));
    }
    Ok(())
}

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
    /// Forgets the grant of `key` (their homeserver keeps everything). An error when it is still
    /// there (it couldn't be deleted).
    async fn disconnect(&self, key: &str) -> Result<(), String>;
    /// Every bookmark of `key`: the stored URI and when it was made (Unix microseconds).
    /// `known` is what the node has already. Timestamps remain mutable at the same path and
    /// must be refreshed; the known list only helps detect an unexpectedly empty listing.
    async fn bookmarks(
        &self,
        key: &str,
        known: &[(String, i64)],
    ) -> Result<Vec<(String, i64)>, PersonalError>;
    async fn put_bookmark(&self, key: &str, uri: &str, created_at: i64) -> Result<(), PersonalError>;
    /// Removes the bookmark of `uri`; one that isn't there is no error.
    async fn delete_bookmark(&self, key: &str, uri: &str) -> Result<(), PersonalError>;
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

/// A refusal that may be the grant's rather than the call's: any 4xx but "slow down", or an auth
/// error (a grant refreshed inside a call fails there).
fn grant_like(e: &HomeserverError) -> bool {
    matches!(status_of(e), Some(s) if (400..500).contains(&s) && s != 408 && s != 429) || is_auth_error(e)
}

fn too_slow() -> PersonalError {
    PersonalError::Unreachable("the homeserver took too long".into())
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
        self.may_hold(
            key,
            &session.public_key().to_string(),
            &pm_pubky::homeserver::session_capabilities(&session),
        )?;
        sessions()
            .lock()
            .expect("sessions")
            .insert(key.to_string(), session.clone());
        Ok((session, true))
    }

    /// Whether a grant read back from its file may stay: theirs, and for exactly what the node
    /// asks for now ([`may_keep`]). One that doesn't fit (kept by an earlier build that asked for
    /// more) is removed, so the node holds no key wider than it uses; they connect again.
    fn may_hold(&self, key: &str, approver: &str, granted: &[String]) -> Result<(), PersonalError> {
        let Err(why) = may_keep(key, approver, granted) else {
            return Ok(());
        };
        sessions().lock().expect("sessions").remove(key);
        if let Some(f) = grant_file(&self.dir, key) {
            let _ = std::fs::remove_file(f);
        }
        tracing::warn!(%key, "a kept listener grant doesn't fit what the node asks for; removed: {why}");
        Err(PersonalError::Refused(format!(
            "{why}: connect your homeserver again"
        )))
    }

    /// One call to their homeserver with their session, time-limited. With `gone_ok`, a 404 is
    /// the file's (`None`). On a session restored earlier, a refusal may be its grant's (gone, or
    /// a refresh of it failing inside the call), so the session is restored afresh (a grant that
    /// is gone fails there) and the call asked once more; so is an answer `suspect` finds
    /// unlikely (an empty listing where there was something: a gone user's folders list as
    /// empty). A refusal forgets the session.
    async fn attempt<T, F, Fut>(
        &self,
        key: &str,
        gone_ok: bool,
        suspect: impl Fn(&T) -> bool,
        f: F,
    ) -> Result<Option<T>, PersonalError>
    where
        F: Fn(PubkySession) -> Fut,
        Fut: std::future::Future<Output = Result<T, HomeserverError>>,
    {
        let (session, fresh) = self.session(key).await?;
        match tokio::time::timeout(CALL_LIMIT, f(session)).await {
            Err(_) => return Err(too_slow()),
            Ok(Ok(v)) if fresh || !suspect(&v) => return Ok(Some(v)),
            Ok(Ok(_)) => {}
            Ok(Err(e)) if gone_ok && fresh && is_not_found(&e) => return Ok(None),
            Ok(Err(e)) if fresh || !grant_like(&e) => return Err(self.noted(key, classify_call(&e))),
            Ok(Err(_)) => {}
        }
        sessions().lock().expect("sessions").remove(key);
        let (session, _) = self.session(key).await?;
        match tokio::time::timeout(CALL_LIMIT, f(session)).await {
            Err(_) => Err(too_slow()),
            Ok(Ok(v)) => Ok(Some(v)),
            Ok(Err(e)) if gone_ok && is_not_found(&e) => Ok(None),
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
}

#[async_trait::async_trait]
impl PersonalHomeserver for Grants {
    fn connected(&self, key: &str) -> bool {
        grant_file(&self.dir, key).is_some_and(|f| f.is_file())
    }

    async fn disconnect(&self, key: &str) -> Result<(), String> {
        sessions().lock().expect("sessions").remove(key);
        if let Some(f) = grant_file(&self.dir, key) {
            if let Err(e) = std::fs::remove_file(&f) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(path = %f.display(), "cannot remove a grant: {e}");
                    return Err(format!("the node couldn't delete its copy of your approval: {e}"));
                }
            }
        }
        Ok(())
    }

    async fn bookmarks(
        &self,
        key: &str,
        known: &[(String, i64)],
    ) -> Result<Vec<(String, i64)>, PersonalError> {
        let hs = self.client()?;
        // A re-heart keeps its URI-derived filename but has a new timestamp. Cached names
        // cannot stand in for current objects when deciding whether an offline clear is old.
        let expected = !known.is_empty();
        let all = self
            .attempt(
                key,
                false,
                |v: &Vec<_>| expected && v.is_empty(),
                |session| async move { hs.read_v1_bookmarks(&session).await },
            )
            .await?
            .unwrap_or_default();
        Ok(all.into_iter().map(|(_, uri, at)| (uri, at)).collect())
    }

    async fn put_bookmark(&self, key: &str, uri: &str, created_at: i64) -> Result<(), PersonalError> {
        let (object, path) = pm_pubky::v1::bookmark_of(uri, created_at).map_err(unbuildable)?;
        let hs = self.client()?;
        let (object, path) = (&object, path.as_str());
        self.attempt(
            key,
            false,
            |_| false,
            |session| async move { hs.write_object(&session, path, object).await },
        )
        .await
        .map(|_| ())
    }

    async fn delete_bookmark(&self, key: &str, uri: &str) -> Result<(), PersonalError> {
        let path = pm_pubky::v1::bookmark_path(uri).map_err(unbuildable)?;
        let hs = self.client()?;
        let path = path.as_str();
        // One that isn't there is no error
        self.attempt(
            key,
            true,
            |_| false,
            |session| async move { hs.delete(&session, path).await },
        )
        .await
        .map(|_| ())
    }
}

/// A bookmark the node can't even build (a target no homeserver would hold) is never taken: dropped
/// like a change their homeserver refuses, never waited on as if their homeserver were away.
fn unbuildable(why: String) -> PersonalError {
    PersonalError::Rejected(why)
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
    /// Bookmarks they cleared, with when (the newest [`CLEARED_KEPT`]): a heart made before one of
    /// these, sent late by another device, doesn't bring it back.
    #[serde(default)]
    cleared: Vec<Item>,
}

/// How many clears a listener's copy remembers.
const CLEARED_KEPT: usize = 500;

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

fn load(s: &NodeState, key: &str) -> Result<Copy, ApiError> {
    let lib = s.library.lock().expect("library poisoned");
    let Some(json) = lib.get_setting(&copy_key(key))? else {
        return Ok(Copy::default());
    };
    serde_json::from_str(&json).map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot read saved bookmarks: {e}"),
        )
    })
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

/// Membership changes and approval starts use a separate lock from homeserver operations.
/// Remove takes this before revoking access, then keeps it until cleanup ends. A slow operation
/// can delay cleanup, but cannot delay revocation or let a re-add overtake that cleanup.
pub(crate) fn membership_lock(key: &str) -> Arc<tokio::sync::Mutex<()>> {
    static L: OnceLock<PersonLocks> = OnceLock::new();
    L.get_or_init(Default::default)
        .lock()
        .expect("membership locks")
        .entry(key.trim_start_matches("pubky").to_string())
        .or_default()
        .clone()
}

fn save_guarded(
    s: &NodeState,
    key: &str,
    c: &Copy,
    started: u64,
    _roster: &tokio::sync::MutexGuard<'_, ()>,
) -> Result<(), ApiError> {
    // Restore and revocation hold the same roster guard. The authenticated generation check
    // and this synchronous write are one commit, even when music remains enabled by restore.
    require_access(s, key, started)?;
    let lib = s.library.lock().expect("library poisoned");
    lib.set_setting(&copy_key(key), &serde_json::to_string(c).expect("serializes"))?;
    Ok(())
}

async fn save(s: &NodeState, key: &str, c: &Copy, started: u64) -> Result<(), ApiError> {
    let roster = s.roster.lock().await;
    save_guarded(s, key, c, started, &roster)
}

impl Copy {
    fn merge_remote(&mut self, list: Vec<(String, i64)>) {
        // Absence still means deleted remotely. A returned older heart cannot erase a newer
        // known heart or undo a newer clear; the latter has no item to preserve here.
        self.items = list
            .into_iter()
            .filter_map(|(uri, at)| {
                if self.outdated(&uri, true, at) {
                    self.items.iter().find(|item| item.uri == uri).cloned()
                } else {
                    Some(Item { uri, at })
                }
            })
            .collect();
    }

    fn apply(&mut self, uri: &str, on: bool, at: i64) {
        self.items.retain(|i| i.uri != uri);
        self.cleared.retain(|i| i.uri != uri);
        let item = Item {
            uri: uri.to_string(),
            at,
        };
        if on {
            self.items.push(item);
        } else {
            self.cleared.push(item);
            if self.cleared.len() > CLEARED_KEPT {
                self.cleared.sort_by_key(|i| std::cmp::Reverse(i.at));
                self.cleared.truncate(CLEARED_KEPT);
            }
        }
    }

    /// Whether a change of `uri` made at `at` is older than the last one the node knows of it:
    /// one of the same kind made later (a late duplicate), or one of the other kind made a second
    /// or more later (clocks a little apart don't matter). A clear older than the bookmark, a
    /// bookmark older than the clear, or an older repeat, changes nothing.
    fn outdated(&self, uri: &str, on: bool, at: i64) -> bool {
        let last = |list: &[Item]| list.iter().filter(|i| i.uri == uri).map(|i| i.at).max();
        let (same, other) = if on {
            (&self.items, &self.cleared)
        } else {
            (&self.cleared, &self.items)
        };
        last(same).is_some_and(|l| l > at) || last(other).is_some_and(|l| l >= at.saturating_add(1_000_000))
    }

    fn bookmarked(&self, uri: &str) -> bool {
        self.items.iter().any(|i| i.uri == uri)
    }
}

/// The answer a listener gets when the node can't write for them: connect (again) first.
fn needs_connect(lost: bool) -> Response {
    let error = if lost {
        "Your homeserver no longer lets this node in: approve it again (Settings › You › My homeserver)"
    } else {
        "Connect your homeserver first (Settings › You › My homeserver): your bookmarks are kept there"
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
    let started = request_started(s);
    let b = backend(s);
    let lock = person_lock(key);
    let _one = lock.lock().await;
    require_access(s, key, started)?;
    let connected = b.connected(key);
    // Not connected (or disconnected): theirs aren't known here; the copy waits for a reconnect
    if !connected {
        return Ok((Vec::new(), PersonalState::default()));
    }
    let mut c = load(s, key)?;
    let mut stale = false;
    if !c.lost {
        while let Some(ch) = c.pending.first().cloned() {
            require_access(s, key, started)?;
            let r = if ch.on {
                b.put_bookmark(key, &ch.uri, ch.at).await
            } else {
                b.delete_bookmark(key, &ch.uri).await
            };
            require_access(s, key, started)?;
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
            require_access(s, key, started)?;
            let known: Vec<(String, i64)> = c.items.iter().map(|i| (i.uri.clone(), i.at)).collect();
            let listed = b.bookmarks(key, &known).await;
            require_access(s, key, started)?;
            match listed {
                Ok(list) => {
                    c.merge_remote(list);
                }
                Err(PersonalError::Refused(m)) => {
                    tracing::info!(%key, "a listener's homeserver refused the node: {m}");
                    c.lost = true;
                }
                Err(_) => stale = true,
            }
        }
        save(s, key, &c, started).await?;
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
/// seconds) is when a device made the change offline: a bookmark keeps that time; a clear older
/// than the bookmark the node knows leaves it (it was bookmarked again since), and a bookmark
/// older than the clear the node knows doesn't bring it back.
pub(crate) async fn set_bookmark(
    s: &NodeState,
    key: &str,
    uri: String,
    on: bool,
    at: Option<i64>,
) -> Result<Response, ApiError> {
    let started = request_started(s);
    let b = backend(s);
    let lock = person_lock(key);
    let _one = lock.lock().await;
    require_access(s, key, started)?;
    if !b.connected(key) {
        return Ok(needs_connect(false));
    }
    let mut c = load(s, key)?;
    if c.lost {
        return Ok(needs_connect(true));
    }
    let now = pm_pubky::ids::now_micros();
    let at = at.map_or(now, |a| a.clamp(0, now / 1_000_000).saturating_mul(1_000_000));
    // Made before the last change of it (another device's, or a repeat, sent late): that one
    // stands, and neither their homeserver nor what waits for it is touched
    if c.outdated(&uri, on, at) {
        let now_on = c.bookmarked(&uri);
        return Ok(Json(BookmarkChanged::new(uri, now_on)).into_response());
    }
    require_access(s, key, started)?;
    let r = if on {
        b.put_bookmark(key, &uri, at).await
    } else {
        b.delete_bookmark(key, &uri).await
    };
    require_access(s, key, started)?;
    let pending = match r {
        // Written: whatever still waited for this one is older and goes
        Ok(()) => {
            c.pending.retain(|p| p.uri != uri);
            false
        }
        Err(PersonalError::NoGrant) => return Ok(needs_connect(false)),
        // For good: a 4xx, so a device's offline queue drops it rather than sending it forever
        Err(PersonalError::Rejected(m)) => {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("Your homeserver won't keep this bookmark: {m}"),
            ))
        }
        Err(PersonalError::Refused(m)) => {
            tracing::info!(%key, "a listener's homeserver refused the node: {m}");
            c.lost = true;
            save(s, key, &c, started).await?;
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
    save(s, key, &c, started).await?;
    let mut answer = BookmarkChanged::new(uri, on);
    answer.pending = pending;
    Ok(Json(answer).into_response())
}

// ---- connecting a listener's homeserver ------------------------------------------------------

/// Whether a Ring approval may become `key`'s grant: approved by them, for every folder, and for
/// nothing beyond those folders (the node never holds a key to the rest of their homeserver).
pub(crate) fn may_keep(key: &str, approver: &str, granted: &[String]) -> Result<(), &'static str> {
    if approver.trim_start_matches("pubky") != key.trim_start_matches("pubky") {
        return Err("approved by another identity than yours");
    }
    if !pm_pubky::homeserver::missing_capabilities(granted, PERSONAL_CAPABILITIES).is_empty() {
        return Err("not approved for your bookmarks folder");
    }
    if granted.iter().any(|g| !within(g, PERSONAL_CAPABILITIES)) {
        return Err("approved for more than your bookmarks folder: approve only what the node asks for");
    }
    Ok(())
}

/// Whether the granted `scope:actions` stays inside one of `allowed` (a list like
/// [`PERSONAL_CAPABILITIES`]): the same folder or one in it, with none of its actions beyond.
fn within(granted: &str, allowed: &str) -> bool {
    let Some((scope, actions)) = granted.trim().rsplit_once(':') else {
        return false;
    };
    // A `..` or `.` segment could step out of the folder it seems to be in
    !scope.is_empty()
        && !scope.split('/').any(|s| s == ".." || s == ".")
        && allowed
            .split(',')
            .filter_map(|a| a.trim().rsplit_once(':'))
            .any(|(a_scope, a_actions)| {
                let inside = scope == a_scope || (a_scope.ends_with('/') && scope.starts_with(a_scope));
                inside && actions.chars().all(|c| a_actions.contains(c))
            })
}

/// How long one relay poll or grant exchange may take before the approval is given up.
const EXCHANGE_LIMIT: Duration = Duration::from_secs(120);
/// How often a waiting approval asks the relay again.
const ASK_EVERY: Duration = Duration::from_secs(1);

/// A Ring approval in progress. A task of its own asks the relay, exchanges an approval with the
/// listener's homeserver and keeps the grant; a poll only reads where it stands. A request that
/// gives up or goes away can't stop that half way (the approval would be lost).
struct Connect {
    started: Instant,
    /// The listener who started it: only they may read its answer.
    who: String,
    /// The authenticated request's access generation, retained across the relay task.
    revision: u64,
    /// `None` while it waits; then `{state: ok|denied|expired|error, reason?}`.
    answer: Option<serde_json::Value>,
}

/// Ring approvals in progress, by id.
type Connects = tokio::sync::Mutex<HashMap<String, Connect>>;

fn connects() -> &'static Connects {
    static C: OnceLock<Connects> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// Whether approval `id` is still asked for: not replaced, expired away or ended by a Disconnect.
async fn still_asked(id: &str) -> bool {
    connects().lock().await.contains_key(id)
}

/// Under `key`'s operation lock and the roster lock, `keep` runs only while the approval and
/// its authenticated access generation still stand. Disconnect serializes on the operation
/// lock; removal and restore cannot cross the final check and synchronous grant write.
async fn keep_if_current(
    s: &NodeState,
    id: &str,
    key: &str,
    keep: impl FnOnce() -> serde_json::Value,
) -> serde_json::Value {
    let lock = person_lock(key);
    let _one = lock.lock().await;
    // Restore/revocation cannot cross the final generation check and synchronous grant write.
    let roster = s.roster.lock().await;
    let revision = connects()
        .lock()
        .await
        .get(id)
        .filter(|c| c.who == key)
        .map(|c| c.revision);
    let Some(revision) = revision else {
        return serde_json::json!({"state": "expired"});
    };
    if let Err(e) = require_access(s, key, revision) {
        return serde_json::json!({"state":"denied", "reason":e.message});
    }
    // Read before replacing the grant: an unreadable copy is not an empty one, and cannot
    // lose its pending changes or tombstones merely because the listener approves again.
    let mut copy = match load(s, key) {
        Ok(copy) => copy,
        Err(e) => return serde_json::json!({"state":"error", "reason":e.message}),
    };
    let answer = keep();
    if answer["state"] == "ok" {
        copy.lost = false;
        if let Err(e) = save_guarded(s, key, &copy, revision, &roster) {
            return serde_json::json!({"state":"error", "reason":format!("approval kept, but bookmarks could not be updated: {}", e.message)});
        }
    }
    answer
}

/// Ends every approval `key` has in progress (Disconnect, People › Remove): none of them can
/// keep a grant after. Called under their person lock, which [`keep_grant`] holds while it
/// checks and writes.
async fn end_approvals_of(key: &str) {
    let key = key.trim_start_matches("pubky");
    connects().lock().await.retain(|_, c| c.who != key);
}

/// The approval `id`'s task: asks the relay until Ring approves (`poll`), then hands the session
/// to `keep`. It ends early when the approval is replaced or gone, and gives up after `ttl`.
async fn ask<S, P, PF, K, KF>(id: String, ttl: Duration, mut poll: P, keep: K)
where
    P: FnMut() -> PF,
    PF: std::future::Future<Output = Result<Option<S>, HomeserverError>>,
    K: FnOnce(S) -> KF,
    KF: std::future::Future<Output = serde_json::Value>,
{
    let started = Instant::now();
    let answer = loop {
        // Replaced by a newer one of theirs, or gone: nobody waits for this one
        if !connects().lock().await.contains_key(&id) {
            return;
        }
        if started.elapsed() > ttl {
            break serde_json::json!({"state": "expired"});
        }
        match tokio::time::timeout(EXCHANGE_LIMIT, poll()).await {
            Ok(Ok(None)) => tokio::time::sleep(ASK_EVERY).await,
            Ok(Ok(Some(session))) => {
                // Approved after it expired, or after it was replaced: nothing is kept (the page
                // has moved on to a new one)
                if started.elapsed() > ttl {
                    break serde_json::json!({"state": "expired"});
                }
                if !connects().lock().await.contains_key(&id) {
                    return;
                }
                break keep(session).await;
            }
            Ok(Err(e)) => break serde_json::json!({"state": "error", "reason": e.to_string()}),
            Err(_) => break serde_json::json!({"state": "error", "reason": "the homeserver took too long"}),
        }
    };
    if let Some(c) = connects().lock().await.get_mut(&id) {
        c.answer = Some(answer);
    }
}

/// Keeps an approved session as `key`'s grant, when it may be theirs and approval `id` is still
/// asked for.
async fn keep_grant(s: Arc<NodeState>, id: String, key: String, session: PubkySession) -> serde_json::Value {
    let approver = session.public_key().to_string();
    let granted = pm_pubky::homeserver::session_capabilities(&session);
    if let Err(why) = may_keep(&key, &approver, &granted) {
        tracing::warn!(%key, %approver, "a listener's homeserver grant was refused: {why}");
        return serde_json::json!({"state": "denied", "reason": why});
    }
    let Some(secret) = pm_pubky::homeserver::export_session_secret(&session).await else {
        return serde_json::json!({"state": "error", "reason": "Ring approved, but the session can't be kept: try again"});
    };
    // They disconnected (or started another approval) while this one was exchanged: nothing is
    // kept, so a Disconnect that has answered stays done
    keep_if_current(&s, &id, &key, || {
        let Some(file) = grant_file(&grants_dir(&s), &key) else {
            return serde_json::json!({"state": "denied", "reason": "not a Pubky key"});
        };
        // Replaced in one step (a new file renamed over the old): a write that fails keeps the
        // grant they had
        if let Err(e) = crate::imports::write_private(&file, secret.as_bytes()) {
            return serde_json::json!({"state": "error", "reason": format!("cannot keep the grant: {e}")});
        }
        sessions().lock().expect("sessions").insert(key.clone(), session);
        tracing::info!(%key, "a listener connected their homeserver");
        serde_json::json!({"state": "ok"})
    })
    .await
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
    let copy = load(&s, &key)?;
    Ok(Json(PersonalState {
        connected,
        lost: connected && copy.lost,
        pending: copy.pending.len(),
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
    let started = request_started(&s);
    let lock = person_lock(&key);
    let _one = lock.lock().await;
    require_access(&s, &key, started)?;
    // An approval still on its way would bring the grant back after this answer
    end_approvals_of(&key).await;
    // Still there (it couldn't be deleted): not answered as disconnected
    backend(&s)
        .disconnect(&key)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let copy = load(&s, &key)?;
    Ok(Json(PersonalState {
        connected: false,
        lost: false,
        pending: copy.pending.len(),
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
    begin_approval(&s, &id, &key).await?;
    let flow = Arc::new(flow);
    tokio::spawn(ask(
        id.clone(),
        CONNECT_TTL,
        move || {
            let flow = flow.clone();
            async move { flow.poll().await }
        },
        {
            let id = id.clone();
            move |session| keep_grant(s, id, key, session)
        },
    ));
    Ok(Json(
        serde_json::json!({"id": id, "qr_svg": crate::qr_svg(&url), "url": url}),
    ))
}

async fn begin_approval(s: &NodeState, id: &str, key: &str) -> Result<(), ApiError> {
    let started = request_started(s);
    let lock = membership_lock(key);
    let _membership = lock.lock().await;
    // A request admitted before Remove must not inherit the membership of a later re-add.
    require_access(s, key, started)?;
    let mut all = connects().lock().await;
    all.retain(|_, c| c.started.elapsed() < CONNECT_TTL && c.who != key);
    all.insert(
        id.to_string(),
        Connect {
            started: Instant::now(),
            who: key.to_string(),
            revision: started,
            answer: None,
        },
    );
    Ok(())
}

/// `GET /v1/me/homeserver/connect/poll?id=`: `pending`, `ok` (the grant is kept), `denied`
/// (approved by someone else, or not for every folder: nothing kept) or `expired`; 502 when the
/// relay or their homeserver failed.
pub(crate) async fn connect_poll(
    axum::extract::Extension(c): axum::extract::Extension<crate::accounts::Caller>,
    axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let key = listener_key(&c)?;
    let id = q.get("id").cloned().unwrap_or_default();
    answer_of(&key, &id, CONNECT_TTL).await.map(Json)
}

/// Where approval `id` of `key` stands, for a poll; one older than `ttl` is gone.
async fn answer_of(key: &str, id: &str, ttl: Duration) -> Result<serde_json::Value, ApiError> {
    let mut all = connects().lock().await;
    // Only the listener who started it may read it
    let Some(c) = all.get(id).filter(|c| c.who == key) else {
        return Ok(serde_json::json!({"state": "expired"}));
    };
    // Past its time, answered or not: gone
    if c.started.elapsed() > ttl {
        all.remove(id);
        return Ok(serde_json::json!({"state": "expired"}));
    }
    match &c.answer {
        None => Ok(serde_json::json!({"state": "pending"})),
        // A failure is answered once; then it is gone, so the page's next poll starts afresh
        Some(a) if a["state"] == "error" => {
            let why = a["reason"]
                .as_str()
                .unwrap_or("Ring's approval failed")
                .to_string();
            all.remove(id);
            Err(ApiError::new(StatusCode::BAD_GATEWAY, why))
        }
        // Kept until it expires: another tab of theirs reads the same answer
        Some(a) => Ok(a.clone()),
    }
}

/// People › Remove: the grant of `key` goes. Called under their person lock (see
/// [`crate::accounts::forget_person`]). Remove already revoked access under the separate
/// membership lock, which it keeps through cleanup: an approval past the relay either kept its
/// grant before (and it is deleted here) or finds itself ended. A re-add and a fresh approval
/// start both wait for that cleanup, even when the owner's request was cancelled.
/// An error says the grant is still on the node (it couldn't be deleted): the removal answers it
/// after the rest of its cleanup, and removing them again finishes it ([`leftover`]).
pub(crate) async fn forget(s: &NodeState, key: &str) -> Result<(), String> {
    forget_with(backend(s).as_ref(), &grants_dir(s), key).await
}

/// [`forget`] with the store and the grants folder given. The store forgets them, and the grant
/// file in `dir` goes too (the node's own store is that file; one standing in for it isn't).
async fn forget_with(b: &dyn PersonalHomeserver, dir: &std::path::Path, key: &str) -> Result<(), String> {
    let key = key.trim_start_matches("pubky");
    sessions().lock().expect("sessions").remove(key);
    let file = match grant_file(dir, key).map(std::fs::remove_file) {
        Some(Err(e)) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    };
    end_approvals_of(key).await;
    b.disconnect(key).await.and(file).map_err(|e| {
        tracing::warn!(%key, "a removed listener's grant is still on the node: {e}");
        format!("their homeserver approval is still on the node ({e}): remove them again")
    })
}

/// Whether anything of `key`'s is still on the node: their grant, their sessions, or their copies
/// (a removal that failed part way). Read uncertainty blocks re-add; Remove still attempts
/// cleanup and reports it. Removing them again finishes what remains.
pub(crate) fn leftover(s: &NodeState, key: &str) -> Result<bool, ApiError> {
    let key = key.trim_start_matches("pubky");
    if backend(s).connected(key)
        || match grant_file(&grants_dir(s), key) {
            Some(file) => file.try_exists().map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("cannot check retained approval: {e}"),
                )
            })?,
            None => false,
        }
        || crate::accounts::sessions_of_checked(s, key)? > 0
    {
        return Ok(true);
    }
    let lib = s.library.lock().expect("library poisoned");
    for k in [
        crate::prefs::storage_key_of(key),
        crate::presence::visible_key(key),
    ] {
        if lib.get_setting(&k)?.is_some() {
            return Ok(true);
        }
    }
    for prefix in [
        format!("{}{key}.", crate::accounts::PERSON_PREFIX),
        crate::discover::contents_prefix_of(key),
        crate::discover::contents_prefix_of(&crate::accounts::Who::ListenerOnNode(key.to_string()).tag()),
    ] {
        if !lib.settings_with_prefix(&prefix)?.is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
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
        assert!(may_keep(k, k, &[]).is_err(), "nothing approved is not enough");
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

    async fn waiting(id: &str, who: &str) {
        connects().lock().await.insert(
            id.into(),
            Connect {
                started: Instant::now(),
                who: who.into(),
                revision: 0,
                answer: None,
            },
        );
    }

    async fn poll_as(who: &str, id: &str) -> Result<serde_json::Value, ApiError> {
        let caller = crate::accounts::Caller {
            role: crate::accounts::Role::Listener,
            key: Some(who.into()),
        };
        let q = HashMap::from([("id".to_string(), id.to_string())]);
        connect_poll(axum::Extension(caller), axum::extract::Query(q))
            .await
            .map(|j| j.0)
    }

    #[tokio::test]
    async fn an_approval_is_kept_by_its_own_task_whatever_the_polls_do() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        waiting("t-kept", "ann").await;
        let (asked, kept) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (a, k) = (asked.clone(), kept.clone());
        let task = tokio::spawn(ask(
            "t-kept".into(),
            Duration::from_secs(60),
            move || {
                let n = a.fetch_add(1, Ordering::SeqCst);
                async move { Ok::<_, HomeserverError>((n >= 1).then_some(())) }
            },
            move |()| async move {
                k.fetch_add(1, Ordering::SeqCst);
                serde_json::json!({"state": "ok"})
            },
        ));
        assert_eq!(poll_as("ann", "t-kept").await.unwrap()["state"], "pending");
        // Nobody asks while it works (the page went away): it finishes all the same
        task.await.unwrap();
        assert_eq!(
            (asked.load(Ordering::SeqCst), kept.load(Ordering::SeqCst)),
            (2, 1)
        );
        // Each tab of hers reads the answer; anyone else reads nothing
        assert_eq!(poll_as("ann", "t-kept").await.unwrap()["state"], "ok");
        assert_eq!(poll_as("ann", "t-kept").await.unwrap()["state"], "ok");
        assert_eq!(poll_as("bo", "t-kept").await.unwrap()["state"], "expired");
        assert_eq!(poll_as("ann", "t-nothing").await.unwrap()["state"], "expired");
    }

    #[tokio::test]
    async fn a_replaced_failed_or_late_approval_keeps_nothing() {
        // Replaced (a new one of hers) or gone: the task ends, nothing is kept
        waiting("t-gone", "ann").await;
        let task = tokio::spawn(ask(
            "t-gone".into(),
            Duration::from_secs(60),
            || async { Ok::<Option<()>, HomeserverError>(None) },
            |()| async { panic!("nothing is kept for a replaced approval") },
        ));
        tokio::time::sleep(Duration::from_millis(50)).await;
        connects().lock().await.remove("t-gone");
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the task ends")
            .unwrap();
        // The relay or the exchange failed: a 502 for her, nothing kept
        waiting("t-failed", "ann").await;
        ask(
            "t-failed".into(),
            Duration::from_secs(60),
            || async { Err::<Option<()>, _>(HomeserverError::Transport("relay down".into())) },
            |()| async { panic!("nothing is kept for a failed approval") },
        )
        .await;
        let e = poll_as("ann", "t-failed").await.unwrap_err();
        assert_eq!(e.status, StatusCode::BAD_GATEWAY, "{}", e.message);
        assert_eq!(
            poll_as("ann", "t-failed").await.unwrap()["state"],
            "expired",
            "then the page starts afresh"
        );
        // Too late: expired
        waiting("t-late", "ann").await;
        ask(
            "t-late".into(),
            Duration::ZERO,
            || async { Ok::<Option<()>, HomeserverError>(None) },
            |()| async { panic!("nothing is kept for an expired approval") },
        )
        .await;
        assert_eq!(poll_as("ann", "t-late").await.unwrap()["state"], "expired");
        // Approved while it was being replaced, or after it expired: nothing kept either
        let approved_slowly = || async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            Ok::<_, HomeserverError>(Some(()))
        };
        waiting("t-swapped", "ann").await;
        let task = tokio::spawn(ask(
            "t-swapped".into(),
            Duration::from_secs(60),
            approved_slowly,
            |()| async { panic!("nothing is kept for a replaced approval") },
        ));
        tokio::time::sleep(Duration::from_millis(50)).await;
        connects().lock().await.remove("t-swapped");
        task.await.unwrap();
        waiting("t-overdue", "ann").await;
        ask(
            "t-overdue".into(),
            Duration::from_millis(50),
            approved_slowly,
            |()| async { panic!("nothing is kept after it expired") },
        )
        .await;
        assert_eq!(poll_as("ann", "t-overdue").await.unwrap()["state"], "expired");
    }

    #[test]
    fn a_refusal_that_may_be_the_grants_is_asked_again_on_a_fresh_session() {
        use pubky::errors::{AuthError, Error as Pe, RequestError};
        let server = |c: u16| {
            HomeserverError::Pubky(Pe::Request(RequestError::Server {
                status: reqwest::StatusCode::from_u16(c).unwrap(),
                message: String::new(),
            }))
        };
        for c in [400, 401, 403, 404, 422] {
            assert!(grant_like(&server(c)), "{c}");
        }
        for c in [408, 429, 500, 503] {
            assert!(!grant_like(&server(c)), "{c}");
        }
        assert!(grant_like(&HomeserverError::Pubky(Pe::Authentication(
            AuthError::Validation("expired".into())
        ))));
        assert!(!grant_like(&HomeserverError::Timeout));
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

    #[test]
    fn a_grant_wider_than_the_bookmarks_folder_is_never_kept() {
        let k = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";
        let all: Vec<String> = PERSONAL_CAPABILITIES.split(',').map(String::from).collect();
        let with = |extra: &[&str]| -> Vec<String> {
            all.iter()
                .cloned()
                .chain(extra.iter().map(|x| x.to_string()))
                .collect()
        };
        // Each covers the bookmarks folder, and more: the node would hold a key to the rest
        for (granted, what) in [
            (vec!["/:rw".to_string()], "the whole homeserver"),
            (
                vec![
                    "/priv/social/v1/:rw".to_string(),
                    "/pub/social/v1/posts/:rw".to_string(),
                ],
                "every private social folder",
            ),
            (with(&["/priv/pubky.music/vault/:rw"]), "their sources vault"),
            (
                with(&["/priv/social/v1/posts/:rw"]),
                "their posts (nothing here writes them)",
            ),
            (with(&["/pub/pubky.app/:r"]), "an unrelated folder, read only"),
            (with(&["/priv/social/v1/:r"]), "its parent folder, read only"),
            (with(&["garbage"]), "something unreadable"),
            (
                with(&["/priv/social/v1/bookmarks/../../pubky.music/vault/:rw"]),
                "a folder that steps out of its own",
            ),
        ] {
            assert!(may_keep(k, k, &granted).is_err(), "{what}: {granted:?}");
        }
        // Exactly what it asks for, in any order, or narrower within it, is theirs to give
        let mut back = all.clone();
        back.reverse();
        assert!(may_keep(k, k, &back).is_ok());
        assert!(may_keep(k, k, &with(&["/priv/social/v1/bookmarks/abc/:r"])).is_ok());
    }

    #[tokio::test]
    async fn a_disconnect_ends_the_listeners_approvals_and_nothing_is_kept_after() {
        waiting("t-dis-ann", "dis-ann").await;
        waiting("t-dis-bo", "dis-bo").await;
        let approved_slowly = || async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            Ok::<_, HomeserverError>(Some(()))
        };
        let task = tokio::spawn(ask(
            "t-dis-ann".into(),
            Duration::from_secs(60),
            approved_slowly,
            |()| async { panic!("nothing is kept once she disconnected") },
        ));
        tokio::time::sleep(Duration::from_millis(50)).await;
        end_approvals_of("dis-ann").await;
        task.await.unwrap();
        assert_eq!(poll_as("dis-ann", "t-dis-ann").await.unwrap()["state"], "expired");
        assert!(still_asked("t-dis-bo").await, "another listener's approval stays");
        // An approval already past the relay checks it is still asked for, under her lock
        waiting("t-dis-late", "dis-ann").await;
        assert!(still_asked("t-dis-late").await);
        end_approvals_of("dis-ann").await;
        assert!(!still_asked("t-dis-late").await);
    }

    #[tokio::test]
    async fn a_grant_is_kept_only_while_its_approval_is_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let s = listener_state(dir.path(), "keep-ann").await;
        waiting("t-keep", "keep-ann").await;
        let ok = || serde_json::json!({"state": "ok"});
        assert_eq!(keep_if_current(&s, "t-keep", "keep-ann", ok).await["state"], "ok");
        // A Disconnect holds her lock while it ends her approvals: an approval past the relay that
        // waits on that lock keeps nothing after
        waiting("t-keep-late", "keep-ann").await;
        let lock = person_lock("keep-ann");
        let disconnecting = lock.lock().await;
        let keeping = keep_if_current(&s, "t-keep-late", "keep-ann", || -> serde_json::Value {
            panic!("a grant kept after her Disconnect")
        });
        tokio::pin!(keeping);
        assert!(futures_util::poll!(&mut keeping).is_pending());
        end_approvals_of("keep-ann").await;
        drop(disconnecting);
        assert_eq!(keeping.await["state"], "expired");
    }

    #[tokio::test]
    async fn a_bookmark_the_node_cant_build_is_dropped_not_waited_on() {
        // The node's own adapter (no client needed: it builds first). Longer than any reference
        // a bookmark may hold (1,024 code points): no homeserver could ever take it
        let dir = tempfile::tempdir().unwrap();
        let g = Grants {
            dir: dir.path().to_path_buf(),
            hs: None,
        };
        let long = format!("https://example.com/{}", "a".repeat(20_000));
        assert!(matches!(
            g.put_bookmark("ann", &long, 0).await,
            Err(PersonalError::Rejected(_))
        ));
        assert!(matches!(
            g.delete_bookmark("ann", &long).await,
            Err(PersonalError::Rejected(_))
        ));
    }

    #[test]
    fn a_kept_grant_wider_than_what_the_node_asks_for_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let g = Grants {
            dir: dir.path().to_path_buf(),
            hs: None,
        };
        let k = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";
        let file = grant_file(dir.path(), k).unwrap();
        crate::imports::write_private(&file, b"fixture-secret").unwrap();
        let exact: Vec<String> = PERSONAL_CAPABILITIES.split(',').map(String::from).collect();
        assert!(g.may_hold(k, k, &exact).is_ok());
        assert!(file.exists(), "one that fits stays");
        // Kept by an earlier build that also asked for their posts
        let wider = [exact, vec!["/priv/social/v1/posts/:rw".to_string()]].concat();
        assert!(matches!(g.may_hold(k, k, &wider), Err(PersonalError::Refused(_))));
        assert!(!file.exists(), "removed: the node holds no wider key");
        assert!(!g.connected(k), "they connect again");
    }

    #[tokio::test]
    async fn an_answered_approval_expires_with_its_time() {
        connects().lock().await.insert(
            "t-old-ok".into(),
            Connect {
                started: Instant::now(),
                who: "old-ann".into(),
                revision: 0,
                answer: Some(serde_json::json!({"state": "ok"})),
            },
        );
        assert_eq!(
            poll_as("old-ann", "t-old-ok").await.unwrap()["state"],
            "ok",
            "within its time"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            answer_of("old-ann", "t-old-ok", Duration::from_millis(10))
                .await
                .unwrap()["state"],
            "expired",
            "answered, and past its time"
        );
        assert!(!still_asked("t-old-ok").await, "and it is gone");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_grant_that_cant_be_deleted_isnt_forgotten() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let grants = dir.path().join("grants");
        let g = Grants {
            dir: grants.clone(),
            hs: None,
        };
        let k = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";
        let file = grant_file(&grants, k).unwrap();
        crate::imports::write_private(&file, b"fixture-secret").unwrap();
        std::fs::set_permissions(&grants, std::fs::Permissions::from_mode(0o500)).unwrap();
        let r = g.disconnect(k).await;
        std::fs::set_permissions(&grants, std::fs::Permissions::from_mode(0o700)).unwrap();
        // (As root the folder can't refuse: nothing to show there)
        if file.exists() {
            assert!(r.is_err(), "still there: an error, not a disconnect");
            assert!(g.connected(k));
        }
        assert!(g.disconnect(k).await.is_ok());
        assert!(!g.connected(k));
    }

    /// A store that only says whether it was told to forget.
    #[derive(Default)]
    struct Forgets(std::sync::atomic::AtomicBool, std::sync::atomic::AtomicBool);

    #[async_trait::async_trait]
    impl PersonalHomeserver for Forgets {
        fn connected(&self, _key: &str) -> bool {
            !self.0.load(std::sync::atomic::Ordering::SeqCst)
        }
        async fn disconnect(&self, _key: &str) -> Result<(), String> {
            if self.1.load(std::sync::atomic::Ordering::SeqCst) {
                return Err("fixture: cannot delete grant".into());
            }
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        async fn bookmarks(
            &self,
            _key: &str,
            _known: &[(String, i64)],
        ) -> Result<Vec<(String, i64)>, PersonalError> {
            Err(PersonalError::NoGrant)
        }
        async fn put_bookmark(&self, _key: &str, _uri: &str, _at: i64) -> Result<(), PersonalError> {
            Err(PersonalError::NoGrant)
        }
        async fn delete_bookmark(&self, _key: &str, _uri: &str) -> Result<(), PersonalError> {
            Err(PersonalError::NoGrant)
        }
    }

    #[tokio::test]
    async fn an_approval_waiting_behind_a_removal_keeps_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let s = listener_state(dir.path(), "rm-ann").await;
        waiting("t-rm-old", "rm-ann").await;
        // People › Remove holds her lock: an approval past the relay waits for it
        let lock = person_lock("rm-ann");
        let removal = lock.lock().await;
        let keeping = keep_if_current(&s, "t-rm-old", "rm-ann", || -> serde_json::Value {
            panic!("a grant kept after her removal")
        });
        tokio::pin!(keeping);
        assert!(futures_util::poll!(&mut keeping).is_pending());
        let store = Forgets::default();
        forget_with(&store, dir.path(), "rm-ann").await.unwrap();
        drop(removal);
        assert_eq!(keeping.await["state"], "expired");
        assert!(!store.connected("rm-ann"), "and the store forgot her");
    }

    async fn listener_state(dir: &std::path::Path, key: &str) -> Arc<NodeState> {
        let s = crate::tests::state(dir, Some("fixture-owner")).await;
        s.library
            .lock()
            .unwrap()
            .set_setting(
                "people",
                &serde_json::json!({"people": [
                    {"key": key, "access": "music", "music_enabled": true, "added_at": 1}
                ]})
                .to_string(),
            )
            .unwrap();
        s.set_personal_homeserver(Arc::new(Forgets::default()));
        crate::accounts::mint(&s, key).unwrap();
        s
    }

    #[tokio::test]
    async fn a_copy_commit_rechecks_its_revision_after_waiting_for_restore() {
        let dir = tempfile::tempdir().unwrap();
        let key = "b".repeat(52);
        let s = listener_state(dir.path(), &key).await;
        let started = crate::people::access_started(&s);
        let old = Copy::default();
        let roster = s.roster.lock().await;
        let committing = save(&s, &key, &old, started);
        tokio::pin!(committing);
        assert!(futures_util::poll!(&mut committing).is_pending());
        crate::people::restored_access(&s);
        let restored = r#"{"items":[],"pending":[],"lost":true,"cleared":[]}"#;
        s.library
            .lock()
            .unwrap()
            .set_setting(&copy_key(&key), restored)
            .unwrap();
        drop(roster);
        assert_eq!(committing.await.unwrap_err().status, StatusCode::CONFLICT);
        assert_eq!(
            s.library
                .lock()
                .unwrap()
                .get_setting(&copy_key(&key))
                .unwrap()
                .as_deref(),
            Some(restored)
        );
    }

    #[tokio::test]
    async fn authentication_passes_its_revision_through_a_delayed_handler() {
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let dir = tempfile::tempdir().unwrap();
        let key = "c".repeat(52);
        let s = listener_state(dir.path(), &key).await;
        let token = crate::accounts::mint(&s, &key).unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let resume = Arc::new(tokio::sync::Notify::new());
        let app = axum::Router::new()
            .route(
                "/bookmarks",
                axum::routing::put({
                    let (s, key, entered, resume) = (s.clone(), key.clone(), entered.clone(), resume.clone());
                    move || {
                        let (s, key, entered, resume) =
                            (s.clone(), key.clone(), entered.clone(), resume.clone());
                        async move {
                            entered.notify_one();
                            resume.notified().await;
                            set_bookmark(&s, &key, "https://example.com/song".into(), true, None).await
                        }
                    }
                }),
            )
            .route_layer(axum::middleware::from_fn_with_state(s.clone(), crate::auth));
        let request = axum::http::Request::builder()
            .method("PUT")
            .uri("/bookmarks")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let response = tokio::spawn(async move { app.oneshot(request).await.unwrap() });
        tokio::time::timeout(Duration::from_secs(10), entered.notified())
            .await
            .unwrap();
        let roster = s.roster.lock().await;
        crate::people::restored_access(&s);
        drop(roster);
        resume.notify_one();
        let response = response.await.unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert!(
            body["error"].as_str().unwrap().contains("music access changed"),
            "{body}"
        );
        assert!(
            body.get("needs_connect").is_none(),
            "the stale handler must stop before using its grant"
        );
    }

    #[tokio::test]
    async fn approval_reset_preserves_an_unreadable_copy_and_its_existing_grant() {
        for malformed in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let key = if malformed { "d".repeat(52) } else { "f".repeat(52) };
            let s = listener_state(dir.path(), &key).await;
            let original = serde_json::json!({
                "items":[], "lost":true,
                "pending":[{"uri":"https://example.com/new","on":true,"at":300_000_000}],
                "cleared":[{"uri":"https://example.com/old","at":200_000_000}],
            })
            .to_string();
            let setting = copy_key(&key);
            s.library
                .lock()
                .unwrap()
                .set_setting(&setting, &original)
                .unwrap();
            let file = grant_file(&grants_dir(&s), &key).unwrap();
            crate::imports::write_private(&file, b"existing-grant").unwrap();
            let db = rusqlite::Connection::open(&s.db_path).unwrap();
            if malformed {
                s.library
                    .lock()
                    .unwrap()
                    .set_setting(&setting, &format!("{original} broken"))
                    .unwrap();
            } else {
                db.execute(
                    "UPDATE settings SET value=CAST(value AS BLOB) WHERE key=?1",
                    [&setting],
                )
                .unwrap();
            }
            let raw = || -> Vec<u8> {
                db.query_row(
                    "SELECT CAST(value AS BLOB) FROM settings WHERE key=?1",
                    [&setting],
                    |r| r.get(0),
                )
                .unwrap()
            };
            let before = raw();
            let id = format!("copy-read-{malformed}");
            begin_approval(&s, &id, &key).await.unwrap();
            let answer = keep_if_current(&s, &id, &key, || {
                panic!("approval must not replace a grant before its existing copy is readable")
            })
            .await;
            assert_eq!(answer["state"], "error", "{answer}");
            assert_eq!(raw(), before);
            assert_eq!(std::fs::read(&file).unwrap(), b"existing-grant");
            s.library
                .lock()
                .unwrap()
                .set_setting(&setting, &original)
                .unwrap();
            let answer = keep_if_current(&s, &id, &key, || {
                crate::imports::write_private(&file, b"new-grant").unwrap();
                serde_json::json!({"state":"ok"})
            })
            .await;
            assert_eq!(answer["state"], "ok", "{answer}");
            let mut expected: serde_json::Value = serde_json::from_str(&original).unwrap();
            expected["lost"] = false.into();
            assert_eq!(serde_json::to_value(load(&s, &key).unwrap()).unwrap(), expected);
            end_approvals_of(&key).await;
        }
    }

    /// Exercise Grants and the SDK's real listing/object fetches over isolated HTTPS. Cached
    /// pkarr records point only at this loopback fixture; no testnet or real account is used.
    #[tokio::test]
    #[allow(deprecated)] // A local legacy cookie session avoids a remote grant exchange.
    async fn the_grants_adapter_refreshes_a_known_hearts_timestamp_before_an_offline_clear() {
        // Pubky's builder cannot disable reqwest's inherited proxies. Run only this test
        // in a fresh process with a proxy-free transport environment, never mutating the
        // process-wide environment of the concurrently running parent suite. The first
        // child deliberately inherits unusable loopback proxies, so the next boundary
        // must remove both cases rather than relying on the developer/CI configuration.
        const CHILD: &str = "ODEION_BOOKMARK_SDK_TEST_PROCESS";
        const TEST: &str =
            "personal::tests::the_grants_adapter_refreshes_a_known_hearts_timestamp_before_an_offline_clear";
        const PROXIES: [&str; 6] = [
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
        ];
        const INJECTED: &str = "http://127.0.0.1:1";
        const FINISHED: &str = "isolated bookmark SDK assertions completed";
        async fn child(command: &mut tokio::process::Command, seconds: u64) {
            let output = tokio::time::timeout(
                Duration::from_secs(seconds),
                command
                    .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
                    .stdin(std::process::Stdio::null())
                    .kill_on_drop(true)
                    .output(),
            )
            .await
            .expect("isolated SDK child timed out")
            .expect("start isolated SDK test process");
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(output.status.success(), "SDK child failed:\n{stdout}\n{stderr}");
            // A mistyped exact filter must not count a zero-test success as coverage.
            assert!(
                stdout.contains(FINISHED),
                "SDK child did not run its assertions:\n{stdout}\n{stderr}"
            );
            println!("{stdout}");
        }
        match std::env::var(CHILD).ok().as_deref() {
            None => {
                let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
                command.env(CHILD, "injected-proxy-parent");
                for name in PROXIES {
                    command.env(name, INJECTED);
                }
                command.env_remove("NO_PROXY").env_remove("no_proxy");
                child(&mut command, 45).await;
                return;
            }
            Some("injected-proxy-parent") => {
                for name in PROXIES {
                    assert!(
                        std::env::var(name).is_ok_and(|v| v == INJECTED),
                        "proxy injection missing: {name}"
                    );
                }
                assert!(std::env::var_os("NO_PROXY").is_none());
                assert!(std::env::var_os("no_proxy").is_none());
                let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
                command.env(CHILD, "isolated-sdk");
                for name in PROXIES {
                    command.env_remove(name);
                }
                command.env("NO_PROXY", "*").env("no_proxy", "*");
                child(&mut command, 30).await;
                return;
            }
            Some("isolated-sdk") => {
                for name in PROXIES {
                    assert!(std::env::var_os(name).is_none(), "SDK inherited proxy: {name}");
                }
                assert!(std::env::var("NO_PROXY").is_ok_and(|v| v == "*"));
                assert!(std::env::var("no_proxy").is_ok_and(|v| v == "*"));
            }
            Some(_) => panic!("unknown SDK test process role"),
        }
        use axum::extract::{Request, State};
        use axum::http::Method;
        use http_body_util::BodyExt;
        use pubky::pkarr::dns::rdata::SVCB;
        use pubky::pkarr::{Cache, InMemoryCache, SignedPacket};
        use pubky::{Keypair, Pubky, PubkyHttpClient};
        use std::num::NonZeroUsize;
        struct Remote {
            owner: String,
            session: Vec<u8>,
            path: String,
            object: Mutex<Vec<u8>>,
            requests: Mutex<Vec<(Method, String)>>,
        }
        async fn answer(State(remote): State<Arc<Remote>>, request: Request) -> Response {
            let path = request.uri().path();
            if path == "/info" {
                return Json(serde_json::json!({"features":[]})).into_response();
            }
            let cookie = format!("{}=bookmark-test-cookie", remote.owner);
            if request.headers().get("cookie").and_then(|h| h.to_str().ok()) != Some(cookie.as_str()) {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            if path == "/session" {
                return remote.session.clone().into_response();
            }
            remote
                .requests
                .lock()
                .unwrap()
                .push((request.method().clone(), path.to_string()));
            if path == pm_pubky::v1::bookmarks_dir() && request.method() == Method::GET {
                return format!("pubky://{}{}\n", remote.owner, remote.path).into_response();
            }
            if path == remote.path && request.method() == Method::GET {
                return remote.object.lock().unwrap().clone().into_response();
            }
            // An intermediate-time clear must never reach the remote DELETE.
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
        struct Stop(axum_server::Handle<std::net::SocketAddr>);
        impl Drop for Stop {
            fn drop(&mut self) {
                self.0.shutdown();
            }
        }
        const URI: &str = "https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69";
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let owner = Keypair::random();
        let homeserver = Keypair::random();
        let key = owner.public_key().z32();
        let (old, path) = pm_pubky::v1::bookmark_of(URI, 100_000_000).unwrap();
        let remote = Arc::new(Remote {
            owner: key.clone(),
            path,
            session: pubky::CookieSessionRecord::new(
                &owner.public_key(),
                PERSONAL_CAPABILITIES.parse().unwrap(),
                None,
            )
            .serialize(),
            object: Mutex::new(serde_json::to_vec(&old).unwrap()),
            requests: Mutex::new(Vec::new()),
        });
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let tls = axum_server::tls_rustls::RustlsConfig::from_pem(
            cert.cert.pem().into_bytes(),
            cert.signing_key.serialize_pem().into_bytes(),
        )
        .await
        .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = axum_server::Handle::new();
        let _stop = Stop(server.clone());
        let handle = server.clone();
        let app = axum::Router::new().fallback(answer).with_state(remote.clone());
        tokio::spawn(async move {
            axum_server::from_tcp_rustls(listener, tls)
                .unwrap()
                .handle(handle)
                .serve(app.into_make_service())
                .await
                .unwrap();
        });
        tokio::time::timeout(Duration::from_secs(5), server.listening())
            .await
            .unwrap()
            .unwrap();
        let cache = Arc::new(InMemoryCache::new(NonZeroUsize::new(2).unwrap()));
        let mut target = SVCB::new(1, "localhost".try_into().unwrap());
        target.set_port(port);
        let hs_packet = SignedPacket::builder()
            .https(".".try_into().unwrap(), target, 3600)
            .sign(&homeserver)
            .unwrap();
        let user_packet = SignedPacket::builder()
            .https(
                "_pubky".try_into().unwrap(),
                SVCB::new(0, homeserver.public_key().z32().as_str().try_into().unwrap()),
                3600,
            )
            .sign(&owner)
            .unwrap();
        cache.put(&hs_packet.public_key().into(), &hs_packet);
        cache.put(&user_packet.public_key().into(), &user_packet);
        let mut builder = PubkyHttpClient::builder();
        builder.pkarr(|b| b.no_default_network().bootstrap(&["127.0.0.1:1"]).cache(cache));
        builder
            .add_root_certificates_pem(cert.cert.pem().as_bytes())
            .unwrap();
        let pubky = Pubky::with_client(builder.build().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let s = listener_state(dir.path(), &key).await;
        let grants = Grants {
            dir: grants_dir(&s),
            hs: Some(NodeHomeserver::from_pubky(pubky)),
        };
        crate::imports::write_private(
            &grant_file(&grants.dir, &key).unwrap(),
            format!("{key}:bookmark-test-cookie").as_bytes(),
        )
        .unwrap();
        s.set_personal_homeserver(Arc::new(grants));
        assert_eq!(bookmark_uris(&s, &key).await.unwrap().0, vec![URI]);
        assert_eq!(load(&s, &key).unwrap().items[0].at, 100_000_000);
        let (new, same_path) = pm_pubky::v1::bookmark_of(URI, 300_000_000).unwrap();
        assert_eq!(same_path, remote.path, "a re-heart does not change the filename");
        *remote.object.lock().unwrap() = serde_json::to_vec(&new).unwrap();
        assert_eq!(bookmark_uris(&s, &key).await.unwrap().0, vec![URI]);
        assert_eq!(load(&s, &key).unwrap().items[0].at, 300_000_000);
        let response = set_bookmark(&s, &key, URI.into(), false, Some(200))
            .await
            .unwrap();
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(
            body["bookmarked"], true,
            "the newer remote heart survives: {body}"
        );
        assert_eq!(*remote.object.lock().unwrap(), serde_json::to_vec(&new).unwrap());
        let requests = remote.requests.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|(method, path)| *method == Method::GET && path == &remote.path)
                .count(),
            2
        );
        assert!(requests.iter().all(|(method, _)| *method != Method::DELETE));
        sessions().lock().unwrap().remove(&key);
        println!("{FINISHED}");
    }

    async fn revoked(s: &NodeState, key: &str) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while crate::people::may_use_music(s, key) || crate::accounts::sessions_of(s, key) != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("revocation waited for a homeserver operation");
    }

    #[tokio::test]
    async fn remove_ends_old_approvals_and_a_post_readd_approval_keeps_its_grant() {
        use axum::extract::{Path, State};
        let dir = tempfile::tempdir().unwrap();
        let key = "yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy";
        let s = listener_state(dir.path(), key).await;
        let file = grant_file(&grants_dir(&s), key).unwrap();
        begin_approval(&s, "rm-kept", key).await.unwrap();
        assert_eq!(
            keep_if_current(&s, "rm-kept", key, || {
                crate::imports::write_private(&file, b"old-fixture-grant").unwrap();
                serde_json::json!({"state": "ok"})
            })
            .await["state"],
            "ok"
        );

        // An older approval queued on the operation lock before Remove. The roster is already
        // revoked when it resumes, exactly as keep_grant checks before writing a credential.
        begin_approval(&s, "rm-waiting", key).await.unwrap();
        let operation = person_lock(key);
        let held = operation.lock().await;
        let old = keep_if_current(&s, "rm-waiting", key, || {
            assert!(!crate::people::may_use_music(&s, key));
            serde_json::json!({"state": "denied"})
        });
        tokio::pin!(old);
        assert!(futures_util::poll!(&mut old).is_pending());
        let removal = tokio::spawn(crate::people::remove(State(s.clone()), Path(key.to_string())));
        revoked(&s, key).await;
        assert!(file.exists(), "the earlier kept grant awaits cleanup");
        let too_early = begin_approval(&s, "rm-too-early", key);
        tokio::pin!(too_early);
        assert!(futures_util::poll!(&mut too_early).is_pending());
        let re_add = crate::people::invite(
            State(s.clone()),
            Json(crate::people::NewInvite {
                access: crate::people::Access::Music,
                key: Some(key.into()),
                name: None,
                storage_quota_mb: None,
            }),
        );
        tokio::pin!(re_add);
        assert!(futures_util::poll!(&mut re_add).is_pending());
        drop(held);
        assert_eq!(old.await["state"], "denied");
        removal.await.unwrap().unwrap();
        assert!(!file.exists(), "Remove deleted the grant kept before it");
        assert!(!still_asked("rm-waiting").await);
        assert!(
            too_early.await.is_err(),
            "an admitted old request cannot create a fresh approval"
        );
        re_add.await.unwrap();
        begin_approval(&s, "rm-fresh", key).await.unwrap();
        assert_eq!(
            keep_if_current(&s, "rm-waiting", key, || panic!("old approval revived")).await["state"],
            "expired"
        );
        assert_eq!(
            keep_if_current(&s, "rm-fresh", key, || {
                crate::imports::write_private(&file, b"fresh-fixture-grant").unwrap();
                serde_json::json!({"state": "ok"})
            })
            .await["state"],
            "ok"
        );
        assert_eq!(std::fs::read(&file).unwrap(), b"fresh-fixture-grant");
        assert!(still_asked("rm-fresh").await);
        end_approvals_of(key).await;
    }

    #[tokio::test]
    async fn requests_authenticated_before_removal_cannot_touch_a_fresh_readd() {
        use axum::extract::{Path, State};
        let dir = tempfile::tempdir().unwrap();
        let key = "hhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhh";
        let s = listener_state(dir.path(), key).await;
        let token = crate::accounts::mint(&s, key).unwrap();
        // The middleware captures this before authenticating. The gates deliberately keep
        // both handlers from even reaching their locks until cleanup AND fresh approval finish.
        let revision = crate::people::access_started(&s);
        let caller = crate::accounts::caller(&s, Some(&token)).unwrap();
        let (start_tx, start_rx) = tokio::sync::oneshot::channel::<()>();
        let (disconnect_tx, disconnect_rx) = tokio::sync::oneshot::channel::<()>();
        let old_start = AUTHENTICATED_ACCESS.scope(revision, async {
            start_rx.await.unwrap();
            begin_approval(&s, "delayed-old-start", key).await
        });
        let old_disconnect = AUTHENTICATED_ACCESS.scope(revision, async {
            disconnect_rx.await.unwrap();
            disconnect(State(s.clone()), axum::Extension(caller)).await
        });
        tokio::pin!(old_start, old_disconnect);
        assert!(futures_util::poll!(&mut old_start).is_pending());
        assert!(futures_util::poll!(&mut old_disconnect).is_pending());
        crate::people::remove(State(s.clone()), Path(key.into()))
            .await
            .unwrap();
        crate::people::invite(
            State(s.clone()),
            Json(crate::people::NewInvite {
                access: crate::people::Access::Music,
                key: Some(key.into()),
                name: None,
                storage_quota_mb: None,
            }),
        )
        .await
        .unwrap();
        let fresh = Arc::new(Forgets::default());
        s.set_personal_homeserver(fresh.clone());
        let file = grant_file(&grants_dir(&s), key).unwrap();
        begin_approval(&s, "delayed-fresh", key).await.unwrap();
        assert_eq!(
            keep_if_current(&s, "delayed-fresh", key, || {
                crate::imports::write_private(&file, b"fresh-grant").unwrap();
                serde_json::json!({"state":"ok"})
            })
            .await["state"],
            "ok"
        );
        start_tx.send(()).unwrap();
        disconnect_tx.send(()).unwrap();
        assert_eq!(old_start.await.unwrap_err().status, StatusCode::CONFLICT);
        assert_eq!(old_disconnect.await.unwrap_err().status, StatusCode::CONFLICT);
        assert!(still_asked("delayed-fresh").await);
        assert!(!still_asked("delayed-old-start").await);
        assert!(
            fresh.connected(key),
            "an old disconnect cannot delete the fresh grant"
        );
        assert_eq!(std::fs::read(&file).unwrap(), b"fresh-grant");
        // Read-triggered replay and direct writes carry the same authenticated generation.
        assert_eq!(
            AUTHENTICATED_ACCESS
                .scope(revision, bookmark_uris(&s, key))
                .await
                .unwrap_err()
                .status,
            StatusCode::CONFLICT
        );
        let stale_write = AUTHENTICATED_ACCESS
            .scope(
                revision,
                set_bookmark(&s, key, "https://example.com/song".into(), true, None),
            )
            .await;
        assert_eq!(stale_write.unwrap_err().status, StatusCode::CONFLICT);
        end_approvals_of(key).await;
    }

    #[tokio::test]
    async fn a_restored_generation_denies_an_old_connect_before_its_grant_write() {
        let dir = tempfile::tempdir().unwrap();
        let key = "iiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiiii";
        let s = listener_state(dir.path(), key).await;
        begin_approval(&s, "restore-old-connect", key).await.unwrap();
        let roster = s.roster.lock().await;
        let keeping = keep_if_current(&s, "restore-old-connect", key, || {
            panic!("a pre-restore approval must not persist its grant")
        });
        tokio::pin!(keeping);
        assert!(futures_util::poll!(&mut keeping).is_pending());
        crate::people::restored_access(&s);
        drop(roster);
        assert_eq!(keeping.await["state"], "denied");
        begin_approval(&s, "restore-fresh-connect", key).await.unwrap();
        assert_eq!(
            keep_if_current(
                &s,
                "restore-fresh-connect",
                key,
                || serde_json::json!({"state":"ok"})
            )
            .await["state"],
            "ok"
        );
        end_approvals_of(key).await;
    }

    #[tokio::test]
    async fn restore_waits_for_cleanup_without_holding_up_another_revocation() {
        use axum::extract::{Path, State};
        let dir = tempfile::tempdir().unwrap();
        let key = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let s = listener_state(dir.path(), key).await;
        let operation = person_lock(key);
        let held = operation.lock().await;
        let removal = tokio::spawn(crate::people::remove(State(s.clone()), Path(key.to_string())));
        revoked(&s, key).await;
        let restore = crate::people::roster_for_restore(&s);
        tokio::pin!(restore);
        assert!(futures_util::poll!(&mut restore).is_pending());
        assert!(
            s.roster.try_lock().is_ok(),
            "restore must not block other roster changes while waiting"
        );
        drop(held);
        removal.await.unwrap().unwrap();
        let roster = restore.await;
        assert!(s.removing_people.lock().unwrap().is_empty());
        drop(roster);
    }

    #[tokio::test]
    async fn restore_also_waits_for_a_readds_retry_of_failed_removal() {
        use axum::extract::{Path, State};
        use std::sync::atomic::Ordering::SeqCst;
        let dir = tempfile::tempdir().unwrap();
        let key = "dddddddddddddddddddddddddddddddddddddddddddddddddddd";
        let s = listener_state(dir.path(), key).await;
        let store = Arc::new(Forgets::default());
        s.set_personal_homeserver(store.clone());
        store.1.store(true, SeqCst);
        assert!(crate::people::remove(State(s.clone()), Path(key.into()))
            .await
            .is_err());
        assert!(leftover(&s, key).unwrap());
        store.1.store(false, SeqCst);

        // An admitted old operation holds up cleanup retried by the re-add.
        let operation = person_lock(key);
        let held = operation.lock().await;
        let re_add = crate::people::invite(
            State(s.clone()),
            Json(crate::people::NewInvite {
                access: crate::people::Access::Music,
                key: Some(key.into()),
                name: None,
                storage_quota_mb: None,
            }),
        );
        tokio::pin!(re_add);
        assert!(futures_util::poll!(&mut re_add).is_pending());
        assert!(s.removing_people.lock().unwrap().contains(key));
        let restore = crate::people::roster_for_restore(&s);
        tokio::pin!(restore);
        assert!(futures_util::poll!(&mut restore).is_pending());
        assert!(s.roster.try_lock().is_ok());
        drop(held);
        re_add.await.unwrap();
        let roster = restore.await;
        crate::people::restored_access(&s);
        let setting = format!("person.{key}.tidal");
        s.library
            .lock()
            .unwrap()
            .set_setting(&setting, "restored-fixture")
            .unwrap();
        drop(roster);
        assert!(s.removing_people.lock().unwrap().is_empty());
        assert_eq!(
            s.library
                .lock()
                .unwrap()
                .get_setting(&setting)
                .unwrap()
                .as_deref(),
            Some("restored-fixture")
        );
    }

    #[tokio::test]
    async fn a_completed_music_off_supersedes_older_queued_on_and_invite() {
        use axum::extract::{Path, State};
        for invite in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let key = "ffffffffffffffffffffffffffffffffffffffffffffffffffff";
            let s = listener_state(dir.path(), key).await;
            // Remove retains a bundled-homeserver person, with Music off.
            let mut roster = crate::people::load(&s);
            roster.people[0].access = crate::people::Access::HomeserverMusic;
            s.library
                .lock()
                .unwrap()
                .set_setting("people", &serde_json::to_string(&roster).unwrap())
                .unwrap();
            let operation = person_lock(key);
            let held = operation.lock().await;
            let removal = tokio::spawn(crate::people::remove(State(s.clone()), Path(key.into())));
            revoked(&s, key).await;
            let enabling = async {
                if invite {
                    crate::people::invite(
                        State(s.clone()),
                        Json(crate::people::NewInvite {
                            access: crate::people::Access::Music,
                            key: Some(key.into()),
                            name: None,
                            storage_quota_mb: None,
                        }),
                    )
                    .await
                    .map(|_| ())
                } else {
                    crate::people::change(
                        State(s.clone()),
                        Path(key.into()),
                        Json(crate::people::PersonChange {
                            music: Some(true),
                            ..Default::default()
                        }),
                    )
                    .await
                    .map(|_| ())
                }
            };
            tokio::pin!(enabling);
            assert!(futures_util::poll!(&mut enabling).is_pending());
            let off = crate::people::change(
                State(s.clone()),
                Path(key.into()),
                Json(crate::people::PersonChange {
                    music: Some(false),
                    ..Default::default()
                }),
            );
            tokio::pin!(off);
            let std::task::Poll::Ready(answer) = futures_util::poll!(&mut off) else {
                panic!("Music off waited for the held homeserver operation")
            };
            assert!(!answer.unwrap().0.music_enabled);
            drop(held);
            removal.await.unwrap().unwrap();
            assert_eq!(enabling.await.unwrap_err().status, StatusCode::CONFLICT);
            assert!(!crate::people::may_use_music(&s, key));
            assert_eq!(crate::accounts::sessions_of(&s, key), 0);
        }
    }

    #[tokio::test]
    async fn a_restore_that_wins_before_retry_cleanup_preserves_its_new_settings() {
        use axum::extract::{Path, State};
        let dir = tempfile::tempdir().unwrap();
        let key = "gggggggggggggggggggggggggggggggggggggggggggggggggggg";
        let s = listener_state(dir.path(), key).await;
        let store = Arc::new(Forgets::default());
        store.1.store(true, std::sync::atomic::Ordering::SeqCst);
        s.set_personal_homeserver(store.clone());
        assert!(crate::people::remove(State(s.clone()), Path(key.into()))
            .await
            .is_err());
        store.1.store(false, std::sync::atomic::Ordering::SeqCst);
        let membership = membership_lock(key);
        let held = membership.lock().await;
        let re_add = crate::people::invite(
            State(s.clone()),
            Json(crate::people::NewInvite {
                access: crate::people::Access::Music,
                key: Some(key.into()),
                name: None,
                storage_quota_mb: None,
            }),
        );
        tokio::pin!(re_add);
        assert!(futures_util::poll!(&mut re_add).is_pending());
        // No cleanup was registered yet: restore wins, replaces access intent and settings.
        let roster = crate::people::roster_for_restore(&s).await;
        crate::people::restored_access(&s);
        let setting = format!("person.{key}.tidal");
        s.library
            .lock()
            .unwrap()
            .set_setting(&setting, "restored-fixture")
            .unwrap();
        drop(roster);
        drop(held);
        assert_eq!(re_add.await.unwrap_err().status, StatusCode::CONFLICT);
        assert_eq!(
            s.library
                .lock()
                .unwrap()
                .get_setting(&setting)
                .unwrap()
                .as_deref(),
            Some("restored-fixture")
        );
        assert!(
            store.connected(key),
            "the superseded retry did not delete the restored state"
        );
    }
}
