//! Phone mode's backend end to end: the real server on 127.0.0.1 (its key and its own-name
//! check), a person's TIDAL full-tracks session on the phone, and a stand-in TIDAL on another
//! local port answering its v1 API as TIDAL does.

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use base64::Engine as _;
use pm_offline::phone::{self, Config, Phone, TidalConf};
use pm_sources::SessionStore;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const TOKEN: &str = "person-session-token";
const AUDIO: &[u8] = b"fLaC\x00\x00\x00\x22 a stand-in for a FLAC file, long enough for ranges";

#[derive(Default)]
struct Tidal {
    playback: AtomicUsize,
    encrypted: std::sync::atomic::AtomicBool,
    base: std::sync::OnceLock<String>,
}

fn signed(h: &HeaderMap) -> bool {
    h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) == Some(&format!("Bearer {TOKEN}"))
}

fn album() -> Value {
    json!({"id": 1, "title": "Fixture Album", "cover": "ab12-cd34", "releaseDate": "2026-01-02",
           "artists": [{"id": 9, "name": "Fixture Band"}], "type": "ALBUM", "upc": "1234567890123"})
}

fn song() -> Value {
    json!({"id": 2, "title": "Fixture Song", "duration": 61, "trackNumber": 1, "isrc": "QZFIX2600002",
           "album": album(), "artists": [{"id": 9, "name": "Fixture Band"}], "streamReady": true,
           "audioQuality": "LOSSLESS"})
}

async fn v1(State(t): State<Arc<Tidal>>, Path(rest): Path<String>, headers: HeaderMap) -> Response {
    if !signed(&headers) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"userMessage": "no"}))).into_response();
    }
    let body = match rest.as_str() {
        "search" => {
            json!({"artists": {"items": []}, "albums": {"items": [album()]}, "tracks": {"items": [song()]}})
        }
        "albums/1" => album(),
        "albums/1/tracks" => json!({"items": [song()], "totalNumberOfItems": 1}),
        "tracks/2/playbackinfopostpaywall" => {
            t.playback.fetch_add(1, Ordering::SeqCst);
            let base = t.base.get().unwrap();
            let manifest = json!({"mimeType": "audio/flac", "codecs": "flac",
                "encryptionType": if t.encrypted.load(Ordering::SeqCst) { "OLD_AES" } else { "NONE" },
                "urls": [format!("{base}/media/song.flac")]});
            json!({"assetPresentation": "FULL", "audioQuality": "LOSSLESS",
                   "manifestMimeType": "application/vnd.tidal.bts",
                   "manifest": base64::engine::general_purpose::STANDARD.encode(manifest.to_string())})
        }
        _ => return (StatusCode::NOT_FOUND, Json(json!({"userMessage": "not found"}))).into_response(),
    };
    Json(body).into_response()
}

async fn tidal() -> (Arc<Tidal>, String) {
    let t = Arc::new(Tidal::default());
    let app = Router::new()
        .route("/v1/{*rest}", get(v1))
        .route(
            "/media/song.flac",
            get(|| async { ([(header::CONTENT_TYPE, "audio/flac")], AUDIO) }),
        )
        .with_state(t.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    t.base.set(base.clone()).unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (t, base)
}

struct Fixture {
    _dir: tempfile::TempDir,
    phone: Arc<Phone>,
    base: String,
    tidal: Arc<Tidal>,
    http: reqwest::Client,
}

impl Fixture {
    async fn new() -> Self {
        let (tidal, tbase) = tidal().await;
        let dir = tempfile::tempdir().unwrap();
        let phone = Phone::open(Config {
            dir: dir.path().to_path_buf(),
            version: "0.0.0-test".into(),
            pubky: Some("o1gg96ewuojmopcjbz8895478wdtxtzzuxnfjjz8o8e77csa1ngo".into()),
            tidal: TidalConf {
                auth_base: tbase.clone(),
                login_base: tbase.clone(),
                v1_base: format!("{tbase}/v1"),
                browser: None,
                device: None,
            },
        })
        .unwrap();
        let st = pm_sources::tidal::full::ModeState {
            enabled: true,
            session: Some(pm_sources::tidal::full::Session {
                access_token: TOKEN.into(),
                expires_at: pm_sources::now_unix() + 3600,
                country: "CH".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let v = serde_json::to_string(&st).unwrap();
        phone
            .store()
            .update(pm_sources::tidal::full::STATE_KEY, &mut |_| Some(v.clone()));
        let (addr, _task) = phone::serve(phone.clone()).await.unwrap();
        Self {
            _dir: dir,
            phone,
            base: format!("http://127.0.0.1:{}", addr.port()),
            tidal,
            http: reqwest::Client::new(),
        }
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.http
            .get(format!("{}{path}", self.base))
            .bearer_auth(self.phone.key())
    }

    async fn json(&self, path: &str) -> (u16, Value) {
        let r = self.get(path).send().await.unwrap();
        let s = r.status().as_u16();
        (s, r.json().await.unwrap_or(Value::Null))
    }
}

#[tokio::test]
async fn the_backend_answers_only_its_own_page_and_only_with_its_key() {
    let f = Fixture::new().await;
    // The page itself loads without the key (the app puts the key in its storage)
    let page = f.http.get(format!("{}/search", f.base)).send().await.unwrap();
    assert_eq!(page.status(), 200);
    assert!(page.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .starts_with("script-src 'self'"));
    // /v1 needs it: none, a wrong one, then the right one as a header or in the address
    let none = f.http.get(format!("{}/v1/info", f.base)).send().await.unwrap();
    assert_eq!(none.status(), 401);
    let wrong = f
        .http
        .get(format!("{}/v1/info", f.base))
        .bearer_auth("not-the-key")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401);
    let (st, info) = f.json("/v1/info").await;
    assert_eq!(st, 200);
    assert_eq!(info["mode"], "phone");
    assert_eq!(info["capabilities"], json!(phone::CAPABILITIES));
    let q = f
        .http
        .get(format!("{}/v1/info?token={}", f.base, f.phone.key()))
        .send()
        .await
        .unwrap();
    assert_eq!(q.status(), 200);
    // Another name for 127.0.0.1 (a rebinding page) is refused before the key is looked at
    let rebound = f
        .http
        .get(format!("{}/v1/info", f.base))
        .bearer_auth(f.phone.key())
        .header(header::HOST, "evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(rebound.status(), 403);
    // What needs a node says so
    let (st, e) = f.json("/v1/zones/abc/queue").await;
    assert_eq!(st, 503, "{e}");
    let (_, me) = f.json("/v1/me").await;
    assert_eq!(me["role"], "owner");
}

#[tokio::test]
async fn search_finds_tidal_songs_with_the_nodes_ids() {
    let f = Fixture::new().await;
    let r = f.get("/v1/search?q=fixture&in=everywhere").send().await.unwrap();
    assert_eq!(
        r.headers()["x-pm-sources"],
        "tidal",
        "Qobuz isn't signed in on this phone"
    );
    let text = r.text().await.unwrap();
    let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 1, "{text}");
    assert_eq!(lines[0]["source"], "tidal");
    assert_eq!(lines[0]["status"], "ok", "{text}");
    let hits = lines[0]["hits"].as_array().unwrap();
    let song = hits.iter().find(|h| h["kind"] == "song").unwrap();
    assert_eq!(song["id"], "tidal-2");
    assert_eq!(song["uri"], "tidal:track:2");
    assert_eq!(song["subtitle"], "Fixture Band");
    assert!(song["keys"]
        .as_array()
        .unwrap()
        .contains(&json!("isrc:QZFIX2600002")));
    let album = hits.iter().find(|h| h["kind"] == "album").unwrap();
    assert_eq!(album["id"], "tidal-1");
    // Nothing to search: no source asked
    let r = f.get("/v1/search?q=%20").send().await.unwrap();
    assert_eq!(r.headers()["x-pm-sources"], "");
}

#[tokio::test]
async fn an_album_page_plays_from_tidal_and_its_song_streams_with_ranges() {
    let f = Fixture::new().await;
    let (st, pg) = f.json("/v1/item/album/tidal-1").await;
    assert_eq!(st, 200, "{pg}");
    assert_eq!(pg["title"], "Fixture Album");
    assert_eq!(pg["plays_from"]["service"], "tidal");
    assert_eq!(pg["songs"][0]["uri"], "tidal:track:2");
    assert_eq!(pg["songs"][0]["id"], "tidal-2");
    assert_eq!(pg["total_ms"], 61_000);
    // The song's stream: the full track through the phone, a range of it, then from the cache
    let r = f
        .get("/v1/sources/tidal/stream/2")
        .header(header::RANGE, "bytes=0-3")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 206);
    assert_eq!(r.headers()[header::CONTENT_TYPE], "audio/flac");
    assert_eq!(&r.bytes().await.unwrap()[..], b"fLaC");
    let whole = f.get("/v1/sources/tidal/stream/2").send().await.unwrap();
    assert_eq!(&whole.bytes().await.unwrap()[..], AUDIO);
    assert_eq!(
        f.tidal.playback.load(Ordering::SeqCst),
        1,
        "the second ask came from the cache"
    );
    // An id that isn't TIDAL's is refused before anything is asked
    let (st, _) = f.json("/v1/sources/tidal/stream/2..%2F3").await;
    assert_eq!(st, 400);
}

#[tokio::test]
async fn an_encrypted_track_is_refused_never_kept() {
    let f = Fixture::new().await;
    f.tidal.encrypted.store(true, Ordering::SeqCst);
    let (st, e) = f.json("/v1/sources/tidal/stream/2").await;
    assert_eq!(st, 404, "{e}");
    assert!(e["error"].as_str().unwrap().contains("encrypted"), "{e}");
    let left: Vec<_> = std::fs::read_dir(f._dir.path().join("streams"))
        .unwrap()
        .flatten()
        .collect();
    assert!(left.is_empty(), "nothing of it is kept");
    // The sign-in itself is fine: the source is still ready
    let (_, rows) = f.json("/v1/sources").await;
    let tidal = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "tidal")
        .unwrap();
    assert_eq!(tidal["healthy"], true, "{tidal}");
}

#[tokio::test]
async fn the_sign_in_pages_have_the_nodes_shape() {
    let f = Fixture::new().await;
    let (_, s) = f.json("/v1/sources/tidal/signin").await;
    assert_eq!(s["source"], "tidal");
    assert_eq!(s["in_use"]["way"], "device");
    assert_eq!(s["ways"][0]["id"], "browser");
    assert_eq!(
        s["ways"][0]["available"], false,
        "this test build has no browser client"
    );
    assert_eq!(s["details"]["unofficial"]["signed_in"], true);
    let (_, q) = f.json("/v1/sources/qobuz/signin").await;
    assert_eq!(q["ways"][0]["id"], "own");
    assert!(q["in_use"].is_null());
    // Signing out of TIDAL takes it out of search
    let r = f
        .http
        .delete(format!("{}/v1/sources/tidal/signin", f.base))
        .bearer_auth(f.phone.key())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let r = f.get("/v1/search?q=fixture").send().await.unwrap();
    assert_eq!(r.headers()["x-pm-sources"], "");
    // Nothing secret in what the page gets
    let (_, s) = f.json("/v1/sources/tidal/unofficial").await;
    assert!(!s.to_string().contains(TOKEN));
}
