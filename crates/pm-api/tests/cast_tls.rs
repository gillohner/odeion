//! The Cast link's TLS set-up inside the node: this test program links the node's other TLS code
//! as the node itself does.

use pm_out_cast::link::{connect_tls, permissive_tls, HANDSHAKE_TIMEOUT};
use pm_out_cast::CastError;

/// The node's other TLS code builds rustls with one crypto back end and the Cast output with
/// another. With both in one program rustls has no default to pick, so the Cast link names its
/// own: making its configuration works here, where nothing has chosen a default.
#[test]
fn the_cast_link_makes_its_tls_configuration_inside_the_node() {
    assert!(
        rustls::crypto::CryptoProvider::get_default().is_none(),
        "nothing in this program has chosen a default back end"
    );
    let cfg = permissive_tls();
    assert!(cfg.alpn_protocols.is_empty());
}

/// A device that takes the connection and never answers the handshake is given up on, and one
/// that hangs up is an error at once: neither holds the caller.
#[tokio::test]
async fn a_cast_device_that_does_not_answer_the_handshake_is_given_up_on() {
    let silent = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = silent.local_addr().unwrap();
    let held = tokio::spawn(async move {
        let (conn, _) = silent.accept().await.unwrap();
        tokio::time::sleep(HANDSHAKE_TIMEOUT * 4).await;
        drop(conn);
    });
    let started = std::time::Instant::now();
    let r = connect_tls(addr).await;
    assert!(
        matches!(r, Err(CastError::Timeout("TLS handshake"))),
        "{:?}",
        r.err()
    );
    assert!(started.elapsed() < HANDSHAKE_TIMEOUT * 2);
    held.abort();

    let closing = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = closing.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = closing.accept().await;
    });
    let r = connect_tls(addr).await;
    assert!(matches!(r, Err(CastError::Tls(_))), "{:?}", r.err());
}
