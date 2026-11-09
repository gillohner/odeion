//! The newest PKARR record wins before a sign-in routes on it (`pm_pubky::records`), against a
//! fake pkarr relay: no testnet, no DHT.

use std::num::NonZeroUsize;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use pm_pubky::records::{freshen, freshen_one, From};
use pubky::pkarr::dns::rdata::SVCB;
use pubky::pkarr::{
    Cache, CacheKey, InMemoryCache, Keypair, PublicKey, ResolvePolicy, SignedPacket, Timestamp,
};
use pubky::PubkyHttpClient;

/// A record of `homeserver` naming `icann` as its ICANN endpoint, dated `at` (microseconds).
fn record(homeserver: &Keypair, icann: &str, at: u64) -> SignedPacket {
    let mut direct = SVCB::new(1, ".".try_into().unwrap());
    direct.set_port(6287);
    let mut name = SVCB::new(10, icann.try_into().unwrap());
    name.set_port(443);
    SignedPacket::builder()
        .https(".".try_into().unwrap(), direct, 3600)
        .https(".".try_into().unwrap(), name, 3600)
        .address(".".try_into().unwrap(), "139.162.128.88".parse().unwrap(), 3600)
        .timestamp(Timestamp::from(at))
        .sign(homeserver)
        .unwrap()
}

fn icann_of(p: &SignedPacket) -> Vec<String> {
    p.all_resource_records()
        .filter_map(|rr| match &rr.rdata {
            pubky::pkarr::dns::rdata::RData::HTTPS(h) => Some(h.0.target.to_string()),
            _ => None,
        })
        .filter(|t| !t.is_empty() && t != ".")
        .collect()
}

/// A fake relay: `GET /{z32}` answers the packet it holds for that key (the relay wire payload),
/// else 404. `served` counts the answers.
struct Relay {
    packets: std::sync::Mutex<Vec<SignedPacket>>,
    served: std::sync::atomic::AtomicUsize,
}

async fn relay_get(State(r): State<Arc<Relay>>, Path(z32): Path<String>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let packets = r.packets.lock().unwrap();
    match packets.iter().find(|p| p.public_key().to_string() == z32) {
        Some(p) => {
            r.served.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (StatusCode::OK, p.to_relay_payload().to_vec()).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn relay(packets: Vec<SignedPacket>) -> (String, Arc<Relay>) {
    let r = Arc::new(Relay {
        packets: std::sync::Mutex::new(packets),
        served: Default::default(),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://127.0.0.1:{}/", listener.local_addr().unwrap().port());
    let app = axum::Router::new()
        .route("/{z32}", get(relay_get))
        .with_state(r.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, r)
}

/// A client on `relay` only (no DHT), whose cache holds `cached`.
fn client(relay: &str, cached: &[SignedPacket]) -> (PubkyHttpClient, Arc<InMemoryCache>) {
    let cache = Arc::new(InMemoryCache::new(NonZeroUsize::new(16).unwrap()));
    for p in cached {
        let key: CacheKey = p.public_key().into();
        cache.put(&key, p);
    }
    let mut b = PubkyHttpClient::builder();
    let (relay, c) = (relay.to_string(), cache.clone());
    b.pkarr(move |p| {
        p.no_dht();
        p.relays(&[relay.as_str()]).unwrap();
        p.cache(c.clone() as Arc<dyn Cache>)
    });
    (b.build().unwrap(), cache)
}

/// The cache holds an older record of the homeserver's (the first answer some node gave, naming
/// `localhost`); the network has the newest. The plain lookup the SDK's routing uses answers the
/// old one; the sign-in's network-first step answers the newest and leaves it in the cache, so
/// the routing that follows uses it.
#[tokio::test]
async fn the_newest_record_wins_over_an_older_cached_one() {
    let hs = Keypair::random();
    let old = record(&hs, "localhost", 1_761_700_000_000_000);
    let new = record(&hs, "homeserver.example.org", 1_762_200_000_000_000);
    let (url, r) = relay(vec![new.clone()]).await;
    let (c, cache) = client(&url, std::slice::from_ref(&old));
    let pk: PublicKey = hs.public_key();
    // The problem: a fresh cached record is what the routing's lookup takes
    let routed = c.pkarr().resolve(&pk, ResolvePolicy::CacheFirst).await.unwrap();
    assert_eq!(icann_of(&routed), ["localhost"]);
    assert_eq!(
        r.served.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the network wasn't asked"
    );
    // Network-first: the newest, from the relay, and the cache holds it now
    let f = freshen_one(c.pkarr(), &pk).await;
    assert_eq!(
        (f.from, f.timestamp_us, f.loopback),
        (From::Network, Some(new.timestamp().as_u64()), false)
    );
    let routed = c.pkarr().resolve(&pk, ResolvePolicy::CacheFirst).await.unwrap();
    assert_eq!(icann_of(&routed), ["homeserver.example.org"]);
    let key: CacheKey = pk.clone().into();
    assert_eq!(cache.get(&key).unwrap().timestamp(), new.timestamp());
    // The network can't go backward: an older record served later doesn't replace it
    *r.packets.lock().unwrap() = vec![old.clone()];
    let f = freshen_one(c.pkarr(), &pk).await;
    assert_eq!(cache.get(&key).unwrap().timestamp(), new.timestamp(), "{f:?}");
    // ...and what is reported is the record that routes: the newer one the cache kept
    assert_eq!(f.timestamp_us, Some(new.timestamp().as_u64()), "{f:?}");
    assert_eq!(f.from, From::Network);
}

/// With no network the sign-in goes on with the copy held here, and says so.
#[tokio::test]
async fn without_the_network_the_cached_record_is_the_fallback() {
    let hs = Keypair::random();
    let old = record(&hs, "homeserver.example.org", 1_761_700_000_000_000);
    // A relay that is not there
    let (c, _) = client("http://127.0.0.1:9/", std::slice::from_ref(&old));
    let f = freshen_one(c.pkarr(), &hs.public_key()).await;
    assert_eq!(f.from, From::Cache, "{f:?}");
    assert_eq!(f.timestamp_us, Some(old.timestamp().as_u64()));
    assert!(f.error.is_some(), "why the network didn't answer: {f:?}");
    // Nothing known at all
    let unknown = Keypair::random().public_key();
    let f = freshen_one(c.pkarr(), &unknown).await;
    assert_eq!((f.from, f.timestamp_us), (From::None, None));
    // A key that isn't one is skipped, the rest answered
    let out = freshen(c.pkarr(), &["not-a-key", &hs.public_key().to_string()]).await;
    assert_eq!(out.len(), 1);
}

/// The newest record of a homeserver naming only the client's own machine is reported as such
/// (on the public network a sign-in would go nowhere; on a testnet it is the homeserver).
#[tokio::test]
async fn a_record_naming_only_localhost_is_reported() {
    let hs = Keypair::random();
    let mut name = SVCB::new(10, "localhost".try_into().unwrap());
    name.set_port(6287);
    let local = SignedPacket::builder()
        .https(".".try_into().unwrap(), name, 3600)
        .sign(&hs)
        .unwrap();
    let (url, _) = relay(vec![local]).await;
    let (c, _) = client(&url, &[]);
    let f = freshen_one(c.pkarr(), &hs.public_key()).await;
    assert_eq!((f.from, f.loopback), (From::Network, true), "{f:?}");
}

/// The whole sign-in step on the public network: a public homeserver the network has no record
/// of (here: a relay that knows nothing) refuses the sign-in before it starts, within the step's
/// bound; on a local testnet nothing is refused.
#[tokio::test]
async fn a_sign_in_with_no_homeserver_record_is_refused_before_it_starts() {
    let (url, _) = relay(vec![]).await;
    let (c, _) = client(&url, &[]);
    let hs = pm_pubky::homeserver::NodeHomeserver::from_pubky_on(
        pubky::Pubky::with_client(c.clone()),
        pm_pubky::Network::Mainnet,
    );
    let started = std::time::Instant::now();
    let r = hs.fresh_for_sign_in(None, None, false).await;
    assert!(r.is_err(), "{r:?}");
    assert!(r.unwrap_err().contains("couldn't be found"));
    assert!(started.elapsed() <= pm_pubky::records::SIGN_IN_LOOKUPS + std::time::Duration::from_secs(1));
    let testnet = pm_pubky::homeserver::NodeHomeserver::from_pubky_on(
        pubky::Pubky::with_client(c),
        pm_pubky::Network::Testnet,
    );
    assert!(testnet.fresh_for_sign_in(None, None, false).await.is_ok());
}
