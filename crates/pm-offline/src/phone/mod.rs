//! Phone mode: the app without a node. A server on `127.0.0.1` serves the web UI the app carries
//! ([`ui`]) and the node's `/v1` routes the UI needs, in the node's shapes, from this phone:
//! search, item pages and streams of the person's own TIDAL and Qobuz (`pm-sources`, the same
//! code the node runs), played by the app's own player. `GET /v1/info` says `mode: "phone"` and
//! lists what it can do (`capabilities`); the UI shows only that.
//!
//! The server takes requests only for itself by name (as the offline server) and only with the
//! key it made at start ([`Phone::key`]): the app hands that key to the page it opens, as it hands
//! a node's token to the node's page. Other apps on the phone can reach `127.0.0.1` too.

pub mod account;
pub mod data;
pub mod files;
pub mod item;
pub mod qobuz;
pub mod search;
pub mod songs;
pub mod store;
pub mod stream;
pub mod tidal;
pub mod ui;
pub mod vault;

#[cfg(test)]
mod stalled;

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use pm_sources::tidal::full::{BrowserClient, Client};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

/// What phone mode can do in this version (`GET /v1/info` `capabilities`).
pub const CAPABILITIES: &[&str] = &[
    "search",
    "items",
    "sources",
    "bookmarks",
    "listens",
    "songs",
    "add-songs",
    "homeserver-files",
];

pub(crate) fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

pub(crate) fn source_err(e: pm_sources::SourceError) -> Response {
    err(e.status, &e.message)
}

/// Where TIDAL is reached (tests point it at a mock) and the clients this build carries.
#[derive(Clone)]
pub struct TidalConf {
    pub auth_base: String,
    pub login_base: String,
    pub v1_base: String,
    pub browser: Option<BrowserClient>,
    pub device: Option<Client>,
}

impl Default for TidalConf {
    fn default() -> Self {
        Self {
            auth_base: "https://auth.tidal.com".into(),
            login_base: "https://login.tidal.com".into(),
            v1_base: "https://api.tidal.com/v1".into(),
            browser: BrowserClient::built_in(),
            device: pm_sources::tidal::built_in_device_client().map(|(id, secret)| Client {
                id,
                secret,
                from: "builtin".into(),
            }),
        }
    }
}

/// How the app starts phone mode.
pub struct Config {
    /// The app's own folder for phone mode, one per person (`<app data>/phone/<pubky>`): their
    /// sign-ins never meet another person's.
    pub dir: PathBuf,
    /// Where TIDAL tracks are cached while they play (the app's cache folder, which the system
    /// may empty).
    pub cache: PathBuf,
    /// The app's version (`/v1/info`).
    pub version: String,
    /// The person's Pubky (z32), from their Ring sign-in.
    pub pubky: Option<String>,
    pub tidal: TidalConf,
}

pub struct Phone {
    pub(crate) dir: PathBuf,
    key: String,
    pub(crate) store: Arc<store::FileStore>,
    pub(crate) http: reqwest::Client,
    pub(crate) tidal: TidalConf,
    version: String,
    pubky: Option<String>,
    pub(crate) streams: stream::Streams,
    /// Songs added to the phone.
    pub(crate) songs: songs::Songs,
    /// The person's Pubky account, once its session is restored.
    account: tokio::sync::RwLock<Option<Arc<account::Account>>>,
    /// Asks the vault sync to run now (a sign-in made or ended here).
    sync_now: Arc<tokio::sync::Notify>,
    /// A Bookmark, listen or quality change waits to be written (that alone, no vault pass).
    data_now: Arc<tokio::sync::Notify>,
    /// One vault step at a time.
    pub(crate) vault_lock: tokio::sync::Mutex<()>,
    /// One TIDAL renewal at a time, through the vault or here (`tidal::session`).
    pub(crate) tidal_renewal: tokio::sync::Mutex<()>,
    /// Whether the person's homeserver is the one installed with their node, as last known.
    pub(crate) bundled_here: std::sync::Mutex<Option<bool>>,
    /// Songs being copied to the homeserver.
    pub(crate) copier: files::Copier,
    /// One change at a time to what the phone keeps of the person's data (`data.json`).
    pub(crate) data_lock: std::sync::Mutex<()>,
}

/// 32 random bytes as hex.
fn new_key() -> String {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).expect("the system's random source");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

impl Phone {
    pub fn open(c: Config) -> std::io::Result<Arc<Self>> {
        let store = Arc::new(store::FileStore::open(&c.dir)?);
        let streams = stream::Streams::open(&c.cache)?;
        let songs = songs::Songs::open(&c.dir)?;
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(std::io::Error::other)?;
        Ok(Arc::new(Self {
            dir: c.dir,
            key: new_key(),
            store,
            http,
            tidal: c.tidal,
            version: c.version,
            pubky: c.pubky,
            streams,
            songs,
            account: Default::default(),
            sync_now: Default::default(),
            data_now: Default::default(),
            vault_lock: Default::default(),
            tidal_renewal: Default::default(),
            data_lock: Default::default(),
            bundled_here: Default::default(),
            copier: Default::default(),
        }))
    }

    /// The key every `/v1` request needs (made at start, never stored).
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Phone mode's folder on the phone.
    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// The songs added to the phone.
    pub fn songs(&self) -> &songs::Songs {
        &self.songs
    }

    /// The cache of TIDAL tracks.
    pub fn streams(&self) -> &stream::Streams {
        &self.streams
    }

    /// The person's Pubky account, when its session is restored.
    pub async fn account(&self) -> Option<Arc<account::Account>> {
        self.account.read().await.clone()
    }

    /// Use this account (the app's own sign-in, or a test's).
    pub async fn set_account(&self, a: account::Account) {
        *self.account.write().await = Some(Arc::new(a));
        self.sync_now.notify_one();
    }

    /// Runs the vault sync now.
    pub fn sync_soon(&self) {
        self.sync_now.notify_one();
    }

    /// Writes the Bookmarks, listens and quality changes waiting here now (not a whole sync).
    pub fn data_soon(&self) {
        self.data_now.notify_one();
    }

    /// The streaming sign-ins kept on the phone.
    pub fn store(&self) -> &Arc<store::FileStore> {
        &self.store
    }
}

/// Whether a request carries the server's key: `Authorization: Bearer <key>`, or `token=<key>`
/// in the query (the audio element and the app's player fetch streams by address).
fn has_key(headers: &HeaderMap, query: Option<&str>, key: &str) -> bool {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| same(t, key));
    let in_query = query.is_some_and(|q| {
        url::form_urlencoded::parse(q.as_bytes()).any(|(k, v)| k == "token" && same(&v, key))
    });
    bearer || in_query
}

/// Compares in the same time whatever the text (a key is never guessed byte by byte).
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// The routes, over a phone.
pub fn router(phone: Arc<Phone>) -> Router {
    Router::new()
        .route("/v1/info", get(info))
        .route("/v1/me", get(me))
        .route("/v1/zones", get(|| async { Json(json!([])) }))
        .route("/v1/play-order", get(play_order))
        .route("/v1/sources", get(sources))
        .route("/v1/search", get(search::search))
        .route(
            "/v1/favourites",
            get(data::bookmarks).put(data::add).delete(data::remove),
        )
        .route("/v1/library/history", post(data::played))
        .route("/v1/offline", get(songs::listing))
        .route("/v1/phone/storage", get(files::status).put(files::choose))
        .route("/v1/library/uploads", post(songs::start))
        .route(
            "/v1/library/uploads/{id}",
            get(songs::get)
                .put(songs::put_part)
                .layer(axum::extract::DefaultBodyLimit::max(
                    songs::CHUNK as usize + 64 * 1024,
                )),
        )
        .route("/v1/library/tracks/{id}", get(songs::track))
        .route("/v1/library/tracks/{id}/file", get(songs::file))
        .route("/v1/library/albums/{id}/cover", get(songs::cover))
        .route("/v1/item/{kind}/{id}", get(item::item))
        .route("/v1/item/{kind}/{id}/extras", get(item::extras))
        .route("/v1/sources/{source}/items/{id}", get(item::source_item))
        .route("/v1/sources/{source}/artists/{id}", get(item::source_artist))
        .route(
            "/v1/sources/{source}/signin",
            get(signin).put(signin_choose).delete(signin_out),
        )
        .route("/v1/sources/tidal/unofficial", get(tidal::status).put(tidal::put))
        .route(
            "/v1/sources/tidal/unofficial/login",
            post(tidal::login).delete(tidal::logout),
        )
        .route("/v1/sources/tidal/unofficial/browser", post(tidal::start_browser))
        .route(
            "/v1/sources/tidal/unofficial/browser/finish",
            post(tidal::finish_browser),
        )
        .route("/v1/sources/tidal/stream/{id}", get(stream::tidal))
        .route("/v1/sources/qobuz/unofficial", get(qobuz::status).put(qobuz::put))
        .route(
            "/v1/sources/qobuz/unofficial/login",
            post(qobuz::login).delete(qobuz::logout),
        )
        .route("/v1/sources/qobuz/stream/{id}", get(stream::qobuz))
        .route(
            "/v1/{*rest}",
            any(|| async {
                err(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "This needs an Odeion node. Connect one in Settings.",
                )
            }),
        )
        .fallback(|uri: axum::http::Uri| async move { ui::file(uri.path()) })
        .with_state(phone)
}

/// Where the port phone mode last served on is kept (in the person's folder).
const PORT_FILE: &str = "port";

/// Serve on `127.0.0.1` (a free port). Returns the address and the server's task. Only requests
/// for this server by its own name are answered (403 otherwise), and `/v1` only with the key.
pub async fn serve(
    phone: Arc<Phone>,
) -> std::io::Result<(std::net::SocketAddr, tokio::task::JoinHandle<()>)> {
    // The port this person's phone mode had before, so the page keeps its origin (and with it what
    // it keeps in the browser: the queue, the theme, the layout); another if that one is taken
    let saved = phone.dir.join(PORT_FILE);
    let before = std::fs::read_to_string(&saved)
        .ok()
        .and_then(|p| p.trim().parse::<u16>().ok());
    let listener = match before {
        Some(p) if p != 0 => match tokio::net::TcpListener::bind(("127.0.0.1", p)).await {
            Ok(l) => l,
            Err(_) => tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?,
        },
        _ => tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?,
    };
    let addr = listener.local_addr()?;
    let port = addr.port();
    if before != Some(port) {
        let _ = std::fs::write(&saved, port.to_string());
    }
    let key = phone.key.clone();
    let phone2 = phone.clone();
    // A sign-in made or ended here goes to the vault at once
    {
        let weak = Arc::downgrade(&phone);
        phone.store.on_change(move |key| {
            // Only a change the vault would carry (a sign-in, a sign-out, a renewal), not a
            // health note written as a song starts
            if let Some(form) = vault::FORMS.iter().find(|f| f.carry.iter().any(|c| c.key == key)) {
                if let Some(p) = weak.upgrade() {
                    if vault::changed_here(&p, form) {
                        p.sync_soon();
                    }
                }
            }
        });
    }
    let app = router(phone).layer(axum::middleware::from_fn(move |req: Request, next: Next| {
        let key = key.clone();
        async move {
            if !crate::server::is_local(req.headers(), port) {
                return err(StatusCode::FORBIDDEN, "not for this address");
            }
            if req.uri().path().starts_with("/v1/") && !has_key(req.headers(), req.uri().query(), &key) {
                return err(StatusCode::UNAUTHORIZED, "phone mode: this needs the app's key");
            }
            next.run(req).await
        }
    }));
    let task = tokio::spawn(async move {
        tokio::spawn(sync_loop(Arc::downgrade(&phone2)));
        if let Err(e) = axum::serve(listener, app).await {
            tracing::warn!("phone mode server stopped: {e}");
        }
    });
    Ok((addr, task))
}

/// `GET /v1/info`: what this is and what it can do.
async fn info(State(p): State<Arc<Phone>>) -> Json<Value> {
    Json(json!({
        "id": Value::Null,
        "name": "This phone",
        "version": p.version,
        "api": "v1",
        "mode": "phone",
        "capabilities": CAPABILITIES,
        "offline": false,
        "library": {
            "tracks": 0, "albums": 0, "artists": 0, "files": 0, "missing_files": 0,
            "total_duration_ms": 0,
        },
        "zones": 0,
        "outputs": 0,
        "scanning": false,
    }))
}

/// `GET /v1/me`: the one person phone mode is for.
async fn me(State(p): State<Arc<Phone>>) -> Json<Value> {
    Json(json!({ "role": "owner", "zones": [], "pubky": p.pubky }))
}

/// `GET /v1/play-order`: the services songs play from, in order.
async fn play_order(State(p): State<Arc<Phone>>) -> Json<Value> {
    Json(json!({
        "order": ["tidal", "qobuz"],
        "services": [
            {"id": "tidal", "name": "TIDAL", "connected": tidal::streams(&p)},
            {"id": "qobuz", "name": "Qobuz", "connected": qobuz::streams(&p)},
        ],
    }))
}

/// `GET /v1/sources`: TIDAL and Qobuz, in the node's shape.
async fn sources(State(p): State<Arc<Phone>>) -> Json<Value> {
    Json(json!([tidal::source_row(&p), qobuz::source_row(&p)]))
}

/// `GET /v1/sources/{source}/signin`: the ways to sign in, in the node's shape.
async fn signin(
    State(p): State<Arc<Phone>>,
    axum::extract::Path(source): axum::extract::Path<String>,
) -> Response {
    match source.as_str() {
        "tidal" => Json(tidal::sign_in(&p)).into_response(),
        "qobuz" => Json(qobuz::sign_in(&p)).into_response(),
        _ => err(StatusCode::NOT_FOUND, "phone mode has TIDAL and Qobuz"),
    }
}

/// `PUT /v1/sources/{source}/signin {way}`: the way picked (nothing to keep on a phone).
async fn signin_choose(
    State(p): State<Arc<Phone>>,
    axum::extract::Path(source): axum::extract::Path<String>,
) -> Response {
    signin(State(p), axum::extract::Path(source)).await
}

/// `DELETE /v1/sources/{source}/signin`: sign out of that service on this phone.
async fn signin_out(
    State(p): State<Arc<Phone>>,
    axum::extract::Path(source): axum::extract::Path<String>,
) -> Response {
    match source.as_str() {
        "tidal" => match tidal::sign_out(&p) {
            Ok(()) => Json(tidal::sign_in(&p)).into_response(),
            Err(e) => source_err(e),
        },
        "qobuz" => match qobuz::sign_out(&p) {
            Ok(()) => Json(qobuz::sign_in(&p)).into_response(),
            Err(e) => source_err(e),
        },
        _ => err(StatusCode::NOT_FOUND, "phone mode has TIDAL and Qobuz"),
    }
}

/// How often phone mode syncs with the vault while the app runs.
const SYNC_EVERY: std::time::Duration = std::time::Duration::from_secs(15 * 60);
/// The most a vault pass may take.
const PASS_FOR: std::time::Duration = std::time::Duration::from_secs(120);
/// How long it waits before trying the homeserver again.
const RETRY: std::time::Duration = std::time::Duration::from_secs(60);
/// The most the person's data may take in one pass (the waiting writes; on a whole pass also the
/// Bookmarks and prefs.json read back), as a vault pass.
const DATA_FOR: std::time::Duration = PASS_FOR;

/// The data part of a pass within [`DATA_FOR`] (`None`: it took too long; the rest waits for the
/// next pass). Cut off anywhere, nothing is lost: each change to what the phone keeps is made
/// whole between two homeserver calls, a listen has its id saved before its first try, a write
/// goes to the same path every try, and a write leaves the queue only once the homeserver took it.
async fn data_pass(p: &Phone, acct: &account::Account, full: bool) -> Option<data::Report> {
    let pass = async {
        if full {
            data::sync(p, acct).await
        } else {
            data::write_pending(p, acct).await
        }
    };
    tokio::time::timeout(DATA_FOR, pass).await.ok()
}

/// The Pubky account restored, then a vault sync now and every [`SYNC_EVERY`], and whenever a
/// sign-in changed here; until the phone backend is gone. Nothing without a session.
/// Waits for the next pass: `true` for a whole one (asked for, or `next_full` reached), `false`
/// for the waiting writes only. Writes don't move `next_full`, so listening all day still syncs.
async fn next_wake(
    now: &tokio::sync::Notify,
    data: &tokio::sync::Notify,
    next_full: tokio::time::Instant,
) -> bool {
    tokio::select! {
        _ = now.notified() => true,
        _ = data.notified() => false,
        _ = tokio::time::sleep_until(next_full) => true,
    }
}

async fn sync_loop(weak: std::sync::Weak<Phone>) {
    // A whole pass (the vault, the person's data read back) at start, every SYNC_EVERY and when a
    // sign-in changed; only the waiting writes when the page changed something
    let mut full = true;
    // When the next whole pass is due (a write in between doesn't move it)
    let mut next_full = tokio::time::Instant::now();
    loop {
        let Some(p) = weak.upgrade() else { return };
        let acct = match p.account().await {
            Some(a) => a,
            None => match account::Account::restore(&p.dir).await {
                Ok(Some(a)) => {
                    tracing::info!("phone mode: Pubky session restored");
                    let a = Arc::new(a);
                    *p.account.write().await = Some(a.clone());
                    a
                }
                // No session (yet): wait to be told (set_account) or try again later
                Ok(None) => {
                    let now = p.sync_now.clone();
                    drop(p);
                    tokio::select! {
                        _ = now.notified() => {}
                        _ = tokio::time::sleep(SYNC_EVERY) => {}
                    }
                    continue;
                }
                Err(e) => {
                    tracing::warn!("phone mode: Pubky not reachable now: {e}");
                    drop(p);
                    tokio::time::sleep(RETRY).await;
                    continue;
                }
            },
        };
        if full {
            next_full = tokio::time::Instant::now() + SYNC_EVERY;
            // Within a limit: a dead connection (a network switch) never holds the vault, and
            // with it TIDAL's renewal, until the app restarts
            match tokio::time::timeout(PASS_FOR, vault::pass(&p, &acct)).await {
                Ok(Ok(r)) if r != vault::Report::default() => tracing::info!(?r, "phone mode: vault synced"),
                Ok(Ok(_)) => {}
                Ok(Err(e)) => tracing::warn!("phone mode: vault: {e}"),
                Err(_) => tracing::warn!("phone mode: the vault sync took too long; next time"),
            }
        }
        // Within a limit too: a stalled write never stops the vault passes, the other writes or
        // the songs' copy
        match data_pass(&p, &acct, full).await {
            Some(r) if r != data::Report::default() => tracing::info!(?r, "phone mode: Pubky data synced"),
            Some(_) => {}
            None => {
                tracing::warn!("phone mode: the homeserver took too long with the person's data; next time")
            }
        }
        if full {
            files::start(&p, &acct);
        }
        let (now, data_now) = (p.sync_now.clone(), p.data_now.clone());
        drop(p);
        drop(acct);
        full = next_wake(&now, &data_now, next_full).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_in_between_dont_put_off_the_whole_sync() {
        let (now, data) = (
            Arc::new(tokio::sync::Notify::new()),
            Arc::new(tokio::sync::Notify::new()),
        );
        let due = tokio::time::Instant::now() + std::time::Duration::from_millis(150);
        // A song starts every 40 ms (a listen to write each time)
        let d2 = data.clone();
        let songs = tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                d2.notify_one();
            }
        });
        let mut writes = 0;
        while !next_wake(&now, &data, due).await {
            writes += 1;
            assert!(writes < 20, "the whole sync never came");
        }
        songs.abort();
        assert!(writes >= 2, "the writes went in between ({writes})");
        assert!(tokio::time::Instant::now() >= due);
    }

    #[tokio::test]
    async fn a_homeserver_that_never_answers_doesnt_hold_the_data_sync() {
        let dir = tempfile::tempdir().unwrap();
        let p = Phone::open(Config {
            dir: dir.path().to_path_buf(),
            cache: dir.path().join("streams"),
            version: "test".into(),
            pubky: None,
            tidal: Default::default(),
        })
        .unwrap();
        let wait = data::Pending::Bookmark {
            url: "https://tidal.com/browse/track/1".into(),
            on: true,
            at: 5,
        };
        data::change(&p, |l| l.pending.push(wait.clone())).unwrap();
        let s = stalled::homeserver().await;
        let acct = s.account();
        tokio::time::pause();
        for full in [false, true] {
            let started = tokio::time::Instant::now();
            let r = tokio::time::timeout(std::time::Duration::from_secs(3600), data_pass(&p, &acct, full))
                .await
                .expect("the data pass ends by itself");
            assert_eq!(r, None, "given up");
            assert!(started.elapsed() <= DATA_FOR + std::time::Duration::from_secs(1));
            // The write still waits, for the next pass
            assert_eq!(data::load(&p).pending, std::slice::from_ref(&wait));
        }
        assert!(s.taken.load(std::sync::atomic::Ordering::SeqCst) > 0);
    }

    #[test]
    fn the_key_is_needed_and_compared_whole() {
        let key = new_key();
        assert_eq!(key.len(), 64);
        assert_ne!(key, new_key(), "a new key at every start");
        let mut h = HeaderMap::new();
        assert!(!has_key(&h, None, &key));
        assert!(has_key(&h, Some(&format!("a=1&token={key}")), &key));
        assert!(!has_key(&h, Some("token=x"), &key));
        assert!(!has_key(&h, Some(&format!("token={}", &key[..63])), &key));
        h.insert(header::AUTHORIZATION, format!("Bearer {key}").parse().unwrap());
        assert!(has_key(&h, None, &key));
        h.insert(header::AUTHORIZATION, "Bearer nope".parse().unwrap());
        assert!(!has_key(&h, None, &key));
    }
}
