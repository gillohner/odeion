//! The node's own loop for its API listeners (the API's address and remote access's loopback
//! one). `axum::serve` gives hyper no timer, so hyper's limit on reading a request's headers
//! never applied: a connection that sent half a request, or none, stayed open for good. Nor did
//! it limit how many were open, and each holds a file descriptor (1,024 per process is a common
//! limit; the library, the art and the audio need theirs too). Here every connection has
//! [`HEADER_TIMEOUT`] for each request's headers (also between requests on a kept-alive
//! connection) and a listener holds at most [`MAX_CONNECTIONS`]; more wait in the system's
//! queue until one closes. One address holds at most [`MAX_PER_PEER`] of them, so one host cannot
//! take them all; loopback is not counted (a proxy or tunnel on this machine, remote access's
//! own listener, which its tunnel limits itself). HTTP/1 only: HTTP/2 without TLS (which no app or page of ours speaks;
//! a browser uses HTTP/2 only over TLS, through a proxy that talks HTTP/1 to the node) would wait
//! for a silent connection's first bytes with no limit, and has no header limit of its own.
//! A connection that becomes a WebSocket (the event stream) leaves the count when it does.

use axum::extract::ConnectInfo;
use axum::Router;
use hyper_util::rt::{TokioIo, TokioTimer};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{watch, Semaphore};
use tower::ServiceExt;

/// How long a connection has to send a request's headers, also the next request's on a kept-alive
/// connection: an idle one is closed after this (browsers keep theirs longer and open a new one;
/// a request sent just as it closes fails like any request on a closed connection). An open event
/// stream or audio being sent is a request already read, so it is not cut.
pub const HEADER_TIMEOUT: Duration = Duration::from_secs(30);

/// Connections one listener holds at once. A household's apps, players and pages use a few
/// dozen (an event stream per open app, audio per player, a browser's six per page).
pub const MAX_CONNECTIONS: usize = 256;

/// Connections one address holds at once on a listener; loopback is not counted.
/// A reverse proxy on another machine is one address for everyone behind it.
pub const MAX_PER_PEER: usize = 64;

/// The limits of one listener; tests use smaller ones.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pub header_timeout: Duration,
    pub max_connections: usize,
    pub per_peer: usize,
    /// Loopback peers are not counted per address.
    pub loopback_free: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            header_timeout: HEADER_TIMEOUT,
            max_connections: MAX_CONNECTIONS,
            per_peer: MAX_PER_PEER,
            loopback_free: true,
        }
    }
}

/// Open connections per address.
#[derive(Default)]
struct Peers(Mutex<HashMap<IpAddr, usize>>);

/// One counted connection; the count goes down when it is dropped.
struct PeerSlot {
    peers: Arc<Peers>,
    key: IpAddr,
}

impl Drop for PeerSlot {
    fn drop(&mut self) {
        let mut open = self.peers.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = open.get_mut(&self.key) {
            *n -= 1;
            if *n == 0 {
                open.remove(&self.key);
            }
        }
    }
}

/// Whom a connection counts against: its address (an IPv4 one written as IPv6 as itself; not
/// an IPv6 prefix: a home network's devices share one /64). `None`: loopback, when it is free.
fn peer_key(ip: IpAddr, loopback_free: bool) -> Option<IpAddr> {
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
        v4 => v4,
    };
    if loopback_free && ip.is_loopback() {
        return None;
    }
    Some(ip)
}

impl Peers {
    /// Count one more connection for `key`, unless it holds `limit` already.
    fn take(self: &Arc<Self>, key: IpAddr, limit: usize) -> Option<PeerSlot> {
        let mut open = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let n = open.entry(key).or_insert(0);
        if *n >= limit {
            return None;
        }
        *n += 1;
        Some(PeerSlot {
            peers: self.clone(),
            key,
        })
    }
}

/// Serve `app` on `listener` until `stop` resolves; then take no new connection, ask the open
/// ones to close once their request is answered, and return when they have. Each request carries
/// its peer's address (`ConnectInfo<SocketAddr>`), as `into_make_service_with_connect_info` gives.
pub(crate) async fn serve(
    mut listener: tokio::net::TcpListener,
    app: Router,
    limits: Limits,
    stop: impl std::future::Future<Output = ()>,
) {
    let slots = Arc::new(Semaphore::new(limits.max_connections));
    let peers = Arc::new(Peers::default());
    let mut refused_said: Option<tokio::time::Instant> = None;
    let (stopping, stop_seen) = watch::channel(false);
    let mut builder = hyper::server::conn::http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(limits.header_timeout);
    let mut stop = std::pin::pin!(stop);
    let mut full_said = false;
    loop {
        // A free slot first: at the limit, new connections wait in the system's queue
        let slot = tokio::select! {
            s = slots.clone().acquire_owned() => match s {
                Ok(s) => s,
                Err(_) => break,
            },
            () = &mut stop => break,
        };
        if slots.available_permits() == 0 && !full_said {
            full_said = true;
            tracing::warn!(
                max = limits.max_connections,
                "the API listener holds as many connections as it takes; new ones wait until one closes"
            );
        } else if slots.available_permits() > limits.max_connections / 2 {
            full_said = false;
        }
        let (tcp, addr) = tokio::select! {
            a = axum::serve::Listener::accept(&mut listener) => a,
            () = &mut stop => break,
        };
        // One address over its share is closed at once (its slot is free again)
        let peer_slot = match peer_key(addr.ip(), limits.loopback_free) {
            None => None,
            Some(key) => match peers.take(key, limits.per_peer) {
                Some(p) => Some(p),
                None => {
                    if refused_said.is_none_or(|t| t.elapsed() > Duration::from_secs(60)) {
                        refused_said = Some(tokio::time::Instant::now());
                        tracing::warn!(peer = %key, max = limits.per_peer, "one address holds as many API connections as it may; more from it are closed");
                    }
                    drop(tcp);
                    continue;
                }
            },
        };
        let app = app
            .clone()
            .map_request(move |mut req: axum::http::Request<hyper::body::Incoming>| {
                req.extensions_mut().insert(ConnectInfo(addr));
                req.map(axum::body::Body::new)
            });
        let service = hyper_util::service::TowerToHyperService::new(app);
        let builder = builder.clone();
        let mut stop_seen = stop_seen.clone();
        tokio::spawn(async move {
            let _slots = (slot, peer_slot);
            let conn = builder
                .serve_connection(TokioIo::new(tcp), service)
                .with_upgrades();
            let mut conn = std::pin::pin!(conn);
            tokio::select! {
                r = conn.as_mut() => {
                    if let Err(e) = r {
                        tracing::trace!(%addr, "connection ended: {e}");
                    }
                    return;
                }
                _ = stop_seen.wait_for(|s| *s) => {}
            }
            conn.as_mut().graceful_shutdown();
            let _ = conn.await;
        });
    }
    drop(listener);
    let _ = stopping.send(true);
    // Every slot back: every connection has closed
    let _ = slots.acquire_many(limits.max_connections as u32).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn started(
        limits: Limits,
    ) -> (
        SocketAddr,
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/peer",
            get(|ConnectInfo(peer): ConnectInfo<SocketAddr>| async move { peer.ip().to_string() }),
        );
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(serve(listener, app, limits, async move {
            let _ = rx.await;
        }));
        (addr, tx, task)
    }

    /// Read until the peer closes or `limit` passes; what came, and whether it closed.
    async fn read_all(s: &mut tokio::net::TcpStream, limit: Duration) -> (String, bool) {
        let mut out = Vec::new();
        let mut buf = [0u8; 1024];
        let end = tokio::time::Instant::now() + limit;
        loop {
            match tokio::time::timeout_at(end, s.read(&mut buf)).await {
                Ok(Ok(0)) | Ok(Err(_)) => return (String::from_utf8_lossy(&out).into(), true),
                Ok(Ok(n)) => out.extend_from_slice(&buf[..n]),
                Err(_) => return (String::from_utf8_lossy(&out).into(), false),
            }
        }
    }

    /// A request is answered with its peer's address, as before.
    #[tokio::test]
    async fn a_request_is_answered_with_its_peers_address() {
        let (addr, _stop, _task) = started(Limits::default()).await;
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        s.write_all(b"GET /peer HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let (got, closed) = read_all(&mut s, Duration::from_secs(5)).await;
        assert!(got.starts_with("HTTP/1.1 200"), "{got}");
        assert!(got.ends_with("127.0.0.1"), "{got}");
        assert!(closed);
    }

    /// A connection that sends half a request's headers is closed after the header limit; one
    /// that sends nothing too.
    #[tokio::test]
    async fn half_a_request_is_closed_after_the_header_limit() {
        let limits = Limits {
            header_timeout: Duration::from_millis(300),
            ..Limits::default()
        };
        let (addr, _stop, _task) = started(limits).await;
        let mut half = tokio::net::TcpStream::connect(addr).await.unwrap();
        half.write_all(b"GET /peer HTTP/1.1\r\nHost: x\r\n")
            .await
            .unwrap();
        let mut silent = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (_, closed) = read_all(&mut half, Duration::from_secs(5)).await;
        assert!(closed, "half a request was not closed");
        let (_, closed) = read_all(&mut silent, Duration::from_secs(5)).await;
        assert!(closed, "a silent connection was not closed");
    }

    /// At the limit a new connection waits, and is answered once another closes.
    #[tokio::test]
    async fn past_the_limit_a_connection_waits_for_a_free_one() {
        let limits = Limits {
            max_connections: 2,
            ..Limits::default()
        };
        let (addr, _stop, _task) = started(limits).await;
        let mut held = Vec::new();
        for _ in 0..2 {
            let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
            // One full request each, kept alive: the connection is open and in use
            s.write_all(b"GET /peer HTTP/1.1\r\nHost: x\r\n\r\n")
                .await
                .unwrap();
            let mut buf = [0u8; 256];
            let n = s.read(&mut buf).await.unwrap();
            assert!(String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 200"));
            held.push(s);
        }
        let mut third = tokio::net::TcpStream::connect(addr).await.unwrap();
        third
            .write_all(b"GET /peer HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let (got, _) = read_all(&mut third, Duration::from_millis(500)).await;
        assert_eq!(got, "", "a third connection was served past the limit of two");
        drop(held.pop());
        let (got, closed) = read_all(&mut third, Duration::from_secs(5)).await;
        assert!(got.starts_with("HTTP/1.1 200"), "{got}");
        assert!(closed);
    }

    /// Told to stop: no new connection, a kept-alive idle one is closed, and the call returns.
    #[tokio::test]
    async fn told_to_stop_it_closes_idle_connections_and_returns() {
        let (addr, stop, task) = started(Limits::default()).await;
        let mut idle = tokio::net::TcpStream::connect(addr).await.unwrap();
        idle.write_all(b"GET /peer HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let mut buf = [0u8; 256];
        let _ = idle.read(&mut buf).await.unwrap();
        stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("serve did not return")
            .unwrap();
        let (_, closed) = read_all(&mut idle, Duration::from_secs(5)).await;
        assert!(closed);
        assert!(
            tokio::net::TcpStream::connect(addr).await.is_err(),
            "still taking connections"
        );
    }

    /// One address takes at most its share; the others still get in, and a closed connection
    /// gives its place back.
    #[tokio::test]
    async fn one_address_takes_at_most_its_share() {
        let limits = Limits {
            per_peer: 2,
            loopback_free: false,
            ..Limits::default()
        };
        let (addr, _stop, _task) = started(limits).await;
        let mut held = Vec::new();
        for _ in 0..2 {
            let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
            s.write_all(b"GET /peer HTTP/1.1\r\nHost: x\r\n\r\n")
                .await
                .unwrap();
            let mut buf = [0u8; 256];
            let n = s.read(&mut buf).await.unwrap();
            assert!(String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 200"));
            held.push(s);
        }
        let mut third = tokio::net::TcpStream::connect(addr).await.unwrap();
        let _ = third
            .write_all(b"GET /peer HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .await;
        let (got, closed) = read_all(&mut third, Duration::from_secs(5)).await;
        assert!(
            closed && got.is_empty(),
            "a third from one address was served: {got}"
        );
        drop(held.pop());
        // Its place is free again once the server has seen the close
        let mut served = false;
        for _ in 0..50 {
            let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
            let _ = s
                .write_all(b"GET /peer HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
                .await;
            let (got, _) = read_all(&mut s, Duration::from_secs(2)).await;
            if got.starts_with("HTTP/1.1 200") {
                served = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(served, "the closed connection's place did not come back");
    }

    #[test]
    fn each_address_counts_alone_and_loopback_is_free() {
        let key = |s: &str| peer_key(s.parse().unwrap(), true).map(|k| k.to_string());
        assert_eq!(key("127.0.0.1"), None);
        assert_eq!(key("::1"), None);
        assert_eq!(key("::ffff:127.0.0.1"), None);
        assert_eq!(key("192.168.1.20").as_deref(), Some("192.168.1.20"));
        assert_eq!(key("::ffff:192.168.1.20").as_deref(), Some("192.168.1.20"));
        // A home network's devices share one /64: each address counts alone
        assert_ne!(key("2001:db8:1:2::10"), key("2001:db8:1:2::11"));
        assert_eq!(
            peer_key("127.0.0.1".parse().unwrap(), false)
                .map(|k| k.to_string())
                .as_deref(),
            Some("127.0.0.1")
        );
    }
}
