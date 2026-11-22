//! The Odeion node's web UI: the page, its script and styles, the installable web app's files,
//! and the routes that serve them ([`router`]). The node puts them next to its API
//! (`pm_api::router_with_pages`).
//!
//! The files are compiled in: `static/` as they are, and the page's script from its ordered
//! parts in `ui/app/` (see `ui/README.md`). They live in this small crate so that a change to the
//! UI rebuilds it and relinks the node, not the API crate.

mod assets;
mod csp;

use axum::extract::Path;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
pub use csp::PAGE_CSP;

/// The page and the files a test of the node reads as text (which views the page has, what the
/// worker keeps).
pub const INDEX_HTML: &str = include_str!("../static/index.html");
pub const PARTY_HTML: &str = include_str!("../static/party.html");
/// The page's script: its parts (`ui/app/`) joined in order by the build (`build.rs`).
pub const APP_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/app.js"));
pub const APP_CSS: &str = include_str!("../static/app.css");
pub const USAGE_JS: &str = include_str!("../static/usage.js");
pub const SW_JS: &str = include_str!("../static/sw.js");

/// The built-in browser controller: one static page over `/v1`, its files, and a party guest's
/// page. The page asks for the token itself, so all of it is served without one.
pub fn router() -> Router {
    // Hashed and gzipped before the first page load asks for them (off the async threads when
    // there is a runtime to do it on)
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        rt.spawn_blocking(assets::prepare);
    }
    // The web UI's paths (navigation brief): a deep link or reload lands on
    // the same view, so every page path serves index.html.
    let mut web = Router::new()
        .route("/", get(web_ui))
        .route("/app", get(web_ui))
        .route("/app/", get(web_ui))
        .route("/song/{id}", get(web_ui))
        .route("/album/{id}", get(web_ui))
        .route("/artist/{id}", get(web_ui))
        .route("/artist/{id}/releases/{section}", get(web_ui))
        .route("/playlist/{id}", get(web_ui))
        .route("/person/{id}", get(web_ui))
        .route("/smart/{id}", get(web_ui))
        .route("/now-playing", get(web_ui))
        .route("/shared/{user}", get(web_ui))
        .route("/shared/{user}/{id}", get(web_ui))
        .route("/genre/{id}", get(web_ui))
        .route("/source/{id}", get(web_ui))
        .route("/sources/{id}", get(web_ui))
        .route("/settings/{tab}", get(web_ui))
        .route("/library/{tab}", get(web_ui))
        .route("/home/{section}", get(web_ui))
        .route("/tag/{labels}", get(web_ui))
        .route("/discover", get(web_ui))
        .route("/discover/{*rest}", get(web_ui))
        .route("/search", get(web_ui));
    for view in WEB_VIEWS {
        web = web.route(&format!("/{view}"), get(web_ui));
    }
    web.route("/app.css", get(web_css))
        .route("/ux/", get(ux_index))
        .route("/ux/{*path}", get(ux_file))
        .route("/app.js", get(web_js))
        .route("/feedback.js", get(feedback_js))
        .route("/feedback.css", get(feedback_css))
        .route("/usage.js", get(usage_js))
        .route("/manifest.webmanifest", get(web_manifest))
        .route("/icon.svg", get(web_icon))
        // Installing the web app (docs/guides/install-web-app.md): public like the page
        .route(
            "/pwa.js",
            get(|h: HeaderMap| async move { assets::PWA_JS.respond(&h, &[]) }),
        )
        .route(
            "/pwa.css",
            get(|h: HeaderMap| async move { assets::PWA_CSS.respond(&h, &[]) }),
        )
        // (the worker is asked for again at every visit like the rest, so a changed one reaches
        // browsers at once)
        .route(
            "/sw.js",
            get(|h: HeaderMap| async move { assets::SW_JS.respond(&h, &[]) }),
        )
        .route(
            "/icon-192.png",
            get(|h: HeaderMap| async move { assets::ICON_192.respond(&h, &[]) }),
        )
        .route(
            "/icon-512.png",
            get(|h: HeaderMap| async move { assets::ICON_512.respond(&h, &[]) }),
        )
        .route(
            "/icon-maskable-512.png",
            get(|h: HeaderMap| async move { assets::ICON_MASKABLE_512.respond(&h, &[]) }),
        )
        .route(
            "/apple-touch-icon.png",
            get(|h: HeaderMap| async move { assets::APPLE_TOUCH_ICON.respond(&h, &[]) }),
        )
        // A party guest's link: the page (joining it is the API's, party.rs)
        .route("/party/{code}", get(party_page))
}

/// Top-level views of the web UI that have their own path.
pub const WEB_VIEWS: [&str; 15] = [
    "library",
    "add-music",
    "downloads",
    "favourites",
    "albums",
    "artists",
    "genres",
    "tracks",
    "playlists",
    "radio",
    "sources",
    "queue",
    "zones",
    "settings",
    "identify",
];

/// `/ux/`: the UX wireframes in `docs/ux/` (steward brief 2026-10-20), read
/// from the source tree on debug builds only; release builds answer 404.
async fn ux_index() -> Response {
    ux_file(Path("index.html".to_string())).await
}

async fn ux_file(Path(path): Path<String>) -> Response {
    if !cfg!(debug_assertions) || path.contains("..") || path.starts_with('/') {
        return StatusCode::NOT_FOUND.into_response();
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/ux");
    let file = root.join(&path);
    let mime = match file.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("png") => "image/png",
        Some("md") => "text/markdown; charset=utf-8",
        _ => "application/octet-stream",
    };
    match tokio::fs::read(&file).await {
        Ok(bytes) => (
            [(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "no-cache")],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The page's files (assets.rs): each with an ETag, text gzipped for browsers that take it,
/// and asked for again at every load (the icons keep their names when the picture changes).
async fn web_css(h: HeaderMap) -> Response {
    assets::APP_CSS.respond(&h, &[])
}

async fn web_js(h: HeaderMap) -> Response {
    assets::APP_JS.respond(&h, &[])
}

/// Help and problem reports (feedback.js, feedback.css), loaded by the page.
async fn feedback_js(h: HeaderMap) -> Response {
    assets::FEEDBACK_JS.respond(&h, &[])
}

/// Usage insights' browser part (usage.js): records nothing unless the node's switch is on.
async fn usage_js(h: HeaderMap) -> Response {
    assets::USAGE_JS.respond(&h, &[])
}

async fn feedback_css(h: HeaderMap) -> Response {
    assets::FEEDBACK_CSS.respond(&h, &[])
}

async fn web_manifest(h: HeaderMap) -> Response {
    assets::MANIFEST.respond(&h, &[])
}

async fn web_icon(h: HeaderMap) -> Response {
    assets::ICON_SVG.respond(&h, &[])
}

/// What the node's pages are served with besides the file: the policy for their scripts
/// (csp.rs), and no framing by other sites.
fn page_headers() -> [(header::HeaderName, &'static str); 2] {
    [
        (header::CONTENT_SECURITY_POLICY, PAGE_CSP.as_str()),
        (header::X_FRAME_OPTIONS, "DENY"),
    ]
}

/// `GET /party/{code}`: the page a party guest opens (no token; it joins with the code).
async fn party_page(h: HeaderMap) -> Response {
    assets::PARTY_HTML.respond(&h, &page_headers())
}

async fn web_ui(h: HeaderMap) -> Response {
    assets::INDEX_HTML.respond(&h, &page_headers())
}

#[cfg(test)]
mod tests;
