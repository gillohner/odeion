//! The Content-Security-Policy the node's pages are served with (security review 2026-10-27,
//! #7): scripts run only from the node itself, so a `javascript:` link, an injected handler or a
//! `<script>` slipped into a page does nothing.

use base64::Engine as _;
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

/// `Content-Security-Policy` for the node's own pages (`index.html`, `party.html`). Scripts run
/// only from the node, the pages' own inline scripts (by hash) and the YouTube player's two
/// addresses, so a `javascript:` link, an
/// injected handler or a `<script>` slipped into a page does nothing; no plugins; `<base>` only to
/// the node. Fetches, images and media stay open: covers and streams come from everywhere, and the
/// desktop app's bridge from its own scheme.
pub static PAGE_CSP: LazyLock<String> = LazyLock::new(|| policy(&[crate::INDEX_HTML, crate::PARTY_HTML]));

fn policy(pages: &[&str]) -> String {
    let mut hashes = String::new();
    for script in pages.iter().flat_map(|p| inline_scripts(p)) {
        let digest = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(script.as_bytes()));
        hashes.push_str(&format!(" 'sha256-{digest}'"));
    }
    // `frame-ancestors 'none'`: no other site shows the node's pages in a frame, with
    // `X-Frame-Options: DENY` for browsers that predate it
    format!(
        "script-src 'self'{hashes} {YOUTUBE_API}; object-src 'none'; base-uri 'self'; frame-ancestors 'none'"
    )
}

/// The YouTube IFrame API (videos play in "This browser" through it): its loader and the player
/// script the loader adds. Exact paths, not the whole origin, so YouTube's other endpoints can't be
/// used to run script on the page.
const YOUTUBE_API: &str = "https://www.youtube.com/iframe_api https://www.youtube.com/s/player/";

/// The text of every `<script>` without attributes in a page.
fn inline_scripts(page: &str) -> impl Iterator<Item = &str> {
    page.split("<script>")
        .skip(1)
        .filter_map(|rest| rest.split_once("</script>").map(|(s, _)| s))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pages are served with the policy, and its hashes are those of the pages' inline
    /// scripts as they are now (an edit to one changes its hash here by itself).
    #[tokio::test]
    async fn the_pages_forbid_scripts_that_are_not_the_nodes() {
        for (page, html) in [
            (crate::web_ui(Default::default()).await, crate::INDEX_HTML),
            (crate::party_page(Default::default()).await, crate::PARTY_HTML),
        ] {
            let csp = page
                .headers()
                .get(axum::http::header::CONTENT_SECURITY_POLICY)
                .expect("the page has a Content-Security-Policy")
                .to_str()
                .unwrap()
                .to_string();
            assert!(csp.starts_with("script-src 'self' 'sha256-"), "{csp}");
            assert!(!csp.contains("unsafe-inline"), "{csp}");
            assert!(!csp.contains("unsafe-eval"), "{csp}");
            assert!(csp.contains("object-src 'none'"), "{csp}");
            let scripts: Vec<_> = inline_scripts(html).collect();
            assert!(!scripts.is_empty());
            for s in scripts {
                let h = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(s.as_bytes()));
                assert!(csp.contains(&format!("'sha256-{h}'")), "{csp}");
            }
        }
    }

    /// Videos play in "This browser" through YouTube's IFrame API: the page loads
    /// `https://www.youtube.com/iframe_api` (app.js) and it adds YouTube's widget script from
    /// `/s/player/…`. Both are allowed in `script-src`, and nothing else of youtube.com is (its
    /// other endpoints could run script here). The player's own scripts run in YouTube's iframe,
    /// under YouTube's policy. CI's ui-youtube.js plays a video through it.
    #[test]
    fn the_youtube_player_may_load_and_nothing_else_of_youtube() {
        let csp = PAGE_CSP.as_str();
        let script_src: Vec<&str> = csp
            .split(';')
            .find_map(|d| d.trim().strip_prefix("script-src "))
            .expect("a script-src")
            .split_whitespace()
            .collect();
        let allows = |url: &str| {
            script_src.iter().any(|src| {
                src.starts_with("https://")
                    && if src.ends_with('/') {
                        url.starts_with(src)
                    } else {
                        url == *src
                    }
            })
        };
        assert!(allows("https://www.youtube.com/iframe_api"), "{csp}");
        assert!(
            allows("https://www.youtube.com/s/player/abc123/www-widgetapi.vflset/www-widgetapi.js"),
            "{csp}"
        );
        for not in [
            "https://www.youtube.com/oembed?callback=x",
            "https://www.youtube.com/",
            "https://youtube.com/iframe_api",
            "https://s.ytimg.com/yts/jsbin/www-widgetapi.js",
            "https://example.org/iframe_api",
        ] {
            assert!(!allows(not), "{not} in {csp}");
        }
        assert!(script_src.iter().all(|s| !s.contains('*')), "{csp}");
    }

    /// Inline event handlers would need `'unsafe-inline'` (or hashes): the page uses none.
    #[test]
    fn the_pages_have_no_inline_handlers() {
        for (name, src) in [
            ("app.js", crate::APP_JS),
            ("index.html", crate::INDEX_HTML),
            ("party.html", crate::PARTY_HTML),
        ] {
            for attr in [
                " onerror=",
                " onload=",
                " onclick=",
                " onchange=",
                " oninput=",
                " onsubmit=",
            ] {
                assert!(!src.contains(attr), "{name} has an inline{attr} handler");
            }
        }
    }
}
