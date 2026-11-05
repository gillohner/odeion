//! Where a request came from. The renderer routes take no token (renderers never do), so they
//! answer LAN peers only: a private or loopback source address on the node's own listener, not
//! through iroh ([`Remote`], its own listener) and not through a proxy or tunnel (Cloudflare,
//! a reverse proxy: any forwarding header). A node without an API token answers as the owner
//! only to requests that did not come through iroh or a proxy.

use crate::state::NodeState;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

/// Marks requests that came through the iroh tunnel (the remote listener adds it).
#[derive(Debug, Clone, Copy)]
pub struct Remote;

/// Headers a proxy or tunnel adds; a request carrying any of them did not come from the LAN
/// peer it seems to (a forged one only makes a LAN request look proxied).
const FORWARDED: &[&str] = &[
    "forwarded",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-real-ip",
    "cf-connecting-ip",
    "cf-ray",
];

fn proxied(h: &HeaderMap) -> bool {
    FORWARDED.iter().any(|k| h.contains_key(*k))
}

/// The request reached the node's listener directly: not through iroh, not through a proxy.
pub fn direct(req: &Request) -> bool {
    req.extensions().get::<Remote>().is_none() && !proxied(req.headers())
}

/// The source address of a direct request from the LAN or this machine, if it is one.
pub fn lan_peer(req: &Request) -> Option<IpAddr> {
    if !direct(req) {
        return None;
    }
    let ip = req.extensions().get::<ConnectInfo<SocketAddr>>()?.0.ip();
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    };
    let lan = match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        // Loopback, unique local (fc00::/7) and link-local (fe80::/10)
        IpAddr::V6(v6) => {
            v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    };
    lan.then_some(ip)
}

/// Layer for the renderer routes: LAN peers only, and the handlers learn who asked.
pub async fn lan_only(mut req: Request, next: Next) -> Response {
    let Some(ip) = lan_peer(&req) else {
        return (
            StatusCode::FORBIDDEN,
            "the renderer answers on the local network only",
        )
            .into_response();
    };
    req.extensions_mut().insert(pm_renderer::Peer(ip));
    next.run(req).await
}

/// Layer for the whole API: a node without an API token answers only requests that reached its
/// listener directly, never through iroh or a tunnel (it would make whoever is there the owner),
/// and only when they are addressed to it by a loopback name from its own pages
/// ([`loopback_addressed`]): another website must not reach it through the visitor's browser.
pub async fn tokenless_direct_only(State(s): State<Arc<NodeState>>, req: Request, next: Next) -> Response {
    if s.token().is_none() {
        if !direct(&req) {
            return (
                StatusCode::UNAUTHORIZED,
                "this node has no API token, so it answers only directly; set [api] token to reach it remotely",
            )
                .into_response();
        }
        if !loopback_addressed(req.headers(), req.uri()) {
            return (
                StatusCode::FORBIDDEN,
                "this node has no API token, so it answers only at 127.0.0.1 or localhost from its own pages; \
                 set [api] token to reach it by another name",
            )
                .into_response();
        }
    }
    next.run(req).await
}

/// The host a request names (`Host`, else the URI's authority, as HTTP/2 sends it), lowercased.
fn named_host(h: &HeaderMap, uri: &axum::http::Uri) -> Option<String> {
    h.get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| uri.authority().map(|a| a.as_str().to_string()))
        .map(|v| v.to_ascii_lowercase())
}

/// The name part of `host[:port]` (`[::1]:7788` → `::1`).
fn host_name(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    match host.rsplit_once(':') {
        Some((name, port)) if port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => host,
    }
}

/// A name that only ever means this machine: `localhost` (and `*.localhost`), 127.0.0.0/8, `::1`.
/// A website can't make a visitor's browser send one of these for its own pages.
fn is_loopback_name(name: &str) -> bool {
    name == "localhost"
        || name.ends_with(".localhost")
        || name.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Whether a request to a node without a token comes from the node's own pages, or from a
/// program that isn't a browser (DNS rebinding, security audit 2026-11-05):
/// - the host it names is a loopback name: a page on another site that rebinds its own name to
///   127.0.0.1 still sends that name, and is refused;
/// - an `Origin`, when the browser sends one, is that very host (scheme and port included): a
///   page elsewhere, another local web app on another port, or a sandboxed `null` page is refused.
///
/// A request naming no host at all (not a browser: they always do) passes on the source checks.
pub fn loopback_addressed(h: &HeaderMap, uri: &axum::http::Uri) -> bool {
    let Some(host) = named_host(h, uri) else {
        return true;
    };
    if !is_loopback_name(host_name(&host)) {
        return false;
    }
    same_origin(h, &host)
}

/// Whether an `Origin`, when there is one, is `http(s)://<host>` for the host the request names.
/// Browsers send `Origin` on every WebSocket handshake and on cross-site requests.
pub fn same_origin(h: &HeaderMap, host: &str) -> bool {
    match h.get(axum::http::header::ORIGIN) {
        None => true,
        Some(o) => o.to_str().ok().is_some_and(|o| {
            let o = o.to_ascii_lowercase();
            o == format!("http://{host}") || o == format!("https://{host}")
        }),
    }
}

/// A WebSocket handshake from a page on this node, or from a program that isn't a browser (no
/// `Origin`). Checked in every mode: a page on another site never opens the node's event stream
/// through a visitor's browser, token or not. "This node" is the host the request names, the one
/// a reverse proxy says it was asked for (`X-Forwarded-Host`; proxies often rewrite `Host`), and
/// the node's configured public address.
pub fn ws_origin_ok(h: &HeaderMap, uri: &axum::http::Uri, public_url: &str) -> bool {
    let Some(origin) = h.get(axum::http::header::ORIGIN) else {
        return true;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    let origin = origin.to_ascii_lowercase();
    let forwarded = h
        .get("x-forwarded-host")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_ascii_lowercase());
    let public = url::Url::parse(public_url)
        .ok()
        .map(|u| u.origin().ascii_serialization().to_ascii_lowercase());
    named_host(h, uri)
        .into_iter()
        .chain(forwarded)
        .any(|host| origin == format!("http://{host}") || origin == format!("https://{host}"))
        || public.is_some_and(|p| p == origin)
}

/// Serve the API for the iroh tunnel on its own loopback listener, every request marked
/// [`Remote`]; the tunnel's target. Returns the listener's address.
pub async fn serve_remote(state: Arc<NodeState>) -> std::io::Result<SocketAddr> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let addr = listener.local_addr()?;
    let app = crate::router(state).layer(axum::Extension(Remote));
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await {
            tracing::warn!("the remote-access listener stopped: {e}");
        }
    });
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;

    fn req(from: Option<&str>, headers: &[(&str, &str)], remote: bool) -> Request {
        let mut b = axum::http::Request::builder().uri("/upnp/zone/z/desc.xml");
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        let mut r = b.body(Body::empty()).unwrap();
        if let Some(a) = from {
            r.extensions_mut()
                .insert(ConnectInfo(a.parse::<SocketAddr>().unwrap()));
        }
        if remote {
            r.extensions_mut().insert(Remote);
        }
        r
    }

    #[test]
    fn only_direct_lan_peers_reach_the_renderer() {
        for a in [
            "192.168.1.20:5000",
            "10.0.0.3:1",
            "172.20.0.5:1",
            "127.0.0.1:9",
            "[::1]:9",
            "[fd00::5]:1",
            "[::ffff:192.168.1.2]:1",
        ] {
            assert!(lan_peer(&req(Some(a), &[], false)).is_some(), "{a}");
        }
        for a in [
            "8.8.8.8:53",
            "100.64.0.1:1",
            "[2001:db8::1]:1",
            "[::ffff:1.2.3.4]:1",
        ] {
            assert!(lan_peer(&req(Some(a), &[], false)).is_none(), "{a}");
        }
        // Through iroh (its listener is on loopback), through Cloudflare or a proxy, or unknown
        assert!(lan_peer(&req(Some("127.0.0.1:9"), &[], true)).is_none());
        assert!(lan_peer(&req(
            Some("172.18.0.4:1"),
            &[("cf-connecting-ip", "1.2.3.4")],
            false
        ))
        .is_none());
        assert!(lan_peer(&req(
            Some("127.0.0.1:9"),
            &[("x-forwarded-for", "1.2.3.4")],
            false
        ))
        .is_none());
        assert!(lan_peer(&req(None, &[], false)).is_none());
        assert!(direct(&req(Some("127.0.0.1:9"), &[], false)));
        assert!(!direct(&req(Some("127.0.0.1:9"), &[], true)));
    }

    fn heads(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    /// A node without a token answers its own pages at a loopback name, and nothing a page on
    /// another site sends through a visitor's browser (DNS rebinding; security audit 2026-11-05).
    #[test]
    fn a_tokenless_node_answers_only_its_loopback_names_and_own_pages() {
        let uri: axum::http::Uri = "/v1/status".parse().unwrap();
        for ok in [
            vec![("host", "127.0.0.1:7788")],
            vec![("host", "localhost:7788")],
            vec![("host", "LOCALHOST:7788"), ("origin", "http://localhost:7788")],
            vec![("host", "[::1]:7788"), ("origin", "http://[::1]:7788")],
            vec![("host", "127.0.0.1:7788"), ("origin", "http://127.0.0.1:7788")],
            vec![("host", "odeion.localhost:7788")],
            // A program that isn't a browser, naming no host at all
            vec![],
        ] {
            assert!(loopback_addressed(&heads(&ok), &uri), "{ok:?}");
        }
        for refused in [
            // A rebound name: the attacker's own host
            vec![("host", "evil.example:7788")],
            vec![("host", "127.0.0.1.evil.example:7788")],
            vec![("host", "music.lan:7788")],
            // Another site, another local app on another port, a sandboxed page
            vec![("host", "127.0.0.1:7788"), ("origin", "http://evil.example")],
            vec![("host", "localhost:7788"), ("origin", "http://localhost:3000")],
            vec![("host", "127.0.0.1:7788"), ("origin", "null")],
        ] {
            assert!(!loopback_addressed(&heads(&refused), &uri), "{refused:?}");
        }
        // HTTP/2 names the host in the URI
        let abs: axum::http::Uri = "http://evil.example:7788/v1/status".parse().unwrap();
        assert!(!loopback_addressed(&HeaderMap::new(), &abs));
    }

    /// The event stream opens from this node's pages, behind a reverse proxy too, and never
    /// from a page on another site.
    #[test]
    fn the_event_stream_opens_only_from_this_nodes_pages() {
        let uri: axum::http::Uri = "/v1/events".parse().unwrap();
        let public = "https://music.example";
        assert!(
            ws_origin_ok(&heads(&[("host", "127.0.0.1:7788")]), &uri, ""),
            "no Origin: not a browser"
        );
        assert!(ws_origin_ok(
            &heads(&[("host", "music.lan:7788"), ("origin", "http://music.lan:7788")]),
            &uri,
            ""
        ));
        // A proxy that rewrote Host: its X-Forwarded-Host, or the node's public address
        assert!(ws_origin_ok(
            &heads(&[
                ("host", "127.0.0.1:7788"),
                ("x-forwarded-host", "music.example"),
                ("origin", "https://music.example")
            ]),
            &uri,
            ""
        ));
        assert!(ws_origin_ok(
            &heads(&[("host", "127.0.0.1:7788"), ("origin", "https://music.example")]),
            &uri,
            public
        ));
        for o in [
            "http://evil.example",
            "null",
            "https://music.example.evil.example",
        ] {
            assert!(
                !ws_origin_ok(&heads(&[("host", "127.0.0.1:7788"), ("origin", o)]), &uri, public),
                "{o}"
            );
        }
    }
}
