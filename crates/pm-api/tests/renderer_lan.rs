//! The renderer routes take no token, so the node serves them to LAN peers only: not through the
//! iroh tunnel's listener, not through a proxy or Cloudflare. A node without an API token answers
//! nothing through the tunnel (security review 2026-10-27, #2).

use pm_api::{NodeSettings, NodeState};
use pm_audio::PipelineConfig;
use pm_library::Library;
use pm_zones::OutputRegistry;
use std::sync::Arc;

fn node(dir: &std::path::Path, token: Option<&str>) -> Arc<NodeState> {
    let db = dir.join("lib.sqlite");
    NodeState::new(
        NodeSettings {
            name: "t".into(),
            version: "0".into(),
            db_path: db.clone(),
            scan_folders: vec![],
            art_dir: dir.join("art"),
            mirror_dir: dir.join("homeserver"),
            pipeline: PipelineConfig::default(),
            token: token.map(str::to_string),
            subsonic_user: "pubky".into(),
            pubky: Default::default(),
            public_url: "http://127.0.0.1:7788".into(),
            upnp: false,
            cast: false,
            slimproto_port: 0,
            session_secret_path: None,
            tidal: Default::default(),
            spotify: Default::default(),
            acoustid_key: String::new(),
            jamendo: Default::default(),
        },
        Library::open(&db).unwrap(),
        OutputRegistry::new(),
    )
}

/// The node's own listener and the tunnel's, as base URLs.
async fn serve(s: Arc<NodeState>) -> (String, String) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let direct = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(pm_api::serve_listener_with_pages(s.clone(), l, pm_web::router()));
    let remote = format!(
        "http://{}",
        pm_api::peer::serve_remote(s, pm_web::router()).await.unwrap()
    );
    (direct, remote)
}

async fn status(url: &str, headers: &[(&str, &str)]) -> u16 {
    let mut r = reqwest::Client::new().get(url);
    for (k, v) in headers {
        r = r.header(*k, *v);
    }
    r.send().await.unwrap().status().as_u16()
}

#[tokio::test]
async fn the_renderer_answers_lan_peers_only() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some("owner-token"));
    let _ssdp = s.start_renderer(0, false);
    let (direct, remote) = serve(s).await;
    // No zones here, so a request that gets through is a 404; one refused is a 403
    let desc = "/upnp/zone/kitchen/desc.xml";
    assert_eq!(
        status(&format!("{direct}{desc}"), &[]).await,
        404,
        "a peer on this machine"
    );
    assert_eq!(status(&format!("{remote}{desc}"), &[]).await, 403, "through iroh");
    for h in ["x-forwarded-for", "cf-connecting-ip", "forwarded", "x-real-ip"] {
        assert_eq!(
            status(&format!("{direct}{desc}"), &[(h, "203.0.113.9")]).await,
            403,
            "through a proxy ({h})"
        );
    }
    // The rest of the API still needs the token, the same on both listeners
    assert_eq!(status(&format!("{remote}/v1/info"), &[]).await, 401);
    let bearer = [("authorization", "Bearer owner-token")];
    assert_eq!(status(&format!("{remote}/v1/info"), &bearer).await, 200);
    assert_eq!(status(&format!("{direct}/v1/info"), &bearer).await, 200);
}

/// The page and its files need no token and come through the tunnel too: the app away from home
/// opens the same page, and keeps a copy of these files.
#[tokio::test]
async fn the_page_is_served_on_both_listeners() {
    let dir = tempfile::tempdir().unwrap();
    let (direct, remote) = serve(node(dir.path(), Some("owner-token"))).await;
    for base in [&direct, &remote] {
        for path in [
            "/",
            "/app",
            "/albums",
            "/settings/about",
            "/party/abc",
            "/app.js",
            "/app.css",
            "/usage.js",
            "/feedback.js",
            "/feedback.css",
            "/pwa.js",
            "/pwa.css",
            "/sw.js",
            "/manifest.webmanifest",
            "/icon.svg",
            "/icon-192.png",
            "/icon-512.png",
            "/icon-maskable-512.png",
            "/apple-touch-icon.png",
        ] {
            assert_eq!(status(&format!("{base}{path}"), &[]).await, 200, "{base}{path}");
        }
        // (what the page holds is pm-web's own test; here: it is a page, on this listener too)
        let page = reqwest::get(format!("{base}/")).await.unwrap();
        let kind = page.headers()["content-type"].to_str().unwrap().to_string();
        assert!(kind.starts_with("text/html"), "{base}: {kind}");
    }
}

#[tokio::test]
async fn a_node_without_a_token_answers_only_directly() {
    let dir = tempfile::tempdir().unwrap();
    let (direct, remote) = serve(node(dir.path(), None)).await;
    assert_eq!(status(&format!("{direct}/v1/info"), &[]).await, 200);
    assert_eq!(status(&format!("{remote}/v1/info"), &[]).await, 401);
    assert_eq!(
        status(
            &format!("{direct}/v1/info"),
            &[("cf-connecting-ip", "203.0.113.9")]
        )
        .await,
        401
    );
    assert_eq!(
        status(&format!("{remote}/rest/ping"), &[]).await,
        401,
        "Subsonic too"
    );
}

async fn post_status(url: &str, headers: &[(&str, &str)]) -> u16 {
    let mut r = reqwest::Client::new().post(url).body("<s:Envelope/>");
    for (k, v) in headers {
        r = r.header(*k, *v);
    }
    r.send().await.unwrap().status().as_u16()
}

/// A page on another site, opened by someone on the LAN, must not drive the renderer through
/// their browser, also when the node has an API token (the renderer routes take none).
#[tokio::test]
async fn other_sites_never_reach_the_renderer_through_a_browser() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path(), Some("owner-token"));
    let _ssdp = s.start_renderer(0, false);
    let (direct, _) = serve(s).await;
    let host = direct.trim_start_matches("http://").to_string();
    // No zones here, so a request that gets through is a 404; one refused is a 403
    let ctl = format!("{direct}/upnp/zone/kitchen/ctl/avt");
    let desc = format!("{direct}/upnp/zone/kitchen/desc.xml");
    let soap = [
        ("content-type", "text/xml"),
        (
            "soapaction",
            "\"urn:schemas-upnp-org:service:AVTransport:1#Play\"",
        ),
    ];
    for site in ["cross-site", "same-site"] {
        let h = [("sec-fetch-site", site), ("sec-fetch-mode", "no-cors")];
        assert_eq!(post_status(&ctl, &h).await, 403, "a POST ({site})");
        assert_eq!(status(&desc, &h).await, 403, "a GET ({site})");
    }
    for origin in ["http://evil.example", "null", "http://127.0.0.1:1"] {
        assert_eq!(
            post_status(&ctl, &[("origin", origin)]).await,
            403,
            "Origin {origin}"
        );
    }
    // A page on another site whose name was made to resolve to the node (DNS rebinding): to
    // the browser it is the same origin, so only the name it is reached by tells
    let rebound = [
        soap[0],
        soap[1],
        ("host", "evil.example"),
        ("origin", "http://evil.example"),
        ("sec-fetch-site", "same-origin"),
    ];
    assert_eq!(post_status(&ctl, &rebound).await, 403, "DNS rebinding");
    // A control point (no Sec-Fetch headers, no Origin), and the node's own pages
    assert_eq!(post_status(&ctl, &soap).await, 404);
    assert_eq!(status(&desc, &[]).await, 404);
    let own = format!("http://{host}");
    let from_own_page = [soap[0], soap[1], ("origin", own.as_str())];
    assert_eq!(post_status(&ctl, &from_own_page).await, 404);
    assert_eq!(status(&desc, &[("sec-fetch-site", "same-origin")]).await, 404);
    // GENA NOTIFY from renderers takes the same layer
    let notify = format!("{direct}/v1/upnp/events/nope");
    assert_eq!(
        post_status(&notify, &[("sec-fetch-site", "cross-site")]).await,
        403
    );
    // ... and refuses a rebound name too; by the node's address it gets as far as the method
    assert_eq!(
        post_status(&notify, &rebound[2..]).await,
        403,
        "GENA, DNS rebinding"
    );
    assert_eq!(post_status(&notify, &[]).await, 405);
}
