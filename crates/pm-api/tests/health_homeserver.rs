//! Exercise /v1/health through the real Pubky SDK against an isolated HTTPS homeserver.
//! Pkarr records are cached locally: no testnet, public network, or real account is needed.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use http_body_util::BodyExt;
use pm_api::sync::PubkySync;
use pm_api::{router, NodeSettings, NodeState};
use pm_library::Library;
use pm_pubky::homeserver::{NodeHomeserver, LOGIN_CAPABILITIES, PRIVATE_FILES_DIR};
use pubky::pkarr::dns::rdata::SVCB;
use pubky::pkarr::{Cache, InMemoryCache, SignedPacket};
use pubky::{Keypair, Pubky, PubkyHttpClient};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

struct Homeserver {
    owner: String,
    session: Vec<u8>,
    inventory: usize,
    status: StatusCode,
    stall: bool,
    requests: Mutex<Vec<(Method, String, bool)>>,
}

async fn answer(State(hs): State<Arc<Homeserver>>, request: Request) -> Response {
    let path = request.uri().path();
    if path == "/info" {
        return axum::Json(serde_json::json!({"features": []})).into_response();
    }
    // Only the fake session accepted at setup may read the private namespace.
    let expected = format!("{}=health-test-cookie", hs.owner);
    let authenticated = request.headers().get("cookie").and_then(|h| h.to_str().ok()) == Some(&expected);
    if path != "/session" {
        hs.requests.lock().unwrap().push((
            request.method().clone(),
            request.uri().to_string(),
            authenticated,
        ));
    }
    if !authenticated {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if path == "/session" {
        return hs.session.clone().into_response();
    }

    if path == PRIVATE_FILES_DIR && request.method() == Method::GET {
        if hs.stall {
            std::future::pending::<()>().await;
        }
        if hs.status != StatusCode::OK {
            return hs.status.into_response();
        }
        let params: std::collections::HashMap<_, _> =
            url::form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
                .into_owned()
                .collect();
        let limit: usize = params.get("limit").unwrap().parse().unwrap();
        let start: usize = params
            .get("cursor")
            .and_then(|s| s.rsplit('/').next())
            .and_then(|s| s.parse().ok())
            .map_or(0, |n: usize| n + 1);
        let body = (start..hs.inventory.min(start + limit))
            .map(|i| format!("pubky://{}{PRIVATE_FILES_DIR}{i}\n", hs.owner))
            .collect::<String>();
        return body.into_response();
    }
    // Metadata is deliberately unavailable. A health check must not ask for it.
    StatusCode::SERVICE_UNAVAILABLE.into_response()
}

struct Fixture {
    hs: Arc<Homeserver>,
    app: axum::Router,
    sync: Arc<PubkySync>,
    server: axum_server::Handle<std::net::SocketAddr>,
    _dir: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.shutdown();
    }
}

impl Fixture {
    async fn new(inventory: usize) -> Self {
        Self::with_response(inventory, StatusCode::OK, false, LOGIN_CAPABILITIES).await
    }

    #[allow(deprecated)] // A fake legacy session avoids implementing grant exchange in the fixture.
    async fn with_response(inventory: usize, status: StatusCode, stall: bool, capabilities: &str) -> Self {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let owner = Keypair::random();
        let homeserver = Keypair::random();
        let hs = Arc::new(Homeserver {
            owner: owner.public_key().z32(),
            session: pubky::CookieSessionRecord::new(
                &owner.public_key(),
                capabilities.parse().unwrap(),
                None,
            )
            .serialize(),
            inventory,
            status,
            stall,
            requests: Default::default(),
        });
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let tls = axum_server::tls_rustls::RustlsConfig::from_pem(
            cert.cert.pem().into_bytes(),
            cert.signing_key.serialize_pem().into_bytes(),
        )
        .await
        .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = axum_server::Handle::new();
        let handle = server.clone();
        let app = axum::Router::new().fallback(answer).with_state(hs.clone());
        tokio::spawn(async move {
            axum_server::from_tcp_rustls(listener, tls)
                .unwrap()
                .handle(handle)
                .serve(app.into_make_service())
                .await
                .unwrap();
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), server.listening())
            .await
            .unwrap()
            .unwrap();

        let cache = Arc::new(InMemoryCache::new(NonZeroUsize::new(2).unwrap()));
        let mut target = SVCB::new(1, "localhost".try_into().unwrap());
        target.set_port(port);
        let hs_packet = SignedPacket::builder()
            .https(".".try_into().unwrap(), target, 3600)
            .sign(&homeserver)
            .unwrap();
        let user_packet = SignedPacket::builder()
            .https(
                "_pubky".try_into().unwrap(),
                SVCB::new(0, homeserver.public_key().z32().as_str().try_into().unwrap()),
                3600,
            )
            .sign(&owner)
            .unwrap();
        cache.put(&hs_packet.public_key().into(), &hs_packet);
        cache.put(&user_packet.public_key().into(), &user_packet);
        let mut builder = PubkyHttpClient::builder();
        builder.pkarr(|b| b.no_default_network().bootstrap(&["127.0.0.1:1"]).cache(cache));
        builder
            .add_root_certificates_pem(cert.cert.pem().as_bytes())
            .unwrap();
        let pubky = Pubky::with_client(builder.build().unwrap());
        let session = pubky
            .restore_session(&format!("{}:health-test-cookie", hs.owner))
            .await
            .unwrap();
        let sync = Arc::new(PubkySync::from_session(
            NodeHomeserver::from_pubky(pubky),
            session,
        ));
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("health.sqlite");
        let node = NodeState::new(
            NodeSettings {
                name: "health-test".into(),
                version: "0".into(),
                db_path: db.clone(),
                scan_folders: vec![],
                art_dir: dir.path().join("art"),
                mirror_dir: dir.path().join("mirror"),
                pipeline: Default::default(),
                token: Some("node-health-test".into()),
                subsonic_user: "pubky".into(),
                pubky: pm_pubky::PmPubkyConfig {
                    network: pm_pubky::Network::Testnet,
                    ..Default::default()
                },
                public_url: "http://127.0.0.1:1".into(),
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
            pm_zones::OutputRegistry::new(),
        );
        node.set_sync(Some(sync.clone()));
        Self {
            hs,
            app: router(node),
            sync,
            server,
            _dir: dir,
        }
    }

    async fn health(&self) -> serde_json::Value {
        let response = self
            .app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/health")
                    .header("authorization", "Bearer node-health-test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let parts: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
        parts.into_iter().find(|p| p["id"] == "homeserver").unwrap()
    }
}

#[tokio::test]
async fn a_missing_private_directory_is_healthy() {
    let fixture = Fixture::with_response(0, StatusCode::NOT_FOUND, false, LOGIN_CAPABILITIES).await;
    assert_eq!(fixture.health().await["state"], "ok");
    assert_eq!(fixture.hs.requests.lock().unwrap().len(), 1);
    assert!(!fixture.sync.needs_resign());
}

#[tokio::test]
async fn access_refusals_are_not_mistaken_for_a_healthy_empty_directory() {
    let fixture = Fixture::with_response(0, StatusCode::FORBIDDEN, false, LOGIN_CAPABILITIES).await;
    let part = fixture.health().await;
    assert_eq!(part["state"], "down");
    assert_eq!(part["detail"], pm_api::sync::RESIGN_NOTICE);
    assert!(fixture.sync.needs_resign());
    assert_eq!(fixture.hs.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn missing_capabilities_need_sign_in_without_a_storage_request() {
    let fixture = Fixture::with_response(0, StatusCode::OK, false, "/pub/pubky.app/:r").await;
    let part = fixture.health().await;
    assert_eq!(part["state"], "needs_sign_in");
    assert_eq!(part["detail"], pm_api::sync::RESIGN_NOTICE);
    assert!(fixture.hs.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn storage_errors_still_report_down_without_expiring_the_session() {
    for status in [StatusCode::SERVICE_UNAVAILABLE, StatusCode::UNAUTHORIZED] {
        let fixture = Fixture::with_response(0, status, false, LOGIN_CAPABILITIES).await;
        let part = fixture.health().await;
        assert_eq!(part["state"], "down");
        assert_eq!(part["detail"], "no answer");
        assert!(!fixture.sync.needs_resign());
        assert_eq!(fixture.hs.requests.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn a_stalled_storage_request_keeps_the_four_second_deadline() {
    let fixture = Fixture::with_response(0, StatusCode::OK, true, LOGIN_CAPABILITIES).await;
    let part = tokio::time::timeout(std::time::Duration::from_secs(6), fixture.health())
        .await
        .unwrap();
    assert_eq!(part["state"], "down");
    assert_eq!(part["detail"], "no answer in 4 s");
    assert!(!fixture.sync.needs_resign());
    assert_eq!(fixture.hs.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn health_uses_one_bounded_authenticated_list_regardless_of_inventory() {
    for inventory in [101, 0, 1, 100_000] {
        let fixture = Fixture::new(inventory).await;
        let part = fixture.health().await;
        assert_eq!(part["state"], "ok", "inventory={inventory}: {part}");
        let requests = fixture.hs.requests.lock().unwrap();
        assert_eq!(requests.len(), 1, "inventory={inventory}: {requests:?}");
        assert_eq!(requests[0].0, Method::GET);
        assert!(requests[0].2, "the one storage request must be authenticated");
        let url = url::Url::parse(&format!("https://localhost{}", requests[0].1)).unwrap();
        assert_eq!(url.path(), PRIVATE_FILES_DIR);
        let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(params.get("limit").map(String::as_str), Some("1"));
        assert!(params.contains_key("shallow"));
        assert!(!params.contains_key("cursor"));
    }
}
