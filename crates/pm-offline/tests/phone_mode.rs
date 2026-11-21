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
    /// The slow track's second half comes only once this is told
    rest: tokio::sync::Notify,
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
        "tracks/3/playbackinfopostpaywall" => {
            t.playback.fetch_add(1, Ordering::SeqCst);
            let base = t.base.get().unwrap();
            let manifest = json!({"mimeType": "audio/flac", "codecs": "flac", "encryptionType": "NONE",
                "urls": [format!("{base}/media/slow.flac")]});
            json!({"assetPresentation": "FULL", "audioQuality": "LOSSLESS",
                   "manifestMimeType": "application/vnd.tidal.bts",
                   "manifest": base64::engine::general_purpose::STANDARD.encode(manifest.to_string())})
        }
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

/// A track whose first part comes at once and the rest only when the test says.
async fn slow(State(t): State<Arc<Tidal>>) -> Response {
    let (a, b) = AUDIO.split_at(10);
    let body = futures_util::stream::unfold(0, move |step| {
        let t = t.clone();
        async move {
            match step {
                0 => Some((Ok::<_, std::io::Error>(a.to_vec()), 1)),
                1 => {
                    t.rest.notified().await;
                    Some((Ok(b.to_vec()), 2))
                }
                _ => None,
            }
        }
    });
    (
        [(header::CONTENT_TYPE, "audio/flac")],
        axum::body::Body::from_stream(body),
    )
        .into_response()
}

async fn tidal() -> (Arc<Tidal>, String) {
    let t = Arc::new(Tidal::default());
    let app = Router::new()
        .route("/v1/{*rest}", get(v1))
        .route(
            "/media/song.flac",
            get(|| async { ([(header::CONTENT_TYPE, "audio/flac")], AUDIO) }),
        )
        .route("/media/slow.flac", get(slow))
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
            cache: dir.path().join("streams"),
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
async fn phone_mode_comes_back_on_the_same_address() {
    let dir = tempfile::tempdir().unwrap();
    let open = || {
        phone::Phone::open(Config {
            dir: dir.path().to_path_buf(),
            cache: dir.path().join("streams"),
            version: "test".into(),
            pubky: None,
            tidal: Default::default(),
        })
        .unwrap()
    };
    let (first, task) = phone::serve(open()).await.unwrap();
    task.abort();
    let _ = task.await;
    // The next start: the same origin, so what the page keeps in the browser is still there
    let (again, _task) = phone::serve(open()).await.unwrap();
    assert_eq!(again.port(), first.port());
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
    // Bookmarked from its page, the page knows it is (the same link as the Bookmark's)
    let r = f
        .http
        .put(format!("{}/v1/favourites", f.base))
        .bearer_auth(f.phone.key())
        .json(&json!({ "uri": pg["uri"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let (_, favs) = f.json("/v1/favourites").await;
    assert!(
        favs["uris"].as_array().unwrap().contains(&pg["uri"]),
        "{} in {favs}",
        pg["uri"]
    );
    assert_eq!(pg["target"]["uri"], pg["uri"]);
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

#[tokio::test]
async fn a_slow_track_starts_before_it_is_whole_and_is_fetched_once() {
    let f = Fixture::new().await;
    f.phone
        .streams()
        .set_whole_within(std::time::Duration::from_millis(200));
    // The first ask is answered while the track is still on its way
    let first = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        f.get("/v1/sources/tidal/stream/3").send(),
    )
    .await
    .expect("an answer before the track is whole")
    .unwrap();
    assert_eq!(first.status(), 200);
    assert_eq!(first.headers()[header::CONTENT_TYPE], "audio/flac");
    assert!(
        first.headers().get(header::CONTENT_LENGTH).is_none(),
        "its length isn't known yet"
    );
    // A second ask meanwhile follows the same fetch
    let second = f.get("/v1/sources/tidal/stream/3").send().await.unwrap();
    assert_eq!(second.status(), 200);
    f.tidal.rest.notify_one();
    let (a, b) = tokio::join!(first.bytes(), second.bytes());
    assert_eq!(&a.unwrap()[..], AUDIO);
    assert_eq!(&b.unwrap()[..], AUDIO);
    assert_eq!(f.tidal.playback.load(Ordering::SeqCst), 1, "one fetch for both");
    // Whole now: the next ask has ranges
    let r = f
        .get("/v1/sources/tidal/stream/3")
        .header(header::RANGE, "bytes=0-3")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 206);
    assert_eq!(&r.bytes().await.unwrap()[..], b"fLaC");
}

#[tokio::test]
async fn a_fetch_nobody_plays_any_more_stops_and_leaves_nothing() {
    let f = Fixture::new().await;
    f.phone
        .streams()
        .set_whole_within(std::time::Duration::from_millis(200));
    let r = f.get("/v1/sources/tidal/stream/3").send().await.unwrap();
    assert_eq!(r.status(), 200);
    // The person moved on: the answer is dropped half-way
    drop(r);
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    f.tidal.rest.notify_one();
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    let left: Vec<String> = std::fs::read_dir(f._dir.path().join("streams"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(left.is_empty(), "nothing kept or half-written: {left:?}");
    // Asked again, it is fetched anew
    let again = f.get("/v1/sources/tidal/stream/3").send().await.unwrap();
    assert_eq!(again.status(), 200);
    f.tidal.rest.notify_one();
    assert_eq!(&again.bytes().await.unwrap()[..], AUDIO);
    assert_eq!(f.tidal.playback.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_song_added_from_the_phone_is_kept_listed_found_and_played() {
    let f = Fixture::new().await;
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let upload = |name: &'static str| {
        let (f, flac) = (&f, flac.clone());
        async move {
            let r: Value = f
                .http
                .post(format!("{}/v1/library/uploads", f.base))
                .bearer_auth(f.phone.key())
                .json(&json!({"name": name, "size": flac.len()}))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            let id = r["id"].as_str().unwrap().to_string();
            // A part at the wrong place is refused with where the upload is
            let wrong = f
                .http
                .put(format!("{}/v1/library/uploads/{id}?offset=5", f.base))
                .bearer_auth(f.phone.key())
                .body(flac.clone())
                .send()
                .await
                .unwrap();
            assert_eq!(wrong.status(), 409);
            let done: Value = f
                .http
                .put(format!("{}/v1/library/uploads/{id}?offset=0", f.base))
                .bearer_auth(f.phone.key())
                .body(flac.clone())
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(done["state"], "stored", "{done}");
        }
    };
    upload("01 Mono Test.flac").await;
    // The same file again is the same song
    upload("01 Mono Test.flac").await;
    let (_, list) = f.json("/v1/offline").await;
    let tracks = list["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 1, "{list}");
    let id = tracks[0]["id"].as_i64().unwrap();
    assert_eq!(tracks[0]["offline"]["bytes"], flac.len());
    // Kept under the node's file name for it
    let hash = blake3::hash(&flac).to_hex().to_string();
    assert!(f
        ._dir
        .path()
        .join("music")
        .join(format!("{}-01_Mono_Test.flac", &hash[..16]))
        .is_file());
    // It plays from the phone, with ranges
    let r = f
        .get(&format!("/v1/library/tracks/{id}/file"))
        .header(header::RANGE, "bytes=0-3")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 206);
    assert_eq!(&r.bytes().await.unwrap()[..], b"fLaC");
    // Search finds it among the phone's songs
    let title = tracks[0]["title"].as_str().unwrap().to_string();
    let word = title.split_whitespace().next().unwrap().to_string();
    let r = f.get(&format!("/v1/search?q={word}")).send().await.unwrap();
    assert!(r.headers()["x-pm-sources"].to_str().unwrap().contains("library"));
    let text = r.text().await.unwrap();
    assert!(text.contains(&format!("\"track_id\":{id}")), "{text}");
    // Only music files
    let r = f
        .http
        .post(format!("{}/v1/library/uploads", f.base))
        .bearer_auth(f.phone.key())
        .json(&json!({"name": "setup.exe", "size": 10}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn a_song_sent_in_parts_is_whole_on_the_phone() {
    let f = Fixture::new().await;
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let r: Value = f
        .http
        .post(format!("{}/v1/library/uploads", f.base))
        .bearer_auth(f.phone.key())
        .json(&json!({"name": "Parts.flac", "size": flac.len()}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = r["id"].as_str().unwrap().to_string();
    let (a, b) = flac.split_at(20_000);
    let put = |offset: usize, body: Vec<u8>| {
        let (f, id) = (&f, id.clone());
        async move {
            f.http
                .put(format!("{}/v1/library/uploads/{id}?offset={offset}", f.base))
                .bearer_auth(f.phone.key())
                .body(body)
                .send()
                .await
                .unwrap()
        }
    };
    let first: Value = put(0, a.to_vec()).await.json().await.unwrap();
    assert_eq!(
        (first["offset"].as_u64(), first["state"].as_str()),
        (Some(20_000), Some("receiving"))
    );
    // The same part again (a retry after a lost answer) is refused with where the upload is
    assert_eq!(put(0, a.to_vec()).await.status(), 409);
    let done: Value = put(20_000, b.to_vec()).await.json().await.unwrap();
    assert_eq!(done["state"], "stored", "{done}");
    let (_, list) = f.json("/v1/offline").await;
    let id = list["tracks"][0]["id"].as_i64().unwrap();
    let whole = f
        .get(&format!("/v1/library/tracks/{id}/file"))
        .send()
        .await
        .unwrap();
    assert_eq!(whole.bytes().await.unwrap().len(), flac.len());
}

/// The mono FLAC fixture with these Vorbis comments in it.
fn tagged_flac(comments: &[&str]) -> Vec<u8> {
    let src = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let mut body = Vec::new();
    let vendor = b"phone mode test";
    body.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    body.extend_from_slice(vendor);
    body.extend_from_slice(&(comments.len() as u32).to_le_bytes());
    for c in comments {
        body.extend_from_slice(&(c.len() as u32).to_le_bytes());
        body.extend_from_slice(c.as_bytes());
    }
    // The blocks as they are, the comment block replaced
    let mut out = src[..4].to_vec();
    let mut i = 4;
    loop {
        let h = src[i];
        let last = h & 0x80 != 0;
        let len = u32::from_be_bytes([0, src[i + 1], src[i + 2], src[i + 3]]) as usize;
        if h & 0x7f == 4 {
            out.push((if last { 0x80 } else { 0 }) | 4);
            out.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
            out.extend_from_slice(&body);
        } else {
            out.extend_from_slice(&src[i..i + 4 + len]);
        }
        i += 4 + len;
        if last {
            break;
        }
    }
    out.extend_from_slice(&src[i..]);
    out
}

#[tokio::test]
async fn a_phone_song_is_bookmarked_and_listened_by_its_canonical_url() {
    let f = Fixture::new().await;
    let add = |name: &'static str, bytes: Vec<u8>| {
        let f = &f;
        async move {
            let r: Value = f
                .http
                .post(format!("{}/v1/library/uploads", f.base))
                .bearer_auth(f.phone.key())
                .json(&json!({"name": name, "size": bytes.len()}))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            let id = r["id"].as_str().unwrap().to_string();
            let done: Value = f
                .http
                .put(format!("{}/v1/library/uploads/{id}?offset=0", f.base))
                .bearer_auth(f.phone.key())
                .body(bytes)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(done["state"], "stored", "{done}");
        }
    };
    const MBID: &str = "b1a9c0e9-d987-4042-ae91-78d6a3267d69";
    add(
        "tagged.flac",
        tagged_flac(&["TITLE=Tagged Song", &format!("MUSICBRAINZ_TRACKID={MBID}")]),
    )
    .await;
    add("plain.flac", tagged_flac(&["TITLE=Plain Song"])).await;
    let (_, list) = f.json("/v1/offline").await;
    let id_of = |t: &str| {
        list["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["title"] == t)
            .unwrap()["id"]
            .as_i64()
            .unwrap()
    };
    let (tagged, plain) = (id_of("Tagged Song"), id_of("Plain Song"));
    // Their pages come from the phone: by the phone's id, or the song's canonical one
    for page in [
        format!("/v1/item/song/lib-{tagged}"),
        format!("/v1/item/song/mb-{MBID}"),
    ] {
        let (st, pg) = f.json(&page).await;
        assert_eq!(st, 200, "{page}: {pg}");
        assert_eq!(pg["title"], "Tagged Song");
        assert_eq!(pg["songs"][0]["track_id"], tagged, "it plays from the phone");
    }
    let (st, _) = f.json("/v1/item/song/lib-999").await;
    assert_eq!(st, 404);
    let put = |track: i64| {
        f.http
            .put(format!("{}/v1/favourites", f.base))
            .bearer_auth(f.phone.key())
            .json(&json!({ "track_id": track }))
            .send()
    };
    // A song with a recording MBID is bookmarked by its MusicBrainz URL, as a node would
    let r = put(tagged).await.unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    let url = format!("https://musicbrainz.org/recording/{MBID}");
    assert_eq!(v["uri"], url, "{v}");
    let (_, list) = f.json("/v1/offline").await;
    assert_eq!(list["favourites"]["track_ids"], json!([tagged]), "{list}");
    // One without says why it can't be
    let r = put(plain).await.unwrap();
    assert_eq!(r.status(), 422);
    assert!(r.text().await.unwrap().contains("Plain Song"));
    // Playing them: a listen of the tagged one only, and no error for the other
    for track in [tagged, plain] {
        let r = f
            .http
            .post(format!("{}/v1/library/history", f.base))
            .bearer_auth(f.phone.key())
            .json(&json!({ "track_id": track }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
    }
    let data: Value =
        serde_json::from_slice(&std::fs::read(f._dir.path().join("data.json")).unwrap()).unwrap();
    let listens: Vec<&Value> = data["pending"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|x| x["op"] == "listen")
        .collect();
    assert_eq!(listens.len(), 1, "{data}");
    assert!(listens[0].to_string().contains(&url), "{data}");
}
