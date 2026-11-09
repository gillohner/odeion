//! The newest PKARR record before a sign-in routes on it.
//!
//! pkarr's `CacheFirst` lookup (what pubky's grant exchange and transport routing use) keeps the
//! first fresh answer a relay or DHT node gives when the cache is empty, and nodes still serve
//! older signed records of a key next to the newest one. A sign-in that routed on such a record
//! went where that record said (an old record of a public homeserver named `localhost`), and a
//! failed grant exchange consumes Ring's approval: there is no second try.
//!
//! A cached packet is a freshness floor for `CacheFirst`: older network answers are stale, and
//! a fresh one is returned without asking the network. So the records a sign-in will route on
//! are asked for network-first here ([`freshen`]: `NetworkOnly`, every relevant node, the most
//! recent value; pkarr stores it unless the cache holds a newer one), before any request routes
//! on them, and the cache is only the fallback when the network fails.

use std::net::IpAddr;
use std::time::Duration;

use pubky::pkarr::dns::rdata::RData;
use pubky::pkarr::{self, PublicKey, ResolvePolicy, SignedPacket};
use serde::{Deserialize, Serialize};

/// The project's public homeserver (a server key, not a person).
pub const PUBLIC_HOMESERVER: &str = "p815irkx17jo489ba9ym56kb5zmcuf385w88jz8ncq5b38t7k1ay";

/// How long one network-first lookup may take: a sign-in waits for it, so it is short.
pub const FRESHEN_TIMEOUT: Duration = Duration::from_secs(5);
/// How long reading the copy held here may take (pkarr's cache-only lookup asks the relays'
/// caches when the client has none).
pub const CACHE_TIMEOUT: Duration = Duration::from_secs(1);
/// The most all of a sign-in's lookups take together (the identity's record, then its
/// homeserver's): the node's sign-in start must answer the app well within its own wait.
pub const SIGN_IN_LOOKUPS: Duration = Duration::from_secs(9);
/// The identity's own record is looked up within this much of [`SIGN_IN_LOOKUPS`]; the rest is
/// its homeserver's. Each stage keeps [`CACHE_TIMEOUT`] of its share for the copy held here, so a
/// network that stalls never takes away the cached fallback.
pub const IDENTITY_STAGE: Duration = Duration::from_secs(4);

/// Where a record came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum From {
    /// The newest the network has (relays and DHT asked, the most recent value taken).
    Network,
    /// The network failed; the copy this client held.
    Cache,
    /// Neither: nothing is known of the key.
    None,
}

/// One key's record after [`freshen`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Freshened {
    /// The key, z32.
    pub key: String,
    pub from: From,
    /// The record's time (microseconds), when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp_us: Option<u64>,
    /// Every endpoint the record names is a loopback address ([`routes_to_loopback`]).
    pub loopback: bool,
    /// Why the network didn't answer, when it didn't.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The newest record of each of `keys`, network-first: pkarr's `NetworkOnly` within
/// [`FRESHEN_TIMEOUT`] (the result is stored in the client's cache unless a newer one is there
/// already, so what routes next uses it), else the cached copy. Keys that can't be parsed are
/// skipped. Never fails: a sign-in goes on with what is known.
pub async fn freshen(pkarr: &pkarr::Client, keys: &[&str]) -> Vec<Freshened> {
    let keys: Vec<PublicKey> = keys.iter().filter_map(|z| PublicKey::try_from(*z).ok()).collect();
    // All at once: a sign-in waits for the slowest, not for the sum
    futures_util::future::join_all(keys.iter().map(|pk| freshen_one(pkarr, pk))).await
}

/// [`freshen`] for one key.
pub async fn freshen_one(pkarr: &pkarr::Client, pk: &PublicKey) -> Freshened {
    let until = tokio::time::Instant::now() + FRESHEN_TIMEOUT + CACHE_TIMEOUT;
    freshen_until(pkarr, pk, until).await.0
}

/// [`freshen_one`] done by `until`: the network gets what is left of the time but
/// [`CACHE_TIMEOUT`] (at most [`FRESHEN_TIMEOUT`]), the copy held here always gets its read.
/// Also returns the record itself.
pub async fn freshen_until(
    pkarr: &pkarr::Client,
    pk: &PublicKey,
    until: tokio::time::Instant,
) -> (Freshened, Option<SignedPacket>) {
    let key = pk.to_string();
    let left = until
        .saturating_duration_since(tokio::time::Instant::now())
        .saturating_sub(CACHE_TIMEOUT)
        .min(FRESHEN_TIMEOUT);
    let network = tokio::time::timeout(left, pkarr.resolve(pk, ResolvePolicy::NetworkOnly)).await;
    let (from, error) = match network {
        Ok(Ok(_)) => (From::Network, None),
        Ok(Err(e)) => (From::None, Some(e.to_string())),
        Err(_) => (From::None, Some("the network didn't answer in time".to_string())),
    };
    // What the client holds now is what routes next: the network's answer, or a newer copy the
    // cache kept over it (pkarr never replaces a newer one), or, the network failed, the copy held
    let held = tokio::time::timeout(CACHE_TIMEOUT, pkarr.resolve(pk, ResolvePolicy::CacheOnly))
        .await
        .ok()
        .and_then(Result::ok);
    let (packet, from) = match (held, from) {
        (Some(p), From::Network) => (Some(p), From::Network),
        (Some(p), _) => (Some(p), From::Cache),
        (None, _) => (None, From::None),
    };
    let f = Freshened {
        key,
        from,
        timestamp_us: packet.as_ref().map(|p| p.timestamp().as_u64()),
        loopback: packet.as_ref().is_some_and(routes_to_loopback),
        error,
    };
    (f, packet)
}

/// Why a sign-in on the public network must not start with the record of `selected`, the
/// homeserver it will route to (none known: nothing refuses), if it must not: its newest record
/// names only the client's own machine (an old record of a public homeserver did), or nothing is
/// known of it (the exchange would take the first record some node answers with, which is what
/// went wrong). Other homeservers' records, looked up only to have them ready, never refuse.
/// Refused before the sign-in's link exists, so the person tries again without spending a Ring
/// approval.
///
/// `allow_local`: the node is configured for a local homeserver (its bundled one), whose record
/// names a local address by design, and which needs no lookup on the wider network: only a
/// loopback record of the public homeserver refuses then, when the sign-in routes to it.
pub fn refusal(records: &[Freshened], selected: Option<&str>, allow_local: bool) -> Option<String> {
    let selected = records.iter().filter(|r| Some(r.key.as_str()) == selected);
    let held = |r: &&Freshened| !allow_local || r.key == PUBLIC_HOMESERVER;
    if let Some(r) = selected.clone().filter(held).find(|r| r.loopback) {
        return Some(format!(
            "the homeserver's record ({}) names only this device; try again in a moment",
            short(&r.key)
        ));
    }
    if let Some(r) = selected.filter(|_| !allow_local).find(|r| r.from == From::None) {
        return Some(format!(
            "the homeserver's current address couldn't be found ({}{}); check the connection and try again",
            short(&r.key),
            r.error.as_deref().map(|e| format!(": {e}")).unwrap_or_default()
        ));
    }
    None
}

fn short(key: &str) -> &str {
    &key[..key.len().min(8)]
}

/// The homeserver a user's freshened record names (`_pubky`), if any: the key the grant exchange
/// will route to, to be freshened in turn.
pub fn homeserver_in(pkarr_cached: &SignedPacket) -> Option<PublicKey> {
    let host = pkarr_cached
        .resource_records("_pubky")
        .find_map(|rr| match &rr.rdata {
            RData::SVCB(svcb) => Some(svcb.target.to_string()),
            RData::HTTPS(https) => Some(https.0.target.to_string()),
            _ => None,
        })?;
    PublicKey::try_from(host.trim_end_matches('.')).ok()
}

/// Whether every endpoint a record names is a loopback address: its HTTPS and SVCB targets
/// (`localhost`, or `.` with only loopback A/AAAA records) and its addresses. A record naming
/// nothing is not loopback. On the public network such a record of a homeserver's sends a
/// sign-in to the client's own machine; on a local testnet it is what the homeserver is.
pub fn routes_to_loopback(packet: &SignedPacket) -> bool {
    let mut targets = 0usize;
    let mut loopback = 0usize;
    let addresses: Vec<IpAddr> = packet
        .all_resource_records()
        .filter_map(|rr| match &rr.rdata {
            RData::A(a) => Some(IpAddr::V4(a.address.into())),
            RData::AAAA(a) => Some(IpAddr::V6(a.address.into())),
            _ => None,
        })
        .collect();
    for rr in packet.all_resource_records() {
        let target = match &rr.rdata {
            RData::HTTPS(h) => h.0.target.to_string(),
            RData::SVCB(s) => s.target.to_string(),
            _ => continue,
        };
        targets += 1;
        let t = target.trim_end_matches('.');
        let is_loopback = if t.is_empty() {
            // The direct endpoint: the record's own addresses
            !addresses.is_empty() && addresses.iter().all(IpAddr::is_loopback)
        } else {
            t.eq_ignore_ascii_case("localhost") || t.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
        };
        if is_loopback {
            loopback += 1;
        }
    }
    targets > 0 && loopback == targets
}

#[cfg(test)]
mod tests {
    use super::*;
    use pubky::pkarr::dns::rdata::SVCB;
    use pubky::pkarr::Keypair;

    fn record(icann: Option<&str>, direct_ip: Option<&str>) -> SignedPacket {
        let mut b = SignedPacket::builder();
        if let Some(ip) = direct_ip {
            let mut direct = SVCB::new(1, ".".try_into().unwrap());
            direct.set_port(6287);
            b = b.https(".".try_into().unwrap(), direct, 3600).address(
                ".".try_into().unwrap(),
                ip.parse().unwrap(),
                3600,
            );
        }
        if let Some(name) = icann {
            let mut svcb = SVCB::new(10, name.try_into().unwrap());
            svcb.set_port(443);
            b = b.https(".".try_into().unwrap(), svcb, 3600);
        }
        b.sign(&Keypair::random()).unwrap()
    }

    #[test]
    fn a_record_naming_only_the_clients_own_machine_is_loopback() {
        assert!(routes_to_loopback(&record(Some("localhost"), None)));
        assert!(routes_to_loopback(&record(Some("localhost"), Some("127.0.0.1"))));
        assert!(routes_to_loopback(&record(None, Some("::1"))));
        // A public name or address next to it: a route exists
        assert!(!routes_to_loopback(&record(
            Some("homeserver.example.org"),
            Some("127.0.0.1")
        )));
        assert!(!routes_to_loopback(&record(
            Some("localhost"),
            Some("139.162.128.88")
        )));
        assert!(!routes_to_loopback(&record(None, Some("139.162.128.88"))));
        // Nothing named: not loopback (nothing to route to either)
        assert!(!routes_to_loopback(&record(None, None)));
    }

    fn freshened(key: &str, from: From, loopback: bool) -> Freshened {
        Freshened {
            key: key.into(),
            from,
            timestamp_us: (from != From::None).then_some(1),
            loopback,
            error: None,
        }
    }

    #[test]
    fn a_sign_in_is_refused_on_a_loopback_or_unknown_homeserver() {
        let other = "8pinxxgqs41n4aididenw5apqp1urfmzdztr8jt4abrkdn435ewo";
        let public = Some(PUBLIC_HOMESERVER);
        let r = |rs: &[Freshened], sel: Option<&str>, local| refusal(rs, sel, local);
        assert_eq!(
            r(
                &[freshened(PUBLIC_HOMESERVER, From::Network, false)],
                public,
                false
            ),
            None
        );
        assert_eq!(
            r(&[freshened(PUBLIC_HOMESERVER, From::Cache, false)], public, false),
            None
        );
        assert!(r(
            &[freshened(PUBLIC_HOMESERVER, From::Network, true)],
            public,
            false
        )
        .is_some());
        assert!(r(&[freshened(PUBLIC_HOMESERVER, From::None, false)], public, false).is_some());
        assert!(r(&[freshened(other, From::Network, true)], Some(other), false).is_some());
        // A node with a homeserver of its own: that one may be local; the public one never
        assert_eq!(
            r(&[freshened(other, From::Network, true)], Some(other), true),
            None
        );
        assert_eq!(r(&[freshened(other, From::None, false)], Some(other), true), None);
        assert_eq!(
            r(&[freshened(PUBLIC_HOMESERVER, From::None, false)], public, true),
            None
        );
        assert!(r(&[freshened(PUBLIC_HOMESERVER, From::Network, true)], public, true).is_some());
    }

    /// Only the homeserver the sign-in routes to refuses it: the public homeserver's record,
    /// looked up only to have it ready, never blocks a sign-in on another, healthy one (nor a
    /// bundled signup), and with no homeserver known nothing refuses.
    #[test]
    fn another_homeservers_record_never_refuses_the_sign_in() {
        let other = "8pinxxgqs41n4aididenw5apqp1urfmzdztr8jt4abrkdn435ewo";
        let records = [
            freshened(PUBLIC_HOMESERVER, From::Network, true),
            freshened(other, From::Network, false),
        ];
        assert_eq!(refusal(&records, Some(other), false), None);
        assert_eq!(refusal(&records, Some(other), true), None);
        assert!(refusal(&records, Some(PUBLIC_HOMESERVER), false).is_some());
        let unknown_public = [
            freshened(PUBLIC_HOMESERVER, From::None, false),
            freshened(other, From::Cache, false),
        ];
        assert_eq!(refusal(&unknown_public, Some(other), false), None);
        assert_eq!(refusal(&records, None, false), None);
    }

    #[test]
    fn the_homeserver_of_a_user_record_is_its_pubky_target() {
        let user = Keypair::random();
        let hs = Keypair::random().public_key();
        let packet = SignedPacket::builder()
            .https(
                "_pubky".try_into().unwrap(),
                SVCB::new(0, hs.to_string().as_str().try_into().unwrap()),
                3600,
            )
            .sign(&user)
            .unwrap();
        assert_eq!(homeserver_in(&packet), Some(hs));
        assert_eq!(homeserver_in(&record(Some("localhost"), None)), None);
    }
}
