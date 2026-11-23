//! The other services and sources, each against a mock.
//!
//! Part of pm-api's unit tests (`mod tests` in lib.rs, which holds the helpers these use). A new
//! test of this area goes at the end of this file.

use super::*;

/// Serve the FLAC fixture over HTTP (directly and behind a PLS playlist)
/// and play both through a zone: the HTTP stream source end to end.
#[tokio::test]
async fn http_streams_play_like_files() {
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let pls =
        format!("[playlist]\nNumberOfEntries=1\nFile1=http://127.0.0.1:{port}/a.flac\nTitle1=Fixture\n");
    let files = Router::new()
        .route(
            "/a.flac",
            get(move || {
                let flac = flac.clone();
                async move { ([(header::CONTENT_TYPE, "audio/flac")], flac) }
            }),
        )
        .route(
            "/list.pls",
            get(move || {
                let pls = pls.clone();
                async move { ([(header::CONTENT_TYPE, "audio/x-scpls")], pls) }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, files).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    for uri in [
        format!("http://127.0.0.1:{port}/a.flac"),
        format!("http://127.0.0.1:{port}/list.pls"),
    ] {
        let (st, q) = call(
            &app,
            "POST",
            &format!("/v1/zones/{zid}/queue"),
            Some(serde_json::json!({"items": [{"uri": uri, "title": "Fixture radio"}], "replace": true, "play": true})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{q}");
        let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
        let st = zone.wait_stopped().await;
        assert_eq!(st.error, None, "{uri}");
    }
    // Saved stations round trip; playing one goes through the same path.
    let (st, saved) = call(
        &app,
        "PUT",
        "/v1/radio/stations",
        Some(serde_json::json!({"id": "", "name": "Fixture FM", "url": format!("http://127.0.0.1:{port}/a.flac"), "tags": ["test"]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{saved}");
    let id = saved["stations"][0]["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("url-"));
    let (st, played) = call(
        &app,
        "POST",
        "/v1/radio/play",
        Some(serde_json::json!({"zone": zid, "station": saved["stations"][0]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{played}");
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    assert_eq!(zone.wait_stopped().await.error, None);
    let (_, q) = call(&app, "GET", &format!("/v1/zones/{zid}/queue"), None).await;
    assert_eq!(q[0]["title"], "Fixture FM");
    assert_eq!(q[0]["album"], "Internet radio");
    let (st, _) = call(&app, "DELETE", &format!("/v1/radio/stations/{id}"), None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call(&app, "DELETE", &format!("/v1/radio/stations/{id}"), None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (_, left) = call(&app, "GET", "/v1/radio/stations", None).await;
    assert!(left["stations"].as_array().unwrap().is_empty());
    let (st, _) = call(&app, "GET", "/v1/radio/search", None).await;
    assert_eq!(
        st,
        StatusCode::BAD_REQUEST,
        "search without terms is refused before any network call"
    );
}

/// YouTube is a Sources tab (on/off) that plays in the browser only: a zone refuses its items
/// with the reason, and it does not search yet.
#[tokio::test]
async fn youtube_items_play_only_in_the_browser() {
    use pm_zones::SourceFactory;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (_, sources) = call(&app, "GET", "/v1/sources", None).await;
    let yt = sources
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "youtube")
        .unwrap()
        .clone();
    assert_eq!(yt["status"], "browser");
    let (st, _) = call(&app, "GET", "/v1/sources/youtube/search?q=x", None).await;
    assert_eq!(
        st,
        StatusCode::PRECONDITION_FAILED,
        "search needs the owner's key"
    );
    assert!(!crate::search::sources_for(&s, "everywhere").contains(&"youtube".to_string()));
    let err = s
        .sources
        .open(&pm_zones::QueueItem::new("youtube:video:dQw4w9WgXcQ"))
        .err()
        .expect("a zone refuses a YouTube item");
    assert!(err.to_string().contains("only in This browser"), "{err}");
}

/// The Internet Archive source against a local mock: search, item
/// tracks with the best format, and the tracks play through a zone.
#[tokio::test]
async fn internet_archive_source_searches_and_plays() {
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = Router::new()
        .route(
            "/advancedsearch.php",
            get(|| async {
                Json(serde_json::json!({"response": {"docs": [
                    {"identifier": "fixture-set", "title": "Fixture Set", "creator": "Tester", "year": "2020", "downloads": 7}
                ]}}))
            }),
        )
        .route(
            "/metadata/{id}",
            get(|| async {
                Json(serde_json::json!({
                    "metadata": {"title": "Fixture Set", "creator": "Tester", "year": "2020"},
                    "files": [
                        {"name": "one.flac", "format": "Flac", "source": "original", "title": "One", "track": "1", "length": "5.15"},
                        {"name": "one.mp3", "format": "VBR MP3", "source": "derivative", "original": "one.flac", "title": "One", "track": "1"}
                    ]
                }))
            }),
        )
        .route(
            "/download/{id}/{file}",
            get(move || {
                let flac = flac.clone();
                async move { ([(header::CONTENT_TYPE, "audio/flac")], flac) }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    *s.archive_base.write().unwrap() = format!("http://127.0.0.1:{port}");
    let app = router(s.clone());
    let (st, sources) = call(&app, "GET", "/v1/sources", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(sources[0]["id"], "archive");
    let (st, _) = call(&app, "GET", "/v1/sources/archive/search?q=", None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, r) = call(&app, "GET", "/v1/sources/archive/search?q=fixture", None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["items"][0]["id"], "fixture-set");
    let (st, it) = call(&app, "GET", "/v1/sources/archive/items/fixture-set", None).await;
    assert_eq!(st, StatusCode::OK, "{it}");
    assert_eq!(
        it["tracks"].as_array().unwrap().len(),
        1,
        "one file per recording"
    );
    assert_eq!(it["tracks"][0]["format"], "Flac");
    let url = it["tracks"][0]["url"].as_str().unwrap().to_string();
    assert!(
        url.starts_with(&format!("http://127.0.0.1:{port}/download/fixture-set/")),
        "{url}"
    );
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let (st, q) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"items": [{"uri": url, "title": "One", "album": "Fixture Set"}], "replace": true, "play": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{q}");
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    assert_eq!(zone.wait_stopped().await.error, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jellyfin_signs_in_once_and_keeps_only_the_token() {
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    const SONG: &str = "0123456789abcdef0123456789abcdef";
    const ALBUM: &str = "fedcba9876543210fedcba9876543210";
    const PL: &str = "aaaabbbbccccddddeeeeffff00001111";
    let song = serde_json::json!({"Id": SONG, "Name": "Bohemian Rhapsody", "Type": "Audio", "Artists": ["Queen"],
        "Album": "A Night at the Opera", "AlbumId": ALBUM, "AlbumPrimaryImageTag": "x", "RunTimeTicks": 50_000_000u64, "IndexNumber": 11,
        "ProviderIds": {"MusicBrainzRecording": "b1a9c0e9-d987-4042-ae91-78d6a3267d69"}, "MediaSources": [{"Container": "flac"}]});
    let album = serde_json::json!({"Id": ALBUM, "Name": "A Night at the Opera", "Type": "MusicAlbum", "AlbumArtist": "Queen",
        "ProductionYear": 1975, "ImageTags": {"Primary": "y"}});
    // Every call but the public info and the sign-in carries the token
    fn authed(h: &axum::http::HeaderMap, q: &std::collections::HashMap<String, String>) -> bool {
        h.get("authorization")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|a| a.contains("Token=\"T1\"") && a.contains("Client=\"Odeion\""))
            || q.get("api_key").map(String::as_str) == Some("T1")
    }
    type Q = axum::extract::Query<std::collections::HashMap<String, String>>;
    let (s1, s2, a1) = (song.clone(), song.clone(), album.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = Router::new()
        .route("/System/Info/Public", get(|| async { Json(serde_json::json!({"ServerName": "Rigifin", "Version": "10.11.11", "ProductName": "Jellyfin Server"})) }))
        .route(
            "/Users/AuthenticateByName",
            post(|h: axum::http::HeaderMap, Json(b): Json<serde_json::Value>| async move {
                assert!(h["authorization"].to_str().unwrap().starts_with("MediaBrowser Client="));
                if b["Username"] == "owner" && b["Pw"] == "pw" {
                    (StatusCode::OK, Json(serde_json::json!({"AccessToken": "T1", "User": {"Id": "u1", "Name": "owner"}})))
                } else {
                    (StatusCode::UNAUTHORIZED, Json(serde_json::json!({})))
                }
            }),
        )
        .route(
            "/Items",
            get(move |h: axum::http::HeaderMap, q: Q| {
                let (s1, a1) = (s1.clone(), a1.clone());
                async move {
                    assert!(authed(&h, &q));
                    let items = match (q.get("ParentId"), q.get("IncludeItemTypes").map(String::as_str)) {
                        (Some(_), _) => vec![s1],
                        (None, Some("Playlist")) => vec![serde_json::json!({"Id": PL, "Name": "Car", "Type": "Playlist", "MediaType": "Audio", "ChildCount": 2})],
                        _ => vec![s1, a1],
                    };
                    Json(serde_json::json!({"Items": items}))
                }
            }),
        )
        .route(
            "/Items/{id}",
            get(move |h: axum::http::HeaderMap, q: Q, axum::extract::Path(id): axum::extract::Path<String>| {
                let (s2, a2) = (s2.clone(), album.clone());
                async move {
                    assert!(authed(&h, &q));
                    Json(match id.as_str() {
                        ALBUM => a2,
                        PL => serde_json::json!({"Id": PL, "Name": "Car", "Type": "Playlist"}),
                        _ => s2,
                    })
                }
            }),
        )
        .route(
            "/Playlists/{id}/Items",
            get(move |h: axum::http::HeaderMap, q: Q| {
                let s3 = song.clone();
                async move {
                    assert!(authed(&h, &q));
                    Json(serde_json::json!({"Items": [s3, {"Id": "b", "Name": "Untagged", "Type": "Audio", "ProviderIds": {}}]}))
                }
            }),
        )
        .route(
            "/Audio/{id}/stream",
            get(move |h: axum::http::HeaderMap, q: Q| {
                let flac = flac.clone();
                async move {
                    if !authed(&h, &q) {
                        return (StatusCode::UNAUTHORIZED, [(header::CONTENT_TYPE, "text/plain")], Vec::new());
                    }
                    (StatusCode::OK, [(header::CONTENT_TYPE, "audio/flac")], flac)
                }
            }),
        )
        .route(
            "/Items/{id}/Images/Primary",
            get(|h: axum::http::HeaderMap, q: Q| async move {
                assert!(authed(&h, &q));
                ([(header::CONTENT_TYPE, "image/png")], b"\x89PNG\r\n\x1a\n".to_vec())
            }),
        )
        .route("/Sessions/Logout", post(|| async { StatusCode::NO_CONTENT }));
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let url = format!("http://127.0.0.1:{port}/");
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/sources/jellyfin/server",
        Some(serde_json::json!({"url": url, "user": "owner", "password": "nope"})),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, v) = call(
        &app,
        "PUT",
        "/v1/sources/jellyfin/server",
        Some(serde_json::json!({"url": url, "user": "owner", "password": "pw"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["server_name"], "Rigifin");
    {
        let stored = s
            .library
            .lock()
            .unwrap()
            .get_setting("jellyfin.server")
            .unwrap()
            .unwrap();
        assert!(
            stored.contains("T1") && !stored.contains("\"pw\""),
            "the token is kept, the password is not"
        );
    }
    let (st, r) = call(&app, "GET", "/v1/sources/jellyfin/search?q=bohemian", None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["items"][0]["kind"], "album");
    assert_eq!(r["items"][1]["uri"], format!("jellyfin:track:{SONG}"));
    assert_eq!(
        r["items"][1]["url"],
        "https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69"
    );
    assert!(!r.to_string().contains("T1"));
    let (st, al) = call(
        &app,
        "GET",
        &format!("/v1/sources/jellyfin/items/album:{ALBUM}"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{al}");
    assert_eq!(al["tracks"][0]["format"], "FLAC (Jellyfin)");
    // A zone plays the original file
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    call(&app, "POST", &format!("/v1/zones/{zid}/queue"), Some(serde_json::json!({"items": [{"uri": format!("jellyfin:track:{SONG}"), "title": "x"}], "replace": true, "play": true}))).await;
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    assert_eq!(zone.wait_stopped().await.error, None);
    // Browsers and covers go through the node
    for (path, magic) in [
        (format!("/v1/sources/jellyfin/stream/{SONG}"), &b"fLaC"[..]),
        (
            format!("/v1/sources/jellyfin/cover/{ALBUM}"),
            &b"\x89PNG\r\n\x1a\n"[..],
        ),
    ] {
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::get(path)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            &resp.into_body().collect().await.unwrap().to_bytes()[..magic.len()],
            magic
        );
    }
    // Playlists import by MusicBrainz recording
    let (_, pls) = call(&app, "GET", "/v1/sources/jellyfin/playlists", None).await;
    assert_eq!(pls[0]["name"], "Car");
    let (st, imp) = call(
        &app,
        "POST",
        &format!("/v1/sources/jellyfin/playlists/{PL}/import"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{imp}");
    assert_eq!(
        (imp["imported"].as_u64(), imp["skipped"].as_u64()),
        (Some(1), Some(1))
    );
    let (_, v) = call(&app, "DELETE", "/v1/sources/jellyfin/server", None).await;
    assert_eq!(v["connected"], false);
    assert!(s
        .library
        .lock()
        .unwrap()
        .get_setting("jellyfin.server")
        .unwrap()
        .is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plex_signs_in_with_a_pin_and_keeps_the_token_to_itself() {
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let song = serde_json::json!({"ratingKey": "1201", "type": "track", "title": "Bohemian Rhapsody",
        "grandparentTitle": "Queen", "parentTitle": "A Night at the Opera", "parentRatingKey": "1200",
        "parentThumb": "/library/metadata/1200/thumb/1", "duration": 5000, "index": 11,
        "Media": [{"audioCodec": "flac", "container": "flac", "Part": [{"key": "/library/parts/9/1/file.flac"}]}]});
    let album = serde_json::json!({"ratingKey": "1200", "type": "album", "title": "A Night at the Opera",
        "parentTitle": "Queen", "year": 1975, "thumb": "/library/metadata/1200/thumb/1"});
    type Q = axum::extract::Query<std::collections::HashMap<String, String>>;
    fn authed(h: &axum::http::HeaderMap, q: &std::collections::HashMap<String, String>) -> bool {
        h.get("x-plex-token").and_then(|v| v.to_str().ok()) == Some("T1")
            || q.get("X-Plex-Token").map(String::as_str) == Some("T1")
    }
    let mc = |m: Vec<serde_json::Value>| Json(serde_json::json!({"MediaContainer": {"Metadata": m}}));
    let pms_l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pms_port = pms_l.local_addr().unwrap().port();
    let (s1, s2, a1, a2) = (song.clone(), song.clone(), album.clone(), album.clone());
    let pms = Router::new()
        .route("/identity", get(|| async { Json(serde_json::json!({"MediaContainer": {"machineIdentifier": "m1", "version": "1.41.0"}})) }))
        .route(
            "/",
            get(|h: axum::http::HeaderMap, q: Q| async move {
                if !authed(&h, &q) {
                    return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({})));
                }
                (StatusCode::OK, Json(serde_json::json!({"MediaContainer": {"friendlyName": "Rigiplex", "machineIdentifier": "m1", "version": "1.41.0"}})))
            }),
        )
        .route(
            "/hubs/search",
            get(move |h: axum::http::HeaderMap, q: Q| {
                let (s1, a1) = (s1.clone(), a1.clone());
                async move {
                    assert!(authed(&h, &q) && q["query"] == "bohemian");
                    Json(serde_json::json!({"MediaContainer": {"Hub": [
                        {"type": "movie", "Metadata": [{"ratingKey": "7", "type": "movie", "title": "Bohemian Rhapsody"}]},
                        {"type": "track", "Metadata": [s1]},
                        {"type": "album", "Metadata": [a1]}]}}))
                }
            }),
        )
        .route(
            "/library/metadata/{id}",
            get(move |h: axum::http::HeaderMap, q: Q, axum::extract::Path(id): axum::extract::Path<String>| {
                let (s2, a2) = (s2.clone(), a2.clone());
                async move {
                    assert!(authed(&h, &q));
                    mc(vec![if id == "1200" { a2 } else { s2 }])
                }
            }),
        )
        .route(
            "/library/metadata/{id}/children",
            get(move |h: axum::http::HeaderMap, q: Q| {
                let s3 = song.clone();
                async move {
                    assert!(authed(&h, &q));
                    mc(vec![s3])
                }
            }),
        )
        .route(
            "/library/metadata/{id}/thumb/{ver}",
            get(|h: axum::http::HeaderMap, q: Q| async move {
                assert!(authed(&h, &q));
                ([(header::CONTENT_TYPE, "image/png")], b"\x89PNG\r\n\x1a\n".to_vec())
            }),
        )
        .route(
            "/library/parts/{a}/{b}/{file}",
            get(move |h: axum::http::HeaderMap, q: Q| {
                let flac = flac.clone();
                async move {
                    if !authed(&h, &q) {
                        return (StatusCode::UNAUTHORIZED, [(header::CONTENT_TYPE, "text/plain")], Vec::new());
                    }
                    (StatusCode::OK, [(header::CONTENT_TYPE, "audio/flac")], flac)
                }
            }),
        );
    tokio::spawn(async move { axum::serve(pms_l, pms).await.unwrap() });

    // plex.tv: the PIN is approved on the second poll
    let tv_l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tv_port = tv_l.local_addr().unwrap().port();
    let polls = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let tv = Router::new()
        .route(
            "/api/v2/pins",
            post(|h: axum::http::HeaderMap, q: Q| async move {
                assert!(h["x-plex-client-identifier"].to_str().unwrap().starts_with("pubky-music-"));
                assert_eq!(q["strong"], "true");
                Json(serde_json::json!({"id": 42, "code": "abcd1234"}))
            }),
        )
        .route(
            "/api/v2/pins/{id}",
            get(move || {
                let n = polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move {
                    Json(if n == 0 {
                        serde_json::json!({"id": 42, "authToken": null})
                    } else {
                        serde_json::json!({"id": 42, "authToken": "ACCOUNT"})
                    })
                }
            }),
        )
        .route("/api/v2/user", get(|| async { Json(serde_json::json!({"username": "owner"})) }))
        .route(
            "/api/v2/resources",
            get(move |h: axum::http::HeaderMap| async move {
                assert_eq!(h["x-plex-token"], "ACCOUNT");
                Json(serde_json::json!([
                    {"name": "Phone", "provides": "player", "connections": []},
                    {"name": "Rigiplex", "provides": "server", "owned": true, "accessToken": "T1", "connections": [
                        {"uri": "http://127.0.0.1:1", "local": true, "relay": false},
                        {"uri": format!("http://127.0.0.1:{pms_port}"), "local": false, "relay": false}]}]))
            }),
        );
    tokio::spawn(async move { axum::serve(tv_l, tv).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.plex.write().unwrap().tv_base = format!("http://127.0.0.1:{tv_port}");
    let app = router(s.clone());
    // By address: a wrong token is refused
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/sources/plex/server",
        Some(serde_json::json!({"url": format!("http://127.0.0.1:{pms_port}"), "token": "nope"})),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    // Sign in with Plex
    let (st, pin) = call(&app, "POST", "/v1/sources/plex/pin", None).await;
    assert_eq!(st, StatusCode::OK, "{pin}");
    let auth = pin["auth_url"].as_str().unwrap();
    assert!(
        auth.starts_with("https://app.plex.tv/auth#?clientID=pubky-music-") && auth.contains("code=abcd1234"),
        "{auth}"
    );
    let (_, w) = call(&app, "GET", "/v1/sources/plex/pin/42", None).await;
    assert_eq!(w["status"], "waiting");
    let (st, c) = call(&app, "GET", "/v1/sources/plex/pin/42", None).await;
    assert_eq!(st, StatusCode::OK, "{c}");
    assert_eq!(c["status"], "connected");
    assert_eq!(c["server"]["server_name"], "Rigiplex");
    assert_eq!(c["server"]["user"], "owner");
    assert_eq!(
        c["server"]["url"],
        format!("http://127.0.0.1:{pms_port}"),
        "the dead local connection is skipped"
    );
    {
        let stored = s
            .library
            .lock()
            .unwrap()
            .get_setting("plex.server")
            .unwrap()
            .unwrap();
        assert!(
            stored.contains("T1") && !stored.contains("ACCOUNT"),
            "the server's token is kept, the account's is not"
        );
    }
    let (_, srcs) = call(&app, "GET", "/v1/sources", None).await;
    assert!(srcs.to_string().contains("Your Plex “Rigiplex”"), "{srcs}");
    let (st, r) = call(&app, "GET", "/v1/sources/plex/search?q=bohemian", None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["items"].as_array().unwrap().len(), 2, "films are left out: {r}");
    assert_eq!(r["items"][0]["kind"], "album");
    assert_eq!(r["items"][1]["uri"], "plex:track:1201");
    assert!(!r.to_string().contains("T1"));
    let (st, al) = call(&app, "GET", "/v1/sources/plex/items/album:1200", None).await;
    assert_eq!(st, StatusCode::OK, "{al}");
    assert_eq!(
        (al["year"].as_i64(), al["tracks"][0]["format"].as_str()),
        (Some(1975), Some("FLAC (Plex)"))
    );
    // A zone plays the original file
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    call(&app, "POST", &format!("/v1/zones/{zid}/queue"), Some(serde_json::json!({"items": [{"uri": "plex:track:1201", "title": "x"}], "replace": true, "play": true}))).await;
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    assert_eq!(zone.wait_stopped().await.error, None);
    // Browsers and covers go through the node
    for (path, magic) in [
        ("/v1/sources/plex/stream/1201", &b"fLaC"[..]),
        ("/v1/sources/plex/cover/1200/1", &b"\x89PNG\r\n\x1a\n"[..]),
    ] {
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::get(path)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{path}");
        assert_eq!(
            &resp.into_body().collect().await.unwrap().to_bytes()[..magic.len()],
            magic
        );
    }
    let (_, v) = call(&app, "DELETE", "/v1/sources/plex/server", None).await;
    assert_eq!(v["connected"], false);
    assert!(s
        .library
        .lock()
        .unwrap()
        .get_setting("plex.server")
        .unwrap()
        .is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn audiobookshelf_asks_non_admins_for_a_key_and_ends_the_session() {
    let logouts = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let lo = logouts.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = Router::new()
        .route("/status", get(|| async { Json(serde_json::json!({"app": "audiobookshelf", "serverVersion": "2.37.1"})) }))
        .route(
            "/login",
            post(|h: axum::http::HeaderMap| async move {
                assert_eq!(h["x-return-tokens"], "true");
                Json(serde_json::json!({"user": {"id": "u2", "username": "kid", "type": "user", "accessToken": "eyJACCESS", "refreshToken": "eyJREFRESH"}}))
            }),
        )
        .route(
            "/logout",
            post(move |h: axum::http::HeaderMap| {
                let lo = lo.clone();
                async move {
                    assert_eq!(h["x-refresh-token"], "eyJREFRESH");
                    lo.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    StatusCode::OK
                }
            }),
        )
        .route(
            "/api/me",
            get(|h: axum::http::HeaderMap| async move {
                if h["authorization"] != "Bearer KEY1" {
                    return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({})));
                }
                (StatusCode::OK, Json(serde_json::json!({"username": "kid", "type": "user"})))
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let url = format!("http://127.0.0.1:{port}");
    let (st, v) = call(
        &app,
        "PUT",
        "/v1/sources/audiobookshelf/server",
        Some(serde_json::json!({"url": url, "user": "kid", "password": "pw"})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    assert!(v.to_string().contains("API Keys"), "{v}");
    assert_eq!(
        logouts.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the session is ended"
    );
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/sources/audiobookshelf/server",
        Some(serde_json::json!({"url": url, "api_key": "WRONG"})),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, v) = call(
        &app,
        "PUT",
        "/v1/sources/audiobookshelf/server",
        Some(serde_json::json!({"url": url, "api_key": "KEY1"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        (v["connected"].as_bool(), v["user"].as_str()),
        (Some(true), Some("kid"))
    );
    assert!(!v.to_string().contains("KEY1"));
    let (_, srcs) = call(&app, "GET", "/v1/sources", None).await;
    assert!(srcs.to_string().contains("Your audiobooks at"), "{srcs}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drive_folders_sign_in_with_pkce_scan_and_play() {
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let refreshes = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let base = mock_drives(flac.clone(), refreshes.clone()).await;
    for (provider, prefix, scheme) in [("google", "g", "gdrive://"), ("onedrive", "m", "onedrive://")] {
        let dir = tempfile::tempdir().unwrap();
        let s = state(dir.path(), None).await;
        {
            let mut d = s.drives.write().unwrap();
            d.endpoints = drives::Endpoints {
                google_auth: format!("{base}/google/auth"),
                google_token: format!("{base}/google/token"),
                google_api: format!("{base}/gapi"),
                ms_login: format!("{base}/ms"),
                ms_graph: format!("{base}/graph"),
            };
        }
        s.set_drive_apps(drives::apps_from(
            drives::App {
                client_id: "g-client".into(),
                client_secret: String::new(),
            },
            drives::App {
                client_id: "m-client".into(),
                client_secret: String::new(),
            },
        ));
        let app = router(s.clone());
        let (_, v) = call(&app, "GET", "/v1/library/drives", None).await;
        assert!(
            v["providers"]
                .as_array()
                .unwrap()
                .iter()
                .all(|p| p["configured"] == true && p["connected"] == false),
            "{v}"
        );
        // Sign in: PKCE, no secret, through the return page
        let (st, login) = call(
            &app,
            "POST",
            &format!("/v1/library/drives/{provider}/login"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{login}");
        let u = url::Url::parse(login["url"].as_str().unwrap()).unwrap();
        let q: std::collections::HashMap<String, String> = u.query_pairs().into_owned().collect();
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["redirect_uri"], crate::project_apps::RETURN_PAGE);
        assert!(q["state"].contains('.'), "the state carries the node's callback");
        assert!(!q.contains_key("client_secret"));
        let resp = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/drives/{provider}/callback?code=C0DE&state={}",
                    q["state"]
                ))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            resp.headers()["location"].to_str().unwrap().contains("drive=ok"),
            "{:?}",
            resp.headers()
        );
        let (_, v) = call(&app, "GET", "/v1/library/drives", None).await;
        let pv = v["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == provider)
            .unwrap()
            .clone();
        assert_eq!(pv["connected"], true);
        assert!(pv["user"].as_str().unwrap().starts_with("me@"), "{pv}");
        // Pick the folder and add it
        let (_, folders) = call(
            &app,
            "GET",
            &format!("/v1/library/drives/{provider}/folders"),
            None,
        )
        .await;
        assert_eq!(folders[0]["name"], "Music", "{folders}");
        let (st, root) = call(
            &app,
            "POST",
            "/v1/library/drives",
            Some(serde_json::json!({"provider": provider, "folder_id": folders[0]["id"], "name": "Music"})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{root}");
        let mut roots = serde_json::Value::Null;
        for _ in 0..100 {
            let (_, v) = call(&app, "GET", "/v1/library/drives", None).await;
            if v["roots"][0]["tracks"] == 2 {
                roots = v["roots"].clone();
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert_eq!(
            roots[0]["tracks"], 2,
            "{provider}: both songs, the one in Sub too"
        );
        let (_, tracks) = call(&app, "GET", "/v1/library/tracks", None).await;
        let t = tracks["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["file"]["path"].as_str().unwrap().ends_with("Sub/02 Two.flac"))
            .unwrap()
            .clone();
        assert!(t["file"]["path"].as_str().unwrap().starts_with(scheme));
        // Played in a zone, and relayed to a browser byte for byte
        let tid = t["id"].as_i64().unwrap();
        let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
        let zid = zones[0]["id"].as_str().unwrap().to_string();
        call(
            &app,
            "POST",
            &format!("/v1/zones/{zid}/queue"),
            Some(serde_json::json!({"track_ids": [tid], "replace": true, "play": true})),
        )
        .await;
        let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
        assert_eq!(zone.wait_stopped().await.error, None, "{provider}");
        let resp = app
            .clone()
            .oneshot(
                Request::get(format!("/v1/library/tracks/{tid}/file"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.into_body().collect().await.unwrap().to_bytes().to_vec(),
            flac,
            "{provider}"
        );
        assert!(
            !tracks.to_string().contains(&format!("{prefix}-A")),
            "no token in an answer"
        );
        // Out of the library again
        let (_, gone) = call(
            &app,
            "DELETE",
            &format!("/v1/library/drives/roots/{}", root["id"].as_str().unwrap()),
            None,
        )
        .await;
        assert_eq!(gone["tracks"], 2, "{gone}");
    }
    assert!(
        refreshes.load(std::sync::atomic::Ordering::SeqCst) >= 2,
        "expired tokens were refreshed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jamendo_searches_plays_lossless_and_keeps_the_app_id_to_itself() {
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let track = move |fmt: &str| {
        serde_json::json!({"id": "1369762", "name": "Trifle 9", "duration": 5, "artist_name": "Vladimir",
            "album_name": "Trifles", "album_id": "164070", "releasedate": "2016-08-20", "position": 1,
            "image": "https://usercontent.jamendo.com/x.jpg",
            "audio": format!("{base}/stream?trackid=1369762&format={fmt}&from=app-SECRETID"),
            "license_ccurl": "http://creativecommons.org/licenses/by/3.0/", "shareurl": "https://www.jamendo.com/track/1369762"})
    };
    type Q = axum::extract::Query<std::collections::HashMap<String, String>>;
    let mock = Router::new()
        .route(
            "/tracks/",
            get(move |q: Q| {
                let t = track(q.get("audioformat").map(String::as_str).unwrap_or("mp31"));
                async move {
                    assert_eq!(q.get("client_id").map(String::as_str), Some("SECRETID"));
                    Json(serde_json::json!({"headers": {"status": "success"}, "results": [t]}))
                }
            }),
        )
        .route(
            "/albums/",
            get(|| async {
                Json(serde_json::json!({"headers": {"status": "success"}, "results": [
                    {"id": "164070", "name": "Trifles", "artist_name": "Vladimir", "releasedate": "2016-08-20", "image": "https://usercontent.jamendo.com/a.jpg"}]}))
            }),
        )
        .route(
            "/stream",
            get(move || {
                let flac = flac.clone();
                async move { ([(header::CONTENT_TYPE, "audio/flac")], flac) }
            }),
        )
        .route(
            "/oauth/grant",
            post(|body: String| async move {
                let form: std::collections::HashMap<String, String> =
                    url::form_urlencoded::parse(body.as_bytes()).into_owned().collect();
                assert_eq!(form["client_secret"], "SECRETKEY");
                let ok = match form["grant_type"].as_str() {
                    "authorization_code" => form["code"] == "C0DE" && form["redirect_uri"].ends_with("/jamendo/callback"),
                    "refresh_token" => form["refresh_token"] == "R1",
                    _ => false,
                };
                assert!(ok, "{form:?}");
                let token = if form["grant_type"] == "refresh_token" { "A2" } else { "A1" };
                Json(serde_json::json!({"access_token": token, "expires_in": 7200, "token_type": "bearer", "scope": "music", "refresh_token": "R1"}))
            }),
        )
        .route(
            "/users/",
            get(|| async { Json(serde_json::json!({"headers": {"status": "success"}, "results": [{"id": "9", "name": "owner", "dispname": "The Owner"}]})) }),
        )
        .route(
            "/users/tracks/",
            get(|q: Q| async move {
                assert_eq!(q.get("access_token").map(String::as_str), Some("A2"), "the renewed token");
                assert_eq!(q.get("relation").map(String::as_str), Some("like favorite"));
                Json(serde_json::json!({"headers": {"status": "success"}, "results": [{"id": "9", "tracks": [{"id": "1369762"}, {"id": "1427344"}, {"id": "1369762"}]}]}))
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    {
        let mut j = s.jamendo.write().unwrap();
        j.base = format!("http://127.0.0.1:{port}");
        j.creds = crate::jamendo::JamendoCredentials {
            client_id: "SECRETID".into(),
            client_secret: "SECRETKEY".into(),
        };
    }
    let app = router(s.clone());
    let (_, sources) = call(&app, "GET", "/v1/sources", None).await;
    assert!(sources
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["id"] == "jamendo" && x["status"] == "ready"));
    let (st, r) = call(&app, "GET", "/v1/sources/jamendo/search?q=trifle", None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["items"][0]["kind"], "album");
    assert_eq!(r["items"][1]["uri"], "jamendo:track:1369762");
    let (st, al) = call(&app, "GET", "/v1/sources/jamendo/items/album:164070", None).await;
    assert_eq!(st, StatusCode::OK, "{al}");
    assert_eq!(al["tracks"][0]["url"], "jamendo:track:1369762");
    assert_eq!(al["tracks"][0]["format"], "FLAC (Jamendo)");
    for body in [&r, &al] {
        assert!(
            !body.to_string().contains("SECRET"),
            "the app id never leaves the node: {body}"
        );
    }
    // A zone plays it (the lossless stream, looked up at play time)
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let (st, q) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"items": [{"uri": "jamendo:track:1369762", "title": "Trifle 9"}], "replace": true, "play": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{q}");
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    assert_eq!(zone.wait_stopped().await.error, None);
    // Browsers get it relayed by the node
    let resp = app
        .clone()
        .oneshot(
            axum::http::Request::get("/v1/sources/jamendo/stream/1369762")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        resp.headers().get("location").is_none(),
        "relayed, not redirected"
    );
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..4], b"fLaC");
    // A Jamendo link in a playlist plays from Jamendo
    let (_, pl) = call(
        &app,
        "POST",
        "/v1/playlists",
        Some(serde_json::json!({"name": "CC", "uris": ["https://www.jamendo.com/track/1369762"]})),
    )
    .await;
    let id = pl["summary"]["id"].as_str().unwrap().to_string();
    let (_, pf) = call(&app, "GET", &format!("/v1/playlists/{id}/play-from"), None).await;
    assert_eq!(pf["items"][0]["plays_from"]["service"], "jamendo", "{pf}");
    assert_eq!(pf["items"][0]["stream_uri"], "jamendo:track:1369762");
    assert!(!pf.to_string().contains("SECRET"));

    // Sign in (OAuth, scope music), import likes and favourites, sign out
    let (_, st0) = call(&app, "GET", "/v1/sources/jamendo/status", None).await;
    assert_eq!(st0["signed_in"], false);
    let (st, l) = call(&app, "POST", "/v1/sources/jamendo/login", None).await;
    assert_eq!(st, StatusCode::OK, "{l}");
    let url = url::Url::parse(l["url"].as_str().unwrap()).unwrap();
    assert!(url.path().ends_with("/oauth/authorize"));
    let q: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(q["scope"], "music");
    assert!(q["redirect_uri"].ends_with("/jamendo/callback"));
    let resp = app
        .clone()
        .oneshot(
            axum::http::Request::get(format!("/jamendo/callback?code=C0DE&state={}", q["state"]))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.headers()["location"], "/source/jamendo?jamendo=signed_in");
    let (_, st1) = call(&app, "GET", "/v1/sources/jamendo/status", None).await;
    assert_eq!(st1["signed_in"], true);
    assert_eq!(st1["name"], "The Owner");
    // The access token has expired: the import renews it first
    {
        let lib = s.library.lock().unwrap();
        let mut u: serde_json::Value =
            serde_json::from_str(&lib.get_setting("jamendo.user").unwrap().unwrap()).unwrap();
        u["expires_at"] = serde_json::json!(0);
        lib.set_setting("jamendo.user", &u.to_string()).unwrap();
    }
    let (st, r) = call(&app, "POST", "/v1/sources/jamendo/collection/import", None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["imported"], 2, "two tracks, one listed twice");
    let (_, favs) = call(&app, "GET", "/v1/bookmarks", None).await;
    assert!(
        favs.to_string().contains("https://www.jamendo.com/track/1427344"),
        "{favs}"
    );
    let (_, st2) = call(&app, "POST", "/v1/sources/jamendo/logout", None).await;
    assert_eq!(st2["signed_in"], false);
    let resp = app
        .clone()
        .oneshot(
            axum::http::Request::get("/jamendo/callback?code=C0DE&state=unknown")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(resp.headers()["location"]
        .to_str()
        .unwrap()
        .starts_with("/source/jamendo?jamendo=error"));
}

#[tokio::test]
async fn audius_source_searches_and_plays() {
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let track = serde_json::json!({"id": "W0NJ0", "title": "Ambient Piano Interlude", "user": {"name": "Techwipe"}, "duration": 5,
        "permalink": "/techwipe/ambient-piano-interlude-353852", "artwork": {"480x480": "https://img/480"}, "play_count": 3});
    let t1 = track.clone();
    let t2 = track.clone();
    let mock = Router::new()
        .route(
            "/v1/tracks/search",
            get(move || {
                let t = t1.clone();
                async move { Json(serde_json::json!({"data": [t]})) }
            }),
        )
        .route(
            "/v1/tracks/{id}",
            get(move || {
                let t = t2.clone();
                async move { Json(serde_json::json!({"data": t})) }
            }),
        )
        .route(
            "/v1/tracks/{id}/stream",
            get(move || {
                let flac = flac.clone();
                async move { ([(header::CONTENT_TYPE, "audio/flac")], flac) }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    *s.audius_base.write().unwrap() = format!("http://127.0.0.1:{port}");
    let app = router(s.clone());
    let (_, sources) = call(&app, "GET", "/v1/sources", None).await;
    assert!(sources.as_array().unwrap().iter().any(|x| x["id"] == "audius"));
    let (st, _) = call(&app, "GET", "/v1/sources/nope/search?q=x", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, r) = call(&app, "GET", "/v1/sources/audius/search?q=ambient", None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["source"], "audius");
    assert_eq!(r["items"][0]["kind"], "track");
    assert_eq!(
        r["items"][0]["url"],
        "https://audius.co/techwipe/ambient-piano-interlude-353852"
    );
    let (st, it) = call(&app, "GET", "/v1/sources/audius/items/W0NJ0", None).await;
    assert_eq!(st, StatusCode::OK, "{it}");
    let url = it["tracks"][0]["url"].as_str().unwrap().to_string();
    assert!(
        url.ends_with("/v1/tracks/W0NJ0/stream?app_name=pubky-music-node"),
        "{url}"
    );
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let (st, q) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"items": [{"uri": url, "title": "Ambient Piano Interlude"}], "replace": true, "play": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{q}");
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    assert_eq!(zone.wait_stopped().await.error, None);
}

/// Deezer against a local mock: artists, albums and tracks come back from
/// one search with ISRC/UPC, an album item lists its tracks, versions of a
/// library track/album are found by ISRC/UPC, the browser proxy redirects
/// to the preview, and a `deezer:track:` URI plays through the zone.
#[tokio::test]
async fn deezer_source_searches_matches_and_plays_previews() {
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let preview = format!("http://127.0.0.1:{port}/preview.mp3");
    let track = serde_json::json!({"id": 138547415, "title": "Creep", "isrc": "GBAYE9200070", "duration": 238, "rank": 9,
        "preview": preview, "artist": {"id": 399, "name": "Radiohead"},
        "album": {"id": 12047952, "title": "Pablo Honey", "cover_medium": "http://img/pablo.jpg", "release_date": "1993-02-22"}});
    let album = serde_json::json!({"id": 12047952, "title": "Pablo Honey", "upc": "724381230321", "release_date": "1993-02-22", "label": "Parlophone",
        "cover_big": "http://img/pablo-big.jpg", "artist": {"id": 399, "name": "Radiohead"}, "tracks": {"data": [track]}});
    let artist = serde_json::json!({"id": 399, "name": "Radiohead", "nb_fan": 1000, "picture_big": "http://img/rh.jpg"});
    let (t1, t2, t3, a1, a2, a3, a4, ar1, ar2, ar3) = (
        track.clone(),
        track.clone(),
        track.clone(),
        album.clone(),
        album.clone(),
        album.clone(),
        album.clone(),
        artist.clone(),
        artist.clone(),
        artist.clone(),
    );
    let mock = Router::new()
        .route(
            "/search/track",
            get(move || {
                let t = t1.clone();
                async move { Json(serde_json::json!({"data": [t]})) }
            }),
        )
        .route(
            "/search/album",
            get(move || {
                let a = a1.clone();
                async move { Json(serde_json::json!({"data": [a]})) }
            }),
        )
        .route(
            "/search/artist",
            get(move || {
                let a = ar1.clone();
                async move { Json(serde_json::json!({"data": [a]})) }
            }),
        )
        .route(
            "/album/{id}",
            get(move |Path(id): Path<String>| {
                let a = a2.clone();
                async move {
                    if id == "12047952" || id == "upc:724381230321" {
                        Json(a)
                    } else {
                        Json(serde_json::json!({"error": {"code": 800, "message": "no data"}}))
                    }
                }
            }),
        )
        .route(
            "/track/{id}",
            get(move |Path(id): Path<String>| {
                let t = t2.clone();
                async move {
                    if id == "138547415" || id == "isrc:GBAYE9200070" {
                        Json(t)
                    } else {
                        Json(serde_json::json!({"error": {"code": 800, "message": "no data"}}))
                    }
                }
            }),
        )
        .route(
            "/artist/{id}",
            get(move || {
                let a = ar2.clone();
                async move { Json(a) }
            }),
        )
        .route(
            "/artist/{id}/albums",
            get(move || {
                let a = a3.clone();
                async move { Json(serde_json::json!({"data": [a]})) }
            }),
        )
        .route(
            "/preview.mp3",
            get(move || {
                let flac = flac.clone();
                async move { ([(header::CONTENT_TYPE, "audio/flac")], flac) }
            }),
        );
    let _ = (t3, a4, ar3);
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    *s.deezer_base.write().unwrap() = format!("http://127.0.0.1:{port}");
    let app = router(s.clone());
    let (st, r) = call(&app, "GET", "/v1/sources/deezer/search?q=creep", None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    let kinds: Vec<&str> = r["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["artist", "album", "track"]);
    assert_eq!(r["items"][2]["uri"], "deezer:track:138547415");
    assert_eq!(r["items"][2]["isrc"], "GBAYE9200070");
    assert_eq!(r["items"][2]["duration_ms"], 30_000, "the preview length");
    assert_eq!(r["items"][1]["upc"], "724381230321");
    let (st, it) = call(&app, "GET", "/v1/sources/deezer/items/album:12047952", None).await;
    assert_eq!(st, StatusCode::OK, "{it}");
    assert_eq!(it["tracks"][0]["url"], "deezer:track:138547415");
    assert_eq!(it["image"], "http://img/pablo-big.jpg");
    let (st, _) = call(&app, "GET", "/v1/sources/deezer/items/album:1", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, ar) = call(&app, "GET", "/v1/sources/deezer/artists/artist:399", None).await;
    assert_eq!(st, StatusCode::OK, "{ar}");
    assert_eq!(ar["albums"][0]["id"], "album:12047952");
    // The unified search shows Deezer without any configuration.
    let (st, u) = call(&app, "GET", "/v1/library/search?q=creep", None).await;
    assert_eq!(st, StatusCode::OK, "{u}");
    assert!(
        u["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["source"] == "deezer"),
        "{u}"
    );
    // Versions of a library track/album with the same ISRC/UPC.
    let tid = {
        let lib = s.library.lock().unwrap();
        let t = pm_model::Track {
            id: None,
            title: "Creep".into(),
            artists: vec![pm_model::Artist {
                id: None,
                name: "Radiohead".into(),
                mbid: None,
            }],
            album: Some(pm_model::Album {
                id: None,
                title: "Pablo Honey".into(),
                album_artist: Some("Radiohead".into()),
                release_mbid: None,
                release_group_mbid: None,
                year: Some(1993),
                upc: Some(pm_model::Upc::parse("724381230321").unwrap()),
                total_discs: None,
            }),
            track_no: Some(2),
            disc_no: None,
            duration_ms: Some(238_000),
            file: pm_model::AudioFile {
                path: dir.path().join("creep.flac"),
                size_bytes: 1,
                mtime_unix: 0,
                container: "flac".into(),
                format: pm_model::AudioFormat::pcm(44_100, 16, 2),
                duration_ms: Some(238_000),
            },
            recording_mbid: None,
            isrc: Some(pm_model::Isrc::parse("GBAYE9200070").unwrap()),
            acoustid: None,
            canonical: None,
            replaygain_track_gain_db: None,
            replaygain_album_gain_db: None,
            genres: vec![],
        };
        lib.upsert_track(&t).unwrap().0
    };
    let (st, v) = call(&app, "GET", &format!("/v1/library/tracks/{tid}/versions"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v[0]["source"], "deezer");
    assert_eq!(v[0]["uri"], "deezer:track:138547415");
    let (_, t) = call(&app, "GET", &format!("/v1/library/tracks/{tid}"), None).await;
    let aid = t["album"]["id"].as_i64().unwrap();
    let (st, v) = call(&app, "GET", &format!("/v1/library/albums/{aid}/versions"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v[0]["id"], "album:12047952");
    // Browser proxy: a redirect to the preview.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/sources/deezer/stream/138547415")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        resp.headers()["location"],
        format!("http://127.0.0.1:{port}/preview.mp3")
    );
    // And the zone plays the URI itself.
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let (st, q) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"items": [{"uri": "deezer:track:138547415", "title": "Creep"}], "replace": true, "play": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{q}");
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    assert_eq!(zone.wait_stopped().await.error, None);
}

/// The owner's Deezer page (STEWARD-UX-INSTANCES item 9): Play and the eyebrow come from one
/// decision. A track Deezer doesn't offer here says why instead of failing with a 404; a
/// playable one plays its own 30 s preview.
#[tokio::test]
async fn a_deezer_page_plays_its_preview_or_says_why_not() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = Router::new().route(
        "/track/{id}",
        get(|Path(id): Path<String>| async move {
            let readable = id != "4329888262";
            Json(serde_json::json!({"id": id.parse::<u64>().unwrap(), "title": "Bohemian Rhapsody", "readable": readable,
                "preview": if readable { "https://cdn.example/p.mp3" } else { "" }, "duration": 354,
                "artist": {"id": 412, "name": "Queen"}, "album": {"id": 9, "title": "Greatest Hits"}}))
        }),
    );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    *s.deezer_base.write().unwrap() = format!("http://127.0.0.1:{port}");
    let app = router(s.clone());
    let (st, pg) = call(&app, "GET", "/v1/item/song/deezer-4329888262", None).await;
    assert_eq!(st, StatusCode::OK, "{pg}");
    assert!(pg["plays_note"].as_str().unwrap().contains("country"), "{pg}");
    assert_ne!(pg["plays_from"]["service"], "deezer", "{pg}");
    let (_, pg) = call(&app, "GET", "/v1/item/song/deezer-1109731", None).await;
    assert!(pg.get("plays_note").is_none(), "{pg}");
    assert_eq!(pg["plays_from"]["service"], "deezer");
    assert_eq!(pg["plays_from"]["mode"], "preview");
    assert_eq!(pg["songs"][0]["uri"], "deezer:track:1109731");
}

/// YouTube search with the owner's own Data API key: the key is checked before it is kept,
/// never comes back, and search finds music videos that play in This browser.
#[tokio::test]
async fn youtube_search_uses_the_owners_key() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    type Q = axum::extract::Query<std::collections::HashMap<String, String>>;
    let good = "AIzaSyD-ci-key-0123456789abcdefghijklmn";
    let mock = Router::new()
        .route("/videos", get(move |Query(q): Q| async move {
            if q["key"] != good {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": {"message": "API key not valid.", "errors": [{"reason": "keyInvalid"}]}})));
            }
            (StatusCode::OK, Json(serde_json::json!({"items": [{"id": "5NV6Rdv1a3I", "snippet": {"title": "Daft Punk - Get Lucky (Official Audio) ft. Pharrell Williams, Nile Rodgers", "channelTitle": "Daft Punk - Topic", "publishedAt": "2013-05-17T00:00:00Z", "thumbnails": {"high": {"url": "https://i.ytimg.com/vi/5NV6Rdv1a3I/hq.jpg"}}}, "contentDetails": {"duration": "PT6M10S"}}]})))
        }))
        .route("/search", get(move |Query(q): Q| async move {
            assert_eq!((q["type"].as_str(), q["videoCategoryId"].as_str(), q["key"].as_str()), ("video", "10", good));
            Json(serde_json::json!({"items": [{"id": {"kind": "youtube#video", "videoId": "5NV6Rdv1a3I"}, "snippet": {"title": "Daft Punk &amp; Pharrell - Get Lucky", "channelTitle": "Daft Punk - Topic", "publishedAt": "2013-05-17T00:00:00Z", "thumbnails": {"high": {"url": "https://i.ytimg.com/vi/5NV6Rdv1a3I/hq.jpg"}}}}]}))
        }));
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    youtube::set_api_base(&base);
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (st, _) = call(&app, "GET", "/v1/sources/youtube/search?q=get%20lucky", None).await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED, "no key, no search");
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/sources/youtube/key",
        Some(serde_json::json!({"key": "AIzaSyD-wrong-key-0123456789abcdefghijk"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "a key YouTube refuses is not kept");
    let (st, k) = call(
        &app,
        "PUT",
        "/v1/sources/youtube/key",
        Some(serde_json::json!({"key": good})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(k, serde_json::json!({"set": true}), "the key never comes back");
    let (_, srcs) = call(&app, "GET", "/v1/sources", None).await;
    let yt = srcs
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "youtube")
        .unwrap()
        .clone();
    assert_eq!(yt["status"], "browser + search");
    let (st, r) = call(
        &app,
        "GET",
        "/v1/sources/youtube/search?q=get%20lucky&limit=5",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    let hit = &r["items"][0];
    assert_eq!(hit["title"], "Daft Punk & Pharrell - Get Lucky");
    assert_eq!(hit["creator"], "Daft Punk", "a Topic channel is the artist");
    assert_eq!(hit["uri"], "youtube:video:5NV6Rdv1a3I");
    assert_eq!(hit["duration_ms"], 370_000);
    let (st, it) = call(&app, "GET", "/v1/sources/youtube/items/video:5NV6Rdv1a3I", None).await;
    assert_eq!(st, StatusCode::OK, "{it}");
    assert_eq!(it["tracks"][0]["url"], "youtube:video:5NV6Rdv1a3I");
    // Never in the everyday search (quota)
    assert!(!streaming::find("youtube").unwrap().unified(&s));
    let (_, k) = call(
        &app,
        "PUT",
        "/v1/sources/youtube/key",
        Some(serde_json::json!({"key": ""})),
    )
    .await;
    assert_eq!(k["set"], false);
}

/// SoundCloud (unofficial): with a client ID the owner entered, search, a set page, likes and
/// playlists with the owner's token, and streams (progressive MP3 for one track, HLS stitched
/// for another) from a stand-in api-v2; a refused client switches the mode off.
#[tokio::test]
async fn soundcloud_searches_and_streams_from_a_mock() {
    use pm_zones::SourceFactory;
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let refuse = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let gate = {
        let refuse = refuse.clone();
        move |q: &str| -> Option<StatusCode> {
            if refuse.load(std::sync::atomic::Ordering::SeqCst) {
                return Some(StatusCode::UNAUTHORIZED);
            }
            (!q.contains("client_id=sc-client-0123456789abcdef")).then_some(StatusCode::UNAUTHORIZED)
        }
    };
    let tr = |id: u64, title: &str, kind: &str| {
        let media = match kind {
            "progressive" => {
                serde_json::json!([{"url": format!("{base}/media/{id}/progressive"), "preset": "mp3_1_0", "format": {"protocol": "progressive", "mime_type": "audio/mpeg"}}])
            }
            "hls" => {
                serde_json::json!([{"url": format!("{base}/media/{id}/hls"), "preset": "mp3_1_0", "format": {"protocol": "hls", "mime_type": "audio/mpeg"}}])
            }
            _ => {
                serde_json::json!([{"url": format!("{base}/media/{id}/enc"), "preset": "aac_256k", "format": {"protocol": "encrypted-hls", "mime_type": "audio/mp4"}}])
            }
        };
        serde_json::json!({"id": id, "kind": "track", "title": title, "policy": "ALLOW", "permalink_url": format!("https://soundcloud.com/someone/{}", title.to_lowercase().replace(' ', "-")),
            "duration": 60000, "artwork_url": "https://i1.sndcdn.com/artworks-large.jpg", "created_at": "2024-03-02T10:00:00Z", "playback_count": 7,
            "user": {"username": "Someone"}, "track_authorization": "auth-1", "media": {"transcodings": media}})
    };
    let t1 = tr(1, "Night Drive", "progressive");
    let t2 = tr(2, "Sunrise", "hls");
    let t3 = tr(3, "Locked", "encrypted");
    let set = serde_json::json!({"id": 9, "kind": "playlist", "title": "Late Set", "permalink_url": "https://soundcloud.com/someone/sets/late-set", "user": {"username": "Someone"},
        "track_count": 2, "tracks": [t1.clone(), {"id": 2, "kind": "track"}]});
    let mk = |v: serde_json::Value| {
        let gate = gate.clone();
        move |q: axum::extract::RawQuery| {
            let v = v.clone();
            let gate = gate.clone();
            async move {
                match gate(q.0.as_deref().unwrap_or("")) {
                    Some(st) => (st, Json(serde_json::json!({"error": "no"}))),
                    None => (StatusCode::OK, Json(v)),
                }
            }
        }
    };
    let half = flac.len() / 2;
    let (seg1, seg2) = (flac[..half].to_vec(), flac[half..].to_vec());
    let mock = Router::new()
        .route("/search/tracks", get(mk(serde_json::json!({"collection": [t1.clone(), t2.clone(), t3.clone()], "next_href": null}))))
        .route("/search/playlists", get(mk(serde_json::json!({"collection": [set.clone()]}))))
        .route("/resolve", get(mk(t2.clone())))
        .route("/tracks/1", get(mk(t1.clone())))
        .route("/tracks/2", get(mk(t2.clone())))
        .route("/tracks/3", get(mk(t3.clone())))
        .route("/tracks", get(mk(serde_json::json!([t2.clone()]))))
        .route("/playlists/9", get(mk(set.clone())))
        .route("/me", get({
            let g = gate.clone();
            move |q: axum::extract::RawQuery, h: axum::http::HeaderMap| {
                let g = g.clone();
                async move {
                    let auth = h.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
                    if g(q.0.as_deref().unwrap_or("")).is_some() || auth != "OAuth tok-0123456789" {
                        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "bad token"})));
                    }
                    (StatusCode::OK, Json(serde_json::json!({"id": 77, "username": "Someone", "permalink": "someone"})))
                }
            }
        }))
        .route("/users/77/likes", get(mk(serde_json::json!({"collection": [{"track": t1.clone()}, {"playlist": set.clone()}], "next_href": null}))))
        .route("/users/77/playlists", get(mk(serde_json::json!({"collection": [set.clone()], "next_href": null}))))
        .route("/media/1/progressive", get(mk(serde_json::json!({"url": format!("{base}/cdn/1.mp3")}))))
        .route("/media/2/hls", get(mk(serde_json::json!({"url": format!("{base}/cdn/2.m3u8")}))))
        .route("/cdn/1.mp3", get({ let f = flac.clone(); move || { let f = f.clone(); async move { f } } }))
        .route("/cdn/2.m3u8", get({ let b = base.clone(); move || { let b = b.clone(); async move { format!("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:30\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:30.0,\n{b}/cdn/seg/1\n#EXTINF:30.0,\n{b}/cdn/seg/2\n#EXT-X-ENDLIST\n") } } }))
        .route("/cdn/seg/1", get({ let f = seg1.clone(); move || { let f = f.clone(); async move { f } } }))
        .route("/cdn/seg/2", get({ let f = seg2.clone(); move || { let f = f.clone(); async move { f } } }));
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    soundcloud::set_api_base(&base);
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    // Not configured: no client yet; the everyday search leaves it out
    let (_, srcs) = call(&app, "GET", "/v1/sources", None).await;
    let sc = srcs
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "soundcloud")
        .unwrap()
        .clone();
    assert!(
        sc["status"].as_str().unwrap().starts_with("not configured"),
        "{sc}"
    );
    let (st, _) = call(&app, "GET", "/v1/sources/soundcloud/search?q=night", None).await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED);
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/sources/soundcloud/unofficial",
        Some(serde_json::json!({"client_id": "short"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (_, u) = call(
        &app,
        "PUT",
        "/v1/sources/soundcloud/unofficial",
        Some(serde_json::json!({"client_id": "sc-client-0123456789abcdef", "enabled": true})),
    )
    .await;
    assert_eq!(
        (
            u["enabled"].clone(),
            u["client_from"].clone(),
            u["signed_in"].clone()
        ),
        (
            serde_json::json!(true),
            serde_json::json!("settings"),
            serde_json::json!(false)
        )
    );
    assert!(crate::search::sources_for(&s, "everywhere").contains(&"soundcloud".to_string()));
    // Search: sets first, then tracks; a pasted link resolves
    let (st, r) = call(
        &app,
        "GET",
        "/v1/sources/soundcloud/search?q=night&limit=10",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    let items = r["items"].as_array().unwrap();
    assert_eq!(
        (items[0]["id"].as_str(), items[0]["kind"].as_str()),
        (Some("playlist:9"), Some("item"))
    );
    assert_eq!(items[1]["uri"], "soundcloud:track:1");
    assert_eq!(items[1]["url"], "https://soundcloud.com/someone/night-drive");
    let (_, r) = call(
        &app,
        "GET",
        "/v1/sources/soundcloud/search?q=https%3A%2F%2Fsoundcloud.com%2Fsomeone%2Fsunrise",
        None,
    )
    .await;
    assert_eq!(r["items"][0]["uri"], "soundcloud:track:2");
    // A set page: the stub track is fetched; the encrypted track says so on its own page
    let (_, it) = call(&app, "GET", "/v1/sources/soundcloud/items/playlist:9", None).await;
    assert_eq!(it["tracks"].as_array().unwrap().len(), 2, "{it}");
    assert_eq!(it["tracks"][1]["url"], "soundcloud:track:2");
    assert_eq!(it["tracks"][1]["format"], "MP3 (SoundCloud, HLS)");
    let (_, it) = call(&app, "GET", "/v1/sources/soundcloud/items/track:3", None).await;
    assert!(
        it["tracks"][0]["format"]
            .as_str()
            .unwrap()
            .contains("not playable"),
        "{it}"
    );
    // Streams: progressive for browsers and zones, HLS stitched for both; encrypted refused
    let req = Request::builder()
        .uri("/v1/sources/soundcloud/stream/1")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()["content-type"], "audio/mpeg");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.len(), flac.len());
    let req = Request::builder()
        .uri("/v1/sources/soundcloud/stream/2")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.to_vec(), flac, "HLS segments come back as the one file");
    for id in ["1", "2"] {
        let src = s.sources.clone();
        let it = pm_zones::QueueItem::new(format!("soundcloud:track:{id}"));
        let r = tokio::task::spawn_blocking(move || src.open(&it).map(|_| ()).map_err(|e| e.to_string()))
            .await
            .unwrap();
        assert!(r.is_ok(), "track {id}: {r:?}");
    }
    let src = s.sources.clone();
    let it = pm_zones::QueueItem::new("soundcloud:track:3");
    let r = tokio::task::spawn_blocking(move || src.open(&it).map(|_| ()).map_err(|e| e.to_string()))
        .await
        .unwrap();
    assert!(
        r.unwrap_err().contains("Go+"),
        "encrypted tracks are refused, never decrypted"
    );
    let (_, u) = call(&app, "GET", "/v1/sources/soundcloud/unofficial", None).await;
    assert_eq!(u["health"]["ok"], true, "{u}");
    // The owner's token: checked with `me`; a wrong one is dropped
    let (st, r) = call(
        &app,
        "PUT",
        "/v1/sources/soundcloud/unofficial",
        Some(serde_json::json!({"token": "wrong-token-123"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{r}");
    let (_, u) = call(&app, "GET", "/v1/sources/soundcloud/unofficial", None).await;
    assert_eq!(
        (u["signed_in"].clone(), u["enabled"].clone()),
        (serde_json::json!(false), serde_json::json!(true)),
        "a bad token doesn't switch the mode off: {u}"
    );
    let (st, u) = call(
        &app,
        "PUT",
        "/v1/sources/soundcloud/unofficial",
        Some(serde_json::json!({"token": "tok-0123456789"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{u}");
    assert_eq!(
        (u["signed_in"].clone(), u["user"].clone()),
        (serde_json::json!(true), serde_json::json!("Someone"))
    );
    assert!(
        !u.to_string().contains("tok-0123456789"),
        "the token never comes back: {u}"
    );
    let (_, srcs) = call(&app, "GET", "/v1/sources", None).await;
    let sc = srcs
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "soundcloud")
        .unwrap()
        .clone();
    assert_eq!(sc["status"], "signed in");
    // Likes and playlists, and their imports
    let (st, l) = call(&app, "GET", "/v1/sources/soundcloud/likes", None).await;
    assert_eq!(st, StatusCode::OK, "{l}");
    assert_eq!(
        (
            l["tracks"].as_array().unwrap().len(),
            l["playlists"].as_array().unwrap().len()
        ),
        (1, 1)
    );
    let (_, r) = call(&app, "POST", "/v1/sources/soundcloud/likes/import", None).await;
    assert_eq!(
        (r["imported"].clone(), r["skipped"].clone()),
        (serde_json::json!(2), serde_json::json!(0)),
        "{r}"
    );
    let (_, favs) = call(&app, "GET", "/v1/bookmarks", None).await;
    assert!(
        favs.to_string().contains("soundcloud.com/someone/night-drive"),
        "{favs}"
    );
    let (_, pls) = call(&app, "GET", "/v1/sources/soundcloud/playlists", None).await;
    assert_eq!(
        (pls[0]["id"].clone(), pls[0]["items"].clone()),
        (serde_json::json!("9"), serde_json::json!(2))
    );
    let (st, r) = call(
        &app,
        "POST",
        "/v1/sources/soundcloud/playlists/9/import?keep_source=true",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["imported"], 2);
    assert_eq!(r["playlist"]["summary"]["name"], "Late Set");
    // SoundCloud refuses the client: the mode switches itself off and says why
    refuse.store(true, std::sync::atomic::Ordering::SeqCst);
    let (st, r) = call(&app, "GET", "/v1/sources/soundcloud/search?q=night", None).await;
    assert_eq!(st, StatusCode::BAD_GATEWAY, "{r}");
    let (_, u) = call(&app, "GET", "/v1/sources/soundcloud/unofficial", None).await;
    assert_eq!(u["enabled"], false);
    assert!(
        u["health"]["detail"]
            .as_str()
            .unwrap()
            .contains("refused the client ID"),
        "{u}"
    );
    let (_, srcs) = call(&app, "GET", "/v1/sources", None).await;
    let sc = srcs
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "soundcloud")
        .unwrap()
        .clone();
    assert!(sc["status"].as_str().unwrap().starts_with("error:"), "{sc}");
    assert!(!crate::search::sources_for(&s, "everywhere").contains(&"soundcloud".to_string()));
    // Try again (on) with SoundCloud answering again
    refuse.store(false, std::sync::atomic::Ordering::SeqCst);
    let (_, u) = call(
        &app,
        "PUT",
        "/v1/sources/soundcloud/unofficial",
        Some(serde_json::json!({"enabled": true})),
    )
    .await;
    assert!(u["health"].is_null() && u["enabled"] == true, "{u}");
    // Forget the client: back to not configured
    let (_, u) = call(
        &app,
        "PUT",
        "/v1/sources/soundcloud/unofficial",
        Some(serde_json::json!({"client_id": ""})),
    )
    .await;
    assert!(u["client_from"].is_null() && u["signed_in"] == false, "{u}");
    assert!(backup::SECRET_SETTINGS
        .iter()
        .any(|(k, _)| *k == soundcloud::CLIENT_KEY));
}

/// Spotify Soloist: the node downloads Spotify's archive (a stand-in here) and unpacks it, runs
/// Soloist with the owner's key and the device name, reads what plays over `soloist ctl`, says
/// in plain words when the key is refused or the build has expired, and never returns the key.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn spotify_soloist_runs_as_a_supervised_program() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    // A stand-in soloist: --version; ctl (now --json, pause, play) through the data dir's
    // ws.port; the daemon itself takes only the key "good-key", exits 10 when `expired` sits in
    // its data dir, and otherwise stays up.
    let fake = r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo "soloist 1.2.3 (build 2026-10-01)"; echo "x86_64-linux"; exit 0; fi
if [ "$1" = "ctl" ]; then
  shift; dir=""; cmd=""
  while [ $# -gt 0 ]; do case "$1" in -D) dir="$2"; shift 2;; --json) shift;; *) cmd="$1"; shift;; esac; done
  [ -f "$dir/ws.port" ] || exit 2
  case "$cmd" in
    now) [ -f "$dir/paired" ] || { echo "not logged in" >&2; exit 3; }
         echo '{"type":"playback_state","status":"playing","volume":50,"is_active":true,"position":{"position_ms":1000,"timestamp_ms":1,"speed":1.0},"item":{"uri":"spotify:track:69kOkLUCkxIZYexIgSG8rq","entity_type":"track","decorations":{"identity":{"name":"Get Lucky"},"creators":[{"decorations":{"identity":{"name":"Daft Punk"}}}],"parent":{"entity":{"decorations":{"identity":{"name":"Random Access Memories"}}}},"playback":{"duration_ms":369000}}}}'; exit 0;;
    pause|play|next|prev) echo "$cmd" >> "$dir/commands"; exit 0;;
    *) exit 1;;
  esac
fi
dir=""; key=""
while [ $# -gt 0 ]; do case "$1" in -D|--data-dir) dir="$2"; shift 2;; -k|--api-key) key="$2"; shift 2;; *) shift;; esac; done
[ "$key" = "good-key-0123456789abcdef" ] || { echo "error: Spotify rejected the API key $key" >&2; exit 1; }
[ -f "$dir/expired" ] && { echo "Spotify Soloist build expired." >&2; exit 10; }
echo 127.0.0.1 > "$dir/ws.addr"; echo 4242 > "$dir/ws.port"; echo $$ > "$dir/soloist.pid"
exec sleep 300
"#;
    let work = tempfile::tempdir().unwrap();
    std::fs::create_dir(work.path().join("soloist_release")).unwrap();
    std::fs::write(work.path().join("soloist_release/soloist"), fake).unwrap();
    let tar = std::process::Command::new("tar")
        .arg("-czf")
        .arg(work.path().join("soloist.tar.gz"))
        .arg("-C")
        .arg(work.path())
        .arg("soloist_release")
        .status()
        .unwrap();
    assert!(tar.success());
    let archive = std::fs::read(work.path().join("soloist.tar.gz")).unwrap();
    let asset = if cfg!(target_arch = "aarch64") {
        "soloist_release_arm64.tar.gz"
    } else {
        "soloist_release_x86_64.tar.gz"
    };
    let mock = Router::new().route(
        &format!("/{asset}"),
        get(move || {
            let a = archive.clone();
            async move { a }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    soloist::set_downloads_base(&base);
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let until = |pred: Box<dyn Fn(&serde_json::Value) -> bool>| {
        let app = app.clone();
        async move {
            let mut last = serde_json::Value::Null;
            for _ in 0..60 {
                let (_, u) = call(&app, "GET", "/v1/sources/spotify/soloist", None).await;
                if pred(&u) {
                    return u;
                }
                last = u;
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            panic!("Soloist status never got there: {last}");
        }
    };
    let (_, u) = call(&app, "GET", "/v1/sources/spotify/soloist", None).await;
    assert_eq!(
        (
            u["supported"].clone(),
            u["enabled"].clone(),
            u["running"].clone(),
            u["soloist"].clone(),
            u["can_download"].clone()
        ),
        (
            serde_json::json!(true),
            serde_json::json!(false),
            serde_json::json!(false),
            serde_json::Value::Null,
            serde_json::json!(true)
        ),
        "{u}"
    );
    assert!(u["note"].as_str().unwrap().contains("not on the node's zones"));
    // Download: the archive unpacks and the executable answers --version
    let (st, u) = call(&app, "POST", "/v1/sources/spotify/soloist/binary", None).await;
    assert_eq!(st, StatusCode::OK, "{u}");
    assert_eq!(u["soloist"]["from"], "downloaded");
    assert_eq!(u["soloist"]["version"], "soloist 1.2.3 (build 2026-10-01)");
    // On without a key: it waits and says for what
    let (_, u) = call(
        &app,
        "PUT",
        "/v1/sources/spotify/soloist",
        Some(serde_json::json!({"enabled": true})),
    )
    .await;
    assert_eq!(u["enabled"], true);
    assert_eq!(u["running"], false);
    assert!(u["waiting_for"].as_str().unwrap().contains("API key"), "{u}");
    // A bad-looking key is refused before anything runs
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/sources/spotify/soloist",
        Some(serde_json::json!({"api_key": "short"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    // A key Spotify rejects: Soloist exits, the health says so, and the key is nowhere in the answer
    let (st, u) = call(
        &app,
        "PUT",
        "/v1/sources/spotify/soloist",
        Some(serde_json::json!({"api_key": "rejected-key-0123456789", "device_name": "Test speaker"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{u}");
    assert_eq!(u["key_set"], true);
    assert_eq!(u["device_name"], "Test speaker");
    let u = until(Box::new(|u| u["health"]["ok"] == false)).await;
    assert!(
        u["health"]["detail"]
            .as_str()
            .unwrap()
            .contains("rejected the API key"),
        "{u}"
    );
    assert!(
        !u.to_string().contains("rejected-key-0123456789"),
        "the key never comes back: {u}"
    );
    // The right key: it runs, and nobody has picked the device yet
    let (st, u) = call(
        &app,
        "PUT",
        "/v1/sources/spotify/soloist",
        Some(serde_json::json!({"api_key": "good-key-0123456789abcdef"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{u}");
    let u = until(Box::new(|u| u["running"] == true && u["paired"] == false)).await;
    assert!(u["pid"].as_u64().is_some(), "{u}");
    let (_, srcs) = call(&app, "GET", "/v1/sources", None).await;
    let sp = srcs
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "spotify")
        .unwrap()
        .clone();
    assert!(
        sp["status"]
            .as_str()
            .unwrap()
            .contains("Soloist plays on this machine"),
        "{sp}"
    );
    // Picked in a Spotify app: what plays comes through ctl, and the page can pause it
    let sdir = s.data_dir().join("soloist");
    std::fs::write(sdir.join("paired"), "").unwrap();
    let u = until(Box::new(|u| u["paired"] == true)).await;
    assert_eq!(u["now"]["title"], "Get Lucky", "{u}");
    assert_eq!(u["now"]["artists"][0], "Daft Punk");
    assert_eq!(u["now"]["album"], "Random Access Memories");
    assert_eq!(u["now"]["status"], "playing");
    let (st, _) = call(
        &app,
        "POST",
        "/v1/sources/spotify/soloist/control",
        Some(serde_json::json!({"command": "pause"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        std::fs::read_to_string(sdir.join("commands")).unwrap().trim(),
        "pause"
    );
    let (st, _) = call(
        &app,
        "POST",
        "/v1/sources/spotify/soloist/control",
        Some(serde_json::json!({"command": "eject"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    // Off: it stops
    let (_, u) = call(
        &app,
        "PUT",
        "/v1/sources/spotify/soloist",
        Some(serde_json::json!({"enabled": false})),
    )
    .await;
    assert_eq!(u["enabled"], false);
    let u = until(Box::new(|u| u["running"] == false)).await;
    assert!(u["waiting_for"].is_null(), "{u}");
    let (st, _) = call(
        &app,
        "POST",
        "/v1/sources/spotify/soloist/control",
        Some(serde_json::json!({"command": "play"})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    // An expired build: exit 10, said in plain words, no restart loop
    std::fs::write(sdir.join("expired"), "").unwrap();
    let (_, _) = call(
        &app,
        "PUT",
        "/v1/sources/spotify/soloist",
        Some(serde_json::json!({"enabled": true})),
    )
    .await;
    let u = until(Box::new(|u| u["health"]["ok"] == false)).await;
    assert!(u["health"]["detail"].as_str().unwrap().contains("expired"), "{u}");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let (_, u) = call(&app, "GET", "/v1/sources/spotify/soloist", None).await;
    assert_eq!(u["running"], false, "{u}");
    // Remove the download
    let (_, u) = call(&app, "DELETE", "/v1/sources/spotify/soloist/binary", None).await;
    assert!(u["soloist"].is_null(), "{u}");
    assert!(u["waiting_for"].as_str().unwrap().contains("download"), "{u}");
    // The key is a secret in backups
    assert!(backup::SECRET_SETTINGS
        .iter()
        .any(|(k, _)| *k == soloist::KEY_SETTING));
}

/// Bandcamp (unofficial): search, an album page read from its data-tralbum, its stream on a zone
/// and in the browser, then with the owner's sign-in the collection and a purchase downloaded
/// (a zip) into `<data_dir>/bandcamp`, which becomes a library folder and is scanned.
#[tokio::test]
async fn bandcamp_streams_and_downloads_purchases_into_the_library() {
    let _base_guard = bandcamp::API_TEST_LOCK.lock().await;
    use pm_zones::SourceFactory;
    use std::io::Write as _;
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let mut zipped = Vec::new();
    {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut zipped));
        z.start_file(
            "Band One - RAM - 01 One.flac",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        z.write_all(&flac).unwrap();
        z.start_file("../escape.flac", zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(b"no").unwrap();
        z.finish().unwrap();
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let esc = |v: serde_json::Value| v.to_string().replace('&', "&amp;").replace('"', "&quot;");
    let tralbum = esc(
        serde_json::json!({"artist": "Band One", "art_id": 42, "album_release_date": "16 Jan 2016 00:00:00 GMT",
        "current": {"title": "RAM", "about": "An album"},
        "trackinfo": [{"title": "One", "track_num": 1, "duration": 60.0, "file": {"mp3-128": format!("{base}/stream/1.mp3?token=t")}},
                      {"title": "Two", "track_num": 2, "duration": 60.0, "file": null}]}),
    );
    let album_page = format!("<html><body><script data-tralbum=\"{tralbum}\"></script></body></html>");
    let blob = esc(
        serde_json::json!({"digital_items": [{"title": "RAM", "artist": "Band One", "downloads": {
        "mp3-320": {"url": format!("{base}/download/album/1?fmt=mp3")}, "flac": {"url": format!("{base}/download/album/1?fmt=flac")}}}]}),
    );
    let dl_page = format!("<html><div id=\"pagedata\" data-blob=\"{blob}\"></div></html>");
    let cookie_ok = |h: &axum::http::HeaderMap| {
        h.get("cookie").and_then(|v| v.to_str().ok()) == Some("identity=7%09abc%3D%3D")
    };
    let b2 = base.clone();
    let mock = Router::new()
        .route("/api/bcsearch_public_api/1/autocomplete_elastic", post(move |Json(b): Json<serde_json::Value>| async move {
            assert_eq!(b["search_text"], "ram");
            Json(serde_json::json!({"auto": {"results": [
                {"type": "b", "name": "Band One", "item_url_path": "https://band1.bandcamp.com"},
                {"type": "a", "name": "RAM", "band_name": "Band One", "item_url_path": "https://band1.bandcamp.com/album/ram", "img": "https://f4.bcbits.com/img/9_3.jpg"},
                {"type": "t", "name": "Elsewhere", "band_name": "X", "item_url_path": "https://custom.example/track/x"}]}}))
        }))
        .route("/_band/band1/album/ram", get(move || { let p = album_page.clone(); async move { axum::response::Html(p) } }))
        .route("/_band/band1/album/changed", get(|| async { axum::response::Html("<html>new layout</html>") }))
        .route("/stream/1.mp3", get({ let f = flac.clone(); move || { let f = f.clone(); async move { f } } }))
        .route("/api/fan/2/collection_summary", get(move |h: axum::http::HeaderMap| async move {
            if !cookie_ok(&h) { return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": true}))); }
            (StatusCode::OK, Json(serde_json::json!({"fan_id": 77, "collection_summary": {"fan_id": 77, "username": "listener"}})))
        }))
        .route("/api/fancollection/1/collection_items", post(move |h: axum::http::HeaderMap, Json(b): Json<serde_json::Value>| {
            let base = b2.clone();
            async move {
                assert!(cookie_ok(&h));
                assert_eq!(b["fan_id"], 77);
                let first = b["older_than_token"].as_str().unwrap().ends_with("::a::") && !b["older_than_token"].as_str().unwrap().starts_with("1::");
                if first {
                    Json(serde_json::json!({"items": [{"item_title": "RAM", "band_name": "Band One", "item_url": "https://band1.bandcamp.com/album/ram", "item_type": "album", "sale_item_type": "p", "sale_item_id": 5, "item_art_id": 42}],
                        "redownload_urls": {"p5": format!("{base}/dl-page?payment_id=5")}, "more_available": true, "last_token": "1::a::"}))
                } else {
                    Json(serde_json::json!({"items": [{"item_title": "Gift", "band_name": "Other", "item_url": "https://other.bandcamp.com/track/gift", "item_type": "track", "sale_item_type": "r", "sale_item_id": 6}],
                        "redownload_urls": {}, "more_available": false, "last_token": "0::a::"}))
                }
            }
        }))
        .route("/dl-page", get(move |h: axum::http::HeaderMap| { let p = dl_page.clone(); async move { assert!(cookie_ok(&h)); axum::response::Html(p) } }))
        .route("/statdownload/album/1", get({ let b = base.clone(); move |q: axum::extract::RawQuery| { let b = b.clone(); async move {
            let q = q.0.unwrap_or_default();
            assert!(q.contains("fmt=flac") && q.contains(".vrs=1") && q.contains(".rand="), "{q}");
            format!("if ( window.Downloads ) {{ Downloads.statResult ( {{\"result\":\"ok\",\"download_url\":\"{b}/files/ram.zip\"}} ) }};")
        } } }))
        .route("/files/ram.zip", get(move || { let z = zipped.clone(); async move { z } }));
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    bandcamp::set_site_base(&base);
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    // Off until switched on
    let (st, _) = call(&app, "GET", "/v1/sources/bandcamp/search?q=ram", None).await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED);
    let (_, u) = call(
        &app,
        "PUT",
        "/v1/sources/bandcamp/unofficial",
        Some(serde_json::json!({"enabled": true})),
    )
    .await;
    assert_eq!(
        (u["enabled"].clone(), u["signed_in"].clone(), u["format"].clone()),
        (
            serde_json::json!(true),
            serde_json::json!(false),
            serde_json::json!("flac")
        )
    );
    // Search: albums and tracks on bandcamp.com only
    let (st, r) = call(&app, "GET", "/v1/sources/bandcamp/search?q=ram", None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    let items = r["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{r}");
    assert_eq!(
        (items[0]["id"].as_str(), items[0]["image"].as_str()),
        (
            Some("album:band1.ram"),
            Some("https://f4.bcbits.com/img/9_10.jpg")
        )
    );
    // The album page: the streamable track only
    let (st, it) = call(&app, "GET", "/v1/sources/bandcamp/items/album:band1.ram", None).await;
    assert_eq!(st, StatusCode::OK, "{it}");
    assert_eq!(
        (it["title"].as_str(), it["year"].as_i64()),
        (Some("RAM"), Some(2016))
    );
    assert_eq!(it["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(it["tracks"][0]["url"], "bandcamp:track:band1.a.ram.1");
    assert_eq!(it["url"], "https://band1.bandcamp.com/album/ram");
    // Its stream: in the browser (relayed) and on a zone
    let req = Request::builder()
        .uri("/v1/sources/bandcamp/stream/band1.a.ram.1")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .len(),
        flac.len()
    );
    let src = s.sources.clone();
    let it = pm_zones::QueueItem::new("bandcamp:track:band1.a.ram.1");
    let r = tokio::task::spawn_blocking(move || src.open(&it).map(|_| ()).map_err(|e| e.to_string()))
        .await
        .unwrap();
    assert!(r.is_ok(), "{r:?}");
    let req = Request::builder()
        .uri("/v1/sources/bandcamp/stream/band1.a.ram.2")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::NOT_FOUND,
        "a track the artist doesn't stream"
    );
    // The sign-in: a wrong cookie is refused; the right one names the fan, and never comes back
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/sources/bandcamp/unofficial",
        Some(serde_json::json!({"identity": "wrong"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, u) = call(
        &app,
        "PUT",
        "/v1/sources/bandcamp/unofficial",
        Some(serde_json::json!({"identity": "identity=7%09abc%3D%3D"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{u}");
    assert_eq!(
        (u["signed_in"].clone(), u["user"].clone()),
        (serde_json::json!(true), serde_json::json!("listener"))
    );
    assert!(!u.to_string().contains("7%09abc"), "{u}");
    // The collection, across pages
    let (st, c) = call(&app, "GET", "/v1/sources/bandcamp/collection", None).await;
    assert_eq!(st, StatusCode::OK, "{c}");
    let c = c.as_array().unwrap();
    assert_eq!(c.len(), 2);
    assert_eq!(
        (
            c[0]["id"].as_str(),
            c[0]["downloadable"].as_bool(),
            c[0]["item"].as_str()
        ),
        (Some("p5"), Some(true), Some("album:band1.ram"))
    );
    assert_eq!(c[1]["downloadable"], false);
    // Download the purchase: FLAC, unpacked (nothing outside its folder), and scanned into the library
    let (st, j) = call(&app, "POST", "/v1/sources/bandcamp/collection/p5/download", None).await;
    assert_eq!(st, StatusCode::OK, "{j}");
    let mut last = serde_json::Value::Null;
    for _ in 0..100 {
        let (_, d) = call(&app, "GET", "/v1/sources/bandcamp/downloads", None).await;
        // This purchase's job, by its id (the list is process-wide)
        last = d
            .as_array()
            .unwrap()
            .iter()
            .find(|j| j["id"] == "p5")
            .cloned()
            .unwrap_or_default();
        if last["state"] == "done" || last["state"] == "failed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(last["state"], "done", "{last}");
    assert_eq!(last["format"], "flac");
    let folder = std::path::PathBuf::from(last["folder"].as_str().unwrap());
    assert!(
        folder.join("Band One - RAM - 01 One.flac").is_file(),
        "{}",
        folder.display()
    );
    assert!(
        !folder.parent().unwrap().join("escape.flac").exists(),
        "zip entries stay inside the folder"
    );
    let (mut folders, mut tracks) = (serde_json::Value::Null, serde_json::Value::Null);
    for _ in 0..100 {
        folders = call(&app, "GET", "/v1/library/folders", None).await.1;
        tracks = call(&app, "GET", "/v1/library/tracks", None).await.1;
        if folders.to_string().contains("bandcamp") && tracks.to_string().contains("01 One") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        folders.to_string().contains("bandcamp"),
        "a library folder: {folders}"
    );
    assert!(
        tracks.to_string().contains("01 One"),
        "the purchase is in the library: {tracks}"
    );
    let (_, c) = call(&app, "GET", "/v1/sources/bandcamp/collection", None).await;
    assert_eq!(c[0]["downloaded"], true);
    let (st, _) = call(&app, "POST", "/v1/sources/bandcamp/collection/r6/download", None).await;
    assert_eq!(st, StatusCode::CONFLICT, "no download offered");
    // A page that no longer carries the data switches the mode off
    let (st, _) = call(
        &app,
        "GET",
        "/v1/sources/bandcamp/items/album:band1.changed",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_GATEWAY);
    let (_, u) = call(&app, "GET", "/v1/sources/bandcamp/unofficial", None).await;
    assert_eq!(u["enabled"], false);
    assert!(
        u["health"]["detail"].as_str().unwrap().contains("pages changed"),
        "{u}"
    );
    assert!(backup::SECRET_SETTINGS
        .iter()
        .any(|(k, _)| *k == bandcamp::COOKIE_KEY));
}

/// librespot (unofficial) as an external program: a stand-in librespot pipes PCM and reports its
/// events through the node's `--onevent` script; the chosen zone starts the live item, shows the
/// track's title, pauses and stops with it; off stops librespot.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn librespot_pipes_into_a_zone_and_the_zone_follows_its_events() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("fake-librespot");
    // --version; otherwise: find --onevent and the pipe backend, report a session and a track,
    // play (2 s of a 440 Hz-ish square wave on stdout), pause, and wait
    std::fs::write(&bin, r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo "librespot 0.8.0 test"; exit 0; fi
ev=""; pipe=0; bitrate=""
while [ $# -gt 0 ]; do case "$1" in --onevent) ev="$2"; shift 2;; --backend) [ "$2" = pipe ] && pipe=1; shift 2;; --bitrate) bitrate="$2"; shift 2;; *) shift;; esac; done
[ -n "$ev" ] || { echo "no onevent" >&2; exit 3; }
[ "$bitrate" = 160 ] || { echo "wrong bitrate: $bitrate" >&2; exit 3; }
PLAYER_EVENT=session_connected USER_NAME=listener "$ev"
PLAYER_EVENT=track_changed NAME="Get Lucky" ARTISTS="Daft Punk
Pharrell Williams" ALBUM=RAM DURATION_MS=369000 URI=spotify:track:69kOkLUCkxIZYexIgSG8rq "$ev"
PLAYER_EVENT=playing POSITION_MS=0 "$ev"
if [ "$pipe" = 1 ]; then head -c 352800 /dev/zero | tr '\000' '\100'; fi
sleep 2
PLAYER_EVENT=paused POSITION_MS=2000 "$ev"
exec sleep 300
"#).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    librespot::set_config_path(bin.to_str().unwrap());
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let (_, u) = call(&app, "GET", "/v1/sources/spotify/librespot", None).await;
    assert_eq!(
        (u["enabled"].clone(), u["output"].clone(), u["running"].clone()),
        (
            serde_json::json!(false),
            serde_json::json!("local"),
            serde_json::json!(false)
        )
    );
    assert_eq!(u["librespot"]["version"], "librespot 0.8.0 test");
    assert!(u["warning"].as_str().unwrap().contains("not approved by Spotify"));
    // The selected quality must reach both the process and the player's stream metadata.
    let (st, q) = call(
        &app,
        "PUT",
        "/v1/sources/spotify/quality",
        Some(serde_json::json!({"id": "160"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{q}");
    // Zone output without a zone is refused; with one it runs
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/sources/spotify/librespot",
        Some(serde_json::json!({"zone_id": "nope"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, u) = call(&app, "PUT", "/v1/sources/spotify/librespot", Some(serde_json::json!({"output": "zone", "zone_id": zid, "device_name": "Test speaker", "enabled": true}))).await;
    assert_eq!(st, StatusCode::OK, "{u}");
    assert_eq!(
        (u["running"].clone(), u["device_name"].clone()),
        (serde_json::json!(true), serde_json::json!("Test speaker")),
        "{u}"
    );
    // The zone starts the live item and shows the track
    let mut z = serde_json::Value::Null;
    for _ in 0..60 {
        z = call(&app, "GET", &format!("/v1/zones/{zid}"), None).await.1;
        if z["now_playing"]["item"]["uri"] == librespot::LIVE_URI && z["transport"] == "playing" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(z["now_playing"]["item"]["uri"], librespot::LIVE_URI, "{z}");
    assert_eq!(z["transport"], "playing", "{z}");
    assert_eq!(z["now_playing"]["format"]["bitrate_kbps"], 160, "{z}");
    let (_, u) = call(&app, "GET", "/v1/sources/spotify/librespot", None).await;
    assert_eq!(u["user"], "listener");
    assert_eq!(u["now"]["title"], "Get Lucky");
    assert_eq!(
        u["now"]["artists"],
        serde_json::json!(["Daft Punk", "Pharrell Williams"])
    );
    // librespot pauses: so does the zone
    for _ in 0..60 {
        z = call(&app, "GET", &format!("/v1/zones/{zid}"), None).await.1;
        if z["transport"] == "paused" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(z["transport"], "paused", "{z}");
    // Off: librespot stops, and the zone with it
    let (_, u) = call(
        &app,
        "PUT",
        "/v1/sources/spotify/librespot",
        Some(serde_json::json!({"enabled": false})),
    )
    .await;
    assert_eq!(u["running"], false, "{u}");
    let z = call(&app, "GET", &format!("/v1/zones/{zid}"), None).await.1;
    assert_eq!(z["transport"], "stopped", "{z}");
    // The live item refuses to open while librespot isn't piping
    assert!(librespot::open_live().is_err());
}

/// YouTube on the zones (unofficial): yt-dlp downloads from the (stood-in) official release
/// only when its checksum matches, and a zone then plays a video through it.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn youtube_plays_on_zones_through_a_checked_ytdlp() {
    use pm_zones::SourceFactory;
    use sha2::Digest;
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    // A stand-in yt-dlp: its version, and the audio URL for any video
    let fake = format!("#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 2026.10.01; exit 0; fi\nfor a; do last=\"$a\"; done\ncase \"$last\" in *v=broken*) echo 'ERROR: [youtube] broken: Video unavailable' >&2; exit 1;; esac\necho {base}/file.flac\n");
    let asset = if cfg!(target_arch = "aarch64") {
        "yt-dlp_linux_aarch64"
    } else {
        "yt-dlp_linux"
    };
    let sum = Arc::new(std::sync::Mutex::new(format!(
        "{:x}",
        sha2::Sha256::digest(fake.as_bytes())
    )));
    let sum2 = sum.clone();
    let mock = Router::new()
        .route(
            "/SHA2-256SUMS",
            get(move || {
                let s = sum2.lock().unwrap().clone();
                async move { format!("0000  yt-dlp.tar.gz\n{s}  {asset}\n") }
            }),
        )
        .route(
            &format!("/{asset}"),
            get({
                let f = fake.clone();
                move || {
                    let f = f.clone();
                    async move { f }
                }
            }),
        )
        .route(
            "/file.flac",
            get({
                let f = flac.clone();
                move || {
                    let f = f.clone();
                    async move { f }
                }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    ytdlp::set_releases_base(&base);
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (_, u) = call(&app, "GET", "/v1/sources/youtube/unofficial", None).await;
    assert_eq!(
        (u["enabled"].clone(), u["ytdlp"].clone()),
        (serde_json::json!(false), serde_json::Value::Null)
    );
    // A download that doesn't match the release's checksum is not installed
    *sum.lock().unwrap() = "ff".repeat(32);
    let (st, _) = call(&app, "POST", "/v1/sources/youtube/unofficial/ytdlp", None).await;
    assert_eq!(st, StatusCode::BAD_GATEWAY);
    assert!(!dir.path().join("data/bin/yt-dlp").exists() && !dir.path().join("bin/yt-dlp").exists());
    *sum.lock().unwrap() = format!("{:x}", sha2::Sha256::digest(fake.as_bytes()));
    let (st, u) = call(&app, "POST", "/v1/sources/youtube/unofficial/ytdlp", None).await;
    assert_eq!(st, StatusCode::OK, "{u}");
    assert_eq!(u["ytdlp"]["from"], "downloaded");
    assert_eq!(u["ytdlp"]["version"], "2026.10.01");
    // Off: zones still refuse; on: a zone plays the video through yt-dlp
    let item = pm_zones::QueueItem::new("youtube:video:dQw4w9WgXcQ");
    let src = s.sources.clone();
    let it = item.clone();
    assert!(tokio::task::spawn_blocking(move || src.open(&it).is_err())
        .await
        .unwrap());
    let (_, u) = call(
        &app,
        "PUT",
        "/v1/sources/youtube/unofficial",
        Some(serde_json::json!({"enabled": true})),
    )
    .await;
    assert_eq!(u["enabled"], true);
    let (_, srcs) = call(&app, "GET", "/v1/sources", None).await;
    let yt = srcs
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "youtube")
        .unwrap()
        .clone();
    assert_eq!(yt["status"], "browser + zones");
    let src = s.sources.clone();
    let it = item.clone();
    let r = tokio::task::spawn_blocking(move || src.open(&it).map(|_| ()).map_err(|e| e.to_string()))
        .await
        .unwrap();
    assert!(r.is_ok(), "{r:?}");
    let (_, u) = call(&app, "GET", "/v1/sources/youtube/unofficial", None).await;
    assert_eq!(u["health"]["ok"], true, "{u}");
    // yt-dlp's own error reaches the page
    let src = s.sources.clone();
    let r = tokio::task::spawn_blocking(move || {
        src.open(&pm_zones::QueueItem::new("youtube:video:broken_vid1"))
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
    .await
    .unwrap();
    assert!(r.unwrap_err().contains("Video unavailable"));
    let (_, u) = call(&app, "DELETE", "/v1/sources/youtube/unofficial/ytdlp", None).await;
    assert!(u["ytdlp"].is_null());
}

/// Search hits open the unified item page: an Internet Archive item lists its recordings
/// (public files only) and plays them, an Audius song's row carries its stream, and an item
/// the archive restricts says so (TASK-archive-restricted, revisions 1 and 2).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn archive_and_audius_pages_from_search_play_and_a_restricted_item_says_so() {
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let (flac_a, flac_b) = (flac.clone(), flac);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen_q = Arc::new(std::sync::Mutex::new(String::new()));
    let sq = seen_q.clone();
    let track = serde_json::json!({"id": "W0NJ0", "title": "Ambient Piano Interlude", "user": {"name": "Techwipe"}, "duration": 5,
        "permalink": "/techwipe/ambient-piano-interlude-353852", "artwork": {"480x480": "https://img/480"}, "play_count": 3});
    let (t1, t2) = (track.clone(), track);
    let mock = Router::new()
        .route(
            "/advancedsearch.php",
            get(move |axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| {
                let sq = sq.clone();
                async move {
                    *sq.lock().unwrap() = q.get("q").cloned().unwrap_or_default();
                    Json(serde_json::json!({"response": {"docs": [
                        {"identifier": "fixture-set", "title": "Fixture Set", "creator": "Tester", "year": "2020", "downloads": 7}
                    ]}}))
                }
            }),
        )
        .route(
            "/metadata/{id}",
            get(|axum::extract::Path(id): axum::extract::Path<String>| async move {
                Json(if id == "locked-set" {
                    // A restricted item: the flag, and every audio file private
                    serde_json::json!({
                        "metadata": {"title": "Locked Set", "access-restricted-item": "true"},
                        "files": [{"name": "x.mp3", "format": "VBR MP3", "source": "original", "private": "true"}]
                    })
                } else {
                    serde_json::json!({
                        "metadata": {"title": "Fixture Set", "creator": "Tester", "year": "2020"},
                        "files": [
                            {"name": "one.flac", "format": "Flac", "source": "original", "title": "One", "track": "1", "length": "5.15"},
                            {"name": "two.flac", "format": "Flac", "source": "original", "title": "Two", "track": "2", "private": "true"}
                        ]
                    })
                })
            }),
        )
        .route(
            "/download/{id}/{file}",
            get(move || {
                let flac = flac_a.clone();
                async move { ([(header::CONTENT_TYPE, "audio/flac")], flac) }
            }),
        )
        .route(
            "/v1/resolve",
            get(move |axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| {
                let t = t1.clone();
                async move {
                    assert_eq!(q.get("url").map(String::as_str), Some("https://audius.co/techwipe/ambient-piano-interlude-353852"));
                    Json(serde_json::json!({"data": t}))
                }
            }),
        )
        .route(
            "/v1/tracks/{id}",
            get(move || {
                let t = t2.clone();
                async move { Json(serde_json::json!({"data": t})) }
            }),
        )
        .route(
            "/v1/tracks/{id}/stream",
            get(move || {
                let flac = flac_b.clone();
                async move { ([(header::CONTENT_TYPE, "audio/flac")], flac) }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let base = format!("http://127.0.0.1:{port}");
    *s.archive_base.write().unwrap() = base.clone();
    *s.audius_base.write().unwrap() = base.clone();
    let app = router(s.clone());
    // The search never asks for restricted items
    let (st, r) = call(&app, "GET", "/v1/sources/archive/search?q=fixture", None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert!(
        seen_q
            .lock()
            .unwrap()
            .ends_with("AND mediatype:(audio) AND NOT access-restricted-item:(true)"),
        "{}",
        seen_q.lock().unwrap()
    );
    // An archive item from a search hit: the page lists its public recordings and plays them
    let id = crate::discover::page_id("https://archive.org/details/fixture-set");
    let (st, pg) = call(&app, "GET", &format!("/v1/item/album/{id}"), None).await;
    assert_eq!(st, StatusCode::OK, "{pg}");
    assert_eq!(pg["title"], "Fixture Set", "{pg}");
    let songs = pg["songs"].as_array().unwrap();
    assert_eq!(songs.len(), 1, "the private file is left out: {pg}");
    let uri = songs[0]["uri"].as_str().unwrap().to_string();
    assert_eq!(uri, format!("{base}/download/fixture-set/one.flac"));
    assert_eq!(songs[0]["title"], "One");
    assert_eq!(
        (
            pg["plays_from"]["service"].as_str(),
            pg["plays_from"]["mode"].as_str()
        ),
        (Some("archive"), Some("full")),
        "{pg}"
    );
    assert!(pg["unavailable"].is_null());
    let (st, plan) = call(
        &app,
        "GET",
        "/v1/discover/play?uri=https%3A%2F%2Farchive.org%2Fdetails%2Ffixture-set",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{plan}");
    assert_eq!(plan["items"][0]["uri"], uri, "{plan}");
    assert_eq!(plan["plays_from"]["service"], "archive");
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let (st, q) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"items": [{"uri": uri, "title": "One", "album": "Fixture Set"}], "replace": true, "play": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{q}");
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    assert_eq!(zone.wait_stopped().await.error, None);
    // The archive switched off on Sources: the page lists nothing and plays from nowhere
    let off = serde_json::json!({"enabled": false});
    let (st, _) = call(&app, "PUT", "/v1/sources/archive/enabled", Some(off)).await;
    assert_eq!(st, StatusCode::OK);
    let (st, dark) = call(&app, "GET", &format!("/v1/item/album/{id}"), None).await;
    assert_eq!(st, StatusCode::OK, "{dark}");
    assert!(
        dark["songs"].as_array().unwrap().is_empty() && dark["plays_from"].is_null(),
        "{dark}"
    );
    let on = serde_json::json!({"enabled": true});
    let (st, _) = call(&app, "PUT", "/v1/sources/archive/enabled", Some(on)).await;
    assert_eq!(st, StatusCode::OK);
    // A restricted item: the page says so, with nothing to play and no "plays from"
    let id = crate::discover::page_id("https://archive.org/details/locked-set");
    let (st, pg) = call(&app, "GET", &format!("/v1/item/album/{id}"), None).await;
    assert_eq!(st, StatusCode::OK, "{pg}");
    assert!(
        pg["unavailable"]
            .as_str()
            .unwrap_or("")
            .starts_with("Restricted on the Internet Archive"),
        "{pg}"
    );
    assert!(
        pg["songs"].as_array().unwrap().is_empty() && pg["plays_from"].is_null(),
        "{pg}"
    );
    let (st, e) = call(&app, "GET", "/v1/sources/archive/items/locked-set", None).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{e}");
    assert!(
        e["error"]
            .as_str()
            .unwrap()
            .starts_with("Restricted on the Internet Archive"),
        "{e}"
    );
    // An Audius song from a search hit: its row carries the stream, and Play queues it
    let id = crate::discover::page_id("https://audius.co/techwipe/ambient-piano-interlude-353852");
    let (st, pg) = call(&app, "GET", &format!("/v1/item/song/{id}"), None).await;
    assert_eq!(st, StatusCode::OK, "{pg}");
    assert_eq!(pg["title"], "Ambient Piano Interlude", "{pg}");
    let stream = format!("{base}/v1/tracks/W0NJ0/stream?app_name=pubky-music-node");
    assert_eq!(pg["songs"][0]["uri"], stream, "{pg}");
    assert_eq!(pg["plays_from"]["service"], "audius", "{pg}");
    let (st, plan) = call(
        &app,
        "GET",
        "/v1/discover/play?uri=https%3A%2F%2Faudius.co%2Ftechwipe%2Fambient-piano-interlude-353852",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{plan}");
    assert_eq!(plan["items"][0]["uri"], stream, "{plan}");
}

/// Progress changes made at once, from many devices, are all kept: the progress document is
/// one setting, read and written under one lock.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn podcast_progress_changes_at_once_are_all_kept() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/sources/podcasts/enabled",
        Some(serde_json::json!({"enabled": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    drop(app);
    let n = 200u64;
    let feed = podcasts::Feed {
        id: "feed1".into(),
        url: "https://example.com/feed.xml".into(),
        title: "A show".into(),
        episodes: (0..n)
            .map(|i| podcasts::Episode {
                id: format!("ep{i}"),
                title: format!("Episode {i}"),
                url: format!("https://example.com/ep{i}.mp3"),
                ..podcasts::Episode::default()
            })
            .collect(),
        ..podcasts::Feed::default()
    };
    s.library
        .lock()
        .unwrap()
        .set_setting("podcasts.feeds", &serde_json::to_string(&vec![feed]).unwrap())
        .unwrap();
    let mut tasks = Vec::new();
    for i in 0..n {
        let s = s.clone();
        tasks.push(tokio::spawn(async move {
            podcasts::set_progress(
                State(s),
                Path(format!("ep{i}")),
                Json(podcasts::SetProgress {
                    position_ms: Some(1000 + i),
                    played: None,
                }),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.message)
        }));
    }
    for t in tasks {
        t.await.unwrap().unwrap();
    }
    let prog: std::collections::HashMap<String, podcasts::Progress> = serde_json::from_str(
        &s.library
            .lock()
            .unwrap()
            .get_setting("podcasts.progress")
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(prog.len(), n as usize, "every change kept");
    for i in 0..n {
        assert_eq!(prog[&format!("ep{i}")].position_ms, 1000 + i);
    }
}

/// A Bandcamp purchase download, through the router: the claim is made before any network
/// call (a second request for the same purchase while the first looks it up is told it is
/// running), and a request dropped mid-lookup (the page closed) ends its claim as a failed
/// job, so the purchase can be asked for again at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_bandcamp_download_request_frees_its_purchase() {
    let _base_guard = bandcamp::API_TEST_LOCK.lock().await;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let mock = Router::new()
        .route(
            "/api/fan/2/collection_summary",
            get(|| async {
                Json(serde_json::json!({"fan_id": 77, "collection_summary": {"fan_id": 77, "username": "listener"}}))
            }),
        )
        .route(
            "/api/fancollection/1/collection_items",
            post({
                let gate = gate.clone();
                move || {
                    let gate = gate.clone();
                    async move {
                        // The lookup hangs until the test lets it go
                        gate.acquire().await.unwrap().forget();
                        Json(serde_json::json!({"items": [], "more_available": false, "last_token": "0::a::"}))
                    }
                }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    bandcamp::set_site_base(&base);
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/sources/bandcamp/unofficial",
        Some(serde_json::json!({"enabled": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, u) = call(
        &app,
        "PUT",
        "/v1/sources/bandcamp/unofficial",
        Some(serde_json::json!({"identity": "identity=7%09abc%3D%3D"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{u}");
    let id = "pdroptest1";
    let state_of = |app: Router| async move {
        let (_, d) = call(&app, "GET", "/v1/sources/bandcamp/downloads", None).await;
        d.as_array()
            .unwrap()
            .iter()
            .find(|j| j["id"] == id)
            .map(|j| j["state"].as_str().unwrap().to_string())
    };
    let first = {
        let app = app.clone();
        tokio::spawn(async move {
            call(
                &app,
                "POST",
                &format!("/v1/sources/bandcamp/collection/{id}/download"),
                None,
            )
            .await
        })
    };
    for _ in 0..100 {
        if state_of(app.clone()).await.as_deref() == Some("queued") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        state_of(app.clone()).await.as_deref(),
        Some("queued"),
        "claimed before the lookup"
    );
    // A second request meanwhile: already running
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/sources/bandcamp/collection/{id}/download"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    // The first request goes away mid-lookup
    first.abort();
    let _ = first.await;
    assert_eq!(
        state_of(app.clone()).await.as_deref(),
        Some("failed"),
        "the claim ended"
    );
    // The purchase can be asked for again: past the claim, into the (still held) lookup
    let again = tokio::time::timeout(
        std::time::Duration::from_millis(300),
        call(
            &app,
            "POST",
            &format!("/v1/sources/bandcamp/collection/{id}/download"),
            None,
        ),
    )
    .await;
    assert!(again.is_err(), "not refused as running: {again:?}");
    gate.add_permits(10);
    // Its own job only: the list is process-wide, and another test reads its own from it
    bandcamp::forget_job(id);
}
