//! How services are signed in to: ways, apps, the vault, whose sign-in a play uses.
//!
//! Part of pm-api's unit tests (`mod tests` in lib.rs, which holds the helpers these use). A new
//! test of this area goes at the end of this file.

use super::*;

/// A source switched off on Sources: hidden from search and the play order, kept listed,
/// remembered across restarts.
#[tokio::test]
async fn a_source_switched_off_is_hidden_and_keeps_its_settings() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (_, sources) = call(&app, "GET", "/v1/sources", None).await;
    assert!(
        sources.as_array().unwrap().iter().all(|x| x["enabled"] == true),
        "{sources}"
    );
    for id in ["deezer", "tidal"] {
        let (st, r) = call(
            &app,
            "PUT",
            &format!("/v1/sources/{id}/enabled"),
            Some(serde_json::json!({"enabled": false})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{r}");
        assert_eq!(r["enabled"], false);
    }
    let (_, sources) = call(&app, "GET", "/v1/sources", None).await;
    let deezer = sources
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "deezer")
        .unwrap()
        .clone();
    assert_eq!(
        deezer["enabled"], false,
        "still listed, so it can be switched on again"
    );
    let (st, _) = call(&app, "GET", "/v1/sources/deezer/search?q=x", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "an off source does not search");
    assert!(!crate::search::sources_for(&s, "everywhere").contains(&"deezer".to_string()));
    assert_eq!(crate::search::sources_for(&s, "deezer"), ["library"]);
    let (_, po) = call(&app, "GET", "/v1/play-order", None).await;
    let tidal = po["services"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "tidal")
        .unwrap()
        .clone();
    assert_eq!(tidal["note"], "turned off on Sources", "{po}");
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/sources/nope/enabled",
        Some(serde_json::json!({"enabled": false})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // Kept across a restart; switching on brings it back
    drop(app);
    drop(s);
    let s = state(dir.path(), None).await;
    assert!(!s.source_enabled("deezer") && !s.source_enabled("tidal") && s.source_enabled("jamendo"));
    let app = router(s.clone());
    let (_, r) = call(
        &app,
        "PUT",
        "/v1/sources/deezer/enabled",
        Some(serde_json::json!({"enabled": true})),
    )
    .await;
    assert_eq!(r["enabled"], true);
    assert!(crate::search::sources_for(&s, "everywhere").contains(&"deezer".to_string()));
}

#[tokio::test]
async fn a_source_turned_off_answers_404_on_its_own_routes() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    for (id, method, path) in [
        ("podcasts", "GET", "/v1/podcasts/subscriptions"),
        ("radio", "GET", "/v1/radio/tags"),
        ("plex", "GET", "/v1/sources/plex/stream/1"),
        ("audiobookshelf", "GET", "/v1/sources/audiobookshelf/continue"),
    ] {
        s.set_source_enabled(id, false).unwrap();
        let (st, v) = call(&app, method, path, None).await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{path}: {v}");
        assert!(v.to_string().contains("turned off on Sources"), "{path}: {v}");
        s.set_source_enabled(id, true).unwrap();
        if id == "radio" {
            continue; // on, it asks radio-browser.info: no network in unit tests
        }
        let (st, _) = call(&app, method, path, None).await;
        assert_ne!(st, StatusCode::NOT_FOUND, "{path} works again when on");
    }
    // Settings stay readable while off, so the tab can show (and clear) a connection
    s.set_source_enabled("plex", false).unwrap();
    let (st, _) = call(&app, "GET", "/v1/sources/plex/server", None).await;
    assert_eq!(st, StatusCode::OK);
}

/// Controllers sign in with Pubky Ring; without a node session there is
/// no owner to compare against, so the node says so instead of starting: at once, whatever
/// the network does (here a PKARR relay that never answers, on the public network).
#[tokio::test]
async fn controller_sign_in_needs_the_node_signed_in_first() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), Some("t0k")).await;
    let stalled = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = format!("http://127.0.0.1:{}/", stalled.local_addr().unwrap().port());
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((c, _)) = stalled.accept().await {
            held.push(c);
        }
    });
    let mut b = pubky::PubkyHttpClient::builder();
    b.pkarr(move |p| {
        p.no_dht();
        p.relays(&[relay.as_str()]).unwrap()
    });
    s.set_homeserver(pm_pubky::homeserver::NodeHomeserver::from_pubky_on(
        pubky::Pubky::with_client(b.build().unwrap()),
        pm_pubky::Network::Mainnet,
    ));
    let app = router(s);
    let started = std::time::Instant::now();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/pubky/start")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PRECONDITION_FAILED);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "no homeserver lookup first: {:?}",
        started.elapsed()
    );
    let (st, v) = call(&app, "GET", "/v1/auth/pubky/poll?id=nope", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "expired");
}

/// Anyone may start a Pubky Ring sign-in, so few wait at once: six per address (IPv6 per
/// /64), 64 in all. Nobody is refused: the oldest waiting one makes room, its own when an
/// address has its share, else the biggest bucket's, so one client can't lock everyone out
/// (security reviews 2026-10-30, #4 and of #177).
#[test]
fn sign_ins_make_room_instead_of_locking_everyone_out() {
    let t0 = std::time::Instant::now();
    let at = |n: u64| t0 + std::time::Duration::from_secs(n);
    let entry = |id: &str, n: u64, o: &str| (id.to_string(), at(n), o.to_string());
    // One address with its share: its own oldest goes
    let mine: Vec<_> = (0..MAX_LOGINS_PER_ORIGIN as u64)
        .map(|n| entry(&format!("a{n}"), n, "1.2.3.4"))
        .collect();
    assert_eq!(make_room(&mine, "1.2.3.4").as_deref(), Some("a0"));
    assert_eq!(make_room(&mine, "5.6.7.8"), None, "another address has room");
    // Every place taken (an attacker's ten addresses and some others): the biggest bucket's
    // oldest makes room for a newcomer
    let mut full: Vec<_> = (0..60u64)
        .map(|n| entry(&format!("x{n}"), 100 + n, &format!("10.0.0.{}", n % 10)))
        .collect();
    full.extend((0..4u64).map(|n| entry(&format!("o{n}"), n, &format!("192.168.1.{n}"))));
    assert_eq!(full.len(), MAX_LOGINS);
    let gone = make_room(&full, "192.168.1.50").unwrap();
    assert!(
        gone.starts_with('x'),
        "an attacker's goes, not a lone sign-in: {gone}"
    );
    assert_eq!(gone, "x0", "the oldest of the biggest bucket");
    let req = |h: Option<(&str, &str)>, from: Option<&str>| {
        let mut b = Request::builder().uri("/v1/auth/pubky/start");
        if let Some((k, v)) = h {
            b = b.header(k, v);
        }
        let mut r = b.body(Body::empty()).unwrap();
        if let Some(a) = from {
            r.extensions_mut().insert(axum::extract::ConnectInfo(
                a.parse::<std::net::SocketAddr>().unwrap(),
            ));
        }
        r
    };
    assert_eq!(login_origin(&req(None, Some("192.168.1.9:5000"))), "192.168.1.9");
    // Behind a local proxy: the address it saw is the last one; what the client wrote before
    // it, and Cloudflare-style headers it could set itself, don't count
    assert_eq!(
        login_origin(&req(
            Some(("x-forwarded-for", "6.6.6.6, 198.51.100.2")),
            Some("127.0.0.1:1")
        )),
        "198.51.100.2"
    );
    assert_eq!(
        login_origin(&req(
            Some(("cf-connecting-ip", "203.0.113.7")),
            Some("127.0.0.1:1")
        )),
        "127.0.0.1"
    );
    // From anyone else a forwarded address is ignored
    assert_eq!(
        login_origin(&req(
            Some(("x-forwarded-for", "198.51.100.2")),
            Some("192.168.1.9:5000")
        )),
        "192.168.1.9"
    );
    // IPv6 by /64: one network is one bucket
    assert_eq!(
        login_origin(&req(None, Some("[2001:db8:1:2:aaaa::1]:5000"))),
        "2001:db8:1:2::/64"
    );
    assert_eq!(
        login_origin(&req(None, Some("[2001:db8:1:2:bbbb::9]:5000"))),
        "2001:db8:1:2::/64"
    );
    let mut tunnel = req(None, Some("127.0.0.1:1"));
    tunnel.extensions_mut().insert(peer::Remote);
    assert_eq!(login_origin(&tunnel), "iroh");
}

/// Shared apps (STEWARD-SHARED-APPS.md): an app without its secret on the node (the project's
/// TIDAL app, a Spotify app from the guided setup) signs in with PKCE through the return page,
/// and no secret ever reaches the token endpoint.
#[tokio::test]
async fn apps_without_a_secret_sign_in_through_the_return_page() {
    use base64::Engine;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let bodies = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let token = {
        let bodies = bodies.clone();
        move |headers: axum::http::HeaderMap, body: String| {
            let bodies = bodies.clone();
            async move {
                bodies.lock().unwrap().push(body.clone());
                if headers.contains_key("authorization") || body.contains("client_secret") {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(serde_json::json!({"error": "invalid_client"})),
                    );
                }
                if body.contains("grant_type=client_credentials") {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(serde_json::json!({"error": "invalid_client"})),
                    );
                }
                assert!(
                    body.contains("code_verifier=") && body.contains("code=the-code"),
                    "{body}"
                );
                (
                    StatusCode::OK,
                    Json(
                        serde_json::json!({"access_token": "user-token", "refresh_token": "r1", "expires_in": 3600, "user_id": 4242, "scope": "x"}),
                    ),
                )
            }
        }
    };
    let mock = Router::new()
        .route("/v1/oauth2/token", post(token.clone()))
        .route("/api/token", post(token))
        .route("/users/{id}", get(|| async { Json(serde_json::json!({"data": {"id": "4242", "type": "users", "attributes": {"country": "CH"}}})) }))
        .route("/me", get(|| async { Json(serde_json::json!({"id": "listener", "display_name": "Listener", "country": "CH", "product": "premium"})) }));
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let page = "https://return.example/callback";
    s.set_return_page(page);
    {
        let mut ep = s.tidal.write().unwrap();
        (ep.api_base, ep.auth_base, ep.login_base) = (base.clone(), base.clone(), base.clone());
        ep.creds = tidal::TidalCredentials {
            client_id: "project-app".into(),
            client_secret: String::new(),
            scopes: String::new(),
        };
    }
    {
        let mut ep = s.spotify.write().unwrap();
        (ep.api_base, ep.accounts_base) = (base.clone(), base.clone());
    }
    let app = router(s.clone());
    // The login URL names the return page, and state carries the nonce and the node's callback
    let sign_in = |path: &'static str, callback: &'static str| {
        let app = app.clone();
        async move {
            let (st, l) = call(&app, "GET", path, None).await;
            assert_eq!(st, StatusCode::OK, "{l}");
            let url = url::Url::parse(l["url"].as_str().unwrap()).unwrap();
            let q: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
            assert_eq!(q["redirect_uri"], page);
            assert_eq!(q["code_challenge_method"], "S256");
            let (_, enc) = q["state"].split_once('.').expect("state = nonce.callback");
            let back = String::from_utf8(
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(enc)
                    .unwrap(),
            )
            .unwrap();
            assert!(back.ends_with(callback), "{back}");
            // The return page forwards every parameter unchanged to that callback
            let req = Request::builder()
                .uri(format!(
                    "{callback}?code=the-code&state={}",
                    url::form_urlencoded::byte_serialize(q["state"].as_bytes()).collect::<String>()
                ))
                .body(Body::empty())
                .unwrap();
            let resp = app.clone().oneshot(req).await.unwrap();
            resp.headers()
                .get("location")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string()
        }
    };

    // TIDAL with the project's app: no catalogue before sign-in (client credentials need the secret)
    let (st, _) = call(&app, "GET", "/v1/sources/tidal/search?q=x", None).await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED);
    let (_, stv) = call(&app, "GET", "/v1/sources/tidal/status", None).await;
    assert_eq!(
        stv["redirect_uri"], page,
        "the redirect to register is the return page"
    );
    // The pasted-address fallback: the same parameters, typed in instead of arriving by redirect
    let (_, l) = call(&app, "GET", "/v1/sources/tidal/login", None).await;
    let lurl = url::Url::parse(l["url"].as_str().unwrap()).unwrap();
    let state_param = lurl
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.into_owned())
        .unwrap();
    let (st, r) = call(
        &app,
        "POST",
        "/v1/sources/tidal/callback",
        Some(serde_json::json!({"url": "not a url"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{r}");
    let (st, r) = call(
        &app,
        "POST",
        "/v1/sources/tidal/callback",
        Some(serde_json::json!({"url": "https://gillohner.github.io/pubky-music-auth/callback?foo=bar"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "no code: {r}");
    let pasted = format!(
        "http://192.168.1.5:7788/tidal/callback?code=the-code&state={}",
        url::form_urlencoded::byte_serialize(state_param.as_bytes()).collect::<String>()
    );
    let (st, r) = call(
        &app,
        "POST",
        "/v1/sources/tidal/callback",
        Some(serde_json::json!({"url": pasted})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["signed_in"], true, "{r}");
    call(&app, "DELETE", "/v1/sources/tidal/session", None).await;
    assert_eq!(
        sign_in("/v1/sources/tidal/login", "/tidal/callback").await,
        "/?tidal=signed_in"
    );
    let (_, stv) = call(&app, "GET", "/v1/sources/tidal/status", None).await;
    assert_eq!(stv["signed_in"], true, "{stv}");
    assert_eq!(stv["public_client"]["outcome"], "accepted", "{stv}");
    // A state whose nonce the node never issued is refused
    let req = Request::builder()
        .uri("/tidal/callback?code=the-code&state=forged.aHR0cDovL3g")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert!(resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .contains("error"));

    // Spotify: the guided setup takes only a client id
    let (_, sp) = call(&app, "GET", "/v1/sources/spotify/status", None).await;
    assert_eq!(sp["configured"], false);
    assert_eq!(sp["redirect_uri"], page);
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/sources/spotify/app",
        Some(serde_json::json!({"client_id": "not-an-id"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let id = "0123456789abcdef0123456789ABCDEF";
    let (st, sp) = call(
        &app,
        "PUT",
        "/v1/sources/spotify/app",
        Some(serde_json::json!({"client_id": id})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{sp}");
    assert_eq!(sp["app_from"], "settings");
    assert_eq!(sp["client_id"], id.to_ascii_lowercase());
    let (_, srcs) = call(&app, "GET", "/v1/sources", None).await;
    let row = srcs
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "spotify")
        .unwrap()
        .clone();
    assert_eq!(row["status"], "needs sign-in");
    assert_eq!(
        sign_in("/v1/sources/spotify/login", "/spotify/callback").await,
        "/?spotify=ok"
    );
    let (_, sp) = call(&app, "GET", "/v1/sources/spotify/status", None).await;
    assert_eq!(sp["signed_in"], true, "{sp}");
    assert_eq!(sp["display_name"], "Listener");
    assert!(bodies
        .lock()
        .unwrap()
        .iter()
        .filter(|b| b.contains("authorization_code"))
        .all(|b| b.contains("client_id=") && !b.contains("client_secret")));
    // Forgetting the app signs out of it
    let (_, sp) = call(
        &app,
        "PUT",
        "/v1/sources/spotify/app",
        Some(serde_json::json!({"client_id": ""})),
    )
    .await;
    assert_eq!(
        (sp["configured"].clone(), sp["signed_in"].clone()),
        (serde_json::json!(false), serde_json::json!(false))
    );
    // An app in the node config can't be replaced from the page
    s.spotify.write().unwrap().creds = spotify::SpotifyCredentials {
        client_id: "cid".into(),
        client_secret: "sec".into(),
    };
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/sources/spotify/app",
        Some(serde_json::json!({"client_id": id})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
}

/// Every source declares what it can do and a stage; the Sources list carries both.
#[tokio::test]
async fn sources_declare_their_features_and_stage() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (_, rows) = call(&app, "GET", "/v1/sources", None).await;
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), streaming::services().len());
    for r in rows {
        let feats = r["features"].as_array().unwrap();
        assert!(!feats.is_empty(), "{} declares nothing", r["id"]);
        for f in feats {
            assert!(
                streaming::FEATURES.contains(&f.as_str().unwrap()),
                "{}: unknown feature {f}",
                r["id"]
            );
        }
        assert!(
            streaming::STAGES.contains(&r["stage"].as_str().unwrap()),
            "{}: stage {}",
            r["id"],
            r["stage"]
        );
    }
    let by = |id: &str| rows.iter().find(|r| r["id"] == id).unwrap().clone();
    assert_eq!(by("qobuz")["stage"], "unofficial");
    assert_eq!(by("tidal")["stage"], "stable");
    assert!(by("spotify")["features"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("connect")));
    assert!(!by("spotify")["features"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("play")));
    assert!(by("bandcamp")["features"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("downloads")));
}

/// One front for the quality choice (STEWARD-UX-PLAYER-SOURCES.md §2): the options, the pick
/// kept in the source's own state, no choice where the service sends one format; and
/// `healthy` on the Sources list.
#[tokio::test]
async fn quality_is_one_front_per_source_and_sources_say_healthy() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let ids = |q: &serde_json::Value| {
        q["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    let (st, q) = call(&app, "GET", "/v1/sources/tidal/quality", None).await;
    assert_eq!(st, StatusCode::OK, "{q}");
    assert_eq!(ids(&q), ["max", "high"]);
    assert_eq!(q["current"], "max");
    assert_eq!(q["applies"], "next_track");
    assert!(q["note"].as_str().unwrap().contains("Sign in with TIDAL"), "{q}");
    let (st, r) = call(
        &app,
        "PUT",
        "/v1/sources/tidal/quality",
        Some(serde_json::json!({"id": "high"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["current"], "high");
    let (_, u) = call(&app, "GET", "/v1/sources/tidal/unofficial", None).await;
    assert_eq!(u["quality"], "high", "the source's own state follows");
    let (st, r) = call(
        &app,
        "PUT",
        "/v1/sources/tidal/quality",
        Some(serde_json::json!({"id": "lossy"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{r}");
    assert!(r["error"].as_str().unwrap().contains("max, high"), "{r}");
    let (_, q) = call(&app, "GET", "/v1/sources/qobuz/quality", None).await;
    assert_eq!(ids(&q), ["max", "cd", "mp3"]);
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/sources/qobuz/quality",
        Some(serde_json::json!({"id": "mp3"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (_, u) = call(&app, "GET", "/v1/sources/qobuz/unofficial", None).await;
    assert_eq!(u["quality"], "mp3");
    let (_, q) = call(&app, "GET", "/v1/sources/spotify/quality", None).await;
    assert_eq!(ids(&q), ["320", "160", "96"]);
    assert_eq!(q["current"], "320");
    assert_eq!(q["applies"], "now");
    let (st, r) = call(
        &app,
        "PUT",
        "/v1/sources/spotify/quality",
        Some(serde_json::json!({"id": "160"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["current"], "160");
    let (_, l) = call(&app, "GET", "/v1/sources/spotify/librespot", None).await;
    assert_eq!(l["bitrate"], "160");
    let (st, q) = call(&app, "GET", "/v1/sources/deezer/quality", None).await;
    assert_eq!(st, StatusCode::OK, "{q}");
    assert!(q["options"].as_array().unwrap().is_empty());
    assert!(q["note"].as_str().unwrap().contains("previews"), "{q}");
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/sources/deezer/quality",
        Some(serde_json::json!({"id": "x"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = call(&app, "GET", "/v1/sources/nope/quality", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    // healthy: on and working
    let (_, srcs) = call(&app, "GET", "/v1/sources", None).await;
    let row = |srcs: &serde_json::Value, id: &str| {
        srcs.as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(row(&srcs, "deezer")["healthy"], true, "{srcs}");
    assert_eq!(row(&srcs, "tidal")["healthy"], false, "not signed in: {srcs}");
    call(
        &app,
        "PUT",
        "/v1/sources/deezer/enabled",
        Some(serde_json::json!({"enabled": false})),
    )
    .await;
    let (_, srcs) = call(&app, "GET", "/v1/sources", None).await;
    assert_eq!(row(&srcs, "deezer")["healthy"], false, "off: {srcs}");
}

/// Sources › Qobuz, YouTube, Spotify and Settings › Last.fm have the same Sign in chooser
#[tokio::test]
async fn imported_sign_ins_win_over_a_synced_sessions_refresh() {
    use vault_cache::{account, carried, decide, hash, Do, Side, Synced, VaultSide, Wipe, BRIDGES};

    let _lastfm_guard = lastfm::API_TEST_LOCK.lock().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    lastfm::set_api_base(&format!("{base}/lastfm"));
    let mock = Router::new().route(
        "/lastfm",
        post(|body: String| async move {
            let form: std::collections::HashMap<_, _> =
                url::form_urlencoded::parse(body.as_bytes()).collect();
            assert_eq!(form["method"], "user.getInfo");
            assert_eq!(form["sk"], "imported-session");
            Json(serde_json::json!({"user": {"name": "listener"}}))
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let mut decisions = Vec::new();
    for (service, app_key, session_key, app, session, text) in [(
        "lastfm",
        lastfm::APP_KEY,
        lastfm::SESSION_KEY,
        serde_json::json!({"api_key": "0123456789abcdef0123456789abcdef", "api_secret": "fedcba9876543210fedcba9876543210", "from": "settings"}),
        serde_json::json!({"key": "old-session", "user": "listener", "submit": true}),
        "0123456789abcdef0123456789abcdef\nfedcba9876543210fedcba9876543210\nimported-session\n",
    )] {
        let bridge = BRIDGES.iter().find(|b| b.service == service).unwrap();
        let lineage_key = format!("vault.lineage.{service}");
        let seen = {
            let lib = s.library.lock().unwrap();
            lib.set_setting(app_key, &app.to_string()).unwrap();
            lib.set_setting(session_key, &session.to_string()).unwrap();
            vault_cache::new_sign_in(&lib, service);
            let travels = carried(&lib, bridge).unwrap().unwrap();
            Synced {
                revision: 1,
                hash: hash(&travels),
                account: account(&travels),
                lineage: lib.get_setting(&lineage_key).unwrap(),
                ..Default::default()
            }
        };
        // Another node refreshed the sign-in we last synced. Importing even the same
        // account must start a new sign-in, not lose to that refresh on the next pass.
        let vault = VaultSide {
            side: Side {
                hash: "refreshed-in-the-vault".into(),
                account: seen.account.clone(),
                lineage: seen.lineage.clone(),
                ..Default::default()
            },
            from_a_node: true,
            our_app: true,
            vouched: true,
        };
        signin::source(service)
            .unwrap()
            .import(
                &s,
                &accounts::Who::Owner,
                signin::ImportBody {
                    text: Some(text.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let lib = s.library.lock().unwrap();
        let travels = carried(&lib, bridge).unwrap().unwrap();
        let imported = Side {
            hash: hash(&travels),
            account: account(&travels),
            lineage: lib.get_setting(&lineage_key).unwrap(),
            ..Default::default()
        };
        assert_ne!(imported.hash, seen.hash, "{service}: the new session was saved");
        decisions.push((
            service,
            decide(Some(&seen), Some(&imported), Some(&vault), &Wipe::default()),
        ));
    }
    server.abort();
    assert_eq!(decisions, [("lastfm", Do::Push)]);
}

/// (STEWARD-CREDENTIALS-REUSE.md step 3): other apps' own session files are listed from the
/// known places; a cookies file or a librespot credentials.json is only ever picked or pasted,
/// kept readable by the node only, and gone with Sign out; nothing answers with a token.
#[tokio::test]
async fn the_other_sources_have_sign_in_ways_and_picked_files_stay_private() {
    let _home_guard = imports::HOME_TEST_LOCK.lock().await;
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".config/streamrip")).unwrap();
    std::fs::write(
        home.path().join(".config/streamrip/config.toml"),
        "[qobuz]\nuse_auth_token = true\nemail_or_userid = \"42\"\npassword_or_token = \"qobuz-token-secret\"\napp_id = \"100000000\"\nsecrets = [\"s3cret\"]\n[tidal]\n",
    )
    .unwrap();
    std::fs::create_dir_all(home.path().join(".config/rescrobbled")).unwrap();
    std::fs::write(
        home.path().join(".config/rescrobbled/config.toml"),
        "lastfm-key = \"0123456789abcdef0123456789abcdef\"\nlastfm-secret = \"fedcba9876543210fedcba9876543210\"\n",
    )
    .unwrap();
    std::fs::write(
        home.path().join(".config/rescrobbled/session"),
        "sessKeyRescrobbled\n",
    )
    .unwrap();
    *imports::HOME_OVERRIDE.write().unwrap() = Some(home.path().to_path_buf());
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let ids = |si: &serde_json::Value| {
        si["ways"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    let row = |srcs: &serde_json::Value, id: &str| {
        srcs.as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == id)
            .unwrap()
            .clone()
    };
    // Qobuz has one email/password way and never scans streamrip's Qobuz section.
    let (st, q) = call(&app, "GET", "/v1/sources/qobuz/signin", None).await;
    assert_eq!(st, StatusCode::OK, "{q}");
    assert_eq!(ids(&q), ["own"]);
    assert!(q["imports"].as_array().is_none_or(|v| v.is_empty()));
    assert!(q["places"].as_array().is_none_or(|v| v.is_empty()));
    assert!(!q.to_string().contains("qobuz-token-secret") && !q.to_string().contains("s3cret"));
    assert_eq!(q["line"], "Not signed in");
    let (st, _) = call(
        &app,
        "POST",
        "/v1/sources/qobuz/signin/import",
        Some(serde_json::json!({"id": "streamrip"})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    // Last.fm: rescrobbled's files are listed (the import itself is in the Last.fm mock test)
    let (st, l) = call(&app, "GET", "/v1/sources/lastfm/signin", None).await;
    assert_eq!(st, StatusCode::OK, "{l}");
    assert_eq!(ids(&l), ["own", "import", "paste"]);
    assert_eq!(l["imports"][0]["id"], "rescrobbled");
    assert_eq!(l["imports"][0]["usable"], true, "{l}");
    assert!(
        !l.to_string().contains("sessKeyRescrobbled") && !l.to_string().contains("fedcba98"),
        "{l}"
    );
    // YouTube: a cookies file is picked, never looked for; the copy is the node's own, 0600
    let (_, y) = call(&app, "GET", "/v1/sources/youtube/signin", None).await;
    assert_eq!(ids(&y), ["key", "cookies"]);
    assert!(y["imports"].as_array().is_none_or(|a| a.is_empty()), "{y}");
    assert!(y["places"][0].as_str().unwrap().contains("not looked for"), "{y}");
    assert_eq!(y["line"], "Not signed in");
    let cookies =
        "# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t2000000000\tSID\tcookie-secret\n";
    let (st, r) = call(
        &app,
        "POST",
        "/v1/sources/youtube/signin/import",
        Some(serde_json::json!({"text": cookies})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["in_use"]["way"], "cookies");
    assert_eq!(r["line"], "yt-dlp cookies · signed-in playback");
    assert!(!r.to_string().contains("cookie-secret"), "{r}");
    let file = s.data_dir().join("youtube").join("cookies.txt");
    assert!(file.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let (_, srcs) = call(&app, "GET", "/v1/sources", None).await;
    assert_eq!(
        row(&srcs, "youtube")["sign_in"],
        "yt-dlp cookies · signed-in playback"
    );
    let (st, r) = call(
        &app,
        "POST",
        "/v1/sources/youtube/signin/import",
        Some(serde_json::json!({"text": "junk"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{r}");
    let (_, r) = call(&app, "DELETE", "/v1/sources/youtube/signin", None).await;
    assert_eq!(r["line"], "Not signed in");
    assert!(!file.exists(), "the copy goes with Sign out");
    // Spotify: a librespot credentials.json is picked; librespot itself may be missing here
    let (_, sp) = call(&app, "GET", "/v1/sources/spotify/signin", None).await;
    assert_eq!(ids(&sp), ["app", "librespot", "librespot-file"]);
    assert!(
        sp["places"][0].as_str().unwrap().contains("not looked for"),
        "{sp}"
    );
    let creds = r#"{"username": "alice", "auth_type": 1, "auth_data": "QUJD"}"#;
    let (st, r) = call(
        &app,
        "POST",
        "/v1/sources/spotify/signin/import",
        Some(serde_json::json!({"text": creds})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["in_use"]["way"], "librespot-file");
    assert_eq!(r["in_use"]["user"], "alice");
    assert_eq!(r["line"], "librespot, from a credentials file · alice");
    assert!(!r.to_string().contains("auth_data"), "{r}");
    let file = s.data_dir().join("librespot").join("credentials.json");
    assert!(file.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let (st, r) = call(
        &app,
        "POST",
        "/v1/sources/spotify/signin/import",
        Some(serde_json::json!({"text": "{\"username\": \"x\"}"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{r}");
    let (_, r) = call(&app, "DELETE", "/v1/sources/spotify/signin", None).await;
    assert_eq!(r["line"], "Not signed in");
    assert!(!file.exists(), "the copy goes with Sign out");
    // Sign out of a way by name, also while another way is the one picked (librespot while
    // the app is picked, even with librespot itself missing here)
    let (st, _) = call(
        &app,
        "POST",
        "/v1/sources/spotify/signin/import",
        Some(serde_json::json!({"text": creds})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, r) = call(
        &app,
        "PUT",
        "/v1/sources/spotify/signin",
        Some(serde_json::json!({"way": "app"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["chosen"], "app");
    let (st, r) = call(&app, "DELETE", "/v1/sources/spotify/signin?way=nope", None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{r}");
    assert!(file.exists(), "a refused sign-out changes nothing");
    let (st, r) = call(
        &app,
        "DELETE",
        "/v1/sources/spotify/signin?way=librespot-file",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["line"], "Not signed in");
    assert_eq!(r["details"]["librespot"]["signed_in"], false, "{r}");
    assert!(!file.exists(), "the named way's copy is gone");
    *imports::HOME_OVERRIDE.write().unwrap() = None;
}

/// The LAN address the node picks for speakers when public_url is empty is not where the
/// browser is: sign-in callbacks follow the request (localhost, a `.local` name, a proxy). A
/// public_url the owner set is used as it is.
#[tokio::test]
async fn a_guessed_public_url_leaves_sign_in_callbacks_to_the_request() {
    let dir = tempfile::tempdir().unwrap();
    let lan = "http://192.168.1.10:7788";
    for host in ["localhost:7788", "nas.local:7788"] {
        let s = state_at(dir.path(), lan, true).await;
        assert_eq!(
            tidal_redirect(s, host).await,
            format!("http://{host}/tidal/callback")
        );
    }
    let s = state_at(dir.path(), lan, false).await;
    assert_eq!(
        tidal_redirect(s, "localhost:7788").await,
        "http://192.168.1.10:7788/tidal/callback"
    );
}

/// Settings › You › Services: each person sees their own sign-ins.
#[tokio::test]
async fn me_services_are_the_callers_own() {
    let dir = tempfile::tempdir().unwrap();
    let (s, anna, ben) = two_people_node(dir.path(), "http://127.0.0.1:9").await;
    let app = router(s.clone());
    let get = |token: String| {
        let app = app.clone();
        async move {
            let r = app
                .oneshot(
                    Request::builder()
                        .uri("/v1/me/services")
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let st = r.status();
            let b = r.into_body().collect().await.unwrap().to_bytes();
            (
                st,
                serde_json::from_slice::<serde_json::Value>(&b).unwrap_or_default(),
            )
        }
    };
    let tidal_of = |v: &serde_json::Value| {
        v.as_array()
            .unwrap()
            .iter()
            .find(|x| x["source"] == "tidal")
            .unwrap()
            .clone()
    };
    let (st, owner) = get("s3cret".into()).await;
    assert_eq!(st, StatusCode::OK, "{owner}");
    assert_eq!(tidal_of(&owner)["connected"], true);
    let (_, a) = get(accounts::mint(&s, anna).unwrap()).await;
    assert_eq!(tidal_of(&a)["connected"], true, "{a}");
    let (_, b) = get(accounts::mint(&s, ben).unwrap()).await;
    assert_eq!(tidal_of(&b)["connected"], false, "{b}");
    assert_eq!(tidal_of(&b)["setup"], "/v1/sources/tidal/signin");
}

/// A listener's sign-in goes into a backup only sealed under its passphrase, as the owner's.
#[tokio::test]
async fn listeners_sign_ins_are_only_in_a_sealed_backup() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _, _) = two_people_node(dir.path(), "http://127.0.0.1:9").await;
    let has = |hay: &[u8], needle: &str| hay.windows(needle.len()).any(|w| w == needle.as_bytes());
    let (open, _) = backup::make(&s, None).unwrap();
    let open = backup::parse(&open).unwrap();
    assert!(
        !has(&open.database, "anna-token"),
        "a listener's token in a plain backup"
    );
    assert!(
        open.manifest.left_out.iter().any(|w| w.contains("listeners")),
        "{:?}",
        open.manifest.left_out
    );
    let (locked, _) = backup::make(&s, Some("correct horse")).unwrap();
    let locked = backup::parse(&locked).unwrap();
    assert!(!has(&locked.database, "anna-token"));
    // The sealed part holds Anna's own sign-in, under her key
    let sealed = backup::open_sealed(
        locked.manifest.secrets.as_ref().unwrap(),
        locked.secrets.as_ref().unwrap(),
        "correct horse",
    )
    .unwrap();
    let row = sealed
        .settings
        .iter()
        .find(|(k, _)| k.starts_with("person.") && k.ends_with(".tidal.user"))
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    assert!(
        row.contains("anna-token"),
        "{:?}",
        sealed.settings.keys().collect::<Vec<_>>()
    );
}

/// A listener's own TIDAL sign-in in the browser: kept for them, and only when it comes back
/// to the browser that started it (#226 review M2). The owner approving a listener's sign-in
/// link binds nothing; removing a listener drops their sign-ins still on the way (L1).
#[tokio::test]
async fn a_listeners_browser_sign_in_is_kept_for_them_only_from_their_browser() {
    let (base, _) = tidal_auth_and_search_mock("fresh-token").await;
    let dir = tempfile::tempdir().unwrap();
    let (s, anna, ben) = two_people_node(dir.path(), &base).await;
    let app = router(s.clone());
    let (anna_tok, ben_tok) = (
        accounts::mint(&s, anna).unwrap(),
        accounts::mint(&s, ben).unwrap(),
    );
    // Starts a login as `token`: the state and the cookie its browser got
    let start = |token: String| {
        let app = app.clone();
        async move {
            let (st, v, h) = send_as(&app, "GET", "/v1/sources/tidal/login", Some(&token), &[], None).await;
            assert_eq!(st, StatusCode::OK, "{v}");
            let url = url::Url::parse(v["url"].as_str().unwrap()).unwrap();
            let state = url
                .query_pairs()
                .find(|(k, _)| k == "state")
                .unwrap()
                .1
                .to_string();
            let cookie = h
                .get("set-cookie")
                .map(|c| c.to_str().unwrap().split(';').next().unwrap().to_string());
            (state, cookie)
        }
    };
    let callback = |state: String, cookie: Option<String>| {
        let app = app.clone();
        async move {
            let extra: Vec<(&str, &str)> = cookie.iter().map(|c| ("cookie", c.as_str())).collect();
            let (_, _, h) = send_as(
                &app,
                "GET",
                &format!("/tidal/callback?code=c&state={state}"),
                None,
                &extra,
                None,
            )
            .await;
            h.get("location").unwrap().to_str().unwrap().to_string()
        }
    };
    let row = |key: &str| s.library.lock().unwrap().get_setting(key).unwrap();
    // Anna, in her own browser: hers
    let (state, cookie) = start(anna_tok.clone()).await;
    assert!(cookie.is_some(), "a listener's login is bound to their browser");
    assert_eq!(callback(state, cookie).await, "/?tidal=signed_in");
    assert!(row(&format!("person.{anna}.tidal.user"))
        .unwrap()
        .contains("fresh-token"));
    assert!(
        row("tidal.user").unwrap().contains("owner-token"),
        "the owner's untouched"
    );
    // Ben's link approved in someone else's browser (no cookie), or pasted by the owner: refused
    let (state, _) = start(ben_tok.clone()).await;
    assert!(callback(state, None).await.starts_with("/?tidal=error"));
    let (state, _) = start(ben_tok.clone()).await;
    let pasted = serde_json::json!({"url": format!("http://node/tidal/callback?code=c&state={state}")});
    let (st, _, _) = send_as(
        &app,
        "POST",
        "/v1/sources/tidal/callback",
        Some("s3cret"),
        &[],
        Some(pasted),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(row(&format!("person.{ben}.tidal.user")).is_none());
    // Ben removed while his own sign-in is on the way: it never lands
    let (state, cookie) = start(ben_tok.clone()).await;
    let (st, _, _) = send_as(
        &app,
        "DELETE",
        &format!("/v1/people/{ben}"),
        Some("s3cret"),
        &[],
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(callback(state, cookie).await.starts_with("/?tidal=error"));
    assert!(row(&format!("person.{ben}.tidal.user")).is_none());
}

/// A TIDAL device sign-in whose approval arrives after its person signed out, or after a
/// listener was removed, is not kept: no sign-in comes back for them (per-person follow-up,
/// Kimi M1).
#[tokio::test]
async fn a_sign_in_answered_after_sign_out_or_removal_is_not_kept() {
    let asked = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let go = Arc::new(tokio::sync::Notify::new());
    let mock = Router::new()
        .route(
            "/v1/oauth2/device_authorization",
            post(|| async {
                Json(serde_json::json!({"deviceCode": "DC", "userCode": "ABCDE", "verificationUriComplete": "link.tidal.com/ABCDE", "expiresIn": 300, "interval": 1}))
            }),
        )
        .route(
            "/v1/oauth2/token",
            post({
                let (asked, go) = (asked.clone(), go.clone());
                move || {
                    let (asked, go) = (asked.clone(), go.clone());
                    async move {
                        // Approved only once the test has signed out or removed the person
                        asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        go.notified().await;
                        Json(serde_json::json!({"access_token": "AT", "refresh_token": "RT", "expires_in": 3600}))
                    }
                }
            }),
        )
        .route(
            "/v1/sessions",
            get(|| async { Json(serde_json::json!({"userId": 42, "countryCode": "CH", "sessionId": "sid"})) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let (s, anna, _) = two_people_node(dir.path(), &base).await;
    s.tidal.write().unwrap().v1_base = format!("{base}/v1");
    let app = router(s.clone());
    let owner = Some("s3cret");
    let (st, r, _) = send_as(
        &app,
        "PUT",
        "/v1/sources/tidal/unofficial",
        owner,
        &[],
        Some(serde_json::json!({"client_id": "dev-id", "client_secret": "dev-secret"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    // Until TIDAL has been asked `n` times in all (it then waits for `go`)
    let asked_times = |n: u32| {
        let asked = asked.clone();
        async move {
            for _ in 0..200 {
                if asked.load(std::sync::atomic::Ordering::SeqCst) >= n {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            panic!("TIDAL was never asked");
        }
    };
    let login = "/v1/sources/tidal/unofficial/login";

    // The owner signs out while TIDAL is asked
    let (st, r, _) = send_as(&app, "POST", login, owner, &[], None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    asked_times(1).await;
    let (st, _, _) = send_as(&app, "DELETE", login, owner, &[], None).await;
    assert_eq!(st, StatusCode::OK);
    go.notify_one();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let (_, r, _) = send_as(&app, "GET", "/v1/sources/tidal/unofficial", owner, &[], None).await;
    assert_eq!(r["signed_in"], false, "{r}");

    // A listener removed while TIDAL is asked
    let anna_tok = accounts::mint(&s, anna).unwrap();
    let (st, r, _) = send_as(&app, "POST", login, Some(&anna_tok), &[], None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    asked_times(2).await;
    let (st, r, _) = send_as(&app, "DELETE", &format!("/v1/people/{anna}"), owner, &[], None).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    go.notify_one();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let rows = s
        .library
        .lock()
        .unwrap()
        .settings_with_prefix(&format!("person.{anna}."))
        .unwrap();
    assert!(rows.is_empty(), "{rows:?}");
    // Any other write for them lands nowhere either (a token refresh, an OAuth answer)
    let lib = s.library.lock().unwrap();
    let them = accounts::Who::Listener(anna.to_string());
    assert!(them.put(&lib, "tidal.user", Some("{}")).is_err());
    assert!(them.get(&lib, "tidal.user").is_none());
    assert!(
        them.put(&lib, "tidal.user", None).is_ok(),
        "removing stays allowed"
    );
}

/// Restoring a sealed backup restores each listener's sign-in from it, and keeps one made
/// here that the backup doesn't have, as for the owner's (per-person follow-up, Kimi M2).
#[tokio::test]
async fn a_sealed_restore_keeps_listeners_sign_ins_it_does_not_have() {
    let dir = tempfile::tempdir().unwrap();
    let (s, anna, ben) = two_people_node(dir.path(), "http://127.0.0.1:9").await;
    let (locked, _) = backup::make(&s, Some("correct horse")).unwrap();
    {
        let lib = s.library.lock().unwrap();
        lib.set_setting(&format!("person.{anna}.tidal.user"), "anna-later")
            .unwrap();
        lib.set_setting(&format!("person.{ben}.tidal.user"), "ben-here")
            .unwrap();
    }
    backup::restore(&s, locked, Some("correct horse".into()), false)
        .await
        .unwrap();
    let lib = s.library.lock().unwrap();
    let row = |k: &str| lib.get_setting(&format!("person.{k}.tidal.user")).unwrap();
    assert!(row(anna).unwrap().contains("anna-token"), "from the backup");
    assert_eq!(
        row(ben).as_deref(),
        Some("ben-here"),
        "made here, not in the backup"
    );
}

/// First steps runs off the request's task and still counts the owner's own TIDAL sign-in
/// as music to play (per-person follow-up, Claude low 1).
#[tokio::test]
async fn first_steps_count_the_owners_streaming_sign_in() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _, _) = two_people_node(dir.path(), "http://127.0.0.1:9").await;
    let app = router(s.clone());
    let (st, v, _) = send_as(&app, "GET", "/v1/me/first-steps", Some("s3cret"), &[], None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let ids: Vec<&str> = v["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["id"].as_str().unwrap())
        .collect();
    assert!(!ids.contains(&"music"), "{v}");
}

/// A page's next link is followed only on the service's own API origin: an answer that points
/// the next page at another host never gets the account's token sent there (security review
/// 2026-11-02, low). TIDAL and Spotify end to end; SoundCloud's API base is process-wide in
/// tests, so its `get_json` is covered through the same `sources::api_url` it now goes by.
#[tokio::test]
async fn a_next_page_on_another_host_is_not_followed() {
    let stolen: Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let noted = stolen.clone();
    let thief = Router::new().fallback(move |h: HeaderMap| {
        let noted = noted.clone();
        async move {
            let auth = h.get("authorization").and_then(|v| v.to_str().ok());
            noted.lock().unwrap().push(auth.unwrap_or("").to_string());
            Json(serde_json::json!({"data": [], "items": []}))
        }
    });
    let tl = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let next = format!("http://{}/steal?page=2", tl.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(tl, thief).await.unwrap() });
    let (n1, n2) = (next.clone(), next.clone());
    let api = Router::new()
        .route(
            "/playlists",
            get(move || {
                let n = n1.clone();
                async move { Json(serde_json::json!({"data": [], "links": {"next": n}})) }
            }),
        )
        .route(
            "/me/playlists",
            get(move || {
                let n = n2.clone();
                async move { Json(serde_json::json!({"items": [], "next": n})) }
            }),
        );
    let al = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", al.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(al, api).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.tidal.write().unwrap().api_base = base.clone();
    s.spotify.write().unwrap().api_base = base.clone();
    {
        let lib = s.library.lock().unwrap();
        let until = pm_library::db::now_unix() + 3600;
        lib.set_setting(
            "tidal.user",
            &serde_json::json!({"access_token": "tidal-token", "refresh_token": "r", "expires_at": until, "user_id": "4242", "scope": "playback"}).to_string(),
        )
        .unwrap();
        lib.set_setting(
            "spotify.user",
            &serde_json::json!({"access_token": "spotify-token", "expires_at": until}).to_string(),
        )
        .unwrap();
    }
    let app = router(s.clone());
    for path in ["/v1/sources/tidal/playlists", "/v1/sources/spotify/playlists"] {
        let (st, v) = call(&app, "GET", path, None).await;
        assert_eq!(st, StatusCode::BAD_GATEWAY, "{path}: {v}");
        assert!(
            v["error"].as_str().unwrap_or("").contains("another host"),
            "{path}: {v}"
        );
    }
    let seen = stolen.lock().unwrap().clone();
    assert!(seen.is_empty(), "{seen:?}");

    // Each service's own next links still go
    for (base, next) in [
        (
            "https://openapi.tidal.com/v2",
            "https://openapi.tidal.com/v2/playlists?page%5Bcursor%5D=x",
        ),
        (
            "https://api.spotify.com/v1",
            "https://api.spotify.com/v1/me/playlists?offset=50&limit=50",
        ),
        (
            "https://api-v2.soundcloud.com",
            "https://api-v2.soundcloud.com/me/playlists?offset=50",
        ),
    ] {
        assert_eq!(sources::api_url(base, next).as_deref(), Ok(next));
    }
    assert_eq!(
        sources::api_url("https://openapi.tidal.com/v2", "/playlists?x=1").as_deref(),
        Ok("https://openapi.tidal.com/v2/playlists?x=1")
    );
    assert_eq!(
        sources::api_url("https://api-v2.soundcloud.com", "search/tracks").as_deref(),
        Ok("https://api-v2.soundcloud.com/search/tracks")
    );
    // Anywhere else does not: another host, scheme or port, a look-alike, a user-info trick
    for bad in [
        "https://evil.example/me/playlists",
        "http://api-v2.soundcloud.com/me/playlists",
        "https://api-v2.soundcloud.com:8443/me/playlists",
        "https://api-v2.soundcloud.com.evil.example/me/playlists",
        "https://api-v2.soundcloud.com@evil.example/me/playlists",
    ] {
        assert!(
            sources::api_url("https://api-v2.soundcloud.com", bad).is_err(),
            "{bad}"
        );
    }
}

/// The app's sign-in is a device's, not a browser session: a browser's Log out leaves it
/// (the page in the app forgets it, the app keeps it, as with the API token), and so does the
/// browsers' cap; Make a new token and a change of the node's identity end it.
#[tokio::test]
async fn a_devices_sign_in_outlives_browser_log_outs_but_not_a_new_token() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), Some("s3cret")).await;
    let app = router(s.clone());
    const OWNER: &str = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";
    const OTHER: &str = "gujx6qd8ksydh1makdphd3bxu351d9b8waqka8hfg6q7hnqkxexo";
    let hashed = |t: &str| blake3::hash(t.as_bytes()).to_hex().to_string();
    let set = |setting: &str, entries: &[(&str, &str)]| {
        s.library
            .lock()
            .unwrap()
            .set_setting(
                setting,
                &serde_json::to_string(
                    &entries
                        .iter()
                        .map(|(key, t)| serde_json::json!({"hash": hashed(t), "key": key, "created_at": 1}))
                        .collect::<Vec<_>>(),
                )
                .unwrap(),
            )
            .unwrap()
    };
    let keys = |setting: &str| -> Vec<String> {
        s.library
            .lock()
            .unwrap()
            .get_setting(setting)
            .unwrap()
            .map(|j| serde_json::from_str::<Vec<serde_json::Value>>(&j).unwrap())
            .unwrap_or_default()
            .iter()
            .map(|t| t["key"].as_str().unwrap().to_string())
            .collect()
    };
    set("accounts.owner_sessions", &[(OWNER, "pmo_browser")]);
    set(
        "accounts.device_sessions",
        &[(OWNER, "pmo_laptop"), (OTHER, "pmo_phone-of-other")],
    );
    let send = |method: &'static str, token: &'static str| {
        let app = app.clone();
        async move {
            let req = Request::builder()
                .method(method)
                .uri("/v1/me/token")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap();
            let resp = app.oneshot(req).await.unwrap();
            let st = resp.status();
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            (
                st,
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default(),
            )
        }
    };
    // Log out with the device's sign-in ends nothing; with the browser's, that session
    let (st, v) = send("DELETE", "pmo_laptop").await;
    assert_eq!((st, v["ended"].as_bool()), (StatusCode::OK, Some(false)), "{v}");
    assert_eq!(
        keys("accounts.device_sessions"),
        [OWNER, OTHER],
        "the device is still signed in"
    );
    let (st, v) = send("DELETE", "pmo_browser").await;
    assert_eq!((st, v["ended"].as_bool()), (StatusCode::OK, Some(true)), "{v}");
    assert!(keys("accounts.owner_sessions").is_empty());
    // The node takes on an identity: the other identity's device goes, the owner's stays
    accounts::keep_owner_sessions_of(&s, OWNER).unwrap();
    assert_eq!(keys("accounts.device_sessions"), [OWNER]);
    // A new API token ends every device sign-in too
    accounts::end_owner_sessions(&s).unwrap();
    assert!(
        keys("accounts.device_sessions").is_empty(),
        "Make a new token signs the devices out"
    );
}

/// The node admin chooses whose TIDAL listeners play with (owner, 2026-11-03: "can we have both
/// and node admin can set if user use node or own sources"): `own` (as before), `node` (everyone
/// through the node's sign-in; listeners can't sign in to their own) or `choice` (the node's by
/// default, each listener may switch). Every switch keeps both the node's sign-in and each
/// listener's own; a listener's own status never shows the node's account.
#[tokio::test]
async fn the_admin_chooses_node_or_own_sources() {
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
    let anna_key = format!("person.{anna}.tidal.user");
    {
        let lib = s.library.lock().unwrap();
        lib.set_setting("tidal.user", &session("owner-token")).unwrap();
        lib.set_setting(&anna_key, &session("anna-token")).unwrap();
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
            )
        }
    };
    let zid = s.zones.list()[0].id.to_string();
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    // Plays one item queued by `token`; the token TIDAL was asked with, or the error
    let play = |token: String| {
        let send = &send;
        let zone = zone.clone();
        let zid = zid.clone();
        let seen = seen.clone();
        async move {
            let n = seen.lock().unwrap().len();
            let (st, v) = send(
                "POST",
                format!("/v1/zones/{zid}/queue"),
                token,
                Some(serde_json::json!({"items": [{"uri": "tidal:track:20115564"}], "replace": true, "play": true})),
            )
            .await;
            assert_eq!(st, StatusCode::OK, "{v}");
            let end = zone.wait_stopped().await;
            let asked = seen.lock().unwrap().get(n).cloned();
            match (asked, end.error) {
                (Some(a), None) => Ok(a),
                (_, e) => Err(e.unwrap_or_default()),
            }
        }
    };
    let policy = |p: &'static str| {
        let send = &send;
        async move {
            let (st, v) = send(
                "PUT",
                "/v1/node/settings".into(),
                "s3cret".into(),
                Some(serde_json::json!({"sources_policy": p})),
            )
            .await;
            assert_eq!(st, StatusCode::OK, "{v}");
            assert_eq!(v["sources_policy"], p, "{v}");
        }
    };
    let both_kept = |s: &Arc<NodeState>| {
        let lib = s.library.lock().unwrap();
        assert!(lib
            .get_setting("tidal.user")
            .unwrap()
            .unwrap()
            .contains("owner-token"));
        assert!(lib
            .get_setting(&anna_key)
            .unwrap()
            .unwrap()
            .contains("anna-token"));
    };

    // `own`, the default for a node that never chose: as before
    assert_eq!(play(anna_tok.clone()).await, Ok("Bearer anna-token".into()));
    assert!(play(ben_tok.clone()).await.unwrap_err().contains("Connect TIDAL"));
    let (st, v) = send("GET", "/v1/node/settings".into(), "s3cret".into(), None).await;
    assert_eq!(
        (st, v["sources_policy"].as_str()),
        (StatusCode::OK, Some("own")),
        "{v}"
    );
    // Only the owner chooses
    let (st, _) = send(
        "PUT",
        "/v1/node/settings".into(),
        anna_tok.clone(),
        Some(serde_json::json!({"sources_policy": "node"})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _) = send(
        "PUT",
        "/v1/me/sources".into(),
        anna_tok.clone(),
        Some(serde_json::json!({"use": "node"})),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::CONFLICT,
        "a listener's choice only under `choice`"
    );

    // `node`: everyone through the node's sign-in, the stream proxy too
    policy("node").await;
    assert_eq!(play(anna_tok.clone()).await, Ok("Bearer owner-token".into()));
    assert_eq!(play(ben_tok.clone()).await, Ok("Bearer owner-token".into()));
    let (st, _) = send(
        "GET",
        "/v1/sources/tidal/stream/20115564".into(),
        ben_tok.clone(),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(seen.lock().unwrap().last().unwrap(), "Bearer owner-token");
    // A listener can't sign in to their own; what they had stays, and their own status shows
    // their own (Ben: none), never the node's account
    let (st, v) = send(
        "PUT",
        "/v1/sources/tidal/signin".into(),
        ben_tok.clone(),
        Some(serde_json::json!({"way": "official"})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    assert!(v["error"].as_str().unwrap().contains("its own sign-ins"), "{v}");
    let (st, v) = send("GET", "/v1/sources/tidal/status".into(), ben_tok.clone(), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["signed_in"], false, "Ben's own status, not the node's: {v}");
    let (_, v) = send("GET", "/v1/me/sources".into(), ben_tok.clone(), None).await;
    assert_eq!(
        (
            v["policy"].as_str(),
            v["plays_with"].as_str(),
            v["may_choose"].as_bool()
        ),
        (Some("node"), Some("node"), Some(false)),
        "{v}"
    );
    both_kept(&s);

    // `choice`: the node's by default; Anna switches to her own, Ben stays on the node's, in
    // the same zone, each item with its own account
    policy("choice").await;
    assert_eq!(play(anna_tok.clone()).await, Ok("Bearer owner-token".into()));
    let (st, v) = send(
        "PUT",
        "/v1/me/sources".into(),
        anna_tok.clone(),
        Some(serde_json::json!({"use": "own"})),
    )
    .await;
    assert_eq!(
        (st, v["plays_with"].as_str()),
        (StatusCode::OK, Some("own")),
        "{v}"
    );
    assert_eq!(play(anna_tok.clone()).await, Ok("Bearer anna-token".into()));
    assert_eq!(play(ben_tok.clone()).await, Ok("Bearer owner-token".into()));
    // Listeners may sign in again under `choice`, and what they pick lands at their own keys,
    // never the node's
    let (st, si) = send("GET", "/v1/sources/tidal/signin".into(), ben_tok.clone(), None).await;
    assert_eq!(st, StatusCode::OK, "{si}");
    let way = si["ways"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["available"] == true)
        .and_then(|w| w["id"].as_str())
        .expect("a way open to listeners")
        .to_string();
    let node_way = s.library.lock().unwrap().get_setting("tidal.signin_way").unwrap();
    let (st, v) = send(
        "PUT",
        "/v1/sources/tidal/signin".into(),
        ben_tok.clone(),
        Some(serde_json::json!({"way": way})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    {
        let lib = s.library.lock().unwrap();
        assert_eq!(
            lib.get_setting(&format!("person.{ben}.tidal.signin_way"))
                .unwrap()
                .as_deref(),
            Some(way.as_str())
        );
        assert_eq!(
            lib.get_setting("tidal.signin_way").unwrap(),
            node_way,
            "the node's way untouched"
        );
    }
    // Back to the node's
    let (st, _) = send(
        "PUT",
        "/v1/me/sources".into(),
        anna_tok.clone(),
        Some(serde_json::json!({"use": "node"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(play(anna_tok.clone()).await, Ok("Bearer owner-token".into()));
    // Anna's choice is kept when the policy moves away and back
    let (_, _) = send(
        "PUT",
        "/v1/me/sources".into(),
        anna_tok.clone(),
        Some(serde_json::json!({"use": "own"})),
    )
    .await;
    policy("own").await;
    assert_eq!(play(anna_tok.clone()).await, Ok("Bearer anna-token".into()));
    policy("choice").await;
    let (_, v) = send("GET", "/v1/me/sources".into(), anna_tok.clone(), None).await;
    assert_eq!(v["use"], "own", "{v}");
    both_kept(&s);
    // The owner always plays with the node's
    assert_eq!(play("s3cret".into()).await, Ok("Bearer owner-token".into()));
}

/// Under the node's source policy, a listener's catalogue reads resolve to the node's sign-ins,
/// while what is theirs alone (their own sign-in keys when they sign in, their preferences)
/// stays at their own keys.
#[test]
fn a_listener_on_the_nodes_sign_ins_keeps_their_own_settings() {
    let on = accounts::Who::ListenerOnNode("k".into());
    assert_eq!(on.setting("tidal.user").as_deref(), Some("tidal.user"));
    assert_eq!(
        on.setting("qobuz.unofficial").as_deref(),
        Some("qobuz.unofficial")
    );
    assert_eq!(
        on.setting("spotify.signin_way").as_deref(),
        Some("spotify.signin_way")
    );
    assert_eq!(on.setting("sources.use").as_deref(), Some("person.k.sources.use"));
    assert_eq!(on.setting("me.prefs").as_deref(), Some("person.k.me.prefs"));
    assert_eq!(on.by().as_deref(), Some("k"), "who queued it stays the listener");
    assert!(!on.is_owner());
    assert_eq!(on.own(), accounts::Who::Listener("k".into()));
    // Caches keyed by the tag never share an entry between the node's account and their own
    assert_ne!(on.tag(), on.own().tag());
    assert_ne!(on.tag(), accounts::Who::Owner.tag());
    // The keys the services really use
    for k in [
        tidal_unofficial::STATE_KEY,
        qobuz::STATE_KEY,
        "tidal.user",
        "spotify.user",
    ] {
        assert!(accounts::SIGNIN_KEYS.contains(&k), "{k}");
    }
    for src in ["tidal", "qobuz", "spotify"] {
        assert!(
            accounts::SIGNIN_KEYS.contains(&signin::way_key(src).as_str()),
            "{src}"
        );
    }
}

/// The project's return page forwards only to a node on a local address: a sign-in through
/// it from any other address answers 412 (and the apps listing says so), a local address
/// signs in through it, and a node's own `[auth] return_page` is never second-guessed.
#[tokio::test]
async fn the_shared_return_page_needs_a_local_address() {
    use base64::Engine;
    let dir = tempfile::tempdir().unwrap();
    // A node reached by another name than localhost has a token
    let s = state(dir.path(), Some("fixture-api")).await;
    s.tidal.write().unwrap().creds = tidal::TidalCredentials {
        client_id: "project-app".into(),
        ..Default::default()
    };
    s.spotify.write().unwrap().creds = spotify::SpotifyCredentials {
        client_id: "own-app".into(),
        client_secret: String::new(),
    };
    let no_secret = |id: &str| drives::App {
        client_id: id.into(),
        client_secret: String::new(),
    };
    s.set_drive_apps(drives::apps_from(no_secret("g-client"), no_secret("m-client")));
    let app = router(s.clone());
    let send = |method: &'static str, path: &'static str, host: &'static str| {
        let app = app.clone();
        async move {
            let req = Request::builder()
                .method(method)
                .uri(path)
                .header("host", host)
                .header("authorization", "Bearer fixture-api")
                .body(Body::empty())
                .unwrap();
            let resp = app.oneshot(req).await.unwrap();
            let st = resp.status();
            let b = resp.into_body().collect().await.unwrap().to_bytes();
            (
                st,
                serde_json::from_slice::<serde_json::Value>(&b).unwrap_or_default(),
            )
        }
    };
    let logins = [
        (
            "GET",
            "/v1/sources/tidal/login",
            "/tidal/callback",
            "Use the code sign-in",
        ),
        (
            "GET",
            "/v1/sources/spotify/login",
            "/spotify/callback",
            "Use your own app with its secret in node.toml",
        ),
        (
            "POST",
            "/v1/library/drives/google/login",
            "/drives/google/callback",
            "Use your own app",
        ),
        (
            "POST",
            "/v1/library/drives/onedrive/login",
            "/drives/onedrive/callback",
            "Use your own OneDrive app with a return page of your own",
        ),
    ];
    let notes = |v: &serde_json::Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .filter(|a| {
                a["note"]
                    .as_str()
                    .is_some_and(|n| n.contains("returns only to local"))
            })
            .map(|a| a["id"].as_str().unwrap().to_string())
            .collect()
    };
    // An internet address: refused before any sign-in starts, with what to use instead
    for (m, path, _, instead) in logins {
        let (st, v) = send(m, path, "music.example.com").await;
        assert_eq!(st, StatusCode::PRECONDITION_FAILED, "{path}: {v}");
        let msg = v["error"]
            .as_str()
            .or(v["message"].as_str())
            .unwrap_or_default()
            .to_string();
        assert!(
            msg.contains("isn't a local one") && msg.contains(instead),
            "{path}: {v}"
        );
    }
    let (_, apps) = send("GET", "/v1/node/apps", "music.example.com").await;
    assert_eq!(
        notes(&apps),
        ["spotify", "tidal", "google-drive", "onedrive"],
        "{apps}"
    );
    // A local address: through the return page, which forwards to this node's callback
    for (m, path, callback, _) in logins {
        let (st, v) = send(m, path, "192.168.1.5:7788").await;
        assert_eq!(st, StatusCode::OK, "{path}: {v}");
        let u = url::Url::parse(v["url"].as_str().unwrap()).unwrap();
        let q: std::collections::HashMap<String, String> = u.query_pairs().into_owned().collect();
        assert_eq!(q["redirect_uri"], crate::project_apps::RETURN_PAGE, "{path}");
        let (_, enc) = q["state"].split_once('.').unwrap();
        let back = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(enc)
            .unwrap();
        assert_eq!(
            String::from_utf8(back).unwrap(),
            format!("http://192.168.1.5:7788{callback}")
        );
    }
    let (_, apps) = send("GET", "/v1/node/apps", "192.168.1.5:7788").await;
    assert!(notes(&apps).is_empty(), "{apps}");
    // The node's own return page: its owner's redirector, used from any address
    s.set_return_page("https://return.example/callback");
    for (m, path, _, _) in logins {
        let (st, v) = send(m, path, "music.example.com").await;
        assert_eq!(st, StatusCode::OK, "{path}: {v}");
    }
    let (_, apps) = send("GET", "/v1/node/apps", "music.example.com").await;
    assert!(notes(&apps).is_empty(), "{apps}");
}
