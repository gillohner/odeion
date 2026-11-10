//! A listener's own Collections on their own homeserver (TASK-listener-personal-library-1104,
//! ADR 0016 step 5; the grant and their Bookmarks are in [`crate::personal`]). The owner's stay
//! what they were: rows in the library ([`crate::playlists`]), pushed by the owner's sync.
//!
//! - **Their homeserver** holds them the way the owner's sync writes the owner's: social v1
//!   `collection` posts, every version a private draft under `/priv/social/v1/posts/`, and a
//!   public one also published under `/pub/social/v1/posts/` (pubky-social-specs `plan_publish`).
//! - **The node's copy** (`person.<z32>.collections`) answers at once. A change made while their
//!   homeserver is away waits there and is written at their next read or write: the newest state
//!   of the collection, not every step. A newer version from their homeserver (written on
//!   another device) replaces the copy's, and one deleted there goes here too. A change that
//!   waited is written whole as the node has it, its sharing included: the newest change wins,
//!   so a rename made here while away unpublishes a share made since on another device.
//! - **Routes:** `/v1/collections` and `/v1/playlists` answer each caller with their own: the
//!   owner [`crate::playlists`], a listener this module. What only the owner has stays refused
//!   (accounts.rs): node-only songs, play orders and pins, imports, Play from.
//! - **Play:** a listener's collection queues what the library has, on a zone open to them.
//!   Nothing plays through the owner's streaming accounts.

use crate::accounts::{Caller, Role, PERSON_PREFIX};
use crate::personal::{self, PersonalError, PersonalHomeserver};
use crate::playlists;
use crate::state::NodeState;
use crate::types::*;
use crate::{ApiError, ApiResult};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use pm_library::Library;
use pm_pubky::v1;
use pm_pubky::{MusicContent, MusicPost, Playlist};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

/// At most this many of their collections are downloaded per request (a first connect with
/// many); the rest come with the next.
const READS_PER_REQUEST: usize = 16;

/// Shared by listings, downloads, replay and removals, including the fallback. A slow
/// homeserver cannot keep the person operation lock for a whole backlog. Each completed
/// mutation is checkpointed; reaching either limit leaves the remaining intent pending.
const CALLS_PER_REQUEST: usize = 32;
const REQUEST_TIME: Duration = Duration::from_secs(8);

struct Work {
    left: usize,
    reads: usize,
    deadline: tokio::time::Instant,
}

impl Work {
    fn new() -> Self {
        Self {
            left: CALLS_PER_REQUEST,
            reads: READS_PER_REQUEST,
            deadline: tokio::time::Instant::now() + REQUEST_TIME,
        }
    }

    fn available(&self) -> bool {
        self.left > 0 && tokio::time::Instant::now() < self.deadline
    }

    async fn call<T>(
        &mut self,
        f: impl Future<Output = Result<T, PersonalError>>,
    ) -> Result<T, PersonalError> {
        if !self.available() {
            return Err(PersonalError::Unreachable(
                "collection work will continue on the next request".into(),
            ));
        }
        self.left -= 1;
        tokio::time::timeout_at(self.deadline, f)
            .await
            .unwrap_or_else(|_| {
                self.left = 0;
                Err(PersonalError::Unreachable(
                    "collection request deadline reached".into(),
                ))
            })
    }

    async fn read(
        &mut self,
        b: &dyn PersonalHomeserver,
        key: &str,
        path: &str,
    ) -> Result<Option<Vec<u8>>, PersonalError> {
        if self.reads == 0 {
            return Err(PersonalError::Unreachable(
                "collection downloads will continue on the next request".into(),
            ));
        }
        self.reads -= 1;
        self.call(b.read(key, path)).await
    }
}

/// At most this many of their collections are kept on the node; newer ones from their homeserver
/// beyond it aren't taken, and making one more here is refused. Their listings are bounded too
/// (`personal::PERSONAL_LIST`), so their homeserver can't make the node hold more.
const MAX_COLLECTIONS: usize = 1_000;

/// At most this big, one of their collections as written (its social post): the node can't read
/// back a bigger one ([`pm_pubky::homeserver::OBJECT_MAX`]).
const MAX_COLLECTION_BYTES: usize = pm_pubky::homeserver::OBJECT_MAX;

/// At most this big, the node's whole copy of one listener's collections, as stored.
const MAX_SHELF_BYTES: usize = 32 * 1024 * 1024;

fn bytes_of<T: Serialize>(x: &T) -> usize {
    serde_json::to_vec(x).map_or(usize::MAX, |v| v.len())
}

/// How big `p` is as the social post the node writes for it.
fn post_size(p: &Playlist) -> usize {
    match v1::to_v1(&MusicContent::Playlist(p.clone())) {
        Ok(v1::V1Post::Social(post)) => bytes_of(&post),
        _ => 0,
    }
}

/// The node's copy of one listener's collections.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Shelf {
    #[serde(default)]
    items: Vec<Kept>,
    /// Newest versions of their private posts that aren't music collections (other drafts of
    /// theirs): not downloaded again.
    #[serde(default)]
    other: Vec<String>,
    /// Newest versions of their collections the node left unread because its copy would pass
    /// [`MAX_SHELF_BYTES`], with what they would take: not downloaded again until they fit (other
    /// collections deleted or shortened), and never written over.
    #[serde(default)]
    too_big: Vec<TooBig>,
    /// The last read couldn't list all of their posts (more than the node lists, or their
    /// homeserver away): `GET /v1/me/homeserver` says `stale`.
    #[serde(default)]
    behind: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TooBig {
    path: String,
    bytes: usize,
}

/// Whether a version left unread for the copy's size would fit now, in place of `was`.
fn fits_now(sh: &Shelf, path: &str, was: usize) -> bool {
    sh.too_big
        .iter()
        .find(|t| t.path == path)
        .is_some_and(|t| reserved_size(sh).saturating_sub(was).saturating_add(t.bytes) <= MAX_SHELF_BYTES)
}

/// What a refresh found: whether the node couldn't read what is there, and the collections
/// whose newer version it left unread (to stay within [`MAX_SHELF_BYTES`], or one it can't read
/// or hold at all): a change of those is refused rather than written over it; a delete goes.
#[derive(Debug, Default)]
struct Caught {
    stale: bool,
    held_back: std::collections::HashMap<String, Held>,
}

/// Why a collection is held back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    /// Its newer version would take the node past [`MAX_SHELF_BYTES`].
    Size,
    /// Its newer version is one the node can't read or hold (over its read cap, refused, or not
    /// a collection it can keep).
    Unreadable,
}

/// Whether the node already knows a version of `k` newer than its head that it set aside
/// (`other`) or left unread for the copy's size (`too_big`), even when it can read nothing now.
fn known_unread(other: &[String], too_big: &[TooBig], k: &Kept) -> Option<Held> {
    let newer = |path: &str| {
        v1::parse_version(path).is_some_and(|(id, edit)| id == k.id && edit.as_bytes() >= k.head.as_bytes())
    };
    if other.iter().any(|p| newer(p)) {
        Some(Held::Unreadable)
    } else if too_big.iter().any(|t| newer(&t.path)) {
        Some(Held::Size)
    } else {
        None
    }
}

impl Caught {
    fn stale(stale: bool) -> Self {
        Self {
            stale,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Kept {
    id: String,
    playlist: Playlist,
    #[serde(default)]
    public: bool,
    /// The newest version the node has, written or read (its edit id); empty before the first.
    #[serde(default)]
    head: String,
    /// Only current heads and an in-flight write's paths. Full histories are listed for each
    /// deletion batch, never cached here: older remote paths cannot grow the shelf unboundedly.
    #[serde(default)]
    versions: Vec<String>,
    /// Changed here and not on their homeserver yet.
    #[serde(default)]
    pending: bool,
    /// Deleted here: its versions go at the next write.
    #[serde(default)]
    deleted: bool,
    /// An explicit make-private intent may remove published copies even while an unfamiliar
    /// private head prevents rewriting the collection.
    #[serde(default, skip_serializing_if = "is_false")]
    unshare: bool,
    /// The exact version being sent. Retrying an interrupted write uses the same path/body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    writing: Option<Writing>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Writing {
    edit: String,
    drafted: bool,
    published: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Reserve the largest checkpoint, not just today's encoding. A first draft fills the head;
/// after an uncertain response, observation can fill it before `drafted` becomes true. Keep
/// the full head growth available without claiming that a planned write has landed. Clearing
/// `behind` can also grow the shelf by one byte; completion removes the larger Writing record.
fn reserved_size(sh: &Shelf) -> usize {
    bytes_of(sh)
        + usize::from(sh.behind)
        + sh.items
            .iter()
            .filter_map(|k| k.writing.as_ref().map(|w| (k, w)))
            .map(|(k, w)| bytes_of(&w.edit).saturating_sub(bytes_of(&k.head)))
            .sum::<usize>()
}

fn shelf_key(key: &str) -> String {
    format!("{PERSON_PREFIX}{}.collections", key.trim_start_matches("pubky"))
}

fn load(s: &NodeState, key: &str) -> Result<Shelf, ApiError> {
    let lib = s.library.lock().expect("library poisoned");
    let Some(json) = lib.get_setting(&shelf_key(key))? else {
        return Ok(Shelf::default());
    };
    serde_json::from_str(&json).map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot read saved collections: {e}"),
        )
    })
}

fn save(s: &NodeState, key: &str, sh: &Shelf) -> Result<(), ApiError> {
    let encoded = serde_json::to_string(sh).expect("serializes");
    if encoded.len() > MAX_SHELF_BYTES {
        return Err(shelf_full());
    }
    let lib = s.library.lock().expect("library poisoned");
    // Removed (People › Remove) while this request waited on their homeserver: nothing of
    // theirs comes back. Checked under the same hold as the write, so a removal can't land
    // between the two
    if !crate::people::listed_for_music(&lib, key.trim_start_matches("pubky")) {
        return Ok(());
    }
    lib.set_setting(&shelf_key(key), &encoded)?;
    Ok(())
}

/// How many of their collections wait to be written (`GET /v1/me/homeserver`).
pub(crate) fn pending(s: &NodeState, key: &str) -> Result<usize, ApiError> {
    Ok(load(s, key)?.items.iter().filter(|k| k.pending).count())
}

/// Whether the node's last read of their collections was incomplete (`GET /v1/me/homeserver`).
pub(crate) fn behind(s: &NodeState, key: &str) -> Result<bool, ApiError> {
    Ok(load(s, key)?.behind)
}

fn rejected(e: impl std::fmt::Display) -> PersonalError {
    PersonalError::Rejected(e.to_string())
}

fn shelf_full() -> ApiError {
    ApiError::new(
        StatusCode::PAYLOAD_TOO_LARGE,
        format!(
            "Your collections on this node would be over {} MiB: delete or shorten some first",
            MAX_SHELF_BYTES >> 20
        ),
    )
}

/// Keep only the two current heads. History remains on the homeserver and is listed afresh
/// before every mutation. In-flight paths are reserved before their writes begin.
fn compact_versions(paths: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut roots = BTreeMap::new();
    for path in paths {
        let root = if path.starts_with("/pub/") { "pub" } else { "priv" };
        let entry = roots.entry(root).or_insert_with(String::new);
        if path > *entry {
            *entry = path;
        }
    }
    roots.into_values().collect()
}

/// Old releases could persist an oversized cache through history growth. Only remotely backed,
/// non-pending cache entries may be evicted to recover that legacy state. Pending user intent
/// and the collection being edited are retained. Evicted data is fetched again from its source.
fn bound_existing(sh: &mut Shelf, keep: Option<&str>) -> Result<(), ApiError> {
    for k in &mut sh.items {
        k.versions = compact_versions(std::mem::take(&mut k.versions));
    }
    while reserved_size(sh) > MAX_SHELF_BYTES {
        sh.behind = true;
        if sh.other.pop().is_some() || sh.too_big.pop().is_some() {
            continue;
        }
        let Some(i) = sh
            .items
            .iter()
            .rposition(|k| !k.pending && !k.head.is_empty() && Some(k.id.as_str()) != keep)
        else {
            return Err(shelf_full());
        };
        sh.items.remove(i);
    }
    Ok(())
}

fn replace(sh: &mut Shelf, k: Kept) -> Result<(), ApiError> {
    let mut candidate = sh.clone();
    if let Some(i) = candidate.items.iter().position(|x| x.id == k.id) {
        candidate.items[i] = k;
    } else {
        candidate.items.push(k);
    }
    if reserved_size(&candidate) > MAX_SHELF_BYTES {
        return Err(shelf_full());
    }
    *sh = candidate;
    Ok(())
}

#[derive(Default)]
struct Remote {
    private: Option<v1::Head>,
    public: Option<v1::Head>,
}

impl Remote {
    fn heads(&self) -> impl Iterator<Item = &v1::Head> {
        [&self.private, &self.public].into_iter().flatten()
    }
    fn versions(&self) -> Vec<String> {
        // Public copies go first: an interrupted delete still reduces exposure.
        [&self.public, &self.private]
            .into_iter()
            .flatten()
            .flat_map(|h| h.versions.iter().cloned())
            .collect()
    }
}

enum Pushed {
    Done,
    Held,
    Waiting(PersonalError),
    Full,
}

type Prepared = (String, serde_json::Value, Option<(String, serde_json::Value)>);

fn prepare(key: &str, k: &mut Kept) -> Result<Prepared, PersonalError> {
    let id = k.id.clone();
    let owner = match v1::PubkyId::try_from(key) {
        Ok(o) => o,
        Err(e) => return Err(rejected(e)),
    };
    let post = match v1::to_v1(&MusicContent::Playlist(k.playlist.clone())) {
        Ok(v1::V1Post::Social(p)) => p,
        other => return Err(rejected(format!("not a collection: {other:?}"))),
    };
    if k.writing.is_none() {
        let path = match v1::V1Post::Social(post.clone()).version_path(
            &k.id,
            (!k.head.is_empty()).then_some(k.head.as_str()),
            v1::Root::Priv,
            &owner,
        ) {
            Ok(p) => p,
            Err(e) => return Err(rejected(e)),
        };
        let (_, edit) = v1::parse_version(&path).expect("a version path");
        k.writing = Some(Writing {
            edit,
            drafted: false,
            published: false,
        });
    }
    let writing = k.writing.clone().expect("prepared write");
    let path = format!(
        "{}{id}/{}.json",
        v1::posts_dir(v1::Root::Priv, v1::SOCIAL_NAMESPACE),
        writing.edit
    );
    let publication = if k.public {
        match v1::publish_plan(&post, &id, &writing.edit, &owner) {
            Ok(p) => Some(p),
            Err(e) => return Err(rejected(e)),
        }
    } else {
        None
    };
    k.versions.push(path.clone());
    if let Some((dest, _)) = &publication {
        k.versions.push(dest.clone());
    }
    k.versions = compact_versions(std::mem::take(&mut k.versions));
    Ok((
        path,
        serde_json::to_value(post).expect("a post serializes"),
        publication,
    ))
}

/// Durable intent precedes I/O. Each returned success is checkpointed before the next await;
/// a cancelled/expired write is retried at its reserved path, and removals are idempotent.
async fn push(
    s: &NodeState,
    b: &dyn PersonalHomeserver,
    key: &str,
    sh: &mut Shelf,
    id: &str,
    remote: &Remote,
    held: Option<Held>,
    work: &mut Work,
) -> Result<Pushed, ApiError> {
    let mut k = sh
        .items
        .iter()
        .find(|k| k.id == id)
        .expect("pending collection")
        .clone();
    if !crate::people::may_use_music(s, key) {
        return Ok(Pushed::Waiting(PersonalError::NoGrant));
    }
    let prepared = if !k.deleted && held.is_none() {
        let prepared = match prepare(key, &mut k) {
            Ok(p) => p,
            Err(e) => return Ok(Pushed::Waiting(e)),
        };
        // Budget the complete shelf, including resulting paths and progress, before ANY
        // remote mutation. A refusal here leaves remote files untouched.
        if replace(sh, k.clone()).is_err() {
            return Ok(Pushed::Full);
        }
        save(s, key, sh)?;
        Some(prepared)
    } else {
        None
    };
    if k.deleted || k.unshare || (held.is_none() && !k.public) {
        for path in remote
            .versions()
            .into_iter()
            .filter(|p| k.deleted || p.starts_with("/pub/"))
        {
            if !crate::people::may_use_music(s, key) {
                return Ok(Pushed::Waiting(PersonalError::NoGrant));
            }
            if let Err(e) = work.call(b.remove(key, &path)).await {
                return Ok(Pushed::Waiting(e));
            }
            k.versions.retain(|p| p != &path);
            replace(sh, k.clone())?;
            save(s, key, sh)?;
        }
    }
    if k.deleted {
        sh.items.retain(|x| x.id != id);
        save(s, key, sh)?;
        return Ok(Pushed::Done);
    }
    if held.is_some() {
        return Ok(Pushed::Held);
    }
    let (path, body, publication) = prepared.expect("a writable collection was prepared");
    let writing = k.writing.clone().expect("prepared write");
    if !writing.drafted {
        if !crate::people::may_use_music(s, key) {
            return Ok(Pushed::Waiting(PersonalError::NoGrant));
        }
        if let Err(e) = work.call(b.write(key, &path, &body)).await {
            return Ok(Pushed::Waiting(e));
        }
        k.head = writing.edit.clone();
        k.writing.as_mut().expect("prepared write").drafted = true;
        replace(sh, k.clone())?;
        save(s, key, sh)?;
    }
    if let Some((dest, body)) = publication {
        if !writing.published {
            if !crate::people::may_use_music(s, key) {
                return Ok(Pushed::Waiting(PersonalError::NoGrant));
            }
            if let Err(e) = work.call(b.write(key, &dest, &body)).await {
                return Ok(Pushed::Waiting(e));
            }
            k.writing.as_mut().expect("prepared write").published = true;
            replace(sh, k.clone())?;
            save(s, key, sh)?;
        }
    }
    k.pending = false;
    k.unshare = false;
    k.writing = None;
    replace(sh, k)?;
    save(s, key, sh)?;
    Ok(Pushed::Done)
}

/// What a homeserver answer means for a read: a refusal marks the grant lost.
fn read_outcome(s: &NodeState, key: &str, e: PersonalError) -> Result<bool, ApiError> {
    match e {
        PersonalError::Refused(m) => {
            personal::mark_lost(s, key, &m)?;
            Ok(false)
        }
        _ => Ok(true),
    }
}

fn hold(sh: &mut Shelf, id: &str, head: &v1::Head, why: Held, bytes: usize, caught: &mut Caught) {
    caught.held_back.insert(id.to_string(), why);
    let mut candidate = sh.clone();
    candidate.other.retain(|p| p != &head.path);
    candidate.too_big.retain(|t| t.path != head.path);
    match why {
        Held::Unreadable => candidate.other.push(head.path.clone()),
        Held::Size => candidate.too_big.push(TooBig {
            path: head.path.clone(),
            bytes,
        }),
    }
    if reserved_size(&candidate) <= MAX_SHELF_BYTES {
        *sh = candidate;
    } else {
        // No space even for a marker: don't write or advance the head. The next request must
        // classify it again. A marker is only a download optimization, never write authority.
        sh.behind = true;
    }
}

/// Classify new heads BEFORE replay, on both listing paths. An edit also validates the cached
/// head: older releases may have cached a lossy display projection. Pending intent is retained
/// while all newly seen original posts are checked, including a newer public copy.
async fn observe(
    s: &NodeState,
    b: &dyn PersonalHomeserver,
    key: &str,
    sh: &mut Shelf,
    id: &str,
    remote: &Remote,
    force: bool,
    work: &mut Work,
) -> Result<Caught, ApiError> {
    let mut caught = Caught::default();
    let old = sh.items.iter().find(|k| k.id == id).cloned();
    if old.as_ref().is_some_and(|k| k.deleted) {
        return Ok(caught);
    }
    if remote.private.is_none() && remote.public.is_none() {
        if old.as_ref().is_some_and(|k| !k.pending) {
            sh.items.retain(|k| k.id != id);
        }
        return Ok(caught);
    }
    if old.is_none() && remote.private.is_none() {
        return Ok(caught);
    }
    if old.is_none() && sh.items.len() >= MAX_COLLECTIONS {
        return Ok(caught);
    }
    let paths: Vec<String> = remote.heads().map(|h| h.path.clone()).collect();
    sh.other
        .retain(|p| v1::parse_version(p).is_none_or(|(i, _)| i != id) || paths.contains(p));
    sh.too_big
        .retain(|t| v1::parse_version(&t.path).is_none_or(|(i, _)| i != id) || paths.contains(&t.path));
    let needs_read = |head: &v1::Head| {
        old.as_ref()
            .is_none_or(|k| force || k.pending || head.edit_id > k.head)
    };
    let was = old.as_ref().map_or(0, bytes_of);
    // A current held head already prevents replacing or replaying this collection. Check
    // both roots before downloading either sibling: otherwise an uncached private sibling
    // spends a read on every request and can permanently starve older discovery. A size
    // hold stops doing so once there is room. No cached candidate becomes write authority;
    // explicit deletion/unsharing can still remove the current public paths while held.
    for head in remote.heads() {
        if !needs_read(head) {
            continue;
        }
        if sh.other.contains(&head.path) {
            caught.held_back.insert(id.to_string(), Held::Unreadable);
        } else if sh.too_big.iter().any(|t| t.path == head.path) && !fits_now(sh, &head.path, was) {
            caught.held_back.insert(id.to_string(), Held::Size);
        }
    }
    if caught.held_back.contains_key(id) {
        return Ok(caught);
    }
    let mut candidate = old.clone();
    for head in remote.heads() {
        if !needs_read(head) {
            continue;
        }
        let bytes = match work.read(b, key, &head.path).await {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                caught.stale = true;
                continue;
            }
            Err(PersonalError::Rejected(_)) => {
                hold(sh, id, head, Held::Unreadable, 0, &mut caught);
                continue;
            }
            Err(e) => {
                caught.stale = read_outcome(s, key, e)?;
                return Ok(caught);
            }
        };
        let root = if head.path.starts_with("/pub/") {
            v1::Root::Pub
        } else {
            v1::Root::Priv
        };
        let Some(playlist) = their_collection(&bytes, key, id, &head.edit_id, root) else {
            hold(sh, id, head, Held::Unreadable, 0, &mut caught);
            continue;
        };
        let remote_k = Kept {
            id: id.to_string(),
            playlist,
            public: remote.public.is_some(),
            head: head.edit_id.clone(),
            versions: paths.clone(),
            pending: false,
            deleted: false,
            unshare: false,
            writing: None,
        };
        let mut trial = sh.clone();
        if replace(&mut trial, remote_k.clone()).is_err() {
            hold(sh, id, head, Held::Size, bytes_of(&remote_k), &mut caught);
            continue;
        }
        sh.too_big.retain(|t| t.path != head.path);
        // Drafts are authoritative for display. A public head is checked for safe rewriting,
        // but cannot replace the private collection's content.
        if root == v1::Root::Priv && !old.as_ref().is_some_and(|k| k.pending) {
            candidate = Some(remote_k);
        }
    }
    if caught.stale || caught.held_back.contains_key(id) {
        return Ok(caught);
    }
    if let Some(mut k) = candidate {
        k.versions = paths;
        if !k.pending {
            k.public = remote.public.is_some();
        }
        if let Some(newest) = remote.heads().map(|h| &h.edit_id).max() {
            if k.writing.as_ref().is_some_and(|w| newest > &w.edit) {
                k.writing = None;
            }
            if newest > &k.head {
                k.head = newest.clone();
            }
        }
        if let Some(w) = &k.writing {
            k.versions.push(format!(
                "{}{id}/{}.json",
                v1::posts_dir(v1::Root::Priv, v1::SOCIAL_NAMESPACE),
                w.edit
            ));
            if k.public {
                k.versions.push(format!(
                    "{}{id}/{}.json",
                    v1::posts_dir(v1::Root::Pub, v1::SOCIAL_NAMESPACE),
                    w.edit
                ));
            }
        }
        k.versions = compact_versions(k.versions);
        if replace(sh, k).is_err() {
            caught.held_back.insert(id.to_string(), Held::Size);
        }
    }
    Ok(caught)
}

async fn list_one(
    b: &dyn PersonalHomeserver,
    key: &str,
    id: &str,
    expected: (bool, bool),
    work: &mut Work,
) -> Result<Remote, PersonalError> {
    let dir = |root| format!("{}{id}/", v1::posts_dir(root, v1::SOCIAL_NAMESPACE));
    let private = v1::heads(work.call(b.list(key, &dir(v1::Root::Priv), expected.0)).await?).remove(id);
    let public = v1::heads(work.call(b.list(key, &dir(v1::Root::Pub), expected.1)).await?).remove(id);
    Ok(Remote { private, public })
}

async fn catch_up_one(
    s: &NodeState,
    b: &dyn PersonalHomeserver,
    key: &str,
    sh: &mut Shelf,
    id: &str,
    work: &mut Work,
) -> Result<(Caught, Remote), ApiError> {
    let expected = sh.items.iter().find(|k| k.id == id).map_or((false, false), |k| {
        (!k.pending && !k.versions.is_empty(), !k.pending && k.public)
    });
    let remote = match list_one(b, key, id, expected, work).await {
        Ok(r) => r,
        Err(e) => return Ok((Caught::stale(read_outcome(s, key, e)?), Remote::default())),
    };
    let caught = observe(s, b, key, sh, id, &remote, true, work).await?;
    save(s, key, sh)?;
    Ok((caught, remote))
}

fn discard_rejected(sh: &mut Shelf, id: &str) {
    if let Some(k) = sh.items.iter_mut().find(|k| k.id == id) {
        // A reserved path is not proof that a draft landed. For an existing collection,
        // re-read its actual remote state; a rejected never-written new one is dropped.
        if k.head.is_empty() {
            sh.items.retain(|k| k.id != id);
        } else {
            k.pending = false;
            k.writing = None;
            k.unshare = false;
            k.head.clear();
        }
    }
}

async fn replay(
    s: &NodeState,
    b: &dyn PersonalHomeserver,
    key: &str,
    sh: &mut Shelf,
    id: &str,
    remote: &Remote,
    caught: &mut Caught,
    work: &mut Work,
) -> Result<(), ApiError> {
    if !sh.items.iter().any(|k| k.id == id && k.pending) {
        return Ok(());
    }
    save(s, key, sh)?;
    match push(s, b, key, sh, id, remote, caught.held_back.get(id).copied(), work).await? {
        Pushed::Done | Pushed::Held => {}
        Pushed::Full => {
            caught.held_back.insert(id.to_string(), Held::Size);
        }
        Pushed::Waiting(PersonalError::Rejected(_)) => {
            if sh.items.iter().any(|k| k.id == id && (k.deleted || k.unshare)) {
                caught.stale = true;
                return Ok(());
            }
            discard_rejected(sh, id);
            let read = observe(s, b, key, sh, id, remote, true, work).await?;
            caught.stale |= read.stale;
            caught.held_back.extend(read.held_back);
            save(s, key, sh)?;
        }
        Pushed::Waiting(e) => caught.stale |= read_outcome(s, key, e)?,
    }
    Ok(())
}

/// Full and targeted listings share the same budget and classification/replay code. During an
/// edit refresh, no pending target is replayed. Unrelated replay follows the durable new edit.
async fn refresh(
    s: &NodeState,
    b: &dyn PersonalHomeserver,
    key: &str,
    sh: &mut Shelf,
    first: Option<&str>,
    editing: bool,
    work: &mut Work,
) -> Result<(Caught, HashMap<String, Remote>), ApiError> {
    let had = sh.items.iter().any(|k| !k.pending && !k.versions.is_empty());
    let had_public = sh.items.iter().any(|k| !k.pending && k.public);
    let private = match work
        .call(b.list(key, &v1::posts_dir(v1::Root::Priv, v1::SOCIAL_NAMESPACE), had))
        .await
    {
        Ok(paths) => v1::heads(paths),
        Err(e) => return Ok((Caught::stale(read_outcome(s, key, e)?), HashMap::new())),
    };
    let public = match work
        .call(b.list(
            key,
            &v1::posts_dir(v1::Root::Pub, v1::SOCIAL_NAMESPACE),
            had_public,
        ))
        .await
    {
        Ok(paths) => v1::heads(paths),
        Err(e) => return Ok((Caught::stale(read_outcome(s, key, e)?), HashMap::new())),
    };
    let heads: std::collections::HashSet<&str> = private
        .values()
        .chain(public.values())
        .map(|h| h.path.as_str())
        .collect();
    sh.other.retain(|p| heads.contains(p.as_str()));
    sh.too_big.retain(|t| heads.contains(t.path.as_str()));
    let mut ids: Vec<String> = private.keys().cloned().collect();
    ids.extend(sh.items.iter().map(|k| k.id.clone()));
    ids.sort();
    ids.dedup();
    let pending: HashMap<String, usize> = sh
        .items
        .iter()
        .filter(|k| k.pending)
        .enumerate()
        .map(|(i, k)| (k.id.clone(), i))
        .collect();
    let turn = sh
        .items
        .iter()
        .find(|k| k.pending && Some(k.id.as_str()) != first)
        .map(|k| k.id.clone());
    ids.sort_by(|a, b| {
        (Some(b.as_str()) == first)
            .cmp(&(Some(a.as_str()) == first))
            // Give the stored pending queue a turn before discovery can spend every read.
            // Then leave room for discovery, with the remaining pending work in queue order.
            .then_with(|| (Some(b) == turn.as_ref()).cmp(&(Some(a) == turn.as_ref())))
            .then_with(|| match (pending.get(a), pending.get(b)) {
                (Some(a), Some(b)) => a.cmp(b),
                (Some(_), None) => std::cmp::Ordering::Greater,
                (None, Some(_)) => std::cmp::Ordering::Less,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| {
                private
                    .get(b)
                    .map(|h| &h.edit_id)
                    .cmp(&private.get(a).map(|h| &h.edit_id))
            })
    });
    let mut caught = Caught::default();
    let mut remotes = HashMap::new();
    for id in ids {
        if !work.available() || (pending.contains_key(&id) && work.reads == 0) {
            caught.stale = true;
            break;
        }
        let remote = Remote {
            private: private.get(&id).cloned(),
            public: public.get(&id).cloned(),
        };
        if pending.contains_key(&id) {
            rotate_pending(s, key, sh, &id)?;
        }
        // Work::read charges actual downloads. Known held heads cost no read allowance,
        // so unrelated or unrepresentable posts cannot starve older valid collections.
        let mut one = observe(s, b, key, sh, &id, &remote, Some(id.as_str()) == first, work).await?;
        if !editing && !one.stale && !personal::is_lost(s, key) {
            replay(s, b, key, sh, &id, &remote, &mut one, work).await?;
        }
        let stopped = one.stale || personal::is_lost(s, key);
        caught.stale |= one.stale;
        caught.held_back.extend(one.held_back);
        remotes.insert(id, remote);
        if stopped {
            break;
        }
    }
    save(s, key, sh)?;
    Ok((caught, remotes))
}

fn rotate_pending(s: &NodeState, key: &str, sh: &mut Shelf, id: &str) -> Result<(), ApiError> {
    // The stored order is a round-robin queue (the UI sorts by playlist timestamps).
    // Move an attempted entry before awaiting: budget exhaustion or cancellation must
    // leave the next request a new starting point. Reordering adds no shelf metadata.
    let i = sh.items.iter().position(|k| k.id == id).expect("pending item");
    let k = sh.items.remove(i);
    sh.items.push(k);
    save(s, key, sh)
}

async fn drain_one_by_one(
    s: &NodeState,
    b: &dyn PersonalHomeserver,
    key: &str,
    sh: &mut Shelf,
    skip: Option<&str>,
    work: &mut Work,
) -> Result<(), ApiError> {
    let ids: Vec<String> = sh
        .items
        .iter()
        .filter(|k| k.pending && Some(k.id.as_str()) != skip)
        .map(|k| k.id.clone())
        .take(READS_PER_REQUEST)
        .collect();
    for id in ids {
        if !work.available() || work.reads == 0 {
            sh.behind = true;
            break;
        }
        rotate_pending(s, key, sh, &id)?;
        let (mut caught, remote) = catch_up_one(s, b, key, sh, &id, work).await?;
        if caught.stale || personal::is_lost(s, key) {
            break;
        }
        replay(s, b, key, sh, &id, &remote, &mut caught, work).await?;
        if caught.stale || personal::is_lost(s, key) {
            break;
        }
    }
    Ok(())
}

/// The display projection is editable only when it is lossless. Validate the ORIGINAL post,
/// then compare its full JSON (including opaque members and the parsed content envelope) to
/// what the node would write. Notes, unsupported URIs/layouts and unknown extensions are held.
fn their_collection(bytes: &[u8], key: &str, id: &str, edit_id: &str, root: v1::Root) -> Option<Playlist> {
    if bytes.len() > MAX_COLLECTION_BYTES {
        return None;
    }
    let mut original: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let post: pubky_social_specs::PubkySocialPost = serde_json::from_value(original.clone()).ok()?;
    let owner = v1::PubkyId::try_from(key).ok()?;
    v1::validate_social(&post, id, root, &owner).ok()?;
    let MusicContent::Playlist(p) = v1::from_social(&post, id, edit_id)? else {
        return None;
    };
    let p = checked(id, p).ok()?;
    let mut encoded = v1::to_v1(&MusicContent::Playlist(p.clone())).ok()?.to_json();
    original["content"] = serde_json::from_str(original["content"].as_str()?).ok()?;
    encoded["content"] = serde_json::from_str(encoded["content"].as_str()?).ok()?;
    (original == encoded).then_some(p)
}

/// A collection as the node keeps one, or 422 with why not.
fn checked(id: &str, p: Playlist) -> Result<Playlist, ApiError> {
    let post = MusicPost::new(MusicContent::Playlist(p));
    post.validate(Some(id))
        .map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    match post.content {
        MusicContent::Playlist(p) => Ok(p),
        _ => unreachable!("a playlist stays a playlist"),
    }
}

/// Library songs as collection items; a song not identified yet can't be in a listener's
/// collection (node-only songs are the owner's).
fn identified_tracks(lib: &Library, ids: &[i64]) -> Result<Vec<pm_pubky::PlaylistItem>, ApiError> {
    let (items, local) = playlists::split_tracks(lib, ids)?;
    if !local.is_empty() {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "{} not matched to a song yet, so {} can't go in your collection",
                crate::plural(local.len() as u64, "library song"),
                if local.len() == 1 { "it" } else { "they" }
            ),
        ));
    }
    Ok(items)
}

/// A listener's key, when the request is a listener's.
fn listener(c: &Caller) -> Option<String> {
    (c.role == Role::Listener)
        .then_some(c.key.as_deref())
        .flatten()
        .map(|k| k.trim_start_matches("pubky").to_string())
}

/// Their copy, refreshed from their homeserver when it answers; `None` when they haven't
/// connected it (theirs aren't known here then).
async fn read(s: &NodeState, key: &str) -> Result<Option<Shelf>, ApiError> {
    let b = personal::backend(s);
    if !b.connected(key) {
        return Ok(None);
    }
    let mut sh = load(s, key)?;
    bound_existing(&mut sh, None)?;
    let mut work = Work::new();
    if !personal::is_lost(s, key) {
        let (caught, _) = refresh(s, b.as_ref(), key, &mut sh, None, false, &mut work).await?;
        sh.behind = caught.stale;
        if caught.stale && !personal::is_lost(s, key) {
            drain_one_by_one(s, b.as_ref(), key, &mut sh, None, &mut work).await?;
        }
        save(s, key, &sh)?;
    }
    Ok(Some(sh))
}

fn not_found(id: &str) -> ApiError {
    ApiError::not_found(format!("no collection {id}"))
}

/// One change to one of their collections (`id` `None`: a new one), written to their homeserver
/// at once, or kept to be written when it answers again.
async fn change(
    s: &NodeState,
    key: &str,
    id: Option<&str>,
    edit: impl FnOnce(&Library, Option<Kept>) -> Result<Kept, ApiError>,
) -> Result<Response, ApiError> {
    let b = personal::backend(s);
    let lock = personal::person_lock(key);
    let _one = lock.lock().await;
    if !b.connected(key) {
        return Ok(personal::needs_connect(false));
    }
    if personal::is_lost(s, key) {
        return Ok(personal::needs_connect(true));
    }
    let mut sh = load(s, key)?;
    let mut bounded = sh.clone();
    let recovering = bound_existing(&mut bounded, id).is_err();
    if !recovering {
        sh = bounded;
    }
    let mut work = Work::new();
    // The target's old queued publication must never precede this request's newer intent.
    // Refresh validates it but replays nothing until the new edit has been saved durably.
    // Protected legacy intent may itself exceed today's budget. Let an explicit reducing
    // edit build a bounded candidate first, without losing other intent or held markers.
    // No remote operation starts until a later request can validate that saved candidate.
    let (mut caught, mut remotes) = if recovering {
        (Caught::stale(true), HashMap::new())
    } else {
        refresh(s, b.as_ref(), key, &mut sh, id, true, &mut work).await?
    };
    sh.behind = caught.stale;
    if let (true, Some(id)) = (caught.stale, id) {
        if !recovering && !personal::is_lost(s, key) && work.available() {
            let (one, remote) = catch_up_one(s, b.as_ref(), key, &mut sh, id, &mut work).await?;
            caught.stale = one.stale;
            caught.held_back.extend(one.held_back);
            remotes.insert(id.to_string(), remote);
        }
    }
    if !recovering {
        save(s, key, &sh)?;
    }
    if personal::is_lost(s, key) {
        return Ok(personal::needs_connect(true));
    }
    if id.is_none() && sh.items.iter().filter(|k| !k.deleted).count() >= MAX_COLLECTIONS {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("You have {MAX_COLLECTIONS} collections here: delete one first"),
        ));
    }
    let old = match id {
        Some(id) => Some(
            sh.items
                .iter()
                .find(|k| k.id == id && !k.deleted)
                .cloned()
                .ok_or_else(|| not_found(id))?,
        ),
        None => None,
    };
    let mut k = {
        let lib = s.library.lock().expect("library poisoned");
        edit(&lib, old.clone())?
    };
    let mut held = caught
        .held_back
        .get(&k.id)
        .copied()
        .or_else(|| old.as_ref().and_then(|k| known_unread(&sh.other, &sh.too_big, k)));
    if !k.deleted && !k.unshare {
        if let Some(why) = held {
            let message = match why {
                Held::Size => format!("This collection has a newer version from another app that would take this node past the {} MiB of collections it keeps for you: delete or shorten others here and try again, or change it there", MAX_SHELF_BYTES >> 20),
                Held::Unreadable => "This collection has a version from another app that this node can't read or safely rewrite (over 1 MiB, invalid, or containing data it cannot preserve): change it there. Deleting it or making it private here still works".to_string(),
            };
            return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, message));
        }
    }
    if !k.deleted && !k.unshare && post_size(&k.playlist) > MAX_COLLECTION_BYTES {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "This collection would be over {} MiB: make it smaller, or split it",
                MAX_COLLECTION_BYTES >> 20
            ),
        ));
    }
    k.pending = true;
    // This request supersedes the complete previous edit, including its share.
    k.writing = None;
    // A delete no longer needs its cached payload; this also makes space for durable intent
    // at a full shelf. Remote history is discovered again before every deletion batch.
    if k.deleted {
        k.playlist = Playlist::new(String::new(), None, None, Vec::new());
        k.versions.clear();
    }
    if !k.deleted && held.is_none() {
        prepare(key, &mut k).map_err(|e| {
            ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("Your homeserver won't keep this collection: {e}"),
            )
        })?;
    }
    if k.unshare {
        let mut candidate = sh.clone();
        if replace(&mut candidate, k.clone()).is_err() {
            // At a full shelf, retain the smaller privacy intent without reserving a new
            // draft. Published versions can be removed now; the write resumes with room.
            k.writing = None;
            k.versions = old
                .as_ref()
                .map(|k| {
                    k.versions
                        .iter()
                        .filter(|p| !p.starts_with("/pub/"))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            held = Some(Held::Size);
        }
    }
    replace(&mut sh, k.clone())?;
    save(s, key, &sh)?;
    let remote = remotes.remove(&k.id).unwrap_or_default();
    // An existing collection waits if it couldn't be listed/read first. A new collection has
    // no unseen head to overwrite. All remaining work uses the SAME deadline/call allowance.
    if !recovering && (!caught.stale || id.is_none()) {
        match push(s, b.as_ref(), key, &mut sh, &k.id, &remote, held, &mut work).await? {
            Pushed::Done | Pushed::Held => {}
            Pushed::Full if k.unshare => {} // The privacy intent remains durable for the next batch.
            Pushed::Full => return Err(shelf_full()),
            Pushed::Waiting(PersonalError::NoGrant) => {
                // This edit was already accepted durably. Return its pending result below:
                // a reconnect response would make apiSend apply the same command again.
                sh.behind = true;
            }
            Pushed::Waiting(PersonalError::Refused(m)) => {
                personal::mark_lost(s, key, &m)?;
            }
            Pushed::Waiting(PersonalError::Rejected(m)) => {
                if !k.deleted && !k.unshare {
                    discard_rejected(&mut sh, &k.id);
                }
                save(s, key, &sh)?;
                return Err(ApiError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!("Your homeserver won't keep this collection: {m}"),
                ));
            }
            Pushed::Waiting(PersonalError::Unreachable(_)) => {
                sh.behind = true;
            }
        }
    }
    if !recovering && work.available() && !personal::is_lost(s, key) {
        drain_one_by_one(s, b.as_ref(), key, &mut sh, Some(&k.id), &mut work).await?;
    }
    save(s, key, &sh)?;
    if k.deleted {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let k = sh
        .items
        .iter()
        .find(|x| x.id == k.id)
        .expect("the edited collection is kept");
    let lib = s.library.lock().expect("library poisoned");
    Ok(Json(playlists::detail_of(
        &lib,
        &k.id,
        &k.playlist,
        k.public,
        !k.pending,
    ))
    .into_response())
}

/// An edit of an existing collection: `f` changes its playlist, which is then checked.
fn edited<'a>(
    id: &'a str,
    f: impl FnOnce(&Library, &mut Playlist) -> Result<(), ApiError> + 'a,
) -> impl FnOnce(&Library, Option<Kept>) -> Result<Kept, ApiError> + 'a {
    move |lib, k| {
        let mut k = k.ok_or_else(|| not_found(id))?;
        f(lib, &mut k.playlist)?;
        k.unshare = false;
        k.playlist.touch();
        k.playlist = checked(id, k.playlist)?;
        Ok(k)
    }
}

/// `GET /v1/collections`: the owner's, or a listener's own (none before they connect their homeserver).
pub(crate) async fn list(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
) -> ApiResult<Vec<PlaylistSummary>> {
    let Some(key) = listener(&c) else {
        return playlists::list(State(s)).await;
    };
    let lock = personal::person_lock(&key);
    let _one = lock.lock().await;
    let Some(sh) = read(&s, &key).await? else {
        return Ok(Json(Vec::new()));
    };
    let lib = s.library.lock().expect("library poisoned");
    let mut out: Vec<PlaylistSummary> = sh
        .items
        .iter()
        .filter(|k| !k.deleted)
        .map(|k| playlists::own_summary(&lib, &k.id, &k.playlist, k.public, !k.pending))
        .collect();
    out.sort_by_key(|p| std::cmp::Reverse(p.updated_at));
    Ok(Json(out))
}

/// `GET /v1/collections/{id}`.
pub(crate) async fn get(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    Path(id): Path<String>,
) -> ApiResult<PlaylistDetail> {
    let Some(key) = listener(&c) else {
        return playlists::get(State(s), Path(id)).await;
    };
    let lock = personal::person_lock(&key);
    let _one = lock.lock().await;
    let sh = read(&s, &key).await?.ok_or_else(|| not_found(&id))?;
    let k = sh
        .items
        .iter()
        .find(|k| k.id == id && !k.deleted)
        .ok_or_else(|| not_found(&id))?;
    let lib = s.library.lock().expect("library poisoned");
    Ok(Json(playlists::detail_of(
        &lib,
        &k.id,
        &k.playlist,
        k.public,
        !k.pending,
    )))
}

/// `POST /v1/collections`.
pub(crate) async fn create(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    Json(body): Json<CreatePlaylist>,
) -> Result<Response, ApiError> {
    let Some(key) = listener(&c) else {
        return Ok(playlists::create(State(s), Json(body)).await?.into_response());
    };
    change(&s, &key, None, |lib, _| {
        // Where a song plays from (pins) is the owner's to set: a listener's `sources` are left out
        let mut items = playlists::parse_uris(&body.uris)?;
        items.extend(identified_tracks(lib, &body.track_ids)?);
        let mut p = Playlist::new(body.name, body.description, body.image, items);
        p.layout = match &body.layout {
            Some(l) => playlists::checked_layout(l)?,
            None => None,
        };
        p.original = match &body.original {
            Some(o) => Some(
                pm_pubky::schemas::post_uri(o)
                    .ok_or_else(|| ApiError::bad_request(format!("{o}: not a public Pubky post")))?,
            ),
            None => None,
        };
        let id = MusicPost::new(MusicContent::Playlist(p.clone())).create_id();
        Ok(Kept {
            playlist: checked(&id, p)?,
            id,
            public: body.public == Some(true),
            head: String::new(),
            versions: Vec::new(),
            pending: true,
            deleted: false,
            unshare: false,
            writing: None,
        })
    })
    .await
}

/// `PUT /v1/collections/{id}`: name, description, cover, layout, or the items in a new order.
pub(crate) async fn update(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    Path(id): Path<String>,
    Json(body): Json<UpdatePlaylist>,
) -> Result<Response, ApiError> {
    let Some(key) = listener(&c) else {
        return Ok(playlists::update(State(s), Path(id), Json(body))
            .await?
            .into_response());
    };
    let edit = edited(&id, |_, p| {
        if let Some(n) = body.name {
            p.name = n;
        }
        if let Some(d) = body.description {
            p.description = Some(d).filter(|d| !d.trim().is_empty());
        }
        if let Some(i) = body.image {
            p.image = Some(i).filter(|i| !i.trim().is_empty());
        }
        if let Some(l) = body.layout {
            p.layout = playlists::checked_layout(&l)?;
        }
        if let Some(uris) = body.uris {
            // Items that stay keep when they were added and their pin
            let mut old = std::mem::take(&mut p.items);
            p.items = playlists::parse_uris(&uris)?
                .into_iter()
                .map(|mut it| {
                    if let Some(k) = old.iter().position(|o| o.uri == it.uri) {
                        let prev = old.remove(k);
                        it.added_at = prev.added_at;
                        it.source = prev.source;
                    }
                    it
                })
                .collect();
        }
        Ok(())
    });
    change(&s, &key, Some(&id), edit).await
}

/// `POST /v1/collections/{id}/items`.
pub(crate) async fn add_items(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    Path(id): Path<String>,
    Json(body): Json<AddPlaylistItems>,
) -> Result<Response, ApiError> {
    let Some(key) = listener(&c) else {
        return Ok(playlists::add_items(State(s), Path(id), Json(body))
            .await?
            .into_response());
    };
    let edit = edited(&id, |lib, p| {
        let mut items = playlists::parse_uris(&body.uris)?;
        items.extend(identified_tracks(lib, &body.track_ids)?);
        if items.is_empty() {
            return Err(ApiError::bad_request("nothing to add"));
        }
        let at = body.at.unwrap_or(p.items.len()).min(p.items.len());
        p.items.splice(at..at, items);
        Ok(())
    });
    change(&s, &key, Some(&id), edit).await
}

/// `DELETE /v1/collections/{id}/items/{index}`.
pub(crate) async fn remove_item(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    Path((id, index)): Path<(String, usize)>,
) -> Result<Response, ApiError> {
    let Some(key) = listener(&c) else {
        return Ok(playlists::remove_item(State(s), Path((id, index)))
            .await?
            .into_response());
    };
    let edit = edited(&id, |_, p| {
        if index >= p.items.len() {
            return Err(ApiError::not_found(format!("no item {index}")));
        }
        p.items.remove(index);
        Ok(())
    });
    change(&s, &key, Some(&id), edit).await
}

/// `PUT /v1/collections/{id}/visibility {public}`: published on their homeserver, or private again.
pub(crate) async fn set_visibility(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    Path(id): Path<String>,
    Json(body): Json<SetVisibility>,
) -> Result<Response, ApiError> {
    let Some(key) = listener(&c) else {
        return Ok(playlists::set_visibility(State(s), Path(id), Json(body))
            .await?
            .into_response());
    };
    let public = body.public;
    change(&s, &key, Some(&id), |_, k| {
        let mut k = k.ok_or_else(|| not_found(&id))?;
        k.public = public;
        k.unshare = !public;
        k.playlist.touch();
        Ok(k)
    })
    .await
}

/// `DELETE /v1/collections/{id}`: from their homeserver too, every version.
pub(crate) async fn delete(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let Some(key) = listener(&c) else {
        return Ok(playlists::delete(State(s), Path(id)).await?.into_response());
    };
    change(&s, &key, Some(&id), |_, k| {
        let mut k = k.ok_or_else(|| not_found(&id))?;
        k.deleted = true;
        Ok(k)
    })
    .await
}

/// `POST /v1/collections/{id}/play {zone, replace?}`: a listener's queues what the library has
/// of it, on a zone open to them.
pub(crate) async fn play(
    State(s): State<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    Path(id): Path<String>,
    Json(body): Json<PlayPlaylist>,
) -> ApiResult<PlayedPlaylist> {
    let Some(key) = listener(&c) else {
        return playlists::play(State(s), Path(id), Json(body)).await;
    };
    if !crate::accounts::may_use_zone(&s, &c, &body.zone) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "the owner has not opened that zone to listeners",
        ));
    }
    let zone = s
        .zones
        .get(&pm_zones::ZoneId::new(&body.zone))
        .ok_or_else(|| ApiError::not_found(format!("no zone {}", body.zone)))?;
    if !personal::backend(&s).connected(&key) {
        return Err(not_found(&id));
    }
    let sh = load(&s, &key)?;
    let k = sh
        .items
        .iter()
        .find(|k| k.id == id && !k.deleted)
        .ok_or_else(|| not_found(&id))?;
    let (items, missing) =
        playlists::library_queue(&s.library.lock().expect("library poisoned"), &k.playlist);
    if items.is_empty() {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "none of the collection's items is in this node's library",
        ));
    }
    crate::listens::zone_changed_by(&s, &body.zone, false);
    let queued = items.len();
    playlists::queue_into(&zone, items, body.replace);
    Ok(Json(PlayedPlaylist { queued, missing }))
}

/// Whether a listener's request is one of the routes above, for their own collections
/// (accounts.rs lets exactly these through).
pub(crate) fn is_own_route(method: &axum::http::Method, path: &str) -> bool {
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
        (&Method::GET | &Method::PUT | &Method::DELETE, [id]) => !id.is_empty() && !RESERVED.contains(id),
        (&Method::POST, [id, "items"]) | (&Method::POST, [id, "play"]) => !id.is_empty(),
        (&Method::DELETE, [id, "items", index]) => !id.is_empty() && index.parse::<usize>().is_ok(),
        (&Method::PUT, [id, "visibility"]) => !id.is_empty(),
        _ => false,
    }
}

/// Names under `/playlists/` that aren't collections.
const RESERVED: &[&str] = &["from-queue", "import"];

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Method;

    #[test]
    fn a_listener_reaches_only_their_own_collection_routes() {
        for (m, p) in [
            (Method::GET, "/collections"),
            (Method::POST, "/collections"),
            (Method::GET, "/playlists"),
            (Method::GET, "/collections/0034A0Z1Y2X3W"),
            (Method::PUT, "/collections/0034A0Z1Y2X3W"),
            (Method::DELETE, "/playlists/0034A0Z1Y2X3W"),
            (Method::POST, "/collections/0034A0Z1Y2X3W/items"),
            (Method::DELETE, "/collections/0034A0Z1Y2X3W/items/3"),
            (Method::PUT, "/collections/0034A0Z1Y2X3W/visibility"),
            (Method::POST, "/collections/0034A0Z1Y2X3W/play"),
        ] {
            assert!(is_own_route(&m, p), "{m} {p}");
        }
        for (m, p) in [
            (Method::POST, "/playlists/from-queue"),
            (Method::POST, "/playlists/import"),
            (Method::POST, "/playlists/import/file"),
            (Method::GET, "/collections/x/play-from"),
            (Method::PUT, "/collections/x/order"),
            (Method::PUT, "/collections/x/items/2/source"),
            (Method::DELETE, "/collections/x/local/0"),
            (Method::DELETE, "/collections/x/items/two"),
            (Method::DELETE, "/collections"),
            (Method::GET, "/collectionsx"),
            (Method::GET, "/collections/"),
        ] {
            assert!(!is_own_route(&m, p), "{m} {p}");
        }
    }
}
