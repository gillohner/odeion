//! TIDAL: sign-in ways, search and streams against a mock.
//!
//! Part of pm-api's unit tests (`mod tests` in lib.rs, which holds the helpers these use). A new
//! test of this area goes at the end of this file.

use super::*;

/// TIDAL full tracks (unofficial) against a mock of TIDAL's auth and private API: the device
/// sign-in, a BTS stream and a DASH stream read whole, an encrypted track refused, and a
/// refused client switching the mode off (playback then falls back to the official path).
#[tokio::test]
async fn tidal_full_tracks_sign_in_and_stream_from_a_mock() {
    use base64::Engine;
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let b64 = |v: &str| base64::engine::general_purpose::STANDARD.encode(v);
    let polls = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let mpd = format!(
        r#"<MPD><Period><AdaptationSet mimeType="audio/mp4"><Representation codecs="flac" audioSamplingRate="96000"><SegmentTemplate initialization="{base}/seg/init" media="{base}/seg/$Number$" startNumber="1"><SegmentTimeline><S d="1" r="1"/></SegmentTimeline></SegmentTemplate></Representation></AdaptationSet></Period></MPD>"#
    );
    let bts = format!(
        r#"{{"mimeType":"audio/flac","codecs":"flac","encryptionType":"NONE","urls":["{base}/file.flac"]}}"#
    );
    let enc = format!(
        r#"{{"mimeType":"audio/flac","codecs":"flac","encryptionType":"OLD_AES","urls":["{base}/file.flac"]}}"#
    );
    let mock = Router::new()
        .route(
            "/v1/oauth2/device_authorization",
            post(|body: String| async move {
                // An app from TIDAL's developer portal gets TIDAL's refusal
                if body.contains("client_id=portal-app") {
                    return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "invalid_request", "error_description": "Client is not a Limited Input Device client"})));
                }
                if body.contains("client_id=nobody") {
                    return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "invalid_client"})));
                }
                assert!(body.contains("client_id=dev-id"), "{body}");
                (StatusCode::OK, Json(serde_json::json!({"deviceCode": "DC", "userCode": "ABCDE", "verificationUriComplete": "link.tidal.com/ABCDE", "expiresIn": 300, "interval": 1})))
            }),
        )
        .route(
            "/v1/oauth2/token",
            post({
                let polls = polls.clone();
                move |body: String| {
                    let polls = polls.clone();
                    async move {
                        if !body.contains("client_secret=dev-secret") {
                            return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "invalid_client", "error_description": "Client authentication failed"})));
                        }
                        // The first token request (the check's, below) waits; the owner approves at the next
                        if polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "authorization_pending"})));
                        }
                        (StatusCode::OK, Json(serde_json::json!({"access_token": "AT", "refresh_token": "RT", "expires_in": 3600})))
                    }
                }
            }),
        )
        .route(
            "/v1/sessions",
            get(|| async { Json(serde_json::json!({"userId": 42, "countryCode": "CH", "sessionId": "sid"})) }),
        )
        .route(
            "/v1/tracks/{id}/playbackinfopostpaywall",
            get(move |Path(id): Path<String>, q: axum::extract::Query<std::collections::HashMap<String, String>>| {
                let (mpd, bts, enc) = (mpd.clone(), bts.clone(), enc.clone());
                async move {
                    assert_eq!(q.get("countryCode").map(String::as_str), Some("CH"));
                    assert_eq!(q.get("audioquality").map(String::as_str), Some("HI_RES_LOSSLESS"));
                    let (kind, m) = match id.as_str() {
                        "1" => ("application/vnd.tidal.bts", bts),
                        "2" => ("application/dash+xml", mpd),
                        "3" => ("application/vnd.tidal.bts", enc),
                        _ => return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"userMessage": "client not allowed"}))),
                    };
                    (StatusCode::OK, Json(serde_json::json!({"assetPresentation": "FULL", "manifestMimeType": kind, "manifest": b64(&m)})))
                }
            }),
        )
        .route("/file.flac", get(move || { let f = flac.clone(); async move { f } }))
        .route("/seg/{n}", get(|Path(n): Path<String>| async move { format!("[{n}]") }));
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    {
        let mut ep = s.tidal.write().unwrap();
        ep.auth_base = base.clone();
        ep.v1_base = format!("{base}/v1");
    }
    let app = router(s.clone());
    let (_, st) = call(&app, "GET", "/v1/sources/tidal/unofficial", None).await;
    assert_eq!(st["enabled"], false, "off by default");
    assert!(st["warning"].as_str().unwrap().contains("not approved"));
    let (st, r) = call(&app, "POST", "/v1/sources/tidal/unofficial/login", None).await;
    assert_eq!(st, StatusCode::BAD_GATEWAY, "no device client yet: {r}");
    // A developer-portal app is not a device client: plain words, not TIDAL's raw error
    let (_, _) = call(
        &app,
        "PUT",
        "/v1/sources/tidal/unofficial",
        Some(serde_json::json!({"client_id": "portal-app", "client_secret": "portal-secret"})),
    )
    .await;
    let (st, r) = call(&app, "POST", "/v1/sources/tidal/unofficial/login", None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{r}");
    assert_eq!(r["error"], tidal_unofficial::NOT_A_DEVICE_CLIENT, "{r}");
    // The client check (the step card's Check): an unknown ID, a portal app, a wrong secret and
    // a good client, each in its own words, nothing saved
    let check = |id: &str, secret: &str| {
        let app = app.clone();
        let body = serde_json::json!({"client_id": id, "client_secret": secret});
        async move {
            call(
                &app,
                "POST",
                "/v1/sources/tidal/unofficial/client/check",
                Some(body),
            )
            .await
            .1
        }
    };
    let r = check("nobody", "x").await;
    assert_eq!(r["ok"], false, "{r}");
    assert!(
        r["detail"]
            .as_str()
            .unwrap()
            .contains("doesn't know this client ID"),
        "{r}"
    );
    let r = check("portal-app", "x").await;
    assert_eq!(
        (r["ok"].as_bool(), r["detail"].as_str()),
        (Some(false), Some(tidal_unofficial::NOT_A_DEVICE_CLIENT)),
        "{r}"
    );
    let r = check("dev-id", "wrong").await;
    assert!(
        r["ok"] == false && r["detail"].as_str().unwrap().contains("secret"),
        "{r}"
    );
    let r = check("dev-id", "dev-secret").await;
    assert!(
        r["ok"] == true && r["detail"].as_str().unwrap().starts_with("OK"),
        "{r}"
    );
    let (_, st) = call(&app, "GET", "/v1/sources/tidal/unofficial", None).await;
    assert_eq!(
        st["client_from"], "settings",
        "a check saves nothing: the portal app is still set: {st}"
    );
    let (_, r) = call(
        &app,
        "PUT",
        "/v1/sources/tidal/unofficial",
        Some(serde_json::json!({"client_id": "dev-id", "client_secret": "dev-secret"})),
    )
    .await;
    assert_eq!(r["client_from"], "settings");
    let (st, r) = call(&app, "POST", "/v1/sources/tidal/unofficial/login", None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["pending"]["user_code"], "ABCDE");
    assert_eq!(r["pending"]["uri"], "https://link.tidal.com/ABCDE");
    let mut signed = serde_json::Value::Null;
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let (_, r) = call(&app, "GET", "/v1/sources/tidal/unofficial", None).await;
        if r["signed_in"] == true {
            signed = r;
            break;
        }
    }
    assert_eq!(signed["user"], "42", "{signed}");
    assert_eq!(signed["enabled"], true, "signing in switches the mode on");
    assert!(signed.get("pending").is_none());

    let open = |id: &'static str| {
        let s = s.clone();
        tokio::task::spawn_blocking(move || {
            use std::io::Read;
            let ep = s.tidal.read().unwrap().clone();
            let r = crate::tidal_unofficial::open_blocking(&s.library, &ep, id, &accounts::Who::Owner);
            crate::tidal_unofficial::record(&s.library, &accounts::Who::Owner, &r);
            r.map(|o| {
                o.map(|mut f| {
                    let mut b = Vec::new();
                    f.reader.read_to_end(&mut b).unwrap();
                    (f.ext, b)
                })
            })
        })
    };
    let (ext, bytes) = open("1").await.unwrap().unwrap().unwrap();
    assert_eq!(ext, "flac");
    assert!(
        bytes.starts_with(b"fLaC") && bytes.len() > 1000,
        "the whole file through BTS"
    );
    let (ext, bytes) = open("2").await.unwrap().unwrap().unwrap();
    assert_eq!(
        (ext, String::from_utf8(bytes).unwrap().as_str()),
        ("m4a", "[init][1][2]"),
        "DASH: init, then the segments in order"
    );
    match open("3").await.unwrap() {
        Err(crate::tidal_unofficial::FullError::Track(e)) => assert!(e.contains("encrypted"), "{e}"),
        other => panic!(
            "an encrypted track is refused, never decrypted: {:?}",
            other.map(|o| o.map(|x| x.0))
        ),
    }
    let (_, r) = call(&app, "GET", "/v1/sources/tidal/unofficial", None).await;
    assert_eq!(r["enabled"], true, "one track's trouble leaves the mode on");
    match open("9").await.unwrap() {
        Err(crate::tidal_unofficial::FullError::Mode(e)) => {
            assert!(e.contains("client not allowed"), "{e}")
        }
        other => panic!(
            "a refused client is the mode's failure: {:?}",
            other.map(|o| o.map(|x| x.0))
        ),
    }
    let (_, r) = call(&app, "GET", "/v1/sources/tidal/unofficial", None).await;
    assert_eq!(r["enabled"], false, "the mode switched itself off: {r}");
    assert_eq!(r["health"]["ok"], false);
    assert_eq!(r["signed_in"], true, "the sign-in is kept for Retry");
    assert!(
        open("1").await.unwrap().unwrap().is_none(),
        "off: the official path plays"
    );
    let (_, r) = call(
        &app,
        "PUT",
        "/v1/sources/tidal/unofficial",
        Some(serde_json::json!({"enabled": true})),
    )
    .await;
    assert!(r["enabled"] == true && r.get("health").is_none(), "Retry: {r}");
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/sources/tidal/unofficial",
        Some(serde_json::json!({"quality": "best"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

/// Behind a reverse proxy with no public_url, the TIDAL redirect URI is
/// built from the request the browser made.
#[tokio::test]
async fn the_tidal_redirect_follows_the_request_host() {
    let dir = tempfile::tempdir().unwrap();
    // A node reached by another name than localhost has a token
    let s = state(dir.path(), Some("s3cret")).await;
    // The node's own app, with its secret: the sign-in comes straight back
    s.tidal.write().unwrap().creds = tidal::TidalCredentials {
        client_id: "cid".into(),
        client_secret: "sec".into(),
        scopes: String::new(),
    };
    let app = router(s);
    let req = Request::builder()
        .uri("/v1/sources/tidal/status")
        .header("host", "music.example")
        .header("x-forwarded-proto", "https")
        .header("authorization", "Bearer s3cret")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["redirect_uri"], "https://music.example/tidal/callback");
}

#[tokio::test]
async fn tidal_searches_signs_in_and_lists_albums() {
    use axum::extract::RawQuery;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let tokens = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let tokens2 = tokens.clone();
    let album = serde_json::json!({"id": "20115556", "type": "albums", "attributes": {"title": "Random Access Memories", "releaseDate": "2013-05-17", "mediaTags": ["LOSSLESS"], "barcodeId": "886443927087"},
        "relationships": {"artists": {"data": [{"id": "3996", "type": "artists"}]}, "coverArt": {"data": [{"id": "art1", "type": "artworks"}]},
                          "items": {"data": [{"id": "20115564", "type": "tracks", "meta": {"trackNumber": 8, "volumeNumber": 1}}, {"id": "20115557", "type": "tracks", "meta": {"trackNumber": 1, "volumeNumber": 1}}]}}});
    let artist = serde_json::json!({"id": "3996", "type": "artists", "attributes": {"name": "Daft Punk"}});
    let art = serde_json::json!({"id": "art1", "type": "artworks", "attributes": {"files": [{"href": "https://img/160.jpg", "meta": {"width": 160, "height": 160}}, {"href": "https://img/640.jpg", "meta": {"width": 640, "height": 640}}]}});
    let track = |id: &str, title: &str| {
        serde_json::json!({"id": id, "type": "tracks", "attributes": {"title": title, "isrc": "USQX91300108", "duration": "PT6M9S", "mediaTags": ["HIRES_LOSSLESS"]},
        "relationships": {"artists": {"data": [{"id": "3996", "type": "artists"}]}, "albums": {"data": [{"id": "20115556", "type": "albums"}]}}})
    };
    let (t1, t2) = (
        track("20115564", "Get Lucky"),
        track("20115557", "Give Life Back to Music"),
    );
    let mock = Router::new()
        .route(
            "/v1/oauth2/token",
            post(move |body: String| {
                let tokens = tokens2.clone();
                async move {
                    let grant = body.split('&').find(|p| p.starts_with("grant_type=")).unwrap_or("").to_string();
                    tokens.lock().unwrap().push(body.clone());
                    if grant.ends_with("client_credentials") {
                        Json(serde_json::json!({"access_token": "app-token", "expires_in": 14400, "token_type": "Bearer"}))
                    } else {
                        assert!(body.contains("code_verifier="), "{body}");
                        assert!(body.contains("code=the-code"), "{body}");
                        Json(serde_json::json!({"access_token": "user-token", "refresh_token": "refresh-1", "expires_in": 3600, "user_id": 4242, "scope": "playback collection.read"}))
                    }
                }
            }),
        )
        .route("/users/{id}", get(|| async { Json(serde_json::json!({"data": {"id": "4242", "type": "users", "attributes": {"country": "DE"}}})) }))
        .route(
            "/searchResults",
            get({
                let (album, t1) = (album.clone(), t1.clone());
                move |RawQuery(q): RawQuery| {
                    let (album, t1) = (album.clone(), t1.clone());
                    async move {
                        let q = q.unwrap_or_default();
                        assert!(q.contains("filter%5Bquery%5D=daft+punk") || q.contains("filter%5Bquery%5D=daft%20punk"), "{q}");
                        if q.contains("include=albums") {
                            Json(serde_json::json!({"data": [{"id": "s", "type": "searchResults", "relationships": {"albums": {"data": [{"id": "20115556", "type": "albums"}]}}}], "included": [album]}))
                        } else if q.contains("include=artists") {
                            Json(serde_json::json!({"data": [{"id": "s", "type": "searchResults", "relationships": {"artists": {"data": [{"id": "3996", "type": "artists"}]}}}]}))
                        } else {
                            Json(serde_json::json!({"data": [{"id": "s", "type": "searchResults", "relationships": {"tracks": {"data": [{"id": "20115564", "type": "tracks"}]}}}], "included": [t1]}))
                        }
                    }
                }
            }),
        )
        .route("/albums", get({ let (album, artist, art) = (album.clone(), artist.clone(), art.clone()); move || async move { Json(serde_json::json!({"data": [album], "included": [artist, art]})) } }))
        .route("/albums/{id}", get({ let (album, artist, art, t1, t2) = (album.clone(), artist.clone(), art.clone(), t1.clone(), t2.clone()); move || async move { Json(serde_json::json!({"data": album, "included": [artist, art, t1, t2]})) } }))
        .route("/tracks", get({ let (t1, artist, album) = (t1.clone(), artist.clone(), album.clone()); move || async move { Json(serde_json::json!({"data": [t1], "included": [artist, album]})) } }))
        .route("/tracks/{id}", get({ let (t1, artist, album, art) = (t1.clone(), artist.clone(), album.clone(), art.clone()); move || async move { Json(serde_json::json!({"data": t1, "included": [artist, album, art]})) } }))
        .route(
            "/userCollectionTracks/{uid}/relationships/items",
            get(|Path(uid): Path<String>| async move {
                assert_eq!(uid, "me");
                Json(serde_json::json!({"data": [{"id": "20115564", "type": "tracks"}]}))
            }),
        )
        .route(
            "/userCollectionAlbums/{uid}/relationships/items",
            get(|| async { Json(serde_json::json!({"data": [{"id": "20115556", "type": "albums"}]})) }),
        )
        .route(
            "/playlists",
            get(|RawQuery(q): RawQuery| async move {
                assert!(q.unwrap_or_default().contains("owners.id%5D=4242"), "owner filter");
                Json(serde_json::json!({"data": [{"id": "pl-1", "type": "playlists", "attributes": {"name": "Road trip", "description": "", "numberOfTrackItems": 2}}]}))
            }),
        )
        .route(
            "/playlists/{id}",
            get(|| async { Json(serde_json::json!({"data": {"id": "pl-1", "type": "playlists", "attributes": {"name": "Road trip"}}})) }),
        )
        .route(
            "/playlists/{id}/relationships/items",
            get({
                let (t1, t2) = (t1.clone(), t2.clone());
                move || async move {
                    let mut t2 = t2.clone();
                    t2["attributes"]["isrc"] = serde_json::Value::Null;
                    Json(serde_json::json!({"data": [{"id": "20115564", "type": "tracks"}, {"id": "20115557", "type": "tracks"}, {"id": "v1", "type": "videos"}], "included": [t1, t2]}))
                }
            }),
        )
        .route(
            "/artists",
            get(|| async {
                Json(serde_json::json!({"data": [{"id": "3996", "type": "artists", "attributes": {"name": "Daft Punk", "numberOfFollowers": 12}, "relationships": {"profileArt": {"data": [{"id": "pa1", "type": "artworks"}]}}}],
                    "included": [{"id": "pa1", "type": "artworks", "attributes": {"files": [{"href": "https://img/artist.jpg", "meta": {"width": 640, "height": 640}}]}}]}))
            }),
        )
        .route(
            "/artists/{id}/relationships/albums",
            get(|| async { Json(serde_json::json!({"data": [{"id": "20115556", "type": "albums"}]})) }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    {
        let mut ep = s.tidal.write().unwrap();
        ep.api_base = base.clone();
        ep.auth_base = base.clone();
        ep.login_base = base.clone();
    }
    let app = router(s.clone());
    // Not configured: the source says so and login refuses.
    let (_, srcs) = call(&app, "GET", "/v1/sources", None).await;
    let tidal = srcs
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "tidal")
        .unwrap();
    assert!(tidal["status"].as_str().unwrap().starts_with("not configured"));
    let (st, _) = call(&app, "GET", "/v1/sources/tidal/login", None).await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED);
    s.tidal.write().unwrap().creds = tidal::TidalCredentials {
        client_id: "cid".into(),
        client_secret: "sec".into(),
        scopes: String::new(),
    };
    // Search with the app token: albums carry artists and art, tracks carry artists.
    let (st, r) = call(
        &app,
        "GET",
        "/v1/sources/tidal/search?q=daft%20punk&limit=10",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    let items = r["items"].as_array().unwrap();
    assert_eq!(items[0]["kind"], "artist", "artists come first: {r}");
    assert_eq!(items[0]["id"], "artist:3996");
    assert_eq!(items[0]["image"], "https://img/artist.jpg");
    assert_eq!(items[1]["kind"], "album");
    assert_eq!(items[1]["creator"], "Daft Punk");
    assert_eq!(items[1]["image"], "https://img/640.jpg");
    assert_eq!(items[1]["url"], "https://tidal.com/browse/album/20115556");
    assert_eq!(items[2]["kind"], "track");
    assert_eq!(items[2]["title"], "Get Lucky");
    assert_eq!(items[2]["creator"], "Daft Punk");
    assert_eq!(items[2]["duration_ms"], 369_000);
    // An artist page lists the artist's albums with library matches.
    let (st, a) = call(&app, "GET", "/v1/sources/tidal/artists/artist:3996", None).await;
    assert_eq!(st, StatusCode::OK, "{a}");
    assert_eq!(a["name"], "Daft Punk");
    assert_eq!(a["albums"][0]["id"], "album:20115556");
    let (st, _) = call(&app, "GET", "/v1/sources/archive/artists/x", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert!(tokens
        .lock()
        .unwrap()
        .iter()
        .any(|b| b.contains("client_credentials")));
    // An album item lists its tracks in order with hi-res labels and tidal: URIs.
    let (st, it) = call(&app, "GET", "/v1/sources/tidal/items/album:20115556", None).await;
    assert_eq!(st, StatusCode::OK, "{it}");
    assert_eq!(it["title"], "Random Access Memories");
    assert_eq!(it["creator"], "Daft Punk");
    assert_eq!(it["year"], 2013);
    let tracks = it["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0]["track_no"], 8);
    assert_eq!(tracks[0]["url"], "tidal:track:20115564");
    assert_eq!(tracks[0]["duration_ms"], 369_000);
    assert_eq!(tracks[0]["format"], "FLAC hi-res (TIDAL)");
    let (st, _) = call(&app, "GET", "/v1/sources/tidal/items/nope", None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    // Unified search: the library answer carries TIDAL hits, flagged when the
    // library already has the same ISRC or UPC.
    {
        let lib = s.library.lock().unwrap();
        let mut t = pm_model::Track {
            id: None,
            title: "Get Lucky".into(),
            artists: vec![pm_model::Artist {
                id: None,
                name: "Daft Punk".into(),
                mbid: None,
            }],
            album: Some(pm_model::Album {
                id: None,
                title: "Random Access Memories".into(),
                album_artist: Some("Daft Punk".into()),
                release_mbid: None,
                release_group_mbid: None,
                year: Some(2013),
                upc: pm_model::Upc::parse("886443927087").ok(),
                total_discs: None,
            }),
            track_no: Some(8),
            disc_no: None,
            duration_ms: Some(369_000),
            file: pm_model::AudioFile {
                path: dir.path().join("get-lucky.flac"),
                size_bytes: 1,
                mtime_unix: 0,
                container: "flac".into(),
                format: pm_model::AudioFormat::pcm(44_100, 16, 2),
                duration_ms: Some(369_000),
            },
            recording_mbid: None,
            isrc: pm_model::Isrc::parse("USQX91300108").ok(),
            acoustid: None,
            canonical: Some((
                pm_model::MusicUri::Isrc {
                    isrc: pm_model::Isrc::parse("USQX91300108").unwrap(),
                },
                pm_model::Confidence::TaggedIsrc,
            )),
            replaygain_track_gain_db: None,
            replaygain_album_gain_db: None,
            genres: vec![],
        };
        t.id = None;
        lib.upsert_track(&t).unwrap();
    }
    let (st, r) = call(&app, "GET", "/v1/library/search?q=daft%20punk", None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["tracks"][0]["title"], "Get Lucky");
    let src = r["sources"].as_array().unwrap();
    assert_eq!(src.len(), 1, "{r}");
    assert_eq!(src[0]["source"], "tidal");
    let tidal_hits = src[0]["items"].as_array().unwrap();
    let track_hit = tidal_hits.iter().find(|h| h["kind"] == "track").unwrap();
    assert_eq!(track_hit["isrc"], "USQX91300108");
    assert!(track_hit["library_track_id"].is_i64(), "{track_hit}");
    let album_hit = tidal_hits.iter().find(|h| h["kind"] == "album").unwrap();
    assert_eq!(album_hit["upc"], "886443927087");
    assert!(album_hit["library_album_id"].is_i64(), "{album_hit}");
    let (_, r) = call(&app, "GET", "/v1/library/search?q=daft%20punk&sources=0", None).await;
    assert!(r["sources"].as_array().unwrap().is_empty());
    // Versions: the library album and track point back at TIDAL by UPC / ISRC.
    let lid = track_hit["library_track_id"].as_i64().unwrap();
    let aid = album_hit["library_album_id"].as_i64().unwrap();
    let (st, v) = call(&app, "GET", &format!("/v1/library/albums/{aid}/versions"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v[0]["id"], "album:20115556");
    assert!(v[0]["label"].as_str().unwrap().starts_with("TIDAL"));
    let (st, v) = call(&app, "GET", &format!("/v1/library/tracks/{lid}/versions"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v[0]["uri"], "tidal:track:20115564");
    // Sign-in: the login URL carries PKCE, the callback exchanges the code and stores the user.
    let (st, l) = call(&app, "GET", "/v1/sources/tidal/login", None).await;
    assert_eq!(st, StatusCode::OK);
    let url = url::Url::parse(l["url"].as_str().unwrap()).unwrap();
    let q: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(q["code_challenge_method"], "S256");
    assert!(q["scope"].contains("playback"));
    assert!(q["redirect_uri"].ends_with("/tidal/callback"));
    let state_param = q["state"].clone();
    let req = Request::builder()
        .uri(format!("/tidal/callback?code=the-code&state={state_param}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER, "callback redirects");
    assert_eq!(resp.headers().get("location").unwrap(), "/?tidal=signed_in");
    let (_, stv) = call(&app, "GET", "/v1/sources/tidal/status", None).await;
    assert_eq!(stv["signed_in"], true, "{stv}");
    assert_eq!(stv["user_id"], "4242");
    assert_eq!(stv["country"], "DE");
    let (_, srcs) = call(&app, "GET", "/v1/sources", None).await;
    let tidal = srcs
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "tidal")
        .unwrap();
    assert_eq!(tidal["status"], "ready");
    // Signed in: the collection lists, and imports land as favourites and a Pubky playlist.
    let (st, c) = call(&app, "GET", "/v1/sources/tidal/collection", None).await;
    assert_eq!(st, StatusCode::OK, "{c}");
    assert_eq!(c["tracks"][0]["title"], "Get Lucky");
    assert_eq!(c["albums"][0]["creator"], "Daft Punk");
    let (st, r) = call(&app, "POST", "/v1/sources/tidal/collection/import", None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["imported"], 2);
    let (_, favs) = call(&app, "GET", "/v1/bookmarks", None).await;
    let uris: Vec<&str> = favs["uris"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|u| u.as_str())
        .collect();
    assert!(
        uris.contains(&"https://musicbrainz.org/isrc/USQX91300108"),
        "{uris:?}"
    );
    assert!(
        uris.iter().any(|u| u.contains("tidal.com/album/20115556")),
        "{uris:?}"
    );
    let (st, pls) = call(&app, "GET", "/v1/sources/tidal/playlists", None).await;
    assert_eq!(st, StatusCode::OK, "{pls}");
    assert_eq!(pls[0]["name"], "Road trip");
    assert_eq!(pls[0]["items"], 2);
    let (st, r) = call(&app, "POST", "/v1/sources/tidal/playlists/pl-1/import", None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(
        r["imported"], 2,
        "ISRC for one, the TIDAL link for the other; the video is skipped"
    );
    assert_eq!(r["skipped"], 1);
    let items = r["playlist"]["playlist"]["items"].as_array().unwrap();
    assert_eq!(items[0]["uri"], "https://musicbrainz.org/isrc/USQX91300108");
    assert!(items[1]["uri"]
        .as_str()
        .unwrap()
        .contains("tidal.com/track/20115557"));
    assert!(items[0].get("source").is_none(), "Auto unless asked");
    // "Keep TIDAL as the source": every song is pinned to its TIDAL track
    let (st, r) = call(
        &app,
        "POST",
        "/v1/sources/tidal/playlists/pl-1/import?keep_source=true",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    let items = r["playlist"]["playlist"]["items"].as_array().unwrap();
    assert_eq!(
        items[0]["uri"], "https://musicbrainz.org/isrc/USQX91300108",
        "items stay canonical"
    );
    assert_eq!(items[0]["source"]["service"], "tidal");
    assert!(
        items[0]["source"]["uri"].as_str().unwrap().contains("tidal.com/"),
        "{}",
        items[0]
    );
    assert_eq!(items[1]["source"]["service"], "tidal");
    // A stale state is refused; sign-out forgets the tokens.
    let req = Request::builder()
        .uri("/tidal/callback?code=x&state=unknown")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert!(resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("/?tidal=error"));
    let (_, stv) = call(&app, "DELETE", "/v1/sources/tidal/session", None).await;
    assert_eq!(stv["signed_in"], false);
    let (st, _) = call(&app, "GET", "/v1/sources/tidal/collection", None).await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED);
}

#[tokio::test]
async fn tidal_tracks_play_through_the_manifest_and_hls_segments() {
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let seg_len = flac.len().div_ceil(3);
    let segs: Vec<Vec<u8>> = flac.chunks(seg_len).map(|c| c.to_vec()).collect();
    let manifest_base = base.clone();
    let mock = Router::new()
        .route(
            "/trackManifests/{id}",
            get(move |Path(id): Path<String>, headers: HeaderMap| {
                let manifest_base = manifest_base.clone();
                async move {
                    assert_eq!(id, "20115564");
                    assert_eq!(headers.get("authorization").unwrap(), "Bearer user-token");
                    Json(serde_json::json!({"data": {"id": id, "type": "trackManifests", "attributes": {
                        "uri": format!("{manifest_base}/hls/master.m3u8"), "formats": ["FLAC"], "trackPresentation": "FULL", "hash": "h"}}}))
                }
            }),
        )
        .route("/hls/master.m3u8", get(|| async { "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000000\nmedia.m3u8\n" }))
        .route("/hls/media.m3u8", get(|| async { "#EXTM3U\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:1.0,\nseg/0\n#EXTINF:1.0,\nseg/1\n#EXTINF:1.0,\n/hls/seg/2\n#EXT-X-ENDLIST\n" }))
        .route(
            "/hls/seg/{n}",
            get(move |Path(n): Path<usize>| {
                let segs = segs.clone();
                async move { ([(header::CONTENT_TYPE, "application/octet-stream")], segs[n].clone()) }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    {
        let mut ep = s.tidal.write().unwrap();
        ep.api_base = base.clone();
        ep.auth_base = base.clone();
        ep.creds = tidal::TidalCredentials {
            client_id: "cid".into(),
            client_secret: "sec".into(),
            scopes: String::new(),
        };
    }
    s.library
        .lock()
        .unwrap()
        .set_setting(
            "tidal.user",
            &serde_json::json!({"access_token": "user-token", "refresh_token": "r", "expires_at": pm_library::db::now_unix() + 3600, "user_id": "4242", "scope": "playback"}).to_string(),
        )
        .unwrap();
    let app = router(s.clone());
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    let mut events = zone.subscribe();
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"items": [{"uri": "tidal:track:20115564", "title": "Get Lucky"}], "replace": true, "play": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let mut started = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_secs(1), events.recv()).await {
            Ok(Ok(pm_zones::ZoneEvent::TrackStarted { source_format, .. })) => {
                started = true;
                assert_eq!(source_format.rate_hz, 44_100);
            }
            Ok(Ok(pm_zones::ZoneEvent::Error { message })) => panic!("playback error: {message}"),
            Ok(Ok(pm_zones::ZoneEvent::Stopped { .. })) if started => break,
            Ok(Ok(_)) => {}
            Ok(Err(_)) => break,
            Err(_) => {}
        }
    }
    assert!(started, "the TIDAL track never started");
    let st = zone.wait_stopped().await;
    assert!(st.error.is_none(), "{:?}", st.error);
    // The browser proxy serves the same segments as one body.
    let req = Request::builder()
        .uri("/v1/sources/tidal/stream/20115564")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("audio/mp4"));
    assert_eq!(resp.headers().get("accept-ranges").unwrap(), "bytes");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(body.len(), flac.len(), "proxy body is the whole file");
    assert_eq!(&body[..], &flac[..]);
    // Seeking: a byte range comes back as 206 with the right slice.
    let req = Request::builder()
        .uri("/v1/sources/tidal/stream/20115564")
        .header("range", "bytes=100-199")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        resp.headers().get("content-range").unwrap(),
        &format!("bytes 100-199/{}", flac.len())
    );
    let part = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&part[..], &flac[100..200]);
    // Without a session the resolver refuses with a clear reason.
    s.library.lock().unwrap().delete_setting("tidal.user").unwrap();
    let (_, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"items": [{"uri": "tidal:track:20115564"}], "replace": true, "play": true})),
    )
    .await;
    let st = zone.wait_stopped().await;
    assert!(
        st.error.as_deref().is_some_and(|e| e.contains("sign in")),
        "{:?}",
        st.error
    );
}

/// Sources › TIDAL › Sign in: the ways side by side, the picked one remembered, what is in
/// use (user and expiry, never a token) and one line for the overview
/// (STEWARD-CREDENTIALS-REUSE.md).
#[tokio::test]
async fn tidal_sign_in_lists_the_ways_and_remembers_the_choice() {
    let _home_guard = imports::HOME_TEST_LOCK.lock().await;
    let home = tempfile::tempdir().unwrap();
    *imports::HOME_OVERRIDE.write().unwrap() = Some(home.path().to_path_buf());
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    // A build without TIDAL's built-in clients
    s.tidal.write().unwrap().browser_client = None;
    s.tidal.write().unwrap().creds = tidal::TidalCredentials {
        client_id: "own-app".into(),
        client_secret: String::new(),
        scopes: String::new(),
    };
    let app = router(s.clone());
    let signin = "/v1/sources/tidal/signin";
    // Without the build's clients: Sign in with TIDAL and the code sign-in are listed but not
    // available, each saying why; the device client of your own is under Advanced
    let (st, si) = call(&app, "GET", signin, None).await;
    assert_eq!(st, StatusCode::OK, "{si}");
    let ids: Vec<&str> = si["ways"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        ["browser", "device", "app", "own-device", "paste"],
        "no import way: nothing found"
    );
    assert!(
        si["ways"]
            .as_array()
            .unwrap()
            .iter()
            .all(|w| w["recommended"] == false),
        "{si}"
    );
    assert_eq!(si["ways"][0]["available"], false);
    assert_eq!(si["ways"][1]["available"], false);
    assert!(
        si["ways"][1]["reason"]
            .as_str()
            .unwrap()
            .contains("Advanced › Your own device client"),
        "{si}"
    );
    assert_eq!(
        si["ways"][2]["label"], "Your own TIDAL app",
        "no project app in a test build"
    );
    assert_eq!(
        (
            si["ways"][2]["available"].as_bool(),
            si["ways"][2]["advanced"].as_bool()
        ),
        (Some(true), Some(true))
    );
    assert_eq!(
        si["ways"][3]["available"], true,
        "the owner sets a device client up"
    );
    assert_eq!(si["ways"][4]["available"], true);
    assert!(si["in_use"].is_null(), "{si}");
    assert_eq!(si["line"], "Not signed in");
    assert_eq!(si["details"]["unofficial"]["enabled"], false);
    assert_eq!(si["details"]["status"]["signed_in"], false);
    // Something that is not a way is refused in plain words
    let (st, r) = call(&app, "PUT", signin, Some(serde_json::json!({"way": "nope"}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{r}");
    assert!(
        r["error"]
            .as_str()
            .unwrap()
            .contains("one of browser, device, app, own-device, paste"),
        "{r}"
    );
    // Your own device client makes the device way available, labelled as yours; picking it is
    // what the Full tracks switch used to do
    call(
        &app,
        "PUT",
        "/v1/sources/tidal/unofficial",
        Some(serde_json::json!({"client_id": "dev-1", "client_secret": "s"})),
    )
    .await;
    let (st, r) = call(&app, "PUT", signin, Some(serde_json::json!({"way": "device"}))).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["chosen"], "device");
    assert_eq!(r["ways"][1]["label"], "Your own device client");
    assert_eq!(r["ways"][1]["available"], true);
    assert_eq!(r["details"]["unofficial"]["enabled"], true, "{r}");
    assert!(r["in_use"].is_null(), "picked, not signed in yet: {r}");
    // The approval stores a session: it is in use, shown by user and expiry, never the token
    {
        let lib = s.library.lock().unwrap();
        let mut m = tidal_unofficial::load(&lib, &accounts::Who::Owner);
        m.session = Some(tidal_unofficial::Session {
            access_token: "secret-token".into(),
            refresh_token: Some("secret-refresh".into()),
            expires_at: 4_000_000_000,
            user_id: Some("4242".into()),
            country: "DE".into(),
            session_id: None,
            from: None,
            client: Default::default(),
            unconfirmed: false,
            named_client: None,
        });
        lib.set_setting(tidal_unofficial::STATE_KEY, &serde_json::to_string(&m).unwrap())
            .unwrap();
    }
    let (_, r) = call(&app, "GET", signin, None).await;
    assert_eq!(r["in_use"]["way"], "device", "{r}");
    assert_eq!(r["in_use"]["user"], "4242");
    assert_eq!(r["in_use"]["expires_at"], 4_000_000_000i64);
    assert_eq!(r["in_use"]["gives"], "full tracks");
    assert_eq!(r["line"], "Your own device client · full tracks");
    assert!(!r.to_string().contains("secret-"), "{r}");
    // The Sources overview carries the same line; a source without a sign-in has none
    let (_, srcs) = call(&app, "GET", "/v1/sources", None).await;
    let row = |id: &str| {
        srcs.as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(row("tidal")["sign_in"], "Your own device client · full tracks");
    assert_eq!(
        row("tidal")["status"],
        "ready",
        "the device sign-in counts: {srcs}"
    );
    assert!(row("deezer").get("sign_in").is_none(), "{srcs}");
    // Picking the app way turns the device way off (its session is kept until Sign out)
    let (st, r) = call(&app, "PUT", signin, Some(serde_json::json!({"way": "app"}))).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["details"]["unofficial"]["enabled"], false);
    assert!(r["in_use"].is_null(), "{r}");
    assert_eq!(r["line"], "Not signed in");
    // The official sign-in on a third-party tier app: previews
    {
        let lib = s.library.lock().unwrap();
        let u = tidal::UserSession {
            access_token: "secret-official".into(),
            refresh_token: None,
            expires_at: 4_000_000_000,
            user_id: Some("7".into()),
            country: Some("DE".into()),
            scope: String::new(),
            access_tier: Some("THIRD_PARTY".into()),
        };
        lib.set_setting("tidal.user", &serde_json::to_string(&u).unwrap())
            .unwrap();
    }
    let (_, r) = call(&app, "GET", signin, None).await;
    assert_eq!(r["in_use"]["way"], "app", "{r}");
    assert_eq!(r["in_use"]["gives"], "30-second previews, your collection");
    assert_eq!(r["line"], "Your own TIDAL app · previews");
    assert!(!r.to_string().contains("secret-"), "{r}");
    // Sign out of the way in use; the picked way stays remembered
    let (st, r) = call(&app, "DELETE", signin, None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert!(r["in_use"].is_null(), "{r}");
    assert_eq!(r["chosen"], "app");
    assert_eq!(r["line"], "Not signed in");
    assert_eq!(r["details"]["status"]["signed_in"], false);
    // Sources without ways, and unknown ones
    let (st, _) = call(&app, "GET", "/v1/sources/deezer/signin", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = call(&app, "GET", "/v1/sources/nope/signin", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    *imports::HOME_OVERRIDE.write().unwrap() = None;
}

/// Sources › TIDAL › Sign in › from another app on this machine: the known places are read,
/// what was found is listed by user and expiry (never a token), a usable one is imported on
/// a click, an unusable one is refused in plain words, the other app's file is untouched,
/// and a pasted token works too (STEWARD-CREDENTIALS-REUSE.md step 2).
#[tokio::test]
async fn tidal_sign_ins_from_other_apps_are_listed_imported_or_refused() {
    let _home_guard = imports::HOME_TEST_LOCK.lock().await;
    use base64::Engine;
    let jwt = |uid: i64, exp: i64| {
        let b = |v: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v);
        format!(
            "{}.{}.{}",
            b(r#"{"alg":"RS256"}"#),
            b(&format!(r#"{{"uid":{uid},"cc":"DE","exp":{exp}}}"#)),
            b("s")
        )
    };
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".config/tidal_dl_ng")).unwrap();
    std::fs::write(
        home.path().join(".config/tidal_dl_ng/token.json"),
        format!(
            r#"{{"token_type": {{"data": "Bearer"}}, "access_token": {{"data": "{}"}}, "refresh_token": {{"data": "secret-refresh"}}, "is_pkce": {{"data": false}}, "expiry_time": {{"data": "2096-01-01T00:00:00"}}}}"#,
            jwt(4242, 4_000_000_000)
        ),
    )
    .unwrap();
    std::fs::write(
        home.path().join(".tidal-dl.token.json"),
        base64::engine::general_purpose::STANDARD.encode(format!(
            r#"{{"userid": 7, "countryCode": "US", "accessToken": "{}", "refreshToken": "r", "expiresAfter": 1000}}"#,
            jwt(7, 1000)
        )),
    )
    .unwrap();
    *imports::HOME_OVERRIDE.write().unwrap() = Some(home.path().to_path_buf());
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    // A device client of the owner's (a fake one): it renews python-tidal's device sessions
    s.library
        .lock()
        .unwrap()
        .set_setting(
            tidal_unofficial::CLIENT_KEY,
            r#"{"id": "fake-device-client-id", "secret": "fake-device-secret", "from": "settings"}"#,
        )
        .unwrap();
    let app = router(s.clone());
    let signin = "/v1/sources/tidal/signin";
    let import = "/v1/sources/tidal/signin/import";
    let (st, si) = call(&app, "GET", signin, None).await;
    assert_eq!(st, StatusCode::OK, "{si}");
    let found = si["imports"].as_array().unwrap();
    assert_eq!(found.len(), 2, "{si}");
    assert_eq!(found[0]["id"], "tidal-dl-ng");
    assert_eq!(found[0]["user"], "4242");
    assert_eq!(found[0]["usable"], true);
    assert_eq!(
        found[0]["renews"], true,
        "a device session renews with the node's device client"
    );
    assert_eq!(found[1]["id"], "tidal-dl");
    assert_eq!(found[1]["usable"], false);
    assert!(found[1]["reason"].as_str().unwrap().contains("expired"), "{si}");
    assert!(
        si["places"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().starts_with("Music Assistant: not read")),
        "{si}"
    );
    // Found: listed after the code sign-in, not under Advanced
    assert_eq!(si["ways"][2]["id"], "import", "{si}");
    assert_eq!(si["ways"][2]["available"], true);
    assert!(si["ways"][2].get("advanced").is_none(), "{si}");
    assert!(
        si["ways"][2]["note"]
            .as_str()
            .unwrap()
            .contains("tidal-dl-ng (user 4242)"),
        "{si}"
    );
    let text = si.to_string();
    assert!(
        !text.contains("secret-refresh") && !text.contains(&jwt(4242, 4_000_000_000)),
        "{si}"
    );
    // Refused in plain words; nothing changes
    let (st, r) = call(&app, "POST", import, Some(serde_json::json!({"id": "tidal-dl"}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{r}");
    assert!(r["error"].as_str().unwrap().contains("expired"), "{r}");
    let (st, _) = call(&app, "POST", import, Some(serde_json::json!({"id": "nope"}))).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, r) = call(&app, "POST", import, Some(serde_json::json!({"text": "hello"}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{r}");
    assert!(r["error"].as_str().unwrap().starts_with("Not usable"), "{r}");
    let (_, r) = call(&app, "GET", signin, None).await;
    assert!(r["in_use"].is_null(), "{r}");
    // Used on a click: the device-way session, named after the app
    let (st, r) = call(
        &app,
        "POST",
        import,
        Some(serde_json::json!({"id": "tidal-dl-ng"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["in_use"]["way"], "import");
    assert_eq!(r["in_use"]["label"], "tidal-dl-ng's sign-in");
    assert_eq!(r["in_use"]["user"], "4242");
    assert_eq!(r["chosen"], "import");
    assert_eq!(r["line"], "tidal-dl-ng's sign-in · full tracks");
    assert_eq!(r["details"]["unofficial"]["enabled"], true);
    assert_eq!(r["details"]["unofficial"]["from"], "tidal-dl-ng");
    assert!(!r.to_string().contains("secret-refresh"), "{r}");
    let (_, srcs) = call(&app, "GET", "/v1/sources", None).await;
    let t = srcs
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "tidal")
        .unwrap();
    assert_eq!(t["sign_in"], "tidal-dl-ng's sign-in · full tracks");
    assert_eq!(t["status"], "ready");
    assert!(
        std::fs::read_to_string(home.path().join(".config/tidal_dl_ng/token.json"))
            .unwrap()
            .contains("secret-refresh"),
        "the other app's file is only read"
    );
    // Sign out, then a pasted token with its refresh token on the next line
    let (_, r) = call(&app, "DELETE", signin, None).await;
    assert!(r["in_use"].is_null(), "{r}");
    let (st, r) = call(
        &app,
        "POST",
        import,
        Some(serde_json::json!({"text": format!("{}\nmy-refresh", jwt(5, 4_000_000_000))})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["in_use"]["way"], "paste");
    assert_eq!(r["in_use"]["label"], "Pasted token");
    assert_eq!(r["in_use"]["user"], "5");
    assert_eq!(r["chosen"], "paste");
    assert!(!r.to_string().contains("my-refresh"), "{r}");
    *imports::HOME_OVERRIDE.write().unwrap() = None;
}

/// Each person plays TIDAL with their own account, never anyone else's
/// (TASK-per-person-service-accounts): the owner's items with the owner's token, a listener's
/// with theirs, in the same zone; a listener without TIDAL gets "Connect TIDAL" and TIDAL is
/// never asked with someone else's token. The browser proxy does the same.
#[tokio::test]
async fn each_person_plays_tidal_with_their_own_account() {
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let (base, seen) = tidal_mock_recording_tokens(flac).await;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), Some("s3cret")).await;
    {
        let mut ep = s.tidal.write().unwrap();
        ep.api_base = base.clone();
        ep.auth_base = base.clone();
        ep.creds = tidal::TidalCredentials {
            client_id: "cid".into(),
            client_secret: "sec".into(),
            scopes: String::new(),
        };
    }
    let session = |token: &str| {
        serde_json::json!({"access_token": token, "refresh_token": "r", "expires_at": pm_library::db::now_unix() + 3600, "user_id": "1", "scope": "playback"}).to_string()
    };
    let (anna, ben) = (
        "tbepfp8fisyiqpskbmxsmpub47gxb7xdsh5x9ut9qrb97gcq9phy",
        "o4dksfbqk85ogzdb5osziw6befigbuxmuxkuxq8434q89uj56uyy",
    );
    {
        let lib = s.library.lock().unwrap();
        lib.set_setting("tidal.user", &session("owner-token")).unwrap();
        lib.set_setting(&format!("person.{anna}.tidal.user"), &session("anna-token"))
            .unwrap();
        lib.set_setting(
            "people",
            &serde_json::json!({"people": [
            {"key": anna, "access": "music", "music_enabled": true, "added_at": 1},
            {"key": ben, "access": "music", "music_enabled": true, "added_at": 1}]})
            .to_string(),
        )
        .unwrap();
    }
    let (anna_tok, ben_tok) = (
        accounts::mint(&s, anna).unwrap(),
        accounts::mint(&s, ben).unwrap(),
    );
    let app = router(s.clone());
    let send = |method: &'static str, uri: String, token: String, body: Option<serde_json::Value>| {
        let app = app.clone();
        async move {
            let req = Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(
                    body.map(|b| Body::from(b.to_string()))
                        .unwrap_or_else(Body::empty),
                )
                .unwrap();
            let r = app.oneshot(req).await.unwrap();
            let st = r.status();
            let b = r.into_body().collect().await.unwrap().to_bytes();
            (
                st,
                serde_json::from_slice::<serde_json::Value>(&b).unwrap_or_default(),
                b,
            )
        }
    };
    let zones = s.zones.list();
    let zid = zones[0].id.to_string();
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    // One person's item at a time; each plays to its end (or fails)
    let play = |token: String, by_in_body: Option<&'static str>| {
        let send = &send;
        let zone = zone.clone();
        let zid = zid.clone();
        async move {
            let mut item = serde_json::json!({"uri": "tidal:track:20115564"});
            if let Some(b) = by_in_body {
                // What a client says about who queued it is never believed
                item["by"] = b.into();
            }
            let (st, v, _) = send(
                "POST",
                format!("/v1/zones/{zid}/queue"),
                token,
                Some(serde_json::json!({"items": [item], "replace": true, "play": true})),
            )
            .await;
            assert_eq!(st, StatusCode::OK, "{v}");
            zone.wait_stopped().await
        }
    };
    let st = play("s3cret".into(), None).await;
    assert!(st.error.is_none(), "{:?}", st.error);
    let st = play(anna_tok.clone(), None).await;
    assert!(st.error.is_none(), "{:?}", st.error);
    assert_eq!(
        *seen.lock().unwrap(),
        ["Bearer owner-token", "Bearer anna-token"],
        "each item with its queuer's account"
    );
    // Ben has no TIDAL: no fallback to the owner's (nor Anna's), even claiming to be them
    let st = play(ben_tok.clone(), Some(anna)).await;
    assert!(
        st.error.as_deref().is_some_and(|e| e.contains("Connect TIDAL")),
        "{:?}",
        st.error
    );
    let st = play(anna_tok.clone(), Some("")).await;
    assert!(st.error.is_none(), "{:?}", st.error);
    assert_eq!(seen.lock().unwrap().len(), 3, "{:?}", seen.lock().unwrap());
    assert_eq!(seen.lock().unwrap()[2], "Bearer anna-token");

    // The browser proxy: each with their own, Ben refused with the same words
    let (st, _, body) = send(
        "GET",
        "/v1/sources/tidal/stream/20115564".into(),
        anna_tok.clone(),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(!body.is_empty());
    assert_eq!(seen.lock().unwrap().last().unwrap(), "Bearer anna-token");
    let n = seen.lock().unwrap().len();
    let (st, v, _) = send(
        "GET",
        "/v1/sources/tidal/stream/20115564".into(),
        ben_tok.clone(),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED, "{v}");
    assert!(v["error"].as_str().unwrap().contains("Connect TIDAL"), "{v}");
    assert_eq!(seen.lock().unwrap().len(), n, "TIDAL never asked for Ben");
    // Each sees their own sign-in, and can't change the node's device client
    let (_, me, _) = send("GET", "/v1/sources/tidal/status".into(), anna_tok.clone(), None).await;
    assert_eq!(me["signed_in"], true, "{me}");
    let (_, me, _) = send("GET", "/v1/sources/tidal/status".into(), ben_tok.clone(), None).await;
    assert_eq!(me["signed_in"], false, "{me}");
    let (st, _, _) = send(
        "PUT",
        "/v1/sources/tidal/unofficial".into(),
        ben_tok.clone(),
        Some(serde_json::json!({"client_id": "x", "client_secret": "y"})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    // Ben signing out touches nothing of anyone else's
    let (st, _, _) = send(
        "DELETE",
        "/v1/sources/tidal/session".into(),
        ben_tok.clone(),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let lib = s.library.lock().unwrap();
    assert!(lib.get_setting("tidal.user").unwrap().is_some());
    assert!(lib
        .get_setting(&format!("person.{anna}.tidal.user"))
        .unwrap()
        .is_some());
}

/// The TIDAL stream for browsers refuses someone with no account of their own: nothing
/// fetched for anyone else is served to them (#226 review L3, the missing test).
#[tokio::test]
async fn the_tidal_stream_refuses_someone_without_an_account() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _, _) = two_people_node(dir.path(), "http://127.0.0.1:9").await;
    let guest = accounts::Caller {
        role: accounts::Role::Guest,
        key: None,
    };
    let r = tidal::stream(
        State(s),
        axum::Extension(guest),
        axum::extract::Path("4242".to_string()),
        HeaderMap::new(),
    )
    .await;
    assert_eq!(r.err().map(|e| e.status), Some(StatusCode::FORBIDDEN));
}

/// TIDAL tracks kept for browsers are bounded by their bytes, not only by their count, and a
/// byte range is served as a slice of the kept track, never a copy of it (security review
/// 2026-11-02, medium 4).
#[tokio::test]
async fn tidal_tracks_kept_for_browsers_are_bounded_and_ranges_not_copied() {
    let at = std::time::Instant::now();
    let mut cache = Vec::new();
    // Zeroed allocations: the pages are never touched
    let track = |n: usize| bytes::Bytes::from(vec![0u8; n]);
    for i in 0..6 {
        tidal::keep_stream(
            &mut cache,
            (format!("k/{i}"), at, track(100 * 1024 * 1024), "audio/mp4".into()),
        );
        let held: usize = cache.iter().map(|(_, _, b, _)| b.len()).sum();
        assert!(held <= tidal::STREAM_CACHE_BYTES, "{held} bytes kept after {i}");
        assert_eq!(cache.last().unwrap().0, format!("k/{i}"), "the newest is kept");
    }
    // One bigger than all of it is served, not kept
    tidal::keep_stream(
        &mut cache,
        (
            "k/big".into(),
            at,
            track(tidal::STREAM_CACHE_BYTES + 1),
            "audio/mp4".into(),
        ),
    );
    assert!(cache.iter().all(|(k, _, _, _)| k != "k/big"));

    let kept: bytes::Bytes = (0..=255u8).cycle().take(1 << 20).collect::<Vec<u8>>().into();
    let mut h = HeaderMap::new();
    h.insert(header::RANGE, "bytes=1000-1999".parse().unwrap());
    let resp = ranged_response(&h, kept.clone(), "audio/mp4");
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&body[..], &kept[1000..2000]);
    assert_eq!(
        body.as_ptr(),
        kept[1000..].as_ptr(),
        "the range is the kept bytes, not a copy"
    );
    let resp = ranged_response(&HeaderMap::new(), kept.clone(), "audio/mp4");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        body.as_ptr(),
        kept.as_ptr(),
        "the whole track is not a copy either"
    );
}

/// The pasted address of the browser sign-in: the whole tidal.com address or the bare code;
/// anything else (another host, a look-alike, http, no code, TIDAL's error) in plain words.
#[test]
fn tidal_browser_sign_in_paste_is_parsed_after_its_host() {
    use tidal_unofficial::code_from_paste as code;
    assert_eq!(
        code(" https://tidal.com/android/login/auth?code=abc_DEF-1.2~3&state=na \n").as_deref(),
        Ok("abc_DEF-1.2~3")
    );
    assert_eq!(
        code("https://listen.tidal.com/login/auth?code=x1").as_deref(),
        Ok("x1")
    );
    assert_eq!(
        code("tidal.com/android/login/auth?code=x2").as_deref(),
        Ok("x2"),
        "no scheme: https"
    );
    assert_eq!(
        code("HTTPS://TIDAL.COM./android/login/auth?code=x3").as_deref(),
        Ok("x3")
    );
    assert_eq!(code("bare-Code_123.x~y").as_deref(), Ok("bare-Code_123.x~y"));
    let err = |t: &str| code(t).unwrap_err();
    for evil in [
        "https://evil.example/android/login/auth?code=x",
        "https://tidal.com.evil.example/android/login/auth?code=x",
        "https://eviltidal.com/android/login/auth?code=x",
        "https://tidal.com@evil.example/?code=x",
    ] {
        assert!(
            err(evil).contains("isn't a TIDAL address"),
            "{evil}: {}",
            err(evil)
        );
    }
    assert!(err("http://tidal.com/android/login/auth?code=x").contains("https://"));
    assert!(err("https://tidal.com/android/login/auth?state=1").contains("no sign-in code"));
    assert!(err("https://tidal.com/android/login/auth?code=").contains("no sign-in code"));
    assert!(err(
        "https://tidal.com/android/login/auth?error=access_denied&error_description=User%20cancelled"
    )
    .contains("User cancelled"));
    assert!(err("  ").contains("Paste the address"));
    assert!(err("hello world").contains("neither a TIDAL address nor a sign-in code"));
}

/// Sign in with TIDAL against a mock TIDAL: the login.tidal.com address carries python-tidal's
/// parameters and an S256 challenge of the verifier kept here; a wrong paste, a refused code,
/// no attempt and an expired one each say so; the code is exchanged with the verifier and the
/// client unique key (no secret); the session is kept marked as the browser one, is in use as
/// "Sign in with TIDAL", renews with the browser client, and nothing secret is ever logged.
#[tokio::test]
async fn tidal_browser_sign_in_from_a_mock() {
    let _home_guard = imports::HOME_TEST_LOCK.lock().await;
    let home = tempfile::tempdir().unwrap();
    *imports::HOME_OVERRIDE.write().unwrap() = Some(home.path().to_path_buf());
    let (logs, dispatch) = log_capture();
    let _logging = tracing::dispatcher::set_default(&dispatch);
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let (base, seen) = full_tracks_mock(flac).await;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    point_tidal_at(&s, &base, true);
    let app = router(s.clone());
    let signin = "/v1/sources/tidal/signin";

    // The chooser: Sign in with TIDAL first and Recommended, the code sign-in next; the rest
    // under Advanced; no import way with nothing found
    let (_, si) = call(&app, "GET", signin, None).await;
    let ids: Vec<&str> = si["ways"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["browser", "device", "app", "own-device", "paste"], "{si}");
    let flags: Vec<(bool, bool)> = si["ways"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| (w["recommended"] == true, w["advanced"] == true))
        .collect();
    assert_eq!(
        flags,
        [
            (true, false),
            (false, false),
            (false, true),
            (false, true),
            (false, true)
        ],
        "{si}"
    );
    assert_eq!(si["ways"][0]["label"], "Sign in with TIDAL");
    assert_eq!(si["ways"][0]["available"], true);
    assert!(si["ways"][1]["note"].as_str().unwrap().contains("AAC"), "{si}");
    assert_eq!(si["details"]["unofficial"]["browser_available"], true);

    let finish = |text: &str| {
        let app = app.clone();
        let body = serde_json::json!({ "text": text });
        async move {
            call(
                &app,
                "POST",
                "/v1/sources/tidal/unofficial/browser/finish",
                Some(body),
            )
            .await
        }
    };
    // Nothing started yet
    let (st, r) = finish("https://tidal.com/android/login/auth?code=good-code").await;
    assert_eq!(st, StatusCode::CONFLICT, "{r}");
    assert!(
        r["error"].as_str().unwrap().contains("Press Sign in with TIDAL"),
        "{r}"
    );

    // The address to open
    let (st, r) = call(&app, "POST", "/v1/sources/tidal/unofficial/browser", None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    let url = url::Url::parse(r["url"].as_str().unwrap()).unwrap();
    assert_eq!(
        url.as_str().split('?').next().unwrap(),
        format!("{base}/authorize")
    );
    let pending = tidal_unofficial::load(&s.library.lock().unwrap(), &accounts::Who::Owner)
        .browser
        .unwrap();
    assert_eq!(
        pending.code_verifier.len(),
        43,
        "32 random bytes, base64url without padding"
    );
    assert!(pending
        .code_verifier
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    let q: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    let names: Vec<&str> = q.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        names,
        [
            "response_type",
            "redirect_uri",
            "client_id",
            "lang",
            "appMode",
            "client_unique_key",
            "code_challenge",
            "code_challenge_method",
            "restrict_signup"
        ],
        "python-tidal's pkce_login_url, in its order"
    );
    let v = |k: &str| q.iter().find(|(n, _)| n == k).unwrap().1.as_str();
    assert_eq!(v("response_type"), "code");
    assert_eq!(v("redirect_uri"), "https://tidal.com/android/login/auth");
    assert_eq!(v("client_id"), FAKE_BROWSER_ID);
    assert_eq!(
        (v("lang"), v("appMode"), v("restrict_signup")),
        ("EN", "android", "true")
    );
    assert_eq!(v("client_unique_key"), pending.client_unique_key);
    assert_eq!(v("code_challenge_method"), "S256");
    {
        use base64::Engine;
        use sha2::Digest;
        let want = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(sha2::Sha256::digest(pending.code_verifier.as_bytes()));
        assert_eq!(v("code_challenge"), want);
        assert_ne!(v("code_challenge"), pending.code_verifier);
    }
    assert!(
        !r.to_string().contains(&pending.code_verifier),
        "the verifier stays on the node"
    );
    let (_, u) = call(&app, "GET", "/v1/sources/tidal/unofficial", None).await;
    assert!(
        u["browser_pending"].as_i64().unwrap() > pm_library::db::now_unix(),
        "{u}"
    );
    assert!(!u.to_string().contains(&pending.code_verifier));

    // Wrong pastes: nothing is sent to TIDAL for them
    for (text, says) in [
        (
            "https://evil.example/android/login/auth?code=good-code",
            "isn't a TIDAL address",
        ),
        ("http://tidal.com/android/login/auth?code=good-code", "https://"),
        ("https://tidal.com/android/login/auth", "no sign-in code"),
    ] {
        let (st, r) = finish(text).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{text}: {r}");
        assert!(r["error"].as_str().unwrap().contains(says), "{text}: {r}");
    }
    assert!(
        seen.lock().unwrap().tokens.is_empty(),
        "nothing sent for a wrong paste"
    );
    // TIDAL refuses the code: said so, the attempt stays for another code
    let (st, r) = finish("https://tidal.com/android/login/auth?code=old-code").await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{r}");
    assert!(
        r["error"]
            .as_str()
            .unwrap()
            .contains("didn't accept that sign-in code"),
        "{r}"
    );
    assert!(
        r["error"]
            .as_str()
            .unwrap()
            .contains("Authorization code is invalid"),
        "{r}"
    );
    // The right address
    let (st, r) = finish("https://tidal.com/android/login/auth?code=good-code&lang=en").await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["signed_in"], true, "{r}");
    assert_eq!(r["enabled"], true, "{r}");
    assert_eq!(r["session_client"], "pkce", "{r}");
    assert_eq!(r["user"], "77");
    assert!(r.get("browser_pending").is_none(), "{r}");
    let exchange = seen.lock().unwrap().tokens.last().unwrap().clone();
    let form: std::collections::HashMap<String, String> = url::form_urlencoded::parse(exchange.as_bytes())
        .into_owned()
        .collect();
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["code"], "good-code");
    assert_eq!(form["client_id"], FAKE_BROWSER_ID);
    assert_eq!(form["redirect_uri"], "https://tidal.com/android/login/auth");
    assert_eq!(
        form["scope"], "r_usr+w_usr+w_sub",
        "python-tidal's scope, as it sends it"
    );
    assert_eq!(form["code_verifier"], pending.code_verifier);
    assert_eq!(form["client_unique_key"], pending.client_unique_key);
    assert!(
        !form.contains_key("client_secret"),
        "python-tidal's exchange has no secret: {exchange}"
    );
    // Kept marked as the browser sign-in; in use as Sign in with TIDAL, the way remembered
    let raw = s
        .library
        .lock()
        .unwrap()
        .get_setting(tidal_unofficial::STATE_KEY)
        .unwrap()
        .unwrap();
    assert!(
        raw.contains(r#""client":"pkce""#),
        "the session says which client renews it"
    );
    let (_, si) = call(&app, "GET", signin, None).await;
    assert_eq!(si["in_use"]["way"], "browser", "{si}");
    assert_eq!(si["in_use"]["label"], "Sign in with TIDAL");
    assert_eq!(si["chosen"], "browser");
    assert_eq!(si["line"], "Sign in with TIDAL · full tracks");
    assert!(
        !si.to_string().contains("pk-at") && !si.to_string().contains("pk-rt"),
        "{si}"
    );
    // The attempt is used up
    let (st, _) = finish("https://tidal.com/android/login/auth?code=good-code").await;
    assert_eq!(st, StatusCode::CONFLICT);

    // An expired attempt
    call(&app, "POST", "/v1/sources/tidal/unofficial/browser", None).await;
    tidal_unofficial::update(&s, &accounts::Who::Owner, |st| {
        st.browser.as_mut().unwrap().expires_at = pm_library::db::now_unix() - 1;
    });
    let (st, r) = finish("good-code").await;
    assert_eq!(st, StatusCode::GONE, "{r}");
    assert!(r["error"].as_str().unwrap().contains("expired"), "{r}");
    assert!(
        tidal_unofficial::load(&s.library.lock().unwrap(), &accounts::Who::Owner)
            .browser
            .is_none()
    );
    // A bare code works as well as the address
    call(&app, "POST", "/v1/sources/tidal/unofficial/browser", None).await;
    let (st, r) = finish("good-code").await;
    assert_eq!(st, StatusCode::OK, "{r}");

    // Renewal with the browser client (python-tidal's token_refresh: id and secret, no scope)
    tidal_unofficial::update(&s, &accounts::Who::Owner, |st| {
        st.session.as_mut().unwrap().expires_at = pm_library::db::now_unix();
    });
    let (ext, lossy, bytes) = open_full(&s, accounts::Who::Owner, "2", &dispatch)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (ext, lossy),
        ("m4a", false),
        "hi-res DASH FLAC: fragmented MP4 into the decoder"
    );
    assert_eq!(String::from_utf8(bytes).unwrap(), "[init][1][2][3]");
    let renewal = seen.lock().unwrap().tokens.last().unwrap().clone();
    let form: std::collections::HashMap<String, String> = url::form_urlencoded::parse(renewal.as_bytes())
        .into_owned()
        .collect();
    assert_eq!(form["grant_type"], "refresh_token");
    assert_eq!(form["refresh_token"], "pk-rt");
    assert_eq!(form["client_id"], FAKE_BROWSER_ID);
    assert_eq!(form["client_secret"], FAKE_BROWSER_SECRET);
    assert!(!form.contains_key("scope"), "{renewal}");
    let st = tidal_unofficial::load(&s.library.lock().unwrap(), &accounts::Who::Owner);
    let sess = st.session.unwrap();
    assert_eq!(sess.access_token, "pk-at-renewed");
    assert_eq!(
        sess.refresh_token.as_deref(),
        Some("pk-rt"),
        "kept when TIDAL sends none"
    );
    assert_eq!(
        sess.client,
        tidal_unofficial::ClientKind::Pkce,
        "still the browser one"
    );
    // python-tidal's Android web view headers on the private API
    assert!(seen.lock().unwrap().agents.iter().all(|a| a.contains("Android")));

    // A build without the browser client: the way is there, not available, says why, and is
    // not recommended; its endpoints refuse; a browser session can't be renewed
    point_tidal_at(&s, &base, false);
    let (_, si) = call(&app, "GET", signin, None).await;
    assert_eq!(si["ways"][0]["id"], "browser");
    assert_eq!(si["ways"][0]["available"], false);
    assert_eq!(si["ways"][0]["recommended"], false);
    assert!(
        si["ways"][0]["reason"].as_str().unwrap().contains("code sign-in"),
        "{si}"
    );
    let (st, _) = call(&app, "POST", "/v1/sources/tidal/unofficial/browser", None).await;
    assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE);
    tidal_unofficial::update(&s, &accounts::Who::Owner, |st| {
        st.session.as_mut().unwrap().expires_at = pm_library::db::now_unix();
    });
    let r = open_full(&s, accounts::Who::Owner, "1", &dispatch).await;
    assert!(
        r.as_ref()
            .is_err_and(|e| e.starts_with("mode:") && e.contains("Sign in with TIDAL")),
        "{r:?}"
    );

    *imports::HOME_OVERRIDE.write().unwrap() = None;
    // Nothing secret in any log line: the client, the verifier, tokens, CDN addresses
    let text = logs.text();
    assert!(text.contains("signed in with TIDAL"), "the sign-in is logged");
    for secret in [
        FAKE_BROWSER_SECRET,
        FAKE_BROWSER_ID,
        pending.code_verifier.as_str(),
        "pk-at",
        "pk-rt",
        "good-code",
        "SECRET-CDN",
    ] {
        assert!(!text.contains(secret), "a secret in the logs: {secret}");
    }
}

/// Each person's Sign in with TIDAL is their own: an attempt, its pasted address and the
/// session it makes belong to whoever started it.
#[tokio::test]
async fn tidal_browser_sign_in_is_per_person() {
    let (base, _) = full_tracks_mock(Vec::new()).await;
    let dir = tempfile::tempdir().unwrap();
    let (s, anna, _ben) = two_people_node(dir.path(), "http://127.0.0.1:9").await;
    point_tidal_at(&s, &base, true);
    let app = router(s.clone());
    let anna_tok = accounts::mint(&s, anna).unwrap();
    let owner = Some("s3cret");
    let start = "/v1/sources/tidal/unofficial/browser";
    let finish = "/v1/sources/tidal/unofficial/browser/finish";
    let (st, r, _) = send_as(&app, "POST", start, Some(&anna_tok), &[], None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    // The owner has no attempt of their own: Anna's address does nothing for them
    let body = serde_json::json!({"text": "https://tidal.com/android/login/auth?code=anna-code"});
    let (st, r, _) = send_as(&app, "POST", finish, owner, &[], Some(body.clone())).await;
    assert_eq!(st, StatusCode::CONFLICT, "{r}");
    let (st, r, _) = send_as(&app, "POST", finish, Some(&anna_tok), &[], Some(body)).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["session_client"], "pkce");
    let anna_who = accounts::Who::Listener(anna.to_string());
    {
        let lib = s.library.lock().unwrap();
        let a = tidal_unofficial::load(&lib, &anna_who);
        assert_eq!(a.session.unwrap().access_token, "pk-at-anna-code");
        assert!(tidal_unofficial::load(&lib, &accounts::Who::Owner)
            .session
            .is_none());
    }
    let (_, u, _) = send_as(&app, "GET", "/v1/sources/tidal/unofficial", owner, &[], None).await;
    assert_eq!(u["signed_in"], false, "{u}");
    let (_, si, _) = send_as(
        &app,
        "GET",
        "/v1/sources/tidal/signin",
        Some(&anna_tok),
        &[],
        None,
    )
    .await;
    assert_eq!(si["in_use"]["way"], "browser", "{si}");
    assert_eq!(
        si["ways"][0]["available"], true,
        "a listener signs in with TIDAL too: {si}"
    );
    // Nobody signed in to the node has no attempt to start
    let (st, _, _) = send_as(&app, "POST", start, None, &[], None).await;
    assert!(
        st == StatusCode::UNAUTHORIZED || st == StatusCode::FORBIDDEN,
        "{st}"
    );
}

/// What TIDAL answered is logged per track (quality, manifest, codecs, depth and rate; no
/// address, no token); AAC for HI_RES_LOSSLESS is asked again as LOSSLESS before it plays; a
/// code sign-in that still gets AAC says so on its health line with what to do; and a session
/// stored before sessions said their client still loads and renews with the device client.
#[tokio::test]
async fn tidal_full_track_answers_are_logged_and_aac_is_asked_again() {
    let (logs, dispatch) = log_capture();
    let _logging = tracing::dispatcher::set_default(&dispatch);
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let (base, seen) = full_tracks_mock(flac).await;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    point_tidal_at(&s, &base, true);
    let app = router(s.clone());
    call(
        &app,
        "PUT",
        "/v1/sources/tidal/unofficial",
        Some(serde_json::json!({"client_id": "dev-id", "client_secret": "dev-secret"})),
    )
    .await;
    // As an older node stored it: no `client`, ran out
    let old = format!(
        r#"{{"enabled":true,"quality":"max","session":{{"access_token":"dev-at","refresh_token":"dev-rt","expires_at":{},"user_id":"42","country":"CH","session_id":"sid"}}}}"#,
        pm_library::db::now_unix() - 10
    );
    s.library
        .lock()
        .unwrap()
        .set_setting(tidal_unofficial::STATE_KEY, &old)
        .unwrap();
    let (_, u) = call(&app, "GET", "/v1/sources/tidal/unofficial", None).await;
    assert_eq!(
        (u["signed_in"].as_bool(), u["session_client"].as_str()),
        (Some(true), Some("device")),
        "{u}"
    );
    let who = accounts::Who::Owner;

    // BTS FLAC (16/44.1): the old session renewed with the device client first
    let (ext, lossy, bytes) = open_full(&s, who.clone(), "1", &dispatch).await.unwrap().unwrap();
    assert_eq!((ext, lossy), ("flac", false));
    assert!(bytes.starts_with(b"fLaC"));
    let renewal = seen.lock().unwrap().tokens.last().unwrap().clone();
    assert!(
        renewal.contains("grant_type=refresh_token")
            && renewal.contains("refresh_token=dev-rt")
            && renewal.contains("client_id=dev-id")
            && renewal.contains("client_secret=dev-secret")
            && renewal.contains("scope=r_usr+w_usr+w_sub"),
        "{renewal}"
    );
    let raw = s
        .library
        .lock()
        .unwrap()
        .get_setting(tidal_unofficial::STATE_KEY)
        .unwrap()
        .unwrap();
    assert!(
        raw.contains("dev-at-renewed") && !raw.contains(r#""client""#),
        "still a device session"
    );
    let (_, u) = call(&app, "GET", "/v1/sources/tidal/unofficial", None).await;
    assert_eq!(u["health"]["detail"], "full tracks are playing", "{u}");
    assert!(u["health"].get("lossy").is_none(), "{u}");

    // DASH FLAC, hi-res
    let (ext, lossy, bytes) = open_full(&s, who.clone(), "2", &dispatch).await.unwrap().unwrap();
    assert_eq!(
        (ext, lossy, bytes.as_slice()),
        ("m4a", false, b"[init][1][2][3]".as_slice())
    );
    // DASH AAC for HI_RES_LOSSLESS: asked again as LOSSLESS, which is FLAC
    let (ext, lossy, _) = open_full(&s, who.clone(), "3", &dispatch).await.unwrap().unwrap();
    assert_eq!((ext, lossy), ("flac", false), "the LOSSLESS answer plays");
    // AAC either way: played, and the health line says why and what to do
    let (ext, lossy, _) = open_full(&s, who.clone(), "4", &dispatch).await.unwrap().unwrap();
    assert_eq!((ext, lossy), ("m4a", true));
    let asked: Vec<(String, String)> = seen.lock().unwrap().asked.clone();
    let q = |id: &str| {
        asked
            .iter()
            .filter(|(t, _)| t == id)
            .map(|(_, q)| q.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(q("1"), ["HI_RES_LOSSLESS"]);
    assert_eq!(q("2"), ["HI_RES_LOSSLESS"]);
    assert_eq!(q("3"), ["HI_RES_LOSSLESS", "LOSSLESS"]);
    assert_eq!(q("4"), ["HI_RES_LOSSLESS", "LOSSLESS"]);
    let (_, u) = call(&app, "GET", "/v1/sources/tidal/unofficial", None).await;
    assert_eq!(u["health"]["ok"], true, "AAC plays: {u}");
    assert_eq!(u["health"]["lossy"], true, "{u}");
    assert_eq!(u["health"]["detail"], tidal_unofficial::AAC_ONLY_DEVICE);
    assert!(tidal_unofficial::AAC_ONLY_DEVICE.contains("Sign in with TIDAL again for lossless"));
    assert_eq!(u["enabled"], true, "AAC is not a failure of the mode");
    // A lossless track again: OK again
    open_full(&s, who.clone(), "1", &dispatch).await.unwrap().unwrap();
    let (_, u) = call(&app, "GET", "/v1/sources/tidal/unofficial", None).await;
    assert_eq!(u["health"]["detail"], "full tracks are playing", "{u}");
    // High asks LOSSLESS only: no second question
    call(
        &app,
        "PUT",
        "/v1/sources/tidal/unofficial",
        Some(serde_json::json!({"quality": "high"})),
    )
    .await;
    open_full(&s, who.clone(), "4", &dispatch).await.unwrap().unwrap();
    assert_eq!(
        seen.lock().unwrap().asked.last().unwrap(),
        &("4".to_string(), "LOSSLESS".to_string())
    );
    assert_eq!(
        seen.lock()
            .unwrap()
            .asked
            .iter()
            .filter(|(t, _)| t == "4")
            .count(),
        3
    );
    // A build without the browser client doesn't send people to Sign in with TIDAL: it says
    // what lossless needs
    let browser = s.tidal.write().unwrap().browser_client.take();
    assert!(browser.is_some());
    open_full(&s, who.clone(), "4", &dispatch).await.unwrap().unwrap();
    let (_, u) = call(&app, "GET", "/v1/sources/tidal/unofficial", None).await;
    assert_eq!(
        u["health"]["detail"],
        tidal_unofficial::AAC_ONLY_NO_BROWSER,
        "{u}"
    );
    assert!(!tidal_unofficial::AAC_ONLY_NO_BROWSER.contains("again"));
    s.tidal.write().unwrap().browser_client = browser;
    // An imported device session wasn't signed in here: Sign in with TIDAL, not "again"
    tidal_unofficial::update(&s, &who, |st| {
        st.session.as_mut().unwrap().from = Some("tidal-dl-ng".into())
    });
    open_full(&s, who.clone(), "4", &dispatch).await.unwrap().unwrap();
    let (_, u) = call(&app, "GET", "/v1/sources/tidal/unofficial", None).await;
    assert_eq!(u["health"]["detail"], tidal_unofficial::AAC_ONLY_IMPORTED, "{u}");
    tidal_unofficial::update(&s, &who, |st| st.session.as_mut().unwrap().from = None);
    // A browser session's AAC track: that track, not the sign-in
    call(
        &app,
        "PUT",
        "/v1/sources/tidal/unofficial",
        Some(serde_json::json!({"quality": "max"})),
    )
    .await;
    tidal_unofficial::update(&s, &who, |st| {
        st.session.as_mut().unwrap().client = tidal_unofficial::ClientKind::Pkce
    });
    open_full(&s, who.clone(), "4", &dispatch).await.unwrap().unwrap();
    let (_, u) = call(&app, "GET", "/v1/sources/tidal/unofficial", None).await;
    assert_eq!(u["health"]["detail"], tidal_unofficial::AAC_TRACK, "{u}");

    // One line per opened track with what TIDAL answered; no address or token in any line
    let text = logs.text();
    let lines: Vec<&str> = text.lines().filter(|l| l.contains("TIDAL full track ")).collect();
    let line = |id: &str| {
        lines
            .iter()
            .find(|l| l.contains(&format!("TIDAL full track {id} ")))
            .copied()
            .unwrap_or_else(|| panic!("no line for {id}: {text}"))
    };
    assert!(line("1").contains("(code sign-in): asked HI_RES_LOSSLESS, TIDAL answered audioQuality=LOSSLESS manifestMimeType=application/vnd.tidal.bts codecs=flac bitDepth=16 sampleRate=44100"), "{}", line("1"));
    assert!(line("2").contains("audioQuality=HI_RES_LOSSLESS manifestMimeType=application/dash+xml codecs=flac bitDepth=24 sampleRate=96000"), "{}", line("2"));
    assert!(line("3").contains("asked LOSSLESS, TIDAL answered audioQuality=LOSSLESS") && line("3").contains("(HI_RES_LOSSLESS had answered audioQuality=HIGH manifestMimeType=application/dash+xml codecs=mp4a.40.2 sampleRate=44100)"), "{}", line("3"));
    assert!(line("4").contains("codecs=mp4a.40.2"), "{}", line("4"));
    assert!(lines.iter().any(|l| l.contains("(browser sign-in)")), "{text}");
    for secret in [
        "SECRET-CDN",
        "dev-secret",
        "dev-at",
        "dev-rt",
        FAKE_BROWSER_SECRET,
    ] {
        assert!(!text.contains(secret), "a secret in the logs: {secret}");
    }
}

/// The vault carries each kind of TIDAL full-tracks session under its own app: a code sign-in
/// under the device client's id (as before), a Sign in with TIDAL one under `pkce:` and the
/// browser client's id. A node takes only a session it can renew, and a code sign-in made on a
/// node without the browser client doesn't replace a lossless session made elsewhere.
#[tokio::test]
async fn the_vault_carries_each_kind_of_tidal_session_under_its_own_app() {
    use vault_cache::{
        decide, made_with_our_app, node_app_for, sign_in_for, Do, Side, Synced, VaultSide, Wipe,
    };
    let b = vault_cache::BRIDGES
        .iter()
        .find(|b| b.service == "tidal-full")
        .unwrap();
    let node = |device: &'static str, browser: bool| async move {
        let dir = tempfile::tempdir().unwrap();
        let s = state(dir.path(), None).await;
        s.library
            .lock()
            .unwrap()
            .set_setting(
                tidal_unofficial::CLIENT_KEY,
                &serde_json::json!({"id": device, "secret": "fake-device-secret", "from": "settings"})
                    .to_string(),
            )
            .unwrap();
        s.tidal.write().unwrap().browser_client = browser.then(|| tidal_unofficial::BrowserClient {
            id: "fake-browser-client-id".into(),
            secret: "fake-browser-secret".into(),
        });
        (dir, s)
    };
    let travels = |client: Option<&str>, at: &str| {
        let mut sess = serde_json::json!({"access_token": at, "refresh_token": "fake-rt", "expires_at": 4_000_000_000i64, "user_id": "42"});
        if let Some(c) = client {
            sess["client"] = serde_json::json!(c);
        }
        serde_json::json!({"tidal.unofficial": {"session": sess}})
            .as_object()
            .unwrap()
            .clone()
    };
    let (device, browser) = (
        travels(None, "fake-at-device"),
        travels(Some("pkce"), "fake-at-browser"),
    );
    let (_d1, both) = node("fake-device-client-id", true).await;
    // Labelled by kind; the device one exactly as before, so device sessions synced already
    // still match
    let device_app = node_app_for(&both, b, Some(&device));
    let browser_app = node_app_for(&both, b, Some(&browser));
    assert_eq!(device_app.as_deref(), Some("fake-device-client-id"));
    assert_eq!(browser_app.as_deref(), Some("pkce:fake-browser-client-id"));
    assert_eq!(
        node_app_for(&both, b, None),
        device_app,
        "no copy: the usual kind's"
    );
    let in_vault_device = sign_in_for(b, &device, None, device_app.clone(), "L-device");
    let in_vault_browser = sign_in_for(b, &browser, None, browser_app.clone(), "L-browser");
    assert_eq!(
        in_vault_browser.client_id.as_deref(),
        Some("pkce:fake-browser-client-id")
    );
    assert!(made_with_our_app(&both, b, &in_vault_device));
    assert!(made_with_our_app(&both, b, &in_vault_browser));
    // The same device client, no browser client (a build from source): the device session
    // is taken, the browser one isn't (it couldn't be renewed here)
    let (_d2, no_browser) = node("fake-device-client-id", false).await;
    assert_eq!(node_app_for(&no_browser, b, Some(&browser)), None);
    assert!(made_with_our_app(&no_browser, b, &in_vault_device));
    assert!(!made_with_our_app(&no_browser, b, &in_vault_browser));
    // A device client of the owner's and this build's browser client: the browser session is
    // taken (it renews with the browser client), the other device client's isn't
    let (_d3, own_device) = node("fake-own-device-client-id", true).await;
    assert!(made_with_our_app(&own_device, b, &in_vault_browser));
    assert!(!made_with_our_app(&own_device, b, &in_vault_device));
    // On the node without the browser client: it synced a code sign-in, another node then
    // signed in with TIDAL. Its own code sign-in, refreshed or new, doesn't replace that
    // lossless session in the vault
    let side = |hash: &str, lineage: &str| Side {
        hash: hash.into(),
        account: Some("42".into()),
        lineage: Some(lineage.into()),
        expires_at: Some(4_000_000_000),
    };
    let seen = Synced {
        hash: "h-device".into(),
        account: Some("42".into()),
        lineage: Some("L-device".into()),
        app: device_app.clone(),
        ..Default::default()
    };
    let theirs = VaultSide {
        side: side("h-browser", "L-browser"),
        from_a_node: true,
        our_app: made_with_our_app(&no_browser, b, &in_vault_browser),
        vouched: true,
    };
    let wipe = Wipe::default();
    let refreshed = side("h-device-2", "L-device");
    let new_code = side("h-device-3", "L-device-new");
    assert_eq!(
        decide(Some(&seen), Some(&refreshed), Some(&theirs), &wipe),
        Do::NotUsable
    );
    assert_ne!(
        decide(Some(&seen), Some(&new_code), Some(&theirs), &wipe),
        Do::Push
    );
    assert_ne!(decide(None, Some(&new_code), Some(&theirs), &wipe), Do::Push);
    // ...while on a node with the browser client, the same vault copy is taken
    let ours = VaultSide {
        our_app: made_with_our_app(&both, b, &in_vault_browser),
        ..theirs.clone()
    };
    assert_eq!(
        decide(Some(&seen), Some(&refreshed), Some(&ours), &wipe),
        Do::Pull
    );
}

/// The chooser's Sign out while TIDAL is asked for the browser sign-in's tokens: the attempt
/// ends with the sign-out, so the answer is not kept (`tidal::sign_out_of`).
#[tokio::test]
async fn tidal_sign_out_during_the_browser_exchange_is_not_undone() {
    let arrived = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let mock = Router::new()
        .route(
            "/v1/oauth2/token",
            post({
                let (arrived, release) = (arrived.clone(), release.clone());
                move || {
                    let (arrived, release) = (arrived.clone(), release.clone());
                    async move {
                        arrived.notify_one();
                        release.notified().await;
                        Json(serde_json::json!({"access_token": "late-at", "refresh_token": "late-rt", "expires_in": 3600, "user_id": 77, "user": {"userId": 77, "countryCode": "CH"}}))
                    }
                }
            }),
        )
        .route(
            "/v1/sessions",
            get(|| async { Json(serde_json::json!({"userId": 77, "countryCode": "CH", "sessionId": "sid"})) }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    point_tidal_at(&s, &base, true);
    let app = router(s.clone());
    let signin = "/v1/sources/tidal/signin";
    // A code sign-in in use, then Sign in with TIDAL started
    tidal_unofficial::update(&s, &accounts::Who::Owner, |st| {
        st.enabled = true;
        st.session = Some(tidal_unofficial::Session {
            access_token: "code-at".into(),
            refresh_token: Some("code-rt".into()),
            expires_at: 4_000_000_000,
            user_id: Some("42".into()),
            ..Default::default()
        });
    });
    let (st, r) = call(&app, "POST", "/v1/sources/tidal/unofficial/browser", None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    let (_, si) = call(&app, "GET", signin, None).await;
    assert_eq!(si["in_use"]["way"], "device", "{si}");
    // The paste goes in; TIDAL holds its answer while the person signs out
    let finish = tokio::spawn({
        let app = app.clone();
        async move {
            call(
                &app,
                "POST",
                "/v1/sources/tidal/unofficial/browser/finish",
                Some(serde_json::json!({ "text": "https://tidal.com/android/login/auth?code=late-code" })),
            )
            .await
        }
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), arrived.notified())
        .await
        .expect("the token request reached TIDAL");
    let (st, si) = call(&app, "DELETE", signin, None).await;
    assert_eq!(st, StatusCode::OK, "{si}");
    assert!(si["in_use"].is_null(), "{si}");
    let m = tidal_unofficial::load(&s.library.lock().unwrap(), &accounts::Who::Owner);
    assert!(m.browser.is_none(), "the sign-out ends the attempt");
    release.notify_one();
    let (st, r) = finish.await.unwrap();
    assert_eq!(st, StatusCode::CONFLICT, "{r}");
    assert!(r["error"].as_str().unwrap().contains("changed meanwhile"), "{r}");
    let m = tidal_unofficial::load(&s.library.lock().unwrap(), &accounts::Who::Owner);
    assert!(
        m.session.is_none() && !m.enabled,
        "signed out stays signed out (session kept: {})",
        m.session.is_some()
    );
    let (_, si) = call(&app, "GET", signin, None).await;
    assert!(si["in_use"].is_null(), "{si}");
}
