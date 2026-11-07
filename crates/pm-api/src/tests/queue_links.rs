//! Links on the node in a zone's queue never carry a sign-in: a browser's cover link
//! (`/v1/library/albums/{id}/cover?token=…`) loses the token on the way into the queue, in every
//! answer to listeners (zones, queue, presence) and on a renderer's screen; a saved queue that
//! held one is cleaned when the node starts, and that sign-in ends.

use super::*;

const LISTENER: &str = "tbepfp8fisyiqpskbmxsmpub47gxb7xdsh5x9ut9qrb97gcq9phy";

async fn as_token(
    app: &Router,
    token: &str,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, String) {
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
    let r = app.clone().oneshot(req).await.unwrap();
    let st = r.status();
    let b = r.into_body().collect().await.unwrap().to_bytes();
    (st, String::from_utf8_lossy(&b).into_owned())
}

fn let_in(s: &NodeState, key: &str) -> String {
    s.library
        .lock()
        .unwrap()
        .set_setting(
            "people",
            &serde_json::json!({"people": [{"key": key, "access": "music", "music_enabled": true, "added_at": 1}]})
                .to_string(),
        )
        .unwrap();
    accounts::mint(s, key).unwrap()
}

/// A queued cover link with the owner's sign-in: the queue keeps the link without it, so a
/// listener's reads of the zone, its queue, who is listening and the event stream, and a
/// renderer's screen, never see the sign-in. The cover itself still loads with `?token=`.
#[tokio::test]
async fn queue_links_never_carry_a_sign_in() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("a.wav");
    write_wav(&wav, 44100);
    let s = state(dir.path(), Some("s3cret")).await;
    let app = router(s.clone());
    let listener = let_in(&s, LISTENER);
    let (_, zones) = call_as(&app, "s3cret", "/v1/zones").await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    s.library
        .lock()
        .unwrap()
        .set_setting("presence.visible.owner", "everyone")
        .unwrap();
    // The owner's browser queues a mixed list: a library row's cover link carries its sign-in
    let (st, _) = as_token(
        &app,
        "s3cret",
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"items": [
            {"uri": wav.to_string_lossy(), "title": "A", "image": "/v1/library/albums/1/cover?token=s3cret"},
            {"uri": "https://radio.example/live?token=theirs", "title": "B", "image": "https://cdn.example/b.jpg"}
        ], "play": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    // The queue kept the link, not the sign-in; a link that isn't the node's is as it was
    let (_, q) = call_as(&app, "s3cret", &format!("/v1/zones/{zid}/queue")).await;
    assert_eq!(q[0]["image"], "/v1/library/albums/1/cover", "{q}");
    assert_eq!(q[1]["uri"], "https://radio.example/live?token=theirs", "{q}");
    // Playing: who is listening carries the item
    let playing = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if s.zones
                .get(&pm_zones::ZoneId(zid.clone()))
                .unwrap()
                .state()
                .now_playing
                .is_some()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(playing.is_ok(), "the zone plays");
    for path in [
        "/v1/zones".to_string(),
        format!("/v1/zones/{zid}"),
        format!("/v1/zones/{zid}/queue"),
        "/v1/presence".to_string(),
    ] {
        let (st, body) = as_token(&app, &listener, "GET", &path, None).await;
        assert_eq!(st, StatusCode::OK, "{path}: {body}");
        assert!(body.contains("/v1/library/albums/1/cover"), "{path}: {body}");
        assert!(!body.contains("s3cret"), "{path}: {body}");
    }
    // A renderer's screen gets the keyed art link, which it can fetch
    let describe = s.describe_for_renderers();
    let mut item = pm_zones::QueueItem::new("https://radio.example/live");
    item.image = Some("/v1/library/albums/1/cover".into());
    assert_eq!(describe(&item).image, Some(s.art_url(1)));
    item.image = Some("/v1/library/tracks/1/file".into());
    assert_eq!(
        describe(&item).image,
        None,
        "no other node link reaches a renderer"
    );
    // The cover still loads by address, for the owner and for a listener
    for t in ["s3cret", listener.as_str()] {
        let (st, _) = call(
            &app,
            "GET",
            &format!("/v1/library/albums/1/cover?token={t}"),
            None,
        )
        .await;
        assert_eq!(
            st,
            StatusCode::NOT_FOUND,
            "no art kept, but past the sign-in check"
        );
    }
}

async fn call_as(app: &Router, token: &str, path: &str) -> (StatusCode, serde_json::Value) {
    let (st, body) = as_token(app, token, "GET", path, None).await;
    (st, serde_json::from_str(&body).unwrap_or_default())
}

/// A queue saved by an older browser holds sign-ins in its links: at start they are removed from
/// the saved queue, the owner's sessions end once, a listener's token in it ends, and the API
/// token in it is reported (it can only be replaced by hand).
#[tokio::test]
async fn a_saved_queue_with_a_sign_in_is_cleaned_at_start() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), Some("s3cret")).await;
    let app = router(s.clone());
    let listener = let_in(&s, LISTENER);
    let (_, zones) = call_as(&app, "s3cret", "/v1/zones").await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let key = format!("queue.{zid}");
    // As saved by this node, then with the links an older browser wrote
    let saved = serde_json::json!({
        "items": [
            {"id": 1, "uri": "track:1", "image": format!("/v1/library/albums/1/cover?token={listener}")},
            {"id": 2, "uri": "/v1/library/tracks/2/file?token=s3cret", "image": "/v1/library/albums/2/cover?token=pmo_browser"},
            {"id": 3, "uri": "https://radio.example/live?token=theirs"}
        ],
        "repeat": "off", "radio": false, "shuffle": false, "order": []
    });
    let sessions = |name: &str| {
        serde_json::json!([{"hash": blake3::hash(name.as_bytes()).to_hex().to_string(), "key": "owner", "created_at": 1}])
            .to_string()
    };
    {
        let lib = s.library.lock().unwrap();
        lib.set_setting(&key, &saved.to_string()).unwrap();
        lib.set_setting("accounts.owner_sessions", &sessions("pmo_browser"))
            .unwrap();
        lib.set_setting("accounts.device_sessions", &sessions("pmo_phone"))
            .unwrap();
    }
    let cleaned = accounts::clean_saved_queues(&s).unwrap();
    assert_eq!(
        cleaned,
        accounts::CleanedQueues {
            items: 2,
            owner_sessions_ended: true,
            api_token_found: true,
            listener_tokens_ended: 1,
        }
    );
    let setting = |k: &str| {
        s.library
            .lock()
            .unwrap()
            .get_setting(k)
            .unwrap()
            .unwrap_or_default()
    };
    let json = setting(&key);
    for t in ["s3cret", "pmo_browser", listener.as_str()] {
        assert!(!json.contains(t), "{t}: {json}");
    }
    assert!(json.contains("/v1/library/albums/1/cover\""), "{json}");
    assert!(
        json.contains("https://radio.example/live?token=theirs"),
        "not the node's link: {json}"
    );
    assert_eq!(setting("accounts.owner_sessions"), "[]");
    assert_eq!(setting("accounts.device_sessions"), "[]");
    let (st, _) = as_token(&app, &listener, "GET", "/v1/info", None).await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "the listener's token in the queue ended"
    );
    // Nothing left to clean: a second start ends nothing
    assert_eq!(
        accounts::clean_saved_queues(&s).unwrap(),
        accounts::CleanedQueues::default()
    );
    // The node starts again: the restored queue is the cleaned one
    drop(app);
    drop(s);
    let s2 = state(dir.path(), Some("s3cret")).await;
    let app2 = router(s2.clone());
    let (_, q) = call_as(&app2, "s3cret", &format!("/v1/zones/{zid}/queue")).await;
    assert_eq!(q[0]["image"], "/v1/library/albums/1/cover", "{q}");
    assert_eq!(q[1]["uri"], "/v1/library/tracks/2/file", "{q}");
    assert_eq!(q[2]["uri"], "https://radio.example/live?token=theirs", "{q}");
    assert!(!q.to_string().contains("s3cret"), "{q}");
}
