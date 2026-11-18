//! Local stand-ins for the public relays a Ring sign-in goes through, for the tests that run
//! against the bundle's real homeserver (`bundled_homeserver`, `vault_api`, `vault_homeserver`):
//!
//! - an HTTP relay (the `http-relay` pubky's own tests run): the node's sign-in links name it, so
//!   the test signer posts its approval there and the node reads it there;
//! - a pkarr relay: every test client (node and signer) also publishes to and reads from it, on
//!   top of the public network, so a new account's record is found here at once.
//!
//! A slow public relay left approvals and new accounts' records undelivered, which failed these
//! tests at random. The bundled homeserver publishes its own record on the public network only:
//! it is looked up there once when the relays start (a bounded retry that names the failure) and
//! kept in the local pkarr relay, so no test's lookup waits on a public relay. The clients keep the
//! public relays and the DHT too.
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

/// The relays, started on first use. A start that failed is kept too, so every test after it fails
/// at once with the same reason instead of starting them again.
fn relays() -> &'static Relays {
    static RELAYS: OnceLock<Result<Relays, String>> = OnceLock::new();
    let started = RELAYS.get_or_init(|| {
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
                let packets = Packets::default();
                let pkarr = pkarr_relay(packets.clone()).await;
                let seeded = seed_bundled_homeserver(&packets).await;
                tx.send(seeded.map(|()| Relays { inbox, pkarr })).unwrap();
                // Keeps both serving until the test binary ends
                let _http = http;
                std::future::pending::<()>().await;
            });
        });
        rx.recv()
            .unwrap_or_else(|_| Err("the local relays did not start".into()))
    });
    match started {
        Ok(relays) => relays,
        Err(why) => panic!("{why}"),
    }
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
        .is_some_and(|held| held.more_recent_than(&packet))
    {
        return StatusCode::CONFLICT.into_response();
    }
    packets.insert(z32, packet);
    StatusCode::NO_CONTENT.into_response()
}

async fn pkarr_relay(packets: Packets) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://127.0.0.1:{}/", listener.local_addr().unwrap().port());
    let app = axum::Router::new()
        .route("/{z32}", get(pkarr_get).put(pkarr_put))
        .with_state(packets);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

/// How often, and how far apart, the bundled homeserver's record is looked up on the public network.
const SEED_TRIES: u32 = 12;
const SEED_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

/// The bundled homeserver's own record (its key from the admin API's `/info`), looked up on the
/// public network and put in the local pkarr relay. Nothing to do without the admin API (the tests
/// skip) or on a local testnet (its records are local already).
async fn seed_bundled_homeserver(packets: &Packets) -> Result<(), String> {
    let (Ok(admin), Ok(password)) = (
        std::env::var("PM_BUNDLED_HS_ADMIN"),
        std::env::var("PM_BUNDLED_HS_PASSWORD"),
    ) else {
        return Ok(());
    };
    if std::env::var("PM_TESTNET").as_deref() == Ok("1") {
        return Ok(());
    }
    let info: serde_json::Value = reqwest::Client::new()
        .get(format!("{}/info", admin.trim_end_matches('/')))
        .header("X-Admin-Password", password)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("the bundled homeserver's admin API: {e}"))?
        .json()
        .await
        .map_err(|e| format!("the bundled homeserver's /info: {e}"))?;
    let z32 = info["public_key"].as_str().unwrap_or_default();
    let z32 = z32.strip_prefix("pubky").filter(|k| k.len() == 52).unwrap_or(z32);
    let key = PublicKey::try_from(z32).map_err(|e| format!("the bundled homeserver's key {z32}: {e}"))?;
    let public = PubkyHttpClient::new().map_err(|e| e.to_string())?;
    let mut last = String::new();
    for _ in 0..SEED_TRIES {
        match public
            .pkarr()
            .resolve(&key, pubky::pkarr::ResolvePolicy::NetworkOnly)
            .await
        {
            Ok(packet) => {
                packets
                    .lock()
                    .unwrap()
                    .insert(packet.public_key().to_string(), packet);
                return Ok(());
            }
            Err(e) => last = e.to_string(),
        }
        tokio::time::sleep(SEED_EVERY).await;
    }
    Err(format!(
        "the bundled homeserver's record ({z32}) was not found on the public network in {SEED_TRIES} tries {}s apart: {last}",
        SEED_EVERY.as_secs()
    ))
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
