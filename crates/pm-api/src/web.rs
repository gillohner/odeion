//! What the web UI may link to (security review 2026-10-27, #7).
//!
//! Links in item pages come from other people's data: a MusicBrainz homepage, a radio station's
//! site, a link someone pasted. A `javascript:` one would run in the node's page, with the owner's
//! token at hand. So every [`Fact`](crate::types::Fact) link passes [`web_link`] on its way out,
//! the page checks again before it renders one, and the pages' Content-Security-Policy (pm-web)
//! stops scripts that aren't the node's own from running at all.

use serde::Serialize;

/// `u` (trimmed) when it is an http or https URL, as a browser parses it (`url` follows the same
/// WHATWG rules: `java\tscript:` is `javascript:`); `None` for `javascript:`, `data:` and every
/// other scheme. The text is kept as it was, so a stored homepage reads the same.
pub(crate) fn web_link(u: &str) -> Option<String> {
    let u = u.trim();
    let url = url::Url::parse(u).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| u.to_string())
}

/// Serde `serialize_with` for a link field: only [`web_link`]s go out.
pub(crate) fn serialize_link<S: serde::Serializer>(v: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
    v.as_deref().and_then(web_link).serialize(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Fact, FactKind};

    #[test]
    fn only_web_links_pass() {
        for ok in [
            "https://example.org/a?b=1",
            "http://192.168.1.10:7788/x",
            " HTTPS://Example.org ",
        ] {
            assert!(web_link(ok).is_some(), "{ok}");
        }
        for bad in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            " javascript:alert(1)",
            "java\tscript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "vbscript:x",
            "file:///etc/passwd",
            "/relative/path",
            "",
        ] {
            assert_eq!(web_link(bad), None, "{bad}");
        }
    }

    /// Every Fact link goes through the one check, however the Fact was built.
    #[test]
    fn a_fact_never_carries_a_script_link() {
        let fact = |href: &str| Fact {
            label: "Homepage".into(),
            value: href.into(),
            kind: FactKind::Fact,
            href: Some(href.into()),
        };
        let out = serde_json::to_value(fact("javascript:alert(document.cookie)")).unwrap();
        assert!(out["href"].is_null(), "{out}");
        let out = serde_json::to_value(fact("https://band.example/")).unwrap();
        assert_eq!(out["href"], "https://band.example/");
    }
}
