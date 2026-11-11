//! Deterministic delayed-request regressions through the real feedback and restore HTTP routes.
//! The capture gate is task-local and compiled only into this crate's unit-test binary.

use super::*;
use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use pm_audio::PipelineConfig;
use pm_library::Library;
use pm_zones::OutputRegistry;
use serde_json::{json, Value};
use tokio::sync::Notify;
use tower::ServiceExt;

const TOKEN: &str = "owner-fixture-token-report-restore";
const ANN: &str = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";
const BEN: &str = "8pinxxgqs41n4aididenw5apqp1urfmzdztr8jt4abrkdn435ewo";

fn node(dir: &Path) -> Arc<NodeState> {
    let db = dir.join("library.db");
    NodeState::new(
        crate::NodeSettings {
            name: "report-restore-test".into(),
            version: "0".into(),
            db_path: db.clone(),
            scan_folders: vec![],
            art_dir: dir.join("art"),
            mirror_dir: dir.join("homeserver"),
            pipeline: PipelineConfig::default(),
            token: Some(TOKEN.into()),
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
        OutputRegistry::new(),
    )
}

fn person(key: &str, access: &str, enabled: bool, membership: Option<&str>) -> Value {
    json!({"key": key, "access": access, "music_enabled": enabled,
        "added_at": 1, "reports": membership})
}

fn roster(s: &NodeState, people: Vec<Value>) {
    s.library
        .lock()
        .unwrap()
        .set_setting("people", &json!({"people": people}).to_string())
        .unwrap();
}

fn request(token: &str, method: &str, path: &str, body: Option<Value>) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(
            body.map(|v| Body::from(v.to_string()))
                .unwrap_or_else(Body::empty),
        )
        .unwrap()
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn report(text: &str) -> Value {
    json!({"kind": "problem", "text": text, "client_id": uuid::Uuid::new_v4().to_string()})
}

async fn restore(app: &axum::Router, bytes: Vec<u8>) {
    let req = Request::post("/v1/restore")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/octet-stream")
        .body(Body::from(bytes))
        .unwrap();
    let (status, response) = send(app, req).await;
    assert_eq!(status, StatusCode::OK, "{response}");
}

async fn delayed_request_across_restores(method: &str) {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path());
    // R0 has an explicit roster, not a missing setting that would carry the current roster over.
    roster(&s, vec![person(BEN, "music", true, None)]);
    let (r0, _) = crate::backup::make(&s, None).unwrap();
    let app = crate::router(s.clone());
    let (status, response) = send(
        &app,
        request(
            TOKEN,
            "POST",
            "/v1/people/invites",
            Some(json!({"access": "music", "key": ANN})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let a = crate::people::reports_of(&s, ANN).unwrap();
    let token = crate::accounts::mint(&s, ANN).unwrap();
    let (status, old) = send(
        &app,
        request(&token, "POST", "/v1/feedback", Some(report("retired A report"))),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{old}");
    let (r1, _) = crate::backup::make(&s, None).unwrap();

    let path = if method == "POST" {
        "/v1/feedback"
    } else {
        "/v1/feedback/mine"
    };
    let body = (method == "POST").then(|| report("delayed A report"));
    let req = request(&token, method, path, body);
    let captured = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let pending_app = app.clone();
    let pending = tokio::spawn(
        CAPTURE_GATE.scope((captured.clone(), resume.clone()), async move {
            send(&pending_app, req).await
        }),
    );
    // This is the exact boundary after membership capture and before opening the inbox. There
    // is no sleep, busy-timeout deadline, global hook or assumption about worker scheduling.
    tokio::time::timeout(std::time::Duration::from_secs(10), captured.notified())
        .await
        .expect("request reaches its captured-membership boundary");
    restore(&app, r0).await;
    assert!(!crate::people::may_use_music(&s, ANN));
    restore(&app, r1).await;
    let b = crate::people::reports_of(&s, ANN).unwrap();
    assert_ne!(a, b, "restore grants a fresh report membership");
    let authenticated = crate::accounts::caller(&s, Some(&token)).unwrap();
    assert_eq!(authenticated.role, Role::Listener);
    assert_eq!(authenticated.key.as_deref(), Some(ANN), "R1 reinstates token T");
    resume.notify_one();
    let (status, response) = pending.await.unwrap();
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{response}");
    assert!(!response.to_string().contains("retired A report"));
    assert!(!response.to_string().contains("delayed A report"));
    let kept = Inbox::open(dir.path()).unwrap().pending(now()).unwrap();
    assert_eq!(kept.len(), 1, "the stale POST saved nothing");
    assert_eq!(kept[0].id, old["id"].as_str().unwrap());

    // The same restored token can begin a new B request. It sees no retired A row, saves to B,
    // and that accepted report survives a real membership housekeeping sweep.
    let (status, mine) = send(&app, request(&token, "GET", "/v1/feedback/mine", None)).await;
    assert_eq!(status, StatusCode::OK, "{mine}");
    assert!(mine.as_array().unwrap().is_empty());
    let (status, fresh) = send(
        &app,
        request(&token, "POST", "/v1/feedback", Some(report("fresh B report"))),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{fresh}");
    let housekeeping = s.clone();
    let removed = tokio::task::spawn_blocking(move || {
        let _roster = housekeeping.roster.blocking_lock();
        forget_unlisted(
            &housekeeping.data_dir(),
            &crate::people::music_reporters(&housekeeping).unwrap(),
        )
        .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(removed, 1, "the retired A report goes");
    let (status, mine) = send(&app, request(&token, "GET", "/v1/feedback/mine", None)).await;
    assert_eq!(status, StatusCode::OK, "{mine}");
    assert_eq!(mine.as_array().unwrap().len(), 1);
    assert_eq!(mine[0]["id"], fresh["id"]);
    let kept = Inbox::open(dir.path()).unwrap().pending(now()).unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].text, "fresh B report");
}

#[tokio::test]
async fn a_delayed_post_cannot_use_a_retired_membership_after_restores() {
    delayed_request_across_restores("POST").await;
}

#[tokio::test]
async fn a_delayed_read_cannot_use_a_retired_membership_after_restores() {
    delayed_request_across_restores("GET").await;
}

#[tokio::test]
async fn legacy_membership_is_distinct_from_missing_disabled_or_unreadable_access() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path());
    let token = crate::accounts::mint(&s, ANN).unwrap();
    let caller = Caller {
        role: Role::Listener,
        key: Some(ANN.into()),
    };
    for (people, expected) in [
        (vec![person(ANN, "music", true, None)], Some(None)),
        (
            vec![person(ANN, "music", true, Some("named"))],
            Some(Some("named".into())),
        ),
        (vec![person(ANN, "music", false, None)], None),
        (vec![person(ANN, "homeserver", true, None)], None),
        (vec![person(BEN, "music", true, None)], None),
        (Vec::new(), None),
    ] {
        roster(&s, people);
        assert_eq!(membership_of(&s, &token, &caller), expected);
    }
    s.library
        .lock()
        .unwrap()
        .set_setting("people", "{not a roster")
        .unwrap();
    assert_eq!(membership_of(&s, &token, &caller), None);
    s.library.lock().unwrap().delete_setting("people").unwrap();
    assert_eq!(membership_of(&s, &token, &caller), None);
    roster(&s, vec![person(ANN, "music", true, None)]);
    assert_eq!(membership_of(&s, "pml_missing-token", &caller), None);
    let other = Caller {
        role: Role::Listener,
        key: Some(BEN.into()),
    };
    assert_eq!(membership_of(&s, &token, &other), None);
    s.library
        .lock()
        .unwrap()
        .set_setting("accounts.tokens", "{not tokens")
        .unwrap();
    assert_eq!(membership_of(&s, &token, &caller), None);
    let owner = Caller {
        role: Role::Owner,
        key: None,
    };
    assert_eq!(membership_of(&s, TOKEN, &owner), Some(None));
}
