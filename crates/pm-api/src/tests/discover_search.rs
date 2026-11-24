//! Search, item pages, Home and Discover, profiles, comments and smart playlists.
//!
//! Part of pm-api's unit tests (`mod tests` in lib.rs, which holds the helpers these use). A new
//! test of this area goes at the end of this file.

use super::*;

#[tokio::test]
async fn new_releases_come_from_musicbrainz_for_tagged_artists() {
    let artist_mbid = "056e4f3e-d505-4dad-8ec1-d04f521cbb56";
    let fresh_rg = "11111111-2222-3333-4444-555555555555";
    let owned_rg = "66666666-7777-8888-9999-000000000000";
    let today = time::OffsetDateTime::now_utc().date();
    let recent = (today - time::Duration::days(30)).to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hits2 = hits.clone();
    let mock = Router::new().route(
        "/ws/2/release-group",
        get(move |Query(q): Query<std::collections::HashMap<String, String>>| {
            let hits = hits2.clone();
            let recent = recent.clone();
            async move {
                hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                assert_eq!(q.get("artist").map(String::as_str), Some(artist_mbid));
                Json(serde_json::json!({"release-group-count": 4, "release-group-offset": 0, "release-groups": [
                    {"id": fresh_rg, "title": "Brand New", "first-release-date": recent, "primary-type": "Album", "secondary-types": []},
                    {"id": owned_rg, "title": "Already Here", "first-release-date": recent, "primary-type": "EP", "secondary-types": []},
                    {"id": "aaaaaaaa-0000-0000-0000-000000000000", "title": "Old", "first-release-date": "2001-05-01", "primary-type": "Album", "secondary-types": []},
                    {"id": "bbbbbbbb-0000-0000-0000-000000000000", "title": "Live", "first-release-date": recent, "primary-type": "Album", "secondary-types": ["Live"]}
                ]}))
            }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.info_bases.write().unwrap().musicbrainz = format!("http://127.0.0.1:{port}");
    s.set_contact("tests@example.org").ok();
    {
        let lib = s.library.lock().unwrap();
        let mut t = pm_model::Track {
            id: None,
            title: "Song".into(),
            artists: vec![pm_model::Artist {
                id: None,
                name: "Daft Punk".into(),
                mbid: Some(pm_model::Mbid::parse(artist_mbid).unwrap()),
            }],
            album: Some(pm_model::Album {
                id: None,
                title: "Already Here".into(),
                album_artist: Some("Daft Punk".into()),
                release_mbid: None,
                release_group_mbid: Some(pm_model::Mbid::parse(owned_rg).unwrap()),
                year: Some(2026),
                upc: None,
                total_discs: None,
            }),
            track_no: Some(1),
            disc_no: None,
            duration_ms: Some(1000),
            file: pm_model::AudioFile {
                path: dir.path().join("song.flac"),
                size_bytes: 1,
                mtime_unix: 0,
                container: "flac".into(),
                format: pm_model::AudioFormat::pcm(44_100, 16, 2),
                duration_ms: Some(1000),
            },
            recording_mbid: None,
            isrc: None,
            acoustid: None,
            canonical: None,
            replaygain_track_gain_db: None,
            replaygain_album_gain_db: None,
            genres: vec![],
        };
        t.id = None;
        lib.upsert_track(&t).unwrap();
    }
    let app = router(s.clone());
    // First call: nothing cached yet, a refresh starts in the background.
    let (st, p) = call(&app, "GET", "/v1/library/new-releases", None).await;
    assert_eq!(st, StatusCode::OK, "{p}");
    assert_eq!(p["artists_total"], 1);
    let mut page = p;
    for _ in 0..300 {
        if page["artists_checked"] == 1 && page["refreshing"] == false {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        page = call(&app, "GET", "/v1/library/new-releases", None).await.1;
    }
    assert_eq!(page["artists_checked"], 1, "{page}");
    let items = page["items"].as_array().unwrap();
    let titles: Vec<&str> = items.iter().map(|i| i["title"].as_str().unwrap()).collect();
    assert_eq!(
        titles,
        ["Brand New", "Already Here"],
        "old and live records are left out"
    );
    assert!(items[0].get("in_library").is_none());
    assert_eq!(items[1]["in_library"]["title"], "Already Here");
    assert_eq!(
        items[0]["url"],
        format!("https://musicbrainz.org/release-group/{fresh_rg}")
    );
    // Cached for a week: another page read does not hit MusicBrainz again.
    let before = hits.load(std::sync::atomic::Ordering::SeqCst);
    call(&app, "GET", "/v1/library/new-releases", None).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), before);
}

/// `GET /v1/discover/play` resolves through the node's connected
/// services: with nothing connected it names the services that have the
/// song (and offers a preview); once the library owns the recording it
/// plays from there.
#[tokio::test]
async fn play_plan_resolves_through_connected_services() {
    let fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-pubky/tests/fixtures/pinakes/music/detail-tidal-track-bohemian.json"
    ))
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = Router::new().route(
        "/v1/resources/by-uri",
        get(move || {
            let fixture = fixture.clone();
            async move { ([(header::CONTENT_TYPE, "application/json")], fixture) }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let pubky = pm_pubky::config::PmPubkyConfig {
        pinakes_base: format!("http://127.0.0.1:{port}"),
        nexus_base: format!("http://127.0.0.1:{port}/nexus"),
        user_agent_contact: "test@example.org".into(),
        ..Default::default()
    };
    let s = state_with(dir.path(), None, pubky).await;
    s.set_contact("test@example.org").unwrap();
    let app = router(s.clone());
    let q = "/v1/discover/play?uri=https%3A%2F%2Ftidal.com%2Ftrack%2F534050211";
    let (st, plan) = call(&app, "GET", q, None).await;
    assert_eq!(st, StatusCode::OK, "{plan}");
    assert!(plan["plays_from"].is_null(), "nothing is connected: {plan}");
    let connect: Vec<&str> = plan["connect"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        connect.contains(&"tidal") && connect.contains(&"spotify"),
        "{plan}"
    );
    assert_eq!(plan["preview"]["service"], "deezer");
    assert_eq!(plan["preview"]["mode"], "preview");
    assert!(plan["items"].is_null() || plan["items"].as_array().unwrap().is_empty());
    // The item page says the same, without expanding anything.
    let (st, d) = call(
        &app,
        "GET",
        "/v1/discover/item?uri=https%3A%2F%2Ftidal.com%2Ftrack%2F534050211",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{d}");
    assert!(d["item"]["plays_from"].is_null());
    assert_eq!(d["item"]["connect"][0], "tidal");
    assert_eq!(d["item"]["page"], "https://tidal.com/track/534050211");
    // Own the recording: the library wins.
    let tid = {
        let lib = s.library.lock().unwrap();
        let mbid = pm_model::Mbid::parse("b1a9c0e9-d987-4042-ae91-78d6a3267d69").unwrap();
        let t = pm_model::Track {
            id: None,
            title: "Bohemian Rhapsody".into(),
            artists: vec![pm_model::Artist {
                id: None,
                name: "Queen".into(),
                mbid: None,
            }],
            album: None,
            track_no: Some(11),
            disc_no: None,
            duration_ms: Some(355_000),
            file: pm_model::AudioFile {
                path: dir.path().join("bo.flac"),
                size_bytes: 1,
                mtime_unix: 0,
                container: "flac".into(),
                format: pm_model::AudioFormat::pcm(44_100, 16, 2),
                duration_ms: Some(355_000),
            },
            recording_mbid: Some(mbid),
            isrc: None,
            acoustid: None,
            canonical: Some((
                pm_model::MusicUri::Recording { mbid },
                pm_model::Confidence::TaggedMbid,
            )),
            replaygain_track_gain_db: None,
            replaygain_album_gain_db: None,
            genres: vec![],
        };
        lib.upsert_track(&t).unwrap().0
    };
    let (st, plan) = call(&app, "GET", q, None).await;
    assert_eq!(st, StatusCode::OK, "{plan}");
    assert_eq!(plan["plays_from"]["service"], "library");
    assert_eq!(plan["plays_from"]["mode"], "full");
    assert_eq!(plan["track_ids"][0], tid);
    assert!(plan["connect"].is_null());
}

/// Favourites and playlist items from other services render as rows:
/// Pinakes's card gives title, artist and cover in one batch; a link
/// nobody knows keeps an empty title (never the URL) and its service.
#[tokio::test]
async fn describe_names_links_outside_the_library() {
    let card = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-pubky/tests/fixtures/pinakes/music/card-tidal-track-bohemian.json"
    ))
    .unwrap();
    let card: serde_json::Value = serde_json::from_str(&card).unwrap();
    let card = serde_json::json!([card.get("card").cloned().unwrap_or(card.clone())]).to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = Router::new().route(
        "/v1/resources/lookup",
        post(move || {
            let card = card.clone();
            async move { ([(header::CONTENT_TYPE, "application/json")], card) }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let pubky = pm_pubky::config::PmPubkyConfig {
        pinakes_base: format!("http://127.0.0.1:{port}"),
        nexus_base: format!("http://127.0.0.1:{port}/nexus"),
        user_agent_contact: "test@example.org".into(),
        ..Default::default()
    };
    let s = state_with(dir.path(), None, pubky).await;
    s.set_contact("test@example.org").unwrap();
    let app = router(s.clone());
    let body = serde_json::json!({"uris": ["https://tidal.com/track/534050211", "https://tidal.com/album/338204560"]});
    let (st, v) = call(&app, "POST", "/v1/discover/describe", Some(body.clone())).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let v = v.as_array().unwrap();
    assert_eq!(v.len(), 2);
    assert_eq!(v[0]["title"], "Bohemian Rhapsody");
    assert!(v[0]["image"].as_str().unwrap().starts_with("https://"));
    assert!(v[1]["title"].is_null(), "never the URL as a name: {}", v[1]);
    assert_eq!(v[1]["site"], "TIDAL");
    assert_eq!(v[1]["page"], "https://tidal.com/album/338204560");
    // Cached: the same answer without Pinakes.
    let (st, again) = call(&app, "POST", "/v1/discover/describe", Some(body)).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(again[0]["title"], "Bohemian Rhapsody");
}

/// An item page for a link nobody tagged (owner bug "Pinakes has not seen
/// this link"): by-uri says 404, so the node asks Pinakes to read it
/// (once, even for two pages at the same time); while Pinakes reads, the
/// page answers with what is known and `reading`; then the card arrives.
#[tokio::test]
async fn an_unknown_link_is_resolved_never_an_error() {
    let card = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-pubky/tests/fixtures/pinakes/music/card-deezer-album-cut-to-the-feeling.json"
    ))
    .unwrap();
    let resolves = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let r2 = resolves.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = Router::new()
        .route(
            "/v1/resources/by-uri",
            get(|| async { (StatusCode::NOT_FOUND, r#"{"error":"not found"}"#) }),
        )
        .route(
            "/v1/resolve",
            post(move |Json(body): Json<serde_json::Value>| {
                let card = card.clone();
                let n = r2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move {
                    assert_eq!(body["uri"], "https://www.deezer.com/album/41839661");
                    // Slow enough that two pages overlap.
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    if n == 0 {
                        let mut v: serde_json::Value = serde_json::from_str(&card).unwrap();
                        v["status"] = "pending".into();
                        v["also_on"] = serde_json::json!([]);
                        (
                            StatusCode::ACCEPTED,
                            [(header::CONTENT_TYPE, "application/json")],
                            v.to_string(),
                        )
                    } else {
                        (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], card)
                    }
                }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let pubky = pm_pubky::config::PmPubkyConfig {
        pinakes_base: format!("http://127.0.0.1:{port}"),
        nexus_base: format!("http://127.0.0.1:{port}/nexus"),
        user_agent_contact: "test@example.org".into(),
        ..Default::default()
    };
    let s = state_with(dir.path(), None, pubky).await;
    s.set_contact("test@example.org").unwrap();
    let app = router(s.clone());
    let q = "/v1/discover/item?uri=https%3A%2F%2Fwww.deezer.com%2Falbum%2F41839661";
    let (a, b) = tokio::join!(call(&app, "GET", q, None), call(&app, "GET", q, None));
    for (st, d) in [&a, &b] {
        assert_eq!(*st, StatusCode::OK, "never an error page: {d}");
        assert_eq!(d["reading"], true, "{d}");
        assert_eq!(
            d["item"]["title"], "Cut To The Feeling",
            "the pending card's name: {d}"
        );
    }
    assert_eq!(
        resolves.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "one resolve for two pages"
    );
    // A moment later the page asks again and Pinakes has read it.
    *s.pinakes_reader.retry_after.lock().unwrap() = std::time::Duration::ZERO;
    let (st, d) = call(&app, "GET", q, None).await;
    assert_eq!(st, StatusCode::OK, "{d}");
    assert!(d["reading"].is_null(), "{d}");
    assert_eq!(d["upc"], "0602557656527");
    assert!(!d["item"]["also_on"].as_array().unwrap().is_empty());
    assert_eq!(resolves.load(std::sync::atomic::Ordering::SeqCst), 2);
    // Found cards are cached: no more resolves.
    *s.pinakes_reader.retry_after.lock().unwrap() = crate::pinakes_reader::RETRY_AFTER;
    call(&app, "GET", q, None).await;
    assert_eq!(resolves.load(std::sync::atomic::Ordering::SeqCst), 2);
}

/// A page that goes away while its link waits to be read (its request is dropped) does not
/// hold up the reader: that read is finished all the same, and the next link is read after
/// it. (Before, the read stood still with its turn taken, and no other link was read until
/// someone asked for the same one again.)
#[tokio::test]
async fn a_page_that_goes_away_does_not_hold_up_the_link_reader() {
    use std::sync::atomic::Ordering::SeqCst;
    use std::time::Duration;
    // (Pinakes answers every read with a card it is still reading)
    let mut pending: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../pm-pubky/tests/fixtures/pinakes/music/card-deezer-album-cut-to-the-feeling.json"
        ))
        .unwrap(),
    )
    .unwrap();
    pending["status"] = "pending".into();
    pending["also_on"] = serde_json::json!([]);
    let pending = pending.to_string();
    let resolves = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let r2 = resolves.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = Router::new()
        .route(
            "/v1/resources/by-uri",
            get(|| async { (StatusCode::NOT_FOUND, r#"{"error":"not found"}"#) }),
        )
        .route(
            "/v1/resolve",
            post(move || {
                r2.fetch_add(1, SeqCst);
                let pending = pending.clone();
                async move {
                    (
                        StatusCode::ACCEPTED,
                        [(header::CONTENT_TYPE, "application/json")],
                        pending,
                    )
                }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let pubky = pm_pubky::config::PmPubkyConfig {
        pinakes_base: format!("http://127.0.0.1:{port}"),
        nexus_base: format!("http://127.0.0.1:{port}/nexus"),
        user_agent_contact: "test@example.org".into(),
        ..Default::default()
    };
    let s = state_with(dir.path(), None, pubky).await;
    s.set_contact("test@example.org").unwrap();
    let link = |n: u32| format!("https://www.deezer.com/album/{n}");
    let (two, three) = (link(2), link(3));
    // A first link is read: from now on the next read waits its turn
    crate::pinakes_reader::card(&s, &link(1)).await;
    assert_eq!(resolves.load(SeqCst), 1);
    // Whoever asked for a second link goes away while that read waits
    let gone = tokio::time::timeout(
        Duration::from_millis(100),
        crate::pinakes_reader::resolve(&s, &two),
    );
    assert!(gone.await.is_err(), "the second read waits its turn");
    assert_eq!(resolves.load(SeqCst), 1);
    // A third link is read after it
    let third = tokio::time::timeout(
        crate::pinakes_reader::RESOLVE_INTERVAL * 4,
        crate::pinakes_reader::card(&s, &three),
    );
    assert!(
        third.await.is_ok(),
        "the reader is held up by a page that went away"
    );
    assert_eq!(resolves.load(SeqCst), 3, "the second link was read too");
    // and its answer is kept: asking again reads nothing
    crate::pinakes_reader::card(&s, &two).await;
    assert_eq!(resolves.load(SeqCst), 3);
}

/// One item page (docs/ux/08-item-page.md): an album only a service knows
/// lists its songs (each with its own song page), Pinakes not knowing the
/// link is no error, and library albums and songs answer the same shape.
#[tokio::test]
async fn item_pages_list_songs_for_every_source() {
    let t = |id: u64, title: &str, n: u64| {
        serde_json::json!({"id": id, "title": title, "duration": 200 + n, "track_position": n, "preview": "http://x/p.mp3",
            "artist": {"id": 7, "name": "DAT ADAM"}, "album": {"id": 1999, "title": "HYDRA 3D"}})
    };
    let album = serde_json::json!({"id": 1999, "title": "HYDRA 3D", "release_date": "2024-01-01", "cover_big": "http://img/h.jpg",
        "artist": {"id": 7, "name": "DAT ADAM"}, "tracks": {"data": [t(11, "HYDRA", 1), t(12, "MEDUSA", 2), t(13, "NEBEL", 3)]}});
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = Router::new()
        .route(
            "/album/{id}",
            get(move || {
                let a = album.clone();
                async move { Json(a) }
            }),
        )
        // Pinakes has never seen the link and cannot read it.
        .route(
            "/v1/resources/by-uri",
            get(|| async { (StatusCode::NOT_FOUND, r#"{"error":"not found"}"#) }),
        )
        .route(
            "/v1/resolve",
            post(|| async { (StatusCode::NOT_FOUND, r#"{"error":"not found"}"#) }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let pubky = pm_pubky::config::PmPubkyConfig {
        pinakes_base: format!("http://127.0.0.1:{port}"),
        nexus_base: format!("http://127.0.0.1:{port}/nexus"),
        user_agent_contact: "test@example.org".into(),
        ..Default::default()
    };
    let s = state_with(dir.path(), None, pubky).await;
    s.set_contact("test@example.org").unwrap();
    *s.deezer_base.write().unwrap() = format!("http://127.0.0.1:{port}");
    let app = router(s.clone());
    let id = discover::page_id("https://www.deezer.com/album/1999");
    assert_eq!(
        discover::url_of_page("album", &id).as_deref(),
        Some("https://www.deezer.com/album/1999"),
        "page ids round-trip"
    );
    let (st, pg) = call(&app, "GET", &format!("/v1/item/album/{id}"), None).await;
    assert_eq!(st, StatusCode::OK, "{pg}");
    assert_eq!(pg["title"], "HYDRA 3D");
    assert_eq!(pg["artist"]["name"], "DAT ADAM");
    let songs = pg["songs"].as_array().unwrap();
    assert_eq!(songs.len(), 3, "{pg}");
    assert_eq!(songs[1]["title"], "MEDUSA");
    assert_eq!(songs[1]["no"], 2);
    assert_eq!(songs[1]["uri"], "deezer:track:12", "each row plays its own track");
    let song_id = songs[1]["id"].as_str().unwrap();
    assert_eq!(
        discover::url_of_page("song", song_id).as_deref(),
        Some("https://www.deezer.com/track/12"),
        "the title opens that song's page"
    );
    assert_eq!(pg["plays_from"]["mode"], "preview");
    assert_eq!(pg["target"]["uri"], "https://www.deezer.com/album/1999");
    assert_eq!(pg["section"], "home");
    // The library: an album and a song answer the same shape.
    let (aid, tid) = {
        let lib = s.library.lock().unwrap();
        let mut tr = untagged_track(dir.path().join("01.flac"), "Death on Two Legs", "Queen");
        tr.album = Some(pm_model::Album {
            id: None,
            title: "A Night at the Opera".into(),
            album_artist: Some("Queen".into()),
            release_mbid: None,
            release_group_mbid: None,
            year: Some(1975),
            upc: None,
            total_discs: None,
        });
        let mbid = pm_model::Mbid::parse("b1a9c0e9-d987-4042-ae91-78d6a3267d69").unwrap();
        tr.canonical = Some((
            pm_model::MusicUri::Recording { mbid },
            pm_model::Confidence::TaggedMbid,
        ));
        let tid = lib.upsert_track(&tr).unwrap();
        let t = lib.get_track(tid).unwrap().unwrap();
        (t.album.unwrap().id.unwrap(), tid)
    };
    // Another recording of the same song (a live take) in the library
    let live = {
        let lib = s.library.lock().unwrap();
        let tr = untagged_track(dir.path().join("02.flac"), "Death on Two Legs (Live)", "Queen");
        lib.upsert_track(&tr).unwrap()
    };
    assert_eq!(id, "deezer-1999", "a service page has a readable id");
    let (st, al) = call(&app, "GET", &format!("/v1/item/album/lib-{aid}"), None).await;
    assert_eq!(st, StatusCode::OK, "{al}");
    assert_eq!(al["songs"][0]["track_id"], tid.0);
    assert_eq!(
        al["songs"][0]["id"], "mb-b1a9c0e9-d987-4042-ae91-78d6a3267d69",
        "matched songs use their canonical id"
    );
    assert_eq!(al["target"]["album_id"], aid);
    assert_eq!(al["section"], "albums");
    // The canonical id opens the library's song page.
    let (st, so) = call(
        &app,
        "GET",
        "/v1/item/song/mb-b1a9c0e9-d987-4042-ae91-78d6a3267d69",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{so}");
    assert_eq!(so["target"]["track_id"], tid.0);
    // The generic id is the song's global page; the library copy is one of its sources
    assert_eq!(so["id"], "mb-b1a9c0e9-d987-4042-ae91-78d6a3267d69");
    assert_eq!(so["source_id"], "global");
    assert!(
        so["also_on"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["page"] == format!("lib-{}", tid.0)),
        "{so}"
    );
    // The library page itself is the thin view, linking to the global page
    let (_, lp) = call(&app, "GET", &format!("/v1/item/song/lib-{}", tid.0), None).await;
    assert_eq!(
        lp["song_uri"],
        "https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69"
    );
    // Versions are other recordings only; the page itself and MusicBrainz are not "also on"
    let versions = so["versions"].as_array().unwrap();
    assert_eq!(versions.len(), 1, "{so}");
    assert_eq!(versions[0]["page"], format!("lib-{}", live.0));
    assert!(so["also_on"]
        .as_array()
        .unwrap()
        .iter()
        .all(|v| v["source"] != "musicbrainz" && v["page"] != so["id"]));
    assert_eq!(so["album"]["id"], format!("lib-{aid}"));
    assert!(
        so["about"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["label"] == "File"),
        "library facts in About"
    );
    let (st, _) = call(&app, "GET", "/v1/item/playlist/x", None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

/// Search finds Pubky tags (STEWARD-FIX-search-tags.md): a term that only a tag matches
/// still finds the song; `#a #b` asks for both tags; suggestions list yours first.
#[tokio::test]
async fn search_finds_songs_by_their_pubky_tags() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let card = |id: &str, name: &str| serde_json::json!({"id": id, "uri": format!("https://www.deezer.com/track/{id}"), "status": "ok", "type": "MusicRecording", "name": name, "site": "Deezer"});
    let mock = Router::new()
        .route(
            "/v1/search",
            get(
                move |Query(q): Query<std::collections::HashMap<String, String>>| {
                    let items = match (
                        q.get("q"),
                        q.get("tags").map(String::as_str),
                        q.get("match").map(String::as_str),
                    ) {
                        (Some(_), _, _) => vec![],
                        (None, Some("chill"), None) => vec![card("1", "Tagged Only")],
                        (None, Some("chill,90s"), Some("all")) => vec![card("2", "Both Tags")],
                        _ => vec![],
                    };
                    async move { Json(serde_json::json!({"items": items})) }
                },
            ),
        )
        .route(
            "/nexus/v0/search/tags/by_prefix/{p}",
            get(|| async { Json(serde_json::json!(["chill", "chicago"])) }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let pubky = pm_pubky::config::PmPubkyConfig {
        pinakes_base: format!("http://127.0.0.1:{port}"),
        nexus_base: format!("http://127.0.0.1:{port}/nexus"),
        user_agent_contact: "test@example.org".into(),
        ..Default::default()
    };
    let s = state_with(dir.path(), None, pubky).await;
    s.set_contact("test@example.org").unwrap();
    let tid = {
        let lib = s.library.lock().unwrap();
        let mut tr = untagged_track(dir.path().join("c.flac"), "Calm Song", "Someone");
        tr.canonical = Some((
            pm_model::MusicUri::Isrc {
                isrc: pm_model::Isrc::parse("USAAA1111111").unwrap(),
            },
            pm_model::Confidence::TaggedIsrc,
        ));
        lib.upsert_track(&tr).unwrap().0
    };
    let app = router(s.clone());
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/tags",
        Some(serde_json::json!({"track_id": tid, "label": "chill"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let lines = |app: Router, uri: String| async move {
        let resp = app
            .oneshot(
                axum::http::Request::get(uri)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        String::from_utf8_lossy(&body)
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| v.get("source").is_some())
            .collect::<Vec<_>>()
    };
    let ans = lines(app.clone(), "/v1/search?q=chill&in=pubky".into()).await;
    let pubky = ans
        .iter()
        .find(|a| a["source"] == "pubky")
        .expect("pubky answered");
    assert_eq!(pubky["hits"][0]["title"], "Tagged Only", "{pubky}");
    assert_eq!(pubky["hits"][0]["tagged"], "#chill");
    let ans = lines(app.clone(), "/v1/search?q=%23chill%20%2390s".into()).await;
    let srcs: Vec<&str> = ans.iter().filter_map(|a| a["source"].as_str()).collect();
    // tags only: the library, Pubky's cards and posts carrying the tag
    assert!(
        srcs.iter()
            .all(|x| *x == "library" || *x == "pubky" || *x == "posts"),
        "{srcs:?}"
    );
    let pubky = ans.iter().find(|a| a["source"] == "pubky").unwrap();
    assert_eq!(pubky["hits"][0]["title"], "Both Tags", "{pubky}");
    let ans = lines(app.clone(), "/v1/search?q=%23chill".into()).await;
    let lib = ans.iter().find(|a| a["source"] == "library").unwrap();
    assert_eq!(
        lib["hits"][0]["title"], "Calm Song",
        "your own tag, offline: {lib}"
    );
    let (_, sug) = call(&app, "GET", "/v1/search/tags?prefix=chi", None).await;
    assert_eq!(sug["mine"][0]["label"], "chill", "{sug}");
    assert_eq!(sug["others"], serde_json::json!(["chicago"]));
}

/// A pasted link on a Pinakes without `by-uri` (it answers 400): the card comes from `lookup`
/// with the "also on" list it carries, in two requests, not three.
#[tokio::test]
async fn a_pasted_link_on_a_pinakes_without_by_uri_takes_two_requests() {
    let seen: Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    const LINK: &str = "https://open.spotify.com/track/2Foc5Q5nqNiosCNqttzHof";
    let (s1, s2, s3) = (seen.clone(), seen.clone(), seen.clone());
    let mock = Router::new()
        .route(
            "/v1/resources/lookup",
            post(move |Json(body): Json<serde_json::Value>| {
                s1.lock().unwrap().push(format!("lookup {}", body["uris"]));
                async move {
                    Json(serde_json::json!([{
                        "id": "r1", "uri": LINK, "status": "ok", "type": "MusicRecording", "name": "Get Lucky", "site": "Spotify",
                        "also_on": [{"uri": "https://tidal.com/track/20115557", "site": "TIDAL"}]
                    }]))
                }
            }),
        )
        // (an old daemon reads "by-uri" as a resource id)
        .route(
            "/v1/resources/{id}",
            get(move |axum::extract::Path(id): axum::extract::Path<String>| {
                s2.lock().unwrap().push(format!("resource {id}"));
                async move {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({"error": "invalid resource ID"})),
                    )
                }
            }),
        )
        .route(
            "/v1/search",
            get(move || {
                s3.lock().unwrap().push("search".to_string());
                async move { Json(serde_json::json!({"items": []})) }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let pubky = pm_pubky::config::PmPubkyConfig {
        pinakes_base: format!("http://127.0.0.1:{port}"),
        nexus_base: format!("http://127.0.0.1:{port}/nexus"),
        user_agent_contact: "test@example.org".into(),
        ..Default::default()
    };
    let s = state_with(dir.path(), None, pubky).await;
    s.set_contact("test@example.org").unwrap();
    let resp = router(s.clone())
        .oneshot(
            axum::http::Request::get(
                "/v1/search?q=https%3A%2F%2Fopen.spotify.com%2Ftrack%2F2Foc5Q5nqNiosCNqttzHof&in=pubky",
            )
            .body(axum::body::Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let a = String::from_utf8_lossy(&body)
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| v["source"] == "pubky")
        .expect("pubky answered");
    assert_eq!(a["status"], "ok", "{a}");
    assert_eq!(a["hits"][0]["title"], "Get Lucky", "{a}");
    assert_eq!(a["hits"][0]["exact"], true, "{a}");
    assert_eq!(a["hits"][0]["also"], serde_json::json!(["TIDAL"]), "{a}");
    let reqs = seen.lock().unwrap().clone();
    assert_eq!(
        reqs,
        vec!["resource by-uri".to_string(), format!("lookup [\"{LINK}\"]")],
        "by-uri refused, then one lookup"
    );
}

/// Profiles (STEWARD-UX-PEOPLE.md): cards in one batch, a person's page with what they
/// tagged, avatars served by the node, and the faces behind a song's tags.
#[tokio::test]
async fn profiles_come_from_nexus_and_tags_show_faces() {
    const A: &str = "gujx6qd8ksydh1makdphd3bxu351d9b8waqka8hfg6q7hnqkxexo";
    const B: &str = "pxnu33x7jtpx9ar1ytsi4yxbp6a5o36gwhffs8zoxmbuptici1jy";
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let batches = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let b2 = batches.clone();
    let user = |id: &str| serde_json::json!({"details": {"id": id, "name": "John", "bio": "music", "image": format!("pubky://{id}/pub/pubky.app/files/1"), "links": [{"title": "web", "url": "https://synonym.to"}]}, "counts": {"followers": 3, "following": 2}});
    let mock = Router::new()
        .route(
            "/nexus/v0/stream/users/by_ids",
            post(move |Json(b): Json<serde_json::Value>| {
                b2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let ids: Vec<String> = serde_json::from_value(b["user_ids"].clone()).unwrap();
                async move { Json(serde_json::Value::Array(ids.iter().filter(|i| *i == A).map(|i| user(i)).collect())) }
            }),
        )
        .route("/nexus/v0/user/{id}", get(move |Path(id): Path<String>| async move { Json(user(&id)) }))
        .route("/nexus/static/avatar/{id}", get(|| async { ([(header::CONTENT_TYPE, "image/png")], vec![0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3]) }))
        .route(
            "/nexus/v0/resource/by-uri",
            get(|| async { Json(serde_json::json!({"resource": {"id": "r1", "uri": "https://musicbrainz.org/isrc/USAAA1111111"}, "tags": [{"label": "chill", "taggers": [A, B], "taggers_count": 2}]})) }),
        )
        .route(
            "/v1/search",
            get(|Query(q): Query<std::collections::HashMap<String, String>>| async move {
                assert_eq!(q.get("author").map(String::as_str), Some(A));
                Json(serde_json::json!({"items": [{"id": "c1", "uri": "https://www.deezer.com/track/9", "status": "ok", "type": "MusicRecording", "name": "Calm Song", "site": "Deezer"}],
                    "facets": {"tags": [{"label": "chill", "count": 4}]}}))
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let pubky = pm_pubky::config::PmPubkyConfig {
        pinakes_base: format!("http://127.0.0.1:{port}"),
        nexus_base: format!("http://127.0.0.1:{port}/nexus"),
        user_agent_contact: "test@example.org".into(),
        ..Default::default()
    };
    let s = state_with(dir.path(), None, pubky).await;
    s.set_contact("test@example.org").unwrap();
    let app = router(s.clone());
    let (st, cards) = call(&app, "GET", &format!("/v1/profiles?ids={A},{B},nonsense"), None).await;
    assert_eq!(st, StatusCode::OK, "{cards}");
    assert_eq!(cards[0]["name"], "John");
    assert_eq!(cards[0]["avatar"], format!("/v1/profiles/{A}/avatar"));
    assert_eq!(cards[1]["name"], "pxnu…i1jy", "not indexed: a short key");
    assert_eq!(cards[1]["known"], false);
    assert_eq!(cards.as_array().unwrap().len(), 2, "invalid keys are dropped");
    // Cached: a second ask makes no call
    call(&app, "GET", &format!("/v1/profiles?ids={A},{B}"), None).await;
    assert_eq!(batches.load(std::sync::atomic::Ordering::SeqCst), 1);
    let (_, pg) = call(&app, "GET", &format!("/v1/profiles/{A}"), None).await;
    assert_eq!(pg["bio"], "music");
    assert_eq!(pg["tags"][0][0], "chill");
    assert_eq!(pg["tagged"][0]["title"], "Calm Song");
    assert_eq!(pg["profile_url"], format!("https://pubky.app/profile/{A}"));
    let req = Request::builder()
        .uri(format!("/v1/profiles/{A}/avatar"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.headers()["content-type"], "image/png");
    assert!(
        dir.path().join("art/people").join(A).is_file(),
        "kept on the node"
    );
    // Following needs a Pubky sign-in
    let (st, _) = call(&app, "PUT", &format!("/v1/profiles/{A}/follow"), None).await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED);
    // The faces behind a tag
    let (_, op) = call(
        &app,
        "GET",
        "/v1/discover/uri?uri=https%3A%2F%2Fmusicbrainz.org%2Fisrc%2FUSAAA1111111",
        None,
    )
    .await;
    assert_eq!(op["faces"]["chill"], serde_json::json!([A, B]), "{op}");
}

/// Smart playlists (UX review #9): rules over genre, year, favourites, plays and your tags,
/// all or any, previewed while editing, saved, and worked out again each time.
#[tokio::test]
async fn smart_playlists_pick_songs_by_rules() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let ids: Vec<i64> = {
        let lib = s.library.lock().unwrap();
        let mut out = Vec::new();
        for (i, (title, genre, year)) in [
            ("Calm", "Ambient", 1995),
            ("Fast", "Techno", 2020),
            ("Faster", "Techno", 2005),
        ]
        .iter()
        .enumerate()
        {
            let mut t = untagged_track(dir.path().join(format!("{i}.flac")), title, "Someone");
            t.genres = vec![genre.to_string()];
            t.duration_ms = Some(200_000);
            t.album = Some(pm_model::Album {
                id: None,
                title: format!("Album {i}"),
                album_artist: Some("Someone".into()),
                release_mbid: None,
                release_group_mbid: None,
                year: Some(*year),
                upc: None,
                total_discs: None,
            });
            t.canonical = Some((
                pm_model::MusicUri::Isrc {
                    isrc: pm_model::Isrc::parse(&format!("USAAA111111{i}")).unwrap(),
                },
                pm_model::Confidence::TaggedIsrc,
            ));
            out.push(lib.upsert_track(&t).unwrap().0);
        }
        lib.record_play(TrackId(out[2]), Some("z")).unwrap();
        lib.record_play(TrackId(out[2]), Some("z")).unwrap();
        out
    };
    let app = router(s.clone());
    call(
        &app,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"track_id": ids[1]})),
    )
    .await;
    call(
        &app,
        "PUT",
        "/v1/tags",
        Some(serde_json::json!({"track_id": ids[0], "label": "sleep"})),
    )
    .await;
    let titles = |v: &serde_json::Value| -> Vec<String> {
        let mut t: Vec<String> = v["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["title"].as_str().unwrap().to_string())
            .collect();
        t.sort();
        t
    };
    let (st, r) = call(
        &app,
        "POST",
        "/v1/smart/preview",
        Some(serde_json::json!({"rules": [{"field": "genre", "op": "is", "value": "techno"}]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(titles(&r), ["Fast", "Faster"]);
    let (_, r) = call(&app, "POST", "/v1/smart/preview", Some(serde_json::json!({"rules": [{"field": "genre", "value": "techno"}, {"field": "bookmark", "value": true}]}))).await;
    assert_eq!(titles(&r), ["Fast"], "all rules");
    // A rule saved under the older name still works
    let (_, r) = call(&app, "POST", "/v1/smart/preview", Some(serde_json::json!({"rules": [{"field": "genre", "value": "techno"}, {"field": "favourite", "value": true}]}))).await;
    assert_eq!(titles(&r), ["Fast"], "favourite is bookmark");
    let (_, r) = call(&app, "POST", "/v1/smart/preview", Some(serde_json::json!({"matches": "any", "rules": [{"field": "tag", "value": "#sleep"}, {"field": "plays", "op": "at_least", "value": 2}]}))).await;
    assert_eq!(titles(&r), ["Calm", "Faster"], "any rule");
    let (_, r) = call(
        &app,
        "POST",
        "/v1/smart/preview",
        Some(serde_json::json!({"rules": [{"field": "year", "op": "between", "value": 2000, "to": 2010}]})),
    )
    .await;
    assert_eq!(titles(&r), ["Faster"]);
    let (st, _) = call(
        &app,
        "POST",
        "/v1/smart/preview",
        Some(serde_json::json!({"rules": []})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, p) = call(&app, "POST", "/v1/smart", Some(serde_json::json!({"name": "Techno", "rules": [{"field": "genre", "value": "techno"}], "sort": "most_played"}))).await;
    assert_eq!(st, StatusCode::OK, "{p}");
    let id = p["id"].as_str().unwrap().to_string();
    let (_, list) = call(&app, "GET", "/v1/smart", None).await;
    assert_eq!(list[0]["name"], "Techno");
    let (_, t) = call(&app, "GET", &format!("/v1/smart/{id}/tracks"), None).await;
    assert_eq!(t["tracks"][0]["title"], "Faster", "most played first: {t}");
    assert_eq!(t["matched"], 2);
    let (st, _) = call(&app, "DELETE", &format!("/v1/smart/{id}"), None).await;
    assert_eq!(st, StatusCode::OK);
    let (_, list) = call(&app, "GET", "/v1/smart", None).await;
    assert!(list.as_array().unwrap().is_empty());
    // Seeds: the playlist grows from songs like the seed (same genre first), the seed itself first
    let (st, r) = call(
        &app,
        "POST",
        "/v1/smart/preview",
        Some(serde_json::json!({"seeds": [{"kind": "track", "id": ids[1]}], "sort": "title"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(
        titles(&r),
        ["Calm", "Fast", "Faster"],
        "the whole library is reachable through genre, then artist: {r}"
    );
    // Exclusions: a genre, recently played songs, a song
    let (_, r) = call(&app, "POST", "/v1/smart/preview", Some(serde_json::json!({"seeds": [{"kind": "track", "id": ids[1]}], "exclude": {"genres": ["ambient"]}}))).await;
    assert_eq!(titles(&r), ["Fast", "Faster"]);
    let (_, r) = call(&app, "POST", "/v1/smart/preview", Some(serde_json::json!({"seeds": [{"kind": "track", "id": ids[1]}], "exclude": {"played_within_days": 1}}))).await;
    assert_eq!(titles(&r), ["Calm", "Fast"], "Faster was played today");
    let (_, r) = call(&app, "POST", "/v1/smart/preview", Some(serde_json::json!({"seeds": [{"kind": "track", "id": ids[1]}], "exclude": {"tracks": [ids[0]], "artists": ["nobody"]}}))).await;
    assert_eq!(titles(&r), ["Fast", "Faster"]);
    // Rules still narrow a seeded pool; a length in minutes caps it
    let (_, r) = call(&app, "POST", "/v1/smart/preview", Some(serde_json::json!({"seeds": [{"kind": "track", "id": ids[1]}], "rules": [{"field": "favourite", "value": true}]}))).await;
    assert_eq!(titles(&r), ["Fast"]);
    let (_, r) = call(
        &app,
        "POST",
        "/v1/smart/preview",
        Some(serde_json::json!({"seeds": [{"kind": "track", "id": ids[1]}], "minutes": 1, "sort": "title"})),
    )
    .await;
    assert_eq!(
        r["tracks"].as_array().unwrap().len(),
        1,
        "one song fills a minute when lengths are unknown: {r}"
    );
    assert_eq!(r["matched"], 3);
    // An artist seed, and a bad kind
    let aid = s
        .library
        .lock()
        .unwrap()
        .list_artists()
        .unwrap()
        .into_iter()
        .find(|a| a.name == "Someone")
        .unwrap()
        .id;
    let (_, r) = call(
        &app,
        "POST",
        "/v1/smart/preview",
        Some(serde_json::json!({"seeds": [{"kind": "artist", "id": aid}]})),
    )
    .await;
    assert_eq!(titles(&r), ["Calm", "Fast", "Faster"]);
    let (st, _) = call(
        &app,
        "POST",
        "/v1/smart/preview",
        Some(serde_json::json!({"seeds": [{"kind": "mood", "id": 1}]})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    // Saved with its seeds and exclusions
    let (st, p) = call(&app, "POST", "/v1/smart", Some(serde_json::json!({"name": "Like Fast", "seeds": [{"kind": "track", "id": ids[1], "label": "Fast"}], "exclude": {"genres": ["ambient"]}, "minutes": 90}))).await;
    assert_eq!(st, StatusCode::OK, "{p}");
    assert_eq!(
        (
            p["seeds"][0]["label"].clone(),
            p["minutes"].clone(),
            p["exclude"]["genres"][0].clone()
        ),
        (
            serde_json::json!("Fast"),
            serde_json::json!(90),
            serde_json::json!("ambient")
        )
    );
    let (_, t) = call(
        &app,
        "GET",
        &format!("/v1/smart/{}/tracks", p["id"].as_str().unwrap()),
        None,
    )
    .await;
    assert_eq!(titles(&t), ["Fast", "Faster"]);
}

/// One search, streamed per source: the library answers first, a slow
/// source still arrives, a failing one reports itself without holding
/// the others back, and a second ask comes from the cache.
#[tokio::test]
async fn search_streams_each_source_as_it_answers() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let track = serde_json::json!({"id": 3135556, "title": "Bohemian Rhapsody", "isrc": "GBUM71029604", "duration": 355, "preview": "http://x/p.mp3",
        "artist": {"id": 412, "name": "Queen"}, "album": {"id": 1, "title": "A Night at the Opera", "cover_medium": "http://img/o.jpg"}});
    let mock = Router::new()
        .route(
            "/search/track",
            get(move || {
                let t = track.clone();
                async move {
                    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                    Json(serde_json::json!({"data": [t]}))
                }
            }),
        )
        .route(
            "/search/album",
            get(|| async { Json(serde_json::json!({"data": []})) }),
        )
        .route(
            "/search/artist",
            get(|| async { Json(serde_json::json!({"data": []})) }),
        )
        .route(
            "/v1/search",
            get(|| async {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    r#"{"error":"index rebuilding"}"#,
                )
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let pubky = pm_pubky::config::PmPubkyConfig {
        pinakes_base: format!("http://127.0.0.1:{port}"),
        nexus_base: format!("http://127.0.0.1:{port}/nexus"),
        user_agent_contact: "test@example.org".into(),
        ..Default::default()
    };
    let s = state_with(dir.path(), None, pubky).await;
    s.set_contact("test@example.org").unwrap();
    *s.deezer_base.write().unwrap() = format!("http://127.0.0.1:{port}");
    {
        let lib = s.library.lock().unwrap();
        lib.upsert_track(&untagged_track(
            dir.path().join("b.flac"),
            "Bohemian Rhapsody",
            "Queen",
        ))
        .unwrap();
    }
    let app = router(s.clone());
    let ask = |app: Router| async move {
        let resp = app
            .oneshot(
                axum::http::Request::get("/v1/search?q=bohemian")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.headers()["content-type"], "application/x-ndjson");
        let body = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        String::from_utf8(body.to_vec())
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str::<search::SourceAnswer>(l).unwrap())
            .collect::<Vec<_>>()
    };
    let lines = ask(app.clone()).await;
    let order: Vec<&str> = lines.iter().map(|a| a.source.as_str()).collect();
    // people and posts (Nexus) answer too; the three this test watches are the rest
    let core: Vec<&str> = order
        .iter()
        .copied()
        .filter(|s| *s != "people" && *s != "posts")
        .collect();
    assert_eq!(core.len(), 3, "{order:?}");
    assert_eq!(order[0], "library", "the library answers first: {order:?}");
    let lib = &lines[0];
    assert_eq!(lib.status, "ok");
    assert_eq!(lib.hits[0].kind, "song");
    assert_eq!(lib.hits[0].title, "Bohemian Rhapsody");
    let dz = lines.iter().find(|a| a.source == "deezer").unwrap();
    assert_eq!(dz.status, "ok");
    assert!(dz.ms >= 500, "the slow source: {} ms", dz.ms);
    assert_eq!(
        dz.hits[0].keys,
        vec!["isrc:GBUM71029604".to_string()],
        "rows merge by ISRC"
    );
    assert_eq!(dz.hits[0].uri.as_deref(), Some("deezer:track:3135556"));
    let pk = lines.iter().find(|a| a.source == "pubky").unwrap();
    assert_eq!(pk.status, "error", "{pk:?}");
    assert!(pk.error.as_deref().unwrap().contains("index rebuilding"));
    // Scoped to one source, and cached the second time.
    let again = ask(app.clone()).await;
    assert!(again.iter().find(|a| a.source == "deezer").unwrap().cached);
    let resp = app
        .oneshot(
            axum::http::Request::get("/v1/search?q=bohemian&in=library")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(
        String::from_utf8_lossy(&body).lines().count(),
        1,
        "in=library asks only the library"
    );
}

/// Home: one Pinakes call per section; empty sections say why, a failing
/// one carries its own error, signed-out and city-less sections ask for
/// what they need, and a tag page returns co-tags.
#[tokio::test]
async fn home_sections_answer_items_reasons_or_their_own_error() {
    let chart = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-pubky/tests/fixtures/pinakes/music/search-trending-tracks-7d.json"
    ))
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = Router::new()
        .route(
            "/v1/search",
            get(move |Query(q): Query<std::collections::HashMap<String, String>>| {
                let chart = chart.clone();
                async move {
                    let t = q.get("type").map(String::as_str);
                    let body = match (t, q.get("sort").map(String::as_str), q.get("facet").map(String::as_str)) {
                        (Some("MusicRecording"), Some("trending"), _) => chart,
                        (Some("MusicAlbum"), Some("trending"), _) => r#"{"items":[],"cursor":null}"#.into(),
                        (Some("MusicGroup"), _, _) => return (StatusCode::INTERNAL_SERVER_ERROR, r#"{"error":"index rebuilding"}"#.to_string()),
                        (Some("MusicRecording"), None, Some("tags")) => r#"{"items":[],"facets":{"tags":[{"label":"hip-hop","count":12},{"label":"german","count":4}]}}"#.into(),
                        (None, None, Some("tags")) => r#"{"items":[],"facets":{"tags":[{"label":"hip-hop","count":12},{"label":"rap","count":7}]}}"#.into(),
                        _ => r#"{"items":[]}"#.into(),
                    };
                    (StatusCode::OK, body)
                }
            }),
        )
        .route(
            "/search",
            get(|| async { Json(serde_json::json!([{"name": "Zürich", "lat": "47.3744", "lon": "8.5410"}])) }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let pubky = pm_pubky::config::PmPubkyConfig {
        pinakes_base: format!("http://127.0.0.1:{port}"),
        nexus_base: format!("http://127.0.0.1:{port}/nexus"),
        user_agent_contact: "test@example.org".into(),
        ..Default::default()
    };
    let s = state_with(dir.path(), None, pubky).await;
    s.set_contact("test@example.org").unwrap();
    *s.nominatim_base.write().unwrap() = format!("http://127.0.0.1:{port}");
    let app = router(s.clone());
    let (st, songs) = call(&app, "GET", "/v1/home/trending-songs?window=7d", None).await;
    assert_eq!(st, StatusCode::OK, "{songs}");
    // The recorded chart lists the same songs more than once (two groups, a remaster): merged
    let items = songs["items"].as_array().unwrap();
    assert_eq!(items.len(), 6, "{songs}");
    assert_eq!(
        items
            .iter()
            .map(|i| i["versions"].as_u64().unwrap_or(0))
            .sum::<u64>(),
        2
    );
    assert!(songs["note"].is_null());
    let (_, albums) = call(&app, "GET", "/v1/home/trending-albums", None).await;
    assert!(albums["items"].as_array().unwrap().is_empty());
    assert_eq!(albums["note"], "No albums were tagged in this window.");
    let (st, artists) = call(&app, "GET", "/v1/home/trending-artists", None).await;
    assert_eq!(
        st,
        StatusCode::OK,
        "a failing section is still a 200 with its own error"
    );
    assert!(
        artists["error"].as_str().unwrap().contains("index rebuilding"),
        "{artists}"
    );
    let (_, following) = call(&app, "GET", "/v1/home/following", None).await;
    assert_eq!(following["needs"], "pubky", "signed out: {following}");
    let (_, tags) = call(&app, "GET", "/v1/home/tags", None).await;
    assert_eq!(tags["tags"][0][0], "hip-hop");
    let (_, concerts) = call(&app, "GET", "/v1/home/concerts", None).await;
    assert_eq!(concerts["needs"], "location");
    let (st, loc) = call(
        &app,
        "PUT",
        "/v1/home/location",
        Some(serde_json::json!({"city": "Zurich"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{loc}");
    assert_eq!(loc["label"], "Zürich");
    let (_, concerts) = call(&app, "GET", "/v1/home/concerts", None).await;
    assert_eq!(concerts["note"], "No concerts near you in the next 30 days.");
    let (_, tag) = call(&app, "GET", "/v1/home/tag?labels=hip-hop", None).await;
    assert_eq!(tag["tags"][1][0], "rap", "co-tags on the tag page: {tag}");
    let (st, _) = call(&app, "GET", "/v1/home/nope", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[test]
fn service_links_are_recognised() {
    use discover::service_link;
    let l = service_link("https://tidal.com/album/495402659", true).unwrap();
    assert_eq!(
        (l.service, l.kind, l.id.as_str()),
        ("tidal", "album", "495402659")
    );
    let l = service_link("https://www.deezer.com/en/track/4329888262", false).unwrap();
    assert_eq!((l.service, l.kind), ("deezer", "track"));
    let l = service_link("https://open.spotify.com/track/2OBofMJx94NryV2SK8p8Zf", false).unwrap();
    assert_eq!(
        (l.service, l.kind, l.id.as_str()),
        ("spotify", "track", "2OBofMJx94NryV2SK8p8Zf")
    );
    assert!(service_link("https://music.apple.com/us/album/dracula/1830000000", true).is_none());
    assert!(service_link(
        "https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69",
        true
    )
    .is_none());
}

/// Pinakes's `also_on` joins the Versions list: every service that has
/// the same recording, tagged links first, Deezer playable as a preview.
#[tokio::test]
async fn versions_include_pinakes_also_on_links() {
    let fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-pubky/tests/fixtures/pinakes/music/search-key-isrc.json"
    ))
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen2 = seen.clone();
    let mock = Router::new().route(
        "/v1/search",
        get(
            move |Query(q): Query<std::collections::HashMap<String, String>>| {
                let fixture = fixture.clone();
                let seen = seen2.clone();
                async move {
                    seen.lock()
                        .unwrap()
                        .push(q.get("key").cloned().unwrap_or_default());
                    ([(header::CONTENT_TYPE, "application/json")], fixture)
                }
            },
        ),
    );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let pubky = pm_pubky::config::PmPubkyConfig {
        pinakes_base: format!("http://127.0.0.1:{port}"),
        nexus_base: format!("http://127.0.0.1:{port}/nexus"),
        user_agent_contact: "test@example.org".into(),
        ..Default::default()
    };
    let s = state_with(dir.path(), None, pubky).await;
    s.set_contact("test@example.org").unwrap();
    let tid = {
        let lib = s.library.lock().unwrap();
        let mbid = pm_model::Mbid::parse("b1a9c0e9-d987-4042-ae91-78d6a3267d69").unwrap();
        let t = pm_model::Track {
            id: None,
            title: "Bohemian Rhapsody".into(),
            artists: vec![pm_model::Artist {
                id: None,
                name: "Queen".into(),
                mbid: None,
            }],
            album: None,
            track_no: Some(11),
            disc_no: None,
            duration_ms: Some(355_000),
            file: pm_model::AudioFile {
                path: dir.path().join("bo.flac"),
                size_bytes: 1,
                mtime_unix: 0,
                container: "flac".into(),
                format: pm_model::AudioFormat::pcm(44_100, 16, 2),
                duration_ms: Some(355_000),
            },
            recording_mbid: Some(mbid),
            isrc: None,
            acoustid: None,
            canonical: Some((
                pm_model::MusicUri::Recording { mbid },
                pm_model::Confidence::TaggedMbid,
            )),
            replaygain_track_gain_db: None,
            replaygain_album_gain_db: None,
            genres: vec![],
        };
        lib.upsert_track(&t).unwrap().0
    };
    let app = router(s.clone());
    let (st, v) = call(&app, "GET", &format!("/v1/library/tracks/{tid}/versions"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        ["mbid:b1a9c0e9-d987-4042-ae91-78d6a3267d69"]
    );
    let sources: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["source"].as_str().unwrap())
        .collect();
    assert!(
        sources.contains(&"spotify") && sources.contains(&"youtube") && sources.contains(&"deezer"),
        "{v}"
    );
    assert!(
        sources.contains(&"tidal"),
        "another TIDAL page is still a version"
    );
    let deezer = v
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["source"] == "deezer")
        .unwrap();
    assert_eq!(deezer["uri"], "deezer:track:4329888262");
    assert_eq!(deezer["label"], "Deezer", "a chip names the source");
    // No page twice, even though every item repeats the group's links.
    let mut urls: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["url"].as_str().unwrap())
        .collect();
    let n = urls.len();
    urls.sort();
    urls.dedup();
    assert_eq!(urls.len(), n, "{v}");
    assert!(v
        .as_array()
        .unwrap()
        .iter()
        .all(|x| x["source"] != "spotify" || x["uri"].is_null()));
    // Cached: a second read does not ask Pinakes again.
    let _ = call(&app, "GET", &format!("/v1/library/tracks/{tid}/versions"), None).await;
    assert_eq!(seen.lock().unwrap().len(), 1);
}

/// Artist pages fall back to fanart.tv (the project's key) for a portrait and to TheAudioDB for
/// a portrait and a biography when Wikidata and Wikipedia have none.
#[tokio::test]
async fn artist_images_fall_back_to_fanart_and_theaudiodb() {
    let mbid = "056e4f3e-d505-4dad-8ec1-d04f521cbb56";
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let png: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3];
    let b2 = base.clone();
    let mock = Router::new()
        .route(&format!("/ws/2/artist/{mbid}"), get(|| async { Json(serde_json::json!({"id": "x", "relations": []})) }))
        .route(&format!("/v3/music/{mbid}"), get({ let b = b2.clone(); move |q: axum::extract::RawQuery| { let b = b.clone(); async move {
            assert!(q.0.unwrap_or_default().contains("api_key=fanart-test-key"));
            Json(serde_json::json!({"name": "Daft Punk", "artistthumb": [
                {"id": "1", "url": format!("{b}/img/less.png"), "likes": "2"},
                {"id": "2", "url": format!("{b}/img/best.png"), "likes": "7"}]}))
        } } }))
        .route("/api/v1/json/123/artist-mb.php", get(move |q: axum::extract::RawQuery| async move {
            assert!(q.0.unwrap_or_default().contains(&format!("i={mbid}")));
            Json(serde_json::json!({"artists": [{"strArtist": "Daft Punk", "strArtistThumb": format!("{b2}/img/adb.png"), "strBiography": "Daft Punk were a French electronic music duo."}]}))
        }))
        .route("/img/{name}", get({ let png = png.clone(); move |Path(name): Path<String>| { let png = png.clone(); async move {
            assert_eq!(name, "best.png", "the most liked portrait");
            ([(axum::http::header::CONTENT_TYPE, "image/png")], png)
        } } }));
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    {
        let mut b = s.info_bases.write().unwrap();
        b.musicbrainz = base.clone();
        b.wikidata = "http://127.0.0.1:9".into();
        b.wikipedia = "http://127.0.0.1:9".into();
        b.fanart = base.clone();
        b.audiodb = base.clone();
        b.fanart_key = Some("fanart-test-key".into());
    }
    s.set_contact("tests@example.org").ok();
    let aid = {
        let lib = s.library.lock().unwrap();
        let mut t = untagged_track(dir.path().join("song.flac"), "Song", "Daft Punk");
        t.artists[0].mbid = Some(pm_model::Mbid::parse(mbid).unwrap());
        lib.upsert_track(&t).unwrap();
        lib.list_artists().unwrap()[0].id
    };
    let app = router(s.clone());
    let mut info = serde_json::Value::Null;
    for _ in 0..50 {
        info = call(&app, "GET", &format!("/v1/library/artists/{aid}/info"), None)
            .await
            .1;
        if info["image"] == true || info["bio"].is_string() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(info["image"], true, "{info}");
    assert_eq!(info["image_from"], "fanart.tv");
    assert_eq!(info["bio_from"], "TheAudioDB");
    assert!(
        info["bio"].as_str().unwrap().starts_with("Daft Punk were"),
        "{info}"
    );
    assert!(info["note"].is_null(), "{info}");
    let req = Request::builder()
        .uri(format!("/v1/library/artists/{aid}/image"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .len(),
        11
    );
    assert_eq!(
        artist_info::fanart_thumb(&serde_json::json!({"artistbackground": [{"url": "u", "likes": "1"}]}))
            .as_deref(),
        Some("u")
    );
    assert!(artist_info::fanart_thumb(&serde_json::json!({})).is_none());
}

/// Bookmarks (social v1 naming, 2026-10-26): `/v1/favourites` answers the same as
/// `/v1/bookmarks` for one release, marked deprecated.
#[tokio::test]
async fn favourites_is_a_deprecated_name_for_bookmarks() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(state(dir.path(), None).await);
    let uri = "https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69";
    let (st, r) = call(
        &app,
        "PUT",
        "/v1/favourites",
        Some(serde_json::json!({"uri": uri})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(
        (r["bookmarked"].as_bool(), r["favourite"].as_bool()),
        (Some(true), Some(true))
    );
    let (_, new) = call(&app, "GET", "/v1/bookmarks", None).await;
    let (_, old) = call(&app, "GET", "/v1/favourites", None).await;
    assert_eq!(new, old);
    assert_eq!(new["uris"], serde_json::json!([uri]));
    let get = |path: &'static str| Request::builder().uri(path).body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(get("/v1/favourites")).await.unwrap();
    assert_eq!(resp.headers()["deprecation"], "true");
    assert!(resp.headers()["link"].to_str().unwrap().contains("/v1/bookmarks"));
    let resp = app.clone().oneshot(get("/v1/bookmarks")).await.unwrap();
    assert!(resp.headers().get("deprecation").is_none());
    let (st, r) = call(&app, "DELETE", &format!("/v1/bookmarks?uri={uri}"), None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["bookmarked"], false);
}

/// Comments (TASK-pinakes-social-ux.md B1) are read from Nexus by parent: the item's Resource
/// replies, newest first, with the authors' names; a rating that names the item as its parent
/// is not a comment; an item Nexus doesn't know has none; a comment's replies come by its post.
/// Writing needs a Pubky sign-in (the testnet test covers writing and deleting).
#[tokio::test]
async fn comments_are_read_from_nexus_by_parent() {
    let a = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";
    let b = "pxnu33x7jtpx9ar1ytsi4yxbp6a5o36gwhffs8zoxmbuptici1jy";
    let song = "https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69";
    let view = |author: &str, id: &str, kind: &str, content: &str, at: i64, replies: u64| {
        serde_json::json!({"details": {"id": id, "author": author, "kind": kind, "content": content,
            "uri": format!("pubky://{author}/pub/social/v1/posts/{id}"), "indexed_at": at,
            "attachments": null}, "counts": {"replies": replies, "tags": 0}, "tags": [],
            "relationships": {"replied": song, "mentioned": []}, "bookmark": null})
    };
    let replies = serde_json::json!([
        view(a, "0034A", "note", "Older", 1_000, 2),
        view(b, "0034C", "note", "Newest", 3_000, 0),
        view(b, "0034R", "rating", "", 2_500, 0),
    ]);
    let thread = serde_json::json!([view(b, "0034D", "note", "A reply", 4_000, 0)]);
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = {
        let seen = seen.clone();
        Router::new()
            .route(
                "/nexus/v0/resource/by-uri",
                get(
                    move |Query(q): Query<std::collections::HashMap<String, String>>| async move {
                        if q["uri"].contains("b1a9c0e9") {
                            Json(serde_json::json!({"resource": {"id": "r1", "uri": q["uri"]}, "tags": []}))
                                .into_response()
                        } else {
                            (
                                StatusCode::NOT_FOUND,
                                Json(serde_json::json!({"error": "not found"})),
                            )
                                .into_response()
                        }
                    },
                ),
            )
            .route(
                "/nexus/v0/resource/{id}/replies",
                get(move || async move { Json(replies) }),
            )
            .route(
                "/nexus/v0/stream/posts",
                get(
                    move |Query(q): Query<std::collections::HashMap<String, String>>| {
                        let seen = seen.clone();
                        async move {
                            seen.lock()
                                .unwrap()
                                .push(format!("{}/{}/{}", q["source"], q["author_id"], q["post_id"]));
                            Json(thread)
                        }
                    },
                ),
            )
            .route(
                "/nexus/v0/stream/users/by_ids",
                post(move || async move { Json(serde_json::json!([{"details": {"id": a, "name": "Ann"}}])) }),
            )
    };
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let pubky = pm_pubky::config::PmPubkyConfig {
        pinakes_base: format!("http://127.0.0.1:{port}"),
        nexus_base: format!("http://127.0.0.1:{port}/nexus"),
        user_agent_contact: "test@example.org".into(),
        ..Default::default()
    };
    let s = state_with(dir.path(), None, pubky).await;
    s.set_contact("test@example.org").unwrap();
    let app = router(s.clone());
    let q = format!(
        "/v1/comments?uri={}",
        url::form_urlencoded::byte_serialize(song.as_bytes()).collect::<String>()
    );
    let (st, t) = call(&app, "GET", &q, None).await;
    assert_eq!(st, StatusCode::OK, "{t}");
    assert_eq!(t["indexed"], true);
    assert_eq!(t["parent"], song);
    let c = t["comments"].as_array().unwrap();
    assert_eq!(
        c.iter()
            .map(|x| x["content"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["Newest", "Older"]
    );
    assert_eq!(c[1]["name"], "Ann");
    assert_eq!(c[1]["replies"], 2);
    assert_eq!(c[1]["post"], format!("pubky://{a}/pub/social/v1/posts/0034A"));
    assert_eq!(c[1]["mine"], false, "not signed in: nothing is mine");
    // A comment's replies, by its post
    let (st, r) = call(
        &app,
        "GET",
        &format!("/v1/comments?post=pubky://{a}/pub/social/v1/posts/0034A"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["comments"][0]["content"], "A reply");
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        [format!("post_replies/{a}/0034A")]
    );
    // An item Nexus has no Resource for has no comments yet
    let (_, none) = call(
        &app,
        "GET",
        "/v1/comments?uri=https%3A%2F%2Ftidal.com%2Ftrack%2F1",
        None,
    )
    .await;
    assert_eq!(
        (
            none["indexed"].as_bool(),
            none["comments"].as_array().map(Vec::len)
        ),
        (Some(true), Some(0))
    );
    // Writing needs a Pubky sign-in; a bad link or a missing item is a 400
    let (st, e) = call(
        &app,
        "POST",
        "/v1/comments",
        Some(serde_json::json!({"uri": song, "content": "Hi"})),
    )
    .await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED, "{e}");
    // "Post about this" too (B2)
    let (st, e) = call(
        &app,
        "POST",
        "/v1/posts",
        Some(serde_json::json!({"uri": song, "content": "Listen"})),
    )
    .await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED, "{e}");
    assert_eq!(
        call(&app, "GET", "/v1/comments", None).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(&app, "GET", "/v1/comments?post=https://example.org/x", None)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &app,
            "DELETE",
            &format!("/v1/comments?post=pubky://{a}/pub/social/v1/posts/0034A"),
            None
        )
        .await
        .0,
        StatusCode::PRECONDITION_FAILED
    );
}

/// `GET /v1/cards` (TASK-pinakes-social-ux.md B3): the Pinakes card for a collection item the
/// node doesn't play (a web page), with only an http(s) image; a link Pinakes can't read is
/// `unknown`, never an error.
#[tokio::test]
async fn a_web_link_has_its_pinakes_card() {
    // Whether Pinakes was ever asked about an import's search link (it must not be)
    static ASKED_SEARCH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = Router::new()
        .route(
            "/v1/resources/by-uri",
            get(
                |Query(q): Query<std::collections::HashMap<String, String>>| async move {
                    if q["uri"].contains("musicbrainz.org/search") {
                        ASKED_SEARCH.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    match q["uri"].as_str() {
                        "https://example.com/liner-notes" => Json(serde_json::json!({
                            "id": "c1", "uri": q["uri"], "status": "ok", "type": "Article",
                            "name": "Liner notes", "description": "How it was made",
                            "image": "https://example.com/cover.jpg", "site": "example.com"
                        }))
                        .into_response(),
                        "https://example.com/odd" => Json(serde_json::json!({
                            "id": "c2", "uri": q["uri"], "status": "ok", "name": "Odd",
                            "image": "javascript:alert(1)"
                        }))
                        .into_response(),
                        _ => (
                            StatusCode::NOT_FOUND,
                            Json(serde_json::json!({"error": "not found"})),
                        )
                            .into_response(),
                    }
                },
            ),
        )
        .route(
            "/v1/resources/lookup",
            post(|body: String| async move {
                if body.contains("musicbrainz.org/search") {
                    ASKED_SEARCH.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                Json(serde_json::json!([]))
            }),
        )
        .route(
            "/v1/resolve",
            post(|body: String| async move {
                if body.contains("musicbrainz.org/search") {
                    ASKED_SEARCH.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::json!({"error": "cannot read"})),
                )
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let pubky = pm_pubky::config::PmPubkyConfig {
        pinakes_base: format!("http://127.0.0.1:{port}"),
        nexus_base: format!("http://127.0.0.1:{port}/nexus"),
        user_agent_contact: "test@example.org".into(),
        ..Default::default()
    };
    let s = state_with(dir.path(), None, pubky).await;
    s.set_contact("test@example.org").unwrap();
    let app = router(s.clone());
    let (st, c) = call(
        &app,
        "GET",
        "/v1/cards?uri=https%3A%2F%2Fexample.com%2Fliner-notes",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{c}");
    assert_eq!(
        c,
        serde_json::json!({"uri": "https://example.com/liner-notes", "status": "found", "kind": "Article",
            "name": "Liner notes", "description": "How it was made",
            "image": "https://example.com/cover.jpg", "site": "example.com"})
    );
    let (_, odd) = call(&app, "GET", "/v1/cards?uri=https%3A%2F%2Fexample.com%2Fodd", None).await;
    assert_eq!(odd["name"], "Odd");
    assert!(odd["image"].is_null(), "only an http(s) image: {odd}");
    let (st, none) = call(
        &app,
        "GET",
        "/v1/cards?uri=https%3A%2F%2Fexample.com%2Fnothing",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{none}");
    assert!(
        matches!(none["status"].as_str(), Some("unknown" | "reading")),
        "{none}"
    );
    assert_eq!(
        call(&app, "GET", "/v1/cards?uri=", None).await.0,
        StatusCode::BAD_REQUEST
    );
    // An import's search link holds a private playlist's words: answered here, never asked of
    // Pinakes (review of #156)
    let search = "https://musicbrainz.org/search?type=recording&query=Nobody+Missing";
    let (st, c) = call(
        &app,
        "GET",
        &format!(
            "/v1/cards?uri={}",
            url::form_urlencoded::byte_serialize(search.as_bytes()).collect::<String>()
        ),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{c}");
    assert_eq!(c, serde_json::json!({"uri": search, "status": "unknown"}));
    // Nor when a playlist that has one is opened: its sources are found without it (review
    // of #156 at ed1ff43: /play-from looked it up, then had Pinakes read it)
    let (st, pl) = call(
        &app,
        "POST",
        "/v1/playlists",
        Some(serde_json::json!({"name": "Imported", "uris": [search, "https://example.com/nothing"]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{pl}");
    let id = pl["summary"]["id"].as_str().unwrap().to_string();
    let (st, pf) = call(&app, "GET", &format!("/v1/playlists/{id}/play-from"), None).await;
    assert_eq!(st, StatusCode::OK, "{pf}");
    assert_eq!(pf["items"].as_array().map(Vec::len), Some(2), "{pf}");
    let (_, _) = call(
        &app,
        "POST",
        "/v1/discover/describe",
        Some(serde_json::json!({"uris": [search]})),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(
        !ASKED_SEARCH.load(std::sync::atomic::Ordering::SeqCst),
        "Pinakes saw the search words"
    );
}

/// Smart playlists with tags from the network (TASK-pinakes-social-ux.md E2): `reach` other than
/// `me` asks Pinakes' generic search for music tagged with the tags in that reach, and Nexus by
/// URI for who tagged it (five named, the rest counted, labels in canonical (sorted) order). Items
/// that play here come first; the answer is cached; a personal reach needs a Pubky sign-in; a
/// plain smart playlist is unchanged.
#[tokio::test]
async fn smart_playlists_take_tags_from_the_network() {
    let a = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";
    let searches = Arc::new(std::sync::Mutex::new(Vec::<
        std::collections::HashMap<String, String>,
    >::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    // Pinakes down: it answers 503
    let down = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mock = {
        let searches = searches.clone();
        let down = down.clone();
        Router::new()
            .route(
                "/v1/search",
                get(move |Query(q): Query<std::collections::HashMap<String, String>>| {
                    let searches = searches.clone();
                    let down = down.clone();
                    async move {
                        searches.lock().unwrap().push(q);
                        if down.load(std::sync::atomic::Ordering::SeqCst) {
                            return Err(StatusCode::SERVICE_UNAVAILABLE);
                        }
                        Ok(Json(serde_json::json!({"items": [
                            {"id": "c1", "uri": "https://example.org/not-music", "status": "ok", "type": "Article", "name": "Not music"},
                            {"id": "c2", "uri": "https://musicbrainz.org/recording/833f00e1-781f-4edd-90e4-e52712618862", "status": "ok", "type": "MusicRecording", "name": "On MusicBrainz", "site": "musicbrainz.org"},
                            {"id": "c3", "uri": "https://tidal.com/track/50850120", "status": "ok", "type": "MusicRecording", "name": "On TIDAL", "site": "tidal.com"}
                        ], "cursor": null})))
                    }
                }),
            )
            .route(
                "/nexus/v0/resource/by-uri",
                get(move |Query(q): Query<std::collections::HashMap<String, String>>| async move {
                    let taggers: Vec<String> = (0..7).map(|i| if i == 0 { a.to_string() } else { format!("tagger{i}") }).collect();
                    Json(serde_json::json!({"resource": {"id": "r", "uri": q["uri"]}, "tags": [
                        {"label": "house", "taggers": [a], "taggers_count": 1, "relationship": false},
                        // Nexus lists some of the taggers; the label says how many there are
                        {"label": "techno", "taggers": taggers, "taggers_count": 9, "relationship": false},
                        {"label": "other", "taggers": ["x"], "taggers_count": 1, "relationship": false}
                    ]}))
                }),
            )
            .route(
                "/nexus/v0/stream/users/by_ids",
                post(move || async move { Json(serde_json::json!([{"details": {"id": a, "name": "Ann"}}])) }),
            )
    };
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let pubky = pm_pubky::config::PmPubkyConfig {
        pinakes_base: format!("http://127.0.0.1:{port}"),
        nexus_base: format!("http://127.0.0.1:{port}/nexus"),
        user_agent_contact: "test@example.org".into(),
        ..Default::default()
    };
    let s = state_with(dir.path(), None, pubky).await;
    s.set_contact("test@example.org").unwrap();
    let app = router(s.clone());
    let rules = serde_json::json!({"reach": "all", "tags": ["Techno", "house"], "network_limit": 5});
    let (st, r) = call(&app, "POST", "/v1/smart/preview", Some(rules.clone())).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    let q = searches.lock().unwrap()[0].clone();
    assert_eq!(
        (q["tags"].as_str(), q.get("reach"), q.get("viewer")),
        ("house,techno", None, None),
        "{q:?}"
    );
    let net = r["network"].as_array().unwrap();
    assert_eq!(net.len(), 2, "music cards only: {r}");
    // Nothing is connected here: neither plays, so both are cards, in Pinakes' order
    assert!(net.iter().all(|n| n["plays_from"].is_null()), "{r}");
    let tidal = &net[1];
    assert_eq!(tidal["site"], "tidal.com");
    let by = &tidal["tagged_by"];
    assert_eq!(by.as_array().unwrap().len(), 5);
    assert_eq!(
        tidal["taggers_count"], 9,
        "the label's count, not the listed taggers"
    );
    assert_eq!(
        by[0],
        serde_json::json!({"key": a, "name": "Ann", "labels": ["house", "techno"]})
    );
    assert_eq!(by[1], serde_json::json!({"key": "tagger1", "labels": ["techno"]}));
    assert_eq!(tidal["page"], "https://tidal.com/track/50850120");
    // The same answer again comes from the cache
    call(&app, "POST", "/v1/smart/preview", Some(rules.clone())).await;
    assert_eq!(searches.lock().unwrap().len(), 1);
    // One query, as the feed says (review of #159): with a rule, what the network found meets
    // it too; a card nothing can check it on (no genre known) is left out
    let (st, r) = call(
        &app,
        "POST",
        "/v1/smart/preview",
        Some(
            serde_json::json!({"reach": "all", "tags": ["Techno", "house"], "network_limit": 5,
                                "rules": [{"field": "genre", "value": "techno"}]}),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert!(r.get("network").is_none(), "{r}");
    // ...because of the rule, not a failed call (review of #192): the network answered
    assert!(r.get("network_note").is_none(), "{r}");
    assert_eq!(searches.lock().unwrap().len(), 1);
    // Pinakes down once the answer is old: the last answer stays, said to be old (review of
    // #192: the playlist doesn't empty)
    crate::smart_network::age_answers(std::time::Duration::from_secs(600));
    down.store(true, std::sync::atomic::Ordering::SeqCst);
    let (st, r) = call(&app, "POST", "/v1/smart/preview", Some(rules.clone())).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(searches.lock().unwrap().len(), 2, "asked again");
    assert_eq!(r["network"].as_array().map(Vec::len), Some(2), "{r}");
    assert!(r["network_note"].as_str().unwrap().contains("minutes ago"), "{r}");
    down.store(false, std::sync::atomic::Ordering::SeqCst);
    // People you follow need a Pubky sign-in: said in plain words, not an error
    let (st, r) = call(
        &app,
        "POST",
        "/v1/smart/preview",
        Some(serde_json::json!({"reach": "following", "tags": ["techno"]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert!(
        r["network_note"].as_str().unwrap().contains("Sign in with Pubky"),
        "{r}"
    );
    // Refused: tags with "just me", a network reach without tags, every one of four tags
    for bad in [
        serde_json::json!({"reach": "me", "tags": ["techno"], "rules": [{"field": "genre", "op": "contains", "value": "x"}]}),
        serde_json::json!({"reach": "friends"}),
        serde_json::json!({"reach": "all", "tags": ["a", "b", "c", "d"], "match": "all"}),
        serde_json::json!({"reach": "nearby", "tags": ["a"]}),
    ] {
        assert_eq!(
            call(&app, "POST", "/v1/smart/preview", Some(bad.clone())).await.0,
            StatusCode::BAD_REQUEST,
            "{bad}"
        );
    }
    // A plain smart playlist is as before: no network, nothing new in its answer
    let (st, plain) = call(
        &app,
        "POST",
        "/v1/smart/preview",
        Some(serde_json::json!({"rules": [{"field": "genre", "op": "contains", "value": "techno"}]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{plain}");
    assert!(
        plain.get("network").is_none() && plain.get("network_note").is_none(),
        "{plain}"
    );
    // Saving tags from the network writes a music feed: that needs a Pubky sign-in
    let (st, e) = call(
        &app,
        "POST",
        "/v1/smart",
        Some(serde_json::json!({"name": "Techno", "reach": "all", "tags": ["techno"]})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_GATEWAY, "{e}");
    assert!(e["error"].as_str().unwrap().contains("Sign in with Pubky"), "{e}");
}

/// A smart playlist with tags from the network is also a music feed on the owner's homeserver
/// (E2): written on save, rewritten in place on an edit, gone when it goes back to "just me"
/// or is deleted. On a testnet only (`PM_TESTNET=1`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_network_smart_playlist_is_a_music_feed() {
    if std::env::var("PM_TESTNET").as_deref() != Ok("1") {
        eprintln!("PM_TESTNET not set; skipping");
        return;
    }
    let pubky = pubky::Pubky::testnet().unwrap();
    let signer = pubky.signer(pubky::Keypair::random());
    let hs = pubky::PublicKey::try_from("8pinxxgqs41n4aididenw5apqp1urfmzdztr8jt4abrkdn435ewo").unwrap();
    signer.signup(&hs, None).await.unwrap();
    let session = signer
        .signin_blocking(pubky::ClientId::new("pm-api-feeds").unwrap())
        .await
        .unwrap();
    let z32 = session.public_key().to_string();
    let homeserver = pm_pubky::homeserver::NodeHomeserver::from_pubky(pubky.clone());
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_sync(Some(Arc::new(crate::sync::PubkySync::from_session(
        homeserver.clone(),
        session.clone(),
    ))));
    let app = router(s.clone());
    let read = |id: String| {
        let (hs, z32, session) = (homeserver.clone(), z32.clone(), session.clone());
        async move {
            hs.get_bytes(
                &z32,
                &format!("/priv/music/v1/feeds/{id}.json"),
                pm_pubky::Visibility::Private,
                Some(&session),
            )
            .await
            .ok()
            .map(|b| serde_json::from_slice::<pubky_music_specs::MusicFeed>(&b).unwrap())
        }
    };
    let (st, p) = call(
        &app,
        "POST",
        "/v1/smart",
        Some(serde_json::json!({"name": "Techno from friends", "reach": "friends", "tags": ["Techno"], "match": "any",
            "rules": [{"field": "year", "op": "between", "value": 1990, "to": 1999}], "limit": 40})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{p}");
    let fid = p["feed"]["id"].as_str().unwrap().to_string();
    assert_eq!(p["feed"]["public"], false);
    assert!(p["feed"]["uri"]
        .as_str()
        .unwrap()
        .ends_with(&format!("/priv/music/v1/feeds/{fid}.json")));
    let f = read(fid.clone()).await.expect("the feed is on the homeserver");
    f.validate(Some(&fid), true).unwrap();
    assert_eq!(f.name, "Techno from friends");
    assert_eq!(f.feed.tags.as_deref(), Some(&["techno".to_string()][..]));
    assert_eq!(serde_json::to_value(&f.feed.reach).unwrap(), "friends");
    assert_eq!(f.feed.rules[0].field, "year");
    assert_eq!(f.feed.limit.as_ref().unwrap().songs, Some(40));
    // An edit rewrites the same feed
    let id = p["id"].as_str().unwrap().to_string();
    let (st, p2) = call(
        &app,
        "PUT",
        &format!("/v1/smart/{id}"),
        Some(serde_json::json!({"name": "Techno from everyone", "reach": "all", "tags": ["techno"]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{p2}");
    assert_eq!(p2["feed"]["id"], fid.as_str());
    assert_eq!(read(fid.clone()).await.unwrap().name, "Techno from everyone");
    // Back to "just me": the feed goes
    let (st, p3) = call(
        &app,
        "PUT",
        &format!("/v1/smart/{id}"),
        Some(serde_json::json!({"name": "Mine", "rules": [{"field": "genre", "op": "contains", "value": "techno"}]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{p3}");
    assert!(p3.get("feed").is_none(), "{p3}");
    assert!(read(fid).await.is_none());
    // Deleting one with a feed removes the feed too
    let (_, p4) = call(
        &app,
        "PUT",
        &format!("/v1/smart/{id}"),
        Some(serde_json::json!({"name": "Again", "reach": "all", "tags": ["house"]})),
    )
    .await;
    let fid4 = p4["feed"]["id"].as_str().unwrap().to_string();
    assert!(read(fid4.clone()).await.is_some());
    let (st, _) = call(&app, "DELETE", &format!("/v1/smart/{id}"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(read(fid4).await.is_none());
}

/// About from open sources (F2–F5): the artist's "Also on" from MusicBrainz's links, its
/// Commons picture with author and licence, Wikidata facts with labels and the Wikipedia
/// summary in the reader's language; the album's label, release date and the English article
/// when there is none in that language; the song's work and its article. Cached per language.
#[tokio::test]
async fn library_pages_say_what_open_sources_know_in_the_readers_language() {
    const ARTIST: &str = "2d7f5a4d-6d2e-4f7b-9f6d-3a1c4d5e6f70";
    const GROUP: &str = "0b5d3e2b-4b0c-4d5f-9d4b-1e9a2b3c4d5e";
    const RELEASE: &str = "4f9b7c6f-8f4a-4b9d-9b8f-5c3e6f708192";
    const RECORDING: &str = "5a0c8d7a-9a5b-4c0e-8c9a-6d4f708192a3";
    const WORK: &str = "6b1d9e8b-0b6c-4d1f-9d0b-7e5a8192a3b4";
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let rel = |kind: &str, url: &str| serde_json::json!({"type": kind, "url": {"resource": url}});
    let claim = |v: serde_json::Value| serde_json::json!([{"rank": "normal", "mainsnak": {"datavalue": {"value": v}}}]);
    let item = |q: &str| claim(serde_json::json!({"id": q}));
    let mut answers: std::collections::HashMap<String, serde_json::Value> = std::collections::HashMap::new();
    answers.insert(
        format!("/ws/2/artist/{ARTIST}"),
        serde_json::json!({"relations": [
            rel("wikidata", "https://www.wikidata.org/wiki/Q1"),
            // A direct article with no summary (encoded in the link): the next one is used
            rel("wikipedia", "https://de.wikipedia.org/wiki/Sieben_%C3%84rzte"),
            rel("official homepage", "https://7dollartaxi.example/"),
            rel("free streaming", "https://open.spotify.com/artist/abc"),
            rel("streaming", "https://tidal.com/browse/artist/42"),
        ]}),
    );
    answers.insert("/wiki/Special:EntityData/Q1.json".into(), serde_json::json!({"entities": {"Q1": {
        "claims": {
            "P31": item("Q215380"), "P740": item("Q72"), "P136": item("Q11399"),
            "P527": [{"mainsnak": {"datavalue": {"value": {"id": "Q100"}}}}, {"mainsnak": {"datavalue": {"value": {"id": "Q101"}}}}],
            "P571": claim(serde_json::json!({"time": "+1994-00-00T00:00:00Z"})),
            "P18": claim(serde_json::json!("Taxi Band.jpg"))
        },
        "sitelinks": {"dewiki": {"title": "7 Dollar Taxi (Band)"}, "enwiki": {"title": "7 Dollar Taxi"}}
    }}}));
    answers.insert(
        format!("/ws/2/release-group/{GROUP}"),
        serde_json::json!({
            "first-release-date": "2001-05-04",
            "releases": [{"id": RELEASE, "status": "Official", "date": "2001-05-04"}],
            "relations": [rel("wikidata", "https://www.wikidata.org/wiki/Q2")]
        }),
    );
    answers.insert(
        format!("/ws/2/release/{RELEASE}"),
        serde_json::json!({
            "label-info": [{"label": {"name": "Sound Service"}}],
            "relations": [rel("purchase for download", "https://7dollartaxi.bandcamp.com/album/zahltag"),
                          rel("free streaming", "https://www.deezer.com/album/77")]
        }),
    );
    answers.insert(
        "/wiki/Special:EntityData/Q2.json".into(),
        serde_json::json!({"entities": {"Q2": {
        "claims": {}, "sitelinks": {"enwiki": {"title": "Zahltag (album)"}}}}}),
    );
    answers.insert(
        format!("/ws/2/recording/{RECORDING}"),
        serde_json::json!({"relations": [
        {"type": "performance", "target-type": "work", "work": {"id": WORK, "title": "Taxi"}}]}),
    );
    answers.insert(
        format!("/ws/2/work/{WORK}"),
        serde_json::json!({"relations": [
        rel("wikidata", "https://www.wikidata.org/wiki/Q3")]}),
    );
    answers.insert(
        "/wiki/Special:EntityData/Q3.json".into(),
        serde_json::json!({"entities": {"Q3": {
        "claims": {}, "sitelinks": {"dewiki": {"title": "Taxi (Lied)"}}}}}),
    );
    let answers = Arc::new(answers);
    let asked_titles: Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let b2 = base.clone();
    let c2 = calls.clone();
    let mock = Router::new()
        .route("/w/api.php", get(move |Query(q): Query<std::collections::HashMap<String, String>>| {
            let b = b2.clone();
            c2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move {
                if q.get("action").map(String::as_str) == Some("wbgetentities") {
                    assert_eq!(q["languages"], "de|en");
                    let de = |n: &str| serde_json::json!({"labels": {"de": {"value": n}}});
                    return Json(serde_json::json!({"entities": {
                        "Q72": de("Zürich"), "Q11399": de("Rockmusik"), "Q100": de("Anna"),
                        "Q101": {"labels": {"en": {"value": "Ben"}}}}}));
                }
                assert_eq!(q["titles"], "File:Taxi_Band.jpg");
                Json(serde_json::json!({"query": {"pages": {"7": {"imageinfo": [{
                    "thumburl": format!("{b}/img/taxi.jpg"),
                    "descriptionurl": "https://commons.wikimedia.org/wiki/File:Taxi_Band.jpg",
                    "extmetadata": {
                        "Artist": {"value": "<a href=\"//commons.wikimedia.org/wiki/User:Jane\">Jane Doe</a>"},
                        "LicenseShortName": {"value": "CC BY-SA 4.0"},
                        "LicenseUrl": {"value": "https://creativecommons.org/licenses/by-sa/4.0"}
                    }}]}}}}))
            }
        }))
        .route("/api/rest_v1/page/summary/{title}", get({
            let asked = asked_titles.clone();
            move |Path(title): Path<String>| {
                asked.lock().unwrap().push(title.clone());
                async move {
                    let text = match title.as_str() {
                        "7_Dollar_Taxi_(Band)" => "7 Dollar Taxi ist eine Band aus Zürich.",
                        "Zahltag_(album)" => "Zahltag is the first album by 7 Dollar Taxi.",
                        "Taxi_(Lied)" => "Taxi ist ein Lied von 7 Dollar Taxi.",
                        _ => return StatusCode::NOT_FOUND.into_response(),
                    };
                    Json(serde_json::json!({"title": title.replace('_', " "), "extract": text,
                        "content_urls": {"desktop": {"page": format!("https://de.wikipedia.org/wiki/{title}")}}}))
                    .into_response()
                }
            }
        }))
        .route("/img/taxi.jpg", get(|| async { ([(header::CONTENT_TYPE, "image/jpeg")], vec![0xffu8, 0xd8, 0xff, 1, 2, 3]) }))
        .fallback({
            let c = calls.clone();
            move |uri: axum::http::Uri| {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let a = answers.clone();
                async move {
                    match a.get(uri.path()) {
                        Some(v) => Json(v.clone()).into_response(),
                        None => StatusCode::NOT_FOUND.into_response(),
                    }
                }
            }
        });
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    {
        let mut b = s.info_bases.write().unwrap();
        b.musicbrainz = base.clone();
        b.wikidata = base.clone();
        b.wikipedia = base.clone();
        b.commons = base.clone();
        b.fanart_key = None;
        b.audiodb = "http://127.0.0.1:9".into();
        b.wikimedia_interval = std::time::Duration::ZERO;
    }
    let (artist, album, track) = {
        let lib = s.library.lock().unwrap();
        let mut t = untagged_track(dir.path().join("taxi.flac"), "Taxi", "7 Dollar Taxi");
        t.artists[0].mbid = Some(pm_model::Mbid::parse(ARTIST).unwrap());
        t.recording_mbid = Some(pm_model::Mbid::parse(RECORDING).unwrap());
        t.album = Some(pm_model::Album {
            id: None,
            title: "Zahltag".into(),
            album_artist: Some("7 Dollar Taxi".into()),
            release_mbid: Some(pm_model::Mbid::parse(RELEASE).unwrap()),
            release_group_mbid: Some(pm_model::Mbid::parse(GROUP).unwrap()),
            year: Some(2001),
            upc: None,
            total_discs: None,
        });
        let tid = lib.upsert_track(&t).unwrap();
        let t = lib.get_track(tid).unwrap().unwrap();
        (t.artists[0].id.unwrap(), t.album.unwrap().id.unwrap(), tid.0)
    };
    let app = router(s.clone());
    let fact = |p: &serde_json::Value, label: &str| {
        p["about"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["label"] == label)
            .map(|f| f["value"].clone())
    };
    // A page never waits for open sources: it says they are coming, and has them when asked again
    let settled = |path: String, lang: Option<&'static str>| {
        let app = app.clone();
        async move {
            let t = std::time::Instant::now();
            let mut first = None;
            for _ in 0..100 {
                let mut req = Request::builder().uri(path.as_str());
                if let Some(l) = lang {
                    req = req.header(header::ACCEPT_LANGUAGE, l);
                }
                let resp = app
                    .clone()
                    .oneshot(req.body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(resp.status(), StatusCode::OK);
                let p: serde_json::Value =
                    serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
                first.get_or_insert((p["about_pending"] == true, t.elapsed()));
                if p["about_pending"] != true {
                    return (p, first.unwrap());
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            panic!("{path} never settled");
        }
    };
    let (p, (pending, took)) = settled(format!("/v1/item/artist/lib-{artist}?lang=de-CH"), None).await;
    assert!(pending, "the first answer says About is coming");
    assert!(
        took < std::time::Duration::from_secs(1),
        "and doesn't wait for it: {took:?}"
    );
    // The direct link was asked once, decoded (not "%C3%84" twice over), then the sitelink
    let asked = asked_titles.lock().unwrap().clone();
    assert_eq!(
        asked[..2],
        ["Sieben_Ärzte".to_string(), "7_Dollar_Taxi_(Band)".to_string()],
        "{asked:?}"
    );
    assert_eq!(
        p["about_text"]["text"], "7 Dollar Taxi ist eine Band aus Zürich.",
        "{p}"
    );
    assert_eq!(p["about_text"]["lang"], "de");
    assert_eq!(p["about_text"]["licence"], "CC BY-SA 4.0");
    assert_eq!(p["image"], format!("/v1/library/artists/{artist}/image"));
    assert_eq!(p["image_credit"]["author"], "Jane Doe", "{p}");
    assert_eq!(p["image_credit"]["licence"], "CC BY-SA 4.0");
    assert_eq!(
        p["image_credit"]["file_page"],
        "https://commons.wikimedia.org/wiki/File:Taxi_Band.jpg"
    );
    assert_eq!(fact(&p, "Origin"), Some("Zürich".into()), "{p}");
    assert_eq!(fact(&p, "Active"), Some("since 1994".into()));
    assert_eq!(fact(&p, "Genres"), Some("Rockmusik".into()));
    assert_eq!(
        fact(&p, "Members"),
        Some("Anna, Ben".into()),
        "English where German has no label"
    );
    // In the play-from order, as every page's sources
    let sources = |p: &serde_json::Value| {
        let mut v: Vec<String> = p["also_on"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["source"].as_str().unwrap().to_string())
            .collect();
        v.sort();
        v
    };
    assert_eq!(sources(&p), ["homepage", "spotify", "tidal"], "{p}");
    let tidal = p["also_on"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["source"] == "tidal")
        .unwrap();
    assert_eq!(tidal["id"], "artist:42");
    // The album: no German article, so the English one; label and date from the release
    let (p, _) = settled(format!("/v1/item/album/lib-{album}"), Some("de-DE,de;q=0.9")).await;
    assert_eq!(p["about_text"]["lang"], "en", "{p}");
    assert_eq!(p["blurb"], "Zahltag is the first album by 7 Dollar Taxi.");
    assert_eq!(fact(&p, "Released"), Some("2001-05-04".into()), "{p}");
    assert_eq!(fact(&p, "Label"), Some("Sound Service".into()));
    assert_eq!(sources(&p), ["bandcamp", "deezer"], "{p}");
    // The song: its work's article
    let (p, _) = settled(format!("/v1/item/song/lib-{track}?lang=de"), None).await;
    assert_eq!(
        p["about_text"]["text"], "Taxi ist ein Lied von 7 Dollar Taxi.",
        "{p}"
    );
    assert_eq!(fact(&p, "Work"), Some("Taxi".into()));
    // Cached per language: the same pages again ask nothing
    let asked = calls.load(std::sync::atomic::Ordering::SeqCst);
    let (_, p) = call(
        &app,
        "GET",
        &format!("/v1/item/artist/lib-{artist}?lang=de"),
        None,
    )
    .await;
    assert!(p["about_pending"].is_null(), "cached: {p}");
    call(&app, "GET", &format!("/v1/item/song/lib-{track}?lang=de"), None).await;
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), asked);
}

/// "In collections" (TASK-in-collections-client): the public collections Pinakes says hold
/// the page's link, as a count and the first few, each pointing at its shared-collection
/// page; absent while Pinakes doesn't list them. Nothing about bookmarks.
#[tokio::test]
async fn item_pages_name_the_public_collections_holding_them() {
    let fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-pubky/tests/fixtures/pinakes/music/detail-tidal-track-bohemian.json"
    ))
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let ships = Arc::new(std::sync::atomic::AtomicBool::new(false));
    const A: &str = "gujx6qd8ksydh1makdphd3bxu351d9b8waqka8hfg6q7hnqkxexo";
    const B: &str = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";
    let mock = Router::new()
        .route(
            "/v1/info",
            get({
                let ships = ships.clone();
                move || {
                    let ships = ships.clone();
                    async move {
                        let limits = if ships.load(std::sync::atomic::Ordering::SeqCst) {
                            serde_json::json!({"search": 50, "collections": 20})
                        } else {
                            serde_json::json!({"search": 50})
                        };
                        Json(serde_json::json!({"version": "test", "limits": limits}))
                    }
                }
            }),
        )
        .route(
            "/v1/resources/by-uri",
            get(move || {
                let fixture = fixture.clone();
                async move { ([(header::CONTENT_TYPE, "application/json")], fixture) }
            }),
        )
        .route(
            "/v1/resources/{id}/collections",
            get(|Path(id): Path<String>, axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
                assert!(!id.is_empty());
                assert_eq!(q.get("limit").map(String::as_str), Some("3"), "{q:?}");
                Json(serde_json::json!({"items": [
                    {"uri": format!("pubky://{A}/pub/social/v1/posts/0035WKVKQDHSM"), "author": A, "indexed_at": 1700000000000i64},
                    {"uri": format!("pubky://{B}/pub/social/v1/posts/00351MT54YS9G"), "author": B, "indexed_at": 1690000000000i64},
                    {"uri": "not a post", "author": ""},
                ], "total": 5}))
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let pubky = pm_pubky::config::PmPubkyConfig {
        pinakes_base: format!("http://127.0.0.1:{port}"),
        nexus_base: format!("http://127.0.0.1:{port}/nexus"),
        user_agent_contact: "test@example.org".into(),
        ..Default::default()
    };
    let s = state_with(dir.path(), None, pubky).await;
    s.set_contact("test@example.org").unwrap();
    let app = router(s.clone());
    let id = discover::page_id("https://tidal.com/track/534050211");
    // Pinakes without the route: no row at all (the page never says "0 collections")
    let (st, v) = call(&app, "GET", &format!("/v1/item/song/{id}/extras"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(v.get("collections").is_none(), "{v}");
    // The feature probe is cached: a fresh node sees the route
    ships.store(true, std::sync::atomic::Ordering::SeqCst);
    let dir2 = tempfile::tempdir().unwrap();
    let pubky2 = pm_pubky::config::PmPubkyConfig {
        pinakes_base: format!("http://127.0.0.1:{port}/"),
        nexus_base: format!("http://127.0.0.1:{port}/nexus"),
        user_agent_contact: "test@example.org".into(),
        ..Default::default()
    };
    let s2 = state_with(dir2.path(), None, pubky2).await;
    s2.set_contact("test@example.org").unwrap();
    let app2 = router(s2.clone());
    let (st, v) = call(&app2, "GET", &format!("/v1/item/song/{id}/extras"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let c = &v["collections"];
    assert_eq!(c["count"], 4, "five in all, one line that isn't a post: {v}");
    let items = c["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "a line that is not a post is left out: {v}");
    assert_eq!(items[0]["author"], A);
    assert_eq!(items[0]["id"], "0035WKVKQDHSM");
    assert_eq!(items[0]["mine"], false, "no identity here");
    assert!(
        items[0].get("name").is_none(),
        "no homeserver to read the name from: {v}"
    );
    assert!(
        !v.to_string().contains("bookmark"),
        "nothing about bookmarks: {v}"
    );
    let (st, pg) = call(&app2, "GET", &format!("/v1/item/song/{id}"), None).await;
    assert_eq!(st, StatusCode::OK, "{pg}");
    assert!(
        !pg.to_string().contains("bookmark"),
        "the page itself carries no bookmark count either: {pg}"
    );
}

#[tokio::test]
async fn item_pages_list_similar_music_from_the_library() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let add = |name: &str, artist: &str, album: &str, genres: &[&str]| {
        let mut t = untagged_track(dir.path().join(format!("{name}.wav")), name, artist);
        t.album = Some(pm_model::Album {
            id: None,
            title: album.into(),
            album_artist: Some(artist.into()),
            release_mbid: None,
            release_group_mbid: None,
            year: None,
            upc: None,
            total_discs: None,
        });
        t.genres = genres.iter().map(|g| g.to_string()).collect();
        s.library.lock().unwrap().upsert_track(&t).unwrap().0
    };
    let seed = add("Seed", "Alpha", "First", &["Disco", "Funk"]);
    add("Both", "Beta", "Second", &["Funk", "Disco"]);
    add("One", "Gamma", "Third", &["Disco"]);
    add("Same Artist", "Alpha", "Other Alpha", &["Jazz"]);
    add("Unrelated", "Delta", "Fourth", &["Jazz"]);
    let app = router(s.clone());
    let titles = |v: &serde_json::Value| -> Vec<String> {
        v["similar"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["title"].as_str().unwrap().to_string())
            .collect()
    };
    // A song: shared genres first (most first), then the same artist's; never itself
    let (st, v) = call(&app, "GET", &format!("/v1/item/song/lib-{seed}/extras"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(titles(&v), vec!["Both", "One", "Same Artist"], "{v}");
    assert_eq!(v["similar"][0]["kind"], "song");
    assert!(v["similar"][0]["track_id"].is_i64(), "playable: {v}");
    // An album: other albums, then the same artist's other albums
    let album = s
        .library
        .lock()
        .unwrap()
        .get_track(pm_model::TrackId(seed))
        .unwrap()
        .unwrap()
        .album
        .unwrap()
        .id
        .unwrap();
    let (_, v) = call(&app, "GET", &format!("/v1/item/album/lib-{album}/extras"), None).await;
    assert_eq!(titles(&v), vec!["Second", "Third", "Other Alpha"], "{v}");
    // An artist: the artists of the songs that share genres, never the artist itself
    let artist = s
        .library
        .lock()
        .unwrap()
        .get_track(pm_model::TrackId(seed))
        .unwrap()
        .unwrap()
        .artists[0]
        .id
        .unwrap();
    let (_, v) = call(&app, "GET", &format!("/v1/item/artist/lib-{artist}/extras"), None).await;
    // (Alpha has a Jazz song too, so Delta shares a genre; equals come in any order)
    let got = titles(&v);
    assert_eq!(got[0], "Beta", "the most shared first: {v}");
    let mut rest = got[1..].to_vec();
    rest.sort();
    assert_eq!(rest, vec!["Delta", "Gamma"], "{v}");
}

/// Search groups (TASK-pinakes-social-ux A1/A2): own playlists, people and posts from Nexus,
/// `tag:` as `#`, the `tags` parameter, and a library file's format and quality on its row.
#[tokio::test]
async fn search_lists_playlists_people_posts_and_formats() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let key = "k".repeat(52);
    let who = key.clone();
    let mock = Router::new()
        .route("/v1/search", get(|| async { Json(serde_json::json!({"items": []})) }))
        .route(
            "/nexus/v0/stream/users/username",
            get(move |Query(q): Query<std::collections::HashMap<String, String>>| {
                let who = who.clone();
                async move {
                    let items = if q.get("username").map(String::as_str) == Some("sat") {
                        serde_json::json!([{"details": {"id": who, "name": "Sat", "bio": "plays bass"}}])
                    } else {
                        serde_json::json!([])
                    };
                    Json(items)
                }
            }),
        )
        .route(
            "/nexus/v0/search/posts/by_content",
            get(|| async { Json(serde_json::json!([{"post_key": "author1:0033", "score": 1.0}])) }),
        )
        .route(
            "/nexus/v0/search/posts/by_tag/{t}",
            // every tag carries post 0033; `rare` carries only another post
            get(|Path(t): Path<String>| async move {
                let key = if t == "rare" { "author2:0044" } else { "author1:0033" };
                Json(serde_json::json!([{"post_key": key, "score": 1}]))
            }),
        )
        .route(
            "/nexus/v0/stream/posts/by_ids",
            post(|| async {
                Json(serde_json::json!([{"details": {"id": "0033", "author": "author1", "content": "Great music tonight\nand more", "uri": "pubky://author1/pub/pubky.app/posts/0033", "kind": "short"}}]))
            }),
        )
        .route(
            "/nexus/v0/stream/users/by_ids",
            post(|| async { Json(serde_json::json!([{"details": {"id": "author1", "name": "Author One"}}])) }),
        )
        .route(
            "/nexus/v0/search/tags/by_prefix/{p}",
            get(|| async { Json(serde_json::json!([])) }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let pubky = pm_pubky::config::PmPubkyConfig {
        pinakes_base: format!("http://127.0.0.1:{port}"),
        nexus_base: format!("http://127.0.0.1:{port}/nexus"),
        user_agent_contact: "test@example.org".into(),
        ..Default::default()
    };
    let s = state_with(dir.path(), None, pubky).await;
    s.set_contact("test@example.org").unwrap();
    let tid = {
        let lib = s.library.lock().unwrap();
        let tr = untagged_track(dir.path().join("s.flac"), "Saturday Song", "Someone");
        lib.upsert_track(&tr).unwrap().0
    };
    let app = router(s.clone());
    let (st, _) = call(
        &app,
        "POST",
        "/v1/playlists",
        Some(serde_json::json!({"name": "Saturday chill", "track_ids": [tid]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let lines = |app: Router, uri: String| async move {
        let resp = app
            .oneshot(
                axum::http::Request::get(uri)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let sources = resp
            .headers()
            .get("x-pm-sources")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let answers = String::from_utf8_lossy(&body)
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| v.get("source").is_some())
            .collect::<Vec<_>>();
        (sources, answers)
    };
    let (sources, ans) = lines(app.clone(), "/v1/search?q=sat".into()).await;
    assert!(
        sources.contains("people") && sources.contains("posts"),
        "{sources}"
    );
    let lib = ans
        .iter()
        .find(|a| a["source"] == "library")
        .expect("library answered");
    let hits = lib["hits"].as_array().unwrap();
    let pl = hits
        .iter()
        .find(|h| h["kind"] == "playlist")
        .expect("the playlist is a result");
    assert_eq!(pl["title"], "Saturday chill", "{pl}");
    let song = hits
        .iter()
        .find(|h| h["kind"] == "song")
        .expect("the song is a result");
    assert!(song["format"].as_str().is_some_and(|f| !f.is_empty()), "{song}");
    assert!(
        ["hi_res", "lossless", "lossy"].contains(&song["quality"].as_str().unwrap_or("")),
        "{song}"
    );
    let people = ans
        .iter()
        .find(|a| a["source"] == "people")
        .expect("people answered");
    assert_eq!(people["status"], "ok", "{people}");
    assert_eq!(people["hits"][0]["kind"], "person");
    assert_eq!(people["hits"][0]["title"], "Sat");
    assert_eq!(people["hits"][0]["id"], key);
    let posts = ans
        .iter()
        .find(|a| a["source"] == "posts")
        .expect("posts answered");
    assert_eq!(posts["status"], "ok", "{posts}");
    assert_eq!(posts["hits"][0]["kind"], "post");
    assert_eq!(posts["hits"][0]["title"], "Great music tonight");
    assert_eq!(posts["hits"][0]["subtitle"], "Author One");
    assert_eq!(posts["hits"][0]["page"], "https://pubky.app/post/author1/0033");
    // `tag:` is `#`: tags only, posts carrying the tag included (a tag no other test searches:
    // answers are cached per process)
    let (sources, _) = lines(app.clone(), "/v1/search?q=tag%3Alofi".into()).await;
    assert_eq!(sources, "library,pubky,posts");
    // several tags: posts carrying all of them
    let (_, ans) = lines(app.clone(), "/v1/search?tags=lofi,deep".into()).await;
    let posts = ans
        .iter()
        .find(|a| a["source"] == "posts")
        .expect("posts answered");
    assert_eq!(
        posts["hits"].as_array().map(Vec::len),
        Some(1),
        "both tags on 0033: {posts}"
    );
    let (_, ans) = lines(app.clone(), "/v1/search?tags=lofi,rare".into()).await;
    let posts = ans
        .iter()
        .find(|a| a["source"] == "posts")
        .expect("posts answered");
    assert_eq!(
        posts["hits"].as_array().map(Vec::len),
        Some(0),
        "no post has both: {posts}"
    );
    let (sources2, ans) = lines(app.clone(), "/v1/search?tags=lofi".into()).await;
    assert_eq!(sources2, sources);
    // the tag reaches the posts search (Nexus's by_tag answers the post)
    let posts = ans
        .iter()
        .find(|a| a["source"] == "posts")
        .expect("posts answered");
    assert_eq!(posts["status"], "ok", "{posts}");
    assert_eq!(posts["hits"][0]["title"], "Great music tonight");
    // One letter is no search for Nexus: people and posts answer with nothing, not an error
    let (_, ans) = lines(app.clone(), "/v1/search?q=a".into()).await;
    for src in ["people", "posts"] {
        let a = ans.iter().find(|a| a["source"] == src).expect("answered");
        assert_eq!(a["status"], "ok", "{a}");
        assert_eq!(a["hits"].as_array().map(Vec::len), Some(0), "{a}");
    }
    assert_eq!(search::hash_tags("rock tag:chill #90s"), vec!["chill", "90s"]);
    assert_eq!(search::text_words("rock tag:chill #90s"), "rock");
    let (sources, _) = lines(app.clone(), "/v1/search?q=sat&in=library".into()).await;
    assert_eq!(sources, "library");
}

/// "Share what I'm listening to" is removed (owner, 2026-10-31: Post about this covers it):
/// its API is gone, and a setting saved before is ignored.
#[tokio::test]
async fn sharing_what_you_listen_to_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.library
        .lock()
        .unwrap()
        .set_setting("listening.share", r#"{"share":true}"#)
        .unwrap();
    let app = router(s.clone());
    let (st, _) = call(&app, "GET", "/v1/me/listening", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let on = serde_json::json!({"share": true});
    let (st, _) = call(&app, "PUT", "/v1/me/listening", Some(on)).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

/// Owner report (iPad, 2026-11-04): an album showed every song twice, the NAS file and an old
/// homeserver copy whose file is gone. The album page and its tracks show each song once,
/// play-all gets one per song, and a song kept in two present copies lists them as `versions`.
#[tokio::test]
async fn album_shows_one_row_per_song_with_versions() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (aid, nas, kept, home) = {
        let lib = s.library.lock().unwrap();
        let album = pm_model::Album {
            id: None,
            title: "Crossroad: Unplugged Live".into(),
            album_artist: Some("Calvin Russell".into()),
            release_mbid: None,
            release_group_mbid: None,
            year: Some(1997),
            upc: None,
            total_discs: None,
        };
        let mut ids = Vec::new();
        for (no, path) in [
            (1, "music/Roon/01.flac"),
            (
                1,
                "homeserver/data/files/u/priv/pubky.music/files/aaaaaaaaaaaaaaaa-01.flac",
            ),
            (2, "music/Roon/02.flac"),
            (
                2,
                "homeserver/data/files/u/priv/pubky.music/files/bbbbbbbbbbbbbbbb-02.flac",
            ),
        ] {
            let mut tr = untagged_track(dir.path().join(path), &format!("Song {no}"), "Calvin Russell");
            tr.album = Some(album.clone());
            tr.track_no = Some(no);
            tr.duration_ms = Some(60_000);
            ids.push(lib.upsert_track(&tr).unwrap().0);
        }
        // The copy of song 1 was deleted; the copy of song 2 is still there
        let gone = dir
            .path()
            .join("homeserver/data/files/u/priv/pubky.music/files/aaaaaaaaaaaaaaaa-01.flac");
        let root = dir.path().join("homeserver/data/files/u/priv/pubky.music/files");
        let scan = lib.begin_scan(&root).unwrap();
        lib.mark_file_seen(&root.join("bbbbbbbbbbbbbbbb-02.flac"), scan)
            .unwrap();
        assert_eq!(lib.mark_missing_under(&root, scan).unwrap(), 1, "{gone:?}");
        let aid = lib
            .get_track(pm_model::TrackId(ids[0]))
            .unwrap()
            .unwrap()
            .album
            .unwrap()
            .id
            .unwrap();
        (aid, ids[0], ids[2], ids[3])
    };
    let (st, al) = call(&app, "GET", &format!("/v1/item/album/lib-{aid}"), None).await;
    assert_eq!(st, StatusCode::OK, "{al}");
    let songs = al["songs"].as_array().unwrap();
    assert_eq!(songs.len(), 2, "{al}");
    assert_eq!(songs[0]["track_id"], nas);
    assert!(
        songs[0].get("versions").is_none(),
        "one present copy: no versions {al}"
    );
    assert_eq!(
        songs[1]["track_id"], kept,
        "the NAS file over the homeserver copy"
    );
    let v = songs[1]["versions"].as_array().unwrap();
    assert_eq!(v.len(), 2, "{al}");
    assert_eq!(
        (v[0]["track_id"].as_i64(), v[0]["best"].as_bool()),
        (Some(kept), Some(true))
    );
    assert_eq!(v[1]["track_id"], home);
    assert_eq!(v[1]["source"], "homeserver");
    assert!(v[1].get("path").is_none(), "no file paths in versions");
    let (st, tr) = call(&app, "GET", &format!("/v1/library/albums/{aid}/tracks"), None).await;
    assert_eq!(st, StatusCode::OK, "{tr}");
    assert_eq!(
        tr["tracks"].as_array().unwrap().len(),
        2,
        "play-all queues one per song"
    );
    assert_eq!(
        tr["versions"][kept.to_string()].as_array().unwrap().len(),
        2,
        "{tr}"
    );
}

/// PR #268 review (Claude H1): a node-only favourite of a song whose file is gone for a while
/// (a NAS share not mounted at a scan) is kept, and is there again when the file is back.
#[tokio::test]
async fn a_favourite_survives_its_file_going_missing() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let nas = dir.path().join("nas");
    let id = {
        let lib = s.library.lock().unwrap();
        lib.upsert_track(&untagged_track(nas.join("01.flac"), "Song", "Someone"))
            .unwrap()
            .0
    };
    let body = serde_json::json!({ "track_id": id });
    let (st, b) = call(&app, "PUT", "/v1/bookmarks", Some(body)).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(bookmarks::local_ids(&s.library.lock().unwrap()), vec![id]);
    {
        let lib = s.library.lock().unwrap();
        let scan = lib.begin_scan(&nas).unwrap();
        lib.mark_missing_under(&nas, scan).unwrap();
    }
    // Reading the favourites tidies node-only ones: the gone song must stay
    let (st, _) = call(&app, "GET", "/v1/bookmarks", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(bookmarks::local_ids(&s.library.lock().unwrap()), vec![id]);
    {
        let lib = s.library.lock().unwrap();
        let scan = lib.begin_scan(&nas).unwrap();
        lib.mark_file_seen(&nas.join("01.flac"), scan).unwrap();
    }
    let (_, _) = call(&app, "GET", "/v1/bookmarks", None).await;
    assert_eq!(bookmarks::local_ids(&s.library.lock().unwrap()), vec![id]);
}

/// A library album with no MusicBrainz match keeps its Year in About (the release group's
/// "Released" stands in for it only when there is one), and the facts say their kind: the
/// year is a fact, a file's format a reference.
#[tokio::test]
async fn album_without_a_match_keeps_its_year_as_a_fact() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let aid = {
        let lib = s.library.lock().unwrap();
        let album = pm_model::Album {
            id: None,
            title: "Sounds from the Fourth World".into(),
            album_artist: Some("Calvin Russell".into()),
            release_mbid: None,
            release_group_mbid: None,
            year: Some(1991),
            upc: None,
            total_discs: None,
        };
        let mut tr = untagged_track(dir.path().join("music/fw/01.flac"), "Song 1", "Calvin Russell");
        tr.album = Some(album);
        tr.track_no = Some(1);
        let id = lib.upsert_track(&tr).unwrap().0;
        lib.get_track(pm_model::TrackId(id))
            .unwrap()
            .unwrap()
            .album
            .unwrap()
            .id
            .unwrap()
    };
    let (st, al) = call(&app, "GET", &format!("/v1/item/album/lib-{aid}"), None).await;
    assert_eq!(st, StatusCode::OK, "{al}");
    let about = al["about"].as_array().unwrap();
    let year = about.iter().find(|f| f["label"] == "Year");
    assert_eq!(
        year.map(|f| (f["value"].as_str().unwrap(), f["kind"].as_str().unwrap())),
        Some(("1991", "fact")),
        "{al}"
    );
    assert!(
        about.iter().all(|f| f["label"] != "Released"),
        "no release group, no Released: {al}"
    );
    let format = about.iter().find(|f| f["label"] == "Format").unwrap();
    assert_eq!(format["kind"], "ref", "a file's format is a reference: {al}");
}

/// A song put back into the library is found by search at once: restore drops search's kept
/// answer itself. The scan slot is held meanwhile, so restore only queues its folder's scan
/// and no scan's end can drop that answer instead.
#[tokio::test]
async fn a_song_put_back_is_found_by_search_at_once_without_a_scan() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("music")).unwrap();
    write_wav(&dir.path().join("music/one.wav"), 4410);
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (st, _) = call(&app, "POST", "/v1/library/scan", Some(serde_json::json!({}))).await;
    assert_eq!(st, StatusCode::OK);
    for _ in 0..500 {
        if !s.is_scanning() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(!s.is_scanning());
    let found = |app: Router| async move {
        let (st, a) = call(&app, "GET", "/v1/search?q=one&in=library", None).await;
        assert_eq!(st, StatusCode::OK, "{a}");
        let titles: Vec<String> = a["hits"]
            .as_array()
            .unwrap_or_else(|| panic!("no hits in {a}"))
            .iter()
            .filter(|h| h["kind"] == "song")
            .map(|h| h["title"].as_str().unwrap_or_default().to_string())
            .collect();
        (a["cached"].as_bool().unwrap_or(false), titles)
    };
    let one = vec!["One".to_string()];
    assert_eq!(found(app.clone()).await, (false, one.clone()));
    let (_, t) = call(&app, "GET", "/v1/library/tracks?limit=10", None).await;
    let id = t["tracks"][0]["id"]
        .as_i64()
        .unwrap_or_else(|| panic!("no song in {t}"));
    let (st, _) = call(&app, "DELETE", &format!("/v1/library/tracks/{id}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    // Taken out, and asked again: the kept answer without it
    assert_eq!(found(app.clone()).await, (false, Vec::new()));
    assert_eq!(found(app.clone()).await, (true, Vec::new()));
    let (_, h) = call(&app, "GET", "/v1/library/hidden", None).await;
    let path = h[0]["path"]
        .as_str()
        .unwrap_or_else(|| panic!("nothing hidden: {h}"))
        .to_string();
    // No scan can run or end while the slot is held
    let slot = s.scan_slot().expect("no scan running");
    let (st, r) = call(
        &app,
        "POST",
        "/v1/library/hidden/restore",
        Some(serde_json::json!({ "path": path })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(
        (r["scan_started"].as_bool(), r["scan_queued"].as_bool()),
        (Some(false), Some(true)),
        "{r}"
    );
    // Put back: a fresh library answer that finds it
    assert_eq!(found(app.clone()).await, (false, one));
    drop(slot);
}
