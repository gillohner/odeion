//! Item pages in phone mode (`GET /v1/item/{kind}/{id}`): a TIDAL or Qobuz album, song or artist,
//! read from the service with the person's own sign-in, in the node's page shape. Page ids are the
//! node's (`tidal-<id>`, `u-<link>`; `pm_sources::pages`), so search, links and Back are the same.

use super::{err, qobuz, source_err, tidal, Phone};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use pm_sources::pages::{page_id, url_of_page};
use pm_sources::types::{ItemAlbum, SourceArtist, SourceItem};
use pm_sources::SourceError;
use serde_json::{json, Value};
use std::sync::Arc;

/// Which service item a page id names: `(service, kind, id)` with kind `track`, `album` or
/// `artist`.
pub(crate) fn service_item(kind: &str, id: &str) -> Option<(&'static str, &'static str, String)> {
    let url = url_of_page(kind, id)?;
    let u = url::Url::parse(&url).ok()?;
    let segs: Vec<&str> = u.path_segments()?.filter(|x| !x.is_empty()).collect();
    let (svc, k, n) = match (u.host_str()?, segs.as_slice()) {
        ("tidal.com" | "listen.tidal.com" | "www.tidal.com", [k, n] | ["browse", k, n]) => ("tidal", *k, *n),
        ("open.qobuz.com" | "play.qobuz.com", [k, n]) => ("qobuz", *k, *n),
        ("www.qobuz.com", [_, k, _, n] | [_, k, n]) => ("qobuz", *k, *n),
        _ => return None,
    };
    let k = match k {
        "track" => "track",
        "album" => "album",
        "artist" => "artist",
        _ => return None,
    };
    let ok = !n.is_empty() && n.len() <= 40 && n.bytes().all(|b| b.is_ascii_alphanumeric());
    ok.then(|| (svc, k, n.to_string()))
}

/// The service page of a queue URI (`qobuz:track:1` → its open.qobuz.com page).
pub(crate) fn song_page(uri: &str) -> Option<String> {
    let (svc, rest) = uri.split_once(':')?;
    let (kind, id) = rest.split_once(':')?;
    match svc {
        "tidal" => Some(pm_sources::tidal::catalog::page_url(kind, id)),
        "qobuz" => Some(pm_sources::qobuz::page_url(kind, id)),
        _ => None,
    }
}

fn label(svc: &str) -> &'static str {
    if svc == "tidal" {
        "TIDAL"
    } else {
        "Qobuz"
    }
}

async fn read_item(p: &Arc<Phone>, svc: &str, id: &str) -> Result<SourceItem, SourceError> {
    match svc {
        "tidal" => {
            let sess = tidal::session(p).await?;
            pm_sources::tidal::catalog::item(&p.http, &p.tidal.v1_base, &sess, id).await
        }
        _ => {
            let (app, sess) = qobuz::ready(p)?;
            pm_sources::qobuz::item(&p.http, &app, &sess.token, id)
                .await
                .map_err(|e| qobuz::failed(p, e))
        }
    }
}

async fn read_artist(p: &Arc<Phone>, svc: &str, id: &str) -> Result<SourceArtist, SourceError> {
    match svc {
        "tidal" => {
            let sess = tidal::session(p).await?;
            pm_sources::tidal::catalog::artist(&p.http, &p.tidal.v1_base, &sess, id).await
        }
        _ => {
            let (app, sess) = qobuz::ready(p)?;
            pm_sources::qobuz::artist(&p.http, &app, &sess.token, id)
                .await
                .map_err(|e| qobuz::failed(p, e))
        }
    }
}

/// An album or song page from the service's item.
pub(crate) fn item_page(kind: &str, id: &str, svc: &str, skind: &str, sid: &str, it: &SourceItem) -> Value {
    let songs: Vec<Value> = it
        .tracks
        .iter()
        .map(|t| {
            let page = song_page(&t.url);
            json!({
                "no": t.track_no,
                "title": t.title,
                "artist": t.artist.clone().or_else(|| it.creator.clone()),
                "duration_ms": t.duration_ms,
                "uri": t.url,
                "page": page,
                "image": it.image,
                "id": page.as_deref().map(page_id),
                "album": (skind == "album").then(|| it.title.clone()),
                "album_id": (skind == "album").then(|| id.to_string()),
            })
        })
        .collect();
    let total: u64 = it.tracks.iter().filter_map(|t| t.duration_ms).sum();
    json!({
        "kind": kind,
        "id": id,
        "uri": it.url,
        "target": { "uri": it.url },
        "title": it.title,
        "artist": it.creator.as_ref().map(|name| json!({ "name": name })),
        "year": it.year,
        "image": it.image,
        "source": label(svc),
        "source_id": svc,
        "plays_from": (!songs.is_empty()).then(|| json!({
            "service": svc, "kind": skind, "id": sid, "label": label(svc), "url": it.url, "mode": "full",
        })),
        "songs": songs,
        "total_ms": total,
        "albums": [],
        "versions": [],
        "also_on": [],
        "open_on": [{ "source": svc, "id": format!("{skind}:{sid}"), "label": label(svc), "url": it.url }],
        "link": it.url,
        "about": [],
        "blurb": it.description,
        "unavailable": it.tracks.is_empty().then(|| format!("{} lists nothing playable here", label(svc))),
        "section": "home",
    })
}

/// An artist page from the service's artist.
pub(crate) fn artist_page(id: &str, svc: &str, a: &SourceArtist) -> Value {
    let albums: Vec<ItemAlbum> = a
        .albums
        .iter()
        .map(|r| ItemAlbum {
            id: page_id(&r.hit.url),
            title: r.hit.title.clone(),
            year: r.hit.year,
            image: r.hit.image.clone(),
            page: r.hit.url.clone(),
            release: r.release.clone(),
        })
        .collect();
    json!({
        "kind": "artist",
        "id": id,
        "uri": a.url,
        "target": { "uri": a.url },
        "title": a.name,
        "image": a.image,
        "source": label(svc),
        "source_id": svc,
        "songs": [],
        "total_ms": 0,
        "albums": albums,
        "catalogue": a.catalogue,
        "versions": [],
        "also_on": [],
        "open_on": [{ "source": svc, "id": a.id, "label": label(svc), "url": a.url }],
        "link": a.url,
        "about": [],
        "section": "home",
    })
}

/// `GET /v1/item/{kind}/{id}`.
pub(crate) async fn item(State(p): State<Arc<Phone>>, Path((kind, id)): Path<(String, String)>) -> Response {
    if !matches!(kind.as_str(), "song" | "album" | "artist") {
        return err(StatusCode::BAD_REQUEST, "kind is song, album or artist");
    }
    let Some((svc, skind, sid)) = service_item(&kind, &id) else {
        return err(
            StatusCode::NOT_FOUND,
            "On a phone without a node, pages come from TIDAL and Qobuz",
        );
    };
    let page = if skind == "artist" {
        read_artist(&p, svc, &sid)
            .await
            .map(|a| artist_page(&id, svc, &a))
    } else {
        read_item(&p, svc, &format!("{skind}:{sid}"))
            .await
            .map(|it| item_page(&kind, &id, svc, skind, &sid, &it))
    };
    match page {
        Ok(v) => Json(v).into_response(),
        Err(e) => source_err(e),
    }
}

/// `GET /v1/item/{kind}/{id}/extras`: nothing more on a phone (no library, no Pinakes).
pub(crate) async fn extras() -> Json<Value> {
    Json(json!({ "related": [], "concerts": [], "similar": [] }))
}

fn own_source(source: &str) -> Result<&'static str, SourceError> {
    match source {
        "tidal" => Ok("tidal"),
        "qobuz" => Ok("qobuz"),
        _ => Err(SourceError::not_found("phone mode has TIDAL and Qobuz")),
    }
}

/// `GET /v1/sources/{source}/items/{id}`: the service's own item, as on the node.
pub(crate) async fn source_item(
    State(p): State<Arc<Phone>>,
    Path((source, id)): Path<(String, String)>,
) -> Response {
    let svc = match own_source(&source) {
        Ok(s) => s,
        Err(e) => return source_err(e),
    };
    match read_item(&p, svc, &id).await {
        Ok(it) => Json(it).into_response(),
        Err(e) => source_err(e),
    }
}

/// `GET /v1/sources/{source}/artists/{id}`.
pub(crate) async fn source_artist(
    State(p): State<Arc<Phone>>,
    Path((source, id)): Path<(String, String)>,
) -> Response {
    let svc = match own_source(&source) {
        Ok(s) => s,
        Err(e) => return source_err(e),
    };
    match read_artist(&p, svc, &id).await {
        Ok(a) => Json(a).into_response(),
        Err(e) => source_err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_ids_name_tidal_and_qobuz_items_and_nothing_else() {
        assert_eq!(
            service_item("album", "tidal-123"),
            Some(("tidal", "album", "123".into()))
        );
        assert_eq!(
            service_item("song", "tidal-9"),
            Some(("tidal", "track", "9".into()))
        );
        let q = page_id("https://open.qobuz.com/album/0060253780968");
        assert_eq!(
            service_item("album", &q),
            Some(("qobuz", "album", "0060253780968".into()))
        );
        assert_eq!(
            service_item("album", &page_id("https://example.org/album/1")),
            None
        );
        assert_eq!(service_item("album", "lib-3"), None);
        assert_eq!(
            service_item("song", &page_id("https://open.qobuz.com/track/1%2F..%2Fx")),
            None
        );
    }

    #[test]
    fn an_album_page_plays_its_songs_from_the_service() {
        let it = SourceItem {
            source: "qobuz".into(),
            id: "album:a1".into(),
            title: "Album".into(),
            creator: Some("Band".into()),
            year: Some(2020),
            description: None,
            licence: None,
            image: Some("https://img/x.jpg".into()),
            url: "https://open.qobuz.com/album/a1".into(),
            tracks: vec![pm_sources::types::SourceTrack {
                title: "One".into(),
                track_no: Some(1),
                duration_ms: Some(1000),
                format: "FLAC (Qobuz)".into(),
                url: "qobuz:track:55".into(),
                artist: None,
            }],
            library_album_id: None,
        };
        let id = page_id(&it.url);
        let pg = item_page("album", &id, "qobuz", "album", "a1", &it);
        assert_eq!(pg["songs"][0]["uri"], "qobuz:track:55");
        assert_eq!(pg["songs"][0]["artist"], "Band");
        assert_eq!(pg["songs"][0]["id"], page_id("https://open.qobuz.com/track/55"));
        assert_eq!(pg["plays_from"]["service"], "qobuz");
        assert_eq!(pg["total_ms"], 1000);
        assert!(pg["unavailable"].is_null());
    }
}
