//! Tests only: a homeserver that takes every connection and never answers (a dead connection after
//! a network switch), and the person's account on it, with nothing beyond this machine. A local
//! pkarr relay names a fresh homeserver key at a socket that accepts and holds each connection,
//! and the person's grant is made here, so their session exists without asking the homeserver.

use super::account::Account;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use pm_pubky::homeserver::NodeHomeserver;
use pubky::pkarr::dns::rdata::SVCB;
use pubky::pkarr::SignedPacket;
use pubky::{
    Capability, ClientId, GrantClaims, GrantCredential, GrantId, Keypair, Pubky, PubkyHttpClient,
    PubkySession, GRANT_JWS_TYP,
};
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) struct Stalled {
    pub(crate) hs: NodeHomeserver,
    /// The person's session secret, as the app keeps it in `pubky-session`.
    pub(crate) secret: String,
    client: PubkyHttpClient,
    /// Connections the homeserver took.
    pub(crate) taken: Arc<std::sync::atomic::AtomicUsize>,
}

impl Stalled {
    /// The person's account, its session made here (its first request goes to the homeserver).
    pub(crate) fn account(&self) -> Account {
        let cred = GrantCredential::from_shared_secret(&self.secret).expect("a grant secret");
        Account::of(
            self.hs.clone(),
            PubkySession::from_grant_credential(self.client.clone(), cred),
        )
    }
}

type Packets = Arc<HashMap<String, SignedPacket>>;

async fn packet(State(p): State<Packets>, Path(z32): Path<String>) -> Response {
    match p.get(&z32) {
        Some(packet) => (StatusCode::OK, packet.to_relay_payload()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The homeserver that never answers, and a client that finds it (records cached: what is left
/// is the homeserver itself, so a test can pause time from here).
pub(crate) async fn homeserver() -> Stalled {
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = socket.local_addr().unwrap().port();
    let taken = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let t2 = taken.clone();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((c, _)) = socket.accept().await {
            t2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            held.push(c);
        }
    });
    at(port, taken).await
}

/// A homeserver that is down: every connection is refused at once (an error, not a wait).
pub(crate) async fn refusing() -> Stalled {
    let port = {
        let gone = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        gone.local_addr().unwrap().port()
    };
    at(port, Default::default()).await
}

async fn at(port: u16, taken: Arc<std::sync::atomic::AtomicUsize>) -> Stalled {
    let (hs_key, user) = (Keypair::random(), Keypair::random());
    let mut svcb = SVCB::new(1, ".".try_into().unwrap());
    svcb.set_port(port);
    let hs_packet = SignedPacket::builder()
        .address(".".try_into().unwrap(), "127.0.0.1".parse().unwrap(), 3600)
        .https(".".try_into().unwrap(), svcb, 3600)
        .sign(&hs_key)
        .unwrap();
    let hs_z32 = hs_key.public_key().z32();
    let user_packet = SignedPacket::builder()
        .https(
            "_pubky".try_into().unwrap(),
            SVCB::new(0, hs_z32.as_str().try_into().unwrap()),
            3600,
        )
        .sign(&user)
        .unwrap();
    let packets: Packets = Arc::new(
        [
            (hs_z32.clone(), hs_packet),
            (user.public_key().z32(), user_packet),
        ]
        .into_iter()
        .collect(),
    );
    let relay = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_url = format!("http://127.0.0.1:{}/", relay.local_addr().unwrap().port());
    let app = axum::Router::new()
        .route("/{z32}", axum::routing::get(packet))
        .with_state(packets);
    tokio::spawn(async move { axum::serve(relay, app).await.unwrap() });
    let mut b = PubkyHttpClient::builder();
    b.pkarr(move |p| {
        p.no_default_network()
            .relays(&[relay_url.as_str()])
            .expect("a relay URL")
    });
    let client = b.build().expect("a Pubky client");
    // Both records found now, so nothing but the homeserver is left to wait for
    for key in [hs_key.public_key(), user.public_key()] {
        client
            .pkarr()
            .resolve(&key, pubky::pkarr::ResolvePolicy::CacheFirst)
            .await
            .expect("the local record");
    }
    let client_key = Keypair::random();
    let now = pm_sources::now_unix() as u64;
    let claims = GrantClaims {
        iss: user.public_key(),
        client_id: ClientId::new("odeion.test").unwrap(),
        caps: vec![Capability::root()],
        cnf: client_key.public_key(),
        jti: GrantId::generate(),
        iat: now,
        exp: now + 24 * 3600,
    };
    let secret = format!(
        "pubky-grant-credential-v1:{hs_z32}:{}:{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(client_key.secret()),
        claims.sign(&user, GRANT_JWS_TYP)
    );
    let hs = NodeHomeserver::from_pubky_on(Pubky::with_client(client.clone()), pm_pubky::Network::Mainnet);
    Stalled {
        hs,
        secret,
        client,
        taken,
    }
}

#[tokio::test]
async fn the_stand_in_never_answers() {
    let s = homeserver().await;
    let acct = s.account();
    // A read waits; the stand-in itself is found (no error at once)
    let read = acct.hs.read_prefs(&acct.session);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(1500), read)
            .await
            .is_err(),
        "the homeserver answered or the request failed"
    );
    assert!(
        s.taken.load(std::sync::atomic::Ordering::SeqCst) > 0,
        "the request reached the homeserver"
    );
}
