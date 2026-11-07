//! A zone that plays a stream whose server never answers takes Stop and Next while the open
//! is still waiting, and a pass over several such items ends at the Stop. Its own process:
//! the stream wait is shortened for tests before the node's stream client is first built.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pm_api::{router, NodeSettings, NodeState};
use pm_audio::PipelineConfig;
use pm_library::Library;
use pm_zones::outputs::NullOutput;
use pm_zones::OutputRegistry;
use std::sync::Arc;
use tower::ServiceExt;

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(match body {
            Some(b) => Body::from(b.to_string()),
            None => Body::empty(),
        })
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

/// A server that accepts every connection, reads the request and never answers.
fn silent_server() -> String {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/live", l.local_addr().unwrap());
    std::thread::spawn(move || {
        for c in l.incoming().flatten() {
            std::thread::spawn(move || {
                use std::io::Read;
                let mut c = c;
                let mut buf = [0u8; 4096];
                let _ = c.read(&mut buf);
                std::thread::sleep(std::time::Duration::from_secs(120));
            });
        }
    });
    url
}

async fn transport_is(app: &axum::Router, zid: &str, want: &str, within: std::time::Duration) -> bool {
    let t = std::time::Instant::now();
    while t.elapsed() < within {
        let (_, z) = call(app, "GET", &format!("/v1/zones/{zid}"), None).await;
        if z["transport"] == want {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_zone_on_a_stream_that_never_answers_takes_stop_and_next() {
    pm_api::sources::STREAM_WAIT_MS_FOR_TESTS.store(1_000, std::sync::atomic::Ordering::Relaxed);
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("lib.sqlite");
    let outputs = OutputRegistry::new();
    outputs.register(Arc::new(NullOutput::new()));
    let s = NodeState::new(
        NodeSettings {
            name: "t".into(),
            version: "0".into(),
            db_path: db.clone(),
            scan_folders: vec![],
            art_dir: dir.path().join("art"),
            mirror_dir: dir.path().join("homeserver"),
            pipeline: PipelineConfig::default(),
            token: None,
            subsonic_user: "pubky".into(),
            pubky: Default::default(),
            public_url: "http://127.0.0.1:7788".into(),
            upnp: false,
            cast: false,
            slimproto_port: 0,
            session_secret_path: None,
            tidal: Default::default(),
            spotify: Default::default(),
            acoustid_key: String::new(),
            jamendo: Default::default(),
        },
        Library::open(&db).unwrap(),
        outputs,
    );
    s.zones_for_all_outputs();
    let app = router(s.clone());
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let url = silent_server();
    // Three streams that never answer: each open costs the (test-sized) wait
    let items: Vec<serde_json::Value> = (0..3)
        .map(|i| serde_json::json!({"uri": format!("{url}?{i}"), "title": format!("Dead {i}")}))
        .collect();
    let (st, r) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"items": items, "play": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    // Stop while the first open is still waiting: taken before the other two are tried
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let t = std::time::Instant::now();
    let (st, r) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/transport"),
        Some(serde_json::json!({"action": "stop"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert!(
        transport_is(&app, &zid, "stopped", std::time::Duration::from_millis(2_500)).await,
        "Stop is taken within one wait, not three"
    );
    assert!(
        t.elapsed() < std::time::Duration::from_millis(2_500),
        "{:?}",
        t.elapsed()
    );
    // Play again (the pass starts over), then Next while the first open waits: the pass ends
    // there and the zone goes on from the next item; nothing answers, so it ends stopped with
    // the reason within a few waits rather than never
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/transport"),
        Some(serde_json::json!({"action": "play"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/transport"),
        Some(serde_json::json!({"action": "next"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        transport_is(&app, &zid, "stopped", std::time::Duration::from_secs(6)).await,
        "the pass ends"
    );
    let (_, z) = call(&app, "GET", &format!("/v1/zones/{zid}"), None).await;
    assert!(
        z["error"]
            .as_str()
            .unwrap_or("")
            .contains("nothing in the queue could be played"),
        "{z}"
    );
}
