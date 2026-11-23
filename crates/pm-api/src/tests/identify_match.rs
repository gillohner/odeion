//! Identifying files and matching the library to MusicBrainz.
//!
//! Part of pm-api's unit tests (`mod tests` in lib.rs, which holds the helpers these use). A new
//! test of this area goes at the end of this file.

use super::*;

/// Identify library: an untagged file is fingerprinted, looked up on
/// AcoustID once (the answer is cached by file key), and identified;
/// without a key, MusicBrainz search gives suggestions the owner must
/// confirm.
#[tokio::test]
async fn identify_library_fingerprints_looks_up_and_suggests() {
    let single = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-library/tests/fixtures/acoustid/single.json"
    ))
    .unwrap();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c2 = calls.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = Router::new()
        .route(
            "/v2/lookup",
            post(move |body: String| {
                let single = single.clone();
                let calls = c2.clone();
                async move {
                    // Saving a key checks it with one lookup
                    if body.contains("client=OwnKey42") {
                        return ([(header::CONTENT_TYPE, "application/json")], r#"{"status":"ok","results":[]}"#.to_string());
                    }
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    assert!(body.contains("client=test-key") && body.contains("fingerprint=AQ"), "{body}");
                    ([(header::CONTENT_TYPE, "application/json")], single)
                }
            }),
        )
        .route(
            "/ws/2/recording",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "application/json")],
                    r#"{"recordings":[{"id":"b1a9c0e9-d987-4042-ae91-78d6a3267d69","score":100,"title":"Bohemian Rhapsody"},{"id":"ebf79ba5-085e-48d2-9eb8-2d992fbf0f6d","score":62,"title":"Bohemian Rhapsody"}]}"#,
                )
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    let a = music.join("11 bohemian.wav");
    tone_wav(&a, 10);
    let s = state(dir.path(), None).await;
    s.set_contact("test@example.org").unwrap();
    *s.acoustid.write().unwrap() = identify::AcoustIdSettings {
        api_key: "test-key".into(),
        base_url: format!("http://127.0.0.1:{port}"),
        configured: String::new(),
    };
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    let tid = s
        .library
        .lock()
        .unwrap()
        .upsert_track(&untagged_track(a.clone(), "Bohemian Rhapsody", "Queen"))
        .unwrap();
    let app = router(s.clone());
    let (_, st) = call(&app, "GET", "/v1/library/identify", None).await;
    assert_eq!(st["counts"]["unchecked"], 1, "{st}");
    assert_eq!(st["acoustid"], true);
    identify::run(&s, false).await.unwrap();
    let t = s.library.lock().unwrap().get_track(tid).unwrap().unwrap();
    let (uri, conf) = t.canonical.clone().expect("identified");
    assert_eq!(
        uri.to_url(),
        "https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69"
    );
    assert!(
        matches!(conf, pm_model::Confidence::Fingerprint { .. }),
        "{conf:?}"
    );
    assert_eq!(
        t.acoustid.as_deref(),
        Some("9ff43b6a-4f16-427c-93c2-92307ca505e0")
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    // The same audio under another name: no second lookup (cached by key).
    let b = music.join("copy.wav");
    std::fs::copy(&a, &b).unwrap();
    let tid2 = s
        .library
        .lock()
        .unwrap()
        .upsert_track(&untagged_track(b, "Bohemian Rhapsody", "Queen"))
        .unwrap();
    identify::run(&s, false).await.unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(s
        .library
        .lock()
        .unwrap()
        .get_track(tid2)
        .unwrap()
        .unwrap()
        .canonical
        .is_some());
    // A key entered in Settings: kept, used, and removing it goes back to the configured one
    let (st, b) = call(
        &app,
        "PUT",
        "/v1/library/identify/settings",
        Some(serde_json::json!({"acoustid_key": "own-key!"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{b}");
    let (st, b) = call(
        &app,
        "PUT",
        "/v1/library/identify/settings",
        Some(serde_json::json!({"acoustid_key": " OwnKey42 "})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(
        (b["acoustid"].clone(), b["acoustid_own"].clone()),
        (true.into(), true.into())
    );
    assert_eq!(s.acoustid.read().unwrap().api_key, "OwnKey42");
    let (_, b) = call(
        &app,
        "PUT",
        "/v1/library/identify/settings",
        Some(serde_json::json!({"acoustid_key": ""})),
    )
    .await;
    assert_eq!(b["acoustid_own"], false, "{b}");
    assert_eq!(s.acoustid.read().unwrap().api_key, "");
    // Without a key: a new file gets MusicBrainz search suggestions only.
    s.acoustid.write().unwrap().api_key.clear();
    let c = music.join("other.wav");
    tone_wav(&c, 9);
    let tid3 = s
        .library
        .lock()
        .unwrap()
        .upsert_track(&untagged_track(c, "Bohemian Rhapsody", "Queen"))
        .unwrap();
    identify::run(&s, false).await.unwrap();
    assert!(s
        .library
        .lock()
        .unwrap()
        .get_track(tid3)
        .unwrap()
        .unwrap()
        .canonical
        .is_none());
    let (_, sug) = call(&app, "GET", "/v1/library/suggestions", None).await;
    let sug = sug.as_array().unwrap();
    assert_eq!(sug.len(), 1, "only MusicBrainz scores >= 80: {sug:?}");
    assert_eq!(sug[0]["source"], "search");
    assert_eq!(sug[0]["track"]["id"], tid3.0);
    let (st, t3) = call(
        &app,
        "POST",
        &format!("/v1/library/suggestions/{}/accept", sug[0]["id"]),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{t3}");
    assert_eq!(t3["canonical"][1]["source"], "confirmed_by_user", "{t3}");
    let (_, st) = call(&app, "GET", "/v1/library/identify", None).await;
    assert_eq!(st["counts"]["identified"], 3, "{st}");
    assert_eq!(st["counts"]["suggested"], 0);
}

/// An identified local file (fingerprint, no IDs in its tags) gets its
/// versions from its MusicBrainz URL's own card: Pinakes treats MusicBrainz
/// links as hubs (pubky/pinakes#24).
#[tokio::test]
async fn identified_files_get_versions_from_their_musicbrainz_hub() {
    let mut card: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../pm-pubky/tests/fixtures/pinakes/music/detail-tidal-track-bohemian.json"
        ))
        .unwrap(),
    )
    .unwrap();
    // The hub card: the MusicBrainz URL with the group's service links.
    let all = card["details"]["also_on"].as_array().unwrap().clone();
    let mut links = vec![
        serde_json::json!({"uri": "https://tidal.com/track/534050211", "site": "TIDAL", "id": "a14a55b508a614dd965baefb8c4587cc"}),
    ];
    links.extend(all);
    card["uri"] = "https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69".into();
    card["site"] = "MusicBrainz".into();
    card["also_on"] = serde_json::json!(links.clone());
    card["details"]["also_on"] = serde_json::json!(links);
    let card = card.to_string();
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let s2 = seen.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = Router::new()
        .route(
            "/v1/resources/by-uri",
            get(
                move |Query(q): Query<std::collections::HashMap<String, String>>| {
                    let card = card.clone();
                    let seen = s2.clone();
                    async move {
                        seen.lock()
                            .unwrap()
                            .push(q.get("uri").cloned().unwrap_or_default());
                        ([(header::CONTENT_TYPE, "application/json")], card)
                    }
                },
            ),
        )
        .route(
            "/v1/search",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "application/json")],
                    r#"{"items":[],"cursor":null,"reach_truncated":false}"#,
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
    let tid = {
        let lib = s.library.lock().unwrap();
        let id = lib
            .upsert_track(&untagged_track(
                dir.path().join("11.flac"),
                "Bohemian Rhapsody",
                "Queen",
            ))
            .unwrap();
        let mbid = pm_model::Mbid::parse("b1a9c0e9-d987-4042-ae91-78d6a3267d69").unwrap();
        lib.set_identity(
            id.0,
            &pm_model::MusicUri::Recording { mbid },
            pm_model::Confidence::Fingerprint { score: 0.97 },
            Some("9ff43b6a-4f16-427c-93c2-92307ca505e0"),
        )
        .unwrap();
        id
    };
    let app = router(s.clone());
    let (st, v) = call(
        &app,
        "GET",
        &format!("/v1/library/tracks/{}/versions", tid.0),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        seen.lock().unwrap().first().map(String::as_str),
        Some("https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69"),
        "asks for the hub card by the canonical URL"
    );
    let sources: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["source"].as_str().unwrap())
        .collect();
    for want in ["tidal", "spotify", "deezer", "youtube"] {
        assert!(sources.contains(&want), "{want} missing: {v}");
    }
    let tidal = v
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["source"] == "tidal")
        .unwrap();
    assert_eq!(tidal["uri"], "tidal:track:534050211", "TIDAL plays: {tidal}");
}

/// "Write IDs into my files" is off by default; when the owner asks, a
/// fingerprinted FLAC gets its recording and release MBIDs in its own
/// tags, after a backup, and the scanner then reads them as tag IDs.
#[tokio::test]
async fn identified_ids_are_written_into_files_only_when_asked() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("music").join("11 bohemian.flac");
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../pm-library/tests/fixtures/ietf-60-mono-audio.flac"
        ),
        &f,
    )
    .unwrap();
    let s = state(dir.path(), None).await;
    let mbid = pm_model::Mbid::parse("b1a9c0e9-d987-4042-ae91-78d6a3267d69").unwrap();
    let rel = pm_model::Mbid::parse("1a5b6a49-4c4f-3d18-9e3d-1c4d1b0a7a3e").unwrap();
    let tid = {
        let lib = s.library.lock().unwrap();
        let id = lib
            .upsert_track(&untagged_track(f.clone(), "Bohemian Rhapsody", "Queen"))
            .unwrap();
        lib.set_identity(
            id.0,
            &pm_model::MusicUri::Recording { mbid },
            pm_model::Confidence::FingerprintRelease { score: 0.97 },
            Some("84eed5cd-5b1f-4f5c-8c92-2686d2877862"),
        )
        .unwrap();
        lib.set_identify_state(id.0, "identified", Some("fingerprint+release"))
            .unwrap();
        lib.set_identify_release(id.0, &rel).unwrap();
        id
    };
    let app = router(s.clone());
    let (_, st) = call(&app, "GET", "/v1/library/identify", None).await;
    assert_eq!(st["write_ids"], false, "off by default");
    assert!(pm_library::probe_and_read(&f).unwrap().1.recording_mbid.is_none());
    let (st, on) = call(
        &app,
        "PUT",
        "/v1/library/identify/settings",
        Some(serde_json::json!({"write_ids": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{on}");
    assert_eq!(on["write_ids"], true);
    let (st, r) = call(&app, "POST", "/v1/library/identify/write", None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["written"], 1, "{r}");
    let (_, tags) = pm_library::probe_and_read(&f).unwrap();
    assert_eq!(tags.recording_mbid, Some(mbid));
    assert_eq!(tags.release_mbid, Some(rel));
    let backups: Vec<_> = std::fs::read_dir(r["backup_dir"].as_str().unwrap())
        .unwrap()
        .collect();
    assert_eq!(backups.len(), 1, "one backup per write");
    let _ = tid;
}

/// Library tags are Pubky tags (STEWARD-UX-INSTANCES item 6): an unmatched file takes a tag
/// that waits and is published once matched; the files' genres become tags when switched on.
#[tokio::test]
async fn unmatched_files_take_tags_and_genres_publish_on_request() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let tid = {
        let lib = s.library.lock().unwrap();
        let mut tr = untagged_track(dir.path().join("01.flac"), "Trip to Serbia", "Sefa");
        tr.genres = vec!["Hardcore".into()];
        lib.upsert_track(&tr).unwrap().0
    };
    let app = router(s.clone());
    let (st, r) = call(
        &app,
        "PUT",
        "/v1/tags",
        Some(serde_json::json!({"track_id": tid, "label": "Frenchcore"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    let (_, t) = call(&app, "GET", &format!("/v1/tags?track_id={tid}"), None).await;
    assert_eq!(t["tags"][0]["label"], "frenchcore", "{t}");
    assert_eq!(t["tags"][0]["pending"], true);
    let (_, all) = call(&app, "GET", "/v1/tags", None).await;
    assert!(
        all["tags"].as_array().unwrap().is_empty(),
        "nothing to publish yet: {all}"
    );
    // Genres switched on: nothing yet (not matched)
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/node/settings",
        Some(serde_json::json!({"genre_tags": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    // Matched: the waiting tag becomes an ordinary one, and the genre follows at the next sync pass
    s.library
        .lock()
        .unwrap()
        .set_identity(
            tid,
            &pm_model::MusicUri::Isrc {
                isrc: pm_model::Isrc::parse("NLZ3R2400021").unwrap(),
            },
            pm_model::Confidence::ConfirmedByUser,
            None,
        )
        .unwrap();
    let (_, t) = call(&app, "GET", &format!("/v1/tags?track_id={tid}"), None).await;
    assert_eq!(t["uri"], "https://musicbrainz.org/isrc/NLZ3R2400021", "{t}");
    assert_eq!(t["tags"][0]["label"], "frenchcore");
    assert!(t["tags"][0].get("pending").is_none());
    crate::tags::before_sync(&s.library.lock().unwrap());
    let (_, t) = call(&app, "GET", &format!("/v1/tags?track_id={tid}"), None).await;
    let labels: Vec<&str> = t["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["label"].as_str().unwrap())
        .collect();
    assert!(labels.contains(&"hardcore"), "{t}");
}

/// Pubky search is music only, looks identifiers and pasted service pages up exactly, adds
/// concerts near the node's place, and stays within its request budget
/// (TASK-music-search-pinakes-1112).
#[tokio::test]
async fn pubky_search_is_music_only_knows_identifiers_and_counts_its_requests() {
    type Seen = Arc<std::sync::Mutex<Vec<(&'static str, std::collections::HashMap<String, String>)>>>;
    let seen: Seen = Default::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    const LINK: &str = "https://open.spotify.com/track/2Foc5Q5nqNiosCNqttzHof";
    let (s1, s2) = (seen.clone(), seen.clone());
    let mock = Router::new()
        .route(
            "/v1/search",
            get(
                move |Query(q): Query<std::collections::HashMap<String, String>>| {
                    s1.lock().unwrap().push(("search", q.clone()));
                    let card = |id: &str, type_: &str, name: &str| serde_json::json!({"id": id, "uri": format!("https://www.deezer.com/track/{id}"), "status": "ok", "type": type_, "name": name, "site": "Deezer"});
                    let res = if q.get("q").map(String::as_str) == Some("refused term") {
                        (
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({"error": "q is at most 200 characters"})),
                        )
                    } else if q.contains_key("boost") {
                        (
                            StatusCode::NOT_IMPLEMENTED,
                            Json(serde_json::json!({"error": "wot_2 is not supported: the Nexus this instance reads serves no reach set"})),
                        )
                    } else if q.get("key").map(String::as_str) == Some("isrc:USQX91300108") {
                        (
                            StatusCode::OK,
                            Json(serde_json::json!({"items": [{
                                "id": "r1", "uri": LINK, "status": "ok", "type": "MusicRecording", "name": "Get Lucky", "site": "Spotify",
                                "group": {"id": "g1", "size": 3},
                                "also_on": [
                                    {"uri": "https://www.deezer.com/track/66609426", "site": "Deezer"},
                                    {"uri": "https://open.spotify.com/track/other", "site": "Spotify"},
                                    {"uri": "https://tidal.com/track/20115557", "site": "TIDAL"},
                                    {"uri": "https://www.deezer.com/track/2", "site": "Deezer"}
                                ]
                            }]})),
                        )
                    } else if q.contains_key("near") {
                        (
                            StatusCode::OK,
                            Json(serde_json::json!({"items": [{"id": "e1", "uri": "https://www.songkick.com/concerts/1", "status": "ok", "type": "MusicEvent", "name": "Noah Kahan @ Hallenstadion", "site": "Songkick", "start": "2026-12-01T19:00:00Z"}]})),
                        )
                    } else if q.contains_key("q") {
                        (
                            StatusCode::OK,
                            Json(serde_json::json!({
                                "items": [card("1", "MusicRecording", "Around the World"), card("2", "MusicAlbum", "Homework")],
                                "facets": {"type": [
                                    {"type": "MusicRecording", "count": 7},
                                    {"type": "MusicAlbum", "count": 3},
                                    {"type": "WebPage", "count": 40},
                                    {"type": "MusicRelease", "count": 2}
                                ]}
                            })),
                        )
                    } else {
                        (StatusCode::OK, Json(serde_json::json!({"items": []})))
                    };
                    async move { res }
                },
            ),
        )
        .route(
            "/v1/resources/by-uri",
            get(
                move |Query(q): Query<std::collections::HashMap<String, String>>| {
                    s2.lock().unwrap().push(("by-uri", q.clone()));
                    async move {
                        Json(serde_json::json!({
                            "id": "r1", "uri": LINK, "status": "ok", "type": "MusicRecording", "name": "Get Lucky", "site": "Spotify",
                            "details": {"also_on": [
                                {"uri": "https://tidal.com/track/20115557", "site": "TIDAL"},
                                {"uri": "https://www.deezer.com/track/66609426", "site": "Deezer"}
                            ]}
                        }))
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
    let app = router(s.clone());
    let pubky_answer = |app: Router, uri: String| async move {
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
            .find(|v| v["source"] == "pubky")
            .expect("pubky answered")
    };
    let asked = || std::mem::take(&mut *seen.lock().unwrap());
    let types = pm_pubky::pinakes::music_types();

    // Words: the text query and the tag query, both over music types only; counts add up per kind
    let a = pubky_answer(app.clone(), "/v1/search?q=daft%20punk&in=pubky".into()).await;
    let reqs = asked();
    assert_eq!(reqs.len(), 2, "text and tags: {reqs:?}");
    for (_, q) in &reqs {
        assert_eq!(q.get("type"), Some(&types), "{q:?}");
        assert_eq!(q.get("collapse").map(String::as_str), Some("group"), "{q:?}");
        assert!(!q.contains_key("boost") && !q.contains_key("viewer"), "{q:?}");
    }
    let text = reqs
        .iter()
        .find(|(_, q)| q.contains_key("q"))
        .expect("a text query");
    assert_eq!(text.1.get("facet").map(String::as_str), Some("type"));
    assert!(reqs
        .iter()
        .any(|(_, q)| q.get("tags").map(String::as_str) == Some("daftpunk,daft-punk")));
    assert_eq!(a["status"], "ok", "{a}");
    assert_eq!(a["hits"][0]["title"], "Around the World", "{a}");
    assert!(
        a["hits"][0].get("exact").is_none(),
        "a text hit is not exact: {a}"
    );
    assert_eq!(a["counts"], serde_json::json!([["song", 7], ["album", 5]]), "{a}");

    // The same search again is the cached answer: nothing asked
    let a = pubky_answer(app.clone(), "/v1/search?q=daft%20punk&in=pubky".into()).await;
    assert_eq!(a["cached"], true, "{a}");
    assert!(asked().is_empty());

    // An ISRC: one exact lookup by key, no text or tag query; the hit is first, with Also on
    let a = pubky_answer(app.clone(), "/v1/search?q=usqx91300108&in=pubky".into()).await;
    let reqs = asked();
    assert_eq!(reqs.len(), 1, "{reqs:?}");
    assert_eq!(reqs[0].0, "search");
    assert_eq!(
        reqs[0].1.get("key").map(String::as_str),
        Some("isrc:USQX91300108")
    );
    assert_eq!(reqs[0].1.get("type"), Some(&types));
    assert!(!reqs[0].1.contains_key("q") && !reqs[0].1.contains_key("tags"));
    assert_eq!(a["hits"][0]["title"], "Get Lucky", "{a}");
    assert_eq!(a["hits"][0]["exact"], true, "{a}");
    assert_eq!(
        a["hits"][0]["also"],
        serde_json::json!(["Deezer", "TIDAL"]),
        "{a}"
    );
    assert_eq!(a["hits"][0]["kind"], "song");

    // A pasted Spotify page: one read by URI (never a resolve), first with Also on
    let a = pubky_answer(
        app.clone(),
        "/v1/search?q=https%3A%2F%2Fopen.spotify.com%2Ftrack%2F2Foc5Q5nqNiosCNqttzHof&in=pubky".into(),
    )
    .await;
    let reqs = asked();
    assert_eq!(reqs.len(), 1, "{reqs:?}");
    assert_eq!(reqs[0].0, "by-uri");
    assert_eq!(reqs[0].1.get("uri").map(String::as_str), Some(LINK));
    assert_eq!(a["hits"][0]["exact"], true, "{a}");
    assert_eq!(
        a["hits"][0]["also"],
        serde_json::json!(["TIDAL", "Deezer"]),
        "{a}"
    );
    assert_eq!(a["hits"][0]["page"], LINK);

    // The same page pasted in another case is another link (ids in paths differ by case): asked
    // again with its own spelling, not answered from the first one's cache
    let other = LINK.replace("2Foc5Q5nqNiosCNqttzHof", "2foc5q5nqniosCNQTTZHOF");
    pubky_answer(
        app.clone(),
        "/v1/search?q=https%3A%2F%2Fopen.spotify.com%2Ftrack%2F2foc5q5nqniosCNQTTZHOF&in=pubky".into(),
    )
    .await;
    let reqs = asked();
    assert_eq!(reqs.len(), 1, "{reqs:?}");
    assert_eq!(reqs[0].1.get("uri"), Some(&other), "{reqs:?}");
    // ... while words in another case are the cached answer
    let a = pubky_answer(app.clone(), "/v1/search?q=DAFT%20Punk&in=pubky".into()).await;
    assert_eq!(a["cached"], true, "{a}");
    assert!(asked().is_empty());

    // discover/search without a type asks for the cards it shows, no others
    call(&app, "GET", "/v1/discover/search?q=daft", None).await;
    let reqs = asked();
    let text = reqs
        .iter()
        .find(|(_, q)| q.contains_key("q"))
        .expect("a text query");
    assert_eq!(
        text.1.get("type").map(String::as_str),
        Some("MusicRecording,MusicAlbum,MusicGroup,MusicComposition,MusicRelease"),
        "{reqs:?}"
    );

    // A near miss (a barcode with a wrong check digit) is words: text and tags
    pubky_answer(app.clone(), "/v1/search?q=036000291453&in=pubky".into()).await;
    let reqs = asked();
    assert_eq!(reqs.len(), 2, "{reqs:?}");
    assert!(reqs.iter().all(|(_, q)| !q.contains_key("key")));

    // With the node's place saved, concerts near it come too: three requests, never more
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/home/location",
        Some(serde_json::json!({"lat": 47.37, "lon": 8.54, "city": "Zurich"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let a = pubky_answer(app.clone(), "/v1/search?q=noah%20kahan&in=pubky".into()).await;
    let reqs = asked();
    assert_eq!(reqs.len(), 3, "text, tags and concerts near: {reqs:?}");
    let near = reqs
        .iter()
        .find(|(_, q)| q.contains_key("near"))
        .expect("a concerts query");
    assert_eq!(near.1.get("type").map(String::as_str), Some("MusicEvent"));
    assert_eq!(near.1.get("near").map(String::as_str), Some("47.3700,8.5400"));
    assert_eq!(near.1.get("radius").map(String::as_str), Some("50000"));
    assert_eq!(near.1.get("from").map(String::as_str), Some("now"));
    assert_eq!(near.1.get("q").map(String::as_str), Some("noah kahan"));
    let hits = a["hits"].as_array().unwrap();
    assert_eq!(hits[0]["kind"], "concert", "concerts near come first: {a}");
    assert_eq!(hits[0]["title"], "Noah Kahan @ Hallenstadion");
    // An identifier asks for no concerts
    pubky_answer(app.clone(), "/v1/search?q=isrc%3AUSQX91300108&in=pubky".into()).await;
    assert_eq!(asked().len(), 1);

    // A 400 that is about the term, not the boost, is the search's error: no second request,
    // and the boost is still asked for afterwards
    let d = s.discovery().expect("discovery");
    let r = crate::search::text_search(d.pinakes(), "refused term", 20, Some("viewer1".into())).await;
    assert!(
        matches!(r, Err(pm_pubky::pinakes::PinakesError::BadRequest(_))),
        "{r:?}"
    );
    let reqs = asked();
    assert_eq!(reqs.len(), 1, "{reqs:?}");
    assert_eq!(reqs[0].1.get("boost").map(String::as_str), Some("wot_2"));
    // The network boost: an instance that refuses it is asked once more without, and not
    // again with it
    let r = crate::search::text_search(d.pinakes(), "boosted", 20, Some("viewer1".into()))
        .await
        .expect("the search still answers");
    assert_eq!(r.items.len(), 2);
    let reqs = asked();
    assert_eq!(reqs.len(), 2, "{reqs:?}");
    assert_eq!(reqs[0].1.get("boost").map(String::as_str), Some("wot_2"));
    assert_eq!(reqs[0].1.get("viewer").map(String::as_str), Some("viewer1"));
    assert!(!reqs[1].1.contains_key("boost") && !reqs[1].1.contains_key("viewer"));
    crate::search::text_search(d.pinakes(), "boosted again", 20, Some("viewer1".into()))
        .await
        .unwrap();
    let reqs = asked();
    assert_eq!(reqs.len(), 1, "remembered: {reqs:?}");
    assert!(!reqs[0].1.contains_key("boost"));
}

/// One label on every identified track; the sync publishes each as a
/// v1 PubkyAppTag on its canonical MusicBrainz URL, which Pinakes joins.
#[tokio::test]
async fn bulk_tagging_covers_identified_tracks_only() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("music")).unwrap();
    write_wav(&dir.path().join("music/one.wav"), 4410); // no identity
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
    {
        let lib = s.library.lock().unwrap();
        let mbid = pm_model::Mbid::parse("b1a9c0e9-d987-4042-ae91-78d6a3267d69").unwrap();
        let t = pm_model::Track {
            id: None,
            title: "Bohemian Rhapsody".into(),
            artists: vec![],
            album: None,
            track_no: None,
            disc_no: None,
            duration_ms: None,
            file: pm_model::AudioFile {
                path: dir.path().join("bo.flac"),
                size_bytes: 1,
                mtime_unix: 0,
                container: "flac".into(),
                format: pm_model::AudioFormat::pcm(44_100, 16, 2),
                duration_ms: None,
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
        lib.upsert_track(&t).unwrap();
    }
    let (st, r) = call(
        &app,
        "POST",
        "/v1/tags/bulk",
        Some(serde_json::json!({"label": " Library "})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["label"], "library");
    assert_eq!(r["tagged"], 1);
    assert_eq!(r["skipped_no_identity"], 1);
    let (_, r) = call(
        &app,
        "POST",
        "/v1/tags/bulk",
        Some(serde_json::json!({"label": "library"})),
    )
    .await;
    assert_eq!(r["tagged"], 0);
    assert_eq!(r["already"], 1);
    let (_, tags) = call(&app, "GET", "/v1/tags", None).await;
    assert_eq!(
        tags["tags"][0]["uri"],
        "https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69"
    );
    assert_eq!(tags["tags"][0]["synced"], false);
    let (st, e) = call(
        &app,
        "POST",
        "/v1/tags/bulk",
        Some(serde_json::json!({"label": ""})),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
}

/// Library matching (F1): an album is matched by title, artist and song count, a tagged
/// release by its id, and each artist by its albums' credits; an artist with nothing but a
/// name that several MusicBrainz artists share stays unmatched. Pages carry the id and where
/// it came from, and a second pass asks nothing.
#[tokio::test]
async fn library_artists_and_albums_are_matched_without_trusting_a_name_alone() {
    let _one = MATCH_PASSES.lock().await;
    const RG_TAXI: &str = "0b5d3e2b-4b0c-4d5f-9d4b-1e9a2b3c4d5e";
    const TAXI: &str = "2d7f5a4d-6d2e-4f7b-9f6d-3a1c4d5e6f70";
    const RELEASE: &str = "4f9b7c6f-8f4a-4b9d-9b8f-5c3e6f708192";
    const RG_SOLO: &str = "5a0c8d7a-9a5b-4c0e-8c9a-6d4f708192a3";
    const SOLO: &str = "6b1d9e8b-0b6c-4d1f-9d0b-7e5a8192a3b4";
    const SMITH_1: &str = "7c2e0f9c-1c7d-4e2a-8e1c-8f6b92a3b4c5";
    const SMITH_2: &str = "8d3f1a0d-2d8e-4f3b-9f2d-9a7ca3b4c5d6";
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let credit = |name: &str, id: &str| serde_json::json!([{"name": name, "joinphrase": "", "artist": {"id": id, "name": name}}]);
    let (c1, c2, c3) = (calls.clone(), calls.clone(), calls.clone());
    let search = {
        let taxi = credit("7 Dollar Taxi", TAXI);
        let smith = credit("John Smith", SMITH_1);
        move |Query(q): Query<std::collections::HashMap<String, String>>| {
            c1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let query = q.get("query").cloned().unwrap_or_default();
            // Only the title without its edition finds it, as on MusicBrainz
            let releases = if query.starts_with("release:\"Zahltag\" AND") {
                serde_json::json!([
                    {"title": "Zahltag", "track-count": 5, "release-group": {"id": RG_SOLO}, "artist-credit": taxi},
                    {"title": "Zahltag", "track-count": 2, "release-group": {"id": RG_TAXI}, "artist-credit": taxi}
                ])
            } else {
                // John Smith's "Hits" has more songs than the library's copy
                serde_json::json!([{"title": "Hits", "track-count": 9, "release-group": {"id": RG_TAXI}, "artist-credit": smith}])
            };
            async move { Json(serde_json::json!({"releases": releases})) }
        }
    };
    let mock = Router::new()
        .route("/ws/2/release", get(search))
        .route(
            &format!("/ws/2/release/{RELEASE}"),
            get(move || {
                c2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move {
                    Json(serde_json::json!({"id": RELEASE, "release-group": {"id": RG_SOLO},
                        "artist-credit": [{"name": "Solo", "joinphrase": "", "artist": {"id": SOLO, "name": "Solo"}}]}))
                }
            }),
        )
        .route(
            "/ws/2/recording",
            get(move || {
                c3.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // Two artists called John Smith with a song of that title and length
                async move {
                    Json(serde_json::json!({"recordings": [
                        {"title": "Morning", "length": 180_000, "artist-credit": [{"name": "John Smith", "artist": {"id": SMITH_1, "name": "John Smith"}}]},
                        {"title": "Morning", "length": 181_000, "artist-credit": [{"name": "John Smith", "artist": {"id": SMITH_2, "name": "John Smith"}}]}
                    ]}))
                }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("tests@example.org").unwrap();
    *s.discovery.write().unwrap() = None;
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    {
        let mut b = s.info_bases.write().unwrap();
        b.musicbrainz = "http://127.0.0.1:9".into();
        b.fanart_key = None;
        b.audiodb = "http://127.0.0.1:9".into();
    }
    let song = |file: &str, title: &str, artist: &str, album: Option<(&str, Option<&str>)>, ms: u64| {
        let mut t = untagged_track(dir.path().join(file), title, artist);
        t.duration_ms = Some(ms);
        t.album = album.map(|(name, release)| pm_model::Album {
            id: None,
            title: name.into(),
            album_artist: Some(artist.into()),
            release_mbid: release.map(|r| pm_model::Mbid::parse(r).unwrap()),
            release_group_mbid: None,
            year: None,
            upc: None,
            total_discs: None,
        });
        t
    };
    {
        let lib = s.library.lock().unwrap();
        for t in [
            song(
                "1.flac",
                "Taxi",
                "7 Dollar Taxi",
                Some(("Zahltag (Deluxe)", None)),
                200_000,
            ),
            song(
                "2.flac",
                "Zahltag",
                "7 Dollar Taxi",
                Some(("Zahltag (Deluxe)", None)),
                210_000,
            ),
            song(
                "3.flac",
                "Alone",
                "Solo",
                Some(("Solo Album", Some(RELEASE))),
                190_000,
            ),
            song("4.flac", "Morning", "John Smith", Some(("Hits", None)), 180_500),
        ] {
            lib.upsert_track(&t).unwrap();
        }
    }
    let report = library_match::pass(&s, std::time::Duration::ZERO).await;
    assert_eq!(report.failed, 0, "{report:?}");
    // Three albums, then John Smith's song (his album didn't match)
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 4, "{report:?}");
    let app = router(s.clone());
    let (_, m) = call(&app, "GET", "/v1/library/matches", None).await;
    assert_eq!(m["albums"]["lookup"], 1, "{m}");
    assert_eq!(m["albums"]["tags"], 1, "{m}");
    assert_eq!(m["albums"]["unmatched"], 1, "{m}");
    assert_eq!(m["artists"]["lookup"], 2, "{m}");
    assert_eq!(m["artists"]["unmatched"], 1, "{m}");
    assert_eq!(m["artists"]["pending"], 0, "{m}");
    let artists = s.library.lock().unwrap().list_artists().unwrap();
    let id_of = |name: &str| artists.iter().find(|a| a.name == name).unwrap().id;
    let (_, p) = call(
        &app,
        "GET",
        &format!("/v1/item/artist/lib-{}", id_of("7 Dollar Taxi")),
        None,
    )
    .await;
    assert_eq!(p["mb_match"]["mbid"], TAXI, "{p}");
    assert_eq!(p["mb_match"]["source"], "lookup");
    assert_eq!(p["uri"], format!("https://musicbrainz.org/artist/{TAXI}"));
    let (_, p) = call(
        &app,
        "GET",
        &format!("/v1/item/artist/lib-{}", id_of("Solo")),
        None,
    )
    .await;
    assert_eq!(p["mb_match"]["mbid"], SOLO, "{p}");
    let (_, p) = call(
        &app,
        "GET",
        &format!("/v1/item/artist/lib-{}", id_of("John Smith")),
        None,
    )
    .await;
    assert!(
        p["mb_match"].is_null(),
        "a name and a song two artists share: {p}"
    );
    let albums = s.library.lock().unwrap().list_albums(None, 100).unwrap();
    let album_of = |t: &str| albums.iter().find(|a| a.title == t).unwrap().id.unwrap();
    let (_, p) = call(
        &app,
        "GET",
        &format!("/v1/item/album/lib-{}", album_of("Zahltag (Deluxe)")),
        None,
    )
    .await;
    assert_eq!(
        p["mb_match"],
        serde_json::json!({"mbid": RG_TAXI, "source": "lookup"}),
        "{p}"
    );
    assert_eq!(
        p["uri"],
        format!("https://musicbrainz.org/release-group/{RG_TAXI}")
    );
    let (_, p) = call(
        &app,
        "GET",
        &format!("/v1/item/album/lib-{}", album_of("Solo Album")),
        None,
    )
    .await;
    assert_eq!(
        p["mb_match"],
        serde_json::json!({"mbid": RG_SOLO, "source": "tags"}),
        "{p}"
    );
    // The page stays on the tagged release, which its heart and tags were saved on
    assert_eq!(p["uri"], format!("https://musicbrainz.org/release/{RELEASE}"));
    // Answered, sure or not: nothing is asked again
    let again = library_match::pass(&s, std::time::Duration::ZERO).await;
    assert_eq!(again, library_match::PassReport::default());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 4);
}

/// A failed album lookup doesn't hold back another artist in the same pass: an artist with
/// nothing to ask (no album, no song length) is still answered "not sure".
#[tokio::test]
async fn a_failed_album_lookup_does_not_skip_other_artists() {
    let _one = MATCH_PASSES.lock().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = Router::new().route(
        "/ws/2/release",
        get(|| async { StatusCode::INTERNAL_SERVER_ERROR }),
    );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("tests@example.org").unwrap();
    *s.discovery.write().unwrap() = None;
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    {
        let lib = s.library.lock().unwrap();
        let mut on_album = untagged_track(dir.path().join("1.flac"), "One", "Band");
        on_album.album = Some(pm_model::Album {
            id: None,
            title: "Album".into(),
            album_artist: Some("Band".into()),
            release_mbid: None,
            release_group_mbid: None,
            year: None,
            upc: None,
            total_discs: None,
        });
        lib.upsert_track(&on_album).unwrap();
        let mut loose = untagged_track(dir.path().join("2.flac"), "Two", "Loner");
        loose.duration_ms = None;
        loose.file.duration_ms = None;
        lib.upsert_track(&loose).unwrap();
    }
    let report = library_match::pass(&s, std::time::Duration::ZERO).await;
    assert_eq!(report.failed, 1, "{report:?}");
    let c = s.library.lock().unwrap().match_counts().unwrap();
    assert_eq!(c.artists.unmatched, 1, "Loner is answered: {c:?}");
    assert_eq!(c.albums.pending, 1, "the album is asked again next pass: {c:?}");
}

/// An album with no MusicBrainz id yet can't take tags or a review: the reads say so
/// (`needs_match`) instead of a 422; an unmatched song's tags still wait for a match.
#[tokio::test]
async fn unmatched_items_say_they_need_a_match() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let (tid, album) = {
        let lib = s.library.lock().unwrap();
        let mut tr = untagged_track(dir.path().join("a.flac"), "Song", "Someone");
        tr.album = Some(pm_model::Album {
            id: None,
            title: "No Id".into(),
            album_artist: Some("Someone".into()),
            release_mbid: None,
            release_group_mbid: None,
            year: None,
            upc: None,
            total_discs: None,
        });
        let tid = lib.upsert_track(&tr).unwrap();
        let t = lib.get_track(tid).unwrap().unwrap();
        (tid.0, t.album.unwrap().id.unwrap())
    };
    let app = router(s.clone());
    let (st, tg) = call(&app, "GET", &format!("/v1/tags?album_id={album}"), None).await;
    assert_eq!(
        (st, tg["needs_match"].as_bool()),
        (StatusCode::OK, Some(true)),
        "{tg}"
    );
    let (st, rv) = call(&app, "GET", &format!("/v1/reviews?album_id={album}"), None).await;
    assert_eq!(
        (st, rv["needs_match"].as_bool()),
        (StatusCode::OK, Some(true)),
        "{rv}"
    );
    let (st, tg) = call(&app, "GET", &format!("/v1/tags?track_id={tid}"), None).await;
    assert_eq!(st, StatusCode::OK, "{tg}");
    assert!(
        tg.get("needs_match").is_none(),
        "an unmatched song takes tags that wait: {tg}"
    );
}

/// A key AcoustID refuses (code 4, "invalid API key"): saving it is refused with a plain
/// message; a run with it stops fingerprinting after the first refusal (no more decoding),
/// carries on with the search by tags, and the status says why.
#[tokio::test]
async fn an_acoustid_key_acoustid_refuses_is_refused_and_stops_fingerprinting() {
    let lookups = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = {
        let lookups = lookups.clone();
        Router::new().route(
            "/v2/lookup",
            post(move |body: String| {
                lookups.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move {
                    if body.contains("client=GoodKey") && body.contains("fingerprint=AQAAAA") {
                        // As the real service: the key is taken, the dummy fingerprint isn't
                        Json(serde_json::json!({"status": "error",
                            "error": {"code": 3, "message": "invalid fingerprint"}}))
                    } else if body.contains("client=GoodKey") {
                        Json(serde_json::json!({"status": "ok", "results": []}))
                    } else {
                        Json(serde_json::json!({"status": "error",
                            "error": {"code": 4, "message": "invalid API key"}}))
                    }
                }
            }),
        )
    };
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    let s = state(dir.path(), None).await;
    *s.acoustid.write().unwrap() = identify::AcoustIdSettings {
        api_key: String::new(),
        base_url: format!("http://127.0.0.1:{port}"),
        configured: String::new(),
    };
    let app = router(s.clone());
    let key = |k: &str| Some(serde_json::json!({ "acoustid_key": k }));
    // Check (the Identify card's button): nothing saved, each case in its own words
    let (st, r) = call(
        &app,
        "POST",
        "/v1/library/identify/key/check",
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert!(
        r["ok"] == false && r["detail"].as_str().unwrap().starts_with("Key missing"),
        "{r}"
    );
    let (_, r) = call(&app, "POST", "/v1/library/identify/key/check", key("BadKey1")).await;
    assert!(
        r["ok"] == false
            && r["detail"]
                .as_str()
                .unwrap()
                .contains("refused this key (invalid API key)"),
        "{r}"
    );
    let (_, r) = call(&app, "POST", "/v1/library/identify/key/check", key("GoodKey")).await;
    assert!(
        r["ok"] == true && r["detail"].as_str().unwrap().starts_with("OK"),
        "{r}"
    );
    let (_, st0) = call(&app, "GET", "/v1/library/identify", None).await;
    assert_eq!(
        (st0["acoustid"].as_bool(), st0["acoustid_state"].as_str()),
        (Some(false), Some("missing")),
        "a check saves nothing: {st0}"
    );
    // Settings keeps no copy: the Service apps list points at the Identify card
    let (_, apps) = call(&app, "GET", "/v1/node/apps", None).await;
    let ac = apps
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "acoustid")
        .unwrap();
    assert!(
        ac.get("set").is_none() && ac["fields"].as_array().is_none_or(|f| f.is_empty()),
        "{ac}"
    );
    assert_eq!(ac["page"], "library:node", "{ac}");
    let (st, b) = call(&app, "PUT", "/v1/library/identify/settings", key("BadKey1")).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{b}");
    assert!(
        b["error"]
            .as_str()
            .unwrap()
            .contains("AcoustID refused this key (invalid API key)"),
        "{b}"
    );
    assert_eq!(s.acoustid.read().unwrap().api_key, "", "not saved");
    let (st, b) = call(&app, "PUT", "/v1/library/identify/settings", key("GoodKey")).await;
    assert_eq!(
        (st, b["acoustid_own"].clone()),
        (StatusCode::OK, true.into()),
        "{b}"
    );
    assert_eq!(
        lookups.load(std::sync::atomic::Ordering::SeqCst),
        4,
        "one lookup per Check and per key saved"
    );

    // A key that stopped working (or a configured one AcoustID refuses), three songs
    s.acoustid.write().unwrap().api_key = "BadKey1".into();
    let mut paths = Vec::new();
    for i in 1..=3 {
        let path = music.join(format!("{i:02} song.wav"));
        tone_wav(&path, 10 + i);
        let t = untagged_track(path.clone(), &format!("Song {i}"), "Band");
        s.library.lock().unwrap().upsert_track(&t).unwrap();
        paths.push(path);
    }
    lookups.store(0, std::sync::atomic::Ordering::SeqCst);
    identify::run(&s, false).await.unwrap();
    assert_eq!(
        lookups.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "one refusal, then no more lookups"
    );
    // The songs decode in parallel once AcoustID took the key; until then one at a time
    let decoded = paths
        .iter()
        .filter(|p| {
            let k = Library::file_key(p).unwrap();
            s.library
                .lock()
                .unwrap()
                .cached_fingerprint(&k)
                .unwrap()
                .is_some()
        })
        .count();
    assert_eq!(decoded, 1, "only the song that found out was decoded");
    let (_, st) = call(&app, "GET", "/v1/library/identify", None).await;
    assert_eq!(st["progress"]["acoustid_refused"], "invalid API key", "{st}");
    assert_eq!(st["progress"]["errors"], 0, "the songs go on to the search: {st}");
    assert_eq!(st["progress"]["unknown"], 3, "{st}");
    // A new key clears it
    let (_, b) = call(&app, "PUT", "/v1/library/identify/settings", key("GoodKey")).await;
    assert!(b["progress"]["acoustid_refused"].is_null(), "{b}");
}

/// Identify while a scan holds the library's write lock longer than the busy timeout: the
/// first song's state write is turned away, tried again once the scan is done, and the run
/// carries on; no song ends in "error" because of the lock, and every song is checked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn identify_waits_for_a_busy_library_and_checks_every_song() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    let s = state(dir.path(), None).await;
    let mut ids = Vec::new();
    for i in 1..=3 {
        let path = music.join(format!("{i:02} song.wav"));
        tone_wav(&path, 10 + i);
        // Looked up before (an empty answer): the job's first write is the song's state
        let key = Library::file_key(&path).unwrap();
        let t = untagged_track(path, &format!("Song {i}"), "Band");
        let lib = s.library.lock().unwrap();
        lib.save_fingerprint(&key, 10 + i as u64, "AQfake").unwrap();
        lib.save_lookup(&key, r#"{"status":"ok","results":[]}"#).unwrap();
        ids.push(lib.upsert_track(&t).unwrap().0);
    }
    // A scan's connection takes the write lock and keeps it until the job's first write has
    // been turned away; the scan ends a while after it lets go (its last batch is done, the
    // rest is reading). Let go on that event, not after a fixed time: SQLite's busy wait
    // counts tries, so on a loaded machine it outlasted a fixed hold and the write went
    // through without waiting for the scan (flaky under a full test run). The job holds the
    // library through its busy wait and lets go when it is refused.
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let after = std::time::Duration::from_millis(1500);
    assert!(s.begin_scan());
    // The scan ends whatever happens in the thread below: a failed assertion there fails the
    // test instead of leaving the run waiting for a scan that never ends
    struct EndScan(Arc<NodeState>);
    impl Drop for EndScan {
        fn drop(&mut self) {
            self.0.end_scan();
        }
    }
    let other = {
        let db = dir.path().join("lib.sqlite");
        let s = s.clone();
        std::thread::spawn(move || {
            let _end = EndScan(s.clone());
            let other = Library::open(&db).unwrap();
            other.begin_batch().unwrap();
            locked_tx.send(()).unwrap();
            // Held for half the busy timeout without a break: only a busy wait does that
            let tick = std::time::Duration::from_millis(10);
            let give_up = std::time::Instant::now() + 6 * pm_library::db::BUSY_TIMEOUT;
            let mut held_since = None;
            loop {
                assert!(
                    std::time::Instant::now() < give_up,
                    "the job never waited for the lock"
                );
                let now = std::time::Instant::now();
                if s.library.try_lock().is_ok() {
                    if held_since
                        .is_some_and(|t: std::time::Instant| now - t >= pm_library::db::BUSY_TIMEOUT / 2)
                    {
                        break;
                    }
                    held_since = None;
                } else {
                    held_since.get_or_insert(now);
                }
                std::thread::sleep(tick);
            }
            other.commit_batch().unwrap();
            std::thread::sleep(after);
            s.end_scan();
            std::time::Instant::now()
        })
    };
    locked_rx.recv().unwrap();
    tokio::time::timeout(10 * pm_library::db::BUSY_TIMEOUT, identify::run(&s, false))
        .await
        .expect("the run ends")
        .unwrap();
    let finished = std::time::Instant::now();
    let scan_ended = other.join().expect("the scan's thread ran to its end");
    assert!(
        finished >= scan_ended,
        "the retry waits for the scan to end, not for the lock alone"
    );
    let p = s.identify.lock().unwrap().clone();
    assert_eq!((p.done, p.unknown, p.errors), (3, 3, 0), "{p:?}");
    assert!(s
        .library
        .lock()
        .unwrap()
        .tracks_to_identify(false)
        .unwrap()
        .is_empty());
}

/// Identify library as a ladder (owner, 2026-10-30: too slow on the NUC): a folder that is an
/// album is identified from one MusicBrainz search and one release lookup, without decoding
/// a file; only what doesn't fit (a bonus track, a compilation folder of mixed artists) is
/// fingerprinted. A fingerprint an earlier run looked up agrees (higher confidence) or names
/// another song (both go to the owner).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn identify_matches_album_folders_and_fingerprints_only_the_rest() {
    const REL_A: &str = "1a5b6a49-4c4f-4d18-9e3d-1c4d1b0a7a3e";
    const REL_O: &str = "2b6c7b5a-5d5a-4e29-8f4e-2d5e2c1b8b4f";
    const ELSE: &str = "3c7d8c6b-6e6b-4f3a-9a5f-3e6f3d2c9c5a";
    const REL_V: &str = "4d8e9d7c-7f7c-4a4b-8b6a-4f7a4e3d0d6b";
    let rec = |i: u32| format!("00000000-0000-4000-8000-{i:012}");
    let counter = || Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (searches, lookups, recordings, acoustid) = (counter(), counter(), counter(), counter());
    let n = |c: &Arc<std::sync::atomic::AtomicUsize>| c.load(std::sync::atomic::Ordering::SeqCst);
    // A release of `count` tracks "Song i" (Other: "Take i"), each 200 + i seconds
    let tracklist = move |id: &str, title: &str, word: &str, count: u32, base: u32| {
        serde_json::json!({"id": id, "title": title, "media": [{"position": 1, "tracks": (1..=count).map(|i| serde_json::json!({
            "position": i, "title": format!("{word} {i}"), "length": (200 + i) * 1000,
            "recording": {"id": rec(base + i)}
        })).collect::<Vec<_>>()}]})
    };
    let (a, o, v) = (
        tracklist(REL_A, "Album", "Song", 4, 0),
        tracklist(REL_O, "Other", "Take", 3, 10),
        tracklist(REL_V, "Summer Mix", "Anthem", 2, 20),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = {
        let (searches, lookups, recordings, acoustid) = (
            searches.clone(),
            lookups.clone(),
            recordings.clone(),
            acoustid.clone(),
        );
        Router::new()
            .route(
                "/ws/2/release",
                get(
                    move |Query(q): Query<std::collections::HashMap<String, String>>| {
                        searches.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let query = q.get("query").cloned().unwrap_or_default();
                        let (id, title, count, credit) = if query
                            .starts_with("release:\"Album\" AND artist:\"Band\"")
                        {
                            (REL_A, "Album", 4, "Band")
                        } else if query.starts_with("release:\"Other\" AND artist:\"Band\"") {
                            (REL_O, "Other", 3, "Band")
                        } else if query.starts_with("release:\"Summer Mix\" AND artist:\"Various Artists\"") {
                            (REL_V, "Summer Mix", 2, "Various Artists")
                        } else {
                            panic!("unexpected release search {query}")
                        };
                        async move {
                            Json(
                                serde_json::json!({"releases": [{"id": id, "score": 100, "title": title,
                            "track-count": count, "artist-credit": [{"name": credit}]}]}),
                            )
                        }
                    },
                ),
            )
            .route(
                "/ws/2/release/{id}",
                get(move |Path(id): Path<String>| {
                    lookups.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let r = match id.as_str() {
                        REL_A => a.clone(),
                        REL_O => o.clone(),
                        _ => v.clone(),
                    };
                    async move { Json(r) }
                }),
            )
            .route(
                "/ws/2/recording",
                get(move || {
                    recordings.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    async { Json(serde_json::json!({"recordings": []})) }
                }),
            )
            .route(
                "/v2/lookup",
                post(move || {
                    acoustid.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    async { Json(serde_json::json!({"status": "ok", "results": []})) }
                }),
            )
    };
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("tests@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    *s.acoustid.write().unwrap() = identify::AcoustIdSettings {
        api_key: "test-key".into(),
        base_url: format!("http://127.0.0.1:{port}"),
        configured: String::new(),
    };
    // Every file its own audio (the fingerprint cache is by content)
    let mut secs = 1;
    let mut add = |folder: &str, title: &str, artist: &str, album: (&str, Option<&str>), no: u32, ms: u64| {
        let dir = dir.path().join("music").join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{no:02} {title}.wav"));
        tone_wav(&path, secs);
        secs += 1;
        let mut t = untagged_track(path.clone(), title, artist);
        t.track_no = Some(no);
        t.disc_no = None;
        t.duration_ms = Some(ms);
        t.album = Some(pm_model::Album {
            id: None,
            title: album.0.into(),
            album_artist: album.1.map(str::to_string),
            release_mbid: None,
            release_group_mbid: None,
            year: None,
            upc: None,
            total_discs: None,
        });
        let id = s.library.lock().unwrap().upsert_track(&t).unwrap();
        (id.0, path)
    };
    // The album, ripped a second longer than MusicBrainz says
    let album: Vec<(i64, std::path::PathBuf)> = (1..=4)
        .map(|i| {
            add(
                "Band/Album",
                &format!("Song {i}"),
                "Band",
                ("Album (Remastered)", Some("Band")),
                i,
                (201 + i as u64) * 1000,
            )
        })
        .collect();
    // Another album with a bonus track MusicBrainz's release hasn't
    let mut other: Vec<(i64, std::path::PathBuf)> = (1..=3)
        .map(|i| {
            add(
                "Band/Other",
                &format!("Take {i}"),
                "Band",
                ("Other", Some("Band")),
                i,
                (200 + i as u64) * 1000,
            )
        })
        .collect();
    other.push(add(
        "Band/Other",
        "Demo",
        "Band",
        ("Other", Some("Band")),
        4,
        150_000,
    ));
    // A compilation folder of mixed artists, as the scanner stores it ("Various Artists"); a
    // compilation of that name on MusicBrainz has other songs
    let mix: Vec<(i64, std::path::PathBuf)> = (1..=2)
        .map(|i| {
            add(
                "Mix",
                &format!("Hit {i}"),
                &format!("Artist {i}"),
                ("Summer Mix", Some("Various Artists")),
                i,
                180_000,
            )
        })
        .collect();
    // An earlier run's fingerprints: song 1 sounds like another song, song 2 like itself
    let cached = |path: &std::path::Path, recording: &str, title: &str| {
        let key = Library::file_key(path).unwrap();
        let lib = s.library.lock().unwrap();
        lib.save_fingerprint(&key, 3, "AQfake").unwrap();
        let answer = serde_json::json!({"status": "ok", "results": [{"id": "9ff43b6a-4f16-427c-93c2-92307ca505e0",
            "score": 0.95, "recordings": [{"id": recording, "title": title}]}]});
        lib.save_lookup(&key, &answer.to_string()).unwrap();
    };
    cached(&album[0].1, ELSE, "Something Else");
    cached(&album[1].1, &rec(2), "Song 2");
    // Started and watched as the page does: the status answers all along
    let app = router(s.clone());
    let (st, _) = call(&app, "POST", "/v1/library/identify", Some(serde_json::json!({}))).await;
    assert_eq!(st, StatusCode::OK);
    let watch = async {
        loop {
            let (_, st) = call(&app, "GET", "/v1/library/identify", None).await;
            if st["progress"]["running"] == false {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(60), watch)
        .await
        .expect("the status keeps answering and the run ends");

    // One search and one lookup per album (the mix too); no fingerprint for a song its
    // folder settled
    assert_eq!((n(&searches), n(&lookups)), (3, 3));
    assert_eq!(n(&acoustid), 3, "the bonus track and the compilation's two songs");
    // The search by tags for what nothing found (with the length, then without)
    assert_eq!(n(&recordings), 6);
    let track = |id: i64| {
        s.library
            .lock()
            .unwrap()
            .get_track(pm_model::TrackId(id))
            .unwrap()
            .unwrap()
    };
    for (i, (id, _)) in album.iter().enumerate().skip(1) {
        let (uri, conf) = track(*id).canonical.expect("identified by its folder");
        assert_eq!(
            uri.to_url(),
            format!("https://musicbrainz.org/recording/{}", rec(i as u32 + 1))
        );
        let pm_model::Confidence::AlbumFit { score } = conf else {
            panic!("{conf:?}")
        };
        if i == 1 {
            assert!(score > 0.98, "the fingerprint agrees: {score}");
        } else {
            assert!((score - 0.95).abs() < 1e-6, "{score}");
        }
    }
    for (i, (id, _)) in other.iter().take(3).enumerate() {
        let (uri, _) = track(*id).canonical.expect("identified by its folder");
        assert_eq!(
            uri.to_url(),
            format!("https://musicbrainz.org/recording/{}", rec(11 + i as u32))
        );
    }
    // The bonus track and the mix: fingerprinted, searched for by their tags, nothing found
    // (looked at, no suggestion: only a retry looks again)
    let ids = |retry: bool| -> Vec<i64> {
        let lib = s.library.lock().unwrap();
        let todo = lib.tracks_to_identify(retry).unwrap();
        todo.iter().filter_map(|t| t.id.map(|i| i.0)).collect()
    };
    let (again, new) = (ids(true), ids(false));
    for (id, _) in other.iter().skip(3).chain(&mix) {
        assert!(track(*id).canonical.is_none());
        assert!(again.contains(id) && !new.contains(id), "{id} ends unknown");
    }
    // The folder and the fingerprint disagree on song 1: the owner decides
    assert!(track(album[0].0).canonical.is_none());
    let (_, sug) = call(&app, "GET", "/v1/library/suggestions", None).await;
    let mut sources: Vec<(String, String)> = sug
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x["source"].as_str().unwrap().to_string(),
                x["recording_mbid"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    sources.sort();
    assert_eq!(
        sources,
        vec![("album".into(), rec(1)), ("fingerprint".into(), ELSE.into())]
    );
    assert!(sug
        .as_array()
        .unwrap()
        .iter()
        .all(|x| x["track_id"] == album[0].0));
    let (_, st) = call(&app, "GET", "/v1/library/identify", None).await;
    assert_eq!(st["counts"]["by"]["album"], 6, "{st}");
    assert_eq!(st["progress"]["identified_by"]["album_fit"], 6, "{st}");
    assert_eq!(st["progress"]["done"], 10, "{st}");
    assert_eq!(st["progress"]["unknown"], 3, "{st}");
    assert_eq!(st["progress"]["suggested"], 1, "{st}");
}

/// A key saved while a run looks songs up with the old one (#205 review): the run's refusal
/// of the old key is not reported against the new key, which AcoustID took when it was saved,
/// and still stops the run's fingerprinting (it keeps the old key's client).
#[tokio::test]
async fn a_refusal_of_the_old_key_is_not_reported_for_a_key_saved_meanwhile() {
    let node: Arc<std::sync::OnceLock<Arc<NodeState>>> = Default::default();
    let lookups = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = {
        let node = node.clone();
        let lookups = lookups.clone();
        Router::new().route(
            "/v2/lookup",
            post(move |body: String| {
                let node = node.clone();
                lookups.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move {
                    if body.contains("client=GoodKey") {
                        return Json(serde_json::json!({"status": "ok", "results": []}));
                    }
                    // The owner saves a new key while the old one's lookup is under way
                    let s = node.get().unwrap();
                    s.acoustid.write().unwrap().api_key = "GoodKey".into();
                    s.identify.lock().unwrap().acoustid_refused = None;
                    Json(serde_json::json!({"status": "error",
                        "error": {"code": 4, "message": "invalid API key"}}))
                }
            }),
        )
    };
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    let s = state(dir.path(), None).await;
    node.set(s.clone()).ok().unwrap();
    *s.acoustid.write().unwrap() = identify::AcoustIdSettings {
        api_key: "OldKey1".into(),
        base_url: format!("http://127.0.0.1:{port}"),
        configured: String::new(),
    };
    for i in 1..=3 {
        let path = music.join(format!("{i:02} song.wav"));
        tone_wav(&path, 10 + i);
        let t = untagged_track(path, &format!("Song {i}"), "Band");
        s.library.lock().unwrap().upsert_track(&t).unwrap();
    }
    identify::run(&s, false).await.unwrap();
    let p = s.identify.lock().unwrap().clone();
    assert_eq!(p.acoustid_refused, None, "{p:?}");
    assert_eq!(s.acoustid.read().unwrap().api_key, "GoodKey");
    assert_eq!(
        lookups.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the old key's refusal stops the run's lookups all the same"
    );
    assert_eq!((p.done, p.unknown, p.errors), (3, 3, 0), "{p:?}");
}

/// A song's identity and state are stored together or not at all (identification review,
/// fix round 1): a disk error on the state write leaves no identity behind, so no file gets
/// the IDs of an identity that isn't stored, and the song is checked again. Once the disk is
/// back, the next run stores it and writes its IDs into the file ("Write IDs" on).
#[tokio::test]
async fn a_failed_state_write_stores_no_identity_and_the_next_run_writes_the_ids() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("music").join("11 bohemian.flac");
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../pm-library/tests/fixtures/ietf-60-mono-audio.flac"
        ),
        &f,
    )
    .unwrap();
    let s = state(dir.path(), None).await;
    *s.acoustid.write().unwrap() = identify::AcoustIdSettings {
        api_key: "test-key".into(),
        base_url: "http://127.0.0.1:9".into(),
        configured: String::new(),
    };
    let app = router(s.clone());
    let (st, b) = call(
        &app,
        "PUT",
        "/v1/library/identify/settings",
        Some(serde_json::json!({"write_ids": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    let mbid = "b1a9c0e9-d987-4042-ae91-78d6a3267d69";
    {
        let lib = s.library.lock().unwrap();
        lib.upsert_track(&untagged_track(f.clone(), "Bohemian Rhapsody", "Queen"))
            .unwrap();
        // Looked up before: AcoustID knows the song
        let key = Library::file_key(&f).unwrap();
        lib.save_fingerprint(&key, 60, "AQfake").unwrap();
        let answer = serde_json::json!({"status": "ok", "results": [{"id": "9ff43b6a-4f16-427c-93c2-92307ca505e0",
            "score": 0.95, "recordings": [{"id": mbid, "title": "Bohemian Rhapsody"}]}]});
        lib.save_lookup(&key, &answer.to_string()).unwrap();
    }
    // The song's state write fails (with its identity, in one transaction)
    let db = rusqlite::Connection::open(dir.path().join("lib.sqlite")).unwrap();
    db.execute_batch(
        "CREATE TRIGGER fail_state BEFORE INSERT ON identify_state
             BEGIN SELECT RAISE(ABORT, 'disk I/O error'); END;",
    )
    .unwrap();
    let e = identify::run(&s, false).await.unwrap_err();
    assert!(e.message.contains("disk I/O error"), "{}", e.message);
    let track = |s: &Arc<NodeState>| s.library.lock().unwrap().all_tracks().unwrap().remove(0);
    assert!(track(&s).canonical.is_none(), "no identity without its state");
    let (_, tags) = pm_library::probe_and_read(&f).unwrap();
    assert_eq!(
        tags.recording_mbid, None,
        "no IDs for an identity that isn't stored"
    );
    // The disk is back: the next run stores it and its IDs reach the file
    db.execute_batch("DROP TRIGGER fail_state;").unwrap();
    identify::run(&s, false).await.unwrap();
    assert_eq!(
        track(&s).canonical.map(|(u, _)| u.to_url()),
        Some(format!("https://musicbrainz.org/recording/{mbid}"))
    );
    let (_, tags) = pm_library::probe_and_read(&f).unwrap();
    assert_eq!(
        tags.recording_mbid,
        Some(pm_model::Mbid::parse(mbid).unwrap()),
        "the stored identity reached the file"
    );
}
