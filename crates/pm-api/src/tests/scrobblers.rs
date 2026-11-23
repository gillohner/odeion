//! ListenBrainz and Last.fm: what is sent, held back and sent again.
//!
//! Part of pm-api's unit tests (`mod tests` in lib.rs, which holds the helpers these use). A new
//! test of this area goes at the end of this file.

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn listenbrainz_gets_what_zones_play_and_gives_back_history_and_picks() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let got: Arc<std::sync::Mutex<Vec<serde_json::Value>>> = Default::default();
    let down = Arc::new(AtomicBool::new(true));
    let (g1, d1) = (got.clone(), down.clone());
    const PL: &str = "e81452b9-3165-4f43-a8d9-f6c2c5ed7154";
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let token_ok =
        |h: &axum::http::HeaderMap| h.get("authorization").and_then(|v| v.to_str().ok()) == Some("Token LBT");
    let mock = Router::new()
        .route(
            "/1/validate-token",
            get(move |h: axum::http::HeaderMap| async move {
                Json(if token_ok(&h) { serde_json::json!({"valid": true, "user_name": "owner"}) } else { serde_json::json!({"valid": false}) })
            }),
        )
        .route(
            "/1/submit-listens",
            post(move |h: axum::http::HeaderMap, Json(b): Json<serde_json::Value>| {
                let (g, d) = (g1.clone(), d1.clone());
                async move {
                    assert!(token_ok(&h));
                    if b["listen_type"] == "playing_now" {
                        return StatusCode::OK;
                    }
                    if d.load(Ordering::SeqCst) {
                        return StatusCode::SERVICE_UNAVAILABLE;
                    }
                    g.lock().unwrap().extend(b["payload"].as_array().unwrap().iter().cloned());
                    StatusCode::OK
                }
            }),
        )
        .route(
            "/1/user/owner/listens",
            get(|| async {
                Json(serde_json::json!({"payload": {"count": 2, "listens": [
                    {"listened_at": 1792593797, "track_metadata": {"track_name": "Some Resolve", "artist_name": "X", "mbid_mapping": {"recording_mbid": "30d08f4c-d825-4ae1-b79c-44242cddd7c0"}}},
                    {"listened_at": 1792593336, "track_metadata": {"track_name": "Congruence", "artist_name": "Y"}}]}}))
            }),
        )
        .route(
            "/1/cf/recommendation/user/owner/recording",
            get(|| async {
                Json(serde_json::json!({"payload": {"mbids": [{"recording_mbid": "bd953e77-f7df-467b-b1cf-e42d3eec5968", "score": 0.6}, {"recording_mbid": "2456f690-add7-467f-9ca0-000000000000", "score": 0.5}]}}))
            }),
        )
        .route(
            "/1/user/owner/playlists/createdfor",
            get(|| async {
                Json(serde_json::json!({"playlist_count": 1, "playlists": [{"playlist": {"title": "Weekly Jams for owner", "identifier": format!("https://listenbrainz.org/playlist/{PL}"), "track": []}}]}))
            }),
        )
        .route(
            "/1/playlist/{id}",
            get(|| async {
                Json(serde_json::json!({"playlist": {"title": "Weekly Jams for owner", "track": [
                    {"identifier": ["https://musicbrainz.org/recording/bd953e77-f7df-467b-b1cf-e42d3eec5968"], "title": "Sherpa"},
                    {"identifier": ["https://example.com/x"], "title": "?"}]}}))
            }),
        )
        .route(
            "/song.flac",
            get(move || {
                let flac = flac.clone();
                async move { ([(header::CONTENT_TYPE, "audio/flac")], flac) }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    *s.listenbrainz_base.write().unwrap() = format!("http://127.0.0.1:{port}");
    let app = router(s.clone());
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/listenbrainz",
        Some(serde_json::json!({"token": "nope"})),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, v) = call(
        &app,
        "PUT",
        "/v1/listenbrainz",
        Some(serde_json::json!({"token": "LBT"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        (v["user"].as_str(), v["submit"].as_bool()),
        (Some("owner"), Some(true))
    );
    assert!(!v.to_string().contains("LBT"));

    // A zone plays a song to its end while ListenBrainz is down: the listen waits
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    call(&app, "POST", &format!("/v1/zones/{zid}/queue"), Some(serde_json::json!({"items": [
        {"uri": format!("http://127.0.0.1:{port}/song.flac"), "title": "Song", "artist": "Band", "album": "Record"}], "replace": true, "play": true}))).await;
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    assert_eq!(zone.wait_stopped().await.error, None);
    let mut waiting = 0;
    for _ in 0..300 {
        let (_, v) = call(&app, "GET", "/v1/listenbrainz", None).await;
        waiting = v["pending"].as_u64().unwrap();
        if waiting == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(waiting, 1, "kept for later");
    // ListenBrainz is back: the waiting listen goes
    down.store(false, Ordering::SeqCst);
    let (_, v) = call(
        &app,
        "PUT",
        "/v1/listenbrainz",
        Some(serde_json::json!({"submit": true})),
    )
    .await;
    assert_eq!(v["pending"], 0);
    {
        let got = got.lock().unwrap();
        assert_eq!(got.len(), 1, "{got:?}");
        let m = &got[0]["track_metadata"];
        assert_eq!(
            (
                m["track_name"].as_str(),
                m["artist_name"].as_str(),
                m["release_name"].as_str()
            ),
            (Some("Song"), Some("Band"), Some("Record"))
        );
        assert_eq!(m["additional_info"]["media_player"], "Odeion");
        assert!(got[0]["listened_at"].as_i64().unwrap() > 1_700_000_000);
    }

    // History comes back by MusicBrainz recording, once
    let (st, imp) = call(&app, "POST", "/v1/listenbrainz/import", None).await;
    assert_eq!(st, StatusCode::OK, "{imp}");
    assert_eq!(
        (imp["imported"].as_u64(), imp["skipped"].as_u64()),
        (Some(1), Some(1))
    );
    let (_, again) = call(&app, "POST", "/v1/listenbrainz/import", None).await;
    assert_eq!(again["imported"], 0);
    let (_, acts) = call(&app, "GET", "/v1/activity", None).await;
    assert!(
        acts.to_string().contains("30d08f4c-d825-4ae1-b79c-44242cddd7c0"),
        "{acts}"
    );

    // Picks and the playlists made for the user
    let (st, rec) = call(&app, "GET", "/v1/listenbrainz/recommendations", None).await;
    assert_eq!(st, StatusCode::OK, "{rec}");
    assert_eq!(rec.as_array().unwrap().len(), 2);
    let (_, pls) = call(&app, "GET", "/v1/listenbrainz/playlists", None).await;
    assert_eq!(pls[0]["id"], PL);
    let (st, imp) = call(
        &app,
        "POST",
        &format!("/v1/listenbrainz/playlists/{PL}/import"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{imp}");
    assert_eq!(
        (imp["imported"].as_u64(), imp["skipped"].as_u64()),
        (Some(1), Some(1))
    );

    let (_, v) = call(&app, "DELETE", "/v1/listenbrainz", None).await;
    assert_eq!(v["connected"], false);
}

/// Last.fm: the owner's app, the web sign-in through `/lastfm/callback` (a signed getSession),
/// a play scrobbled when it ends (signed, with the session), a stale session noticed and the
/// scrobble kept until the next sign-in, similar artists matched to the library; the secret
/// and the session key never come back.
#[tokio::test]
async fn lastfm_scrobbles_and_finds_similar_artists_from_a_mock() {
    let _api_guard = lastfm::API_TEST_LOCK.lock().await;
    const KEY: &str = "0123456789abcdef0123456789abcdef";
    const SECRET: &str = "fedcba9876543210fedcba9876543210";
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    type Call = std::collections::HashMap<String, String>;
    type Calls = Arc<std::sync::Mutex<Vec<Call>>>;
    type Pred = Box<dyn Fn(&[Call]) -> bool + Send>;
    let calls: Calls = Default::default();
    let refuse = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mock = Router::new()
        .route(
            "/",
            post({
                let calls = calls.clone();
                let refuse = refuse.clone();
                move |body: String| {
                    let calls = calls.clone();
                    let refuse = refuse.clone();
                    async move {
                        let p: std::collections::HashMap<String, String> = url::form_urlencoded::parse(body.as_bytes())
                            .map(|(k, v)| (k.into_owned(), v.into_owned()))
                            .collect();
                        // Every signed call: sorted names and values, then the secret
                        let signed: Vec<(String, String)> = p.iter().filter(|(k, _)| *k != "api_sig" && *k != "format").map(|(k, v)| (k.clone(), v.clone())).collect();
                        assert_eq!(p.get("api_sig").map(String::as_str), Some(lastfm::sign(&signed, SECRET).as_str()), "bad signature: {p:?}");
                        assert_eq!(p.get("api_key").map(String::as_str), Some(KEY));
                        let method = p.get("method").cloned().unwrap_or_default();
                        calls.lock().unwrap().push(p.clone());
                        match method.as_str() {
                            "auth.getSession" => {
                                assert_eq!(p.get("token").map(String::as_str), Some("tok123"));
                                Json(serde_json::json!({"session": {"name": "listener", "key": "sess-key-xyz", "subscriber": 0}}))
                            }
                            "track.updateNowPlaying" => {
                                assert_eq!(p.get("sk").map(String::as_str), Some("sess-key-xyz"));
                                Json(serde_json::json!({"nowplaying": {"artist": {"#text": p.get("artist")}}}))
                            }
                            "user.getInfo" => {
                                if p.get("sk").map(String::as_str) == Some("offline") {
                                    Json(serde_json::json!({"error": 11, "message": "temporarily offline"}))
                                } else if p.get("sk").map(String::as_str) == Some("malformed") {
                                    Json(serde_json::json!({"user": {}}))
                                } else if p.get("sk").map(String::as_str) == Some("sess-key-xyz") {
                                    Json(serde_json::json!({"user": {"name": "listener", "playcount": "1"}}))
                                } else {
                                    Json(serde_json::json!({"error": 9, "message": "Invalid session key - Please re-authenticate"}))
                                }
                            }
                            "track.scrobble" => {
                                assert_eq!(p.get("sk").map(String::as_str), Some("sess-key-xyz"));
                                if refuse.load(std::sync::atomic::Ordering::SeqCst) {
                                    Json(serde_json::json!({"error": 9, "message": "Invalid session key - Please re-authenticate"}))
                                } else {
                                    let n = p.keys().filter(|k| k.starts_with("track[")).count();
                                    Json(serde_json::json!({"scrobbles": {"@attr": {"accepted": n, "ignored": 0}}}))
                                }
                            }
                            _ => Json(serde_json::json!({"error": 3, "message": "Invalid Method"})),
                        }
                    }
                }
            })
            .get(|q: axum::extract::RawQuery| async move {
                let p: std::collections::HashMap<String, String> = url::form_urlencoded::parse(q.0.unwrap_or_default().as_bytes())
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                assert_eq!(p.get("method").map(String::as_str), Some("artist.getSimilar"), "{p:?}");
                assert_eq!(p.get("api_key").map(String::as_str), Some(KEY));
                assert_eq!(p.get("artist").map(String::as_str), Some("Radiohead"), "no mbid: by name");
                Json(serde_json::json!({"similarartists": {"artist": [
                    {"name": "Portishead", "mbid": "8f6bd1e4-fbe1-4f50-aa9b-94c450ec0f11", "match": "0.9", "url": "https://www.last.fm/music/Portishead"},
                    {"name": "Thom Yorke", "mbid": "", "match": 0.5, "url": "https://www.last.fm/music/Thom+Yorke"}
                ], "@attr": {"artist": "Radiohead"}}}))
            }),
        )
        .route("/song.flac", get({ let f = flac.clone(); move || { let f = f.clone(); async move { f } } }));
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    lastfm::set_api_base(&format!("{base}/"));
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    // Two library artists, one of them among the similar ones
    let aid = {
        let lib = s.library.lock().unwrap();
        let mk = |title: &str, artist: &str| pm_model::Track {
            id: None,
            title: title.into(),
            artists: vec![pm_model::Artist {
                id: None,
                name: artist.into(),
                mbid: None,
            }],
            album: None,
            track_no: None,
            disc_no: None,
            duration_ms: Some(200_000),
            file: pm_model::AudioFile {
                path: dir.path().join(format!("{title}.flac")),
                size_bytes: 1,
                mtime_unix: 0,
                container: "flac".into(),
                format: pm_model::AudioFormat::pcm(44_100, 16, 2),
                duration_ms: Some(200_000),
            },
            recording_mbid: None,
            isrc: None,
            acoustid: None,
            canonical: None,
            replaygain_track_gain_db: None,
            replaygain_album_gain_db: None,
            genres: vec![],
        };
        lib.upsert_track(&mk("Creep", "Radiohead")).unwrap();
        lib.upsert_track(&mk("Glory Box", "Portishead")).unwrap();
        lib.list_artists()
            .unwrap()
            .into_iter()
            .find(|a| a.name == "Radiohead")
            .unwrap()
            .id
    };
    // Not configured
    let (_, lf) = call(&app, "GET", "/v1/lastfm", None).await;
    assert_eq!(
        (lf["configured"].clone(), lf["connected"].clone()),
        (serde_json::json!(false), serde_json::json!(false))
    );
    let (_, sim) = call(&app, "GET", &format!("/v1/library/artists/{aid}/similar"), None).await;
    assert_eq!(sim["note"], "not configured");
    let (st, _) = call(&app, "GET", "/v1/lastfm/login", None).await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED);
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/lastfm",
        Some(serde_json::json!({"api_key": "short", "api_secret": SECRET})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, lf) = call(
        &app,
        "PUT",
        "/v1/lastfm",
        Some(serde_json::json!({"api_key": KEY, "api_secret": SECRET})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{lf}");
    assert_eq!(
        (
            lf["configured"].clone(),
            lf["app_from"].clone(),
            lf["api_key"].clone()
        ),
        (
            serde_json::json!(true),
            serde_json::json!("settings"),
            serde_json::json!(KEY)
        )
    );
    assert!(
        !lf.to_string().contains(SECRET),
        "the secret never comes back: {lf}"
    );
    // Sign in: Last.fm's page, then the callback with the token
    let (st, l) = call(&app, "GET", "/v1/lastfm/login", None).await;
    assert_eq!(st, StatusCode::OK, "{l}");
    let url = l["url"].as_str().unwrap();
    assert!(
        url.starts_with("https://www.last.fm/api/auth/?")
            && url.contains(&format!("api_key={KEY}"))
            && url.contains("%2Flastfm%2Fcallback"),
        "{url}"
    );
    let state_of = |url: &str| {
        let u = url::Url::parse(url).unwrap();
        let cb = u
            .query_pairs()
            .find(|(k, _)| k == "cb")
            .expect("cb")
            .1
            .into_owned();
        let cb = url::Url::parse(&cb).unwrap();
        assert_eq!(cb.path(), "/lastfm/callback", "{cb}");
        cb.query_pairs()
            .find(|(k, _)| k == "state")
            .expect("state in cb")
            .1
            .into_owned()
    };
    let first = state_of(url);
    // Last.fm sends the browser back to `cb` with `&token=` added
    let back = |state: &str| {
        let app = app.clone();
        let uri = format!("/lastfm/callback?state={state}&token=tok123");
        async move {
            let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
            let resp = app.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::SEE_OTHER);
            resp.headers()["location"].to_str().unwrap().to_string()
        }
    };
    // No state, or one this node never gave out: refused, Last.fm not asked
    for uri in [
        "/lastfm/callback?token=tok123",
        "/lastfm/callback?state=made-up&token=tok123",
    ] {
        let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert!(
            resp.headers()["location"]
                .to_str()
                .unwrap()
                .starts_with("/?lastfm=error"),
            "{uri}"
        );
    }
    assert!(
        calls.lock().unwrap().is_empty(),
        "no getSession for a sign-in not started here"
    );
    assert_eq!(back(&first).await, "/?lastfm=signed_in&user=listener");
    let (_, lf) = call(&app, "GET", "/v1/lastfm", None).await;
    assert_eq!(
        (lf["connected"].clone(), lf["user"].clone(), lf["submit"].clone()),
        (
            serde_json::json!(true),
            serde_json::json!("listener"),
            serde_json::json!(true)
        )
    );
    assert!(
        !lf.to_string().contains("sess-key"),
        "the session key never comes back: {lf}"
    );
    // A play to its end: now playing at the start, the scrobble when it ends
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let play = |title: &str| serde_json::json!({"items": [{"uri": format!("{base}/song.flac"), "title": title, "artist": "Someone", "album": "Nights"}], "replace": true, "play": true});
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(play("Night Drive")),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let wait_for = |pred: Pred| {
        let calls = calls.clone();
        async move {
            for _ in 0..100 {
                if pred(&calls.lock().unwrap()) {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            panic!(
                "Last.fm never got it: {:?}",
                calls
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|c| c.get("method").cloned().unwrap_or_default())
                    .collect::<Vec<_>>()
            );
        }
    };
    wait_for(Box::new(|c| {
        c.iter()
            .any(|p| p.get("method").map(String::as_str) == Some("track.scrobble"))
    }))
    .await;
    {
        let c = calls.lock().unwrap();
        let np = c
            .iter()
            .find(|p| p.get("method").map(String::as_str) == Some("track.updateNowPlaying"))
            .expect("now playing");
        assert_eq!(
            (
                np.get("artist").map(String::as_str),
                np.get("track").map(String::as_str)
            ),
            (Some("Someone"), Some("Night Drive"))
        );
        assert!(!np.contains_key("chosenByUser"));
        let sc = c
            .iter()
            .find(|p| p.get("method").map(String::as_str) == Some("track.scrobble"))
            .unwrap();
        assert_eq!(sc.get("artist[0]").map(String::as_str), Some("Someone"));
        assert_eq!(sc.get("track[0]").map(String::as_str), Some("Night Drive"));
        assert_eq!(sc.get("album[0]").map(String::as_str), Some("Nights"));
        assert_eq!(sc.get("chosenByUser[0]").map(String::as_str), Some("1"));
        assert!(sc.get("timestamp[0]").is_some_and(|t| t.parse::<i64>().is_ok()));
    }
    // Last.fm drops the session: the scrobble waits and the card says to sign in again
    refuse.store(true, std::sync::atomic::Ordering::SeqCst);
    calls.lock().unwrap().clear();
    let (_, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(play("Second")),
    )
    .await;
    wait_for(Box::new(|c| {
        c.iter()
            .any(|p| p.get("method").map(String::as_str) == Some("track.scrobble"))
    }))
    .await;
    let mut lf = serde_json::Value::Null;
    for _ in 0..50 {
        lf = call(&app, "GET", "/v1/lastfm", None).await.1;
        if lf["pending"] == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(lf["pending"], 1, "{lf}");
    assert!(lf["problem"].as_str().unwrap().contains("sign in again"), "{lf}");
    // Signing in again clears it; the waiting scrobble goes out with the next play. The
    // first sign-in's state is used up; a new one is needed
    refuse.store(false, std::sync::atomic::Ordering::SeqCst);
    calls.lock().unwrap().clear();
    assert!(back(&first).await.starts_with("/?lastfm=error"));
    let (_, lf) = call(&app, "GET", "/v1/lastfm", None).await;
    assert!(
        lf["problem"].as_str().is_some(),
        "a refused callback changes nothing: {lf}"
    );
    let (_, l) = call(&app, "GET", "/v1/lastfm/login", None).await;
    let again = state_of(l["url"].as_str().unwrap());
    assert_ne!(again, first);
    assert_eq!(back(&again).await, "/?lastfm=signed_in&user=listener");
    let (_, lf) = call(&app, "GET", "/v1/lastfm", None).await;
    assert!(lf["problem"].is_null(), "{lf}");
    let (_, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(play("Third")),
    )
    .await;
    wait_for(Box::new(|c| {
        c.iter().any(|p| {
            p.get("method").map(String::as_str) == Some("track.scrobble") && p.contains_key("track[1]")
        })
    }))
    .await;
    {
        let c = calls.lock().unwrap();
        let sc = c.iter().find(|p| p.contains_key("track[1]")).unwrap();
        let titles: std::collections::HashSet<&str> = [sc["track[0]"].as_str(), sc["track[1]"].as_str()]
            .into_iter()
            .collect();
        assert_eq!(titles, ["Second", "Third"].into_iter().collect());
    }
    let (_, lf) = call(&app, "GET", "/v1/lastfm", None).await;
    assert_eq!(lf["pending"], 0, "{lf}");
    // Similar artists, matched to the library by name
    let (st, sim) = call(&app, "GET", &format!("/v1/library/artists/{aid}/similar"), None).await;
    assert_eq!(st, StatusCode::OK, "{sim}");
    let arts = sim["artists"].as_array().unwrap();
    assert_eq!(arts.len(), 2);
    assert_eq!(arts[0]["name"], "Portishead");
    assert!(arts[0]["library_artist_id"].as_i64().is_some(), "{sim}");
    assert_eq!(arts[0]["score"], 0.9);
    assert!(arts[1]["library_artist_id"].is_null());
    // Sign out: the app stays, the session and the queue go
    let (_, lf) = call(&app, "DELETE", "/v1/lastfm", None).await;
    assert_eq!(
        (
            lf["connected"].clone(),
            lf["configured"].clone(),
            lf["pending"].clone()
        ),
        (
            serde_json::json!(false),
            serde_json::json!(true),
            serde_json::json!(0)
        )
    );
    assert!(
        backup::SECRET_SETTINGS.iter().any(|(k, _)| *k == lastfm::APP_KEY)
            && backup::SECRET_SETTINGS
                .iter()
                .any(|(k, _)| *k == lastfm::SESSION_KEY)
    );
    // A sign-in pasted into the Sign in chooser (key, secret, session key): checked with
    // user.getInfo, then kept like the node's own; a dead session key is refused
    call(&app, "DELETE", "/v1/sources/lastfm/signin", None).await;
    let (st, r) = call(
        &app,
        "POST",
        "/v1/sources/lastfm/signin/import",
        Some(serde_json::json!({"text": format!("{KEY}\n{SECRET}\nsess-key-xyz\n")})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["in_use"]["way"], "paste");
    assert_eq!(r["in_use"]["user"], "listener");
    assert_eq!(r["line"], "Pasted sign-in · listener");
    assert!(
        !r.to_string().contains("sess-key-xyz") && !r.to_string().contains(SECRET),
        "{r}"
    );
    let (st, r) = call(
        &app,
        "POST",
        "/v1/sources/lastfm/signin/import",
        Some(serde_json::json!({"text": format!("{KEY}\n{SECRET}\ndeadkey\n")})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{r}");
    assert!(r["error"].as_str().unwrap().contains("did not accept"), "{r}");
    for key in ["offline", "malformed"] {
        let (st, _) = call(
            &app,
            "POST",
            "/v1/sources/lastfm/signin/import",
            Some(serde_json::json!({"text": format!("{KEY}\n{SECRET}\n{key}\n")})),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        let (_, current) = call(&app, "GET", "/v1/sources/lastfm/signin", None).await;
        assert_eq!(
            current["in_use"]["user"], "listener",
            "failed imports preserve the working sign-in"
        );
    }
}

/// A listener's play (or a party's) is not the owner's listen: nothing of it goes to the owner's
/// ListenBrainz (or Last.fm), not even "playing now"; the owner's own play still does
/// (security review 2026-11-02, medium 1).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_listeners_plays_are_not_scrobbled_to_the_owner() {
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let got: Arc<std::sync::Mutex<Vec<serde_json::Value>>> = Default::default();
    let g1 = got.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = Router::new()
        .route(
            "/1/validate-token",
            get(|| async { Json(serde_json::json!({"valid": true, "user_name": "owner"})) }),
        )
        .route(
            "/1/submit-listens",
            post(move |Json(b): Json<serde_json::Value>| {
                let g = g1.clone();
                async move {
                    g.lock().unwrap().push(b);
                    StatusCode::OK
                }
            }),
        )
        .route(
            "/song.flac",
            get(move || {
                let flac = flac.clone();
                async move { ([(header::CONTENT_TYPE, "audio/flac")], flac) }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    *s.listenbrainz_base.write().unwrap() = format!("http://127.0.0.1:{port}");
    let app = router(s.clone());
    let (st, v) = call(
        &app,
        "PUT",
        "/v1/listenbrainz",
        Some(serde_json::json!({"token": "LBT"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let play = || {
        let app = app.clone();
        let (zid, port) = (zid.clone(), port);
        async move {
            call(&app, "POST", &format!("/v1/zones/{zid}/queue"), Some(serde_json::json!({"items": [
                {"uri": format!("http://127.0.0.1:{port}/song.flac"), "title": "Song", "artist": "Band"}], "replace": true, "play": true}))).await;
        }
    };
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();

    // A party has the zone: its plays are the guests', whoever queues
    let (st, party) = call(&app, "POST", "/v1/party", Some(serde_json::json!({"zone": zid}))).await;
    assert_eq!(st, StatusCode::OK, "{party}");
    play().await;
    assert_eq!(zone.wait_stopped().await.error, None);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let sent = got.lock().unwrap().clone();
    assert!(sent.is_empty(), "{sent:?}");

    // The owner's again
    let (st, _) = call(
        &app,
        "DELETE",
        &format!("/v1/party/{}", party["id"].as_str().unwrap()),
        None,
    )
    .await;
    assert!(st.is_success(), "{st}");
    play().await;
    assert_eq!(zone.wait_stopped().await.error, None);
    for _ in 0..100 {
        if got
            .lock()
            .unwrap()
            .iter()
            .any(|b| b["listen_type"] != "playing_now")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        got.lock()
            .unwrap()
            .iter()
            .any(|b| b["listen_type"] != "playing_now"),
        "the owner's play counts"
    );
}

/// A song the owner started whose zone goes to a listener (or a party) before its listen is
/// due is not the owner's listen: its waiting entry is dropped, so even the zone coming back
/// to the owner doesn't count it (security review 2026-11-02, medium 1).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_zone_changing_hands_mid_song_drops_its_scrobble() {
    let got: Arc<std::sync::Mutex<Vec<serde_json::Value>>> = Default::default();
    let g1 = got.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = Router::new()
        .route(
            "/1/validate-token",
            get(|| async { Json(serde_json::json!({"valid": true, "user_name": "owner"})) }),
        )
        .route(
            "/1/submit-listens",
            post(move |Json(b): Json<serde_json::Value>| {
                let g = g1.clone();
                async move {
                    g.lock().unwrap().push(b);
                    StatusCode::OK
                }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    *s.listenbrainz_base.write().unwrap() = format!("http://127.0.0.1:{port}");
    let app = router(s.clone());
    let (st, v) = call(
        &app,
        "PUT",
        "/v1/listenbrainz",
        Some(serde_json::json!({"token": "LBT"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = ZoneId::new(zones[0]["id"].as_str().unwrap());
    let zone = s.zones.get(&zid).unwrap();
    let item: pm_zones::QueueItem = serde_json::from_value(serde_json::json!({
        "id": 7, "uri": "http://127.0.0.1:9/song.flac", "title": "Song", "artist": "Band", "duration_ms": 200000
    }))
    .unwrap();
    let listens = |g: &Vec<serde_json::Value>| g.iter().filter(|b| b["listen_type"] != "playing_now").count();

    // The owner's song starts: "playing now"
    listenbrainz::track_started(&s, zone.clone(), item.clone());
    for _ in 0..100 {
        if !got.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(got.lock().unwrap().len(), 1, "playing now");
    // A listener takes the zone before it counts; it plays to its end
    s.zones_in_listeners_hands.lock().unwrap().insert(zid.0.clone());
    listenbrainz::track_ended(&s, &zid, &item);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(listens(&got.lock().unwrap()), 0, "not the owner's listen");
    // Back in the owner's hands, the dropped entry stays dropped
    s.zones_in_listeners_hands.lock().unwrap().remove(&zid.0);
    listenbrainz::track_ended(&s, &zid, &item);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let sent = got.lock().unwrap().clone();
    assert_eq!(listens(&sent), 0, "{sent:?}");
}

/// Two plays ending together, with listens waiting from before: each listen reaches
/// ListenBrainz once (the waiting ones are taken by one sender), and when ListenBrainz can't
/// take them, every one waits again, none twice, and goes once it is back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_listenbrainz_senders_with_a_backlog_send_each_listen_once() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    // The account the senders send for (a sender checks it is still the one connected)
    s.library
        .lock()
        .unwrap()
        .set_setting(
            "listenbrainz.account",
            &serde_json::json!({"token": "LBT", "user": "owner", "submit": true}).to_string(),
        )
        .unwrap();
    let got: Arc<std::sync::Mutex<Vec<serde_json::Value>>> = Default::default();
    let refuse = Arc::new(AtomicBool::new(false));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    *s.listenbrainz_base.write().unwrap() =
        format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let mock = Router::new().route(
        "/1/submit-listens",
        post({
            let (got, refuse) = (got.clone(), refuse.clone());
            move |Json(b): Json<serde_json::Value>| {
                let (got, refuse) = (got.clone(), refuse.clone());
                async move {
                    // Slow enough that both senders are in flight at once
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    if refuse.load(Ordering::SeqCst) {
                        return StatusCode::SERVICE_UNAVAILABLE;
                    }
                    got.lock()
                        .unwrap()
                        .extend(b["payload"].as_array().unwrap().iter().cloned());
                    StatusCode::OK
                }
            }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let listen = |n: u64| serde_json::json!({"listened_at": n, "track_metadata": {"artist_name": "A", "track_name": format!("T{n}")}});
    let waiting = serde_json::Value::from((1..=3).map(listen).collect::<Vec<_>>()).to_string();
    let pending = || {
        s.library
            .lock()
            .unwrap()
            .get_setting("listenbrainz.pending")
            .unwrap()
    };
    let times = |list: &[serde_json::Value]| {
        let mut t: Vec<u64> = list.iter().map(|l| l["listened_at"].as_u64().unwrap()).collect();
        t.sort();
        t
    };
    s.library
        .lock()
        .unwrap()
        .set_setting("listenbrainz.pending", &waiting)
        .unwrap();
    tokio::join!(
        listenbrainz::send_listens(&s, "LBT", vec![listen(4)]),
        listenbrainz::send_listens(&s, "LBT", vec![listen(5)])
    );
    assert_eq!(times(&got.lock().unwrap()), [1, 2, 3, 4, 5], "each listen once");
    assert_eq!(pending(), None);
    // Refused: everything waits again, nothing twice
    refuse.store(true, Ordering::SeqCst);
    got.lock().unwrap().clear();
    s.library
        .lock()
        .unwrap()
        .set_setting("listenbrainz.pending", &waiting)
        .unwrap();
    tokio::join!(
        listenbrainz::send_listens(&s, "LBT", vec![listen(4)]),
        listenbrainz::send_listens(&s, "LBT", vec![listen(5)])
    );
    let left: Vec<serde_json::Value> = serde_json::from_str(&pending().unwrap()).unwrap();
    assert_eq!(times(&left), [1, 2, 3, 4, 5], "all wait, none twice");
    // Once ListenBrainz is back, the next send takes them
    refuse.store(false, Ordering::SeqCst);
    listenbrainz::send_listens(&s, "LBT", Vec::new()).await;
    assert_eq!(times(&got.lock().unwrap()), [1, 2, 3, 4, 5]);
    assert_eq!(pending(), None);
}

/// The same for Last.fm: two senders with a backlog send each scrobble once; refused ones
/// all wait again, none twice.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_lastfm_senders_with_a_backlog_send_each_scrobble_once() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let _api_guard = lastfm::API_TEST_LOCK.lock().await;
    const KEY: &str = "0123456789abcdef0123456789abcdef";
    const SECRET: &str = "fedcba9876543210fedcba9876543210";
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let got: Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let refuse = Arc::new(AtomicBool::new(false));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let mock = Router::new().route(
        "/",
        post({
            let (got, refuse) = (got.clone(), refuse.clone());
            move |body: String| {
                let (got, refuse) = (got.clone(), refuse.clone());
                async move {
                    let p: std::collections::HashMap<String, String> =
                        url::form_urlencoded::parse(body.as_bytes())
                            .map(|(k, v)| (k.into_owned(), v.into_owned()))
                            .collect();
                    assert_eq!(p.get("method").map(String::as_str), Some("track.scrobble"), "{p:?}");
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    if refuse.load(Ordering::SeqCst) {
                        return Json(serde_json::json!({"error": 9, "message": "Invalid session key - Please re-authenticate"}));
                    }
                    let tracks: Vec<String> = p
                        .iter()
                        .filter(|(k, _)| k.starts_with("track["))
                        .map(|(_, v)| v.clone())
                        .collect();
                    let n = tracks.len();
                    got.lock().unwrap().extend(tracks);
                    Json(serde_json::json!({"scrobbles": {"@attr": {"accepted": n, "ignored": 0}}}))
                }
            }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    lastfm::set_api_base(&format!("{base}/"));
    // The app in this node's own settings, never the process-wide config another test reads
    let app = serde_json::json!({"api_key": KEY, "api_secret": SECRET, "from": "settings"}).to_string();
    s.library
        .lock()
        .unwrap()
        .set_setting(lastfm::APP_KEY, &app)
        .unwrap();
    let session = serde_json::json!({"key": "sess-key-xyz", "user": "listener", "submit": true}).to_string();
    s.library
        .lock()
        .unwrap()
        .set_setting("lastfm.session", &session)
        .unwrap();
    let scrobble =
        |n: u64| serde_json::json!({"artist": "A", "track": format!("T{n}"), "timestamp": n.to_string()});
    let waiting = serde_json::Value::from((1..=3).map(scrobble).collect::<Vec<_>>()).to_string();
    let pending = || s.library.lock().unwrap().get_setting("lastfm.pending").unwrap();
    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };
    s.library
        .lock()
        .unwrap()
        .set_setting("lastfm.pending", &waiting)
        .unwrap();
    tokio::join!(
        lastfm::send_scrobbles(&s, vec![scrobble(4)]),
        lastfm::send_scrobbles(&s, vec![scrobble(5)])
    );
    assert_eq!(
        sorted(got.lock().unwrap().clone()),
        ["T1", "T2", "T3", "T4", "T5"],
        "each once"
    );
    assert_eq!(pending(), None);
    // Refused (the session is marked as broken): everything waits again, nothing twice
    refuse.store(true, Ordering::SeqCst);
    s.library
        .lock()
        .unwrap()
        .set_setting("lastfm.pending", &waiting)
        .unwrap();
    tokio::join!(
        lastfm::send_scrobbles(&s, vec![scrobble(4)]),
        lastfm::send_scrobbles(&s, vec![scrobble(5)])
    );
    let left: Vec<serde_json::Value> = serde_json::from_str(&pending().unwrap()).unwrap();
    let left: Vec<String> = left
        .iter()
        .map(|x| x["track"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        sorted(left),
        ["T1", "T2", "T3", "T4", "T5"],
        "all wait, none twice"
    );
}

/// A send cut short (the node stopping mid-send: the task is dropped while ListenBrainz
/// hasn't answered) loses nothing: every waiting listen, the new one too, is still on disk,
/// and the next send takes them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_listen_send_cut_short_loses_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let (gate, got, entered) = gated_listenbrainz(&s).await;
    crate::send_queue::enqueue(
        &s.library.lock().unwrap(),
        "listenbrainz.pending",
        (1..=3).map(a_listen).collect(),
        1000,
    )
    .unwrap();
    let send = {
        let s = s.clone();
        tokio::spawn(async move { listenbrainz::send_listens(&s, "LBT", vec![a_listen(4)]).await })
    };
    // Prove the request reached the remote boundary before cancelling; it has not
    // committed any listens while its gate is closed.
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert_eq!(waiting_listens(&s), [1, 2, 3, 4], "counted while sending");
    send.abort();
    let _ = send.await;
    assert_eq!(
        waiting_listens(&s),
        [1, 2, 3, 4],
        "nothing lost when the send stops"
    );
    // Dropping a client does not cancel an Axum handler. Permanently refuse that
    // abandoned request before allowing a retry to commit at a fresh endpoint.
    gate.close();
    assert!(got.lock().unwrap().is_empty());
    let (retry_gate, retried, _) = gated_listenbrainz(&s).await;
    retry_gate.add_permits(10);
    listenbrainz::send_listens(&s, "LBT", Vec::new()).await;
    let mut sent: Vec<u64> = retried
        .lock()
        .unwrap()
        .iter()
        .map(|l| l["listened_at"].as_u64().unwrap())
        .collect();
    sent.sort();
    assert_eq!(sent, [1, 2, 3, 4]);
    assert!(waiting_listens(&s).is_empty());
    assert!(
        got.lock().unwrap().is_empty(),
        "abandoned request never committed"
    );
}

/// A listen added while a send is under way waits on disk at once, isn't lost when the send
/// ends, and goes with the next sender; nothing is sent twice.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_listen_added_during_a_send_waits_and_goes_next() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let (gate, got, _) = gated_listenbrainz(&s).await;
    let first = {
        let s = s.clone();
        tokio::spawn(async move { listenbrainz::send_listens(&s, "LBT", vec![a_listen(1)]).await })
    };
    for _ in 0..100 {
        if !waiting_listens(&s).is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    // The first send is held at ListenBrainz; a second play ends now
    let second = {
        let s = s.clone();
        tokio::spawn(async move { listenbrainz::send_listens(&s, "LBT", vec![a_listen(2)]).await })
    };
    for _ in 0..100 {
        if waiting_listens(&s) == [1, 2] {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        waiting_listens(&s),
        [1, 2],
        "the new listen waits on disk at once"
    );
    gate.add_permits(10);
    first.await.unwrap();
    second.await.unwrap();
    let mut sent: Vec<u64> = got
        .lock()
        .unwrap()
        .iter()
        .map(|l| l["listened_at"].as_u64().unwrap())
        .collect();
    sent.sort();
    assert_eq!(sent, [1, 2], "each once");
    assert!(waiting_listens(&s).is_empty());
}

/// When ListenBrainz takes part of the backlog (one chunk of 100 refused, the other taken),
/// exactly the refused ones wait.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_the_listens_listenbrainz_refused_wait() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let (gate, got, _) = gated_listenbrainz(&s).await;
    gate.add_permits(10);
    crate::send_queue::enqueue(
        &s.library.lock().unwrap(),
        "listenbrainz.pending",
        (0..150).map(a_listen).collect(),
        1000,
    )
    .unwrap();
    listenbrainz::send_listens(&s, "LBT", Vec::new()).await;
    assert_eq!(
        waiting_listens(&s),
        (0..100).collect::<Vec<u64>>(),
        "the refused chunk waits"
    );
    assert_eq!(got.lock().unwrap().len(), 50, "the other chunk went");
}

/// A sender that waited for its turn while the owner connected another ListenBrainz account
/// sends nothing with the old account: the waiting list is the new account's now, and its own
/// sender sends it there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_waiting_sender_never_sends_the_next_accounts_listens() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let (gate, got, _) = gated_listenbrainz(&s).await;
    let account =
        |token: &str| serde_json::json!({"token": token, "user": token, "submit": true}).to_string();
    s.library
        .lock()
        .unwrap()
        .set_setting("listenbrainz.account", &account("TOKA"))
        .unwrap();
    // A's first sender is held at ListenBrainz with listen 1; A's second waits its turn
    let first = {
        let s = s.clone();
        tokio::spawn(async move { listenbrainz::send_listens(&s, "TOKA", vec![a_listen(1)]).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let waiting = {
        let s = s.clone();
        tokio::spawn(async move { listenbrainz::send_listens(&s, "TOKA", vec![a_listen(2)]).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    // The owner connects B; B's play ends
    s.library
        .lock()
        .unwrap()
        .set_setting("listenbrainz.account", &account("TOKB"))
        .unwrap();
    let b = {
        let s = s.clone();
        tokio::spawn(async move { listenbrainz::send_listens(&s, "TOKB", vec![a_listen(3)]).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    gate.add_permits(10);
    first.await.unwrap();
    waiting.await.unwrap();
    b.await.unwrap();
    let to = |token: &str| {
        let mut v: Vec<u64> = got
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l["_to"] == format!("Token {token}"))
            .map(|l| l["listened_at"].as_u64().unwrap())
            .collect();
        v.sort();
        v
    };
    assert_eq!(to("TOKA"), [1], "A's account got only what A's first sender took");
    assert_eq!(to("TOKB"), [2, 3], "the rest went to the account connected now");
    assert!(waiting_listens(&s).is_empty());
}

/// Capture the production play future without polling it, then really disconnect and
/// reconnect. Neither a different token nor reuse of the same token revives that play.
#[tokio::test]
async fn an_unpolled_old_play_cannot_enqueue_after_reconnect() {
    for next_token in ["NEXT", "LBT"] {
        let dir = tempfile::tempdir().unwrap();
        let s = state(dir.path(), None).await;
        let (gate, got, _) = gated_listenbrainz(&s).await;
        gate.add_permits(10);
        let old = listenbrainz::send_play(s.clone(), vec![a_listen(1)]);
        let _ = listenbrainz::disconnect(axum::extract::State(s.clone()))
            .await
            .unwrap();
        let _ = listenbrainz::connect(
            axum::extract::State(s.clone()),
            Json(ListenBrainzForm {
                token: Some(next_token.into()),
                submit: Some(true),
            }),
        )
        .await
        .unwrap();
        old.await;
        assert!(
            waiting_listens(&s).is_empty(),
            "old play never entered replacement queue"
        );
        listenbrainz::send_listens(&s, next_token, vec![a_listen(2)]).await;
        let sent: Vec<u64> = got
            .lock()
            .unwrap()
            .iter()
            .map(|l| l["listened_at"].as_u64().unwrap())
            .collect();
        assert_eq!(sent, [2], "only the replacement account's own play was sent");
        assert!(waiting_listens(&s).is_empty());
    }
}

/// Failed account lifecycle writes must leave every related setting unchanged and an
/// already captured, unpolled counted play deliverable under the still-current account.
#[tokio::test]
async fn failed_listenbrainz_account_changes_preserve_unpolled_plays() {
    for generation in [None, Some("generation-A")] {
        for (disconnect, key) in [
            (false, "listenbrainz.account"),
            (false, "vault.lineage.listenbrainz"),
            (true, "listenbrainz.account"),
            (true, "listenbrainz.pending"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let s = state(dir.path(), None).await;
            let (gate, got, _) = gated_listenbrainz(&s).await;
            gate.add_permits(10);
            {
                let lib = s.library.lock().unwrap();
                if let Some(g) = generation {
                    lib.set_setting("listenbrainz.send_generation", g).unwrap();
                }
                lib.set_setting("vault.lineage.listenbrainz", "1-original")
                    .unwrap();
                crate::send_queue::enqueue(&lib, "listenbrainz.pending", vec![a_listen(8)], 1000).unwrap();
            }
            let snapshot = || {
                let lib = s.library.lock().unwrap();
                [
                    "listenbrainz.account",
                    "listenbrainz.send_generation",
                    "listenbrainz.pending",
                    "vault.lineage.listenbrainz",
                ]
                .map(|key| lib.get_setting(key).unwrap())
            };
            let before = snapshot();
            let old = listenbrainz::send_play(s.clone(), vec![a_listen(1)]);
            let db = rusqlite::Connection::open(dir.path().join("lib.sqlite")).unwrap();
            let (op, row) = if disconnect {
                ("DELETE", "OLD")
            } else {
                ("INSERT", "NEW")
            };
            db.execute_batch(&format!(
                "CREATE TRIGGER reject_account_change BEFORE {op} ON settings WHEN {row}.key = '{key}' BEGIN SELECT RAISE(ABORT, 'fixture persistence failure'); END;"
            )).unwrap();
            let failed = if disconnect {
                listenbrainz::disconnect(axum::extract::State(s.clone()))
                    .await
                    .is_err()
            } else {
                listenbrainz::connect(
                    axum::extract::State(s.clone()),
                    Json(ListenBrainzForm {
                        token: Some("NEXT".into()),
                        submit: Some(true),
                    }),
                )
                .await
                .is_err()
            };
            assert!(failed, "the injected {key} failure must reach the caller");
            assert_eq!(snapshot(), before, "all account state rolls back for {key}");
            db.execute_batch("DROP TRIGGER reject_account_change").unwrap();
            old.await;
            let mut sent: Vec<u64> = got
                .lock()
                .unwrap()
                .iter()
                .map(|v| v["listened_at"].as_u64().unwrap())
                .collect();
            sent.sort();
            assert_eq!(sent, [1, 8], "the original account can still deliver both plays");
            assert!(waiting_listens(&s).is_empty());
        }
    }
}

/// A new listen the library refuses to store (a failing write) is still sent with the rest,
/// from memory: it isn't lost before any try.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_listen_that_cant_be_stored_is_still_sent() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let (gate, got, _) = gated_listenbrainz(&s).await;
    gate.add_permits(10);
    let db = rusqlite::Connection::open(dir.path().join("lib.sqlite")).unwrap();
    db.execute_batch(
        "CREATE TRIGGER no_pending_i BEFORE INSERT ON settings WHEN NEW.key = 'listenbrainz.pending'
             BEGIN SELECT RAISE(ABORT, 'disk I/O error'); END;
             CREATE TRIGGER no_pending_u BEFORE UPDATE ON settings WHEN NEW.key = 'listenbrainz.pending'
             BEGIN SELECT RAISE(ABORT, 'disk I/O error'); END;",
    )
    .unwrap();
    listenbrainz::send_listens(&s, "LBT", vec![a_listen(7)]).await;
    let sent: Vec<u64> = got
        .lock()
        .unwrap()
        .iter()
        .map(|l| l["listened_at"].as_u64().unwrap())
        .collect();
    assert_eq!(sent, [7], "sent from memory");
}

/// A listen the library refused to store is sent from memory; ListenBrainz refuses it, and
/// by then the library takes writes again: it is stored to wait for the next send, not lost.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_listen_sent_from_memory_and_refused_waits_once_storage_is_back() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let (gate, got, entered) = gated_listenbrainz(&s).await;
    let db = refuse_writes_of(dir.path(), "listenbrainz.pending");
    // `listened_at` 0: the mock refuses it
    let send = listenbrainz::send_listens(&s, "LBT", vec![a_listen(0)]);
    let drive = async {
        entered.acquire().await.unwrap().forget();
        writes_work_again(&db);
        gate.add_permits(1);
    };
    tokio::join!(send, drive);
    assert!(got.lock().unwrap().is_empty());
    assert_eq!(waiting_listens(&s), [0], "kept for the next send");
}

/// A full backlog (the cap) and a new listen, ListenBrainz taking everything: every one is
/// sent, the oldest too; the cap only holds what is still waiting after a send.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_full_listenbrainz_backlog_and_a_new_listen_are_all_sent() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let (gate, got, _) = gated_listenbrainz(&s).await;
    gate.add_permits(100);
    let backlog = serde_json::Value::from((1..=1000).map(a_listen).collect::<Vec<_>>()).to_string();
    s.library
        .lock()
        .unwrap()
        .set_setting("listenbrainz.pending", &backlog)
        .unwrap();
    listenbrainz::send_listens(&s, "LBT", vec![a_listen(1001)]).await;
    let mut sent: Vec<u64> = got
        .lock()
        .unwrap()
        .iter()
        .map(|l| l["listened_at"].as_u64().unwrap())
        .collect();
    sent.sort();
    assert_eq!(
        sent,
        (1..=1001).collect::<Vec<u64>>(),
        "none dropped before it was tried"
    );
    assert!(waiting_listens(&s).is_empty());
}

/// The same for Last.fm: a scrobble the library refused, sent from memory and refused by
/// Last.fm, is stored once the library takes writes again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_scrobble_sent_from_memory_and_refused_waits_once_storage_is_back() {
    let _api_guard = lastfm::API_TEST_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let LastfmMock {
        gate,
        got,
        refuse,
        entered,
    } = gated_lastfm(&s).await;
    refuse.store(true, std::sync::atomic::Ordering::SeqCst);
    let db = refuse_writes_of(dir.path(), "lastfm.pending");
    let send = lastfm::send_scrobbles(&s, vec![a_scrobble(7)]);
    let drive = async {
        entered.acquire().await.unwrap().forget();
        writes_work_again(&db);
        gate.add_permits(1);
    };
    tokio::join!(send, drive);
    assert!(got.lock().unwrap().is_empty());
    assert_eq!(waiting_scrobbles(&s), ["T7"], "kept for the next send");
}

/// The same for Last.fm: a full backlog and a new scrobble are all sent, the oldest too.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_full_lastfm_backlog_and_a_new_scrobble_are_all_sent() {
    let _api_guard = lastfm::API_TEST_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let LastfmMock { gate, got, .. } = gated_lastfm(&s).await;
    gate.add_permits(100);
    let backlog = serde_json::Value::from((1..=1000).map(a_scrobble).collect::<Vec<_>>()).to_string();
    s.library
        .lock()
        .unwrap()
        .set_setting("lastfm.pending", &backlog)
        .unwrap();
    lastfm::send_scrobbles(&s, vec![a_scrobble(1001)]).await;
    let mut sent: Vec<u64> = got
        .lock()
        .unwrap()
        .iter()
        .map(|t| t[1..].parse().unwrap())
        .collect();
    sent.sort();
    assert_eq!(
        sent,
        (1..=1001).collect::<Vec<u64>>(),
        "none dropped before it was tried"
    );
    assert!(waiting_scrobbles(&s).is_empty());
}

/// A play that started under one Last.fm sign-in and counts after another sign-in (another
/// account, or the same session key signed in again) isn't scrobbled for the new one; one of
/// the sign-in now is.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lastfm_play_from_before_another_sign_in_is_not_sent_for_it() {
    let _api_guard = lastfm::API_TEST_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let LastfmMock { gate, got, .. } = gated_lastfm(&s).await;
    gate.add_permits(100);
    let a = lastfm::current_owner(&s).expect("signed in");
    lastfm::signed_in(&s.library.lock().unwrap(), &lastfm_session("other-account"));
    lastfm::send_scrobbles_for(&s, Some(a), vec![a_scrobble(1)]).await;
    let b = lastfm::current_owner(&s).unwrap();
    lastfm::signed_in(&s.library.lock().unwrap(), &lastfm_session("other-account"));
    lastfm::send_scrobbles_for(&s, Some(b), vec![a_scrobble(2)]).await;
    assert!(got.lock().unwrap().is_empty(), "{:?}", got.lock().unwrap());
    assert!(waiting_scrobbles(&s).is_empty(), "{:?}", waiting_scrobbles(&s));
    let now = lastfm::current_owner(&s).unwrap();
    lastfm::send_scrobbles_for(&s, Some(now), vec![a_scrobble(3)]).await;
    assert_eq!(*got.lock().unwrap(), ["T3"]);
}

/// Settings' sign-out forgets the scrobbles only memory had, and one a sender still had in
/// hand (it holds it again for its own sign-in) isn't sent when the same session key signs
/// in again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lastfm_sign_out_forgets_held_scrobbles_also_for_the_same_key_again() {
    let _api_guard = lastfm::API_TEST_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let LastfmMock {
        gate,
        got,
        refuse,
        entered,
    } = gated_lastfm(&s).await;
    lastfm::signed_in(&s.library.lock().unwrap(), &lastfm_session("sess-key-xyz"));
    refuse.store(true, std::sync::atomic::Ordering::SeqCst);
    let db = refuse_writes_of(dir.path(), "lastfm.pending");
    let send = lastfm::send_scrobbles(&s, vec![a_scrobble(7)]);
    let drive = async {
        entered.acquire().await.unwrap().forget();
        // Settings' sign-out (the sign-in source's), not `DELETE /v1/lastfm`
        use crate::signin::SignInSource as _;
        lastfm::LastfmSignIn
            .sign_out_of(&s, &crate::accounts::Who::Owner, "browser")
            .unwrap();
        gate.add_permits(1);
    };
    tokio::join!(send, drive);
    writes_work_again(&db);
    refuse.store(false, std::sync::atomic::Ordering::SeqCst);
    lastfm::signed_in(&s.library.lock().unwrap(), &lastfm_session("sess-key-xyz"));
    gate.add_permits(100);
    lastfm::send_scrobbles(&s, vec![a_scrobble(8)]).await;
    assert_eq!(
        *got.lock().unwrap(),
        ["T8"],
        "the old sign-in's scrobble stays unsent"
    );
    assert!(waiting_scrobbles(&s).is_empty());
}

/// A late "sign in again" answer to a call of an older sign-in doesn't mark the newer one
/// broken; one for the sign-in now does.
#[tokio::test]
async fn a_late_lastfm_refusal_for_an_old_sign_in_leaves_the_new_one_alone() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    lastfm::signed_in(&s.library.lock().unwrap(), &lastfm_session("old"));
    let old = lastfm::current_owner(&s).unwrap();
    lastfm::signed_in(&s.library.lock().unwrap(), &lastfm_session("new"));
    let problem = |s: &NodeState| -> serde_json::Value {
        let j = s
            .library
            .lock()
            .unwrap()
            .get_setting(lastfm::SESSION_KEY)
            .unwrap()
            .unwrap();
        serde_json::from_str::<serde_json::Value>(&j).unwrap()["problem"].clone()
    };
    let refused = lastfm::CallError::Session("error 9".into());
    lastfm::note_problem(&s, &refused, &old);
    assert_eq!(problem(&s), serde_json::Value::Null);
    let new = lastfm::current_owner(&s).unwrap();
    lastfm::note_problem(&s, &refused, &new);
    assert_eq!(problem(&s), serde_json::json!("error 9"));
}
