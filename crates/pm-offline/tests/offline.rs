//! A device syncing with a stand-in node, then playing offline from its local server.

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use axum::{Json, Router};
use pm_offline::{Event, Keep, Node, Store, SyncOptions};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Seen {
    plays: Vec<Value>,
    hearts_off: Vec<String>,
    ranges: Vec<(i64, Option<String>)>,
    manifests: Vec<Value>,
    devices: Vec<Value>,
    /// The node's id in `/v1/info` ("n1" when empty).
    node_id: String,
    /// Added to the first byte a 206 says it starts at (a node that answers another range).
    range_shift: usize,
    /// What `/v1/playlists/{id}` answers instead of the playlist.
    playlist_status: Option<u16>,
    /// A song the node takes long to answer for (a slow transcode).
    slow: Option<i64>,
    /// A stale manifest or an unknown-length file response.
    manifest_bytes: Option<u64>,
    chunked: bool,
}

type Fake = Arc<Mutex<Seen>>;

fn song(id: i64) -> Vec<u8> {
    format!("song-{id}-").repeat(100).into_bytes()
}

/// Album 7 holds songs 1 and 2.
fn kept_by(keep: &Value) -> Vec<i64> {
    let mut ids: Vec<i64> = keep["tracks"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default();
    if keep["albums"].as_array().is_some_and(|a| a.contains(&json!(7))) {
        ids.extend([1, 2]);
    }
    ids.sort();
    ids.dedup();
    ids
}

async fn manifest(State(f): State<Fake>, Json(b): Json<Value>) -> Json<Value> {
    f.lock().unwrap().manifests.push(b.clone());
    let manifest_bytes = f.lock().unwrap().manifest_bytes;
    let kept = kept_by(&b["keep"]);
    let have: Vec<i64> = b["have"]
        .as_array()
        .map(|a| a.iter().filter_map(|h| h["track_id"].as_i64()).collect())
        .unwrap_or_default();
    let wanted: Vec<Value> = kept
        .iter()
        .filter(|i| !have.contains(i))
        .map(|i| json!({"track_id": i, "title": format!("Song {i}"), "bytes": manifest_bytes.unwrap_or(song(*i).len() as u64)}))
        .collect();
    let unkept: Vec<i64> = have.iter().copied().filter(|i| !kept.contains(i)).collect();
    Json(json!({"wanted": wanted, "unkept": unkept}))
}

async fn track(Path(id): Path<i64>) -> Response {
    if id != 1 && id != 2 {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "no such track"}))).into_response();
    }
    Json(json!({
        "id": id, "title": format!("Song {id}"), "track_no": id, "disc_no": 1, "duration_ms": 1000,
        "artists": [{"id": 3, "name": "Artist"}],
        "album": {"id": 7, "title": "Album"},
        "file": {"path": format!("/srv/Owner's music/0{id}.flac"), "container": "flac", "size_bytes": song(id).len()}
    }))
    .into_response()
}

async fn download(State(f): State<Fake>, Path(id): Path<i64>, h: HeaderMap) -> Response {
    let range = h
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let slow = {
        let mut s = f.lock().unwrap();
        s.ranges.push((id, range.clone()));
        s.slow == Some(id)
    };
    if slow {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    let body = song(id);
    let etag = format!("\"e{id}\"");
    let if_range_ok = h.get(header::IF_RANGE).and_then(|v| v.to_str().ok()) == Some(etag.as_str());
    if let (Some(r), true) = (range, if_range_ok) {
        let from: usize = r
            .trim_start_matches("bytes=")
            .trim_end_matches('-')
            .parse()
            .unwrap();
        let said = from + f.lock().unwrap().range_shift;
        let len = body.len();
        return (
            StatusCode::PARTIAL_CONTENT,
            [
                (header::ETAG, etag),
                (header::CONTENT_RANGE, format!("bytes {said}-{}/{len}", len - 1)),
            ],
            body[from..].to_vec(),
        )
            .into_response();
    }
    if f.lock().unwrap().chunked {
        let (mut writer, reader) = tokio::io::duplex(200);
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let _ = writer.write_all(&body).await;
        });
        return (
            [(header::ETAG, etag)],
            axum::body::Body::from_stream(tokio_util::io::ReaderStream::with_capacity(reader, 200)),
        )
            .into_response();
    }
    ([(header::ETAG, etag)], body).into_response()
}

async fn history(State(f): State<Fake>, Json(b): Json<Value>) -> Response {
    if b["track_id"] == 555 {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "track 555"}))).into_response();
    }
    f.lock().unwrap().plays.push(b);
    Json(json!({"plays": []})).into_response()
}

fn fake_node(f: Fake) -> Router {
    Router::new()
        .route(
            "/",
            get(|| async { ([(header::CONTENT_TYPE, "text/html")], "<html>the web UI</html>") }),
        )
        .route("/app.js", get(|| async { "console.log('app')" }))
        .route(
            "/v1/info",
            get(|State(f): State<Fake>| async move {
                let id = match f.lock().unwrap().node_id.as_str() {
                    "" => "n1".to_string(),
                    id => id.to_string(),
                };
                Json(json!({"id": id, "name": "Home", "version": "0.1.0"}))
            }),
        )
        .route(
            "/v1/me",
            get(|| async { Json(json!({"role": "owner", "zones": []})) }),
        )
        .route(
            "/v1/devices/{id}",
            put(|State(f): State<Fake>, Json(b): Json<Value>| async move {
                f.lock().unwrap().devices.push(b);
                Json(json!({}))
            }),
        )
        .route("/v1/devices/{id}/downloads", put(manifest))
        .route("/v1/library/tracks/{id}", get(track))
        .route("/v1/library/tracks/{id}/download", get(download))
        .route(
            "/v1/library/albums/{id}/cover",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "image/png")],
                    vec![0x89u8, b'P', b'N', b'G'],
                )
            }),
        )
        .route(
            "/v1/playlists/{id}",
            get(|State(f): State<Fake>, Path(id): Path<String>| async move {
                if let Some(code) = f.lock().unwrap().playlist_status {
                    let code = StatusCode::from_u16(code).unwrap();
                    return (code, Json(json!({"error": "no"}))).into_response();
                }
                Json(json!({"summary": {"id": id, "name": "Mix"}, "tracks": [{"id": 2}, {"id": 99}]}))
                    .into_response()
            }),
        )
        .route(
            "/v1/favourites",
            get(|| async {
                Json(json!({"tracks": [{"id": 1}], "albums": [], "artists": [], "other": [], "uris": []}))
            })
            .delete(|State(f): State<Fake>, uri: axum::http::Uri| async move {
                f.lock()
                    .unwrap()
                    .hearts_off
                    .push(uri.query().unwrap_or_default().to_string());
                Json(json!({"favourite": false}))
            }),
        )
        .route("/v1/library/history", axum::routing::post(history))
        .with_state(f)
}

async fn start_node(f: Fake) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, fake_node(f)).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test]
async fn a_device_syncs_then_plays_offline() {
    let seen: Fake = Arc::default();
    let base = start_node(seen.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(dir.path()).unwrap());
    store
        .set_keep(Keep {
            albums: vec![7],
            playlists: vec!["p1".into()],
            favourites: true,
            ..Default::default()
        })
        .unwrap();
    // A download of song 2 that stopped after 300 bytes
    std::fs::write(dir.path().join("files/2.flac.part"), &song(2)[..300]).unwrap();
    std::fs::write(dir.path().join("files/2.flac.etag"), "\"e2\"").unwrap();
    // Done offline before this sync
    store.push(Event::Play { track_id: 1, at: 100 }).unwrap();
    store
        .push(Event::Play {
            track_id: 555,
            at: 101,
        })
        .unwrap();
    store
        .push(Event::Favourite {
            track_id: 1,
            on: false,
            at: 102,
        })
        .unwrap();

    let node = Node::new(&base, "tok").unwrap();
    let opts = SyncOptions {
        name: "Pixel 8".into(),
        kind: "android".into(),
        ..Default::default()
    };
    let r = pm_offline::sync(&store, &node, &opts).await.unwrap();
    assert_eq!(r.sent, 3, "two sent, one refused for good (an unknown song)");
    assert_eq!(r.downloaded, [1, 2]);
    assert!(r.failed.is_empty(), "{:?}", r.failed);

    // The files, whole; song 2 resumed from byte 300
    assert_eq!(std::fs::read(dir.path().join("files/1.flac")).unwrap(), song(1));
    assert_eq!(std::fs::read(dir.path().join("files/2.flac")).unwrap(), song(2));
    {
        let s = seen.lock().unwrap();
        assert!(
            s.ranges.contains(&(2, Some("bytes=300-".into()))),
            "{:?}",
            s.ranges
        );
        assert!(s.ranges.contains(&(1, None)));
        assert_eq!(
            s.plays,
            [json!({"track_id": 1, "zone": "offline", "played_at": 100})]
        );
        assert_eq!(s.hearts_off, ["track_id=1&at=102"], "with the time of the change");
        assert_eq!(s.devices, [json!({"name": "Pixel 8", "kind": "android"})]);
        let last = s.manifests.last().unwrap();
        assert_eq!(
            last["have"].as_array().unwrap().len(),
            2,
            "the node hears what the device has"
        );
        assert_eq!(last["keep"]["albums"], json!([7]));
    }
    assert!(store.queued().is_empty());
    let ix = store.index();
    assert_eq!(
        ix.tracks[&1].track["file"]["path"], "01.flac",
        "no owner's folders on the device"
    );
    assert_eq!(ix.covers[&7], "7.png");
    assert!(ix.playlists.contains_key("p1"));
    assert!(ix.favourite_changes.is_empty());
    assert!(ix.synced_at.is_some());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("ui/index.html")).unwrap(),
        "<html>the web UI</html>"
    );

    // Offline: the local server
    let (addr, _task) = pm_offline::server::serve(store.clone()).await.unwrap();
    let local = format!("http://{addr}");
    let http = reqwest::Client::new();
    let info: Value = http
        .get(format!("{local}/v1/info"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(info["offline"], true);
    assert_eq!(info["name"], "Home");
    assert_eq!(info["library"]["tracks"], 2);
    let off: Value = http
        .get(format!("{local}/v1/offline"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(off["albums"][0]["track_ids"], json!([1, 2]));
    assert_eq!(off["albums"][0]["cover"], true);
    assert_eq!(off["playlists"][0]["name"], "Mix");
    assert_eq!(
        off["playlists"][0]["track_ids"],
        json!([2]),
        "only what the device has"
    );
    assert_eq!(off["favourites"]["track_ids"], json!([1]));
    let part = http
        .get(format!("{local}/v1/library/tracks/2/file?token=x"))
        .header(header::RANGE, "bytes=0-9")
        .send()
        .await
        .unwrap();
    assert_eq!(part.status(), 206);
    assert_eq!(
        part.headers()[header::CONTENT_RANGE],
        format!("bytes 0-9/{}", song(2).len())
    );
    assert_eq!(part.bytes().await.unwrap().as_ref(), &song(2)[..10]);
    let played = http
        .post(format!("{local}/v1/library/history"))
        .json(&json!({"track_id": 2, "zone": "browser"}))
        .send()
        .await
        .unwrap();
    assert_eq!(played.status(), 200);
    let hearted = http
        .put(format!("{local}/v1/favourites"))
        .json(&json!({"track_id": 2}))
        .send()
        .await
        .unwrap();
    assert_eq!(hearted.status(), 200);
    assert!(store.index().is_favourite(2));
    assert_eq!(store.queued().len(), 2);
    let page = http.get(format!("{local}/albums")).send().await.unwrap();
    assert_eq!(
        page.text().await.unwrap(),
        "<html>the web UI</html>",
        "the UI's routes get the page"
    );
    let away = http
        .post(format!("{local}/v1/zones/z/queue"))
        .send()
        .await
        .unwrap();
    assert_eq!(away.status(), 503);

    // Back online, keeping only song 1: song 2 goes, and the queue is sent
    store
        .set_keep(Keep {
            tracks: vec![1],
            ..Default::default()
        })
        .unwrap();
    let r = pm_offline::sync(&store, &node, &opts).await.unwrap();
    assert_eq!(r.sent, 2);
    assert_eq!(r.removed, [2]);
    assert!(!dir.path().join("files/2.flac").exists());
    assert_eq!(seen.lock().unwrap().plays.len(), 2);
    assert_eq!(store.index().favourites, None, "favourites no longer kept");
}

#[tokio::test]
async fn an_unreachable_node_keeps_the_queue() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.push(Event::Play { track_id: 1, at: 1 }).unwrap();
    // Nothing listens there
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    drop(l);
    let node = Node::new(&base, "tok").unwrap();
    let opts = SyncOptions {
        name: "Laptop".into(),
        kind: "desktop".into(),
        ..Default::default()
    };
    assert!(matches!(
        pm_offline::sync(&store, &node, &opts).await,
        Err(pm_offline::SyncError::Unreachable(_))
    ));
    assert_eq!(store.queued().len(), 1);
}

fn opts() -> SyncOptions {
    SyncOptions {
        name: "Laptop".into(),
        kind: "desktop".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn another_node_gets_nothing_from_this_store() {
    let seen: Fake = Arc::default();
    let base = start_node(seen.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let node = Node::new(&base, "tok").unwrap();
    pm_offline::sync(&store, &node, &opts()).await.unwrap();
    assert_eq!(
        store.index().node_id,
        "n1",
        "the first sync ties the store to its node"
    );
    store.push(Event::Play { track_id: 1, at: 5 }).unwrap();
    store
        .set_keep(Keep {
            albums: vec![7],
            ..Default::default()
        })
        .unwrap();

    // The same address now answers as another node (a friend's, or a reinstall)
    let before = {
        let mut s = seen.lock().unwrap();
        s.node_id = "n2".into();
        (s.manifests.len(), s.devices.len(), s.plays.len())
    };
    let r = pm_offline::sync(&store, &node, &opts()).await;
    assert!(
        matches!(&r, Err(pm_offline::SyncError::OtherNode { kept, node }) if kept == "n1" && node == "n2"),
        "{r:?}"
    );
    assert!(matches!(
        pm_offline::flush(&store, &node).await,
        Err(pm_offline::SyncError::OtherNode { .. })
    ));
    {
        let s = seen.lock().unwrap();
        assert_eq!(
            (s.manifests.len(), s.devices.len(), s.plays.len()),
            before,
            "no plays, hearts, keeps or device"
        );
    }
    assert_eq!(store.queued().len(), 1, "kept for its own node");
    assert_eq!(
        pm_offline::node_dir(dir.path(), "n2"),
        dir.path().join("n2"),
        "the app keeps each node in its own folder"
    );
}

#[tokio::test]
async fn a_download_is_kept_only_when_whole() {
    let seen: Fake = Arc::default();
    let base = start_node(seen.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .set_keep(Keep {
            tracks: vec![1, 2],
            ..Default::default()
        })
        .unwrap();
    // Song 2 stopped after 300 bytes, and the node answers the resume with another range
    std::fs::write(dir.path().join("files/2.flac.part"), &song(2)[..300]).unwrap();
    std::fs::write(dir.path().join("files/2.flac.etag"), "\"e2\"").unwrap();
    seen.lock().unwrap().range_shift = 10;
    let node = Node::new(&base, "tok").unwrap();
    let r = pm_offline::sync(&store, &node, &opts()).await.unwrap();
    assert_eq!(r.downloaded, [1]);
    assert_eq!(r.failed.len(), 1, "{:?}", r.failed);
    assert_eq!(r.failed[0].0, 2);
    assert!(!dir.path().join("files/2.flac").exists(), "never a corrupt file");
    assert!(!store.index().tracks.contains_key(&2));

    // Next time the whole file comes again
    seen.lock().unwrap().range_shift = 0;
    let r = pm_offline::sync(&store, &node, &opts()).await.unwrap();
    assert_eq!(r.downloaded, [2]);
    assert_eq!(std::fs::read(dir.path().join("files/2.flac")).unwrap(), song(2));

    // A file the system cleared is no longer reported, so the node asks for it again
    std::fs::remove_file(dir.path().join("files/1.flac")).unwrap();
    let r = pm_offline::sync(&store, &node, &opts()).await.unwrap();
    assert_eq!(r.downloaded, [1]);
    let s = seen.lock().unwrap();
    let first = &s.manifests[s.manifests.len() - 2];
    let have: Vec<i64> = first["have"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["track_id"].as_i64().unwrap())
        .collect();
    assert_eq!(have, [2]);
    assert_eq!(std::fs::read(dir.path().join("files/1.flac")).unwrap(), song(1));
}

#[tokio::test]
async fn a_kept_playlist_goes_only_when_deleted() {
    let seen: Fake = Arc::default();
    let base = start_node(seen.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .set_keep(Keep {
            playlists: vec!["p1".into()],
            ..Default::default()
        })
        .unwrap();
    let node = Node::new(&base, "tok").unwrap();
    pm_offline::sync(&store, &node, &opts()).await.unwrap();
    assert!(store.index().playlists.contains_key("p1"));
    for code in [500, 401, 403] {
        seen.lock().unwrap().playlist_status = Some(code);
        pm_offline::sync(&store, &node, &opts()).await.unwrap();
        assert!(store.index().playlists.contains_key("p1"), "kept after {code}");
    }
    seen.lock().unwrap().playlist_status = Some(404);
    pm_offline::sync(&store, &node, &opts()).await.unwrap();
    assert!(
        store.index().playlists.is_empty(),
        "gone when deleted on the node"
    );
}

/// A raw request, to send any `Host` and path.
async fn raw_get(addr: std::net::SocketAddr, path: &str, host: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut c = tokio::net::TcpStream::connect(addr).await.unwrap();
    c.write_all(format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut out = String::new();
    c.read_to_string(&mut out).await.unwrap();
    out
}

#[tokio::test]
async fn the_local_server_answers_only_itself_and_its_files() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(dir.path()).unwrap());
    std::fs::write(dir.path().join("ui/index.html"), "<html>the web UI</html>").unwrap();
    std::fs::write(dir.path().join("ui/app.js"), "console.log('app')").unwrap();
    std::fs::write(dir.path().join("ui/other.txt"), "not a UI file").unwrap();
    store.push(Event::Play { track_id: 1, at: 1 }).unwrap();
    let (addr, _task) = pm_offline::server::serve(store).await.unwrap();
    let me = format!("127.0.0.1:{}", addr.port());

    let ok = raw_get(addr, "/app.js", &me).await;
    assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
    assert!(ok.ends_with("console.log('app')"));
    let local = raw_get(addr, "/v1/info", &format!("localhost:{}", addr.port())).await;
    assert!(local.starts_with("HTTP/1.1 200"), "{local}");
    // Only the UI's own files; anything else is the page
    for path in [
        "/other.txt",
        "/../queue.jsonl",
        "/C:queue.jsonl",
        "/%2e%2e/index.json",
        "/ui/app.js",
    ] {
        let r = raw_get(addr, path, &me).await;
        assert!(r.ends_with("<html>the web UI</html>"), "{path}: {r}");
    }
    // A page that rebinds its own name to 127.0.0.1
    for path in ["/v1/offline", "/v1/me", "/"] {
        let r = raw_get(addr, path, &format!("evil.example:{}", addr.port())).await;
        assert!(r.starts_with("HTTP/1.1 403"), "{path}: {r}");
    }
    let posted = reqwest::Client::new()
        .post(format!("http://{me}/v1/library/history"))
        .header(header::ORIGIN, "https://evil.example")
        .json(&json!({"track_id": 1}))
        .send()
        .await
        .unwrap();
    assert_eq!(posted.status(), 403);
}

#[tokio::test]
async fn a_slow_transcode_fails_only_its_song() {
    let seen: Fake = Arc::default();
    let base = start_node(seen.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .set_keep(Keep {
            tracks: vec![1, 2],
            ..Default::default()
        })
        .unwrap();
    seen.lock().unwrap().slow = Some(1);
    let wait = std::time::Duration::from_millis(300);
    let node = Node::new(&base, "tok")
        .unwrap()
        .with_download_timeouts(wait, wait);
    let r = pm_offline::sync(&store, &node, &opts()).await.unwrap();
    assert_eq!(r.downloaded, [2], "the sync goes on");
    assert_eq!(r.failed.len(), 1, "{:?}", r.failed);
    assert_eq!(r.failed[0].0, 1);
    assert!(r.failed[0].1.contains("timed out"), "{:?}", r.failed);
    // Once the node is quicker, the song comes
    seen.lock().unwrap().slow = None;
    let r = pm_offline::sync(&store, &node, &opts()).await.unwrap();
    assert_eq!(r.downloaded, [1]);
}

/// The page names the install script and its styles (pm-api pwa.js, pwa.css): the device keeps
/// them with the page, so its offline copy loads them as what they are, not as the page.
#[tokio::test]
async fn the_offline_page_keeps_its_install_script() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(dir.path()).unwrap());
    std::fs::write(dir.path().join("ui/index.html"), "<html>the web UI</html>").unwrap();
    std::fs::write(dir.path().join("ui/pwa.js"), "console.log('pwa')").unwrap();
    std::fs::write(dir.path().join("ui/pwa.css"), ".install-btn{}").unwrap();
    let (addr, _task) = pm_offline::server::serve(store).await.unwrap();
    let me = format!("127.0.0.1:{}", addr.port());
    for (path, body, mime) in [
        ("/pwa.js", "console.log('pwa')", "text/javascript"),
        ("/pwa.css", ".install-btn{}", "text/css"),
    ] {
        let r = raw_get(addr, path, &me).await;
        assert!(r.starts_with("HTTP/1.1 200"), "{path}: {r}");
        assert!(
            r.to_lowercase().contains(&format!("content-type: {mime}")),
            "{path}: {r}"
        );
        assert!(r.ends_with(body), "{path}: {r}");
    }
}

/// The limit the person set: a sync downloads what fits, stops before the next song, says why
/// and where it stood; removing something (stop keeping it) frees room for the rest. Keeping and
/// letting go are the device's own: nothing on the node changes but the device's record.
#[tokio::test]
async fn a_sync_stops_at_the_limit_and_says_so() {
    use pm_offline::{KeepItem, Label, Progress, ProgressSink};
    let seen: Fake = Arc::default();
    let base = start_node(seen.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let label = Label {
        title: "Album".into(),
        sub: "Artist".into(),
        songs: Some(2),
    };
    assert!(store.keep(&KeepItem::Album(7), true, Some(label)).unwrap());
    let told: Arc<Mutex<Vec<Progress>>> = Arc::default();
    let t2 = told.clone();
    let one = song(1).len() as u64;
    let capped = SyncOptions {
        cap_bytes: Some(one + 10),
        progress: Some(ProgressSink(Arc::new(move |p| {
            t2.lock().unwrap().push(p.clone())
        }))),
        ..opts()
    };
    let node = Node::new(&base, "tok").unwrap();
    let r = pm_offline::sync(&store, &node, &capped).await.unwrap();
    assert_eq!(r.downloaded, [1]);
    let why = r.full.clone().expect("stopped at the limit");
    assert!(why.contains("limit"), "{why}");
    let last = told.lock().unwrap().last().cloned().unwrap();
    assert_eq!(
        (last.running, last.wanted, last.done, last.full.as_deref()),
        (false, 2, 1, Some(why.as_str()))
    );
    assert!(told
        .lock()
        .unwrap()
        .iter()
        .any(|p| p.current.as_deref() == Some("Song 1")));
    let s = pm_offline::summary(&store.index());
    assert_eq!(
        (s.items[0].here, s.items[0].bytes, s.track_ids.clone()),
        (1, one, vec![1])
    );

    // A disk that would be left too full stops it the same way
    let full = SyncOptions {
        keep_free: u64::MAX / 4,
        ..opts()
    };
    let r = pm_offline::sync(&store, &node, &full).await.unwrap();
    if pm_offline::sync::free_bytes(dir.path()).is_some() {
        assert!(r.full.as_deref().is_some_and(|w| w.contains("free")), "{r:?}");
        assert!(r.downloaded.is_empty());
    }

    // Room made by letting go: the album goes (its song first, before any download), and a song
    // kept on its own then fits under the same limit
    assert!(store.keep(&KeepItem::Album(7), false, None).unwrap());
    assert!(store.keep(&KeepItem::Track(2), true, None).unwrap());
    let r = pm_offline::sync(&store, &node, &capped).await.unwrap();
    assert_eq!(
        (r.removed.clone(), r.downloaded.clone(), r.full.clone()),
        (vec![1], vec![2], None)
    );
    assert!(!dir.path().join("files/1.flac").exists());
    assert!(store.index().labels.is_empty() || !store.index().labels.contains_key("album:7"));

    // What to keep changed while a sync runs (Remove all, another limit): it stops before the
    // next song, and says it is over
    assert!(store.keep(&KeepItem::Track(2), false, None).unwrap());
    assert!(store.keep(&KeepItem::Album(7), true, None).unwrap());
    let t3: Arc<Mutex<Vec<Progress>>> = Arc::default();
    let t4 = t3.clone();
    let stopping = SyncOptions {
        stop: Some(pm_offline::StopCheck(Arc::new(|| true))),
        progress: Some(ProgressSink(Arc::new(move |p| {
            t4.lock().unwrap().push(p.clone())
        }))),
        ..opts()
    };
    let r = pm_offline::sync(&store, &node, &stopping).await.unwrap();
    assert!(r.downloaded.is_empty(), "{r:?}");
    assert!(!t3.lock().unwrap().last().unwrap().running);

    // Remove all: the device's copies go, and only the device's record changed on the node
    store.keep_nothing().unwrap();
    let r = pm_offline::sync(&store, &node, &capped).await.unwrap();
    assert_eq!(r.removed, [2]);
    assert!(store.index().tracks.is_empty() && store.index().labels.is_empty());
    let last = seen.lock().unwrap().manifests.last().cloned().unwrap();
    assert_eq!(last["keep"]["albums"], json!([]));
    assert_eq!(last["keep"]["tracks"], json!([]));
}

#[tokio::test]
async fn actual_download_bytes_obey_the_cap_even_with_a_stale_manifest() {
    for chunked in [false, true] {
        let seen: Fake = Arc::new(Mutex::new(Seen {
            manifest_bytes: Some(1),
            chunked,
            ..Default::default()
        }));
        let base = start_node(seen).await;
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        store
            .set_keep(Keep {
                tracks: vec![1],
                ..Default::default()
            })
            .unwrap();
        let cap = song(1).len() as u64 - 100;
        let r = pm_offline::sync(
            &store,
            &Node::new(&base, "tok").unwrap(),
            &SyncOptions {
                cap_bytes: Some(cap),
                ..opts()
            },
        )
        .await
        .unwrap();
        assert!(
            r.full.as_deref().is_some_and(|s| s.contains("limit")),
            "chunked={chunked}: {r:?}"
        );
        assert!(r.downloaded.is_empty());
        assert!(store.index().tracks.is_empty());
        assert!(!dir.path().join("files/1.flac").exists());
        assert!(pm_offline::sync::download_bytes(dir.path()).unwrap() <= cap);
    }
}

#[tokio::test]
async fn retained_partial_songs_count_against_the_cap() {
    let base = start_node(Arc::default()).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .set_keep(Keep {
            albums: vec![7],
            ..Default::default()
        })
        .unwrap();
    let part = dir.path().join("files/2.flac.part");
    std::fs::write(&part, &song(2)[..500]).unwrap();
    std::fs::write(dir.path().join("files/2.flac.etag"), "\"e2\"").unwrap();
    let r = pm_offline::sync(
        &store,
        &Node::new(&base, "tok").unwrap(),
        &SyncOptions {
            cap_bytes: Some(1000),
            ..opts()
        },
    )
    .await
    .unwrap();
    assert!(r.full.is_some(), "{r:?}");
    assert!(r.downloaded.is_empty());
    assert_eq!(std::fs::read(part).unwrap(), song(2)[..500]);
    assert_eq!(pm_offline::sync::download_bytes(dir.path()).unwrap(), 500);
}

#[tokio::test]
async fn final_progress_sees_the_downloaded_collection_metadata() {
    let base = start_node(Arc::default()).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(dir.path()).unwrap());
    store
        .set_keep(Keep {
            tracks: vec![2],
            playlists: vec!["p1".into()],
            ..Default::default()
        })
        .unwrap();
    let completed: Arc<Mutex<Vec<pm_offline::Summary>>> = Arc::default();
    let (observed, view) = (completed.clone(), store.clone());
    let r = pm_offline::sync(
        &store,
        &Node::new(&base, "tok").unwrap(),
        &SyncOptions {
            progress: Some(pm_offline::ProgressSink(Arc::new(move |p| {
                if !p.running {
                    observed.lock().unwrap().push(pm_offline::summary(&view.index()));
                }
            }))),
            ..opts()
        },
    )
    .await
    .unwrap();
    assert_eq!(r.downloaded, [2]);
    let done = completed.lock().unwrap();
    assert_eq!(done.len(), 1);
    let collection = done[0].items.iter().find(|i| i.kind == "playlist").unwrap();
    assert_eq!((collection.here, collection.songs), (1, 2));
}
