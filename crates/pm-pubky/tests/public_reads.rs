//! Reading another person's public data must not reach the node's own network: not a homeserver
//! whose record names a local address, and not a local address it redirects to (here a second
//! local server). No testnet, no DHT: the records sit in the client's cache.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use pm_pubky::homeserver::NodeHomeserver;
use pm_pubky::schemas::Visibility;
use pubky::pkarr::dns::rdata::SVCB;
use pubky::pkarr::{Cache, InMemoryCache, Keypair, SignedPacket};
use pubky::{Pubky, PubkyHttpClient};

/// A plain HTTP server on this machine that counts what reaches it.
async fn second_server() -> (u16, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let counted = hits.clone();
    let app = axum::Router::new().fallback(move || {
        let counted = counted.clone();
        async move {
            counted.fetch_add(1, Ordering::SeqCst);
            "{\"stolen\":true}"
        }
    });
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (port, hits)
}

/// A homeserver at `https://localhost:<port>` that redirects every request to `to`, counting them.
async fn redirecting_homeserver(
    to: String,
) -> (
    u16,
    String,
    Arc<AtomicUsize>,
    axum_server::Handle<std::net::SocketAddr>,
) {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
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
    let handle = axum_server::Handle::new();
    let serving = handle.clone();
    let hits = Arc::new(AtomicUsize::new(0));
    let counted = hits.clone();
    let app = axum::Router::new().fallback(move || {
        let to = to.clone();
        counted.fetch_add(1, Ordering::SeqCst);
        async move { axum::response::Redirect::temporary(&to) }
    });
    tokio::spawn(async move {
        axum_server::from_tcp_rustls(listener, tls)
            .unwrap()
            .handle(serving)
            .serve(app.into_make_service())
            .await
            .unwrap();
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), handle.listening())
        .await
        .unwrap()
        .unwrap();
    (port, cert.cert.pem(), hits, handle)
}

/// A node on the public network, and someone whose homeserver record names a server at
/// `https://localhost:<port>` (by an ICANN name) that redirects every request to a second local
/// server. Returns the node, that person, and the two servers' hit counters.
async fn node_and_redirecting_user() -> (
    NodeHomeserver,
    Keypair,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
    axum_server::Handle<std::net::SocketAddr>,
) {
    let (second, second_hits) = second_server().await;
    let (port, pem, homeserver_hits, server) =
        redirecting_homeserver(format!("http://127.0.0.1:{second}/stolen")).await;

    let (user, homeserver) = (Keypair::random(), Keypair::random());
    let mut target = SVCB::new(1, "localhost".try_into().unwrap());
    target.set_port(port);
    let hs_packet = SignedPacket::builder()
        .https(".".try_into().unwrap(), target, 3600)
        .sign(&homeserver)
        .unwrap();
    let user_packet = SignedPacket::builder()
        .https(
            "_pubky".try_into().unwrap(),
            SVCB::new(0, homeserver.public_key().to_z32().as_str().try_into().unwrap()),
            3600,
        )
        .sign(&user)
        .unwrap();
    let cache = Arc::new(InMemoryCache::new(NonZeroUsize::new(4).unwrap()));
    cache.put(&hs_packet.public_key().into(), &hs_packet);
    cache.put(&user_packet.public_key().into(), &user_packet);
    let mut builder = PubkyHttpClient::builder();
    builder.pkarr(|b| {
        b.no_default_network()
            .bootstrap(&["127.0.0.1:1"])
            .cache(cache.clone() as Arc<dyn Cache>)
    });
    builder.add_root_certificates_pem(pem.as_bytes()).unwrap();
    let node = NodeHomeserver::from_pubky_on(
        Pubky::with_client(builder.build().unwrap()),
        pm_pubky::Network::Mainnet,
    );
    (node, user, second_hits, homeserver_hits, server)
}

#[tokio::test]
async fn a_homeserver_cannot_redirect_the_node_into_its_own_network() {
    let (node, user, second_hits, homeserver_hits, server) = node_and_redirecting_user().await;

    let read = node
        .get_bytes(
            &user.public_key().to_z32(),
            "/pub/pubky.app/profile.json",
            Visibility::Public,
            None,
        )
        .await;
    assert!(read.is_err(), "read through the redirect: {read:?}");
    assert_eq!(
        second_hits.load(Ordering::SeqCst),
        0,
        "the node's GET reached the second server"
    );
    assert_eq!(
        homeserver_hits.load(Ordering::SeqCst),
        0,
        "a homeserver at a local address is not reached for someone else's data"
    );
    server.shutdown();
}

/// The node's signed-in owner may keep their homeserver on their own network: reads of their
/// own public data keep the SDK's path also where no session is passed (sync, comments).
#[tokio::test]
async fn the_owners_own_public_data_keeps_the_sdk_path() {
    let (node, user, _second_hits, homeserver_hits, server) = node_and_redirecting_user().await;
    let z32 = user.public_key().to_z32();
    let owner = pm_pubky::homeserver::PublicKey::try_from(z32.as_str()).unwrap();

    node.set_owner(Some(owner));
    let _ = node
        .get_bytes(&z32, "/pub/pubky.app/profile.json", Visibility::Public, None)
        .await;
    let reached = homeserver_hits.load(Ordering::SeqCst);
    assert!(reached > 0, "the owner's own homeserver was not reached");

    // Signed out: the same person is someone else again
    node.set_owner(None);
    let read = node
        .get_bytes(&z32, "/pub/pubky.app/profile.json", Visibility::Public, None)
        .await;
    assert!(
        read.is_err(),
        "read through the redirect after sign-out: {read:?}"
    );
    assert_eq!(
        homeserver_hits.load(Ordering::SeqCst),
        reached,
        "a homeserver at a local address is reached after the owner signed out"
    );
    server.shutdown();
}
