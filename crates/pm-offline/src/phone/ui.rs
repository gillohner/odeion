//! The web UI phone mode shows: the node's own files (`crates/pm-api/static`), compiled into the
//! app, so there is one copy of the UI and the app's matches the commit it was built from.

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

macro_rules! asset {
    ($name:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../pm-api/static/", $name))
    };
}

const INDEX: &[u8] = asset!("index.html");

/// Every file the page loads, by its path (no service worker: the page is local already).
const FILES: &[(&str, &str, &[u8])] = &[
    ("app.js", "text/javascript; charset=utf-8", asset!("app.js")),
    ("app.css", "text/css; charset=utf-8", asset!("app.css")),
    (
        "feedback.js",
        "text/javascript; charset=utf-8",
        asset!("feedback.js"),
    ),
    ("feedback.css", "text/css; charset=utf-8", asset!("feedback.css")),
    ("usage.js", "text/javascript; charset=utf-8", asset!("usage.js")),
    ("pwa.js", "text/javascript; charset=utf-8", asset!("pwa.js")),
    ("pwa.css", "text/css; charset=utf-8", asset!("pwa.css")),
    ("icon.svg", "image/svg+xml", asset!("icon.svg")),
    (
        "manifest.webmanifest",
        "application/manifest+json",
        asset!("manifest.webmanifest"),
    ),
    ("icon-192.png", "image/png", asset!("icon-192.png")),
    ("icon-512.png", "image/png", asset!("icon-512.png")),
    (
        "icon-maskable-512.png",
        "image/png",
        asset!("icon-maskable-512.png"),
    ),
    (
        "apple-touch-icon.png",
        "image/png",
        asset!("apple-touch-icon.png"),
    ),
];

/// The page's Content-Security-Policy, built like the node's: scripts only from this server and
/// the page's own inline scripts (by hash); no plugins, no frames of it elsewhere.
pub static PAGE_CSP: LazyLock<String> = LazyLock::new(|| {
    let page = String::from_utf8_lossy(INDEX);
    let mut hashes = String::new();
    for script in page
        .split("<script>")
        .skip(1)
        .filter_map(|rest| rest.split_once("</script>").map(|(s, _)| s))
    {
        let digest = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(script.as_bytes()));
        hashes.push_str(&format!(" 'sha256-{digest}'"));
    }
    format!("script-src 'self'{hashes}; object-src 'none'; base-uri 'self'; frame-ancestors 'none'")
});

fn with(body: &'static [u8], kind: &'static str, cache: &'static str) -> Response {
    let mut r = (StatusCode::OK, body).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(kind));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    r
}

/// A UI file by its path; every other path (the UI's own routes) is the page.
pub fn file(path: &str) -> Response {
    let name = path.trim_start_matches('/');
    if let Some((_, kind, body)) = FILES.iter().find(|(n, _, _)| *n == name) {
        return with(body, kind, "no-cache");
    }
    if name == "sw.js" {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mut r = with(INDEX, "text/html; charset=utf-8", "no-store");
    if let Ok(v) = HeaderValue::from_str(&PAGE_CSP) {
        r.headers_mut().insert(header::CONTENT_SECURITY_POLICY, v);
    }
    r.headers_mut()
        .insert("x-frame-options", HeaderValue::from_static("DENY"));
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_and_its_files_are_the_nodes_own() {
        let page = file("/library/albums");
        assert_eq!(page.headers()[header::CONTENT_TYPE], "text/html; charset=utf-8");
        let csp = page.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .to_string();
        assert!(csp.starts_with("script-src 'self' 'sha256-"), "{csp}");
        assert!(!csp.contains("unsafe-inline"), "{csp}");
        assert_eq!(
            file("/app.js").headers()[header::CONTENT_TYPE],
            "text/javascript; charset=utf-8"
        );
        // Every file index.html names is here
        let index = String::from_utf8_lossy(INDEX);
        for (name, _, _) in FILES
            .iter()
            .filter(|(n, _, _)| n.ends_with(".js") || n.ends_with(".css"))
        {
            assert!(
                index.contains(&format!("/{name}")),
                "index.html no longer loads {name}"
            );
        }
        assert_eq!(file("/sw.js").status(), StatusCode::NOT_FOUND);
        // And every file the page loads is here (a new one would get the page instead)
        for attr in ["src=\"/", "href=\"/"] {
            for rest in index.split(attr).skip(1) {
                let name = rest.split('"').next().unwrap_or_default();
                if name.is_empty() || name == "sw.js" {
                    continue;
                }
                assert!(
                    FILES.iter().any(|(n, _, _)| *n == name),
                    "index.html loads /{name}, which phone mode doesn't serve"
                );
            }
        }
    }
}
