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
//!   another device) replaces the copy's, and one deleted there goes here too.
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

/// The node's copy of one listener's collections.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Shelf {
    #[serde(default)]
    items: Vec<Kept>,
    /// Newest versions of their private posts that aren't music collections (other drafts of
    /// theirs): not downloaded again.
    #[serde(default)]
    other: Vec<String>,
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
    // Removed (People › Remove) while this request waited on their homeserver: nothing of
    // theirs comes back
    if !crate::people::may_use_music(s, key) {
        return Ok(());
    }
    s.library
        .lock()
        .expect("library poisoned")
        .set_setting(&shelf_key(key), &serde_json::to_string(sh).expect("serializes"))?;
    Ok(())
}

/// How many of their collections wait to be written (`GET /v1/me/homeserver`).
pub(crate) fn pending(s: &NodeState, key: &str) -> usize {
    load(s, key).items.iter().filter(|k| k.pending).count()
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

/// Writes what waited, then takes what their homeserver has that the copy doesn't. Answers
/// whether their homeserver didn't answer (the copy is what the node had).
async fn refresh(
    s: &NodeState,
    b: &dyn PersonalHomeserver,
    key: &str,
    sh: &mut Shelf,
) -> Result<bool, ApiError> {
    for k in sh.items.iter_mut().filter(|k| k.pending) {
        match push(b, key, k).await {
            Ok(()) => k.pending = false,
            // Never taken: dropped (what their homeserver has, if anything, comes back below)
            Err(PersonalError::Rejected(m)) => {
                tracing::warn!(%key, id = %k.id, "a listener's homeserver won't take a waiting collection; dropped: {m}");
                k.pending = false;
            }
            Err(e) => return read_outcome(s, key, e),
        }
    }
    sh.items.retain(|k| !(k.deleted && !k.pending));

    let private = match b
        .list(key, &v1::posts_dir(v1::Root::Priv, v1::SOCIAL_NAMESPACE))
        .await
    {
        Ok(paths) => v1::heads(paths),
        Err(e) => return read_outcome(s, key, e),
    };
    let public = match b
        .list(key, &v1::posts_dir(v1::Root::Pub, v1::SOCIAL_NAMESPACE))
        .await
    {
        Ok(paths) => v1::heads(paths),
        Err(e) => return read_outcome(s, key, e),
    };
    // Deleted on another device: gone here too, unless changed here since. A public post is
    // theirs only as the published copy of a draft (the rest of /pub/social/v1/ is their other posts)
    sh.items.retain(|k| k.pending || private.contains_key(&k.id));
    sh.other.retain(|p| private.values().any(|h| &h.path == p));

    let mut wanted: Vec<(&String, &v1::Head)> = Vec::new();
    for (id, head) in &private {
        let mut versions = head.versions.clone();
        versions.extend(public.get(id).map(|h| h.versions.clone()).unwrap_or_default());
        match sh.items.iter_mut().find(|k| &k.id == id) {
            // Changed here: the copy's is written next
            Some(k) if k.pending => {}
            Some(k) => {
                k.versions = versions;
                k.public = public.contains_key(id);
                if head.edit_id.as_bytes() > k.head.as_bytes() {
                    wanted.push((id, head));
                }
            }
            None if sh.other.contains(&head.path) => {}
            None => wanted.push((id, head)),
        }
    }
    let mut stale = false;
    for (id, head) in wanted.into_iter().take(READS_PER_REQUEST) {
        let bytes = match b.read(key, &head.path).await {
            Ok(Some(b)) => b,
            // Gone a moment ago
            Ok(None) => continue,
            Err(e) => {
                stale = read_outcome(s, key, e)?;
                break;
            }
        };
        let Some(playlist) = their_collection(&bytes, id, &head.edit_id) else {
            sh.other.push(head.path.clone());
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
        match sh.items.iter_mut().find(|x| x.id == *id) {
            Some(x) => *x = k,
            None => sh.items.push(k),
        }
    }
    Ok(stale)
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
        refresh(s, b.as_ref(), key, &mut sh).await?;
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
    let mut sh = load(s, key);
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
    k.pending = true;
    match push(b.as_ref(), key, &mut k).await {
        Ok(()) => k.pending = false,
        Err(PersonalError::NoGrant) => return Ok(personal::needs_connect(false)),
        Err(PersonalError::Refused(m)) => {
            personal::mark_lost(s, key, &m)?;
            return Ok(personal::needs_connect(true));
        }
        Err(PersonalError::Rejected(m)) => {
            return Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                format!("Your homeserver won't keep this collection: {m}"),
            ))
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
fn edited(
    id: &str,
    f: impl FnOnce(&Library, &mut Playlist) -> Result<(), ApiError>,
) -> impl FnOnce(&Library, Option<Kept>) -> Result<Kept, ApiError> + '_ {
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
        let mut items = playlists::parse_uris(&body.uris)?;
        for (it, src) in items.iter_mut().zip(&body.sources) {
            it.source = src.clone();
        }
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
