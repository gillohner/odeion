//! The web UI's own files, as the node sends them.
//!
//! The files are compiled in and change only with the node, but a browser asks for each of them
//! again at every page load (`Cache-Control: no-cache`, so an updated node's page is used at
//! once). Each file therefore has an `ETag`: a browser that still has it gets `304` and no body.
//! Text files also go out gzipped to whoever takes gzip (`app.js` is about a third of its size
//! then).

use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use std::io::Write as _;
use std::sync::LazyLock;

/// One compiled-in file.
pub(crate) struct Asset {
    mime: &'static str,
    body: &'static [u8],
    /// `W/"<hash of the body>"`: weak, since the gzipped answer carries the same one
    etag: HeaderValue,
    /// The body gzipped, for text that gets smaller
    gzip: Option<bytes::Bytes>,
}

impl Asset {
    fn new(mime: &'static str, body: &'static [u8]) -> Self {
        let hash = blake3::hash(body).to_hex();
        let etag = HeaderValue::from_str(&format!("W/\"{}\"", &hash[..32])).expect("an ETag is ASCII");
        let gzip = compressible(mime)
            .then(|| gzip(body))
            .filter(|z| z.len() < body.len())
            .map(bytes::Bytes::from);
        Self {
            mime,
            body,
            etag,
            gzip,
        }
    }

    /// The answer to a request with these headers: `304` when the browser has this very file,
    /// else the file, gzipped when the browser takes that. `extra` headers (a page's policy) go
    /// on both.
    pub(crate) fn respond(&self, asked: &HeaderMap, extra: &[(header::HeaderName, &str)]) -> Response {
        let mut h = HeaderMap::new();
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        h.insert(header::ETAG, self.etag.clone());
        if self.gzip.is_some() {
            h.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
        }
        for (name, value) in extra {
            if let Ok(v) = HeaderValue::from_str(value) {
                h.insert(name.clone(), v);
            }
        }
        if has_it(asked, &self.etag) {
            return (StatusCode::NOT_MODIFIED, h).into_response();
        }
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static(self.mime));
        match &self.gzip {
            Some(z) if takes_gzip(asked) => {
                h.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
                (h, z.clone()).into_response()
            }
            _ => (h, self.body).into_response(),
        }
    }
}

fn compressible(mime: &str) -> bool {
    mime.starts_with("text/")
        || mime.starts_with("application/manifest+json")
        || mime.starts_with("image/svg+xml")
}

fn gzip(body: &[u8]) -> Vec<u8> {
    let mut z =
        flate2::write::GzEncoder::new(Vec::with_capacity(body.len() / 3), flate2::Compression::new(6));
    z.write_all(body).expect("writing to memory");
    z.finish().expect("writing to memory")
}

/// Whether `If-None-Match` names this file (`*`, or its tag among those listed; weak comparison,
/// RFC 9110 §13.1.2).
fn has_it(asked: &HeaderMap, etag: &HeaderValue) -> bool {
    let opaque = |t: &str| t.trim().trim_start_matches("W/").to_string();
    let ours = opaque(etag.to_str().unwrap_or_default());
    asked
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|t| t.trim() == "*" || opaque(t) == ours)
}

/// Whether `Accept-Encoding` lists gzip (and not with `q=0`).
fn takes_gzip(asked: &HeaderMap) -> bool {
    asked
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|coding| {
            let mut parts = coding.split(';').map(str::trim);
            let name = parts.next().unwrap_or_default();
            let refused = parts
                .filter_map(|p| p.strip_prefix("q="))
                .any(|q| q.trim().parse::<f32>().is_ok_and(|q| q <= 0.0));
            (name.eq_ignore_ascii_case("gzip") || name.eq_ignore_ascii_case("x-gzip")) && !refused
        })
}

macro_rules! assets {
    ($($name:ident: $mime:expr, $file:literal;)*) => {
        $(pub(crate) static $name: LazyLock<Asset> =
            LazyLock::new(|| Asset::new($mime, include_bytes!(concat!("../static/", $file))));)*

        /// Hash and gzip every file now (the node does it once as it starts serving, off the
        /// async threads), so no page load waits for it.
        pub(crate) fn prepare() {
            $(LazyLock::force(&$name);)*
        }
    };
}

const HTML: &str = "text/html; charset=utf-8";
const JS: &str = "text/javascript; charset=utf-8";
const CSS: &str = "text/css; charset=utf-8";
const PNG: &str = "image/png";

assets! {
    INDEX_HTML: HTML, "index.html";
    PARTY_HTML: HTML, "party.html";
    APP_JS: JS, "app.js";
    APP_CSS: CSS, "app.css";
    FEEDBACK_JS: JS, "feedback.js";
    FEEDBACK_CSS: CSS, "feedback.css";
    USAGE_JS: JS, "usage.js";
    PWA_JS: JS, "pwa.js";
    PWA_CSS: CSS, "pwa.css";
    SW_JS: JS, "sw.js";
    MANIFEST: "application/manifest+json", "manifest.webmanifest";
    ICON_SVG: "image/svg+xml", "icon.svg";
    ICON_192: PNG, "icon-192.png";
    ICON_512: PNG, "icon-512.png";
    ICON_MASKABLE_512: PNG, "icon-maskable-512.png";
    APPLE_TOUCH_ICON: PNG, "apple-touch-icon.png";
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    fn asked(pairs: &[(header::HeaderName, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(k.clone(), HeaderValue::from_str(v).unwrap());
        }
        h
    }

    async fn body(r: Response) -> Vec<u8> {
        axum::body::to_bytes(r.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec()
    }

    #[tokio::test]
    async fn a_browser_that_has_the_file_gets_no_body() {
        let first = APP_JS.respond(&HeaderMap::new(), &[]);
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(first.headers()[header::CACHE_CONTROL], "no-cache");
        let etag = first.headers()[header::ETAG].to_str().unwrap().to_string();
        assert!(etag.starts_with("W/\"") && etag.ends_with('"'), "{etag}");
        assert_eq!(body(first).await, include_bytes!("../static/app.js"));

        // As a browser sends it back, alone or in a list, weak or not
        let strong = etag.trim_start_matches("W/").to_string();
        for sent in [
            etag.clone(),
            strong,
            format!("\"other\", {etag}"),
            "*".to_string(),
        ] {
            let again = APP_JS.respond(&asked(&[(header::IF_NONE_MATCH, &sent)]), &[]);
            assert_eq!(again.status(), StatusCode::NOT_MODIFIED, "{sent}");
            assert_eq!(again.headers()[header::ETAG].to_str().unwrap(), etag);
            assert_eq!(again.headers()[header::CACHE_CONTROL], "no-cache");
            assert!(body(again).await.is_empty(), "{sent}");
        }
        // Another file's tag, or an older build's, is not this file
        let other = APP_CSS.respond(&HeaderMap::new(), &[]).headers()[header::ETAG].clone();
        assert_ne!(other.to_str().unwrap(), etag);
        for sent in [other.to_str().unwrap(), "\"\"", "W/\"0\"", ""] {
            let r = APP_JS.respond(&asked(&[(header::IF_NONE_MATCH, sent)]), &[]);
            assert_eq!(r.status(), StatusCode::OK, "{sent}");
        }
    }

    #[tokio::test]
    async fn text_goes_out_gzipped_to_browsers_that_take_it() {
        let plain = include_bytes!("../static/app.js");
        for accept in ["gzip", "gzip, deflate, br", "br;q=1.0, GZIP;q=0.5", "x-gzip"] {
            let r = APP_JS.respond(&asked(&[(header::ACCEPT_ENCODING, accept)]), &[]);
            assert_eq!(r.status(), StatusCode::OK);
            assert_eq!(r.headers()[header::CONTENT_ENCODING], "gzip", "{accept}");
            assert_eq!(r.headers()[header::VARY], "Accept-Encoding");
            assert_eq!(r.headers()[header::CONTENT_TYPE], JS);
            let sent = body(r).await;
            assert!(sent.len() * 2 < plain.len(), "{} of {}", sent.len(), plain.len());
            let mut back = Vec::new();
            flate2::read::GzDecoder::new(&sent[..])
                .read_to_end(&mut back)
                .unwrap();
            assert_eq!(back, plain, "{accept}");
        }
        // Not asked for, refused, or another coding only: the file as it is
        for accept in [
            None,
            Some("identity"),
            Some("gzip;q=0"),
            Some("br, deflate"),
            Some("gzip;q=0.0, br"),
        ] {
            let h = accept
                .map(|a| asked(&[(header::ACCEPT_ENCODING, a)]))
                .unwrap_or_default();
            let r = APP_JS.respond(&h, &[]);
            assert!(!r.headers().contains_key(header::CONTENT_ENCODING), "{accept:?}");
            // Still told that the answer depends on it, so a cache in between keeps both apart
            assert_eq!(r.headers()[header::VARY], "Accept-Encoding");
            assert_eq!(body(r).await, plain, "{accept:?}");
        }
    }

    #[tokio::test]
    async fn pictures_are_not_gzipped_and_a_page_keeps_its_own_headers() {
        let r = ICON_192.respond(&asked(&[(header::ACCEPT_ENCODING, "gzip")]), &[]);
        assert!(!r.headers().contains_key(header::CONTENT_ENCODING));
        assert!(!r.headers().contains_key(header::VARY));
        assert!(r.headers().contains_key(header::ETAG));
        assert!(body(r).await.starts_with(b"\x89PNG"));

        let policy = [(header::CONTENT_SECURITY_POLICY, "script-src 'self'")];
        let r = INDEX_HTML.respond(&asked(&[(header::ACCEPT_ENCODING, "gzip")]), &policy);
        assert_eq!(r.headers()[header::CONTENT_SECURITY_POLICY], "script-src 'self'");
        assert_eq!(r.headers()[header::CONTENT_ENCODING], "gzip");
        let etag = r.headers()[header::ETAG].to_str().unwrap().to_string();
        // The 304 carries the policy too: a browser replaces the stored headers with these
        let again = INDEX_HTML.respond(&asked(&[(header::IF_NONE_MATCH, &etag)]), &policy);
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(
            again.headers()[header::CONTENT_SECURITY_POLICY],
            "script-src 'self'"
        );
    }

    #[test]
    fn every_file_is_prepared() {
        prepare();
        assert!(APP_CSS
            .gzip
            .as_ref()
            .is_some_and(|z| z.len() < APP_CSS.body.len()));
        assert!(SW_JS.gzip.is_some() && MANIFEST.gzip.is_some());
        assert!(ICON_512.gzip.is_none());
    }
}
