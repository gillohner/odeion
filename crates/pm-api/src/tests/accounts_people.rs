//! People, tokens and who may do what: the owner, listeners, the controller sign-in.
//!
//! Part of pm-api's unit tests (`mod tests` in lib.rs, which holds the helpers these use). A new
//! test of this area goes at the end of this file.

use super::*;

/// A token changed in the config (`PM_TOKEN`, `[api] token`) ends the owner's browser sessions
/// at the next start, as Make a new token does (Hermes review of #209, low 4).
#[tokio::test]
async fn a_token_changed_in_the_config_ends_the_owners_browser_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), Some("s3cret")).await;
    let stored = || {
        s.library
            .lock()
            .unwrap()
            .get_setting("accounts.owner_sessions")
            .unwrap()
            .unwrap_or_default()
    };
    let one =
        r#"[{"hash":"00","key":"o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy","created_at":1}]"#;
    s.library
        .lock()
        .unwrap()
        .set_setting("accounts.owner_sessions", one)
        .unwrap();
    // Sessions from before the node noted a token: kept, and the token noted
    assert!(!accounts::end_sessions_of_an_old_token(&s).unwrap());
    assert_eq!(stored(), one);
    // The same token at the next start: kept
    assert!(!accounts::end_sessions_of_an_old_token(&s).unwrap());
    assert_eq!(stored(), one);
    // Another token: they end
    *s.token.write().unwrap() = Some("changed-in-the-config".into());
    assert!(accounts::end_sessions_of_an_old_token(&s).unwrap());
    assert_eq!(stored(), "[]");
    assert!(!accounts::end_sessions_of_an_old_token(&s).unwrap());
}

#[tokio::test]
async fn the_log_in_screen_gets_names_and_small_pictures_inline() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), Some("s3cret")).await;
    // Keys no other test uses: this test puts names for them into the profile cache, which
    // is one for the whole test process, and a test that asks Nexus (a mock) for the same
    // key would get these names instead of its mock's
    const ALICE: &str = "a11ce1og1nscreentestkey1111111111111111111111111111a";
    const BOB: &str = "b0b1og1nscreentestkey22222222222222222222222222222b0";
    let view = |id: &str, name: &str| {
        serde_json::from_value::<pm_pubky::nexus::UserView>(serde_json::json!({
            "details": {"id": id, "name": name, "image": format!("pubky://{id}/pub/pubky.app/files/1")}
        }))
        .unwrap()
    };
    profiles::cache_view(ALICE, Some(view(ALICE, "Alice")));
    profiles::cache_view(BOB, Some(view(BOB, "Bob")));
    let people = s.art_dir.join("people");
    std::fs::create_dir_all(&people).unwrap();
    let png = |n: usize| {
        [
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A][..],
            &vec![0u8; n],
        ]
        .concat()
    };
    std::fs::write(people.join(ALICE), png(100)).unwrap();
    std::fs::write(people.join(BOB), png(300 * 1024)).unwrap();
    // Someone who approved: name and a small picture inline (the screen has no token)
    let a = profiles::login_face(&s, &format!("pubky{ALICE}"), profiles::FaceFrom::Nexus).await;
    assert_eq!((a.id.as_str(), a.name.as_str()), (ALICE, "Alice"));
    assert!(
        a.avatar
            .as_deref()
            .is_some_and(|u| u.starts_with("data:image/png;base64,")),
        "{a:?}"
    );
    // A big picture stays off the screen (initials instead)
    let b = profiles::login_face(&s, BOB, profiles::FaceFrom::Nexus).await;
    assert_eq!((b.name.as_str(), b.avatar), ("Bob", None));
    // The owner's picture by its own address, not inline
    let o = profiles::login_face(&s, ALICE, profiles::FaceFrom::Owner).await;
    assert_eq!(o.avatar.as_deref(), Some(profiles::OWNER_AVATAR));
    // Not a key: its short form, nothing looked up
    let x = profiles::login_face(&s, "nonsense", profiles::FaceFrom::Nexus).await;
    assert_eq!(x.avatar, None);
}

/// A log in asks for the node's folders only when the owner's page asks for a renewal and the
/// node lacks them: a listener scanning the QR code is never asked to give the node write
/// access to their own homeserver (Hermes review of #209, medium 1).
#[test]
fn only_the_owners_renewal_asks_for_the_nodes_folders() {
    use pm_pubky::homeserver::{CONTROLLER_CAPABILITIES, LOGIN_CAPABILITIES};
    assert_eq!(
        login_capabilities(false, true),
        (false, CONTROLLER_CAPABILITIES),
        "the node needs folders, but nobody asked: a plain log in"
    );
    assert_eq!(
        login_capabilities(true, false),
        (false, CONTROLLER_CAPABILITIES),
        "asked, but nothing to renew"
    );
    assert_eq!(login_capabilities(false, false), (false, CONTROLLER_CAPABILITIES));
    assert_eq!(login_capabilities(true, true), (true, LOGIN_CAPABILITIES));
    assert!(!CONTROLLER_CAPABILITIES.contains(":rw"), "a log in only reads");
}

/// A renewal needs a code only the owner's page can get, good until it renews (Kimi review of
/// #209 at 6bf5f7d): a made-up one, or `{renew: true}` as before, asks for nothing more.
#[tokio::test]
async fn a_renewal_code_comes_from_the_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    // The node needs no new folders (no Pubky session here): no code
    let (st, v) = call(&app, "POST", "/v1/auth/pubky/renewal", None).await;
    assert_eq!(
        (st, &v["renewal"]),
        (StatusCode::OK, &serde_json::Value::Null),
        "{v}"
    );
    // Not without the owner's token
    *s.token.write().unwrap() = Some("s3cret".into());
    let (st, _) = call(&app, "POST", "/v1/auth/pubky/renewal", None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    // Nor with a listener's
    let key = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";
    s.library.lock().unwrap().set_setting("people", &serde_json::json!({"people": [{"key": key, "access": "music", "music_enabled": true, "added_at": 1}]}).to_string()).unwrap();
    let listener = accounts::mint(&s, key).unwrap();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/pubky/renewal")
                .header("authorization", format!("Bearer {listener}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    // A code the node gave out stays good (a wrong identity, Try again, an expired QR code)
    // until a renewal succeeds, which uses the codes up (tests/bundled_homeserver.rs); a
    // made-up one never is
    s.renewals
        .lock()
        .unwrap()
        .insert("given".into(), std::time::Instant::now());
    assert!(!renewal_good(&s, "made-up"));
    assert!(renewal_good(&s, "given"));
    assert!(renewal_good(&s, "given"), "still, for Try again");
    // An old one has run out
    s.renewals.lock().unwrap().insert(
        "old".into(),
        std::time::Instant::now() - RENEWAL_TTL - std::time::Duration::from_secs(1),
    );
    assert!(!renewal_good(&s, "old"));
    // The old flag means nothing now
    assert_eq!(
        serde_json::from_str::<ControllerLoginStart>(r#"{"renew": true}"#)
            .unwrap()
            .renewal,
        None
    );
}

/// The controller log in's warm-up (`POST /v1/auth/pubky/start` needs no token): many asking
/// at once start one run, none runs again within the interval, a failed run is tried again by
/// the next caller, and once the interval has passed it runs again.
#[tokio::test]
async fn the_controller_warm_up_runs_now_and_then() {
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    let last = tokio::sync::Mutex::new(None);
    let every = std::time::Duration::from_secs(30);
    let count = AtomicUsize::new(0);
    let runs = &count;
    let work = move || async move {
        runs.fetch_add(1, SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        Ok::<(), String>(())
    };
    let all = futures_util::future::join_all((0..10).map(|_| now_and_then(&last, every, work))).await;
    assert!(all.iter().all(Result::is_ok));
    assert_eq!(runs.load(SeqCst), 1, "ten at once: one run");
    now_and_then(&last, every, work).await.unwrap();
    assert_eq!(runs.load(SeqCst), 1, "within the interval: none");
    *last.lock().await = None;
    let failed = now_and_then(&last, every, || async { Err::<(), _>("refused".to_string()) });
    assert_eq!(failed.await, Err("refused".to_string()));
    now_and_then(&last, every, work).await.unwrap();
    assert_eq!(runs.load(SeqCst), 2, "a failure is tried again");
    *last.lock().await = std::time::Instant::now().checked_sub(every);
    now_and_then(&last, every, work).await.unwrap();
    assert_eq!(runs.load(SeqCst), 3, "the interval passed: it runs again");
}

#[tokio::test]
async fn bearer_token_guards_every_route() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), Some("s3cret")).await;
    let app = router(s);
    let (st, _) = call(&app, "GET", "/v1/info", None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let req = Request::builder()
        .uri("/v1/info")
        .header("authorization", "Bearer s3cret")
        .body(Body::empty())
        .unwrap();
    assert_eq!(app.clone().oneshot(req).await.unwrap().status(), StatusCode::OK);
    // `?token=` opens only what a browser loads by address: the event stream, a song's file
    // or download, a cover, a picture, an avatar, a service stream (a token in a shared link
    // opens nothing else)
    let (st, _) = call(&app, "GET", "/v1/info?token=s3cret", None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _) = call(&app, "GET", "/v1/zones?token=s3cret", None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _) = call(&app, "GET", "/v1/library/albums?token=s3cret", None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _) = call(
        &app,
        "POST",
        "/v1/zones/x/queue?token=s3cret",
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED, "no writes by address");
    for p in [
        "/v1/library/artists/1/image",
        "/v1/sources/jellyfin/cover/x",
        "/v1/sources/plex/cover/1/2",
        "/v1/profiles/x/avatar",
    ] {
        let (st, _) = call(&app, "GET", &format!("{p}?token=s3cret"), None).await;
        assert_ne!(st, StatusCode::UNAUTHORIZED, "{p}: past the token check");
    }
    // Decided on the route, not the path: a parameter named like a media route opens nothing
    for p in [
        "/v1/item/song/file",
        "/v1/profiles/avatar",
        "/v1/discover/tag/stream",
        "/v1/library/tracks/cover",
    ] {
        let (st, _) = call(&app, "GET", &format!("{p}?token=s3cret"), None).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED, "{p}");
    }
    let (st, _) = call(&app, "GET", "/v1/library/albums/1/cover?token=s3cret", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "past the token check");
    let (st, _) = call(&app, "GET", "/v1/library/tracks/1/file?token=s3cret", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "past the token check");
    let (st, _) = call(&app, "GET", "/v1/library/tracks/1/download?token=s3cret", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "past the token check");
    let (st, _) = call(&app, "GET", "/v1/sources/tidal/stream/1?token=s3cret", None).await;
    assert_ne!(st, StatusCode::UNAUTHORIZED, "past the token check");
    let (st, _) = call(&app, "GET", "/v1/events?token=s3cret", None).await;
    assert_ne!(st, StatusCode::UNAUTHORIZED, "past the token check");
}

#[tokio::test]
async fn personal_play_order_http_writes_never_change_another_person() {
    let dir = tempfile::tempdir().unwrap();
    let (s, anna, ben) = two_people_node(dir.path(), "http://127.0.0.1:9").await;
    let app = router(s.clone());
    let a = accounts::mint(&s, anna).unwrap();
    let b = accounts::mint(&s, ben).unwrap();
    s.library
        .lock()
        .unwrap()
        .set_setting("play.provider_order", r#"["spotify"]"#)
        .unwrap();
    let send = |method: &str, uri: &str, token: &str, body: serde_json::Value| {
        let app = app.clone();
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        async move {
            let r = app.oneshot(req).await.unwrap();
            let st = r.status();
            let bytes = r.into_body().collect().await.unwrap().to_bytes();
            (st, serde_json::from_slice::<serde_json::Value>(&bytes).unwrap())
        }
    };
    let (st, v) = send(
        "PUT",
        "/v1/play-order",
        &a,
        serde_json::json!({"order":["tidal"]}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["order"][0], "tidal");
    let (st, v) = send(
        "PUT",
        "/v1/play-order",
        &b,
        serde_json::json!({"order":["jamendo"]}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["order"][0], "jamendo");
    let (_, v) = send("GET", "/v1/play-order", &a, serde_json::json!(null)).await;
    assert_eq!(v["order"][0], "tidal");
    let (_, v) = send("GET", "/v1/play-order", "s3cret", serde_json::json!(null)).await;
    assert_eq!(v["order"][0], "spotify", "only owner retains legacy default");
    let (_, v) = send("GET", "/v1/me/prefs", &a, serde_json::json!(null)).await;
    assert_eq!(v["play_order"]["value"][0], "tidal");
    let (st, _) = send(
        "PUT",
        "/v1/me/prefs",
        &a,
        serde_json::json!({"play_order":["unknown"]}),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (_, v) = send("GET", "/v1/play-order", &a, serde_json::json!(null)).await;
    assert_eq!(v["order"][0], "tidal", "invalid update is non-destructive");
    let (st, _) = send("PUT", "/v1/me/prefs", &a, serde_json::json!({"play_order":null})).await;
    assert_eq!(st, StatusCode::OK);
    let (_, v) = send("GET", "/v1/play-order", &a, serde_json::json!(null)).await;
    assert_eq!(
        v["order"][0], "library",
        "listener reset never borrows legacy owner order"
    );
    let (_, v) = send("GET", "/v1/play-order", &b, serde_json::json!(null)).await;
    assert_eq!(v["order"][0], "jamendo");
    assert_eq!(
        s.library
            .lock()
            .unwrap()
            .get_setting("play.provider_order")
            .unwrap()
            .unwrap(),
        r#"["spotify"]"#
    );
}

/// Removing a listener (People) deletes what the node keeps for them: their own sign-ins,
/// their tokens and their preferences; the owner's and other people's stay.
#[tokio::test]
async fn removing_a_listener_deletes_their_sign_ins() {
    let dir = tempfile::tempdir().unwrap();
    let (s, anna, ben) = two_people_node(dir.path(), "http://127.0.0.1:9").await;
    let tok = accounts::mint(&s, anna).unwrap();
    s.library
        .lock()
        .unwrap()
        .set_setting(&format!("me.prefs.{anna}"), "{}")
        .unwrap();
    let app = router(s.clone());
    let send = |method: &'static str, uri: String, token: String| {
        let app = app.clone();
        async move {
            app.oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
        }
    };
    assert_eq!(send("GET", "/v1/me".into(), tok.clone()).await, StatusCode::OK);
    assert_eq!(
        send("DELETE", format!("/v1/people/{anna}"), "s3cret".into()).await,
        StatusCode::OK
    );
    assert_eq!(send("GET", "/v1/me".into(), tok).await, StatusCode::UNAUTHORIZED);
    let lib = s.library.lock().unwrap();
    assert!(lib
        .settings_with_prefix(&format!("person.{anna}."))
        .unwrap()
        .is_empty());
    assert!(lib.get_setting(&format!("me.prefs.{anna}")).unwrap().is_none());
    assert!(
        lib.get_setting("tidal.user").unwrap().is_some(),
        "the owner's stays"
    );
    assert!(!lib.get_setting("people").unwrap().unwrap().contains(anna));
    assert!(lib.get_setting("people").unwrap().unwrap().contains(ben));
}

/// A listener's search runs in a task of its own per source: it still uses their own TIDAL
/// sign-in (or the app's), never the owner's (#226 review H1).
#[tokio::test]
async fn a_listeners_search_uses_their_own_sign_in() {
    let (base, seen) = tidal_auth_and_search_mock("app-token").await;
    let dir = tempfile::tempdir().unwrap();
    let (s, anna, ben) = two_people_node(dir.path(), &base).await;
    let app = router(s.clone());
    let (anna_tok, ben_tok) = (
        accounts::mint(&s, anna).unwrap(),
        accounts::mint(&s, ben).unwrap(),
    );
    let (st, v, _) = send_as(
        &app,
        "GET",
        "/v1/search?q=lucky&scope=tidal",
        Some(&anna_tok),
        &[],
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let mine: Vec<String> = seen.lock().unwrap().drain(..).collect();
    assert!(!mine.is_empty(), "TIDAL was searched");
    assert!(mine.iter().all(|b| b == "Bearer anna-token"), "{mine:?}");
    let (st, _, _) = send_as(
        &app,
        "GET",
        "/v1/search?q=other&scope=tidal",
        Some(&ben_tok),
        &[],
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let bens: Vec<String> = seen.lock().unwrap().drain(..).collect();
    assert!(
        bens.iter()
            .all(|b| b != "Bearer owner-token" && b != "Bearer anna-token"),
        "Ben searches with the app's token: {bens:?}"
    );
}

/// Removing a listener also drops what was fetched with their sign-ins: TIDAL tracks kept for
/// their browser, their search answers and item pages; the owner's stay (per-person
/// follow-up, Kimi M3).
#[tokio::test]
async fn removing_a_listener_drops_what_was_fetched_for_them() {
    let dir = tempfile::tempdir().unwrap();
    let (s, anna, _) = two_people_node(dir.path(), "http://127.0.0.1:9").await;
    let app = router(s.clone());
    let at = std::time::Instant::now();
    tidal::keep_stream(
        &mut tidal::stream_cache().lock().unwrap(),
        (format!("{anna}/9990001"), at, vec![1].into(), "audio/mp4".into()),
    );
    {
        let mut a = s.search_answers.lock().unwrap();
        a.insert(
            ("lucky".into(), format!("tidal|{anna}")),
            (at, Default::default()),
        );
        a.insert(("lucky".into(), "tidal".into()), (at, Default::default()));
        let lib = s.library.lock().unwrap();
        lib.set_setting(&format!("item.contents.{anna}.tidal:album:1"), "[1,{}]")
            .unwrap();
        lib.set_setting("item.contents.tidal:album:1", "[1,{}]").unwrap();
    }
    let (st, r, _) = send_as(
        &app,
        "DELETE",
        &format!("/v1/people/{anna}"),
        Some("s3cret"),
        &[],
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert!(!tidal::stream_cache()
        .lock()
        .unwrap()
        .iter()
        .any(|(k, _, _, _)| *k == format!("{anna}/9990001")));
    let keys: Vec<String> = s
        .search_answers
        .lock()
        .unwrap()
        .keys()
        .map(|k| k.1.clone())
        .collect();
    assert_eq!(keys, ["tidal"], "{keys:?}");
    let lib = s.library.lock().unwrap();
    assert!(lib
        .settings_with_prefix(&format!("item.contents.{anna}."))
        .unwrap()
        .is_empty());
    assert!(lib.get_setting("item.contents.tidal:album:1").unwrap().is_some());
}

/// Health is the node's, as the owner's sign-ins see it: a listener asking first doesn't
/// leave their view for the owner (per-person follow-up, Claude low 2).
#[tokio::test]
async fn health_is_the_owners_view_whoever_asks_first() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _, ben) = two_people_node(dir.path(), "http://127.0.0.1:9").await;
    let app = router(s.clone());
    let ben_tok = accounts::mint(&s, ben).unwrap();
    let (st, first, _) = send_as(&app, "GET", "/v1/health", Some(&ben_tok), &[], None).await;
    assert_eq!(st, StatusCode::OK, "{first}");
    let (_, v, _) = send_as(&app, "GET", "/v1/health", Some("s3cret"), &[], None).await;
    assert!(
        v.as_array().unwrap().iter().any(|p| p["id"] == "tidal"),
        "the owner's TIDAL is checked: {v}"
    );
    // What the owner signed in to, their homeserver and the token note are the owner's to
    // see: a listener gets the parts that change what they can do here, before and after
    let (st, again, _) = send_as(&app, "GET", "/v1/health", Some(&ben_tok), &[], None).await;
    assert_eq!(st, StatusCode::OK, "{again}");
    for seen in [&first, &again] {
        assert!(
            seen.as_array().unwrap().iter().all(|p| p["id"] == "pinakes"),
            "a listener's health: {seen}"
        );
    }
}

/// Home sections are cached per person: what plays in full on a card depends on their own
/// sign-ins (per-person follow-up, Claude low 4).
#[tokio::test]
async fn home_sections_are_cached_per_person() {
    let q = home::HomeQuery::default();
    let key =
        |who: accounts::Who| accounts::ASKING.sync_scope(who, || home::section_key("trending-songs", &q));
    let anna = "tbepfp8fisyiqpskbmxsmpub47gxb7xdsh5x9ut9qrb97gcq9phy";
    assert_ne!(
        key(accounts::Who::Owner),
        key(accounts::Who::Listener(anna.into()))
    );
    assert_ne!(
        key(accounts::Who::Nobody),
        key(accounts::Who::Listener(anna.into()))
    );
}

/// A listener's item page fetches the service's list in a task of its own: still with their
/// own TIDAL sign-in, never the owner's (#226 review, the missing test).
#[tokio::test]
async fn a_listeners_item_page_uses_their_own_sign_in() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen2 = seen.clone();
    let mock = Router::new()
        .route(
            "/v1/oauth2/token",
            post(|| async {
                Json(serde_json::json!({"access_token": "app-token", "refresh_token": "r", "expires_in": 3600}))
            }),
        )
        .fallback(move |headers: HeaderMap| {
            let seen = seen2.clone();
            async move {
                if let Some(b) = headers.get("authorization") {
                    seen.lock().unwrap().push(b.to_str().unwrap().to_string());
                }
                Json(serde_json::json!({"data": [], "included": []}))
            }
        });
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let (s, anna, _) = two_people_node(dir.path(), &base).await;
    let app = router(s.clone());
    let anna_tok = accounts::mint(&s, anna).unwrap();
    let (st, v, _) = send_as(
        &app,
        "GET",
        "/v1/discover/contents?uri=https://tidal.com/album/4242",
        Some(&anna_tok),
        &[],
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let asked: Vec<String> = seen.lock().unwrap().clone();
    // The page and the list fetched for it alike: never the owner's, nor the app's
    assert!(!asked.is_empty());
    assert!(asked.iter().all(|b| b == "Bearer anna-token"), "{asked:?}");
}

/// A node without a token answers only requests addressed to it by a loopback name from its
/// own pages, the event stream included. A node with a token answers by any name, behind a
/// proxy too. No other site frames the pages.
#[tokio::test]
async fn a_tokenless_node_refuses_other_sites_and_pages_cant_be_framed() {
    let send = |app: Router, uri: &'static str, headers: Vec<(&'static str, &'static str)>| async move {
        let mut b = Request::builder().method("GET").uri(uri);
        for (k, v) in headers {
            b = b.header(k, v);
        }
        let r = app.oneshot(b.body(Body::empty()).unwrap()).await.unwrap();
        (r.status(), r.headers().clone())
    };
    let dir = tempfile::tempdir().unwrap();
    let open = router(state(dir.path(), None).await);
    // Its own name, with or without its own Origin
    for h in [
        vec![("host", "127.0.0.1:7788")],
        vec![("host", "localhost:7788"), ("origin", "http://localhost:7788")],
    ] {
        let (st, _) = send(open.clone(), "/v1/info", h.clone()).await;
        assert_eq!(st, StatusCode::OK, "{h:?}");
    }
    // A rebound name, another site's page, its event stream
    let (st, _) = send(open.clone(), "/v1/info", vec![("host", "other.example:7788")]).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _) = send(open.clone(), "/", vec![("host", "other.example:7788")]).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "the page too");
    let (st, _) = send(
        open.clone(),
        "/v1/info",
        vec![("host", "127.0.0.1:7788"), ("origin", "http://other.example")],
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let ws = |origin: &'static str| {
        vec![
            ("host", "127.0.0.1:7788"),
            ("origin", origin),
            ("connection", "upgrade"),
            ("upgrade", "websocket"),
            ("sec-websocket-version", "13"),
            ("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ=="),
        ]
    };
    let (st, _) = send(open.clone(), "/v1/events", ws("http://other.example")).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "cross-site event stream");

    // Its own pages open the event stream (past the Origin check: 426 here, as a test request
    // can't be upgraded)
    let (st, _) = send(open.clone(), "/v1/events", ws("http://127.0.0.1:7788")).await;
    assert_ne!(st, StatusCode::FORBIDDEN);
    // Another site's GET without an Origin (an image, a link) is refused, a navigation to a
    // page isn't
    let (st, _) = send(
        open.clone(),
        "/rest/ping",
        vec![
            ("host", "127.0.0.1:7788"),
            ("sec-fetch-site", "cross-site"),
            ("sec-fetch-mode", "no-cors"),
        ],
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _) = send(
        open.clone(),
        "/",
        vec![
            ("host", "127.0.0.1:7788"),
            ("sec-fetch-site", "cross-site"),
            ("sec-fetch-mode", "navigate"),
            ("sec-fetch-dest", "document"),
        ],
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    // In a frame on another site's page: refused before any page runs (a sign-in callback
    // would have acted before frame-ancestors kept it from showing)
    let (st, _) = send(
        open.clone(),
        "/",
        vec![
            ("host", "127.0.0.1:7788"),
            ("sec-fetch-site", "cross-site"),
            ("sec-fetch-mode", "navigate"),
            ("sec-fetch-dest", "iframe"),
        ],
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    // A form another site's page posts (a top-level navigation too): to a zone's UPnP control or
    // to a page, refused before anything acts on it
    for uri in ["/upnp/zone/z1/ctl/avt", "/upnp/zone/z1/ctl/rc", "/"] {
        let req = Request::builder()
            .method("POST")
            .uri(uri)
            .header("host", "127.0.0.1:7788")
            .header("content-type", "text/plain")
            .header("sec-fetch-site", "cross-site")
            .header("sec-fetch-mode", "navigate")
            .header("sec-fetch-dest", "document")
            .body(Body::from("x=1"))
            .unwrap();
        let st = open.clone().oneshot(req).await.unwrap().status();
        assert_eq!(st, StatusCode::FORBIDDEN, "{uri}");
    }

    // With a token: any name (a proxy, a LAN name) with the token
    let dir2 = tempfile::tempdir().unwrap();
    let tok = router(state(dir2.path(), Some("s3cret")).await);
    let (st, _) = send(
        tok.clone(),
        "/v1/info",
        vec![
            ("host", "music.example"),
            ("authorization", "Bearer s3cret"),
            ("x-forwarded-host", "music.example"),
        ],
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    // Behind a proxy that rewrites Host (nginx's bare proxy_pass), the event stream opens with
    // the token
    let mut proxied = ws("https://music.example");
    proxied.push(("authorization", "Bearer s3cret"));
    let (st, _) = send(tok.clone(), "/v1/events", proxied).await;
    assert_ne!(st, StatusCode::FORBIDDEN);

    // The pages can't be framed by another site
    for page in ["/", "/party/abc"] {
        let (st, h) = send(open.clone(), page, vec![("host", "127.0.0.1:7788")]).await;
        assert_eq!(st, StatusCode::OK, "{page}");
        let csp = h.get(header::CONTENT_SECURITY_POLICY).unwrap().to_str().unwrap();
        assert!(csp.contains("frame-ancestors 'none'"), "{page}: {csp}");
        assert_eq!(h.get(header::X_FRAME_OPTIONS).unwrap(), "DENY", "{page}");
    }
}

/// A token the node record once held stops working, once, and the note of it survives until
/// it has: a node-made token is replaced and the note goes (a second retirement replaces
/// nothing); a token set in the config stays, and Health asks for a change until it changes;
/// one already changed before the update needs nothing (ADR 0008).
#[tokio::test]
async fn a_token_the_node_record_held_is_retired_once() {
    let fake = |n: &str| ["older", "credential", n].join("-");
    // Made by the node
    let dir = tempfile::tempdir().unwrap();
    let old = fake("made");
    let s = state(dir.path(), Some(&old)).await;
    let file = dir.path().join("api-token");
    std::fs::write(&file, &old).unwrap();
    s.set_token_file(file.clone());
    access_token::note_published(&s, &old).unwrap();
    access_token::retire_published(&s).await;
    let now = s.token().unwrap();
    assert_ne!(now, old, "replaced");
    assert_eq!(std::fs::read_to_string(&file).unwrap().trim(), now);
    assert!(!access_token::config_token_published(&s));
    access_token::retire_published(&s).await;
    assert_eq!(s.token().unwrap(), now, "once only");
    assert_eq!(
        s.library
            .lock()
            .unwrap()
            .get_setting(access_token::PUBLISHED_TOKEN_KEY)
            .unwrap(),
        None
    );

    // Set in the config: kept, and Health says so until it changes
    let dir2 = tempfile::tempdir().unwrap();
    let conf = fake("config");
    let c = state(dir2.path(), Some(&conf)).await;
    access_token::note_published(&c, &conf).unwrap();
    access_token::retire_published(&c).await;
    assert_eq!(c.token().unwrap(), conf);
    assert!(access_token::config_token_published(&c));
    let (st, health) = call_auth(&router(c.clone()), "GET", "/v1/health", &conf).await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        health
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "api-token" && p["state"] == "change"),
        "{health}"
    );
    *c.token.write().unwrap() = Some(fake("new-config"));
    assert!(
        !access_token::config_token_published(&c),
        "changed: no more warning"
    );
    access_token::retire_published(&c).await;
    assert_eq!(
        c.library
            .lock()
            .unwrap()
            .get_setting(access_token::PUBLISHED_TOKEN_KEY)
            .unwrap(),
        None,
        "the note goes once the token no longer works"
    );

    // Changed in the config before the update: the record held another one, nothing to do
    let dir3 = tempfile::tempdir().unwrap();
    let d = state(dir3.path(), Some(&fake("current"))).await;
    access_token::note_published(&d, &fake("before")).unwrap();
    assert!(!access_token::config_token_published(&d));
}

/// A rename that read the roster before music was turned off, and stores after, leaves music
/// off: the listener's token stays refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slow_roster_change_does_not_undo_a_newer_one() {
    let dir = tempfile::tempdir().unwrap();
    let (s, app, held, release) = roster_node(dir.path(), serde_json::json!({"items": []})).await;
    let rename = tokio::spawn({
        let app = app.clone();
        async move {
            as_who(
                &app,
                "PATCH",
                format!("/v1/people/{ROSTER_KEY}"),
                "s3cret",
                Some(serde_json::json!({"name": "Sam"})),
            )
            .await
        }
    });
    held.notified().await;
    let (st, v) = as_who(
        &app,
        "PATCH",
        format!("/v1/people/{ROSTER_KEY}"),
        "s3cret",
        Some(serde_json::json!({"music": false})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    release.notify_one();
    let (st, v) = rename.await.unwrap();
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["music_enabled"], false, "{v}");
    let p = people::load(&s)
        .people
        .into_iter()
        .find(|p| p.key == ROSTER_KEY)
        .unwrap();
    assert!(!p.music_enabled, "music stays off");
    assert_eq!(p.name.as_deref(), Some("Sam"), "and the rename is kept");
    let tok = accounts::mint(&s, ROSTER_KEY).unwrap();
    let (st, _) = as_who(&app, "GET", "/v1/library/tracks".into(), &tok, None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

/// `GET /v1/people` turning a used invite into a person, slower than a disable made meanwhile,
/// adds the person without bringing music back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slow_reconcile_does_not_undo_a_disable() {
    let dir = tempfile::tempdir().unwrap();
    const NEW: &str = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";
    let used = serde_json::json!({"items": [{"token": INVITE_TOKEN, "used_by": NEW}]});
    let (s, app, held, release) = roster_node(dir.path(), used).await;
    let list = tokio::spawn({
        let app = app.clone();
        async move { as_who(&app, "GET", "/v1/people".into(), "s3cret", None).await }
    });
    held.notified().await;
    let (st, v) = as_who(
        &app,
        "PATCH",
        format!("/v1/people/{ROSTER_KEY}"),
        "s3cret",
        Some(serde_json::json!({"music": false})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    release.notify_one();
    let (st, v) = list.await.unwrap();
    assert_eq!(st, StatusCode::OK, "{v}");
    let r = people::load(&s);
    let p = r.people.iter().find(|p| p.key == ROSTER_KEY).unwrap();
    assert!(!p.music_enabled, "the disable stays: {r:?}");
    assert_eq!(
        r.people.iter().filter(|p| p.key == NEW).count(),
        1,
        "the invitee is in, once"
    );
    assert!(r.invites.is_empty());
    let tok = accounts::mint(&s, ROSTER_KEY).unwrap();
    let (st, _) = as_who(&app, "GET", "/v1/library/tracks".into(), &tok, None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

/// Under "This node's sign-ins" (SourcePolicy::Node) a listener still reaches their own usage
/// insights: their choice, Stop for me and Clear mine; their own sign-ins stay refused.
#[tokio::test]
async fn a_listener_keeps_their_usage_insights_under_the_nodes_sign_ins() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), Some("s3cret")).await;
    let app = router(s.clone());
    let key = "tbepfp8fisyiqpskbmxsmpub47gxb7xdsh5x9ut9qrb97gcq9phy";
    {
        let lib = s.library.lock().unwrap();
        lib.set_setting("people", &serde_json::json!({"people": [{"key": key, "access": "music", "music_enabled": true, "added_at": 1}]}).to_string()).unwrap();
        lib.set_setting(accounts::SOURCE_POLICY_KEY, "node").unwrap();
    }
    // The rule itself, for every usage path and method
    for (method, path) in [
        (axum::http::Method::GET, "/v1/usage/me"),
        (axum::http::Method::PUT, "/v1/usage/me"),
        (axum::http::Method::POST, "/v1/usage/me/clear"),
        (axum::http::Method::POST, "/v1/usage/me/notice"),
        (axum::http::Method::POST, "/v1/usage/sessions"),
        (axum::http::Method::POST, "/v1/usage/sessions/close"),
        (axum::http::Method::POST, "/v1/usage/events"),
    ] {
        assert_eq!(
            accounts::listener_refusal(&s, &method, path),
            None,
            "{method} {path}"
        );
    }
    assert_eq!(
        accounts::listener_refusal(&s, &axum::http::Method::GET, "/v1/sources/tidal/login"),
        Some(accounts::NODE_SOURCES_ONLY),
        "their own sign-in stays refused under the node's"
    );
    // Over HTTP: their choice reads, and Clear mine isn't refused for the policy
    let tok = accounts::mint(&s, key).unwrap();
    let send = |method: &'static str, uri: &'static str, body: Option<serde_json::Value>| {
        let (app, tok) = (app.clone(), tok.clone());
        async move {
            let req = Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {tok}"))
                .header("content-type", "application/json")
                .body(
                    body.map(|b| Body::from(b.to_string()))
                        .unwrap_or_else(Body::empty),
                )
                .unwrap();
            let resp = app.oneshot(req).await.unwrap();
            let st = resp.status();
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            (st, String::from_utf8_lossy(&bytes).into_owned())
        }
    };
    let (st, body) = send("GET", "/v1/usage/me", None).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    let (st, body) = send("POST", "/v1/usage/me/clear", None).await;
    assert!(!body.contains(accounts::NODE_SOURCES_ONLY), "{st} {body}");
    assert_ne!(st, StatusCode::FORBIDDEN, "{body}");
}

/// The sign-in check decides on what the router matched: the route, and the zone and source
/// as the handler gets them. A name written another way is the same name, one that can't be
/// read opens nothing, and a route's missing method is nobody's but the owner's to ask.
#[tokio::test]
async fn the_sign_in_check_reads_what_the_router_matched() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), Some("s3cret")).await;
    let app = router(s.clone());
    let key = "tbepfp8fisyiqpskbmxsmpub47gxb7xdsh5x9ut9qrb97gcq9phy";
    let people = serde_json::json!({"people": [
        {"key": key, "access": "music", "music_enabled": true, "added_at": 1}]});
    let set = |name: &str, value: &str| s.library.lock().unwrap().set_setting(name, value).unwrap();
    set("people", &people.to_string());
    let tok = accounts::mint(&s, key).unwrap();
    let (_, zones) = call_auth(&app, "GET", "/v1/zones", "s3cret").await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let spelled = format!("%{:02x}{}", zid.as_bytes()[0], &zid[1..]);
    let status = |method: &'static str, uri: String, token: &str| {
        let (app, token) = (app.clone(), token.to_string());
        async move { call_auth(&app, method, &uri, &token).await.0 }
    };
    // A zone the owner opened is open under either spelling, one they did not under neither
    for (open, refused) in [(zid.as_str(), false), ("another", true)] {
        set("accounts.listener_zones", &serde_json::json!([open]).to_string());
        for name in [&zid, &spelled] {
            let uri = format!("/v1/zones/{name}/queue");
            let st = status("GET", uri.clone(), &tok).await;
            assert_eq!(st == StatusCode::FORBIDDEN, refused, "{uri}, open: {open}: {st}");
            assert!(refused || st.is_success(), "{uri}: {st}");
        }
    }
    // A source: the same answer however its name is written; one that can't be read, none
    let plain = status("GET", "/v1/sources/tidal/signin".into(), &tok).await;
    assert_ne!(
        plain,
        StatusCode::FORBIDDEN,
        "their own TIDAL sign-in is theirs to see"
    );
    assert_eq!(
        status("GET", "/v1/sources/tid%61l/signin".into(), &tok).await,
        plain
    );
    let unread = status("GET", "/v1/sources/%FF/signin".into(), &tok).await;
    assert!(unread.is_client_error(), "{unread}");
    assert_eq!(
        status("GET", "/v1/sources/lastfm/signin".into(), &tok).await,
        StatusCode::FORBIDDEN,
        "one sign-in for the whole node: the owner's"
    );
    // A method the route does not have: refused to a listener, the router's 405 for the owner
    for (method, uri) in [("GET", "/v1/playlists/import"), ("POST", "/v1/me/prefs")] {
        assert_eq!(
            status(method, uri.into(), &tok).await,
            StatusCode::FORBIDDEN,
            "{method} {uri}"
        );
        assert_eq!(
            status(method, uri.into(), "s3cret").await,
            StatusCode::METHOD_NOT_ALLOWED,
            "{method} {uri}"
        );
    }
    // No route at all: 404 for everyone, signed in or not
    for token in [tok.as_str(), "s3cret", "nobody"] {
        assert_eq!(
            status("GET", "/v1/no/such/route".into(), token).await,
            StatusCode::NOT_FOUND
        );
    }
}

/// Sign-ins are listed (never a token or its hash) and ended one by one: a person's own by
/// themselves, anyone's by the owner. The others keep working.
#[tokio::test]
async fn sign_ins_are_listed_and_ended_one_by_one() {
    const ANN: &str = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";
    const BEN: &str = "8pinxxgqs41n4aididenw5apqp1urfmzdztr8jt4abrkdn435ewo";
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), Some("s3cret")).await;
    let app = router(s.clone());
    let people = serde_json::json!({"people": [
        {"key": ANN, "access": "music", "music_enabled": true, "added_at": 1},
        {"key": BEN, "access": "music", "music_enabled": true, "added_at": 1}]});
    s.library
        .lock()
        .unwrap()
        .set_setting("people", &people.to_string())
        .unwrap();
    let firefox = "Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0";
    let a = accounts::mint_as(&s, ANN, &accounts::NewSignIn::of(Some(firefox), false, true)).unwrap();
    let b = accounts::mint(&s, ANN).unwrap();
    let bens = accounts::mint(&s, BEN).unwrap();
    let me = |token: String| {
        let app = app.clone();
        async move { call_auth(&app, "GET", "/v1/me", &token).await.0 }
    };

    // Ann's own list: both, the one asking marked, with what it said it is
    let (st, mine) = call_auth(&app, "GET", "/v1/me/sign-ins", &a).await;
    assert_eq!(st, StatusCode::OK, "{mine}");
    let mine = mine.as_array().unwrap().clone();
    assert_eq!(mine.len(), 2, "{mine:?}");
    let (this, other): (Vec<_>, Vec<_>) = mine.iter().partition(|x| x["current"] == true);
    assert_eq!((this.len(), other.len()), (1, 1), "{mine:?}");
    assert_eq!(this[0]["label"], "Firefox on Linux");
    assert_eq!(this[0]["remote"], true);
    assert_eq!(this[0]["kind"], "person");
    assert!(this[0]["last_used"].is_i64(), "used today: {mine:?}");
    assert!(
        other[0]["label"].is_null() && other[0]["last_used"].is_null(),
        "{mine:?}"
    );
    let (id_a, id_b) = (
        this[0]["id"].as_str().unwrap().to_string(),
        other[0]["id"].as_str().unwrap().to_string(),
    );
    // Never the token or what the node keeps of it
    let kept = s
        .library
        .lock()
        .unwrap()
        .get_setting("accounts.tokens")
        .unwrap()
        .unwrap();
    let kept: Vec<serde_json::Value> = serde_json::from_str(&kept).unwrap();
    let shown = serde_json::Value::Array(mine.clone()).to_string();
    for t in &kept {
        assert!(!shown.contains(t["hash"].as_str().unwrap()), "{shown}");
    }
    assert!(!shown.contains(&a) && !shown.contains(&b), "{shown}");

    // The owner sees them in People, and ends one: only that one stops
    let (st, all) = call_auth(&app, "GET", "/v1/people", "s3cret").await;
    assert_eq!(st, StatusCode::OK, "{all}");
    let ann = &all["people"][0];
    assert_eq!(
        (ann["key"].as_str(), ann["sessions"].as_u64()),
        (Some(ANN), Some(2)),
        "{all}"
    );
    assert_eq!(ann["sign_ins"].as_array().unwrap().len(), 2, "{all}");
    let (st, v) = call_auth(
        &app,
        "DELETE",
        &format!("/v1/people/{ANN}/sign-ins/{id_b}"),
        "s3cret",
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(me(b.clone()).await, StatusCode::UNAUTHORIZED);
    assert_eq!(me(a.clone()).await, StatusCode::OK);
    assert_eq!(me(bens.clone()).await, StatusCode::OK);
    // Not twice, not under someone else, and not by someone else
    for (uri, token) in [
        (format!("/v1/people/{ANN}/sign-ins/{id_b}"), "s3cret".to_string()),
        (format!("/v1/people/{BEN}/sign-ins/{id_a}"), "s3cret".to_string()),
        (format!("/v1/me/sign-ins/{id_a}"), bens.clone()),
    ] {
        let (st, v) = call_auth(&app, "DELETE", &uri, &token).await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{uri}: {v}");
    }
    let (st, v) = call_auth(
        &app,
        "DELETE",
        &format!("/v1/people/{ANN}/sign-ins/{id_a}"),
        &bens,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "only the owner ends another person's: {v}"
    );
    assert_eq!(me(a.clone()).await, StatusCode::OK);

    // Ann ends every other sign-in of hers: the one asking stays
    let c = accounts::mint(&s, ANN).unwrap();
    let (st, v) = call_auth(&app, "DELETE", "/v1/me/sign-ins", &a).await;
    assert_eq!((st, v["ended"].as_u64()), (StatusCode::OK, Some(1)), "{v}");
    assert_eq!(me(c).await, StatusCode::UNAUTHORIZED);
    assert_eq!(me(a.clone()).await, StatusCode::OK);
    assert_eq!(me(bens.clone()).await, StatusCode::OK, "Ben's are his");
    // And the owner ends all of Ben's: he keeps his music access and can log in again
    let (st, v) = call_auth(&app, "DELETE", &format!("/v1/people/{BEN}/sign-ins"), "s3cret").await;
    assert_eq!((st, v["ended"].as_u64()), (StatusCode::OK, Some(1)), "{v}");
    assert_eq!(me(bens).await, StatusCode::UNAUTHORIZED);
    assert!(people::may_use_music(&s, BEN));
    // Ann ends the one she is asking with
    let (st, v) = call_auth(&app, "DELETE", &format!("/v1/me/sign-ins/{id_a}"), &a).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(me(a).await, StatusCode::UNAUTHORIZED);
}

/// The owner's browsers and apps are listed and ended one by one with the API token, also
/// when that token is set in the config (where Make a new token is refused), and ending them
/// all leaves the API token as it is.
#[tokio::test]
async fn the_owners_sign_ins_end_without_a_new_api_token() {
    const OWNER: &str = "tbepfp8fisyiqpskbmxsmpub47gxb7xdsh5x9ut9qrb97gcq9phy";
    let dir = tempfile::tempdir().unwrap();
    // A token from the config: it can't be replaced from Settings
    let s = state(dir.path(), Some("s3cret")).await;
    let app = router(s.clone());
    let (st, v) = as_who(&app, "POST", "/v1/node/token".into(), "s3cret", None).await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    let hashed = |t: &str| blake3::hash(t.as_bytes()).to_hex().to_string();
    let set = |setting: &str, tokens: &[&str]| {
        let list: Vec<_> = tokens
            .iter()
            .enumerate()
            .map(|(i, t)| serde_json::json!({"hash": hashed(t), "key": OWNER, "created_at": 10 + i}))
            .collect();
        s.library
            .lock()
            .unwrap()
            .set_setting(setting, &serde_json::Value::Array(list).to_string())
            .unwrap();
    };
    let left = |setting: &str| -> usize {
        let j = s.library.lock().unwrap().get_setting(setting).unwrap().unwrap();
        serde_json::from_str::<Vec<serde_json::Value>>(&j).unwrap().len()
    };
    set("accounts.owner_sessions", &["pmo_browser-1", "pmo_browser-2"]);
    set("accounts.device_sessions", &["pmo_phone"]);
    let (st, mine) = call_auth(&app, "GET", "/v1/me/sign-ins", "s3cret").await;
    assert_eq!(st, StatusCode::OK, "{mine}");
    let kinds: Vec<&str> = mine
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds.iter().filter(|k| **k == "browser").count(), 2, "{mine}");
    assert_eq!(kinds.iter().filter(|k| **k == "app").count(), 1, "{mine}");
    assert!(
        mine.as_array().unwrap().iter().all(|x| x["current"] == false),
        "{mine}"
    );
    // The app's sign-in, which a browser's Log out never ends, ends by its id
    let phone = mine
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["kind"] == "app")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (st, v) = call_auth(&app, "DELETE", &format!("/v1/me/sign-ins/{phone}"), "s3cret").await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        (left("accounts.owner_sessions"), left("accounts.device_sessions")),
        (2, 0)
    );
    // All the rest at once; the API token still works
    let (st, v) = call_auth(&app, "DELETE", "/v1/me/sign-ins", "s3cret").await;
    assert_eq!((st, v["ended"].as_u64()), (StatusCode::OK, Some(2)), "{v}");
    assert_eq!(
        (left("accounts.owner_sessions"), left("accounts.device_sessions")),
        (0, 0)
    );
    let (st, me) = call_auth(&app, "GET", "/v1/me", "s3cret").await;
    assert_eq!((st, me["role"].as_str()), (StatusCode::OK, Some("owner")), "{me}");
    // Ending one that is not there (or all, when none is left) writes nothing
    for list in ["accounts.owner_sessions", "accounts.device_sessions"] {
        s.library.lock().unwrap().delete_setting(list).unwrap();
    }
    let (st, _) = call_auth(&app, "DELETE", "/v1/me/sign-ins/0123456789abcdef", "s3cret").await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, v) = call_auth(&app, "DELETE", "/v1/me/sign-ins", "s3cret").await;
    assert_eq!((st, v["ended"].as_u64()), (StatusCode::OK, Some(0)), "{v}");
    for list in ["accounts.owner_sessions", "accounts.device_sessions"] {
        let now = s.library.lock().unwrap().get_setting(list).unwrap();
        assert_eq!(now, None, "{list}: nothing ended, nothing written");
    }
    // A party guest has none to list or end: these are not a guest's routes
    let (_, zones) = call_auth(&app, "GET", "/v1/zones", "s3cret").await;
    let zone = zones[0]["id"].as_str().unwrap().to_string();
    let (st, made) = as_who(
        &app,
        "POST",
        "/v1/party".into(),
        "s3cret",
        Some(serde_json::json!({"zone": zone})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{made}");
    let code = made["url"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_string();
    let join = Request::builder()
        .method("POST")
        .uri("/v1/party/join")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({"code": code, "name": "Guest"}).to_string(),
        ))
        .unwrap();
    let r = app.clone().oneshot(join).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let b = r.into_body().collect().await.unwrap().to_bytes();
    let guest: serde_json::Value = serde_json::from_slice(&b).unwrap();
    let guest = guest["token"].as_str().unwrap().to_string();
    assert!(guest.starts_with("pmg_"), "{guest}");
    for (method, uri) in [
        ("GET", "/v1/me/sign-ins"),
        ("DELETE", "/v1/me/sign-ins"),
        ("DELETE", "/v1/me/sign-ins/0123456789abcdef"),
    ] {
        let (st, v) = call_auth(&app, method, uri, &guest).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{method} {uri}: {v}");
    }
}

/// What a sign-in says it is becomes a short name, or nothing.
#[test]
fn a_sign_in_is_named_from_what_it_says_it_is() {
    for (ua, want) in [
        (
            "Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0",
            Some("Firefox on Linux"),
        ),
        (
            "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1",
            Some("Safari on iPhone"),
        ),
        (
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0",
            Some("Edge on Windows"),
        ),
        (
            "Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36",
            Some("Chrome on Android"),
        ),
        (
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15",
            Some("Safari on Mac"),
        ),
        ("pubky-music-desktop/0.2.4", Some("Odeion app 0.2.4")),
        ("pubky-music-desktop/0.2.4<script>", Some("Odeion app 0.2.4")),
        // Only a version is taken from what the app says, never words
        ("odeion/Someones-Phone", Some("Odeion app")),
        ("odeion/1.2.3-beta", Some("Odeion app 1.2.3")),
        ("curl/8.5.0", None),
        ("", None),
    ] {
        assert_eq!(accounts::label_of(ua).as_deref(), want, "{ua}");
    }
    let what = accounts::NewSignIn::of(Some("curl/8.5.0"), true, false);
    assert_eq!((what.device, what.label, what.remote), (true, None, false));
}

/// A people list the node can't read (damaged, or written by a newer version) is never
/// replaced by a change: the change is refused and the list stays as it is.
#[tokio::test]
async fn an_unreadable_people_list_is_never_written_over() {
    const ANN: &str = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";
    const BEN: &str = "8pinxxgqs41n4aididenw5apqp1urfmzdztr8jt4abrkdn435ewo";
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), Some("s3cret")).await;
    let app = router(s.clone());
    // An access level this version doesn't know
    let kept = serde_json::json!({"people": [
        {"key": ANN, "access": "family", "music_enabled": true, "added_at": 1}]})
    .to_string();
    s.library.lock().unwrap().set_setting("people", &kept).unwrap();
    let stored = || s.library.lock().unwrap().get_setting("people").unwrap();
    for (method, uri, body) in [
        (
            "POST",
            "/v1/people/invites".to_string(),
            Some(serde_json::json!({"access": "music", "key": BEN})),
        ),
        (
            "PATCH",
            format!("/v1/people/{ANN}"),
            Some(serde_json::json!({"music": false})),
        ),
        ("DELETE", "/v1/people/invites/none".to_string(), None),
        ("DELETE", format!("/v1/people/{ANN}"), None),
        // Reading it says so too, instead of showing nobody
        ("GET", "/v1/people".to_string(), None),
    ] {
        let (st, v) = as_who(&app, method, uri.clone(), "s3cret", body).await;
        assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR, "{method} {uri}: {v}");
        assert!(
            v["error"].as_str().unwrap().contains("can't be read"),
            "{method} {uri}: {v}"
        );
        assert_eq!(stored().as_deref(), Some(kept.as_str()), "{method} {uri}");
    }
    // Sign-ins don't go by the list: a person's can still be ended
    let (st, v) = as_who(
        &app,
        "DELETE",
        format!("/v1/people/{ANN}/sign-ins"),
        "s3cret",
        None,
    )
    .await;
    assert_eq!((st, v["ended"].as_u64()), (StatusCode::OK, Some(0)), "{v}");
}

/// A removal that does not finish is remembered: `GET /v1/people` names the person under
/// `unfinished` (they may be off the list already), so Remove again is still offered after a
/// reload or a restart. It is forgotten when the removal finishes, or nothing of theirs is left.
#[tokio::test]
async fn a_removal_that_did_not_finish_is_remembered_until_it_does() {
    let dir = tempfile::tempdir().unwrap();
    let (s, anna, ben) = two_people_node(dir.path(), "http://127.0.0.1:9").await;
    let app = router(s.clone());
    let owner = Some("s3cret");
    let people = || {
        let app = app.clone();
        async move {
            let (st, v, _) = send_as(&app, "GET", "/v1/people", owner, &[], None).await;
            assert_eq!(st, StatusCode::OK, "{v}");
            let listed: Vec<String> = v["people"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p["key"].as_str().unwrap().to_string())
                .collect();
            (listed, v["unfinished"].clone())
        }
    };
    // (Cy has an account on this node's homeserver too: removed, they stay listed without music)
    let cy = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";
    s.library
        .lock()
        .unwrap()
        .set_setting(
            "people",
            &serde_json::json!({"people": [
                {"key": anna, "access": "music", "music_enabled": true, "added_at": 1},
                {"key": ben, "access": "music", "music_enabled": true, "added_at": 1},
                {"key": cy, "access": "homeserver_music", "music_enabled": true, "added_at": 1}]})
            .to_string(),
        )
        .unwrap();
    let (listed, unfinished) = people().await;
    assert!(
        listed.contains(&ben.to_string()) && unfinished.is_null(),
        "{listed:?} {unfinished}"
    );
    // Ben's homeserver approval can't be taken off the node (a folder stands where its file is)
    let grants = s.db_path.parent().unwrap().join("grants");
    let in_the_way = |key: &str| grants.join(format!("{key}.session"));
    std::fs::create_dir_all(in_the_way(ben).join("x")).unwrap();
    for _ in 0..2 {
        let (st, v, _) = send_as(&app, "DELETE", &format!("/v1/people/{ben}"), owner, &[], None).await;
        assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
        assert!(v["error"].as_str().unwrap().contains("remove them again"), "{v}");
        // Off the list, and named as not finished (once)
        let (listed, unfinished) = people().await;
        assert!(!listed.contains(&ben.to_string()), "{listed:?}");
        assert_eq!(unfinished, serde_json::json!([ben]));
    }
    // Adding Ben again has to finish the removal first: not while it can't
    let add_ben = serde_json::json!({"access": "music", "key": ben});
    let (st, v, _) = send_as(
        &app,
        "POST",
        "/v1/people/invites",
        owner,
        &[],
        Some(add_ben.clone()),
    )
    .await;
    assert!(!st.is_success(), "{st} {v}");
    assert_eq!(people().await.1, serde_json::json!([ben]));
    // What was in the way is gone: adding Ben again finishes it, and Ben is no longer named
    // as someone whose removal did not finish
    std::fs::remove_dir_all(in_the_way(ben)).unwrap();
    let (st, v, _) = send_as(&app, "POST", "/v1/people/invites", owner, &[], Some(add_ben)).await;
    assert!(st.is_success(), "{st} {v}");
    let (listed, unfinished) = people().await;
    assert!(
        listed.contains(&ben.to_string()) && unfinished.is_null(),
        "{listed:?} {unfinished}"
    );
    // Cy's removal fails the same way: still listed (the homeserver account), and named
    std::fs::create_dir_all(in_the_way(cy).join("x")).unwrap();
    let (st, v, _) = send_as(&app, "DELETE", &format!("/v1/people/{cy}"), owner, &[], None).await;
    assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    let (listed, unfinished) = people().await;
    assert!(listed.contains(&cy.to_string()), "{listed:?}");
    assert_eq!(unfinished, serde_json::json!([cy]));
    // Removing again, when it can finish, answers 200 and it is forgotten
    std::fs::remove_dir_all(in_the_way(cy)).unwrap();
    let (st, v, _) = send_as(&app, "DELETE", &format!("/v1/people/{cy}"), owner, &[], None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (listed, unfinished) = people().await;
    assert!(
        listed.contains(&cy.to_string()) && unfinished.is_null(),
        "{listed:?} {unfinished}"
    );
    // Ben again, off the list with something left: the next Remove finishes it (the grant
    // file is there this time, and goes)
    std::fs::create_dir_all(in_the_way(ben).join("x")).unwrap();
    let (st, _, _) = send_as(&app, "DELETE", &format!("/v1/people/{ben}"), owner, &[], None).await;
    assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR);
    std::fs::remove_dir_all(in_the_way(ben)).unwrap();
    std::fs::write(in_the_way(ben), b"left over").unwrap();
    let (st, v, _) = send_as(&app, "DELETE", &format!("/v1/people/{ben}"), owner, &[], None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(!in_the_way(ben).exists());
    assert!(people().await.1.is_null());
    // A removal that works leaves nothing to remember
    let (st, v, _) = send_as(&app, "DELETE", &format!("/v1/people/{anna}"), owner, &[], None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (listed, unfinished) = people().await;
    assert!(
        !listed.contains(&anna.to_string()) && unfinished.is_null(),
        "{listed:?} {unfinished}"
    );
    // A name that was remembered for someone of whom nothing is left (a restored list, say)
    // goes at the next Remove, which answers that there is nobody
    s.library
        .lock()
        .unwrap()
        .set_setting("people.unfinished", &serde_json::json!([ben]).to_string())
        .unwrap();
    let (_, unfinished) = people().await;
    assert_eq!(unfinished, serde_json::json!([ben]));
    let (st, _, _) = send_as(&app, "DELETE", &format!("/v1/people/{ben}"), owner, &[], None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (_, unfinished) = people().await;
    assert!(unfinished.is_null(), "{unfinished}");
}

/// A removal is noted as unfinished in the same write that takes the person off the list: a
/// node that stops during the cleanup still offers Remove after its restart. While it runs the page doesn't
/// name them (it isn't over); when it ends well the note goes. The note is this node's own:
/// a restore keeps it, and someone a restore lists with music again isn't named.
#[tokio::test]
async fn a_removal_is_noted_until_it_ends_and_a_restore_keeps_the_note() {
    let dir = tempfile::tempdir().unwrap();
    let (s, anna, ben) = two_people_node(dir.path(), "http://127.0.0.1:9").await;
    let app = router(s.clone());
    let owner = Some("s3cret");
    let note = || {
        s.library
            .lock()
            .unwrap()
            .get_setting("people.unfinished")
            .unwrap()
    };
    let named = || {
        let app = app.clone();
        async move {
            let (st, v, _) = send_as(&app, "GET", "/v1/people", owner, &[], None).await;
            assert_eq!(st, StatusCode::OK, "{v}");
            v["unfinished"].clone()
        }
    };
    let listed = |key: &str| {
        s.library
            .lock()
            .unwrap()
            .get_setting("people")
            .unwrap()
            .is_some_and(|r| r.contains(key))
    };
    // The list and the note are one write: when the note can't be written, the list stays as
    // it was (no one off the list and not noted) and the removal answers the error
    let db = rusqlite::Connection::open(&s.db_path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER no_note_i BEFORE INSERT ON settings WHEN NEW.key = 'people.unfinished'
             BEGIN SELECT RAISE(ABORT, 'disk I/O error'); END;
             CREATE TRIGGER no_note_u BEFORE UPDATE ON settings WHEN NEW.key = 'people.unfinished'
             BEGIN SELECT RAISE(ABORT, 'disk I/O error'); END;",
    )
    .unwrap();
    let (st, v, _) = send_as(&app, "DELETE", &format!("/v1/people/{ben}"), owner, &[], None).await;
    assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    assert!(listed(ben), "still listed");
    assert_eq!(note(), None);
    db.execute_batch("DROP TRIGGER no_note_i; DROP TRIGGER no_note_u;")
        .unwrap();
    // Ben's removal is held up in its cleanup (another homeserver step of his runs)
    let lock = crate::personal::person_lock(&s, ben);
    let held = lock.lock().await;
    let removal = tokio::spawn({
        let (app, ben) = (app.clone(), ben.to_string());
        async move { send_as(&app, "DELETE", &format!("/v1/people/{ben}"), owner, &[], None).await }
    });
    for _ in 0..200 {
        if !listed(ben) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(!listed(ben), "off the list");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    // A stop now would leave the note; the page doesn't name him while it runs
    assert_eq!(note(), Some(serde_json::json!([ben]).to_string()));
    assert!(named().await.is_null());
    drop(held);
    let (st, v, _) = removal.await.unwrap();
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(note(), None, "it ended well");
    // A backup noting Anna, restored on a node noting Ben: Ben's note stays, Anna's doesn't
    // come; and Anna, listed with music, isn't named even if noted
    let set = |v: serde_json::Value| {
        s.library
            .lock()
            .unwrap()
            .set_setting("people.unfinished", &v.to_string())
            .unwrap()
    };
    set(serde_json::json!([anna]));
    let (taken, _) = backup::make(&s, None).unwrap();
    set(serde_json::json!([ben]));
    backup::restore(&s, taken, None, false).await.unwrap();
    assert_eq!(note(), Some(serde_json::json!([ben]).to_string()));
    assert_eq!(named().await, serde_json::json!([ben]));
    set(serde_json::json!([anna, ben]));
    assert!(listed(anna));
    assert_eq!(named().await, serde_json::json!([ben]));
    // A removal stopped before the list changed (it can't be read) notes nothing: nothing of
    // Anna's was removed yet (she has preferences here, so the removal goes as far as the list)
    s.library
        .lock()
        .unwrap()
        .delete_setting("people.unfinished")
        .unwrap();
    let people = s.library.lock().unwrap().get_setting("people").unwrap().unwrap();
    s.library
        .lock()
        .unwrap()
        .set_setting(&crate::prefs::storage_key_of(anna), "{}")
        .unwrap();
    s.library
        .lock()
        .unwrap()
        .set_setting("people", "{not a list")
        .unwrap();
    let (st, v, _) = send_as(&app, "DELETE", &format!("/v1/people/{anna}"), owner, &[], None).await;
    assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    assert_eq!(note(), None);
    s.library.lock().unwrap().set_setting("people", &people).unwrap();
}

/// An app password (Settings › People & access › Apps) plays, controls zones and browses as
/// the owner, and nothing else: no settings, people, tokens, sign-ins or library changes, never
/// a file path in a queue, and never the API token in what it reads. Subsonic takes it as a
/// password too. Removing it, or the app signing out, stops it at once; the API token stays.
#[tokio::test]
async fn an_app_password_plays_and_browses_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("music")).unwrap();
    write_wav(&dir.path().join("music/one.wav"), 4410);
    let s = state(dir.path(), Some("s3cret")).await;
    let app = router(s.clone());
    let owner = Some("s3cret");
    // One song in the library
    let (st, _, _) = send_as(
        &app,
        "POST",
        "/v1/library/scan",
        owner,
        &[],
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    for _ in 0..500 {
        if !s.is_scanning() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(!s.is_scanning());
    // Old nodes have none: the list is empty, nothing else changes
    let (st, v, _) = send_as(&app, "GET", "/v1/node/app-passwords", owner, &[], None).await;
    assert_eq!((st, &v), (StatusCode::OK, &serde_json::json!([])));
    let make = |name: serde_json::Value| {
        let app = app.clone();
        async move {
            send_as(
                &app,
                "POST",
                "/v1/node/app-passwords",
                owner,
                &[],
                Some(serde_json::json!({"name": name})),
            )
            .await
        }
    };
    let (st, _, _) = make(serde_json::json!("  ")).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "a name is needed");
    let (st, ha, _) = make(serde_json::json!(" Home Assistant ")).await;
    assert_eq!(st, StatusCode::OK, "{ha}");
    let pw = ha["password"].as_str().unwrap().to_string();
    let id = ha["id"].as_str().unwrap().to_string();
    assert!(pw.starts_with(app_passwords::PREFIX) && pw.len() > 40, "{ha}");
    assert_eq!(ha["name"], "Home Assistant");
    let (_, sub, _) = make(serde_json::json!("Symfonium")).await;
    let sub_pw = sub["password"].as_str().unwrap().to_string();
    // Listed without the password
    let (_, listed, _) = send_as(&app, "GET", "/v1/node/app-passwords", owner, &[], None).await;
    assert_eq!(listed.as_array().unwrap().len(), 2, "{listed}");
    assert!(!listed.to_string().contains(&pw) && !listed.to_string().contains(&sub_pw));
    assert_eq!(listed[0]["name"], "Symfonium", "newest first: {listed}");
    assert!(listed[1]["last_used"].is_null());

    let mine = Some(pw.as_str());
    // Who it is, and what it can do
    let (st, me, _) = send_as(&app, "GET", "/v1/me", mine, &[], None).await;
    assert_eq!(st, StatusCode::OK, "{me}");
    assert_eq!(me["role"], "app");
    let can: Vec<&str> = me["can"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert_eq!(
        can,
        [
            "artwork",
            "session",
            "library",
            "zone_list",
            "zone_use",
            "collections",
            "own_play",
            "zone_group"
        ],
        "{me}"
    );
    // Plays and browses: every zone, the library, the owner's collections and play order
    let (st, zones, _) = send_as(&app, "GET", "/v1/zones", mine, &[], None).await;
    assert_eq!(st, StatusCode::OK, "{zones}");
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    for (method, uri) in [
        ("GET", "/v1/library/albums".to_string()),
        ("GET", "/v1/outputs".to_string()),
        ("GET", "/v1/playlists".to_string()),
        ("GET", "/v1/play-order".to_string()),
        ("GET", format!("/v1/zones/{zid}")),
        ("POST", format!("/v1/zones/{zid}/shuffle")),
    ] {
        let (st, v, _) = send_as(&app, method, &uri, mine, &[], Some(serde_json::json!({}))).await;
        assert!(st.is_success(), "{method} {uri}: {st} {v}");
    }
    // Noted as used today
    let (_, listed, _) = send_as(&app, "GET", "/v1/node/app-passwords", owner, &[], None).await;
    let used = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == id.as_str())
        .unwrap();
    assert!(used["last_used"].is_i64(), "{listed}");
    // Nothing else: settings, people, tokens and app passwords, sign-ins, the library, backups
    for (method, uri) in [
        ("GET", "/v1/node/settings"),
        ("POST", "/v1/node/token"),
        ("GET", "/v1/node/app-passwords"),
        ("POST", "/v1/node/app-passwords"),
        ("GET", "/v1/people"),
        ("GET", "/v1/me/sign-ins"),
        ("DELETE", "/v1/me/sign-ins"),
        ("GET", "/v1/me/prefs"),
        ("PUT", "/v1/tags"),
        ("DELETE", "/v1/me/listens"),
        ("POST", "/v1/library/scan"),
        ("POST", "/v1/node/settings/backup"),
    ] {
        let (st, v, _) = send_as(&app, method, uri, mine, &[], Some(serde_json::json!({}))).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{method} {uri}: {v}");
    }
    let (_, v, _) = send_as(&app, "GET", "/v1/people", mine, &[], None).await;
    let why = v["error"].as_str().unwrap();
    assert!(why.contains("app password") && !why.contains("listener"), "{v}");
    // A queue takes library songs and web links, on the home network too (an announcement
    // from Home Assistant), fetched only where an app's may be; never a file
    let queue = format!("/v1/zones/{zid}/queue");
    for uri in ["/etc/hosts", "file:///etc/hosts", "relative/song.flac"] {
        let (st, v, _) = send_as(
            &app,
            "POST",
            &queue,
            mine,
            &[],
            Some(serde_json::json!({"items": [{"uri": uri}]})),
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{uri}: {v}");
    }
    let tts = "http://192.168.1.20:8123/api/tts_proxy/hello.mp3";
    let (st, v, _) = send_as(
        &app,
        "POST",
        &queue,
        mine,
        &[],
        Some(serde_json::json!({"items": [{"uri": format!(" {tts} ")}], "at": "end"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let queued = s.zones.get(&pm_zones::ZoneId::new(&zid)).unwrap().state();
    let it = queued
        .queue
        .iter()
        .find(|q| q.uri == tts)
        .expect("queued as checked");
    assert_eq!(it.limit, Some(accounts::APP_FETCH));
    assert_eq!(it.by, None, "the owner's");
    // An app's answers go through what people's do (file names, not the owner's folders; and
    // no sign-in of the node's in a link, which accounts::KnownSet now names app passwords in)
    let folder = dir.path().join("music").to_string_lossy().into_owned();
    let (_, t, _) = send_as(&app, "GET", "/v1/library/tracks?limit=5", owner, &[], None).await;
    assert!(
        t.to_string().contains(&folder),
        "the owner's answer names the folder: {t}"
    );
    let (_, t, _) = send_as(&app, "GET", "/v1/library/tracks?limit=5", mine, &[], None).await;
    assert!(
        t.to_string().contains("one.wav") && !t.to_string().contains(&folder),
        "{t}"
    );
    let (_, z, _) = send_as(&app, "GET", &format!("/v1/zones/{zid}"), mine, &[], None).await;
    assert!(
        !z.to_string().contains("s3cret") && !z.to_string().contains(&sub_pw),
        "{z}"
    );
    let known = accounts::KnownTokens::lazy(&s);
    assert!(
        known.has(&sub_pw) && known.has("s3cret"),
        "an app password is hidden like a sign-in"
    );
    // The speakers, and grouping them: yes; adding a speaker by address or tuning one: no
    let (st, _, _) = send_as(
        &app,
        "POST",
        "/v1/outputs/upnp",
        mine,
        &[],
        Some(serde_json::json!({"address": "127.0.0.1:1"})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _, _) = send_as(
        &app,
        "PUT",
        &format!("/v1/zones/{zid}/dsp"),
        mine,
        &[],
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    // Removing a speaker's own zone, no; grouping two speakers and ungrouping them, yes
    let (st, v, _) = send_as(&app, "DELETE", &format!("/v1/zones/{zid}"), mine, &[], None).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    assert!(
        s.zones.get(&pm_zones::ZoneId::new(&zid)).is_some(),
        "the zone is still there"
    );
    let a: Arc<dyn Output> = Arc::new(NullOutput::with_volume(pm_model::VolumeKind::Percent));
    let b: Arc<dyn Output> = Arc::new(NullOutput::with_volume(pm_model::VolumeKind::Db {
        min: -60.0,
        max: 0.0,
        step: 1.0,
    }));
    let (aid, bid) = (a.id(), b.id());
    s.outputs.register(a);
    s.outputs.register(b);
    let (st, pair, _) = send_as(
        &app,
        "POST",
        "/v1/zones",
        mine,
        &[],
        Some(serde_json::json!({"name": "Kitchen + Den", "outputs": [aid.0, bid.0]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{pair}");
    let pid = pair["id"].as_str().unwrap().to_string();
    assert!(!pid.starts_with("group:"), "a zone's id is its name's: {pid}");
    let (st, v, _) = send_as(&app, "DELETE", &format!("/v1/zones/{pid}"), mine, &[], None).await;
    assert_eq!(st, StatusCode::NO_CONTENT, "{v}");
    assert!(s.zones.get(&pm_zones::ZoneId::new(&pid)).is_none());
    // Making a collection public is the owner's (it is posted under their name)
    for uri in ["/v1/collections/x/visibility", "/v1/playlists/x/visibility"] {
        let (st, v, _) = send_as(
            &app,
            "PUT",
            uri,
            mine,
            &[],
            Some(serde_json::json!({"public": true})),
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{uri}: {v}");
    }
    // The live Spotify item is that exact link, nothing after it
    let (st, v, _) = send_as(
        &app,
        "POST",
        &queue,
        mine,
        &[],
        Some(serde_json::json!({"items": [{"uri": "librespot:connect/../etc/hosts"}]})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    // (and for a person, the same check)
    let mut theirs = pm_zones::QueueItem::new("librespot:connect/../etc/hosts");
    assert!(accounts::check_listener_item(&mut theirs).is_err());
    let mut live = pm_zones::QueueItem::new(librespot::LIVE_URI);
    assert!(accounts::check_listener_item(&mut live).is_ok());
    // One speaker is no group: an app doesn't add zones it couldn't take away
    let (st, v, _) = send_as(
        &app,
        "POST",
        "/v1/zones",
        mine,
        &[],
        Some(serde_json::json!({"name": "Alone", "outputs": [aid.0]})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    // A collection an app makes is private, whatever it asks
    let (st, v, _) = send_as(
        &app,
        "POST",
        "/v1/collections",
        mine,
        &[],
        Some(serde_json::json!({"name": "From the app", "public": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(
        v.to_string().contains(r#""visibility":"private""#)
            && !v.to_string().contains(r#""visibility":"public""#),
        "{v}"
    );

    // Subsonic: the password, plain, hex or as a salted md5; not for a scan
    let rest = |q: String| {
        let app = app.clone();
        async move {
            let (_, v, _) = send_as(
                &app,
                "GET",
                &format!("/rest/{q}&u=pubky&v=1.16.1&c=t&f=json"),
                None,
                &[],
                None,
            )
            .await;
            v["subsonic-response"].clone()
        }
    };
    let salted = |pw: &str, salt: &str| {
        use md5::Digest;
        let mut h = md5::Md5::new();
        h.update(pw.as_bytes());
        h.update(salt.as_bytes());
        hex::encode(h.finalize())
    };
    assert_eq!(rest(format!("ping?p={sub_pw}")).await["status"], "ok");
    // (its answers name no file on the node's disk)
    let songs = rest(format!("getRandomSongs?p={sub_pw}")).await;
    assert_eq!(songs["status"], "ok", "{songs}");
    assert!(
        songs.to_string().contains(r#""title":"One""#),
        "the song: {songs}"
    );
    assert!(!songs.to_string().contains("\"path\""), "{songs}");
    let theirs = rest("getRandomSongs?p=s3cret".into()).await;
    assert!(
        theirs.to_string().contains("\"path\""),
        "the owner's answer has it: {theirs}"
    );
    assert_eq!(
        rest(format!("ping?p=enc:{}", hex::encode(&sub_pw))).await["status"],
        "ok"
    );
    assert_eq!(
        rest(format!("ping?t={}&s=abc123", salted(&sub_pw, "abc123"))).await["status"],
        "ok"
    );
    assert_eq!(
        rest(format!("ping?t={}&s=abc123", salted("s3cret", "abc123"))).await["status"],
        "ok"
    );
    let wrong = rest(format!("ping?t={}&s=abc123", salted("pma_nope", "abc123"))).await;
    assert_eq!(wrong["error"]["code"], 40, "{wrong}");
    let scan = rest(format!("startScan?p={sub_pw}")).await;
    assert_eq!(scan["error"]["code"], 50, "{scan}");
    assert_eq!(rest("startScan?p=s3cret".into()).await["status"], "ok");
    // A saved radio station's link on the node with the other app's password in it: the
    // owner's Subsonic answer shows it, an app's never
    let origin = s.node_origins().first().cloned().expect("the node's own address");
    let link = format!("{origin}/v1/library/tracks/1/file?token={pw}");
    let enc: String = url::form_urlencoded::byte_serialize(link.as_bytes()).collect();
    let made = rest(format!(
        "createInternetRadioStation?p=s3cret&name=Home&streamUrl={enc}"
    ))
    .await;
    assert_eq!(made["status"], "ok", "{made}");
    let theirs = rest("getInternetRadioStations?p=s3cret".into()).await;
    assert!(
        theirs.to_string().contains(&pw),
        "the owner's answer has it: {theirs}"
    );
    let seen = rest(format!("getInternetRadioStations?p={sub_pw}")).await;
    assert!(
        seen.to_string().contains("/v1/library/tracks/1/file") && !seen.to_string().contains(&pw),
        "{seen}"
    );

    // Search's stream is not JSON (the `auth` filter leaves it): each line is filtered for
    // anyone but the owner, a link with a sign-in in it included
    let hit = search::SearchHit {
        kind: "song".into(),
        title: "One".into(),
        image: Some(format!("/v1/library/albums/1/cover?token={sub_pw}")),
        ..Default::default()
    };
    let answer = search::SourceAnswer {
        source: "library".into(),
        status: "ok".into(),
        hits: vec![hit],
        ..Default::default()
    };
    let hide = (accounts::library_roots(&s), s.node_origins(), s.clone());
    let line = String::from_utf8(search::ndjson_line(&answer, Some(&hide))).unwrap();
    assert!(
        line.contains("/v1/library/albums/1/cover") && !line.contains(&sub_pw),
        "{line}"
    );
    assert!(String::from_utf8(search::ndjson_line(&answer, None))
        .unwrap()
        .contains(&sub_pw));
    let (st, _, _) = send_as(&app, "GET", "/v1/search?q=one", mine, &[], None).await;
    assert_eq!(st, StatusCode::OK);

    // Removed: that app stops at once, the other and the API token go on
    let (st, _, _) = send_as(
        &app,
        "DELETE",
        &format!("/v1/node/app-passwords/{id}"),
        owner,
        &[],
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _, _) = send_as(
        &app,
        "DELETE",
        &format!("/v1/node/app-passwords/{id}"),
        owner,
        &[],
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _, _) = send_as(&app, "GET", "/v1/zones", mine, &[], None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _, _) = send_as(&app, "GET", "/v1/zones", Some(&sub_pw), &[], None).await;
    assert_eq!(st, StatusCode::OK);
    // An app that signs out gives its password up
    let (st, v, _) = send_as(&app, "DELETE", "/v1/me/token", Some(&sub_pw), &[], None).await;
    assert_eq!(
        (st, &v["ended"]),
        (StatusCode::OK, &serde_json::json!(true)),
        "{v}"
    );
    assert_eq!(rest(format!("ping?p={sub_pw}")).await["error"]["code"], 40);
    let (st, v, _) = send_as(&app, "GET", "/v1/node/app-passwords", owner, &[], None).await;
    assert_eq!((st, v), (StatusCode::OK, serde_json::json!([])));
    // At most 20 at once
    for n in 0..20 {
        assert_eq!(
            make(serde_json::json!(format!("app {n}"))).await.0,
            StatusCode::OK
        );
    }
    assert_eq!(make(serde_json::json!("one more")).await.0, StatusCode::CONFLICT);
}
