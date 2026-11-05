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
        if !loopback_addressed(req.headers(), req.uri())
            || cross_site(req.headers(), req.method(), req.uri().path())
        {
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

/// A host and port as a request names them (`Host`, or the URI's authority as HTTP/2 sends it).
#[derive(Debug, PartialEq)]
struct Named {
    /// Lowercased, without IPv6 brackets.
    name: String,
    /// `None` when the authority names no port (or 80, the default of `http`).
    port: Option<u16>,
}

/// `host[:port]`, parsed strictly: no user info, path or junk around it (`[::1].other.example`,
/// `[::1` and `user@localhost` are not hosts).
fn parse_authority(a: &str) -> Option<Named> {
    if a.is_empty() || a.contains(['/', '?', '#', '@', '\\']) {
        return None;
    }
    let u = url::Url::parse(&format!("http://{a}")).ok()?;
    if !u.username().is_empty() || u.password().is_some() || u.path() != "/" {
        return None;
    }
    let name = u
        .host_str()?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    Some(Named { name, port: u.port() })
}

/// What a request says about the host it is for: `Ok(None)` when it names none (no browser
/// does that), `Err` when what it names is unreadable, doubled or contradicts itself.
fn named_host(h: &HeaderMap, uri: &axum::http::Uri) -> Result<Option<Named>, ()> {
    let mut hosts = h.get_all(axum::http::header::HOST).iter();
    let header = match (hosts.next(), hosts.next()) {
        (Some(_), Some(_)) => return Err(()),
        (Some(v), None) => Some(parse_authority(v.to_str().map_err(|_| ())?).ok_or(())?),
        (None, _) => None,
    };
    let authority = match uri.authority() {
        Some(a) => Some(parse_authority(a.as_str()).ok_or(())?),
        None => None,
    };
    match (header, authority) {
        (Some(a), Some(b)) if a != b => Err(()),
        (Some(a), _) | (None, Some(a)) => Ok(Some(a)),
        (None, None) => Ok(None),
    }
}

/// A name that only ever means this machine: `localhost`, 127.0.0.0/8, `::1`. (Not other
/// `*.localhost` names: those go through a resolver.)
fn is_loopback_name(name: &str) -> bool {
    name == "localhost" || name.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// What isn't a page another site may open: the API, Subsonic, the MusicBrainz proxy and the
/// renderers' UPnP routes (a zone's control takes a POST, its description a GET). Everything else
/// the node serves outside these is a GET: the web app's pages and files, a party page, the
/// sign-in callbacks.
const NOT_PAGES: &[&str] = &["/v1/", "/rest/", "/mb/", "/upnp/"];

/// A request a browser made for a page of another site (`Sec-Fetch-Site: cross-site` or
/// `same-site`: another port of this machine is the same site). For a node without a token only
/// a top-level navigation to one of its pages is that: the web app, a sign-in callback. Not its
/// API, Subsonic, the MusicBrainz proxy or the UPnP routes (`NOT_PAGES`), which change things on
/// GET too; only by GET or HEAD (a form another site posts navigates as well); and not a frame (a
/// hidden one would reach a page with side effects, a sign-in callback, before the page's
/// `frame-ancestors` keeps it from showing): `Sec-Fetch-Dest` must be `document`. Browsers that
/// send no `Sec-Fetch-Site` pass on the other checks.
pub fn cross_site(h: &HeaderMap, method: &axum::http::Method, path: &str) -> bool {
    let site = h
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !matches!(site, "cross-site" | "same-site") {
        return false;
    }
    let navigation = h.get("sec-fetch-mode").and_then(|v| v.to_str().ok()) == Some("navigate")
        && h.get("sec-fetch-dest").and_then(|v| v.to_str().ok()) == Some("document");
    // Opening a page only: a form another site posts (a top-level navigation too) never counts
    let opens = matches!(*method, axum::http::Method::GET | axum::http::Method::HEAD);
    let page = !NOT_PAGES.iter().any(|p| path.starts_with(p));
    !(navigation && opens && page)
}

/// The `Origin` a browser sent, parsed; `Err` for one that names no site (`null`) or can't be read.
fn origin_of(h: &HeaderMap) -> Result<Option<url::Url>, ()> {
    match h.get(axum::http::header::ORIGIN) {
        None => Ok(None),
        Some(o) => {
            let u = url::Url::parse(o.to_str().map_err(|_| ())?).map_err(|_| ())?;
            if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() {
                return Err(());
            }
            Ok(Some(u))
        }
    }
}

/// Whether an origin is that host: the same name, and the same port, where a host that names no
/// port means the origin's own default (browsers leave 80 and 443 out of `Origin`, proxies often
/// keep them in `X-Forwarded-Host`).
fn origin_is(origin: &url::Url, host: &Named) -> bool {
    let name = origin
        .host_str()
        .unwrap_or_default()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    name == host.name
        && match host.port {
            None => origin.port().is_none(),
            Some(p) => origin.port_or_known_default() == Some(p),
        }
}

/// Whether a request to a node without a token comes from the node's own pages, or from a
/// program that isn't a browser:
/// - the host it names is a loopback name (any other name is refused, and so is an unreadable,
///   doubled or contradictory host);
/// - an `Origin`, when the browser sends one, is that very host and port (pages elsewhere, other
///   local apps on other ports and `null` origins are refused).
///
/// A request naming no host at all (not a browser: they always do) passes on the source checks.
pub fn loopback_addressed(h: &HeaderMap, uri: &axum::http::Uri) -> bool {
    let host = match named_host(h, uri) {
        Ok(Some(host)) => host,
        Ok(None) => return true,
        Err(()) => return false,
    };
    if !is_loopback_name(&host.name) {
        return false;
    }
    match origin_of(h) {
        Ok(None) => true,
        Ok(Some(o)) => origin_is(&o, &host),
        Err(()) => false,
    }
}

/// A WebSocket handshake from a page on this node, or from a program that isn't a browser (no
/// `Origin`). Checked in every mode: a page on another site never opens the node's event stream
/// through a visitor's browser, token or not. "This node" is the host the request names, the one
/// a reverse proxy says it was asked for (`X-Forwarded-Host`; proxies often rewrite `Host`), and
/// the node's configured public address.
pub fn ws_origin_ok(h: &HeaderMap, uri: &axum::http::Uri, public_url: &str) -> bool {
    let origin = match origin_of(h) {
        Ok(None) => return true,
        Ok(Some(o)) => o,
        Err(()) => return false,
    };
    let named = named_host(h, uri).ok().flatten();
    let forwarded = h
        .get("x-forwarded-host")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .and_then(|v| parse_authority(v.trim()));
    let public = url::Url::parse(public_url).ok().map(|u| u.origin());
    named
        .iter()
        .chain(forwarded.iter())
        .any(|host| origin_is(&origin, host))
        || public.is_some_and(|p| p == origin.origin())
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
    use axum::http::Method;

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
    /// another site sends through a visitor's browser.
    #[test]
    fn a_tokenless_node_answers_only_its_loopback_names_and_own_pages() {
        let uri: axum::http::Uri = "/v1/status".parse().unwrap();
        for ok in [
            vec![("host", "127.0.0.1:7788")],
            vec![("host", "localhost:7788")],
            vec![("host", "LOCALHOST:7788"), ("origin", "http://localhost:7788")],
            vec![("host", "[::1]:7788"), ("origin", "http://[::1]:7788")],
            vec![("host", "127.0.0.1:7788"), ("origin", "http://127.0.0.1:7788")],
            // A program that isn't a browser, naming no host at all
            vec![],
        ] {
            assert!(loopback_addressed(&heads(&ok), &uri), "{ok:?}");
        }
        for refused in [
            // Another name for the address
            vec![("host", "other.example:7788")],
            vec![("host", "127.0.0.1.other.example:7788")],
            vec![("host", "music.lan:7788")],
            // Names that go through a resolver
            vec![("host", "odeion.localhost:7788")],
            // Another site, another local app on another port, a sandboxed page
            vec![("host", "127.0.0.1:7788"), ("origin", "http://other.example")],
            vec![("host", "localhost:7788"), ("origin", "http://localhost:3000")],
            vec![("host", "127.0.0.1:7788"), ("origin", "null")],
        ] {
            assert!(!loopback_addressed(&heads(&refused), &uri), "{refused:?}");
        }
        // HTTP/2 names the host in the URI
        let abs: axum::http::Uri = "http://other.example:7788/v1/status".parse().unwrap();
        assert!(!loopback_addressed(&HeaderMap::new(), &abs));
        // What isn't a plain host is refused, never read as "no host"
        for junk in [
            "[::1].other.example:7788",
            "[::1]garbage",
            "[::1",
            "user@localhost:7788",
            "localhost:7788/x",
            "",
        ] {
            assert!(!loopback_addressed(&heads(&[("host", junk)]), &uri), "{junk:?}");
        }
        let mut two = heads(&[("host", "127.0.0.1:7788")]);
        two.append(axum::http::header::HOST, "other.example:7788".parse().unwrap());
        assert!(!loopback_addressed(&two, &uri), "two Host headers");
        let mut bad = HeaderMap::new();
        bad.insert(
            axum::http::header::HOST,
            axum::http::HeaderValue::from_bytes(b"local\xffhost").unwrap(),
        );
        assert!(!loopback_addressed(&bad, &uri), "an unreadable Host");
        let local_abs: axum::http::Uri = "http://127.0.0.1:7788/v1/status".parse().unwrap();
        assert!(loopback_addressed(
            &heads(&[("host", "127.0.0.1:7788")]),
            &local_abs
        ));
        assert!(
            !loopback_addressed(&heads(&[("host", "127.0.0.1:7788")]), &abs),
            "Host and URI disagree"
        );
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
        // A proxy that keeps the default port in X-Forwarded-Host
        assert!(ws_origin_ok(
            &heads(&[
                ("host", "127.0.0.1:7788"),
                ("x-forwarded-host", "music.example:443"),
                ("origin", "https://music.example")
            ]),
            &uri,
            ""
        ));
        assert!(ws_origin_ok(
            &heads(&[("host", "music.example"), ("origin", "https://music.example")]),
            &uri,
            ""
        ));
        assert!(
            !ws_origin_ok(
                &heads(&[
                    ("host", "music.example:8443"),
                    ("origin", "https://music.example")
                ]),
                &uri,
                ""
            ),
            "another port is another origin"
        );
        for o in [
            "http://other.example",
            "null",
            "https://music.example.other.example",
        ] {
            assert!(
                !ws_origin_ok(&heads(&[("host", "127.0.0.1:7788"), ("origin", o)]), &uri, public),
                "{o}"
            );
        }
    }

    /// Requests another site's page makes reach a node without a token only as top-level
    /// navigations to its pages (a sign-in callback); never its API, Subsonic or proxy, GET
    /// included, and never in a frame.
    #[test]
    fn other_sites_reach_only_pages_by_navigation() {
        let h = |site: &str, mode: &str| {
            heads(&[
                ("sec-fetch-site", site),
                ("sec-fetch-mode", mode),
                (
                    "sec-fetch-dest",
                    if mode == "navigate" { "document" } else { "empty" },
                ),
            ])
        };
        assert!(cross_site(
            &h("cross-site", "no-cors"),
            &Method::GET,
            "/rest/deletePlaylist"
        ));
        assert!(cross_site(
            &h("cross-site", "navigate"),
            &Method::GET,
            "/rest/jukeboxControl"
        ));
        assert!(cross_site(&h("cross-site", "cors"), &Method::GET, "/v1/info"));
        assert!(cross_site(
            &h("same-site", "no-cors"),
            &Method::GET,
            "/v1/library/scan"
        ));
        assert!(!cross_site(
            &h("cross-site", "navigate"),
            &Method::GET,
            "/tidal/callback"
        ));
        assert!(!cross_site(&h("cross-site", "navigate"), &Method::GET, "/"));
        assert!(!cross_site(&h("same-origin", "cors"), &Method::GET, "/v1/info"));
        assert!(
            !cross_site(&h("none", "navigate"), &Method::GET, "/v1/info"),
            "typed in the address bar"
        );
        // A frame on another site's page navigates too, but isn't a document
        for dest in ["iframe", "frame", "embed", "object", "fencedframe"] {
            let framed = heads(&[
                ("sec-fetch-site", "cross-site"),
                ("sec-fetch-mode", "navigate"),
                ("sec-fetch-dest", dest),
            ]);
            assert!(cross_site(&framed, &Method::GET, "/lastfm/callback"), "{dest}");
            assert!(cross_site(&framed, &Method::GET, "/"), "{dest}");
        }
        // A form on another site's page, posted to a page or a zone's UPnP control: a top-level
        // navigation, but no page is opened
        let doc = h("cross-site", "navigate");
        for method in [Method::POST, Method::PUT, Method::DELETE, Method::PATCH] {
            assert!(cross_site(&doc, &method, "/upnp/zone/z1/ctl/avt"), "{method}");
            assert!(cross_site(&doc, &method, "/"), "{method}");
            assert!(cross_site(&doc, &method, "/tidal/callback"), "{method}");
        }
        assert!(!cross_site(&doc, &Method::HEAD, "/"));
        assert!(
            cross_site(&doc, &Method::GET, "/upnp/zone/z1/desc.xml"),
            "the UPnP routes aren't pages"
        );
        assert!(cross_site(&doc, &Method::GET, "/upnp/node.xml"));
        let no_dest = heads(&[("sec-fetch-site", "cross-site"), ("sec-fetch-mode", "navigate")]);
        assert!(
            cross_site(&no_dest, &Method::GET, "/"),
            "a navigation that doesn't say what it is for"
        );
        assert!(
            !cross_site(&HeaderMap::new(), &Method::GET, "/v1/info"),
            "no Sec-Fetch headers"
        );
    }
}
