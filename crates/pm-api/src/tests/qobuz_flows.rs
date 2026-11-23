//! Qobuz: sign-in, search and streams against a mock.
//!
//! Part of pm-api's unit tests (`mod tests` in lib.rs, which holds the helpers these use). A new
//! test of this area goes at the end of this file.

use super::*;

/// Qobuz (unofficial-only): the owner's app and sign-in, search, an album, a signed stream
/// request for browsers and zones, and the mode switching itself off when Qobuz refuses the app.
#[tokio::test]
async fn qobuz_signs_in_searches_and_streams_from_a_mock() {
    let _api_guard = qobuz::API_TEST_LOCK.lock().await;
    use md5::Digest;
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let secret_ok = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let expected_format = Arc::new(std::sync::atomic::AtomicU32::new(5));
    let file_base = base.clone();
    let ok2 = secret_ok.clone();
    let format2 = expected_format.clone();
    let mp3 = b"synthetic MP3 stream";
    use base64::Engine;
    let fixture = |secret: &str, zone: &str| {
        let padded = format!(
            "{}{}",
            base64::engine::general_purpose::STANDARD.encode(secret),
            "A".repeat(44)
        );
        let (seed, rest) = padded.split_at(8);
        let (info, extras) = rest.split_at(8);
        format!("a.initialSeed(\"{seed}\",window.utimezone.{});name:\"Region/{zone}\",info:\"{info}\",extras:\"{extras}\";", zone.to_ascii_lowercase())
    };
    let bundle = format!(
        "production:{{api:{{appId:\"100000000\",appSecret:\"{}\"}}}};{}{}",
        "a".repeat(32),
        fixture("wrongSecret", "First"),
        fixture("s3cret", "Second")
    );
    let album = serde_json::json!({"id": "0060253780968", "title": "Random Access Memories", "artist": {"name": "Daft Punk"}, "upc": "0886443927087",
        "release_date_original": "2013-05-17", "image": {"large": "https://img/600.jpg"}, "maximum_bit_depth": 24, "maximum_sampling_rate": 88.2,
        "tracks": {"items": [{"id": 5966783, "title": "Get Lucky", "track_number": 8, "duration": 369, "performer": {"name": "Daft Punk"}, "isrc": "USQX91300108"}]}});
    let mock = Router::new()
        .route("/login", get(|| async { "<script src=\"/resources/1.2.3-a004/bundle.js\"></script>" }))
        .route("/resources/1.2.3-a004/bundle.js", get(move || { let bundle = bundle.clone(); async move { bundle } }))
        .route(
            "/user/login",
            get(|headers: axum::http::HeaderMap, axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
                assert_eq!(headers.get("x-app-id").unwrap(), "100000000");
                assert_eq!(q["app_id"], "100000000");
                let password = if q["email"] == "wrong@example.com" { "wrong" } else { "hunter2" };
                assert_eq!(q["password"], format!("{:x}", md5::Md5::digest(password.as_bytes())), "the password goes as its MD5");
                if q["email"] == "wrong@example.com" {
                    return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"message": "private refusal"})));
                }
                if q["email"] == "free@example.com" {
                    return (StatusCode::OK, Json(serde_json::json!({"user_auth_token": "t-free", "user": {"id": 2, "credential": {"parameters": null}}})));
                }
                (StatusCode::OK, Json(serde_json::json!({"user_auth_token": "t-1", "user": {"id": 1, "display_name": "Listener", "country_code": "CH",
                    "credential": {"label": "Studio", "parameters": {"lossless_streaming": true, "hires_streaming": true}}}})))
            }),
        )
        .route("/catalog/search", get({ let album = album.clone(); move |headers: axum::http::HeaderMap| { let album = album.clone(); async move {
            assert_eq!(headers.get("x-user-auth-token").unwrap(), "t-1");
            assert!(headers["user-agent"].to_str().unwrap().contains("Firefox"), "Qobuz gets the browser User-Agent");
            Json(serde_json::json!({"albums": {"items": [album.clone()]}, "tracks": {"items": [{"id": 5966783, "title": "Get Lucky", "duration": 369, "performer": {"name": "Daft Punk"}, "isrc": "USQX91300108", "album": album}]},
                "artists": {"items": [{"id": 36819, "name": "Daft Punk"}]}}))
        } } }))
        .route("/album/get", get({ let album = album.clone(); move |headers: axum::http::HeaderMap| { let album = album.clone(); async move {
            assert!(headers["user-agent"].to_str().unwrap().contains("Firefox"), "Qobuz gets the browser User-Agent");
            Json(album)
        } } }))
        .route(
            "/track/getFileUrl",
            get(move |headers: axum::http::HeaderMap, axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| {
                let (base, ok, format) = (file_base.clone(), ok2.clone(), format2.clone());
                async move {
                    // The browser stream, the zone's (blocking) one and the probes alike
                    assert!(headers["user-agent"].to_str().unwrap().contains("Firefox"), "Qobuz gets the browser User-Agent");
                    if headers.get("x-app-id").is_some_and(|a| a != "100000000") {
                        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"status": "error", "code": 400, "message": "Invalid Request Parameters app_id"})));
                    }
                    let want = format!("{:x}", md5::Md5::digest(format!("trackgetFileUrlformat_id{}intentstreamtrack_id{}{}s3cret", q["format_id"], q["track_id"], q["request_ts"]).as_bytes()));
                    if !ok.load(std::sync::atomic::Ordering::SeqCst) || q["request_sig"] != want {
                        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"status": "error", "code": 400, "message": "Invalid Request Signature parameter (request_sig)"})));
                    }
                    // Qobuz looks at the account only once the app is in order
                    if headers.get("x-user-auth-token").is_none() {
                        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"status": "error", "code": 401, "message": "Invalid user auth token"})));
                    }
                    if q["track_id"] == "2" {
                        return (StatusCode::OK, Json(serde_json::json!({"url": format!("{base}/file.flac"), "sample": true})));
                    }
                    let format = format.load(std::sync::atomic::Ordering::SeqCst);
                    assert_eq!(q["format_id"], format.to_string(), "the selected quality reaches Qobuz");
                    let (file, mime) = if format == 5 { ("file.mp3", "audio/mpeg") } else { ("file.flac", "audio/flac") };
                    (StatusCode::OK, Json(serde_json::json!({"url": format!("{base}/{file}"), "format_id": format, "mime_type": mime})))
                }
            }),
        )
        .route("/file.flac", get({ let f = flac.clone(); move || { let f = f.clone(); async move { f } } }))
        .route("/file.mp3", get(move || async move { mp3.as_slice() }));
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    qobuz::set_api_base(&base);
    qobuz::set_player(&base);

    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let row = |srcs: &serde_json::Value| {
        srcs.as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == "qobuz")
            .unwrap()["status"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let (_, srcs) = call(&app, "GET", "/v1/sources", None).await;
    assert_eq!(row(&srcs), "needs sign-in");
    let (st, q) = call(&app, "GET", "/v1/sources/qobuz/signin", None).await;
    assert_eq!(st, StatusCode::OK, "{q}");
    assert_eq!(q["ways"].as_array().unwrap().len(), 1);
    assert_eq!(q["ways"][0]["id"], "own");
    assert!(q["imports"].as_array().is_none_or(|v| v.is_empty()));
    let (st, _) = call(
        &app,
        "POST",
        "/v1/sources/qobuz/signin/import",
        Some(serde_json::json!({"text": "anything"})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = call(
        &app,
        "POST",
        "/v1/sources/qobuz/unofficial/app/check",
        Some(serde_json::json!({"app_id": "100000000", "app_secret": "s3cret"})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    // A free account and a wrong password are refused in plain words; discovery happens on sign-in.
    let (st, r) = call(
        &app,
        "POST",
        "/v1/sources/qobuz/unofficial/login",
        Some(serde_json::json!({"email": "wrong@example.com", "password": "wrong"})),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert!(r["error"]
        .as_str()
        .unwrap()
        .contains("Wrong Qobuz email or password"));
    // A free account is refused; a paid one signs in and only its token is kept
    let (st, _) = call(
        &app,
        "POST",
        "/v1/sources/qobuz/unofficial/login",
        Some(serde_json::json!({"email": "free@example.com", "password": "hunter2"})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, q) = call(
        &app,
        "POST",
        "/v1/sources/qobuz/unofficial/login",
        Some(serde_json::json!({"email": "me@example.com", "password": "hunter2"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{q}");
    assert_eq!(
        (
            q["signed_in"].clone(),
            q["enabled"].clone(),
            q["plan"].clone(),
            q["hires"].clone()
        ),
        (
            serde_json::json!(true),
            serde_json::json!(true),
            serde_json::json!("Studio"),
            serde_json::json!(true)
        )
    );
    assert!(!q.to_string().contains("s3cret") && !q.to_string().contains("t-1"));
    let cached = s
        .library
        .lock()
        .unwrap()
        .get_setting(qobuz::APP_KEY)
        .unwrap()
        .unwrap();
    assert!(cached.contains("s3cret") && cached.contains("1.2.3-a004/bundle.js"));
    expected_format.store(27, std::sync::atomic::Ordering::SeqCst);
    let kept = s
        .library
        .lock()
        .unwrap()
        .get_setting(qobuz::STATE_KEY)
        .unwrap()
        .unwrap();
    assert!(
        !kept.contains("hunter2")
            && !kept.contains("me@example.com")
            && !kept.contains(&format!("{:x}", md5::Md5::digest(b"hunter2"))),
        "neither password, its MD5 nor email is stored"
    );
    let (_, srcs) = call(&app, "GET", "/v1/sources", None).await;
    assert_eq!(row(&srcs), "signed in");
    // Search, an album, and the stream for browsers
    let (st, r) = call(
        &app,
        "GET",
        "/v1/sources/qobuz/search?q=daft%20punk&limit=10",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    let items = r["items"].as_array().unwrap();
    assert_eq!(items[0]["kind"], "artist");
    assert_eq!(items[1]["id"], "album:0060253780968");
    assert_eq!(items[1]["year"], 2013);
    assert_eq!(items[2]["uri"], "qobuz:track:5966783");
    assert_eq!(items[2]["isrc"], "USQX91300108");
    let (st, it) = call(&app, "GET", "/v1/sources/qobuz/items/album:0060253780968", None).await;
    assert_eq!(st, StatusCode::OK, "{it}");
    assert_eq!(it["tracks"][0]["url"], "qobuz:track:5966783");
    assert_eq!(it["tracks"][0]["format"], "FLAC 24-bit/88.2 kHz (Qobuz)");
    let req = Request::builder()
        .uri("/v1/sources/qobuz/stream/5966783")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get("content-type").unwrap(), "audio/flac");
    assert_eq!(
        resp.into_body().collect().await.unwrap().to_bytes().as_ref(),
        flac.as_slice()
    );
    // A zone opens it through the decoder; a sample is refused, not played
    let lib = s.library.clone();
    let opened = tokio::task::spawn_blocking(move || {
        qobuz::open_track_blocking(&lib, "5966783", &accounts::Who::Owner).is_ok()
    })
    .await
    .unwrap();
    assert!(opened);
    let req = Request::builder()
        .uri("/v1/sources/qobuz/stream/2")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    // The browser relay labels the format requested from Qobuz, including MP3 (format 5).
    // Changing quality affects the next request, including switching back to either FLAC mode.
    for (quality, format, mime, bytes) in [
        ("mp3", 5, "audio/mpeg", mp3.as_slice()),
        ("cd", 6, "audio/flac", flac.as_slice()),
        ("max", 27, "audio/flac", flac.as_slice()),
    ] {
        let (st, q) = call(
            &app,
            "PUT",
            "/v1/sources/qobuz/quality",
            Some(serde_json::json!({"id": quality})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{q}");
        expected_format.store(format, std::sync::atomic::Ordering::SeqCst);
        let req = Request::builder()
            .uri("/v1/sources/qobuz/stream/5966783")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()["content-type"], mime, "{quality}");
        assert_eq!(resp.headers()["content-length"], bytes.len().to_string());
        assert_eq!(
            resp.into_body().collect().await.unwrap().to_bytes().as_ref(),
            bytes,
            "{quality}"
        );
    }
    // Persistent refusal after one rediscovery switches the source off without removing the session.
    secret_ok.store(false, std::sync::atomic::Ordering::SeqCst);
    let req = Request::builder()
        .uri("/v1/sources/qobuz/stream/5966783")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::BAD_GATEWAY
    );
    let (_, q) = call(&app, "GET", "/v1/sources/qobuz/unofficial", None).await;
    assert_eq!(q["enabled"], false, "{q}");
    assert!(
        q["health"]["detail"].as_str().unwrap().contains("web player"),
        "{q}"
    );
    let (_, srcs) = call(&app, "GET", "/v1/sources", None).await;
    assert!(row(&srcs).starts_with("error:"));
    let (st, _) = call(&app, "GET", "/v1/sources/qobuz/search?q=x", None).await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED);
    secret_ok.store(true, std::sync::atomic::Ordering::SeqCst);
    let (_, q) = call(
        &app,
        "PUT",
        "/v1/sources/qobuz/unofficial",
        Some(serde_json::json!({"enabled": true})),
    )
    .await;
    assert_eq!(
        (q["enabled"].clone(), q["health"].clone()),
        (serde_json::json!(true), serde_json::Value::Null)
    );
    let (_, r) = call(&app, "DELETE", "/v1/sources/qobuz/signin", None).await;
    assert!(r["in_use"].is_null());
    assert_eq!(r["ways"].as_array().unwrap().len(), 1);
    assert!(!r.to_string().contains("t-1"), "{r}");
}

/// Qobuz plays each person's items with their own sign-in too: the owner's token for the
/// owner, a listener's for theirs, and "Connect Qobuz" for someone without one.
#[tokio::test(flavor = "multi_thread")]
async fn each_person_plays_qobuz_with_their_own_account() {
    let _api_guard = qobuz::API_TEST_LOCK.lock().await;
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let (seen2, file_base) = (seen.clone(), base.clone());
    let mock = Router::new()
        .route(
            "/track/getFileUrl",
            get(move |headers: HeaderMap| {
                let (seen, base) = (seen2.clone(), file_base.clone());
                async move {
                    let t = headers.get("x-user-auth-token").unwrap().to_str().unwrap().to_string();
                    seen.lock().unwrap().push(t);
                    Json(serde_json::json!({"url": format!("{base}/file.flac"), "format_id": 27, "mime_type": "audio/flac"}))
                }
            }),
        )
        .route("/file.flac", get(move || { let f = flac.clone(); async move { f } }));
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    qobuz::set_api_base(&base);
    let dir = tempfile::tempdir().unwrap();
    let (s, anna, ben) = two_people_node(dir.path(), "http://127.0.0.1:9").await;
    let mode = |token: &str| {
        serde_json::json!({"enabled": true, "quality": "max", "session": {"token": token, "hires": true}})
            .to_string()
    };
    {
        let lib = s.library.lock().unwrap();
        lib.set_setting(
            qobuz::APP_KEY,
            r#"{"id": "100000000", "secret": "s3cret", "from": "settings"}"#,
        )
        .unwrap();
        lib.set_setting(qobuz::STATE_KEY, &mode("t-owner")).unwrap();
        lib.set_setting(&format!("person.{anna}.{}", qobuz::STATE_KEY), &mode("t-anna"))
            .unwrap();
    }
    let lib = s.library.clone();
    let opened = tokio::task::spawn_blocking(move || {
        [
            accounts::Who::Owner,
            accounts::Who::Listener(anna.into()),
            accounts::Who::Listener(ben.into()),
            accounts::Who::Nobody,
        ]
        .map(|who| {
            qobuz::open_track_blocking(&lib, "5966783", &who)
                .map(drop)
                .map_err(|e| e.to_string())
        })
    })
    .await
    .unwrap();
    assert!(opened[0].is_ok() && opened[1].is_ok(), "{opened:?}");
    assert!(
        opened[2].as_ref().unwrap_err().contains("Connect Qobuz"),
        "{opened:?}"
    );
    assert!(opened[3].is_err(), "nobody's item plays with no one's account");
    assert_eq!(*seen.lock().unwrap(), ["t-owner", "t-anna"]);
}

#[tokio::test]
async fn qobuz_admin_override_wins_and_skips_discovery() {
    let _api_guard = qobuz::API_TEST_LOCK.lock().await;
    use md5::Digest;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let discoveries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mock = Router::new()
        .route("/login", get({
            let discoveries = discoveries.clone();
            move || async move {
                discoveries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                "discovery must not run"
            }
        }))
        .route("/user/login", get(|headers: HeaderMap, axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
            assert_eq!(headers.get("x-app-id").unwrap(), "200000000");
            assert_eq!(q["app_id"], "200000000");
            assert_eq!(q["password"], format!("{:x}", md5::Md5::digest(b"hunter2")));
            Json(serde_json::json!({"user_auth_token": "configured-token", "user": {"id": 1, "credential": {"parameters": {"hires_streaming": true}}}}))
        }))
        .route("/track/getFileUrl", get(|axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
            let want = format!("{:x}", md5::Md5::digest(format!("trackgetFileUrlformat_id{}intentstreamtrack_id5966783{}configuredSecret", q["format_id"], q["request_ts"]).as_bytes()));
            assert_eq!(q["request_sig"], want);
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({"message": "Invalid Request Signature parameter (request_sig)"})))
        }));
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    qobuz::set_api_base(&base);
    qobuz::set_player(&base);
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let old = r#"{"id":"100000000","secret":"cachedSecret","from":"web","bundle_url":"old"}"#;
    s.library
        .lock()
        .unwrap()
        .set_setting(qobuz::APP_KEY, old)
        .unwrap();
    qobuz::set_test_config_app(Some(qobuz::App {
        id: "200000000".into(),
        secret: "configuredSecret".into(),
        from: "config".into(),
        bundle_url: None,
    }));
    let app = router(s.clone());
    let (st, q) = call(
        &app,
        "POST",
        "/v1/sources/qobuz/unofficial/login",
        Some(serde_json::json!({"email": "owner@example.com", "password": "hunter2"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{q}");
    assert_eq!(discoveries.load(std::sync::atomic::Ordering::SeqCst), 0);
    // A signature 400 on a stream refreshes nothing: the admin's values are the admin's
    let req = Request::builder()
        .uri("/v1/sources/qobuz/stream/5966783")
        .body(Body::empty())
        .unwrap();
    let status = app.clone().oneshot(req).await.unwrap().status();
    qobuz::set_test_config_app(None);
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(discoveries.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(
        s.library
            .lock()
            .unwrap()
            .get_setting(qobuz::APP_KEY)
            .unwrap()
            .as_deref(),
        Some(old)
    );
    server.abort();
}

#[tokio::test]
async fn qobuz_users_are_isolated_and_rotation_keeps_their_sessions() {
    let _api_guard = qobuz::API_TEST_LOCK.lock().await;
    use base64::Engine;
    use md5::Digest;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let fixture = |secret: &str, zone: &str| {
        let padded = format!(
            "{}{}",
            base64::engine::general_purpose::STANDARD.encode(secret),
            "A".repeat(44)
        );
        let (seed, rest) = padded.split_at(8);
        let (info, extras) = rest.split_at(8);
        format!("a.initialSeed(\"{seed}\",window.utimezone.{});name:\"Region/{zone}\",info:\"{info}\",extras:\"{extras}\";", zone.to_ascii_lowercase())
    };
    let bundle = |secret: &str| {
        format!(
            "production:{{api:{{appId:\"100000000\",appSecret:\"{}\"}}}};{}{}",
            "a".repeat(32),
            fixture("wrongSecret", "First"),
            fixture(secret, "Second")
        )
    };
    let current = Arc::new(std::sync::RwLock::new(("s3cret".to_string(), bundle("s3cret"))));
    let discoveries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let reject_next_login = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mock = Router::new()
        .route("/login", get(|| async { "<script src=\"/resources/1.2.3-a004/bundle.js\"></script>" }))
        .route("/resources/1.2.3-a004/bundle.js", get({
            let current = current.clone();
            let discoveries = discoveries.clone();
            move || async move {
                discoveries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                current.read().unwrap().1.clone()
            }
        }))
        .route("/user/login", get({
            let reject_next_login = reject_next_login.clone();
            move |axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| {
                let reject_next_login = reject_next_login.clone();
                async move {
                    assert_eq!(q["password"], format!("{:x}", md5::Md5::digest(b"hunter2")));
                    if reject_next_login.swap(false, std::sync::atomic::Ordering::SeqCst) {
                        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"message": "Invalid or missing app_id parameter"})));
                    }
                    let (token, id) = if q["email"] == "anna@example.com" { ("t-anna", 2) } else { ("t-owner", 1) };
                    (StatusCode::OK, Json(serde_json::json!({"user_auth_token": token, "user": {"id": id, "display_name": id.to_string(), "credential": {"label": "Studio", "parameters": {"hires_streaming": true}}}})))
                }
            }
        }))
        .route("/track/getFileUrl", get({
            let current = current.clone();
            let file_base = base.clone();
            move |axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| {
                let current = current.clone();
                let base = file_base.clone();
                async move {
                    let secret = current.read().unwrap().0.clone();
                    let want = format!("{:x}", md5::Md5::digest(format!("trackgetFileUrlformat_id{}intentstreamtrack_id{}{}{secret}", q["format_id"], q["track_id"], q["request_ts"]).as_bytes()));
                    if q["request_sig"] != want {
                        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"message": "bad signature"})));
                    }
                    (StatusCode::OK, Json(serde_json::json!({"url": format!("{base}/file.flac"), "format_id": 27})))
                }
            }
        }))
        .route("/file.flac", get(|| async { "synthetic audio" }));
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    qobuz::set_api_base(&base);
    qobuz::set_player(&base);
    let dir = tempfile::tempdir().unwrap();
    let (s, anna, _) = two_people_node(dir.path(), "http://127.0.0.1:9").await;
    s.library
        .lock()
        .unwrap()
        .set_setting(accounts::SOURCE_POLICY_KEY, "own")
        .unwrap();
    let app = router(s.clone());
    let anna_token = accounts::mint(&s, anna).unwrap();
    for (token, email) in [
        ("s3cret", "owner@example.com"),
        (anna_token.as_str(), "anna@example.com"),
    ] {
        let (st, q, _) = send_as(
            &app,
            "POST",
            "/v1/sources/qobuz/unofficial/login",
            Some(token),
            &[],
            Some(serde_json::json!({"email": email, "password": "hunter2"})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{q}");
    }
    assert_eq!(
        discoveries.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the second user reuses the node app"
    );
    let (owner, theirs) = {
        let lib = s.library.lock().unwrap();
        (
            lib.get_setting(qobuz::STATE_KEY).unwrap().unwrap(),
            lib.get_setting(&format!("person.{anna}.{}", qobuz::STATE_KEY))
                .unwrap()
                .unwrap(),
        )
    };
    assert!(owner.contains("t-owner") && !owner.contains("t-anna"));
    assert!(theirs.contains("t-anna") && !theirs.contains("t-owner"));
    *current.write().unwrap() = ("newSecret".into(), bundle("newSecret"));
    // Past the discovery rate limit, so the refused values can be looked up again
    qobuz::age_discovery(std::time::Duration::from_secs(6 * 60));
    let req = Request::builder()
        .uri("/v1/sources/qobuz/stream/5966783")
        .header("authorization", "Bearer s3cret")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        discoveries.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "one refresh after 400"
    );
    let (owner_after, theirs_after, cached) = {
        let lib = s.library.lock().unwrap();
        (
            lib.get_setting(qobuz::STATE_KEY).unwrap().unwrap(),
            lib.get_setting(&format!("person.{anna}.{}", qobuz::STATE_KEY))
                .unwrap()
                .unwrap(),
            lib.get_setting(qobuz::APP_KEY).unwrap().unwrap(),
        )
    };
    let owner_before: serde_json::Value = serde_json::from_str(&owner).unwrap();
    let theirs_before: serde_json::Value = serde_json::from_str(&theirs).unwrap();
    let owner_after: serde_json::Value = serde_json::from_str(&owner_after).unwrap();
    let theirs_after: serde_json::Value = serde_json::from_str(&theirs_after).unwrap();
    assert_eq!(owner_after["session"], owner_before["session"]);
    assert_eq!(theirs_after["session"], theirs_before["session"]);
    assert!(cached.contains("newSecret") && cached.contains("bundle.js"));
    reject_next_login.store(true, std::sync::atomic::Ordering::SeqCst);
    *current.write().unwrap() = ("newerSecret".into(), bundle("newerSecret"));
    qobuz::age_discovery(std::time::Duration::from_secs(6 * 60));
    let (st, _, _) = send_as(
        &app,
        "DELETE",
        "/v1/sources/qobuz/unofficial/login",
        Some("s3cret"),
        &[],
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, q, _) = send_as(
        &app,
        "POST",
        "/v1/sources/qobuz/unofficial/login",
        Some("s3cret"),
        &[],
        Some(serde_json::json!({"email": "owner@example.com", "password": "hunter2"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{q}");
    assert_eq!(
        discoveries.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "one refresh after login 400"
    );
    let theirs_after_login = {
        let lib = s.library.lock().unwrap();
        lib.get_setting(&format!("person.{anna}.{}", qobuz::STATE_KEY))
            .unwrap()
            .unwrap()
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&theirs_after_login).unwrap()["session"],
        theirs_before["session"]
    );
    server.abort();
}

/// The source policy reaches every use of an account, not only TIDAL plays (#277 reviews):
/// under `node` a listener's TIDAL search and item pages and their Qobuz plays use the node's
/// sign-in; under `own` never the node's; Spotify Connect (devices, play) is always the
/// listener's own, so the owner's phone is never listed for them.
#[tokio::test]
async fn the_source_policy_reaches_search_item_pages_qobuz_and_spotify_connect() {
    let _api_guard = qobuz::API_TEST_LOCK.lock().await;
    // TIDAL: tokens for the app, every other call recorded with its bearer
    let tl = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tidal_base = format!("http://127.0.0.1:{}", tl.local_addr().unwrap().port());
    let tidal_seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let ts = tidal_seen.clone();
    let tidal_mock = Router::new()
        .route(
            "/v1/oauth2/token",
            post(|| async {
                Json(serde_json::json!({"access_token": "app-token", "refresh_token": "r", "expires_in": 3600}))
            }),
        )
        .fallback(move |headers: HeaderMap| {
            let seen = ts.clone();
            async move {
                if let Some(b) = headers.get("authorization") {
                    seen.lock().unwrap().push(b.to_str().unwrap().to_string());
                }
                Json(serde_json::json!({"data": [], "included": []}))
            }
        });
    tokio::spawn(async move { axum::serve(tl, tidal_mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let (s, anna, ben) = two_people_node(dir.path(), &tidal_base).await;
    let app = router(s.clone());
    let (anna_tok, ben_tok) = (
        accounts::mint(&s, anna).unwrap(),
        accounts::mint(&s, ben).unwrap(),
    );
    let set_policy = |p: &str| {
        s.library
            .lock()
            .unwrap()
            .set_setting(accounts::SOURCE_POLICY_KEY, p)
            .unwrap();
    };
    let asked = |tok: &str, uri: &str| {
        let (app, tok, uri, seen) = (app.clone(), tok.to_string(), uri.to_string(), tidal_seen.clone());
        async move {
            seen.lock().unwrap().clear();
            let (st, v, _) = send_as(&app, "GET", &uri, Some(&tok), &[], None).await;
            assert_eq!(st, StatusCode::OK, "{uri}: {v}");
            let got: Vec<String> = seen.lock().unwrap().drain(..).collect();
            assert!(!got.is_empty(), "TIDAL was asked for {uri}");
            got
        }
    };

    // `node`: Ben (no TIDAL of his own) searches and reads an item page with the node's
    set_policy("node");
    for uri in [
        "/v1/search?q=lucky&scope=tidal",
        "/v1/discover/contents?uri=https://tidal.com/album/4242",
    ] {
        let got = asked(&ben_tok, uri).await;
        assert!(got.iter().all(|b| b == "Bearer owner-token"), "{uri}: {got:?}");
    }
    // `own`: never the node's, for him or for Anna
    set_policy("own");
    let got = asked(&ben_tok, "/v1/search?q=other&scope=tidal").await;
    assert!(got.iter().all(|b| b != "Bearer owner-token"), "{got:?}");
    let got = asked(
        &anna_tok,
        "/v1/discover/contents?uri=https://tidal.com/album/4343",
    )
    .await;
    assert!(got.iter().all(|b| b == "Bearer anna-token"), "{got:?}");

    // Spotify Connect: always the caller's own, whatever the policy
    let sl = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let spotify_base = format!("http://127.0.0.1:{}", sl.local_addr().unwrap().port());
    let sp_seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sps = sp_seen.clone();
    let sp_mock = Router::new().route(
        "/me/player/devices",
        get(move |headers: HeaderMap| {
            let seen = sps.clone();
            async move {
                let b = headers.get("authorization").unwrap().to_str().unwrap().to_string();
                let name = if b.ends_with("sp-owner") { "Owner's phone" } else { "Anna's laptop" };
                seen.lock().unwrap().push(b);
                Json(serde_json::json!({"devices": [{"id": "d1", "name": name, "type": "Smartphone", "is_active": false}]}))
            }
        }),
    );
    tokio::spawn(async move { axum::serve(sl, sp_mock).await.unwrap() });
    s.spotify.write().unwrap().api_base = spotify_base;
    {
        let lib = s.library.lock().unwrap();
        let until = pm_library::db::now_unix() + 3600;
        let sess = |t: &str| serde_json::json!({"access_token": t, "expires_at": until}).to_string();
        lib.set_setting("spotify.user", &sess("sp-owner")).unwrap();
        lib.set_setting(&format!("person.{anna}.spotify.user"), &sess("sp-anna"))
            .unwrap();
    }
    for p in ["node", "choice"] {
        set_policy(p);
        let (st, v, _) = send_as(
            &app,
            "GET",
            "/v1/sources/spotify/devices",
            Some(&anna_tok),
            &[],
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{p}: {v}");
        assert_eq!(v[0]["name"], "Anna's laptop", "{p}: {v}");
        let (st, v, _) = send_as(
            &app,
            "GET",
            "/v1/sources/spotify/devices",
            Some(&ben_tok),
            &[],
            None,
        )
        .await;
        assert_eq!(
            st,
            StatusCode::PRECONDITION_FAILED,
            "{p}: Ben has no Spotify of his own: {v}"
        );
    }
    assert!(
        sp_seen.lock().unwrap().iter().all(|b| b == "Bearer sp-anna"),
        "the owner's devices never asked for a listener: {:?}",
        sp_seen.lock().unwrap()
    );

    // Qobuz plays: the node's under `node`, none under `own` for someone without their own
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let ql = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let qbase = format!("http://127.0.0.1:{}", ql.local_addr().unwrap().port());
    let q_seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let (qs, file_base) = (q_seen.clone(), qbase.clone());
    let q_mock = Router::new()
        .route(
            "/track/getFileUrl",
            get(move |headers: HeaderMap| {
                let (seen, base) = (qs.clone(), file_base.clone());
                async move {
                    seen.lock().unwrap().push(headers.get("x-user-auth-token").unwrap().to_str().unwrap().to_string());
                    Json(serde_json::json!({"url": format!("{base}/file.flac"), "format_id": 27, "mime_type": "audio/flac"}))
                }
            }),
        )
        .route("/file.flac", get(move || { let f = flac.clone(); async move { f } }));
    tokio::spawn(async move { axum::serve(ql, q_mock).await.unwrap() });
    qobuz::set_api_base(&qbase);
    {
        let lib = s.library.lock().unwrap();
        lib.set_setting(
            qobuz::APP_KEY,
            r#"{"id": "100000000", "secret": "s3cret", "from": "settings"}"#,
        )
        .unwrap();
        lib.set_setting(
            qobuz::STATE_KEY,
            &serde_json::json!({"enabled": true, "quality": "max", "session": {"token": "t-owner", "hires": true}}).to_string(),
        )
        .unwrap();
    }
    let open_as_ben = |policy: &'static str| {
        let lib = s.library.clone();
        async move {
            lib.lock()
                .unwrap()
                .set_setting(accounts::SOURCE_POLICY_KEY, policy)
                .unwrap();
            tokio::task::spawn_blocking(move || {
                let who = accounts::Who::Listener(ben.into()).resolved(&lib.lock().unwrap());
                qobuz::open_track_blocking(&lib, "5966783", &who)
                    .map(drop)
                    .map_err(|e| e.to_string())
            })
            .await
            .unwrap()
        }
    };
    assert!(open_as_ben("node").await.is_ok());
    assert_eq!(*q_seen.lock().unwrap(), ["t-owner"]);
    assert!(open_as_ben("own").await.unwrap_err().contains("Connect Qobuz"));
    assert_eq!(q_seen.lock().unwrap().len(), 1, "never the node's under `own`");
}

#[tokio::test]
async fn qobuz_retries_reuse_one_discovery_and_a_track_400_keeps_the_source_on() {
    let _api_guard = qobuz::API_TEST_LOCK.lock().await;
    use base64::Engine;
    use md5::Digest;
    use std::sync::atomic::Ordering::SeqCst;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let fixture = |secret: &str, zone: &str| {
        let padded = format!(
            "{}{}",
            base64::engine::general_purpose::STANDARD.encode(secret),
            "A".repeat(44)
        );
        let (seed, rest) = padded.split_at(8);
        let (info, extras) = rest.split_at(8);
        format!("a.initialSeed(\"{seed}\",window.utimezone.{});name:\"Region/{zone}\",info:\"{info}\",extras:\"{extras}\";", zone.to_ascii_lowercase())
    };
    let bundle = format!(
        "production:{{api:{{appId:\"100000000\",appSecret:\"{}\"}}}};{}{}",
        "a".repeat(32),
        fixture("wrongSecret", "First"),
        fixture("s3cret", "Second")
    );
    let looks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let busy = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mock = Router::new()
        .route("/login", get({
            let looks = looks.clone();
            move || async move {
                looks.fetch_add(1, SeqCst);
                "<script src=\"/resources/1.2.3-a004/bundle.js\"></script>"
            }
        }))
        .route("/resources/1.2.3-a004/bundle.js", get(move || { let bundle = bundle.clone(); async move { bundle } }))
        .route("/user/login", get(|axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
            match q["email"].as_str() {
                "wrong@example.com" => (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"message": "refused"}))),
                "free@example.com" => (StatusCode::OK, Json(serde_json::json!({"user_auth_token": "t-free", "user": {"id": 2, "credential": {"parameters": null}}}))),
                _ => (StatusCode::OK, Json(serde_json::json!({"user_auth_token": "t-1", "user": {"id": 1, "credential": {"label": "Studio", "parameters": {"hires_streaming": true}}}}))),
            }
        }))
        .route("/track/getFileUrl", get({
            let busy = busy.clone();
            move |axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| {
                let busy = busy.load(SeqCst);
                async move {
                    if busy {
                        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({})));
                    }
                    let want = format!("{:x}", md5::Md5::digest(format!("trackgetFileUrlformat_id{}intentstreamtrack_id{}{}s3cret", q["format_id"], q["track_id"], q["request_ts"]).as_bytes()));
                    if q["request_sig"] != want {
                        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"message": "Invalid Request Signature parameter (request_sig)"})));
                    }
                    if q["format_id"] != "5" {
                        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"message": "This format is not available for this track"})));
                    }
                    (StatusCode::OK, Json(serde_json::json!({"url": "https://stream.invalid/mock"})))
                }
            }
        }));
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    qobuz::set_api_base(&base);
    qobuz::set_player(&base);
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let sign_in = |email: &str| {
        let app = app.clone();
        let body = serde_json::json!({"email": email, "password": "hunter2"});
        async move { call(&app, "POST", "/v1/sources/qobuz/unofficial/login", Some(body)).await }
    };
    let cached = || s.library.lock().unwrap().get_setting(qobuz::APP_KEY).unwrap();
    // A wrong password and a free account: one look at the web player for both
    assert_eq!(sign_in("wrong@example.com").await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(sign_in("free@example.com").await.0, StatusCode::FORBIDDEN);
    // A busy Qobuz during the secret probe: a plain temporary error, nothing cached or kept
    busy.store(true, SeqCst);
    let (st, r) = sign_in("me@example.com").await;
    assert_eq!(st, StatusCode::BAD_GATEWAY, "{r}");
    assert_eq!(r["error"], "Could not reach Qobuz. Try again later.");
    assert_eq!(cached(), None);
    let (_, q) = call(&app, "GET", "/v1/sources/qobuz/unofficial", None).await;
    assert_eq!(q["signed_in"], false, "{q}");
    busy.store(false, SeqCst);
    let (st, q) = sign_in("me@example.com").await;
    assert_eq!(st, StatusCode::OK, "{q}");
    assert!(cached().unwrap().contains("s3cret"));
    assert_eq!(looks.load(SeqCst), 1);
    // A 400 about the track, not the app: this track fails, nothing is rediscovered, Qobuz stays on
    let req = Request::builder()
        .uri("/v1/sources/qobuz/stream/5966783")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(looks.load(SeqCst), 1);
    let (_, q) = call(&app, "GET", "/v1/sources/qobuz/unofficial", None).await;
    assert_eq!(
        (q["enabled"].clone(), q["health"].clone()),
        (serde_json::json!(true), serde_json::Value::Null),
        "{q}"
    );
    server.abort();
}

#[tokio::test]
async fn qobuz_a_cut_off_or_refused_look_does_not_hold_sign_in_back() {
    let _api_guard = qobuz::API_TEST_LOCK.lock().await;
    use md5::Digest;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let bundle = qobuz_mock_bundle(&["s3cret"]);
    let looks = Arc::new(AtomicUsize::new(0));
    let logins = Arc::new(AtomicUsize::new(0));
    let hold = Arc::new(AtomicBool::new(false));
    let held = Arc::new(tokio::sync::Notify::new());
    let refuse_app = Arc::new(AtomicBool::new(false));
    let mock = Router::new()
        .route("/login", get({
            let looks = looks.clone();
            move || async move {
                looks.fetch_add(1, SeqCst);
                "<script src=\"/resources/1.2.3-a004/bundle.js\"></script>"
            }
        }))
        .route("/resources/1.2.3-a004/bundle.js", get({
            let (hold, held) = (hold.clone(), held.clone());
            move || {
                let (bundle, hold, held) = (bundle.clone(), hold.load(SeqCst), held.clone());
                async move {
                    if hold {
                        // The bundle never comes: the sign-in waiting for it is dropped
                        held.notify_one();
                        std::future::pending::<()>().await;
                    }
                    bundle
                }
            }
        }))
        .route("/user/login", get({
            let (logins, refuse_app) = (logins.clone(), refuse_app.clone());
            move |axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| {
                logins.fetch_add(1, SeqCst);
                let refuse_app = refuse_app.load(SeqCst);
                async move {
                    assert_eq!(q["password"], format!("{:x}", md5::Md5::digest(b"hunter2")));
                    if refuse_app {
                        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"message": "Invalid app_id"})));
                    }
                    if q["email"] == "wrong@example.com" {
                        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"message": "refused"})));
                    }
                    (StatusCode::OK, Json(serde_json::json!({"user_auth_token": "t-1", "user": {"id": 1, "credential": {"label": "Studio", "parameters": {"hires_streaming": true}}}})))
                }
            }
        }))
        .route("/track/getFileUrl", get(|axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
            let want = format!("{:x}", md5::Md5::digest(format!("trackgetFileUrlformat_id{}intentstreamtrack_id{}{}s3cret", q["format_id"], q["track_id"], q["request_ts"]).as_bytes()));
            if q["request_sig"] != want {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"message": "Invalid Request Signature parameter (request_sig)"})));
            }
            (StatusCode::OK, Json(serde_json::json!({"url": "https://stream.invalid/mock"})))
        }));
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    qobuz::set_api_base(&base);
    qobuz::set_player(&base);
    let sign_in = |app: &Router, email: &str| {
        let app = app.clone();
        let body = serde_json::json!({"email": email, "password": "hunter2"});
        async move { call(&app, "POST", "/v1/sources/qobuz/unofficial/login", Some(body)).await }
    };
    let cut_off = |app: &Router| {
        hold.store(true, SeqCst);
        let task = tokio::spawn(sign_in(app, "me@example.com"));
        let (hold, held) = (hold.clone(), held.clone());
        async move {
            held.notified().await;
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            hold.store(false, SeqCst);
        }
    };
    let cached = |s: &Arc<NodeState>| s.library.lock().unwrap().get_setting(qobuz::APP_KEY).unwrap();

    // Candidates found by an earlier look survive a look that is cut off: a sign-in on a node
    // with nothing cached gets them at once, one whose cached values were refused hears that
    // the look was interrupted
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    assert_eq!(
        sign_in(&app, "wrong@example.com").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(looks.load(SeqCst), 1);
    s.library
        .lock()
        .unwrap()
        .set_setting(
            qobuz::APP_KEY,
            r#"{"id":"100000000","secret":"oldSecret","from":"web","bundle_url":"old"}"#,
        )
        .unwrap();
    qobuz::age_discovery(std::time::Duration::from_secs(6 * 60));
    cut_off(&app).await;
    assert_eq!(looks.load(SeqCst), 2);
    let (st, r) = sign_in(&app, "me@example.com").await;
    assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE, "{r}");
    assert_eq!(r["error"], "Qobuz setup was interrupted. Try again in a minute.");
    assert_eq!(looks.load(SeqCst), 2);
    let other_dir = tempfile::tempdir().unwrap();
    let other = state(other_dir.path(), None).await;
    let (st, r) = sign_in(&router(other.clone()), "me@example.com").await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert!(cached(&other).unwrap().contains("s3cret"));
    assert_eq!(looks.load(SeqCst), 2);

    // With nothing found yet, a cut-off look says so plainly and holds the next one back only
    // a minute
    qobuz::forget_discovery();
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    cut_off(&app).await;
    let (st, r) = sign_in(&app, "me@example.com").await;
    assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE, "{r}");
    assert_eq!(r["error"], "Qobuz setup was interrupted. Try again in a minute.");
    assert_eq!(looks.load(SeqCst), 3);
    qobuz::age_discovery(std::time::Duration::from_secs(61));
    let (st, r) = sign_in(&app, "me@example.com").await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(looks.load(SeqCst), 4);
    assert!(cached(&s).unwrap().contains("s3cret"));

    // An app ID Qobuz refuses at sign-in is not offered again: the next sign-in says so at
    // once, and the one after the rate limit looks again
    qobuz::forget_discovery();
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    refuse_app.store(true, SeqCst);
    let (st, r) = sign_in(&app, "me@example.com").await;
    assert_eq!(st, StatusCode::BAD_GATEWAY, "{r}");
    assert!(
        r["error"].as_str().unwrap().contains("changed its web player"),
        "{r}"
    );
    assert_eq!(looks.load(SeqCst), 5);
    refuse_app.store(false, SeqCst);
    let before = logins.load(SeqCst);
    let (st, r) = sign_in(&app, "me@example.com").await;
    assert_eq!(st, StatusCode::BAD_GATEWAY, "{r}");
    assert_eq!((looks.load(SeqCst), logins.load(SeqCst)), (5, before));
    qobuz::age_discovery(std::time::Duration::from_secs(6 * 60));
    let (st, r) = sign_in(&app, "me@example.com").await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(looks.load(SeqCst), 6);
    assert!(cached(&s).unwrap().contains("s3cret"));
    server.abort();
}

#[tokio::test]
async fn qobuz_a_stream_refresh_off_only_when_the_web_player_values_fail() {
    let _api_guard = qobuz::API_TEST_LOCK.lock().await;
    use md5::Digest;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    // What Qobuz takes now, and what the web player offers
    let secret = Arc::new(std::sync::RwLock::new("s3cret".to_string()));
    let bundle = Arc::new(std::sync::RwLock::new(qobuz_mock_bundle(&["s3cret"])));
    let looks = Arc::new(AtomicUsize::new(0));
    let asked = Arc::new(AtomicUsize::new(0));
    let down = Arc::new(AtomicBool::new(false));
    let mock = Router::new()
        .route("/login", get({
            let looks = looks.clone();
            move || async move {
                looks.fetch_add(1, SeqCst);
                "<script src=\"/resources/1.2.3-a004/bundle.js\"></script>"
            }
        }))
        .route("/resources/1.2.3-a004/bundle.js", get({
            let (bundle, down) = (bundle.clone(), down.clone());
            move || {
                let (bundle, down) = (bundle.read().unwrap().clone(), down.load(SeqCst));
                async move {
                    if down {
                        Err(StatusCode::SERVICE_UNAVAILABLE)
                    } else {
                        Ok(bundle)
                    }
                }
            }
        }))
        .route("/user/login", get(|| async {
            Json(serde_json::json!({"user_auth_token": "t-1", "user": {"id": 1, "credential": {"label": "Studio", "parameters": {"hires_streaming": true}}}}))
        }))
        .route("/track/getFileUrl", get({
            let (secret, asked, file_base) = (secret.clone(), asked.clone(), base.clone());
            move |headers: HeaderMap, axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| {
                asked.fetch_add(1, SeqCst);
                let (secret, base) = (secret.read().unwrap().clone(), file_base.clone());
                async move {
                    assert!(headers["user-agent"].to_str().unwrap().contains("Firefox"), "Qobuz gets the browser User-Agent");
                    let want = format!("{:x}", md5::Md5::digest(format!("trackgetFileUrlformat_id{}intentstreamtrack_id{}{}{secret}", q["format_id"], q["track_id"], q["request_ts"]).as_bytes()));
                    // Track 7 is refused with a signature 400 whatever it is signed with
                    if q["request_sig"] != want || q["track_id"] == "7" {
                        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"message": "Invalid Request Signature parameter (request_sig)"})));
                    }
                    (StatusCode::OK, Json(serde_json::json!({"url": format!("{base}/file.flac")})))
                }
            }
        }))
        .route("/file.flac", get(|| async { "synthetic audio" }));
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    qobuz::set_api_base(&base);
    qobuz::set_player(&base);
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (st, q) = call(
        &app,
        "POST",
        "/v1/sources/qobuz/unofficial/login",
        Some(serde_json::json!({"email": "me@example.com", "password": "hunter2"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{q}");
    assert_eq!(looks.load(SeqCst), 1);
    let stream = |id: &str| {
        let app = app.clone();
        let req = Request::builder()
            .uri(format!("/v1/sources/qobuz/stream/{id}"))
            .body(Body::empty())
            .unwrap();
        async move { app.oneshot(req).await.unwrap().status() }
    };
    let mode = || {
        let app = app.clone();
        async move {
            let (_, q) = call(&app, "GET", "/v1/sources/qobuz/unofficial", None).await;
            (
                q["enabled"].as_bool().unwrap(),
                q["health"]["detail"].as_str().unwrap_or("").to_string(),
            )
        }
    };
    let switch_on = || {
        let app = app.clone();
        async move {
            let (st, _) = call(
                &app,
                "PUT",
                "/v1/sources/qobuz/unofficial",
                Some(serde_json::json!({"enabled": true})),
            )
            .await;
            assert_eq!(st, StatusCode::OK);
        }
    };
    // Qobuz rotates its secret while its web player can't be reached: the stream fails for
    // now (503, not "no such track") and the source stays on
    *secret.write().unwrap() = "newSecret".into();
    down.store(true, SeqCst);
    qobuz::age_discovery(std::time::Duration::from_secs(6 * 60));
    assert_eq!(stream("5966783").await, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(looks.load(SeqCst), 2);
    assert_eq!(mode().await, (true, String::new()));
    // Within the minute after that, no look and no probe of the candidates found before:
    // still 503, and the source stays on
    down.store(false, SeqCst);
    let before = asked.load(SeqCst);
    assert_eq!(stream("5966783").await, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(asked.load(SeqCst), before + 1);
    assert_eq!(looks.load(SeqCst), 2);
    assert_eq!(mode().await, (true, String::new()));
    // A look that reaches the web player finds only the old values: Qobuz refuses them all,
    // so the source switches off and says why
    qobuz::age_discovery(std::time::Duration::from_secs(61));
    assert_eq!(stream("5966783").await, StatusCode::BAD_GATEWAY);
    assert_eq!(looks.load(SeqCst), 3);
    let (on, why) = mode().await;
    assert!(!on && why.contains("web player"), "{why}");
    // Those candidates are not probed again until a new look: one ask, the stream's own
    switch_on().await;
    let before = asked.load(SeqCst);
    assert_eq!(stream("5966783").await, StatusCode::BAD_GATEWAY);
    assert_eq!(asked.load(SeqCst), before + 1);
    assert_eq!(looks.load(SeqCst), 3);
    // A new look finds values that work for the probe, but this track's stream still answers
    // a signature 400 with them: the source switches off
    *bundle.write().unwrap() = qobuz_mock_bundle(&["newSecret"]);
    qobuz::age_discovery(std::time::Duration::from_secs(6 * 60));
    switch_on().await;
    assert_eq!(stream("7").await, StatusCode::BAD_GATEWAY);
    assert_eq!(looks.load(SeqCst), 4);
    let (on, why) = mode().await;
    assert!(!on && why.contains("refused the app secret"), "{why}");
    let cached = s
        .library
        .lock()
        .unwrap()
        .get_setting(qobuz::APP_KEY)
        .unwrap()
        .unwrap();
    assert!(cached.contains("newSecret"));
    // The refreshed values play every other track
    switch_on().await;
    assert_eq!(stream("5966783").await, StatusCode::OK);
    assert!(mode().await.0);
    server.abort();
}

#[tokio::test]
async fn qobuz_a_400_not_about_the_app_fails_only_that_sign_in() {
    let _api_guard = qobuz::API_TEST_LOCK.lock().await;
    use md5::Digest;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let bundle = qobuz_mock_bundle(&["s3cret"]);
    let looks = Arc::new(AtomicUsize::new(0));
    let mock = Router::new()
        .route("/login", get({
            let looks = looks.clone();
            move || async move {
                looks.fetch_add(1, SeqCst);
                "<script src=\"/resources/1.2.3-a004/bundle.js\"></script>"
            }
        }))
        .route("/resources/1.2.3-a004/bundle.js", get(move || { let bundle = bundle.clone(); async move { bundle } }))
        .route("/user/login", get(|axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
            let token = match q["email"].as_str() {
                "typo@example" => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"message": "Invalid email parameter"}))),
                "anna@example.com" => "t-anna",
                "odd@example.com" => "t-odd",
                _ => "t-owner",
            };
            (StatusCode::OK, Json(serde_json::json!({"user_auth_token": token, "user": {"id": 1, "credential": {"label": "Studio", "parameters": {"hires_streaming": true}}}})))
        }))
        .route("/track/getFileUrl", get(|headers: HeaderMap, axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
            if headers["x-user-auth-token"] == "t-odd" {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"message": "Something about this account"})));
            }
            let want = format!("{:x}", md5::Md5::digest(format!("trackgetFileUrlformat_id{}intentstreamtrack_id{}{}s3cret", q["format_id"], q["track_id"], q["request_ts"]).as_bytes()));
            if q["request_sig"] != want {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"message": "Invalid Request Signature parameter (request_sig)"})));
            }
            (StatusCode::OK, Json(serde_json::json!({"url": "https://stream.invalid/mock"})))
        }));
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    qobuz::set_api_base(&base);
    qobuz::set_player(&base);
    let dir = tempfile::tempdir().unwrap();
    let (s, anna, _) = two_people_node(dir.path(), "http://127.0.0.1:9").await;
    s.library
        .lock()
        .unwrap()
        .set_setting(accounts::SOURCE_POLICY_KEY, "own")
        .unwrap();
    let app = router(s.clone());
    let anna_token = accounts::mint(&s, anna).unwrap();
    let sign_in = |token: &str, email: &str| {
        let (app, token) = (app.clone(), token.to_string());
        let body = serde_json::json!({"email": email, "password": "hunter2"});
        async move {
            let (st, q, _) = send_as(
                &app,
                "POST",
                "/v1/sources/qobuz/unofficial/login",
                Some(&token),
                &[],
                Some(body),
            )
            .await;
            (st, q)
        }
    };
    let cached = |s: &Arc<NodeState>| s.library.lock().unwrap().get_setting(qobuz::APP_KEY).unwrap();
    let refused = "Qobuz refused this sign-in. Check your email address and try again.";
    let no_stream = "Qobuz would not stream for this account. Try again later.";
    // The owner's sign-in gets a 400 that is not about the app: theirs alone fails, and Anna's
    // right after it works with what the same look found
    let (st, q) = sign_in("s3cret", "typo@example").await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{q}");
    assert_eq!(q["error"], refused);
    assert_eq!(looks.load(SeqCst), 1);
    let (st, q) = sign_in(&anna_token, "anna@example.com").await;
    assert_eq!(st, StatusCode::OK, "{q}");
    assert_eq!(looks.load(SeqCst), 1);
    let app_values = cached(&s).unwrap();
    assert!(app_values.contains("s3cret"));
    // With values cached: the same 400 at sign-in, or a probe 400 that is not about the app,
    // changes nothing for anyone
    for (email, status, error) in [
        ("typo@example", StatusCode::BAD_REQUEST, refused),
        ("odd@example.com", StatusCode::BAD_GATEWAY, no_stream),
    ] {
        let (st, q) = sign_in("s3cret", email).await;
        assert_eq!(st, status, "{q}");
        assert_eq!(q["error"], error);
    }
    let (st, q) = sign_in(&anna_token, "anna@example.com").await;
    assert_eq!(st, StatusCode::OK, "{q}");
    assert_eq!(cached(&s).unwrap(), app_values);
    assert_eq!(looks.load(SeqCst), 1);
    // On a node with nothing cached, a probe 400 that is not about the app fails only that
    // sign-in: the candidates are not refused, the next sign-in uses them
    let other_dir = tempfile::tempdir().unwrap();
    let other = state(other_dir.path(), None).await;
    let other_app = router(other.clone());
    let body = |email: &str| Some(serde_json::json!({"email": email, "password": "hunter2"}));
    let (st, q) = call(
        &other_app,
        "POST",
        "/v1/sources/qobuz/unofficial/login",
        body("odd@example.com"),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_GATEWAY, "{q}");
    assert_eq!(q["error"], no_stream);
    assert_eq!(cached(&other), None);
    let (st, q) = call(
        &other_app,
        "POST",
        "/v1/sources/qobuz/unofficial/login",
        body("me@example.com"),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{q}");
    assert!(cached(&other).unwrap().contains("s3cret"));
    assert_eq!(looks.load(SeqCst), 1);
    let owner = s.library.lock().unwrap().get_setting(qobuz::STATE_KEY).unwrap();
    assert!(owner.is_none_or(|st| !st.contains("t-owner") && !st.contains("t-odd")));
    server.abort();
}

#[tokio::test]
async fn qobuz_cached_values_qobuz_retired_with_an_unexpected_400_do_not_block_sign_in() {
    let _api_guard = qobuz::API_TEST_LOCK.lock().await;
    use md5::Digest;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let bundle = qobuz_mock_bundle(&["s3cret"]);
    let looks = Arc::new(AtomicUsize::new(0));
    let sig = |q: &std::collections::HashMap<String, String>, secret: &str| {
        format!(
            "{:x}",
            md5::Md5::digest(
                format!(
                    "trackgetFileUrlformat_id{}intentstreamtrack_id{}{}{secret}",
                    q["format_id"], q["track_id"], q["request_ts"]
                )
                .as_bytes()
            )
        )
    };
    let mock = Router::new()
        .route("/login", get({
            let looks = looks.clone();
            move || async move {
                looks.fetch_add(1, SeqCst);
                "<script src=\"/resources/1.2.3-a004/bundle.js\"></script>"
            }
        }))
        .route("/resources/1.2.3-a004/bundle.js", get(move || { let bundle = bundle.clone(); async move { bundle } }))
        .route("/user/login", get(|axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
            // A retired app ID, and one account, get a 400 that says nothing
            if q["app_id"] == "999999999" || q["email"] == "typo@example" {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({})));
            }
            (StatusCode::OK, Json(serde_json::json!({"user_auth_token": "t-1", "user": {"id": 1, "credential": {"label": "Studio", "parameters": {"hires_streaming": true}}}})))
        }))
        .route("/track/getFileUrl", get(move |axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
            if q["request_sig"] == sig(&q, "s3cret") {
                return (StatusCode::OK, Json(serde_json::json!({"url": "https://stream.invalid/mock"})));
            }
            // A retired secret refused in words that are not about the signature
            if q["request_sig"] == sig(&q, "oldSecret") {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"message": "Not permitted"})));
            }
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({"message": "Invalid Request Signature parameter (request_sig)"})))
        }));
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    qobuz::set_api_base(&base);
    qobuz::set_player(&base);
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let sign_in = |email: &str| {
        let app = app.clone();
        let body = serde_json::json!({"email": email, "password": "hunter2"});
        async move { call(&app, "POST", "/v1/sources/qobuz/unofficial/login", Some(body)).await }
    };
    let cache = |values: &str| {
        s.library
            .lock()
            .unwrap()
            .set_setting(qobuz::APP_KEY, values)
            .unwrap()
    };
    let cached = || {
        s.library
            .lock()
            .unwrap()
            .get_setting(qobuz::APP_KEY)
            .unwrap()
            .unwrap()
    };

    // The cached app ID's `user/login` answers `400 {}`: the web player's candidate works, the
    // sign-in succeeds and caches it
    cache(r#"{"id":"999999999","secret":"s3cret","from":"web","bundle_url":"old"}"#);
    let (st, q) = sign_in("me@example.com").await;
    assert_eq!(st, StatusCode::OK, "{q}");
    assert_eq!(looks.load(SeqCst), 1);
    let app_values = cached();
    assert!(
        app_values.contains("100000000") && app_values.contains("s3cret"),
        "{app_values}"
    );

    // A 400 that the web player's values get too is this person's own: it stands, nothing is
    // refused or looked at again, and the next sign-in works
    let (st, q) = sign_in("typo@example").await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{q}");
    assert_eq!(
        q["error"],
        "Qobuz refused this sign-in. Check your email address and try again."
    );
    assert_eq!(looks.load(SeqCst), 1);
    assert_eq!(cached(), app_values);
    let (st, q) = sign_in("me@example.com").await;
    assert_eq!(st, StatusCode::OK, "{q}");

    // The cached secret's stream check answers a 400 not about the signature: the web
    // player's other secret is tried and cached
    cache(r#"{"id":"100000000","secret":"oldSecret","from":"web","bundle_url":"old"}"#);
    let (st, q) = sign_in("me@example.com").await;
    assert_eq!(st, StatusCode::OK, "{q}");
    assert_eq!(looks.load(SeqCst), 1);
    assert!(cached().contains("s3cret"));
    server.abort();
}
