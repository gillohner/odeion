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
use std::sync::Arc;

/// At most this many of their collections are downloaded per request (a first connect with
/// many); the rest come with the next.
const READS_PER_REQUEST: usize = 16;

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
        .is_some_and(|t| bytes_of(sh).saturating_sub(was).saturating_add(t.bytes) <= MAX_SHELF_BYTES)
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
        v1::parse_version(path).is_some_and(|(id, edit)| id == k.id && edit.as_bytes() > k.head.as_bytes())
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
    /// Every version path of it on their homeserver that the node knows: drafts and published copies.
    #[serde(default)]
    versions: Vec<String>,
    /// Changed here and not on their homeserver yet.
    #[serde(default)]
    pending: bool,
    /// Deleted here: its versions go at the next write.
    #[serde(default)]
    deleted: bool,
}

fn shelf_key(key: &str) -> String {
    format!("{PERSON_PREFIX}{}.collections", key.trim_start_matches("pubky"))
}

fn load(s: &NodeState, key: &str) -> Shelf {
    s.library
        .lock()
        .expect("library poisoned")
        .get_setting(&shelf_key(key))
        .ok()
        .flatten()
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default()
}

fn save(s: &NodeState, key: &str, sh: &Shelf) -> Result<(), ApiError> {
    let lib = s.library.lock().expect("library poisoned");
    // Removed (People › Remove) while this request waited on their homeserver: nothing of
    // theirs comes back. Checked under the same hold as the write, so a removal can't land
    // between the two
    if !crate::people::listed_for_music(&lib, key.trim_start_matches("pubky")) {
        return Ok(());
    }
    lib.set_setting(&shelf_key(key), &serde_json::to_string(sh).expect("serializes"))?;
    Ok(())
}

/// How many of their collections wait to be written (`GET /v1/me/homeserver`).
pub(crate) fn pending(s: &NodeState, key: &str) -> usize {
    load(s, key).items.iter().filter(|k| k.pending).count()
}

/// Whether the node's last read of their collections was incomplete (`GET /v1/me/homeserver`).
pub(crate) fn behind(s: &NodeState, key: &str) -> bool {
    load(s, key).behind
}

fn rejected(e: impl std::fmt::Display) -> PersonalError {
    PersonalError::Rejected(e.to_string())
}

/// Writes `k` as the node has it: a delete removes every version; anything else becomes a new
/// private version, also published when public (else every published copy goes). The versions
/// written are recorded as they land, so a failure part way loses none of them.
async fn push(b: &dyn PersonalHomeserver, key: &str, k: &mut Kept) -> Result<(), PersonalError> {
    if k.deleted {
        while let Some(path) = k.versions.last().cloned() {
            b.remove(key, &path).await?;
            k.versions.pop();
        }
        return Ok(());
    }
    let owner = v1::PubkyId::try_from(key).map_err(rejected)?;
    let v1::V1Post::Social(post) =
        v1::to_v1(&MusicContent::Playlist(k.playlist.clone())).map_err(rejected)?
    else {
        return Err(rejected("not a collection"));
    };
    let head = (!k.head.is_empty()).then_some(k.head.as_str());
    let path = v1::V1Post::Social(post.clone())
        .version_path(&k.id, head, v1::Root::Priv, &owner)
        .map_err(PersonalError::Rejected)?;
    let (_, edit_id) = v1::parse_version(&path).ok_or_else(|| rejected("not a version path"))?;
    b.write(key, &path, &serde_json::to_value(&post).map_err(rejected)?)
        .await?;
    k.versions.push(path);
    k.head = edit_id.clone();
    if k.public {
        let (dest, body) =
            v1::publish_plan(&post, &k.id, &edit_id, &owner).map_err(PersonalError::Rejected)?;
        b.write(key, &dest, &body).await?;
        k.versions.push(dest);
    } else {
        let published: Vec<String> = k
            .versions
            .iter()
            .filter(|p| p.starts_with("/pub/"))
            .cloned()
            .collect();
        for path in published {
            b.remove(key, &path).await?;
            k.versions.retain(|p| p != &path);
        }
    }
    Ok(())
}

/// What a homeserver answer means for a read: `Ok(true)` it didn't answer (the copy is stale),
/// a refusal marks the grant lost.
fn read_outcome(s: &NodeState, key: &str, e: PersonalError) -> Result<bool, ApiError> {
    match e {
        PersonalError::Refused(m) => {
            personal::mark_lost(s, key, &m)?;
            Ok(false)
        }
        _ => Ok(true),
    }
}

/// Writes what waited, then takes what their homeserver has that the copy doesn't, `first` (the
/// collection about to be changed) before any other. Answers whether their homeserver didn't
/// answer (the copy is what the node had). A listing it can't make whole changes nothing.
async fn refresh(
    s: &NodeState,
    b: &dyn PersonalHomeserver,
    key: &str,
    sh: &mut Shelf,
    first: Option<&str>,
) -> Result<Caught, ApiError> {
    // What the node knows there: an empty listing then is checked again (a gone user lists as empty)
    let had = sh.items.iter().any(|k| !k.pending && !k.versions.is_empty());
    let had_public = sh.items.iter().any(|k| !k.pending && k.public);
    let mut private = match b
        .list(key, &v1::posts_dir(v1::Root::Priv, v1::SOCIAL_NAMESPACE), had)
        .await
    {
        Ok(paths) => v1::heads(paths),
        Err(e) => return read_outcome(s, key, e).map(Caught::stale),
    };
    let mut public = match b
        .list(
            key,
            &v1::posts_dir(v1::Root::Pub, v1::SOCIAL_NAMESPACE),
            had_public,
        )
        .await
    {
        Ok(paths) => v1::heads(paths),
        Err(e) => return read_outcome(s, key, e).map(Caught::stale),
    };
    // What waited is written over every version there is (another device's too: a delete or
    // "private again" removes those as well)
    let mut written = std::collections::HashSet::new();
    for k in sh.items.iter_mut().filter(|k| k.pending) {
        // A version the node knows it couldn't read or keep is newer: what waits here would go
        // over it unread, so it keeps waiting (its versions are still followed; a delete goes)
        let held = !k.deleted && known_unread(&sh.other, &sh.too_big, k).is_some();
        for h in [private.get(&k.id), public.get(&k.id)].into_iter().flatten() {
            for v in &h.versions {
                if !k.versions.contains(v) {
                    k.versions.push(v.clone());
                }
            }
            // Above every version there (another device's, even one dated ahead of this
            // node's clock), so the change waiting here is the newest
            if !held && h.edit_id.as_bytes() > k.head.as_bytes() {
                k.head = h.edit_id.clone();
            }
        }
        if held {
            continue;
        }
        match push(b, key, k).await {
            Ok(()) => {
                k.pending = false;
                written.insert(k.id.clone());
                if k.deleted {
                    private.remove(&k.id);
                    public.remove(&k.id);
                }
            }
            // Never taken: dropped. What their homeserver has comes back below (the head is
            // read again); one it never had goes
            Err(PersonalError::Rejected(m)) => {
                tracing::warn!(%key, id = %k.id, "a listener's homeserver won't take a waiting collection; dropped: {m}");
                k.pending = false;
                k.head.clear();
            }
            Err(e) => return read_outcome(s, key, e).map(Caught::stale),
        }
    }
    sh.items.retain(|k| !(k.deleted && !k.pending));
    // Deleted on another device: gone here too, unless changed or written here since. A public
    // post is theirs only as the published copy of a draft (the rest of /pub/social/v1/ is their
    // other posts)
    sh.items
        .retain(|k| k.pending || written.contains(&k.id) || private.contains_key(&k.id));
    let heads: std::collections::HashSet<&str> = private.values().map(|h| h.path.as_str()).collect();
    sh.other.retain(|p| heads.contains(p.as_str()));
    sh.too_big.retain(|t| heads.contains(t.path.as_str()));
    let other: std::collections::HashSet<String> = sh.other.iter().cloned().collect();
    let too_big: std::collections::HashSet<String> = sh.too_big.iter().map(|t| t.path.clone()).collect();
    let mut caught = Caught::default();
    let size_now = bytes_of(&*sh);

    let mut wanted: Vec<(&String, &v1::Head)> = Vec::new();
    // New ones are downloaded only while the node keeps fewer than MAX_COLLECTIONS
    let mut room = MAX_COLLECTIONS.saturating_sub(sh.items.len());
    for (id, head) in &private {
        let mut versions = head.versions.clone();
        versions.extend(public.get(id).map(|h| h.versions.clone()).unwrap_or_default());
        match sh.items.iter_mut().find(|k| &k.id == id) {
            // Still waiting (their homeserver stopped answering), or just written: the node's is newer
            Some(k) if k.pending || written.contains(&k.id) => {}
            // Its newest version is no collection the node can hold (another app's, or one it can
            // never read): not read again, but still followed, so a delete reaches it; a change
            // here would write over it unread, so it is refused
            Some(k) if other.contains(&head.path) => {
                k.versions = versions;
                k.public = public.contains_key(id);
                if head.edit_id.as_bytes() > k.head.as_bytes() {
                    caught.held_back.insert(id.clone(), Held::Unreadable);
                }
            }
            Some(k) => {
                k.versions = versions;
                k.public = public.contains_key(id);
                if head.edit_id.as_bytes() > k.head.as_bytes() {
                    // Left unread to stay within MAX_SHELF_BYTES: not downloaded again until it
                    // fits, and a change of it meanwhile is refused rather than written over it
                    if too_big.contains(&head.path) {
                        let was = bytes_of(&*k);
                        let held = sh
                            .too_big
                            .iter()
                            .find(|t| t.path == head.path)
                            .map_or(usize::MAX, |t| t.bytes);
                        if size_now.saturating_sub(was).saturating_add(held) > MAX_SHELF_BYTES {
                            caught.held_back.insert(id.clone(), Held::Size);
                        } else {
                            wanted.push((id, head));
                        }
                    } else {
                        wanted.push((id, head));
                    }
                }
            }
            None if other.contains(&head.path) => {}
            // Left unread for the copy's size: read once it fits (others deleted or shortened)
            None if too_big.contains(&head.path)
                && (room == 0
                    || size_now.saturating_add(
                        sh.too_big
                            .iter()
                            .find(|t| t.path == head.path)
                            .map_or(usize::MAX, |t| t.bytes),
                    ) > MAX_SHELF_BYTES) => {}
            None if room == 0 => {
                tracing::warn!(%key, %id, "a listener has more than {MAX_COLLECTIONS} collections; this one isn't kept on the node");
            }
            None => {
                room -= 1;
                wanted.push((id, head));
            }
        }
    }
    // The newest first: a listener with many other drafts sees their latest collections soonest;
    // the one about to be changed before all
    wanted.sort_by(|a, b| {
        let is_first = |w: &(&String, &v1::Head)| Some(w.0.as_str()) == first;
        is_first(b)
            .cmp(&is_first(a))
            .then_with(|| b.1.edit_id.cmp(&a.1.edit_id))
    });
    let mut size = bytes_of(&*sh);
    for (id, head) in wanted.into_iter().take(READS_PER_REQUEST) {
        let bytes = match b.read(key, &head.path).await {
            Ok(Some(b)) => b,
            // Gone a moment ago
            Ok(None) => continue,
            // It will never be read (too big, or refused for good): left aside like another app's
            // post, so the ones after it still come in
            Err(PersonalError::Rejected(m)) => {
                tracing::warn!(%key, %id, "a listener's collection version can't be read; left aside: {m}");
                set_aside(sh, id, head, &mut caught);
                continue;
            }
            Err(e) => {
                caught.stale = read_outcome(s, key, e)?;
                break;
            }
        };
        let Some(playlist) = their_collection(&bytes, id, &head.edit_id) else {
            set_aside(sh, id, head, &mut caught);
            continue;
        };
        let mut versions = head.versions.clone();
        versions.extend(public.get(id).map(|h| h.versions.clone()).unwrap_or_default());
        let k = Kept {
            id: id.clone(),
            playlist,
            public: public.contains_key(id),
            head: head.edit_id.clone(),
            versions,
            pending: false,
            deleted: false,
        };
        // The node's copy stays within MAX_SHELF_BYTES: what would go past it isn't kept
        let was = sh.items.iter().find(|x| x.id == *id).map_or(0, bytes_of);
        let grown = size.saturating_sub(was).saturating_add(bytes_of(&k));
        if grown > MAX_SHELF_BYTES {
            tracing::warn!(%key, %id, "a listener's collections are over {MAX_SHELF_BYTES} bytes on the node; this version isn't kept");
            sh.too_big.retain(|t| t.path != head.path);
            sh.too_big.push(TooBig {
                path: head.path.clone(),
                bytes: bytes_of(&k),
            });
            caught.held_back.insert(id.clone(), Held::Size);
            continue;
        }
        size = grown;
        sh.too_big.retain(|t| t.path != head.path);
        match sh.items.iter_mut().find(|x| x.id == *id) {
            Some(x) => *x = k,
            None => sh.items.push(k),
        }
    }
    Ok(caught)
}

/// A newest version the node can't hold (another app's post, or one it can never read) is left
/// aside: not downloaded again. When it is a newer version of a collection the node has, that
/// collection is held back: a change here would write over it unread, so it is refused (a
/// delete still goes).
fn set_aside(sh: &mut Shelf, id: &str, head: &v1::Head, caught: &mut Caught) {
    sh.other.push(head.path.clone());
    if sh
        .items
        .iter()
        .any(|k| k.id == id && !k.pending && head.edit_id.as_bytes() > k.head.as_bytes())
    {
        caught.held_back.insert(id.to_string(), Held::Unreadable);
    }
}

/// Catches one collection up from its own folders, which list small even when all of their posts
/// are more than the node lists: its versions, its sharing and its newest version. `stale` when
/// even those couldn't be read; a newer version it leaves unread for the size of the copy is
/// `held_back`.
async fn catch_up_one(
    s: &NodeState,
    b: &dyn PersonalHomeserver,
    key: &str,
    sh: &mut Shelf,
    id: &str,
) -> Result<Caught, ApiError> {
    let mut caught = Caught::default();
    let Some(i) = sh.items.iter().position(|k| k.id == id) else {
        return Ok(caught);
    };
    let dir = |root| format!("{}{id}/", v1::posts_dir(root, v1::SOCIAL_NAMESPACE));
    let (had, had_public) = {
        let k = &sh.items[i];
        (!k.pending && !k.versions.is_empty(), !k.pending && k.public)
    };
    let private = match b.list(key, &dir(v1::Root::Priv), had).await {
        Ok(paths) => v1::heads(paths).remove(id),
        Err(e) => return read_outcome(s, key, e).map(Caught::stale),
    };
    let public = match b.list(key, &dir(v1::Root::Pub), had_public).await {
        Ok(paths) => v1::heads(paths).remove(id),
        Err(e) => return read_outcome(s, key, e).map(Caught::stale),
    };
    let mut versions = private.as_ref().map(|h| h.versions.clone()).unwrap_or_default();
    versions.extend(public.as_ref().map(|h| h.versions.clone()).unwrap_or_default());
    let k = &mut sh.items[i];
    // What waits here goes over every version there, above the newest: unless the node knows one
    // there it couldn't read or keep (then it keeps waiting; a delete still goes)
    if k.pending {
        for v in versions {
            if !k.versions.contains(&v) {
                k.versions.push(v);
            }
        }
        if !k.deleted {
            if let Some(why) = known_unread(&sh.other, &sh.too_big, k) {
                caught.held_back.insert(id.to_string(), why);
                return Ok(caught);
            }
        }
        for h in [&private, &public].into_iter().flatten() {
            if h.edit_id.as_bytes() > k.head.as_bytes() {
                k.head = h.edit_id.clone();
            }
        }
        return Ok(caught);
    }
    let Some(head) = private else {
        // Deleted in another app of theirs: gone here too
        sh.items.remove(i);
        return Ok(caught);
    };
    k.versions = versions;
    k.public = public.is_some();
    if head.edit_id.as_bytes() <= k.head.as_bytes() {
        return Ok(caught);
    }
    // Newer, and one the node can't hold, or left unread for the copy's size (and still too big
    // for it): held back, never written over
    let was = bytes_of(&sh.items[i]);
    if sh.other.contains(&head.path) {
        caught.held_back.insert(id.to_string(), Held::Unreadable);
        return Ok(caught);
    }
    if sh.too_big.iter().any(|t| t.path == head.path) && !fits_now(sh, &head.path, was) {
        caught.held_back.insert(id.to_string(), Held::Size);
        return Ok(caught);
    }
    match b.read(key, &head.path).await {
        Ok(Some(bytes)) => match their_collection(&bytes, id, &head.edit_id) {
            Some(playlist) => {
                let mut newer = sh.items[i].clone();
                newer.playlist = playlist;
                newer.head = head.edit_id.clone();
                let grown = bytes_of(&*sh)
                    .saturating_sub(bytes_of(&sh.items[i]))
                    .saturating_add(bytes_of(&newer));
                sh.too_big.retain(|t| t.path != head.path);
                if grown > MAX_SHELF_BYTES {
                    sh.too_big.push(TooBig {
                        path: head.path.clone(),
                        bytes: bytes_of(&newer),
                    });
                    caught.held_back.insert(id.to_string(), Held::Size);
                } else {
                    sh.items[i] = newer;
                }
            }
            None => set_aside(sh, id, &head, &mut caught),
        },
        // Gone a moment ago: the next read sees it
        Ok(None) => {}
        Err(PersonalError::Rejected(_)) => set_aside(sh, id, &head, &mut caught),
        Err(e) => caught.stale = read_outcome(s, key, e)?,
    }
    Ok(caught)
}

/// After a read that couldn't list all of their posts: what waits goes one collection at a
/// time, each caught up from its own folders first (at most [`READS_PER_REQUEST`] a request).
async fn drain_one_by_one(
    s: &NodeState,
    b: &dyn PersonalHomeserver,
    key: &str,
    sh: &mut Shelf,
) -> Result<(), ApiError> {
    let ids: Vec<String> = sh
        .items
        .iter()
        .filter(|k| k.pending)
        .map(|k| k.id.clone())
        .take(READS_PER_REQUEST)
        .collect();
    for id in ids {
        let caught = catch_up_one(s, b, key, sh, &id).await?;
        if caught.stale || personal::is_lost(s, key) {
            break;
        }
        // It would go over a version the node couldn't read or keep: it keeps waiting
        if caught.held_back.contains_key(&id) {
            continue;
        }
        let Some(i) = sh.items.iter().position(|k| k.id == id) else {
            continue;
        };
        let mut k = sh.items[i].clone();
        match push(b, key, &mut k).await {
            Ok(()) => {
                k.pending = false;
                sh.items[i] = k;
            }
            Err(PersonalError::Rejected(m)) => {
                tracing::warn!(%key, %id, "a listener's homeserver won't take a waiting collection; dropped: {m}");
                if k.versions.is_empty() {
                    // Never on their homeserver (a new one): it goes, rather than showing as kept
                    sh.items.remove(i);
                } else {
                    // What their homeserver has comes back: it is read again now
                    k.pending = false;
                    k.head.clear();
                    sh.items[i] = k;
                    catch_up_one(s, b, key, sh, &id).await?;
                }
            }
            Err(e) => {
                // The versions written before it failed are kept
                sh.items[i] = k;
                read_outcome(s, key, e)?;
                break;
            }
        }
    }
    sh.items.retain(|k| !(k.deleted && !k.pending));
    Ok(())
}

/// One version of a private social post of theirs, as a collection the node can hold: a music
/// collection that passes the node's checks. `None` for anything else.
fn their_collection(bytes: &[u8], id: &str, edit_id: &str) -> Option<Playlist> {
    let post: pubky_social_specs::PubkySocialPost = serde_json::from_slice(bytes).ok()?;
    let MusicContent::Playlist(p) = v1::from_social(&post, id, edit_id)? else {
        return None;
    };
    checked(id, p).ok()
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
    let mut sh = load(s, key);
    if !personal::is_lost(s, key) {
        let before = serde_json::to_string(&sh).expect("serializes");
        let caught = refresh(s, b.as_ref(), key, &mut sh, None).await?;
        sh.behind = caught.stale;
        // All of their posts couldn't be listed: what waits still goes, one collection at a time
        if caught.stale && !personal::is_lost(s, key) {
            drain_one_by_one(s, b.as_ref(), key, &mut sh).await?;
        }
        // Unchanged (the usual read): not written again
        if serde_json::to_string(&sh).expect("serializes") != before {
            save(s, key, &sh)?;
        }
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
    let mut sh = load(s, key);
    // What their homeserver has now (another device's newer version, this one first), so the
    // change starts from it, and a delete or "private again" reaches every version there
    let before = serde_json::to_string(&sh).expect("serializes");
    let mut caught = refresh(s, b.as_ref(), key, &mut sh, id).await?;
    sh.behind = caught.stale;
    // All of their posts couldn't be listed (more than the node lists, say): this one is caught
    // up from its own folders, so changing it doesn't wait on the rest
    if let (true, Some(id)) = (caught.stale, id) {
        if !personal::is_lost(s, key) {
            let one = catch_up_one(s, b.as_ref(), key, &mut sh, id).await?;
            caught.stale = one.stale;
            caught.held_back.extend(one.held_back);
        }
    }
    let stale = caught.stale;
    if serde_json::to_string(&sh).expect("serializes") != before {
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
    let at = match id {
        Some(id) => Some(
            sh.items
                .iter()
                .position(|k| k.id == id && !k.deleted)
                .ok_or_else(|| not_found(id))?,
        ),
        None => None,
    };
    let mut k = {
        let lib = s.library.lock().expect("library poisoned");
        edit(&lib, at.map(|i| sh.items[i].clone()))?
    };
    // A newer version from another app that the node couldn't read or keep (found now, or known
    // from before, also when nothing can be read now): changing it here would write over that
    // version unread, so it is refused (a delete reaches every version, and goes)
    let held = caught
        .held_back
        .get(&k.id)
        .copied()
        .or_else(|| at.and_then(|i| known_unread(&sh.other, &sh.too_big, &sh.items[i])));
    if let (false, Some(why)) = (k.deleted, held) {
        let message = match why {
            Held::Size => format!(
                "This collection has a newer version from another app that would take this node past the {} MiB of collections it keeps for you: delete or shorten others here and try again, or change it there",
                MAX_SHELF_BYTES >> 20
            ),
            Held::Unreadable => "This collection has a newer version from another app that this node can't read (over 1 MiB, or not one it can hold): change it there. Deleting it here still works".to_string(),
        };
        return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, message));
    }
    // Within what the node can read back, and what it keeps for one listener
    if !k.deleted && post_size(&k.playlist) > MAX_COLLECTION_BYTES {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "This collection would be over {} MiB: make it smaller, or split it",
                MAX_COLLECTION_BYTES >> 20
            ),
        ));
    }
    // (A delete, or a change that doesn't grow it, always goes, even over the cap)
    let was = at.map_or(0, |i| bytes_of(&sh.items[i]));
    if !k.deleted
        && bytes_of(&k) > was
        && bytes_of(&sh).saturating_sub(was).saturating_add(bytes_of(&k)) > MAX_SHELF_BYTES
    {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "Your collections on this node would be over {} MiB: delete or shorten some first",
                MAX_SHELF_BYTES >> 20
            ),
        ));
    }
    k.pending = true;
    // Their homeserver didn't let the node catch up (it didn't answer, or its listing was more
    // than the node reads): a change of one they have waits, rather than going out over a newer
    // version it couldn't see. It goes once the node has read what is there.
    if stale && id.is_some() {
        tracing::info!(%key, id = %k.id, "a listener's homeserver couldn't be read first; the change waits");
        match at {
            Some(i) => sh.items[i] = k.clone(),
            None => sh.items.push(k.clone()),
        }
        save(s, key, &sh)?;
        if k.deleted {
            return Ok(StatusCode::NO_CONTENT.into_response());
        }
        let lib = s.library.lock().expect("library poisoned");
        return Ok(Json(playlists::detail_of(&lib, &k.id, &k.playlist, k.public, false)).into_response());
    }
    let before = k.versions.clone();
    let push = push(b.as_ref(), key, &mut k).await;
    // Whether part of it reached their homeserver before it failed
    let landed = k.versions != before;
    match push {
        Ok(()) => k.pending = false,
        Err(PersonalError::NoGrant) => return Ok(personal::needs_connect(false)),
        // Nothing written: approve the node again, then the change is sent again. Once part of it
        // landed (the draft, before its published copy) it is kept and waits, so a resend can't
        // make it twice
        Err(PersonalError::Refused(m)) => {
            personal::mark_lost(s, key, &m)?;
            if !landed {
                return Ok(personal::needs_connect(true));
            }
        }
        Err(PersonalError::Rejected(m)) => {
            if landed {
                // Part of it landed: the node reads what their homeserver has next time
                k.pending = false;
                k.head.clear();
                match at {
                    Some(i) => sh.items[i] = k.clone(),
                    None => sh.items.push(k.clone()),
                }
                save(s, key, &sh)?;
            }
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("Your homeserver won't keep this collection: {m}"),
            ));
        }
        Err(PersonalError::Unreachable(m)) => {
            tracing::info!(%key, id = %k.id, "a listener's homeserver didn't answer; the change waits: {m}");
        }
    }
    match at {
        Some(i) => sh.items[i] = k.clone(),
        None => sh.items.push(k.clone()),
    }
    sh.items.retain(|x| !(x.deleted && !x.pending));
    save(s, key, &sh)?;
    if k.deleted {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
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
        k.playlist.touch();
        k.playlist = checked(id, k.playlist)?;
        Ok(k)
    }
}

// ---- handlers: each caller's own --------------------------------------------------------------

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
    let sh = load(&s, &key);
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
