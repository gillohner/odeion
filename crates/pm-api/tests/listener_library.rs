//! A listener's own bookmarks (TASK-listener-personal-library-1104, ADR 0016 step 5): on their
//! own homeserver, through the session they gave the node, never the owner's. The listener's
//! homeserver is a fake here (`pm_api::personal::PersonalHomeserver`); the real one is checked
//! against a testnet in testnet_sync.rs's style when `PM_TESTNET=1`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pm_api::personal::{PersonalError, PersonalHomeserver};
use pm_api::{router, NodeSettings, NodeState};
use pm_audio::PipelineConfig;
use pm_library::Library;
use pm_zones::OutputRegistry;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

const OWNER: &str = "owner-token";
const ANN: &str = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";
const BO: &str = "pxnu33x7jtpx9ar1ytsi4yxbp6a5o36gwhffs8zoxmbuptici1jy";
const SONG_A: &str = "https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69";
const SONG_B: &str = "https://musicbrainz.org/recording/0c9b1d6e-4e5f-4f3a-9c11-5b7d0a6e2f41";
const SONG_C: &str = "https://musicbrainz.org/recording/6f2b1c7e-1d2a-4b3c-8d4e-5f6a7b8c9d0e";

/// Each person's homeserver, as the node sees it through their grant.
#[derive(Default)]
struct Fake {
    bookmarks: Mutex<HashMap<String, Vec<(String, i64)>>>,
    connected: Mutex<HashSet<String>>,
    /// Their grant was revoked: every call is refused.
    refused: Mutex<HashSet<String>>,
    /// The homeserver doesn't answer.
    down: AtomicBool,
    /// Every call, as `key op uri`.
    calls: Mutex<Vec<String>>,
}

impl Fake {
    fn check(&self, key: &str, op: &str, uri: &str) -> Result<(), PersonalError> {
        self.calls.lock().unwrap().push(format!("{key} {op} {uri}"));
        if !self.connected.lock().unwrap().contains(key) {
            return Err(PersonalError::NoGrant);
        }
        if self.refused.lock().unwrap().contains(key) {
            return Err(PersonalError::Refused("401 Unauthorized".into()));
        }
        if self.down.load(Ordering::SeqCst) {
            return Err(PersonalError::Unreachable("connection refused".into()));
        }
        Ok(())
    }
    fn of(&self, key: &str) -> Vec<String> {
        let mut v: Vec<String> = self
            .bookmarks
            .lock()
            .unwrap()
            .get(key)
            .map(|b| b.iter().map(|(u, _)| u.clone()).collect())
            .unwrap_or_default();
        v.sort();
        v
    }
}

#[async_trait::async_trait]
impl PersonalHomeserver for Fake {
    fn connected(&self, key: &str) -> bool {
        self.connected.lock().unwrap().contains(key)
    }
    async fn disconnect(&self, key: &str) {
        self.connected.lock().unwrap().remove(key);
    }
    async fn bookmarks(&self, key: &str) -> Result<Vec<(String, i64)>, PersonalError> {
        self.check(key, "list", "")?;
        Ok(self
            .bookmarks
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .unwrap_or_default())
    }
    async fn put_bookmark(&self, key: &str, uri: &str, created_at: i64) -> Result<(), PersonalError> {
        self.check(key, "put", uri)?;
        let mut all = self.bookmarks.lock().unwrap();
        let mine = all.entry(key.to_string()).or_default();
        mine.retain(|(u, _)| u != uri);
        mine.push((uri.to_string(), created_at));
        Ok(())
    }
    async fn delete_bookmark(&self, key: &str, uri: &str) -> Result<(), PersonalError> {
        self.check(key, "delete", uri)?;
        if let Some(mine) = self.bookmarks.lock().unwrap().get_mut(key) {
            mine.retain(|(u, _)| u != uri);
        }
        Ok(())
    }
}

async fn call(
    app: &axum::Router,
    token: &str,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
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
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or_else(|_| serde_json::json!(String::from_utf8_lossy(&bytes))),
    )
}

/// A node with an owner token, two listeners (Ann and Bo) and a fake homeserver for them.
fn node(dir: &std::path::Path) -> (Arc<NodeState>, Arc<Fake>, String, String) {
    let db = dir.join("lib.sqlite");
    let s = NodeState::new(
        NodeSettings {
            name: "t".into(),
            version: "0".into(),
            db_path: db.clone(),
            scan_folders: vec![],
            art_dir: dir.join("art"),
            mirror_dir: dir.join("homeserver"),
            pipeline: PipelineConfig::default(),
            token: Some(OWNER.into()),
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
    );
    let roster = serde_json::json!({"people": [
        {"key": ANN, "access": "music", "music_enabled": true, "added_at": 0},
        {"key": BO, "access": "music", "music_enabled": true, "added_at": 0}]});
    s.library
        .lock()
        .unwrap()
        .set_setting("people", &roster.to_string())
        .unwrap();
    let fake = Arc::new(Fake::default());
    s.set_personal_homeserver(fake.clone());
    let ann = pm_api::accounts::mint(&s, ANN).unwrap();
    let bo = pm_api::accounts::mint(&s, BO).unwrap();
    (s, fake, ann, bo)
}

fn uris(v: &serde_json::Value) -> Vec<String> {
    let mut u: Vec<String> = v["uris"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    u.sort();
    u
}

/// The owner's bookmarks as the node's own tables hold them (what its sync pushes).
fn owners_rows(s: &NodeState) -> Vec<String> {
    let mut v: Vec<String> = s
        .library
        .lock()
        .unwrap()
        .list_bookmarks()
        .unwrap()
        .into_iter()
        .map(|r| r.uri)
        .collect();
    v.sort();
    v
}

#[tokio::test]
async fn each_person_bookmarks_on_their_own_homeserver() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, bo) = node(dir.path());
    fake.connected
        .lock()
        .unwrap()
        .extend([ANN.to_string(), BO.to_string()]);
    let app = router(s.clone());

    let (st, v) = call(
        &app,
        OWNER,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_A})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_B})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["bookmarked"], true, "{v}");
    let (st, v) = call(
        &app,
        &bo,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_C})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");

    // Each reads their own, and only that
    let (_, v) = call(&app, OWNER, "GET", "/v1/bookmarks", None).await;
    assert_eq!(uris(&v), vec![SONG_A], "the owner's: {v}");
    let (_, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert_eq!(uris(&v), vec![SONG_B], "Ann's: {v}");
    assert_eq!(v["homeserver"]["connected"], true, "{v}");
    let (_, v) = call(&app, &bo, "GET", "/v1/bookmarks", None).await;
    assert_eq!(uris(&v), vec![SONG_C], "Bo's: {v}");
    // By the older name too (the page and the offline app's sync use it)
    let (st, v) = call(&app, &bo, "GET", "/v1/favourites", None).await;
    assert_eq!((st, uris(&v)), (StatusCode::OK, vec![SONG_C.to_string()]), "{v}");

    // On their homeservers, through their own grant; the owner's tables hold the owner's only
    assert_eq!(fake.of(ANN), vec![SONG_B]);
    assert_eq!(fake.of(BO), vec![SONG_C]);
    assert_eq!(
        owners_rows(&s),
        vec![SONG_A],
        "a listener's bookmark never lands in the owner's"
    );
    assert!(
        !fake.calls.lock().unwrap().iter().any(|c| c.contains(SONG_A)),
        "the owner's bookmark never goes to a listener's homeserver"
    );

    // Removing: only the remover's own
    let q = format!("/v1/bookmarks?uri={}", urlencode(SONG_B));
    let (st, v) = call(&app, &ann, "DELETE", &q, None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["bookmarked"], false);
    assert!(fake.of(ANN).is_empty());
    let q = format!("/v1/bookmarks?uri={}", urlencode(SONG_A));
    let (st, _) = call(&app, &bo, "DELETE", &q, None).await;
    assert_eq!(
        st,
        StatusCode::OK,
        "Bo removing a bookmark he doesn't have is a no-op"
    );
    assert_eq!(owners_rows(&s), vec![SONG_A], "and never the owner's");
}

#[tokio::test]
async fn a_listener_without_their_homeserver_is_asked_to_connect() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    let app = router(s.clone());
    call(
        &app,
        OWNER,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_A})),
    )
    .await;

    let (st, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(uris(&v).is_empty(), "never the owner's: {v}");
    assert_eq!(v["homeserver"]["connected"], false, "{v}");
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_B})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["needs_connect"], true, "{v}");
    assert_eq!(owners_rows(&s), vec![SONG_A]);
    assert!(fake.of(ANN).is_empty());
    let (st, v) = call(&app, &ann, "GET", "/v1/me/homeserver", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["connected"], false, "{v}");
    // The owner's homeserver is the node's own: this is for listeners
    let (st, _) = call(&app, OWNER, "POST", "/v1/me/homeserver/connect", None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_refused_grant_is_lost_and_never_falls_back_to_the_owner() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_B})),
    )
    .await;
    assert_eq!(fake.of(ANN), vec![SONG_B]);

    // Ann revoked the node in Ring
    fake.refused.lock().unwrap().insert(ANN.to_string());
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_C})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["needs_connect"], true, "{v}");
    assert_eq!(v["lost"], true, "{v}");
    // What the node had of hers still reads, marked
    let (st, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(uris(&v), vec![SONG_B], "{v}");
    assert_eq!(v["homeserver"]["lost"], true, "{v}");
    let (_, v) = call(&app, &ann, "GET", "/v1/me/homeserver", None).await;
    assert_eq!(v["lost"], true, "{v}");
    assert!(owners_rows(&s).is_empty(), "nothing went to the owner's");
}

#[tokio::test]
async fn an_unreachable_homeserver_keeps_the_change_until_it_answers() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    fake.down.store(true, Ordering::SeqCst);
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_B})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["pending"], true, "{v}");
    let (_, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert_eq!(uris(&v), vec![SONG_B], "kept on the node meanwhile: {v}");
    assert_eq!(v["homeserver"]["pending"], 1, "{v}");
    assert!(fake.of(ANN).is_empty());
    // Back: the next read writes it
    fake.down.store(false, Ordering::SeqCst);
    let (_, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert_eq!(uris(&v), vec![SONG_B], "{v}");
    assert_eq!(v["homeserver"]["pending"], 0, "{v}");
    assert_eq!(fake.of(ANN), vec![SONG_B]);
    // A removal made while it is away too
    fake.down.store(true, Ordering::SeqCst);
    let q = format!("/v1/bookmarks?uri={}", urlencode(SONG_B));
    let (st, v) = call(&app, &ann, "DELETE", &q, None).await;
    assert_eq!(
        (st, v["pending"].clone()),
        (StatusCode::OK, serde_json::json!(true)),
        "{v}"
    );
    let (_, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert!(uris(&v).is_empty(), "{v}");
    fake.down.store(false, Ordering::SeqCst);
    call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert!(fake.of(ANN).is_empty());
    assert!(owners_rows(&s).is_empty());
}

#[tokio::test]
async fn disconnecting_and_removing_a_person() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_B})),
    )
    .await;
    let (st, v) = call(&app, &ann, "DELETE", "/v1/me/homeserver", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (_, v) = call(&app, &ann, "GET", "/v1/me/homeserver", None).await;
    assert_eq!(v["connected"], false, "{v}");
    assert_eq!(
        fake.of(ANN),
        vec![SONG_B],
        "disconnecting deletes nothing on her homeserver"
    );
    // People › Remove: her grant goes too
    fake.connected.lock().unwrap().insert(ANN.to_string());
    pm_api::accounts::forget_person(&s, ANN).unwrap();
    // (forget_person is sync; the grant goes with it, the fake hears it in the background)
    for _ in 0..50 {
        if !fake.connected(ANN) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(!fake.connected(ANN), "removed people keep no grant");
}

#[tokio::test]
async fn a_listener_reaches_only_their_own_bookmark_routes() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    for (m, p) in [
        ("GET", "/v1/playlists"),
        ("POST", "/v1/playlists"),
        ("GET", "/v1/tags"),
        ("PUT", "/v1/collections"),
        ("GET", "/v1/me/listens"),
    ] {
        let (st, v) = call(
            &app,
            &ann,
            m,
            p,
            Some(serde_json::json!({"name": "x", "uri": SONG_B})),
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{m} {p}: {v}");
    }
    assert!(
        fake.calls.lock().unwrap().is_empty(),
        "{:?}",
        fake.calls.lock().unwrap()
    );
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}
