//! What the listener tests share: a node with an owner and two listeners (Ann and Bo), and a
//! fake homeserver standing in for theirs (`pm_api::personal::PersonalHomeserver`).
#![allow(dead_code)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pm_api::personal::{PersonalError, PersonalHomeserver};
use pm_api::{NodeSettings, NodeState};
use pm_audio::PipelineConfig;
use pm_library::Library;
use pm_zones::OutputRegistry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

pub const OWNER: &str = "owner-token";
pub const ANN: &str = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";
pub const BO: &str = "pxnu33x7jtpx9ar1ytsi4yxbp6a5o36gwhffs8zoxmbuptici1jy";
pub const SONG_A: &str = "https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69";
pub const SONG_B: &str = "https://musicbrainz.org/recording/0c9b1d6e-4e5f-4f3a-9c11-5b7d0a6e2f41";
pub const SONG_C: &str = "https://musicbrainz.org/recording/6f2b1c7e-1d2a-4b3c-8d4e-5f6a7b8c9d0e";

/// Each person's homeserver, as the node sees it through their grant.
#[derive(Default)]
pub struct Fake {
    pub bookmarks: Mutex<HashMap<String, Vec<(String, i64)>>>,
    pub connected: Mutex<HashSet<String>>,
    /// Their grant was revoked: every call is refused.
    pub refused: Mutex<HashSet<String>>,
    /// The homeserver doesn't answer.
    pub down: AtomicBool,
    /// Every call, as `key op uri`.
    pub calls: Mutex<Vec<String>>,
    /// Their files (collections), by key and path.
    pub files: Mutex<HashMap<String, BTreeMap<String, Vec<u8>>>>,
}

impl Fake {
    /// A homeserver takes a moment: requests made at once overlap, as they do for real.
    pub async fn moment() {
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
    }
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
    pub fn at(&self, key: &str, uri: &str) -> Option<i64> {
        self.bookmarks
            .lock()
            .unwrap()
            .get(key)
            .and_then(|b| b.iter().find(|(u, _)| u == uri).map(|(_, at)| *at))
    }
    /// The paths of `key`'s files under `dir`.
    pub fn paths(&self, key: &str, dir: &str) -> Vec<String> {
        self.files
            .lock()
            .unwrap()
            .get(key)
            .map(|f| f.keys().filter(|p| p.starts_with(dir)).cloned().collect())
            .unwrap_or_default()
    }
    /// The JSON of `key`'s file at `path`.
    pub fn json(&self, key: &str, path: &str) -> serde_json::Value {
        let files = self.files.lock().unwrap();
        serde_json::from_slice(&files[key][path]).unwrap()
    }
    pub fn of(&self, key: &str) -> Vec<String> {
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
    async fn bookmarks(
        &self,
        key: &str,
        _known: &[(String, i64)],
    ) -> Result<Vec<(String, i64)>, PersonalError> {
        Self::moment().await;
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
        Self::moment().await;
        self.check(key, "put", uri)?;
        let mut all = self.bookmarks.lock().unwrap();
        let mine = all.entry(key.to_string()).or_default();
        mine.retain(|(u, _)| u != uri);
        mine.push((uri.to_string(), created_at));
        Ok(())
    }
    async fn delete_bookmark(&self, key: &str, uri: &str) -> Result<(), PersonalError> {
        Self::moment().await;
        self.check(key, "delete", uri)?;
        if let Some(mine) = self.bookmarks.lock().unwrap().get_mut(key) {
            mine.retain(|(u, _)| u != uri);
        }
        Ok(())
    }
    async fn list(&self, key: &str, dir: &str) -> Result<Vec<String>, PersonalError> {
        Self::moment().await;
        self.check(key, "list", dir)?;
        Ok(self.paths(key, dir))
    }
    async fn read(&self, key: &str, path: &str) -> Result<Option<Vec<u8>>, PersonalError> {
        Self::moment().await;
        self.check(key, "read", path)?;
        Ok(self
            .files
            .lock()
            .unwrap()
            .get(key)
            .and_then(|f| f.get(path).cloned()))
    }
    async fn write(&self, key: &str, path: &str, body: &serde_json::Value) -> Result<(), PersonalError> {
        Self::moment().await;
        self.check(key, "write", path)?;
        self.files
            .lock()
            .unwrap()
            .entry(key.to_string())
            .or_default()
            .insert(path.to_string(), serde_json::to_vec(body).unwrap());
        Ok(())
    }
    async fn remove(&self, key: &str, path: &str) -> Result<(), PersonalError> {
        Self::moment().await;
        self.check(key, "remove", path)?;
        if let Some(f) = self.files.lock().unwrap().get_mut(key) {
            f.remove(path);
        }
        Ok(())
    }
}

pub async fn call(
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
pub fn node(dir: &std::path::Path) -> (Arc<NodeState>, Arc<Fake>, String, String) {
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

pub fn uris(v: &serde_json::Value) -> Vec<String> {
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
pub fn owners_rows(s: &NodeState) -> Vec<String> {
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
