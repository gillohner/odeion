//! Collections other people share on Pubky (`/pub/social/v1/posts/`, published by any v1 client):
//! list someone's, open one (its items joined to this library, or where each song plays from on
//! this node), play it, or save a private copy. Their data is hostile input:
//! [`pm_pubky::v1::read_public_playlist`] validates it.
//!
//! They are read from the person's homeserver directly, by their key.
// TODO(nexus): list them from our Nexus fork once it indexes collections (a query instead of one
// request per collection), and keep the homeserver read for opening one.

use crate::state::NodeState;
use crate::types::*;
use crate::{ApiError, ApiResult};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use pm_pubky::homeserver::{ListBounds, NodeHomeserver};
use pm_pubky::{v1, Playlist, Visibility};
use std::sync::Arc;

/// At most this many shared collections are listed per person.
const LIST_MAX: usize = 50;
/// At most this many of a person's public posts are read to find them: a person with many
/// notes still shows their collections (the posts are filtered by kind before the cap).
const READ_MAX: usize = 1000;

/// A test-sized shared listing: this many entries when not 0 (the pages follow), so a test
/// needs a few hundred posts, not thousands.
pub static SHARED_ENTRIES_FOR_TESTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// How far someone else's homeserver is listed: their newest posts ([`ListBounds::SHARED`]).
fn bounds() -> ListBounds {
    let b = ListBounds::SHARED;
    match SHARED_ENTRIES_FOR_TESTS.load(std::sync::atomic::Ordering::Relaxed) {
        0 => b,
        n => ListBounds {
            max_entries: n,
            max_pages: n.div_ceil(pm_pubky::homeserver::LIST_LIMIT as usize),
            ..b
        },
    }
}

/// A person (and maybe one post) from what someone pasted: `pubky://<key>/pub/social/v1/posts/<id>`,
/// `pubky<key>` or the bare key.
pub fn parse_link(input: &str) -> Option<(String, Option<String>)> {
    let t = input.trim();
    let rest = t.strip_prefix("pubky://").unwrap_or(t);
    let (key, path) = rest.split_once('/').unwrap_or((rest, ""));
    let key = key.strip_prefix("pubky").filter(|k| k.len() == 52).unwrap_or(key);
    if key.len() != 52 || !key.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    let id = path
        .strip_prefix("pub/social/v1/posts/")
        .map(|p| p.split('/').next().unwrap_or("").to_string())
        .filter(|id| v1::micros_of(id).is_some());
    Some((key.to_ascii_lowercase(), id))
}

/// The author and id of a public post link an item holds (`pubky://<key>/pub/…/posts/<id>`).
pub fn parse_post(uri: &str) -> Option<(String, String)> {
    let rest = uri.strip_prefix("pubky://")?;
    let (key, path) = rest.split_once('/')?;
    let id = path.strip_prefix("pub/")?.rsplit_once("/posts/")?.1;
    (key.len() == 52 && v1::micros_of(id).is_some()).then(|| (key.to_string(), id.to_string()))
}

fn homeserver(s: &NodeState) -> Result<NodeHomeserver, ApiError> {
    match s.sync_handle() {
        Some(h) => Ok(h.homeserver().clone()),
        None => s
            .homeserver()
            .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e.to_string())),
    }
}

fn user_of(raw: &str) -> Result<(String, v1::PubkyId), ApiError> {
    let (key, _) = parse_link(raw).ok_or_else(|| ApiError::bad_request("not a Pubky key or link"))?;
    let owner =
        v1::PubkyId::try_from(key.as_str()).map_err(|e| ApiError::bad_request(format!("{key}: {e}")))?;
    Ok((key, owner))
}

/// Someone's shared collections, newest edit first: their newest public posts (the listing is
/// bounded, [`ListBounds::SHARED`]) are read newest edit first,
/// [`pm_pubky::homeserver::FETCH_AT_ONCE`] at a time, until [`LIST_MAX`] collections are found
/// or [`READ_MAX`] posts were read.
async fn read_all(
    s: &NodeState,
    key: &str,
    owner: &v1::PubkyId,
) -> Result<Found<(String, Playlist)>, ApiError> {
    let hs = homeserver(s)?;
    let (heads, complete) = hs
        .v1_heads(key, v1::Root::Pub, v1::SOCIAL_NAMESPACE, None, bounds())
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, format!("their homeserver: {e}")))?;
    let mut newest: Vec<(String, v1::Head)> = heads.into_iter().collect();
    newest.sort_by(|a, b| b.1.edit_id.cmp(&a.1.edit_id));
    let hs = &hs;
    Ok(gather(newest, complete, |(id, head)| async move {
        read_post(hs, key, owner, &id, &head).await
    })
    .await)
}

/// What reading someone's posts found: the collections, and how many posts couldn't be read
/// (their homeserver didn't answer, or answered with an error) or whether the listing itself
/// was cut, while fewer than [`LIST_MAX`] collections were found.
#[derive(Debug, PartialEq)]
struct Found<T> {
    list: Vec<T>,
    failed: usize,
    cut: bool,
}

/// [`read_all`]'s reading: `posts` newest first (at most [`READ_MAX`] are read), each through
/// `read` (`Err`: not read, `Ok(None)`: not a collection); `complete` is whether the listing was
/// whole.
async fn gather<P, T, F, Fut>(mut posts: Vec<P>, complete: bool, read: F) -> Found<T>
where
    P: Clone,
    F: Fn(P) -> Fut,
    Fut: std::future::Future<Output = Result<Option<T>, ()>>,
{
    let mut cut = !complete || posts.len() > READ_MAX;
    posts.truncate(READ_MAX);
    let mut list = Vec::new();
    let mut failed = 0;
    for batch in posts.chunks(pm_pubky::homeserver::FETCH_AT_ONCE) {
        let read = futures_util::future::join_all(batch.iter().cloned().map(&read)).await;
        for r in read {
            match r {
                Ok(Some(t)) => list.push(t),
                Ok(None) => {}
                Err(()) => failed += 1,
            }
        }
        if list.len() >= LIST_MAX {
            list.truncate(LIST_MAX);
            // Enough found: what wasn't read, or wasn't listed, can't change what is shown
            return Found {
                list,
                failed: 0,
                cut: false,
            };
        }
    }
    cut &= list.len() < LIST_MAX;
    Found { list, failed, cut }
}

/// The answer to a listing of someone's collections. Nothing found while posts couldn't be read
/// is an error (their homeserver didn't answer), not "no shared playlists"; collections found
/// while some posts couldn't be read, or the listing was cut, carry `x-partial: 1`, so the page
/// says the list may miss some.
fn answer<T: serde::Serialize>(found: Found<T>) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    if found.list.is_empty() && found.failed > 0 {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            format!(
                "their homeserver: {} of their posts couldn't be read, so their shared playlists can't be listed",
                found.failed
            ),
        ));
    }
    let partial = found.failed > 0 || found.cut;
    let mut r = Json(found.list).into_response();
    if partial {
        r.headers_mut()
            .insert(PARTIAL, axum::http::HeaderValue::from_static("1"));
    }
    Ok(r)
}

/// Set on a listing that may miss some collections ([`answer`]).
const PARTIAL: &str = "x-partial";

/// One public post: `Ok(Some(_))` when it is a collection the node can read, `Ok(None)` when it
/// is something else, `Err` when it couldn't be read.
async fn read_post(
    hs: &NodeHomeserver,
    key: &str,
    owner: &v1::PubkyId,
    id: &str,
    head: &v1::Head,
) -> Result<Option<(String, Playlist)>, ()> {
    let bytes = hs
        .get_bytes(key, &head.path, Visibility::Public, None)
        .await
        .map_err(|e| tracing::debug!(key, id, "a shared post couldn't be read: {e}"))?;
    Ok(v1::read_public_playlist(&bytes, id, &head.edit_id, owner).map(|p| (id.to_string(), p)))
}

/// One post by id: only that post's folder (`{posts_dir}{id}/`, its versions) is listed, so a
/// post older than the newest ones the bounded account listing keeps still opens.
async fn read_one(s: &NodeState, key: &str, owner: &v1::PubkyId, id: &str) -> Result<Playlist, ApiError> {
    if v1::micros_of(id).is_none() {
        return Err(ApiError::not_found("no shared playlist with that id"));
    }
    let hs = homeserver(s)?;
    let dir = format!("{}{id}/", v1::posts_dir(v1::Root::Pub, v1::SOCIAL_NAMESPACE));
    let listing = hs
        .list_tree(key, &dir, Visibility::Public, None, bounds())
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, format!("their homeserver: {e}")))?;
    let heads = v1::heads(listing.entries);
    let head = heads
        .get(id)
        .ok_or_else(|| ApiError::not_found("no shared playlist with that id (it may be private again)"))?;
    let bytes = hs
        .get_bytes(key, &head.path, Visibility::Public, None)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, format!("their homeserver: {e}")))?;
    v1::read_public_playlist(&bytes, id, &head.edit_id, owner).ok_or_else(|| {
        ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "that post is not a playlist this node can read",
        )
    })
}

/// A public collection's name, for "In collections" on item pages: one read of the author's
/// homeserver; `None` when it can't be read (the row then names the author).
pub(crate) async fn collection_name(s: &NodeState, key: &str, id: &str) -> Option<String> {
    let (key, owner) = user_of(key).ok()?;
    read_one(s, &key, &owner, id).await.ok().map(|p| p.name)
}

fn summary(key: &str, id: &str, p: &Playlist) -> SharedPlaylistSummary {
    SharedPlaylistSummary {
        user: key.to_string(),
        id: id.to_string(),
        link: format!("pubky://{key}/pub/social/v1/posts/{id}"),
        name: p.name.clone(),
        description: p.description.clone(),
        items: p.items.len(),
        updated_at: p.updated_at,
        layout: p.layout.clone().unwrap_or_else(|| "list".into()),
        tracks_only: p.is_tracks_only(),
    }
}

/// `GET /v1/shared/{user}/collections` (and `/playlists`, the same list): someone's public
/// collections, newest edit first.
pub(crate) async fn list_collections(
    State(s): State<Arc<NodeState>>,
    Path(user): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    list(State(s), Path(user)).await
}

/// `GET /v1/shared/{user}/collections/{id}`: one public collection, every item joined to this
/// library.
pub(crate) async fn get_collection(
    State(s): State<Arc<NodeState>>,
    Path((user, id)): Path<(String, String)>,
) -> ApiResult<SharedCollection> {
    let (key, owner) = user_of(&user)?;
    let p = read_one(&s, &key, &owner, &id).await?;
    let entries = crate::playlists::entries(&s.library.lock().expect("library poisoned"), &p);
    Ok(Json(SharedCollection {
        summary: summary(&key, &id, &p),
        entries,
    }))
}

/// `GET /v1/shared/resolve?link=`: who (and which playlist) a pasted link names.
pub(crate) async fn resolve(Query(q): Query<SharedLink>) -> ApiResult<SharedTarget> {
    let (user, id) =
        parse_link(&q.link).ok_or_else(|| ApiError::bad_request("paste a pubky:// link or a Pubky key"))?;
    Ok(Json(SharedTarget { user, id }))
}

/// `GET /v1/shared/{user}/playlists`: `[SharedPlaylistSummary]`, with `x-partial: 1` when it may
/// miss some ([`answer`]).
pub(crate) async fn list(
    State(s): State<Arc<NodeState>>,
    Path(user): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    let (key, owner) = user_of(&user)?;
    let found = read_all(&s, &key, &owner).await?;
    answer(Found {
        list: found
            .list
            .iter()
            .map(|(id, p)| summary(&key, id, p))
            .collect::<Vec<SharedPlaylistSummary>>(),
        failed: found.failed,
        cut: found.cut,
    })
}

/// `GET /v1/shared/{user}/playlists/{id}`: the playlist and where each song plays from here.
pub(crate) async fn get(
    State(s): State<Arc<NodeState>>,
    Path((user, id)): Path<(String, String)>,
) -> ApiResult<SharedPlaylist> {
    let (key, owner) = user_of(&user)?;
    let p = read_one(&s, &key, &owner, &id).await?;
    let (order, own_order, items) = crate::play_from::resolve_playlist(&s, &p).await;
    let acc = crate::discover::service_access(&s);
    let entries = crate::playlists::entries(&s.library.lock().expect("library poisoned"), &p);
    Ok(Json(SharedPlaylist {
        entries,
        summary: summary(&key, &id, &p),
        play_from: PlaylistPlayFrom {
            order: crate::play_from::play_order_of(order, &acc),
            own_order,
            items: items.into_iter().map(|r| r.view).collect(),
        },
    }))
}

/// `POST /v1/shared/{user}/collections/{id}/duplicate` (and `/playlists/{id}/copy`, the older
/// name): a private collection of your own with the same items, pins, order and layout, which
/// records the original in `ext.music.original`. To keep the original itself (and its changes),
/// bookmark its link instead: `PUT /v1/bookmarks {uri: "pubky://…/posts/<id>"}`.
pub(crate) async fn duplicate(
    State(s): State<Arc<NodeState>>,
    Path((user, id)): Path<(String, String)>,
) -> ApiResult<PlaylistDetail> {
    let (key, owner) = user_of(&user)?;
    let p = read_one(&s, &key, &owner, &id).await?;
    let Json(d) = crate::playlists::create(
        State(s.clone()),
        Json(CreatePlaylist {
            name: p.name.clone(),
            description: p.description.clone(),
            image: None,
            uris: p.items.iter().map(|i| i.uri.clone()).collect(),
            track_ids: Vec::new(),
            sources: p.items.iter().map(|i| i.source.clone()).collect(),
            layout: p.layout.clone(),
            public: None,
            original: Some(format!("pubky://{key}/pub/social/v1/posts/{id}")),
        }),
    )
    .await?;
    if let Some(order) = p.source_order.clone() {
        let Json(d) = crate::playlists::set_order(
            State(s.clone()),
            Path(d.summary.id.clone()),
            Json(SetPlaylistOrder { order: Some(order) }),
        )
        .await?;
        return Ok(Json(d));
    }
    Ok(Json(d))
}

#[cfg(test)]
mod tests {
    use super::{answer, gather, parse_link, Found, LIST_MAX, PARTIAL};
    use axum::http::StatusCode;

    const KEY: &str = "zswn37ir9t8ge4r4posn4r7ow3eh384c8ud4kkxm585igt38x31o";

    #[test]
    fn post_items_name_their_author() {
        assert_eq!(
            super::parse_post(&format!("pubky://{KEY}/pub/social/v1/posts/0035WKVKQDHSM")),
            Some((KEY.into(), "0035WKVKQDHSM".into()))
        );
        assert_eq!(
            super::parse_post(&format!("pubky://{KEY}/pub/music/v1/posts/0035WKVKQDHSM")),
            Some((KEY.into(), "0035WKVKQDHSM".into()))
        );
        assert_eq!(
            super::parse_post(&format!("pubky://{KEY}/priv/social/v1/posts/0035WKVKQDHSM")),
            None
        );
        assert_eq!(super::parse_post("https://musicbrainz.org/release/x"), None);
    }

    #[test]
    fn links_name_a_person_and_maybe_a_playlist() {
        assert_eq!(parse_link(KEY), Some((KEY.into(), None)));
        assert_eq!(parse_link(&format!("pubky{KEY}")), Some((KEY.into(), None)));
        assert_eq!(
            parse_link(&format!(" pubky://{KEY}/pub/social/v1/posts/0035WKVKQDHSM ")),
            Some((KEY.into(), Some("0035WKVKQDHSM".into())))
        );
        assert_eq!(
            parse_link(&format!(
                "pubky://{KEY}/pub/social/v1/posts/0035WKVKQDHSM/0035WKVKQDHSM.json"
            )),
            Some((KEY.into(), Some("0035WKVKQDHSM".into())))
        );
        assert_eq!(
            parse_link(&format!("pubky://{KEY}/pub/social/v1/posts/nope")),
            Some((KEY.into(), None))
        );
        assert_eq!(parse_link("https://example.com"), None);
        assert_eq!(parse_link("short"), None);
    }

    /// Posts read: a collection, something else, or one that couldn't be read.
    async fn read_one_of(p: (usize, char)) -> Result<Option<usize>, ()> {
        match p.1 {
            'c' => Ok(Some(p.0)),
            'o' => Ok(None),
            _ => Err(()),
        }
    }

    fn posts(kinds: &str) -> Vec<(usize, char)> {
        kinds.chars().enumerate().collect()
    }

    #[tokio::test]
    async fn a_listing_that_couldnt_read_every_post_says_so() {
        // Whole, every post read: no flag, even with none found
        let f = gather(posts("coc"), true, read_one_of).await;
        assert_eq!(
            f,
            Found {
                list: vec![0, 2],
                failed: 0,
                cut: false
            }
        );
        let r = answer(f).unwrap();
        assert!(r.headers().get(PARTIAL).is_none());
        let r = answer(gather(posts("oo"), true, read_one_of).await).unwrap();
        assert!(r.headers().get(PARTIAL).is_none());
        // Some couldn't be read: what was found, flagged
        let f = gather(posts("cxo"), true, read_one_of).await;
        assert_eq!(
            f,
            Found {
                list: vec![0],
                failed: 1,
                cut: false
            }
        );
        assert_eq!(answer(f).unwrap().headers()[PARTIAL], "1");
        // None could be read: an error, not "no shared playlists"
        let e = answer(gather(posts("xx"), true, read_one_of).await).unwrap_err();
        assert_eq!(e.status, StatusCode::BAD_GATEWAY);
        assert!(e.message.contains("2 of their posts"), "{}", e.message);
        // ...also when the others weren't collections
        assert!(answer(gather(posts("oxo"), true, read_one_of).await).is_err());
        // A cut listing: flagged, with or without any found
        let f = gather(posts("co"), false, read_one_of).await;
        assert!(f.cut);
        assert_eq!(answer(f).unwrap().headers()[PARTIAL], "1");
        assert_eq!(
            answer(gather(posts("oo"), false, read_one_of).await)
                .unwrap()
                .headers()[PARTIAL],
            "1"
        );
        // Enough collections found: nothing missed matters
        let many = "c".repeat(LIST_MAX) + "xx";
        let f = gather(posts(&many), false, read_one_of).await;
        assert_eq!((f.list.len(), f.failed, f.cut), (LIST_MAX, 0, false));
        assert!(answer(f).unwrap().headers().get(PARTIAL).is_none());
    }
}
