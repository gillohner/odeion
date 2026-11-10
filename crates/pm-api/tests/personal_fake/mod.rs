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
    /// Writes it will never take (a 400, 413 or 422): every collection write of a key here, and
    /// the bookmark of a URI here.
    pub rejects: Mutex<HashSet<String>>,
    /// Writes under /pub/ refused (a grant that lost its public folder).
    pub refuse_pub: AtomicBool,
    /// Listings/reads succeed, but the next private draft loses approval before landing.
    pub refuse_draft_once: AtomicBool,
    /// Listings/reads succeed, but the next private draft finds its local grant gone.
    pub lose_draft_once: AtomicBool,
    /// The node can't delete its copy of the grant.
    pub disconnect_fails: AtomicBool,
    /// Held (written) by a test: a bookmark write waits on it, keeping her person lock meanwhile.
    pub gate: tokio::sync::RwLock<()>,
    pub put_started: tokio::sync::Notify,
    pub list_started: tokio::sync::Notify,
    pub delete_started: tokio::sync::Notify,
    pub write_started: tokio::sync::Notify,
    pub publication_gate: tokio::sync::RwLock<()>,
    pub publication_started: tokio::sync::Notify,
    pub collection_list_gate: tokio::sync::RwLock<()>,
    pub collection_list_started: tokio::sync::Notify,
    pub read_gate: tokio::sync::RwLock<()>,
    pub read_started: tokio::sync::Notify,
    pub removal_gate: tokio::sync::RwLock<()>,
    pub remove_started: tokio::sync::Notify,
    pub remove_fails: AtomicBool,
    /// Listings fail (more than the node reads, or too slow): the node can't catch up.
    pub list_fails: AtomicBool,
    /// Only listings of all their posts fail (more than the node lists); one collection's own
    /// folders still list.
    pub whole_list_fails: AtomicBool,
    /// Files that can never be read (over the node's read cap), by path.
    pub unreadable: Mutex<HashSet<String>>,
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
        if (op == "put" || op == "delete") && self.rejects.lock().unwrap().contains(uri) {
            return Err(PersonalError::Rejected("422 Unprocessable Entity".into()));
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
    async fn disconnect(&self, key: &str) -> Result<(), String> {
        if self.disconnect_fails.load(Ordering::SeqCst) {
            return Err("read-only".into());
        }
        self.connected.lock().unwrap().remove(key);
        Ok(())
    }
    async fn bookmarks(
        &self,
        key: &str,
        _known: &[(String, i64)],
    ) -> Result<Vec<(String, i64)>, PersonalError> {
        self.list_started.notify_one();
        let _open = self.gate.read().await;
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
        self.put_started.notify_one();
        let _open = self.gate.read().await;
        Self::moment().await;
        self.check(key, "put", uri)?;
        let mut all = self.bookmarks.lock().unwrap();
        let mine = all.entry(key.to_string()).or_default();
        mine.retain(|(u, _)| u != uri);
        mine.push((uri.to_string(), created_at));
        Ok(())
    }
    async fn delete_bookmark(&self, key: &str, uri: &str) -> Result<(), PersonalError> {
        self.delete_started.notify_one();
        let _open = self.gate.read().await;
        Self::moment().await;
        self.check(key, "delete", uri)?;
        if let Some(mine) = self.bookmarks.lock().unwrap().get_mut(key) {
            mine.retain(|(u, _)| u != uri);
        }
        Ok(())
    }
    async fn list(&self, key: &str, dir: &str, _expected: bool) -> Result<Vec<String>, PersonalError> {
        self.collection_list_started.notify_one();
        let _open = self.collection_list_gate.read().await;
        Self::moment().await;
        self.check(key, "list", dir)?;
        if self.list_fails.load(Ordering::SeqCst)
            || (self.whole_list_fails.load(Ordering::SeqCst) && dir.ends_with("/social/v1/posts/"))
        {
            return Err(PersonalError::Unreachable("the listing was cut".into()));
        }
        Ok(self.paths(key, dir))
    }
    async fn read(&self, key: &str, path: &str) -> Result<Option<Vec<u8>>, PersonalError> {
        self.read_started.notify_one();
        let _open = self.read_gate.read().await;
        Self::moment().await;
        self.check(key, "read", path)?;
        if self.unreadable.lock().unwrap().contains(path) {
            return Err(PersonalError::Rejected("over the node's read cap".into()));
        }
        Ok(self
            .files
            .lock()
            .unwrap()
            .get(key)
            .and_then(|f| f.get(path).cloned()))
    }
    async fn write(&self, key: &str, path: &str, body: &serde_json::Value) -> Result<(), PersonalError> {
        self.write_started.notify_one();
        let _open = self.gate.read().await;
        let _publication = if path.starts_with("/pub/") {
            self.publication_started.notify_one();
            Some(self.publication_gate.read().await)
        } else {
            None
        };
        Self::moment().await;
        self.check(key, "write", path)?;
        if self.rejects.lock().unwrap().contains(key) {
            return Err(PersonalError::Rejected("413 Payload Too Large".into()));
        }
        if path.starts_with("/priv/") && self.refuse_draft_once.swap(false, Ordering::SeqCst) {
            return Err(PersonalError::Refused("403 Forbidden".into()));
        }
        if path.starts_with("/priv/") && self.lose_draft_once.swap(false, Ordering::SeqCst) {
            self.connected.lock().unwrap().remove(key);
            return Err(PersonalError::NoGrant);
        }
        if path.starts_with("/pub/") && self.refuse_pub.load(Ordering::SeqCst) {
            return Err(PersonalError::Refused("403 Forbidden".into()));
        }
        self.files
            .lock()
            .unwrap()
            .entry(key.to_string())
            .or_default()
            .insert(path.to_string(), serde_json::to_vec(body).unwrap());
        Ok(())
    }
    async fn remove(&self, key: &str, path: &str) -> Result<(), PersonalError> {
        self.remove_started.notify_one();
        let _open = self.removal_gate.read().await;
        Self::moment().await;
        self.check(key, "remove", path)?;
        if self.remove_fails.load(Ordering::SeqCst) {
            return Err(PersonalError::Unreachable("fixture: removal interrupted".into()));
        }
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
    node_on(dir, OutputRegistry::new(), vec![])
}

/// The library album of [`SONG_A`] in [`node_with_zone`].
pub const RELEASE_A: &str = "https://musicbrainz.org/release/833f00e1-781f-4edd-90e4-e52712618862";

/// The same node with a zone (`null-output`) and one library song, [`SONG_A`], on its album
/// [`RELEASE_A`] (tagged, scanned). The song's track id too.
pub async fn node_with_zone(dir: &std::path::Path) -> (Arc<NodeState>, Arc<Fake>, String, String, i64) {
    let music = dir.join("music");
    std::fs::create_dir_all(&music).unwrap();
    let wav = music.join("song.wav");
    let mut w = hound::WavWriter::create(
        &wav,
        hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for i in 0..2205 {
        w.write_sample((i % 50) as i16).unwrap();
        w.write_sample(-((i % 50) as i16)).unwrap();
    }
    w.finalize().unwrap();
    let mut tag = id3::Tag::new();
    use id3::TagLike;
    tag.set_title("Song");
    tag.set_artist("Tester");
    tag.set_album("Lists");
    for (d, v) in [
        ("MusicBrainz Track Id", SONG_A.rsplit('/').next().unwrap()),
        ("MusicBrainz Album Id", RELEASE_A.rsplit('/').next().unwrap()),
    ] {
        tag.add_frame(id3::frame::ExtendedText {
            description: d.into(),
            value: v.into(),
        });
    }
    tag.write_to_path(&wav, id3::Version::Id3v24).unwrap();
    let outputs = OutputRegistry::new();
    outputs.register(Arc::new(pm_zones::outputs::NullOutput::new()));
    let (s, fake, ann, bo) = node_on(dir, outputs, vec![music]);
    s.zones_for_all_outputs();
    let app = pm_api::router(s.clone());
    let (st, v) = call(
        &app,
        OWNER,
        "POST",
        "/v1/library/scan",
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    for _ in 0..500 {
        if !s.is_scanning() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let (_, found) = call(&app, OWNER, "GET", "/v1/library/tracks?limit=10", None).await;
    let track = found["tracks"][0]["id"]
        .as_i64()
        .unwrap_or_else(|| panic!("scanned: {found}"));
    (s, fake, ann, bo, track)
}

fn node_on(
    dir: &std::path::Path,
    outputs: OutputRegistry,
    scan_folders: Vec<std::path::PathBuf>,
) -> (Arc<NodeState>, Arc<Fake>, String, String) {
    let db = dir.join("lib.sqlite");
    let s = NodeState::new(
        NodeSettings {
            name: "t".into(),
            version: "0".into(),
            db_path: db.clone(),
            scan_folders,
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
        outputs,
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

/// The owner's bookmarks as the node's own tables hold them, and what its sync still has to push
/// (marked `unsynced:`).
pub fn owners_rows(s: &NodeState) -> Vec<String> {
    let lib = s.library.lock().unwrap();
    let mut v: Vec<String> = lib
        .list_bookmarks()
        .unwrap()
        .into_iter()
        .map(|r| r.uri)
        .chain(
            lib.unsynced_bookmarks()
                .unwrap()
                .into_iter()
                .map(|r| format!("unsynced:{}", r.uri)),
        )
        .collect();
    v.sort();
    v
}
