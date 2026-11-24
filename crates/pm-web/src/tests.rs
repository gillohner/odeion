//! The pages and files as the router answers them. Every test whose result depends on what is
//! in `static/` lives in this crate (or in pm-api's `tests/pages.rs`, where the page is checked
//! against the API's own lists), so a change to the UI alone is tested without pm-api's tests.

use super::*;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

async fn get_page(path: &str) -> Response {
    router()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn get_with(path: &str, name: header::HeaderName, value: &str) -> Response {
    router()
        .oneshot(
            Request::builder()
                .uri(path)
                .header(name, value)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn bytes(r: Response) -> Vec<u8> {
    axum::body::to_bytes(r.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec()
}

async fn text(r: Response) -> String {
    String::from_utf8(
        axum::body::to_bytes(r.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}

/// Every view of the page has its own path, and each answers the page with its policy.
#[tokio::test]
async fn every_page_path_serves_the_page() {
    let mut paths: Vec<String> = WEB_VIEWS.iter().map(|v| format!("/{v}")).collect();
    for p in [
        "/",
        "/app",
        "/app/",
        "/song/lib-1",
        "/album/lib-1",
        "/artist/lib-1",
        "/artist/lib-1/releases/albums",
        "/playlist/c1",
        "/person/p",
        "/smart/s",
        "/now-playing",
        "/shared/u",
        "/shared/u/c1",
        "/genre/rock",
        "/source/tidal",
        "/sources/tidal",
        "/settings/about",
        "/library/songs",
        "/home/recent",
        "/tag/rock",
        "/discover",
        "/discover/a/b",
        "/search",
    ] {
        paths.push(p.to_string());
    }
    for p in paths {
        let r = get_page(&p).await;
        assert_eq!(r.status(), StatusCode::OK, "{p}");
        assert_eq!(
            r.headers()[header::CONTENT_TYPE],
            "text/html; charset=utf-8",
            "{p}"
        );
        assert_eq!(r.headers()[header::CACHE_CONTROL], "no-cache", "{p}");
        assert_eq!(r.headers()[header::X_FRAME_OPTIONS], "DENY", "{p}");
        assert!(r.headers().contains_key(header::CONTENT_SECURITY_POLICY), "{p}");
        assert!(
            text(r).await.contains(r#"<script src="/app.js"></script>"#),
            "{p}"
        );
    }
    // A path the page doesn't have is not the page
    assert_eq!(get_page("/nope").await.status(), StatusCode::NOT_FOUND);
}

/// Every file is served with its type, asked for again at every load, with an ETag.
#[tokio::test]
async fn every_file_is_served_with_its_type() {
    for (path, mime, body) in [
        ("/app.js", "text/javascript; charset=utf-8", APP_JS),
        ("/app.css", "text/css; charset=utf-8", APP_CSS),
        ("/usage.js", "text/javascript; charset=utf-8", USAGE_JS),
        ("/sw.js", "text/javascript; charset=utf-8", SW_JS),
        ("/party/abc", "text/html; charset=utf-8", PARTY_HTML),
    ] {
        let r = get_page(path).await;
        assert_eq!(r.status(), StatusCode::OK, "{path}");
        assert_eq!(r.headers()[header::CONTENT_TYPE], mime, "{path}");
        assert_eq!(r.headers()[header::CACHE_CONTROL], "no-cache", "{path}");
        assert!(r.headers().contains_key(header::ETAG), "{path}");
        assert_eq!(text(r).await, body, "{path}");
    }
    for path in [
        "/feedback.js",
        "/feedback.css",
        "/pwa.js",
        "/pwa.css",
        "/manifest.webmanifest",
        "/icon.svg",
        "/icon-192.png",
        "/icon-512.png",
        "/icon-maskable-512.png",
        "/apple-touch-icon.png",
    ] {
        let r = get_page(path).await;
        assert_eq!(r.status(), StatusCode::OK, "{path}");
        assert!(r.headers().contains_key(header::ETAG), "{path}");
    }
}

/// Every view the page navigates to by path (NAV_VIEWS in app.js) is served on a reload or a
/// shared link, not a 404.
#[tokio::test]
async fn every_page_view_is_served_by_its_path() {
    let js = APP_JS;
    let start = js.find("const NAV_VIEWS = [").expect("NAV_VIEWS");
    let end = start + js[start..].find("];").expect("end of NAV_VIEWS");
    let views: Vec<&str> = js[start..end].split('\'').skip(1).step_by(2).collect();
    assert!(views.len() > 5, "{views:?}");
    for v in views {
        // Home is the root path (pathFor in app.js)
        let v = if v == "home" { "" } else { v };
        assert_eq!(get_page(&format!("/{v}")).await.status(), StatusCode::OK, "/{v}");
    }
}

/// Deep links and reloads of web UI paths land on the UI (SPA fallback).
#[tokio::test]
async fn web_ui_paths_serve_the_app() {
    for path in [
        "/song/mb-b1a9c0e9-d987-4042-ae91-78d6a3267d69",
        "/album/u-aHR0cHM6Ly90aWRhbC5jb20vYWxidW0vMzM4MjA0NTYw",
        "/artist/mb-0383dadf-2a4e-4d10-a46a-e9e041da8eb3",
        "/search?q=cro%20bad%20chick",
        "/discover",
        "/settings",
        "/settings/backup",
        "/sources/tidal",
        "/shared/zswn37ir9t8ge4r4posn4r7ow3eh384c8ud4kkxm585igt38x31o/0035WKVKQDHSM",
    ] {
        let resp = get_page(path).await;
        assert_eq!(resp.status(), StatusCode::OK, "{path}");
        assert!(text(resp).await.contains("/app.js"), "{path} is not the UI");
    }
}

/// The page and its files hold what they are for; a browser that takes gzip gets the same file
/// smaller, and one that has it, nothing.
#[tokio::test]
async fn the_files_are_the_app_gzipped_and_not_sent_twice() {
    for (path, needle) in [
        ("/", "<title>Odeion</title>"),
        ("/app", "<title>Odeion</title>"),
        ("/app/", "<title>Odeion</title>"),
        ("/app.css", ".player"),
        ("/app.js", "/v1/events"),
        ("/manifest.webmanifest", "\"display\": \"standalone\""),
        ("/icon.svg", "<svg"),
    ] {
        let resp = get_page(path).await;
        assert_eq!(resp.status(), StatusCode::OK, "{path}");
        let etag = resp.headers()[header::ETAG].clone();
        let body = text(resp).await;
        assert!(body.contains(needle), "{path}");
        let resp = get_with(path, header::ACCEPT_ENCODING, "gzip, deflate, br").await;
        assert_eq!(resp.headers()[header::CONTENT_ENCODING], "gzip", "{path}");
        assert_eq!(resp.headers()[header::ETAG], etag, "{path}");
        assert!(bytes(resp).await.len() < body.len(), "{path}");
        let resp = get_with(path, header::IF_NONE_MATCH, etag.to_str().unwrap()).await;
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED, "{path}");
    }
}

/// The installable web app (docs/guides/install-web-app.md): the manifest, its icons, the
/// install script and the service worker load with the types browsers insist on; the worker is
/// never cached by HTTP, so a fixed one reaches every browser at its next visit. (That none of
/// them needs a token is the node's test: pm-api's tests/renderer_lan.rs.)
#[tokio::test]
async fn the_install_files_have_their_types() {
    for (path, mime, cache) in [
        ("/manifest.webmanifest", "application/manifest+json", "no-cache"),
        ("/icon.svg", "image/svg+xml", "no-cache"),
        ("/icon-192.png", "image/png", "no-cache"),
        ("/icon-512.png", "image/png", "no-cache"),
        ("/icon-maskable-512.png", "image/png", "no-cache"),
        ("/apple-touch-icon.png", "image/png", "no-cache"),
        ("/pwa.js", "text/javascript; charset=utf-8", "no-cache"),
        ("/pwa.css", "text/css; charset=utf-8", "no-cache"),
        ("/sw.js", "text/javascript; charset=utf-8", "no-cache"),
    ] {
        let resp = get_page(path).await;
        assert_eq!(resp.status(), StatusCode::OK, "{path}");
        assert_eq!(resp.headers()[header::CONTENT_TYPE], mime, "{path}");
        assert_eq!(resp.headers()[header::CACHE_CONTROL], cache, "{path}");
        let etag = resp.headers()[header::ETAG].clone();
        let body = bytes(resp).await;
        if mime == "image/png" {
            assert!(body.starts_with(b"\x89PNG\r\n\x1a\n"), "{path} is a PNG");
        } else {
            assert!(!body.is_empty(), "{path}");
        }
        // Asked for again by a browser that has it: no body (assets.rs)
        let again = get_with(path, header::IF_NONE_MATCH, etag.to_str().unwrap()).await;
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED, "{path}");
        assert!(bytes(again).await.is_empty(), "{path}");
    }
    // The page names them, and its policy lets the node's own worker run
    let resp = get_page("/").await;
    let csp = resp.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .to_string();
    assert!(csp.starts_with("script-src 'self'"), "{csp}");
    assert!(
        !csp.contains("worker-src") && !csp.contains("default-src"),
        "{csp}"
    );
    let page = text(resp).await;
    assert!(page.contains(r#"<script src="/pwa.js"></script>"#));
    assert!(page.contains(r#"<link rel="apple-touch-icon" href="/apple-touch-icon.png">"#));
}

/// Usage insights' browser part is served like the app's own script, loaded before app.js
/// (which mounts it), and kept by the service worker with the page (an installed app that
/// starts without the network waits for it, like for app.js).
#[tokio::test]
async fn usage_js_is_served_and_the_shell_keeps_it() {
    let resp = get_page("/usage.js").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()[header::CONTENT_TYPE],
        "text/javascript; charset=utf-8"
    );
    assert_eq!(resp.headers()[header::CACHE_CONTROL], "no-cache");
    assert!(text(resp).await.contains("window.PM_USAGE"));
    let page = INDEX_HTML;
    let usage = page
        .find(r#"<script src="/usage.js"></script>"#)
        .expect("index.html loads usage.js");
    let app_js = page.find(r#"<script src="/app.js"></script>"#).unwrap();
    assert!(usage < app_js, "usage.js loads before app.js");
    assert!(
        page.contains(r#"<div id="usagenotice" hidden></div>"#),
        "the notice's slot"
    );
    for list in ["const FILES = [", "const BLOCKING = ["] {
        let l = SW_JS
            .split_once(list)
            .and_then(|(_, r)| r.split_once(']'))
            .unwrap()
            .0;
        assert!(l.contains("'/usage.js'"), "sw.js {list} keeps /usage.js");
    }
    // The party page has no part of it yet (guests: a later slice)
    assert!(!PARTY_HTML.contains("usage"));
}

/// The installed web app (PWA) and the page say Odeion; the stored token keeps its key, so a
/// browser that was signed in stays signed in (docs/branding-compatibility.md).
#[tokio::test]
async fn the_web_app_is_called_odeion() {
    let m: serde_json::Value =
        serde_json::from_str(&text(get_page("/manifest.webmanifest").await).await).unwrap();
    assert_eq!(m["name"], "Odeion");
    assert_eq!(m["short_name"], "Odeion");
    assert_eq!(
        m["start_url"], "/",
        "an installed web app keeps opening the same page"
    );
    let page = text(get_page("/").await).await;
    assert!(page.contains("<title>Odeion</title>"), "{page}");
    assert!(page.contains(r#"<meta name="apple-mobile-web-app-title" content="Odeion">"#));
    assert!(page.contains(r#"<span class="brand-name">Odeion</span>"#));
    assert!(!page.contains("Pubky Music")); // compat: pre-Odeion name must be gone
    let js = text(get_page("/app.js").await).await;
    assert!(
        js.contains("localStorage.getItem('pm.token')"),
        "the stored token's key is unchanged"
    );
    assert!(js.contains("‹ Odeion</a>"));
}

/// The apps keep a copy of the page for offline use (pm-offline). Every script and style sheet
/// the page loads from the node is in that copy's list: one that is not would be missing
/// offline, and the app would answer the page itself in its place.
#[tokio::test]
async fn the_apps_keep_every_file_the_page_loads() {
    let kept: Vec<&str> = pm_offline::sync::UI_FILES.iter().map(|(path, _)| *path).collect();
    // Every `<script src=…>` and every `<link …>` that is a style sheet, however its attributes
    // are ordered, spaced or quoted
    let loaded = loaded_files(INDEX_HTML);
    assert!(
        loaded.len() >= 6 && loaded.contains(&"/app.js") && loaded.contains(&"/app.css"),
        "{loaded:?}"
    );
    for url in loaded {
        assert!(url.starts_with('/'), "the page loads {url} from somewhere else");
        assert!(
            kept.contains(&url),
            "{url} is not kept for offline use (pm-offline UI_FILES)"
        );
    }
    // And each kept file is one the node serves
    for path in kept {
        assert_eq!(get_page(path).await.status(), StatusCode::OK, "{path}");
    }
}

/// The files a page loads: `src` of each `<script>` and `href` of each style sheet `<link>`.
fn loaded_files(page: &str) -> Vec<&str> {
    let lower = page.to_ascii_lowercase();
    let mut loaded = Vec::new();
    for (tag, attr) in [("<script", "src"), ("<link", "href")] {
        for (i, _) in lower.match_indices(tag) {
            let after = &page[i + tag.len()..];
            if !after.starts_with(|c: char| c.is_ascii_whitespace()) {
                continue;
            }
            let attrs = attributes(after);
            let get = |name: &str| {
                attrs
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(name))
                    .map(|(_, v)| *v)
            };
            if tag == "<link" && !get("rel").is_some_and(|r| r.eq_ignore_ascii_case("stylesheet")) {
                continue;
            }
            if let Some(url) = get(attr) {
                loaded.push(url);
            }
        }
    }
    loaded
}

/// A tag's attributes in order, up to its `>` (one inside a quoted value does not end it):
/// (name, value), the value double-, single- or not quoted, empty for a name alone.
fn attributes(tag: &str) -> Vec<(&str, &str)> {
    let b = tag.as_bytes();
    let skip_space = |mut i: usize| {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    };
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        if i >= b.len() || b[i] == b'>' {
            return out;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'>' | b'/') {
            i += 1;
        }
        let name = &tag[start..i];
        i = skip_space(i);
        if i >= b.len() || b[i] != b'=' {
            out.push((name, ""));
            continue;
        }
        i = skip_space(i + 1);
        if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
            let quote = b[i];
            let from = i + 1;
            i = from;
            while i < b.len() && b[i] != quote {
                i += 1;
            }
            out.push((name, &tag[from..i]));
            i += 1;
        } else {
            let from = i;
            while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                i += 1;
            }
            out.push((name, &tag[from..i]));
        }
    }
}

#[test]
fn a_loaded_file_is_found_however_its_tag_is_written() {
    let page = "<link rel=\"stylesheet\" href=\"/a.css\">\n<link\n  href='/b.css' rel=stylesheet>\n\
                <link rel=\"manifest\" href=\"/m.webmanifest\">\n<script\tsrc=/c.js defer></script>\n\
                <SCRIPT data-src=\"/no.js\" src = \"/d.js\"></SCRIPT><scripture src=\"/no2.js\">\n\
                <script data-x=\"a>b src=/no3.js\" src=\"/e.js\"></script>\n\
                <link rel=stylesheet title=\"x href=/no4.css\" href=\"/f.css\"/>";
    assert_eq!(
        loaded_files(page),
        ["/c.js", "/d.js", "/e.js", "/a.css", "/b.css", "/f.css"]
    );
}
