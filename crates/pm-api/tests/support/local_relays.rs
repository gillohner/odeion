//! Local stand-ins for the public relays a Ring sign-in goes through, for the tests that run
//! against the bundle's real homeserver (`bundled_homeserver`, `vault_api`, `vault_homeserver`):
//!
//! - an HTTP relay (the `http-relay` pubky's own tests run): the node's sign-in links name it, so
//!   the test signer posts its approval there and the node reads it there;
//! - a pkarr relay: every test client (node and signer) also publishes to and reads from it, on
//!   top of the public network, so a new account's record is found here at once.
//!
//! A slow public relay left approvals and new accounts' records undelivered, which failed these
//! tests at random. The bundled homeserver still publishes its own record on the public network
//! only, so the clients keep the public relays and the DHT too.
//!
//! Both relays run once per test binary on a runtime of their own, so every test (each on its own
//! runtime) can use them for as long as the binary runs.

#![allow(dead_code)] // each test binary uses only some of it

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use pm_pubky::config::Network;
use pm_pubky::homeserver::NodeHomeserver;
use pubky::pkarr::{PublicKey, SignedPacket};
use pubky::{Pubky, PubkyHttpClient};

struct Relays {
    /// The HTTP relay's inbox, e.g. `http://127.0.0.1:<port>/inbox`.
    inbox: url::Url,
    /// The pkarr relay, e.g. `http://127.0.0.1:<port>/`.
    pkarr: String,
}

fn relays() -> &'static Relays {
    static RELAYS: OnceLock<Relays> = OnceLock::new();
    RELAYS.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("the relays' runtime");
            rt.block_on(async move {
                let http = http_relay::HttpRelay::builder()
                    .http_port(0)
                    .run()
                    .await
                    .expect("a local HTTP relay");
                let inbox = http.local_url().join("inbox").unwrap();
                let pkarr = pkarr_relay().await;
                tx.send(Relays { inbox, pkarr }).unwrap();
                // Keeps both serving until the test binary ends
                let _http = http;
                std::future::pending::<()>().await;
            });
        });
        rx.recv().expect("the local relays started")
    })
}

/// Packets by key (z32); a newer one replaces an older one, as on a real relay.
type Packets = Arc<Mutex<HashMap<String, SignedPacket>>>;

async fn pkarr_get(State(p): State<Packets>, Path(z32): Path<String>) -> Response {
    match p.lock().unwrap().get(&z32) {
        Some(packet) => (StatusCode::OK, packet.to_relay_payload()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn pkarr_put(State(p): State<Packets>, Path(z32): Path<String>, body: Bytes) -> Response {
    let Ok(key) = PublicKey::try_from(z32.as_str()) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Ok(packet) = SignedPacket::from_relay_payload(&key, &body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let mut packets = p.lock().unwrap();
    if packets
        .get(&z32)
        .is_some_and(|held| held.timestamp() > packet.timestamp())
    {
        return StatusCode::CONFLICT.into_response();
    }
    packets.insert(z32, packet);
    StatusCode::NO_CONTENT.into_response()
}

async fn pkarr_relay() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://127.0.0.1:{}/", listener.local_addr().unwrap().port());
    let app = axum::Router::new()
        .route("/{z32}", get(pkarr_get).put(pkarr_put))
        .with_state(Packets::default());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

/// A Pubky client on the public network that also publishes to and resolves from the local pkarr
/// relay. For test signers and for the node's client ([`node_homeserver`]).
pub fn pubky() -> Pubky {
    let pkarr = relays().pkarr.clone();
    let mut b = PubkyHttpClient::builder();
    b.pkarr(move |p| p.extra_relays(&[pkarr.as_str()]).expect("a relay URL"));
    Pubky::with_client(b.build().expect("a Pubky client"))
}

/// The node's client: [`pubky`], with its Ring sign-ins through the local HTTP relay.
pub fn node_homeserver() -> NodeHomeserver {
    NodeHomeserver::from_pubky_on(pubky(), Network::Mainnet).with_auth_relay(inbox())
}

/// The local HTTP relay's inbox, for a client of the caller's own.
pub fn inbox() -> url::Url {
    relays().inbox.clone()
}
