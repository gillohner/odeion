//! Reads of other people's public data on the public network.
//!
//! The SDK's HTTP clients follow up to ten redirects (https to http too) to any address, and the
//! SDK sends requests of its own while it prepares one (the homeserver's `/info`, a TCP probe of
//! its addresses). A key whose homeserver record names a server someone else runs could send the
//! node's GET to `127.0.0.1`, the cloud's metadata address or a LAN host. So these reads are
//! prepared and sent by the node itself, as the SDK would send them, with its own clients: no
//! redirect is followed, and no loopback, private, link-local or shared (CGNAT) address is
//! connected to, by name or by address. The filter sits in the resolver the connection uses, so
//! a name that changes its answer between two lookups can't get past it.
//!
//! The node's bundled homeserver is local by design: reads from it keep local addresses
//! ([`crate::homeserver::NodeHomeserver::allow_local_homeserver`]). Redirects are never followed.

use crate::homeserver::HomeserverError;
use futures_util::StreamExt;
use pubky::PublicKey;
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// How long what a homeserver's `/info` said is kept (as the SDK keeps it).
const INFO_FOR: Duration = Duration::from_secs(60);
/// The most homeservers whose `/info` is kept.
const INFO_MAX: usize = 1024;
/// The most of an `/info` answer read (as the SDK reads).
const INFO_MAX_BYTES: usize = 16 * 1024;
/// How long a connection by ICANN name may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a connection by key may take: when it fails the ICANN name is tried, as the SDK
/// probes the key's addresses briefly before it falls back.
const KEY_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// How long `/info` may take (the SDK's own limit).
const INFO_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the way a homeserver was reached is tried first again.
const REACHED_FOR: Duration = Duration::from_secs(60);
/// The `/info` feature of a homeserver that takes `/storage/<owner>/<path>` (pubky-common's
/// `constants::features::PATH_ADDRESSED_STORAGE`).
const PATH_ADDRESSED_STORAGE: &str = "path-addressed-storage";

/// Whether the node may connect to `ip` for someone else's homeserver: a public unicast address.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => public_v4(v4),
            None => public_v6(v6),
        },
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || o[0] == 0
        // Shared address space (CGNAT, some VPNs), 100.64.0.0/10
        || (o[0] == 100 && (o[1] & 0xc0) == 64)
        // IETF protocol assignments, 192.0.0.0/24; benchmarking, 198.18.0.0/15; reserved, 240/4
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)
        || (o[0] == 198 && (o[1] & 0xfe) == 18)
        || o[0] >= 240)
}

fn public_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    let embedded = |hi: u16, lo: u16| Ipv4Addr::from((u32::from(hi) << 16) | u32::from(lo));
    // NAT64 (64:ff9b::/96) and 6to4 (2002::/16) carry an IPv4 address: that one decides
    if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] {
        return public_v4(embedded(s[6], s[7]));
    }
    if s[0] == 0x2002 {
        return public_v4(embedded(s[1], s[2]));
    }
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        // IPv4-compatible (deprecated), ::/96; IPv4-translated, ::ffff:0:0:0/96
        || s[..6] == [0, 0, 0, 0, 0, 0]
        || s[..6] == [0, 0, 0, 0, 0xffff, 0]
        // Discard, 100::/64; local-use NAT64, 64:ff9b:1::/48
        || s[..4] == [0x100, 0, 0, 0]
        || (s[0] == 0x64 && s[1] == 0xff9b && s[2] == 1)
        // Unique local, fc00::/7; link-local, fe80::/10; site-local (deprecated), fec0::/10
        || (s[0] & 0xfe00) == 0xfc00
        || (s[0] & 0xffc0) == 0xfe80
        || (s[0] & 0xffc0) == 0xfec0
        // Teredo, 2001::/32 (it carries an address of its own); documentation, 2001:db8::/32
        || (s[0] == 0x2001 && s[1] == 0)
        || (s[0] == 0x2001 && s[1] == 0x0db8))
}

/// DNS for these reads: the addresses `inner` finds, public ones only when `public_only`.
struct Filtered {
    inner: Arc<dyn reqwest::dns::Resolve>,
    public_only: bool,
}

impl reqwest::dns::Resolve for Filtered {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        let found = self.inner.resolve(name);
        let public_only = self.public_only;
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = found
                .await?
                .filter(|a| !public_only || is_public(a.ip()))
                .collect();
            if addrs.is_empty() {
                return Err(format!("{host} names no public address").into());
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// The system's DNS, as reqwest's own default.
struct System;

impl reqwest::dns::Resolve for System {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// One pair of clients: for a homeserver reached by an ICANN name (X.509 TLS), and by its key
/// (pkarr's TLS and resolver). Neither follows a redirect or uses a proxy (a proxy would resolve
/// the name itself, past the filter).
#[derive(Clone)]
struct Clients {
    icann: reqwest::Client,
    by_key: reqwest::Client,
    public_only: bool,
}

impl Clients {
    fn new(pkarr: &pubky::pkarr::Client, public_only: bool) -> Self {
        let finish = |b: reqwest::ClientBuilder, inner: Arc<dyn reqwest::dns::Resolve>, connect: Duration| {
            b.redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .connect_timeout(connect)
                .user_agent(concat!("odeion/", env!("CARGO_PKG_VERSION")))
                .dns_resolver(Arc::new(Filtered { inner, public_only }))
                .build()
                .expect("an HTTP client builds, as the SDK's own did")
        };
        Self {
            icann: finish(reqwest::Client::builder(), Arc::new(System), CONNECT_TIMEOUT),
            // pkarr's TLS for key hosts, and its resolver (this replaces the one `from` set)
            by_key: finish(
                reqwest::ClientBuilder::from(pkarr.clone()),
                Arc::new(pkarr.clone()),
                KEY_CONNECT_TIMEOUT,
            ),
            public_only,
        }
    }
}

/// Where a homeserver is reached.
#[derive(Debug, Clone, PartialEq)]
enum Transport {
    /// By its key: `https://<homeserver key>` over pkarr's TLS.
    Key,
    /// By the ICANN name its record names.
    Icann { domain: String, port: Option<u16> },
}

/// The node's clients for reads of other people's public data (module docs). Clones share the
/// clients, the local homeservers and what `/info` said.
#[derive(Clone)]
pub(crate) struct PublicReads {
    public: Clients,
    local: Clients,
    /// Homeservers (z32) whose reads may reach local addresses: the node's bundled one.
    local_homeservers: Arc<RwLock<HashSet<String>>>,
    /// Whether a homeserver takes path-addressed storage, and since when that is known.
    path_addressed: Arc<Mutex<HashMap<String, (bool, Instant)>>>,
    /// The way each homeserver was last reached, and when.
    reached: Arc<Mutex<HashMap<String, (Transport, Instant)>>>,
}

impl PublicReads {
    pub(crate) fn new(pkarr: &pubky::pkarr::Client) -> Self {
        Self {
            public: Clients::new(pkarr, true),
            local: Clients::new(pkarr, false),
            local_homeservers: Arc::default(),
            path_addressed: Arc::default(),
            reached: Arc::default(),
        }
    }

    /// Reads from `homeserver` (z32) may reach local addresses (the node's bundled homeserver,
    /// whose record names one by design).
    pub(crate) fn allow_local_homeserver(&self, homeserver: &str) {
        self.local_homeservers
            .write()
            .expect("local homeservers")
            .insert(homeserver.to_string());
    }

    /// GET `path` (absolute, `/pub/…`) of `owner`'s storage, with `query`, as the SDK would send
    /// it: the owner's homeserver looked up, its endpoints tried in the SDK's order (by key, then
    /// by ICANN name), path-addressed storage where its `/info` says so, else the legacy path
    /// with `pubky-host`.
    pub(crate) async fn get(
        &self,
        pubky: &pubky::Pubky,
        owner: &PublicKey,
        path: &str,
        query: Option<&str>,
    ) -> Result<reqwest::Response, HomeserverError> {
        let homeserver = pubky
            .get_homeserver_of(owner)
            .await?
            .ok_or_else(|| HomeserverError::Transport("that key names no homeserver".into()))?;
        let hs = homeserver.z32();
        let mut transports = transports(pubky.client().pkarr(), &hs).await;
        if transports.is_empty() {
            return Err(HomeserverError::Transport(
                "that key's homeserver names no address".into(),
            ));
        }
        let local = self
            .local_homeservers
            .read()
            .expect("local homeservers")
            .contains(&hs);
        let clients = if local { &self.local } else { &self.public };
        // The way that worked lately first
        if let Some((t, at)) = self.reached.lock().expect("reached").get(&hs) {
            if at.elapsed() < REACHED_FOR {
                if let Some(i) = transports.iter().position(|x| x == t) {
                    transports[..=i].rotate_right(1);
                }
            }
        }
        let path_addressed = self.path_addressed(clients, &transports, &hs).await;
        let resource = pubky::PubkyResource::new(owner.clone(), path)?;
        let rel = resource.path.as_str().trim_start_matches('/');
        let (target, pubky_host) = if path_addressed {
            (format!("/storage/{}/{rel}", owner.z32()), None)
        } else {
            (format!("/{rel}"), Some(owner.z32()))
        };
        let target = match query {
            Some(q) if !q.is_empty() => format!("{target}?{q}"),
            _ => target,
        };
        let (resp, how) = send(clients, &transports, &hs, &target, pubky_host.as_deref(), None).await?;
        self.reached_by(&hs, how);
        Ok(resp)
    }

    fn reached_by(&self, hs: &str, how: Transport) {
        let mut reached = self.reached.lock().expect("reached");
        reached.retain(|_, (_, at)| at.elapsed() < REACHED_FOR);
        if reached.len() >= INFO_MAX {
            reached.clear();
        }
        reached.insert(hs.to_string(), (how, Instant::now()));
    }

    /// Whether `hs` takes path-addressed storage, from its `/info` (kept [`INFO_FOR`], also
    /// when it doesn't answer within [`INFO_TIMEOUT`]); `false` when it can't say, as the SDK
    /// decides.
    async fn path_addressed(&self, clients: &Clients, transports: &[Transport], hs: &str) -> bool {
        if let Some((yes, at)) = self.path_addressed.lock().expect("path addressing").get(hs) {
            if at.elapsed() < INFO_FOR {
                return *yes;
            }
        }
        #[derive(serde::Deserialize)]
        struct Info {
            features: Vec<String>,
        }
        let yes = match send(clients, transports, hs, "/info", None, Some(INFO_TIMEOUT)).await {
            Ok((resp, how)) if resp.status().is_success() => {
                self.reached_by(hs, how);
                crate::homeserver::read_capped(resp, INFO_MAX_BYTES, INFO_TIMEOUT)
                    .await
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Info>(&b).ok())
                    .is_some_and(|i| i.features.iter().any(|f| f == PATH_ADDRESSED_STORAGE))
            }
            _ => false,
        };
        let mut known = self.path_addressed.lock().expect("path addressing");
        known.retain(|_, (_, at)| at.elapsed() < INFO_FOR);
        if known.len() >= INFO_MAX {
            known.clear();
        }
        known.insert(hs.to_string(), (yes, Instant::now()));
        yes
    }
}

/// A homeserver's endpoints in the order the SDK tries them: by key when its record names
/// addresses of its own, then by the first ICANN name it names.
async fn transports(pkarr: &pubky::pkarr::Client, hs: &str) -> Vec<Transport> {
    let endpoints: Vec<_> = pkarr.resolve_https_endpoints(hs).collect().await;
    let mut out = Vec::new();
    if endpoints.iter().any(|e| e.domain().is_none()) {
        out.push(Transport::Key);
    }
    if let Some((domain, port)) = endpoints.iter().find_map(|e| e.domain().map(|d| (d, e.port()))) {
        out.push(Transport::Icann {
            domain: domain.trim_end_matches('.').to_string(),
            port,
        });
    }
    out
}

/// GET `target` (path and query) of homeserver `hs` by each transport in turn, the next one
/// tried when one can't be reached; the answer and the way it came. A redirect is an error,
/// never followed. `timeout`: for the whole request.
async fn send(
    clients: &Clients,
    transports: &[Transport],
    hs: &str,
    target: &str,
    pubky_host: Option<&str>,
    timeout: Option<Duration>,
) -> Result<(reqwest::Response, Transport), HomeserverError> {
    let mut last = None;
    for t in transports {
        let (client, url) = match t {
            Transport::Key => (&clients.by_key, format!("https://{hs}{target}")),
            Transport::Icann { domain, port } => {
                let host = match domain.parse::<IpAddr>() {
                    Ok(IpAddr::V6(v6)) => format!("[{v6}]"),
                    _ => domain.clone(),
                };
                let port = port.map(|p| format!(":{p}")).unwrap_or_default();
                (&clients.icann, format!("https://{host}{port}{target}"))
            }
        };
        let url = reqwest::Url::parse(&url)
            .map_err(|e| HomeserverError::Transport(format!("a homeserver URL that doesn't parse: {e}")))?;
        // An address in the URL never goes through the resolver: checked here
        let literal = match url.host() {
            Some(url::Host::Ipv4(ip)) => Some(IpAddr::V4(ip)),
            Some(url::Host::Ipv6(ip)) => Some(IpAddr::V6(ip)),
            _ => None,
        };
        if clients.public_only && literal.is_some_and(|ip| !is_public(ip)) {
            last = Some(refused(url.host_str().unwrap_or("?")));
            continue;
        }
        let mut request = client.get(url);
        if let Some(t) = timeout {
            request = request.timeout(t);
        }
        if let Some(owner) = pubky_host {
            request = request.header("pubky-host", owner);
        }
        match request.send().await {
            Ok(resp) if resp.status().is_redirection() => {
                return Err(HomeserverError::Transport(format!(
                    "the homeserver answered with a redirect ({}); it is not followed",
                    resp.status()
                )));
            }
            Ok(resp) => return Ok((resp, t.clone())),
            // Not reached (refused, no public address, no answer): the next way, as the SDK
            // falls back from the key to the ICANN name
            Err(e) if e.is_connect() || e.is_timeout() => last = Some(pubky::Error::from(e).into()),
            Err(e) => return Err(pubky::Error::from(e).into()),
        }
    }
    Err(last.unwrap_or_else(|| HomeserverError::Transport("no way to reach that homeserver".into())))
}

fn refused(named: &str) -> HomeserverError {
    HomeserverError::Transport(format!(
        "that homeserver names a local address ({named}); the node reads other people's data from public addresses only"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn only_public_unicast_addresses_are_public() {
        for ip in [
            "1.1.1.1",
            "139.162.128.88",
            "2606:4700::1111",
            "64:ff9b::808:808",
            "::ffff:8.8.8.8",
            "2002:808:808::1",
        ] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.20",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.254",
            "0.0.0.0",
            "0.1.2.3",
            "255.255.255.255",
            "224.0.0.1",
            "192.0.0.170",
            "198.18.0.1",
            "240.0.0.1",
            "::1",
            "::",
            "fd00::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::ffff:0:a9fe:a9fe",
            "::127.0.0.1",
            "64:ff9b::7f00:1",
            "64:ff9b:1::1",
            "2002:7f00:1::",
            "2001::1",
            "100::1",
            "2001:db8::1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    /// A server that answers every request with `answer` and counts them.
    async fn server(answer: String) -> (u16, Arc<AtomicUsize>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let counted = hits.clone();
        tokio::spawn(async move {
            while let Ok((mut c, _)) = l.accept().await {
                counted.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 4096];
                let _ = c.read(&mut buf).await;
                let _ = c.write_all(answer.as_bytes()).await;
            }
        });
        (port, hits)
    }

    fn pkarr() -> pubky::pkarr::Client {
        let mut b = pubky::pkarr::Client::builder();
        b.no_default_network().relays(&["http://127.0.0.1:1/"]).unwrap();
        b.build().unwrap()
    }

    /// A redirect is never followed, also where local addresses are allowed (the bundled
    /// homeserver); otherwise a local address is never connected to, by address or by name.
    #[tokio::test]
    async fn redirects_are_not_followed_and_local_addresses_not_reached() {
        let (second, second_hits) =
            server("HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nstolen".into()).await;
        let (first, first_hits) = server(format!(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{second}/stolen\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        ))
        .await;
        let reads = PublicReads::new(&pkarr());
        // Plain HTTP stands in for the homeserver's TLS: the clients are what is checked
        let by = |clients: &Clients, host: &str| clients.icann.get(format!("http://{host}:{first}/pub/x"));

        // A name goes through the resolver, which keeps to public addresses; an address in the
        // URL never does, so `send` checks it
        let e = by(&reads.public, "localhost").send().await.unwrap_err();
        assert!(e.is_connect(), "{e}");
        assert_eq!(
            first_hits.load(Ordering::SeqCst),
            0,
            "a local address, never reached"
        );
        let e = send(
            &reads.public,
            &[Transport::Icann {
                domain: "127.0.0.1".into(),
                port: Some(first),
            }],
            "hs",
            "/pub/x",
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(e.to_string().contains("local address"), "{e}");
        assert_eq!(first_hits.load(Ordering::SeqCst), 0);

        let resp = by(&reads.local, "127.0.0.1").send().await.unwrap();
        assert_eq!(
            resp.status(),
            reqwest::StatusCode::FOUND,
            "the redirect is the answer"
        );
        assert_eq!(first_hits.load(Ordering::SeqCst), 1);
        assert_eq!(
            second_hits.load(Ordering::SeqCst),
            0,
            "the redirect was not followed"
        );
    }
}
