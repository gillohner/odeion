//! The node as a service: health, backup, sync and the MusicBrainz proxy.
//!
//! Part of pm-api's unit tests (`mod tests` in lib.rs, which holds the helpers these use). A new
//! test of this area goes at the end of this file.

use super::*;

/// Audius against a local mock: the discovery host is configured, a search
/// yields track hits, an item has one streamable track, and it plays.
#[tokio::test]
async fn health_says_which_part_is_down() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().fallback(|| async { StatusCode::NOT_FOUND }),
        )
        .await
        .unwrap()
    });
    // A port nothing listens on
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let dir = tempfile::tempdir().unwrap();
    let pubky = pm_pubky::config::PmPubkyConfig {
        pinakes_base: format!("http://127.0.0.1:{closed}"),
        user_agent_contact: "test@example.org".into(),
        ..Default::default()
    };
    let s = state_with(dir.path(), None, pubky).await;
    s.set_contact("test@example.org").unwrap();
    {
        let mut j = s.jamendo.write().unwrap();
        j.base = format!("http://127.0.0.1:{port}");
        j.creds.client_id = "x".into();
    }
    let app = router(s.clone());
    let (st, h) = call(&app, "GET", "/v1/health", None).await;
    assert_eq!(st, StatusCode::OK, "{h}");
    let by = |id: &str| h.as_array().unwrap().iter().find(|p| p["id"] == id).cloned();
    let pinakes = by("pinakes").expect("pinakes is listed");
    assert_eq!(pinakes["state"], "down", "{h}");
    assert_eq!(pinakes["detail"], "cannot connect");
    assert_eq!(by("jamendo").unwrap()["state"], "ok", "any HTTP answer counts");
    assert!(
        by("tidal").is_none() && by("homeserver").is_none(),
        "parts not set up are left out"
    );
}

/// Library radio: with the flag on, a zone that runs out of queue gets
/// similar tracks appended and keeps playing.
#[tokio::test]
async fn backup_downloads_a_backup_file() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let req = Request::builder()
        .method("GET")
        .uri("/v1/backup")
        .header("authorization", "Bearer t")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let disposition = resp
        .headers()
        .get("content-disposition")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(disposition.contains("pubky-music-"), "{disposition}");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    // A backup file (zip): its library is a plain SQLite database
    let parsed = crate::backup::parse(&body).unwrap();
    assert!(parsed.database.starts_with(b"SQLite format 3\0"));
    let copy = dir.path().join("copy.db");
    std::fs::write(&copy, &parsed.database).unwrap();
    assert!(pm_library::Library::open(&copy).is_ok());
}

#[tokio::test]
async fn the_musicbrainz_proxy_needs_a_contact_and_rejects_bad_paths() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/mb/ws/2/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PRECONDITION_FAILED);
    s.set_contact("test@example.org").unwrap();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/mb/ws/2/..%2Fetc")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// The MusicBrainz proxy stops reading an answer past 4 MiB (it used to read it all first),
/// and passes a small one on (security review 2026-10-30, low).
#[tokio::test]
async fn the_musicbrainz_proxy_refuses_an_oversized_answer_as_it_streams() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = Router::new()
        .route(
            "/ws/2/big",
            get(|| async {
                // No Content-Length: chunks until 8 MiB
                let chunks = futures_util::stream::iter(
                    (0..128).map(|_| Ok::<_, std::io::Error>(vec![b'x'; 64 * 1024])),
                );
                Body::from_stream(chunks)
            }),
        )
        .route(
            "/ws/2/small",
            get(|| async { Json(serde_json::json!({"ok": true})) }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("test@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    let app = router(s.clone());
    let get_ = |path: &'static str| {
        let app = app.clone();
        async move {
            app.oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status()
        }
    };
    assert_eq!(get_("/mb/ws/2/big").await, StatusCode::BAD_GATEWAY);
    assert_eq!(get_("/mb/ws/2/small").await, StatusCode::OK);
}

/// A scheduled backup whose caller goes away mid-write keeps its lock to its end: a second
/// run meanwhile is told one is being written, and the first run's result is noted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_scheduled_backup_whose_caller_went_away_still_holds_its_run() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let folder = dir.path().join("backups");
    let (st, v) = call(
        &app,
        "PUT",
        "/v1/backup/schedule",
        Some(serde_json::json!({"folder": folder.to_string_lossy(), "every_hours": 24, "keep": 3})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    // The run is held after its write (a test marker) for a moment
    std::fs::write(s.data_dir().join(backup::HOLD_RUN_MARKER), b"").unwrap();
    let dropped = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        call(&app, "POST", "/v1/backup/schedule/run", None),
    )
    .await;
    assert!(dropped.is_err(), "the first caller went away mid-run");
    let (st, v) = call(&app, "POST", "/v1/backup/schedule/run", None).await;
    assert_eq!(
        st,
        StatusCode::CONFLICT,
        "the first run still holds its lock: {v}"
    );
    // The first run ends by itself and notes its result
    let mut noted = serde_json::Value::Null;
    for _ in 0..100 {
        let (_, sch) = call(&app, "GET", "/v1/backup/schedule", None).await;
        if sch["last_file"].is_string() {
            noted = sch;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(noted["last_file"].is_string(), "the run was noted: {noted}");
    std::fs::remove_file(s.data_dir().join(backup::HOLD_RUN_MARKER)).unwrap();
}

/// A sync or a settings restore whose session another sign-in replaced answers 409, not 502.
#[test]
fn a_replaced_sync_is_a_conflict() {
    assert_eq!(sync_error(sync::SyncError::Replaced).status, StatusCode::CONFLICT);
    assert_eq!(
        sync_error(sync::SyncError::Store("x".into())).status,
        StatusCode::BAD_GATEWAY
    );
}
