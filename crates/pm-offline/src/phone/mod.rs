//! Phone mode: the app without a node. A server on `127.0.0.1` serves the web UI the app carries
//! ([`ui`]) and the node's `/v1` routes the UI needs, in the node's shapes, from this phone:
//! search, item pages and streams of the person's own TIDAL and Qobuz (`pm-sources`, the same
//! code the node runs), played by the app's own player. `GET /v1/info` says `mode: "phone"` and
//! lists what it can do (`capabilities`); the UI shows only that.
//!
//! The server takes requests only for itself by name (as the offline server) and only with the
//! key it made at start ([`Phone::key`]): the app hands that key to the page it opens, as it hands
//! a node's token to the node's page. Other apps on the phone can reach `127.0.0.1` too.

pub mod item;
pub mod qobuz;
pub mod search;
pub mod store;
pub mod stream;
pub mod tidal;
pub mod ui;

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
pub const CAPABILITIES: &[&str] = &["search", "items", "sources"];

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

    /// The cache of TIDAL tracks.
    pub fn streams(&self) -> &stream::Streams {
        &self.streams
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

/// Serve on `127.0.0.1` (a free port). Returns the address and the server's task. Only requests
/// for this server by its own name are answered (403 otherwise), and `/v1` only with the key.
pub async fn serve(
    phone: Arc<Phone>,
) -> std::io::Result<(std::net::SocketAddr, tokio::task::JoinHandle<()>)> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let addr = listener.local_addr()?;
    let port = addr.port();
    let key = phone.key.clone();
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
        "tidal" => {
            tidal::sign_out(&p);
            Json(tidal::sign_in(&p)).into_response()
        }
        "qobuz" => {
            qobuz::sign_out(&p);
            Json(qobuz::sign_in(&p)).into_response()
        }
        _ => err(StatusCode::NOT_FOUND, "phone mode has TIDAL and Qobuz"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
