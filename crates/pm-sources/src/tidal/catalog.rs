//! TIDAL's catalogue with a full-tracks session: TIDAL's v1 API, as python-tidal reads it
//! (`/search`, `/albums/{id}`, `/albums/{id}/tracks`, `/tracks/{id}`, `/artists/{id}`,
//! `/artists/{id}/albums`). The phone uses it: it signs in only the full-tracks ways, and the
//! official API answers a signed-in person only after its own sign-in (or the app's secret,
//! which never ships). Hits and items have the node's ids (`track:<id>`, `album:<id>`,
//! `artist:<id>`, queue URI `tidal:track:<id>`), so pages and queues are the same everywhere.

use super::full::{Session, PLAYBACK_CLIENT_VERSION, PLAYBACK_USER_AGENT};
use super::{SOURCE_ID, TRACK_URI_PREFIX};
use crate::types::*;
use crate::{SourceError, StatusCode};
use serde_json::Value;

/// How many songs of an album are read (TIDAL's page limit is 100).
const ALBUM_TRACKS: usize = 100;
/// At most this many pages of an album's tracks are read (1,000 tracks).
const ALBUM_PAGES: usize = 10;
/// How many of an artist's releases are read (one page each of albums and of EPs and singles).
const ARTIST_RELEASES: usize = 50;

/// `https://tidal.com/browse/{kind}/{id}`: the service's own page.
pub fn page_url(kind: &str, id: &str) -> String {
    format!("https://tidal.com/browse/{kind}/{id}")
}

pub fn track_uri(id: &str) -> String {
    format!("{TRACK_URI_PREFIX}{id}")
}

/// TIDAL's image id (`ab12cd34-…`) as a picture of `size` (`640x640`, `750x750`).
fn image(id: Option<&str>, size: &str) -> Option<String> {
    let id = id.filter(|i| !i.is_empty() && i.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'))?;
    Some(format!(
        "https://resources.tidal.com/images/{}/{size}.jpg",
        id.replace('-', "/")
    ))
}

fn year_of(date: Option<&str>) -> Option<i32> {
    date.and_then(|d| d.get(..4)).and_then(|y| y.parse().ok())
}

fn artists_of(v: &Value) -> Option<String> {
    let names: Vec<&str> = v["artists"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a["name"].as_str())
        .collect();
    if names.is_empty() {
        v["artist"]["name"].as_str().map(str::to_string)
    } else {
        Some(names.join(", "))
    }
}

/// A number id as text (TIDAL sends numbers).
fn id_of(v: &Value) -> Option<String> {
    v["id"]
        .as_u64()
        .map(|n| n.to_string())
        .or_else(|| v["id"].as_str().map(str::to_string))
        .filter(|i| !i.is_empty() && i.len() <= 20 && i.bytes().all(|b| b.is_ascii_digit()))
}

fn numeric(id: &str) -> Result<&str, SourceError> {
    if id.is_empty() || id.len() > 20 || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SourceError::bad_request("bad TIDAL id"));
    }
    Ok(id)
}

/// A playable song (TIDAL marks what it won't stream).
fn streams(t: &Value) -> bool {
    t["streamReady"].as_bool() != Some(false) && t["allowStreaming"].as_bool() != Some(false)
}

pub fn track_hit(t: &Value) -> Option<SourceHit> {
    let id = id_of(t)?;
    Some(SourceHit {
        id: format!("track:{id}"),
        title: t["title"].as_str()?.to_string(),
        creator: artists_of(t),
        year: year_of(t["album"]["releaseDate"].as_str()),
        downloads: None,
        image: image(t["album"]["cover"].as_str(), "640x640"),
        kind: "track".into(),
        duration_ms: t["duration"].as_u64().map(|d| d * 1000),
        library_artist_id: None,
        isrc: t["isrc"].as_str().map(str::to_string),
        upc: None,
        library_track_id: None,
        library_album_id: None,
        uri: Some(track_uri(&id)),
        url: page_url("track", &id),
    })
}

pub fn album_hit(a: &Value) -> Option<SourceHit> {
    let id = id_of(a)?;
    Some(SourceHit {
        id: format!("album:{id}"),
        title: a["title"].as_str()?.to_string(),
        creator: artists_of(a),
        year: year_of(a["releaseDate"].as_str()),
        downloads: None,
        image: image(a["cover"].as_str(), "640x640"),
        kind: "album".into(),
        duration_ms: None,
        library_artist_id: None,
        isrc: None,
        upc: a["upc"].as_str().map(str::to_string),
        library_track_id: None,
        library_album_id: None,
        uri: None,
        url: page_url("album", &id),
    })
}

pub fn artist_hit(a: &Value) -> Option<SourceHit> {
    let id = id_of(a)?;
    Some(SourceHit {
        id: format!("artist:{id}"),
        title: a["name"].as_str()?.to_string(),
        creator: None,
        year: None,
        downloads: None,
        image: image(a["picture"].as_str(), "750x750"),
        kind: "artist".into(),
        duration_ms: None,
        library_artist_id: None,
        isrc: None,
        upc: None,
        library_track_id: None,
        library_album_id: None,
        uri: None,
        url: page_url("artist", &id),
    })
}

fn source_track(t: &Value) -> Option<SourceTrack> {
    let id = id_of(t)?;
    let format = match t["audioQuality"].as_str() {
        Some("HI_RES_LOSSLESS") | Some("HI_RES") => "FLAC hi-res (TIDAL)",
        Some("LOSSLESS") => "FLAC (TIDAL)",
        Some(_) => "AAC (TIDAL)",
        None => "TIDAL",
    };
    Some(SourceTrack {
        title: t["title"].as_str()?.to_string(),
        track_no: t["trackNumber"].as_u64().map(|n| n as u32),
        duration_ms: t["duration"].as_u64().map(|d| d * 1000),
        format: format.into(),
        url: track_uri(&id),
        artist: artists_of(t),
    })
}

/// One GET on the v1 API with the person's session (bearer, country, python-tidal's headers).
pub async fn get(
    http: &reqwest::Client,
    v1_base: &str,
    sess: &Session,
    path: &str,
    query: &[(&str, String)],
) -> Result<Value, SourceError> {
    let mut q: Vec<(&str, String)> = query.to_vec();
    q.push(("countryCode", sess.country.clone()));
    let resp = http
        .get(format!("{}{path}", v1_base.trim_end_matches('/')))
        .bearer_auth(&sess.access_token)
        .header(reqwest::header::USER_AGENT, PLAYBACK_USER_AGENT)
        .header("x-tidal-client-version", PLAYBACK_CLIENT_VERSION)
        .query(&q)
        .send()
        .await
        .map_err(|e| SourceError::new(StatusCode::BAD_GATEWAY, format!("TIDAL: {}", e.without_url())))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    match status.as_u16() {
        200..=299 => Ok(v),
        404 => Err(SourceError::not_found("not on TIDAL")),
        401 | 403 => Err(SourceError::new(
            StatusCode::PRECONDITION_FAILED,
            "TIDAL refused the sign-in: sign in to TIDAL again",
        )),
        _ => Err(SourceError::new(
            StatusCode::BAD_GATEWAY,
            format!(
                "TIDAL answered {status}{}",
                v["userMessage"]
                    .as_str()
                    .map(|m| format!(": {m}"))
                    .unwrap_or_default()
            ),
        )),
    }
}

/// Search: an artist of exactly that name first (at most one), then albums, then songs.
pub async fn search(
    http: &reqwest::Client,
    v1_base: &str,
    sess: &Session,
    term: &str,
    limit: u32,
) -> Result<Vec<SourceHit>, SourceError> {
    let limit = limit.clamp(1, 50);
    let v = get(
        http,
        v1_base,
        sess,
        "/search",
        &[
            ("query", term.to_string()),
            ("limit", limit.to_string()),
            ("offset", "0".into()),
            ("types", "ARTISTS,ALBUMS,TRACKS".into()),
        ],
    )
    .await?;
    Ok(search_hits(&v, term, limit))
}

/// The hits of one `/search` answer.
pub fn search_hits(v: &Value, term: &str, limit: u32) -> Vec<SourceHit> {
    let needle = term.trim().to_lowercase();
    let items = |k: &str| v[k]["items"].as_array().cloned().unwrap_or_default();
    let mut out: Vec<SourceHit> = items("artists")
        .iter()
        .filter_map(artist_hit)
        .filter(|a| a.title.to_lowercase() == needle)
        .take(1)
        .collect();
    out.extend(items("albums").iter().filter_map(album_hit));
    out.extend(
        items("tracks")
            .iter()
            .filter(|t| streams(t))
            .filter_map(track_hit),
    );
    out.truncate(limit as usize * 2);
    out
}

/// An album (`album:<id>`, its playable songs) or a song (`track:<id>`).
pub async fn item(
    http: &reqwest::Client,
    v1_base: &str,
    sess: &Session,
    id: &str,
) -> Result<SourceItem, SourceError> {
    if let Some(aid) = id.strip_prefix("album:") {
        let aid = numeric(aid)?;
        let a = get(http, v1_base, sess, &format!("/albums/{aid}"), &[]).await?;
        // Every page of its tracks (TIDAL gives 100 at a time), up to ALBUM_PAGES
        let mut tracks: Vec<Value> = Vec::new();
        for _ in 0..ALBUM_PAGES {
            let t = get(
                http,
                v1_base,
                sess,
                &format!("/albums/{aid}/tracks"),
                &[
                    ("limit", ALBUM_TRACKS.to_string()),
                    ("offset", tracks.len().to_string()),
                ],
            )
            .await?;
            let page = t["items"].as_array().cloned().unwrap_or_default();
            let total = t["totalNumberOfItems"].as_u64().unwrap_or(0) as usize;
            let empty = page.is_empty();
            tracks.extend(page);
            if empty || tracks.len() >= total {
                break;
            }
        }
        return Ok(album_item(id, aid, &a, &serde_json::json!({ "items": tracks })));
    }
    if let Some(tid) = id.strip_prefix("track:") {
        let tid = numeric(tid)?;
        let t = get(http, v1_base, sess, &format!("/tracks/{tid}"), &[]).await?;
        return Ok(SourceItem {
            source: SOURCE_ID.into(),
            id: id.into(),
            title: t["title"].as_str().unwrap_or_default().to_string(),
            creator: artists_of(&t),
            year: year_of(t["album"]["releaseDate"].as_str()),
            description: t["album"]["title"].as_str().map(|a| format!("From {a}")),
            licence: None,
            image: image(t["album"]["cover"].as_str(), "640x640"),
            url: page_url("track", tid),
            tracks: source_track(&t).into_iter().collect(),
            library_album_id: None,
        });
    }
    Err(SourceError::bad_request(
        "a TIDAL item is album:<id> or track:<id>",
    ))
}

/// An album from its `/albums/{id}` and `/albums/{id}/tracks` answers.
pub fn album_item(id: &str, aid: &str, a: &Value, tracks: &Value) -> SourceItem {
    SourceItem {
        source: SOURCE_ID.into(),
        id: id.into(),
        title: a["title"].as_str().unwrap_or_default().to_string(),
        creator: artists_of(a),
        year: year_of(a["releaseDate"].as_str()),
        description: a["copyright"].as_str().map(str::to_string),
        licence: None,
        image: image(a["cover"].as_str(), "640x640"),
        url: page_url("album", aid),
        tracks: tracks["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| streams(t))
            .filter_map(source_track)
            .collect(),
        library_album_id: None,
    }
}

/// An artist (`artist:<id>` or the bare id) with their albums, EPs and singles, newest first.
pub async fn artist(
    http: &reqwest::Client,
    v1_base: &str,
    sess: &Session,
    id: &str,
) -> Result<SourceArtist, SourceError> {
    let aid = numeric(id.strip_prefix("artist:").unwrap_or(id))?.to_string();
    let a = get(http, v1_base, sess, &format!("/artists/{aid}"), &[]).await?;
    let page = |filter: Option<&'static str>| {
        let mut q: Vec<(&str, String)> = vec![("limit", ARTIST_RELEASES.to_string()), ("offset", "0".into())];
        if let Some(f) = filter {
            q.push(("filter", f.into()));
        }
        q
    };
    let albums = get(
        http,
        v1_base,
        sess,
        &format!("/artists/{aid}/albums"),
        &page(None),
    )
    .await?;
    // EPs and singles are their own list; a failure there leaves the albums
    let singles = get(
        http,
        v1_base,
        sess,
        &format!("/artists/{aid}/albums"),
        &page(Some("EPSANDSINGLES")),
    )
    .await;
    let (releases, catalogue) = artist_releases(&albums, singles.as_ref().ok(), singles.as_ref().err());
    Ok(SourceArtist {
        source: SOURCE_ID.into(),
        id: format!("artist:{aid}"),
        name: a["name"].as_str().unwrap_or_default().to_string(),
        image: image(a["picture"].as_str(), "750x750"),
        url: page_url("artist", &aid),
        albums: releases,
        library_artist_id: None,
        catalogue: Some(catalogue),
    })
}

/// An artist's releases from the albums page and the EPs-and-singles page, honest about what
/// they hold: more listed than arrived is `truncated`; a page that failed makes the list
/// incomplete with the reason.
pub fn artist_releases(
    albums: &Value,
    singles: Option<&Value>,
    singles_failed: Option<&SourceError>,
) -> (Vec<ArtistRelease>, Catalogue) {
    let mut out = Vec::new();
    let mut listed = 0u64;
    let mut arrived = 0u64;
    for page in std::iter::once(albums).chain(singles) {
        let items = page["items"].as_array().cloned().unwrap_or_default();
        listed += page["totalNumberOfItems"].as_u64().unwrap_or(items.len() as u64);
        // Only rows that make a release count as arrived: a row TIDAL sent malformed is missing
        out.extend(items.iter().filter_map(|x| {
            let hit = album_hit(x)?;
            arrived += 1;
            Some(ArtistRelease {
                hit,
                release: crate::releases::tidal(x),
            })
        }));
    }
    crate::releases::dedupe(&mut out);
    crate::releases::sort_releases(&mut out);
    let truncated = listed > arrived;
    let error = singles_failed.map(|e| format!("TIDAL's EPs and singles didn't load: {}", e.message));
    let catalogue = Catalogue {
        source: SOURCE_ID.into(),
        complete: error.is_none() && !truncated,
        truncated,
        total: error.is_none().then_some(listed),
        error,
    };
    (out, catalogue)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn an_album_of_more_than_a_page_has_all_its_tracks() {
        let app = axum::Router::new()
            .route(
                "/v1/albums/7",
                axum::routing::get(|| async { axum::Json(json!({"id": 7, "title": "Long", "artists": [{"name": "A"}]})) }),
            )
            .route(
                "/v1/albums/7/tracks",
                axum::routing::get(
                    |q: axum::extract::Query<std::collections::HashMap<String, String>>| async move {
                        let from: u64 = q.0["offset"].parse().unwrap();
                        let items: Vec<_> = (from..(from + 100).min(150))
                            .map(|n| json!({"id": n + 1, "title": format!("T{}", n + 1), "trackNumber": n + 1}))
                            .collect();
                        axum::Json(json!({"items": items, "totalNumberOfItems": 150}))
                    },
                ),
            );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        let sess = Session {
            access_token: "t".into(),
            country: "CH".into(),
            ..Default::default()
        };
        let it = item(&reqwest::Client::new(), &base, &sess, "album:7")
            .await
            .unwrap();
        assert_eq!(it.tracks.len(), 150);
        assert_eq!(it.tracks[149].title, "T150");
    }

    #[test]
    fn a_malformed_release_makes_the_list_incomplete() {
        let albums =
            json!({"items": [{"id": 1, "title": "Good"}, {"title": "no id"}], "totalNumberOfItems": 2});
        let (out, c) = artist_releases(&albums, None, None);
        assert_eq!(out.len(), 1);
        assert!(c.truncated && !c.complete, "{c:?}");
    }

    #[test]
    fn search_puts_the_named_artist_first_and_drops_what_tidal_wont_stream() {
        let v = json!({
            "artists": {"items": [
                {"id": 7, "name": "Other", "picture": "aa-bb"},
                {"id": 8, "name": "Daft Punk", "picture": "cc-dd"}
            ]},
            "albums": {"items": [
                {"id": 100, "title": "Discovery", "cover": "12ab-34cd", "releaseDate": "2001-03-12",
                 "artists": [{"id": 8, "name": "Daft Punk"}], "upc": "0724384960650", "type": "ALBUM"}
            ]},
            "tracks": {"items": [
                {"id": 200, "title": "One More Time", "duration": 320, "isrc": "GBDUW0000053",
                 "album": {"id": 100, "title": "Discovery", "cover": "12ab-34cd", "releaseDate": "2001"},
                 "artists": [{"id": 8, "name": "Daft Punk"}], "streamReady": true},
                {"id": 201, "title": "Not here", "duration": 10, "artists": [], "streamReady": false}
            ]}
        });
        let hits = search_hits(&v, "daft punk", 10);
        let ids: Vec<&str> = hits.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, ["artist:8", "album:100", "track:200"]);
        let song = &hits[2];
        assert_eq!(song.uri.as_deref(), Some("tidal:track:200"));
        assert_eq!(song.duration_ms, Some(320_000));
        assert_eq!(song.creator.as_deref(), Some("Daft Punk"));
        assert_eq!(song.url, "https://tidal.com/browse/track/200");
        assert_eq!(
            hits[1].image.as_deref(),
            Some("https://resources.tidal.com/images/12ab/34cd/640x640.jpg")
        );
        assert_eq!(hits[1].year, Some(2001));
    }

    #[test]
    fn an_album_lists_its_playable_songs_as_queue_uris() {
        let a = json!({"id": 100, "title": "Discovery", "cover": "12ab-34cd", "releaseDate": "2001-03-12",
                       "artists": [{"name": "Daft Punk"}], "copyright": "(P) 2001"});
        let t = json!({"items": [
            {"id": 1, "title": "One More Time", "trackNumber": 1, "duration": 320, "audioQuality": "LOSSLESS",
             "artists": [{"name": "Daft Punk"}]},
            {"id": 2, "title": "Gone", "trackNumber": 2, "duration": 100, "allowStreaming": false}
        ]});
        let it = album_item("album:100", "100", &a, &t);
        assert_eq!(it.title, "Discovery");
        assert_eq!(it.tracks.len(), 1);
        assert_eq!(it.tracks[0].url, "tidal:track:1");
        assert_eq!(it.tracks[0].track_no, Some(1));
        assert_eq!(it.tracks[0].format, "FLAC (TIDAL)");
    }

    #[test]
    fn an_artists_releases_say_when_tidal_lists_more_or_a_page_failed() {
        let albums = json!({"totalNumberOfItems": 3, "items": [
            {"id": 1, "title": "Old", "releaseDate": "1999-01-01", "type": "ALBUM"},
            {"id": 2, "title": "New", "releaseDate": "2020-01-01", "type": "ALBUM"}
        ]});
        let singles = json!({"totalNumberOfItems": 1, "items": [
            {"id": 3, "title": "Single", "releaseDate": "2021-05-01", "type": "SINGLE"}
        ]});
        let (r, cat) = artist_releases(&albums, Some(&singles), None);
        let titles: Vec<&str> = r.iter().map(|x| x.hit.title.as_str()).collect();
        assert_eq!(titles, ["Single", "New", "Old"], "newest first");
        assert_eq!(r[0].release.release_type.as_deref(), Some("single"));
        assert!(cat.truncated && !cat.complete);
        let failed = SourceError::new(StatusCode::BAD_GATEWAY, "TIDAL answered 500");
        let (_, cat) = artist_releases(&singles, None, Some(&failed));
        assert!(!cat.complete && cat.error.is_some());
    }

    #[test]
    fn ids_and_images_are_checked_before_they_go_into_addresses() {
        assert!(numeric("123").is_ok());
        assert!(numeric("12/../3").is_err());
        assert!(numeric("").is_err());
        assert_eq!(image(Some("../x"), "640x640"), None);
        assert_eq!(id_of(&json!({"id": "1/2"})), None);
    }
}
