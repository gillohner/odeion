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

/// An unknown Ring approver is not tied to PUBLIC. An explicitly selected missing public
/// homeserver still refuses within the step's bound; testnet retains its local policy.
#[tokio::test]
async fn unknown_ring_approver_does_not_select_the_public_homeserver() {
    let (url, _) = relay(vec![]).await;
    let (c, _) = client(&url, &[]);
    let hs = pm_pubky::homeserver::NodeHomeserver::from_pubky_on(
        pubky::Pubky::with_client(c.clone()),
        pm_pubky::Network::Mainnet,
    );
    let started = std::time::Instant::now();
    let r = hs.fresh_for_sign_in(None, None, false).await;
    assert!(r.is_ok(), "unrelated missing PUBLIC must not block Ring: {r:?}");
    assert!(started.elapsed() <= pm_pubky::records::SIGN_IN_LOOKUPS + std::time::Duration::from_secs(1));
    let public = pubky::PublicKey::try_from_z32(pm_pubky::records::PUBLIC_HOMESERVER).unwrap();
    let r = hs.fresh_for_sign_in(None, Some(&public), false).await;
    assert!(r.unwrap_err().contains("couldn't be found"));
    let testnet = pm_pubky::homeserver::NodeHomeserver::from_pubky_on(
        pubky::Pubky::with_client(c),
        pm_pubky::Network::Testnet,
    );
    assert!(testnet.fresh_for_sign_in(None, None, false).await.is_ok());
}

/// A user's record naming `homeserver` (`_pubky`), dated `at`.
fn user_record(user: &Keypair, homeserver: &PublicKey, at: u64) -> SignedPacket {
    SignedPacket::builder()
        .https(
            "_pubky".try_into().unwrap(),
            SVCB::new(0, homeserver.to_string().as_str().try_into().unwrap()),
            3600,
        )
        .timestamp(Timestamp::from(at))
        .sign(user)
        .unwrap()
}

/// A relay that takes requests and never answers.
async fn stalled_relay() -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://127.0.0.1:{}/", l.local_addr().unwrap().port());
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((c, _)) = l.accept().await {
            held.push(c);
        }
    });
    url
}

fn mainnet(c: PubkyHttpClient) -> pm_pubky::homeserver::NodeHomeserver {
    pm_pubky::homeserver::NodeHomeserver::from_pubky_on(
        pubky::Pubky::with_client(c),
        pm_pubky::Network::Mainnet,
    )
}

/// A signup's homeserver is looked up and is the one that decides: the public homeserver (here
/// one the network knows nothing of) doesn't refuse a signup on another, healthy homeserver.
#[tokio::test]
async fn a_signups_target_homeserver_is_freshened_and_decides() {
    let hs = Keypair::random();
    let (url, r) = relay(vec![record(&hs, "homeserver.example.org", 1_762_200_000_000_000)]).await;
    let (c, _) = client(&url, &[]);
    let target = pubky::PublicKey::from(hs.public_key());
    let out = mainnet(c).fresh_for_sign_in(None, Some(&target), false).await;
    let out = out.expect("the target's record is healthy");
    let mine = out
        .iter()
        .find(|f| f.key == hs.public_key().to_string())
        .unwrap_or_else(|| panic!("the target was looked up: {out:?}"));
    assert_eq!((mine.from, mine.loopback), (From::Network, false), "{mine:?}");
    assert!(r.served.load(std::sync::atomic::Ordering::SeqCst) >= 1);
}

/// A known identity on another homeserver: its homeserver's record (localhost) refuses; the
/// public homeserver's state is not what decides.
#[tokio::test]
async fn the_identitys_own_homeserver_decides_its_sign_in() {
    let (user, hs) = (Keypair::random(), Keypair::random());
    let healthy = record(&hs, "homeserver.example.org", 1_762_200_000_000_000);
    let local = {
        let mut name = SVCB::new(10, "localhost".try_into().unwrap());
        name.set_port(6287);
        SignedPacket::builder()
            .https(".".try_into().unwrap(), name, 3600)
            .timestamp(Timestamp::from(1_762_300_000_000_000))
            .sign(&hs)
            .unwrap()
    };
    let me = user_record(&user, &hs.public_key(), 1_762_200_000_000_000);
    let identity = pubky::PublicKey::from(user.public_key());
    let (url, _) = relay(vec![me.clone(), healthy]).await;
    let (c, _) = client(&url, &[]);
    let out = mainnet(c).fresh_for_sign_in(Some(&identity), None, false).await;
    let out = out.expect("its homeserver is healthy; the public one doesn't decide");
    assert!(
        out.iter()
            .any(|f| f.key == hs.public_key().to_string() && f.from == From::Network),
        "{out:?}"
    );
    let (url, _) = relay(vec![me, local]).await;
    let (c, _) = client(&url, &[]);
    let r = mainnet(c).fresh_for_sign_in(Some(&identity), None, false).await;
    assert!(r.unwrap_err().contains("names only this device"));
}

/// The network stalls on every lookup; the identity's record and its homeserver's are held
/// here. Both stages fall back to the held copies within the whole step's time, and the sign-in
/// goes on (the overall bound never cancels the second stage's cache read).
#[tokio::test]
async fn a_stalled_network_still_leaves_both_stages_their_cached_records() {
    let (user, hs) = (Keypair::random(), Keypair::random());
    let me = user_record(&user, &hs.public_key(), 1_762_200_000_000_000);
    let theirs = record(&hs, "homeserver.example.org", 1_762_200_000_000_000);
    let url = stalled_relay().await;
    let (c, _) = client(&url, &[me, theirs.clone()]);
    let identity = pubky::PublicKey::from(user.public_key());
    let started = std::time::Instant::now();
    let out = mainnet(c).fresh_for_sign_in(Some(&identity), None, false).await;
    let elapsed = started.elapsed();
    let out = out.expect("the cached records carry the sign-in");
    let named = out
        .iter()
        .find(|f| f.key == hs.public_key().to_string())
        .unwrap_or_else(|| panic!("its homeserver was looked up: {out:?}"));
    assert_eq!(
        (named.from, named.timestamp_us),
        (From::Cache, Some(theirs.timestamp().as_u64())),
        "{named:?}"
    );
    assert!(
        elapsed <= pm_pubky::records::SIGN_IN_LOOKUPS + std::time::Duration::from_millis(500),
        "{elapsed:?}"
    );
}
