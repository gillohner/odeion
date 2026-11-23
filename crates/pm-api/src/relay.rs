//! Audio relayed to a player as it arrives (a source's stream for the browser). The node's
//! HTTP client gives every request 20 s in all, body included, which cut every song or book that
//! takes longer to send: a relay instead has no total limit shorter than [`RELAY_MAX`], and
//! [`RELAY_IDLE`] for the answer's headers and between two pieces of its body.

use futures_util::StreamExt;
use std::time::Duration;

/// The longest a relayed stream may take in all (a long audiobook, read at the player's pace).
pub(crate) const RELAY_MAX: Duration = Duration::from_secs(6 * 3600);
/// The longest wait for the answer's headers, and between two pieces of its body while the
/// player is reading.
pub(crate) const RELAY_IDLE: Duration = Duration::from_secs(30);

/// The player's `Range` and `If-Range` on the request to the source, so a player can seek past
/// what it has (a source that takes ranges answers 206). Only a range of one part: several
/// would come back as `multipart/byteranges` under an audio type (players ask for one).
pub(crate) fn ranged(rb: reqwest::RequestBuilder, player: &axum::http::HeaderMap) -> reqwest::RequestBuilder {
    let one_part = player
        .get(axum::http::header::RANGE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|r| !r.contains(','));
    if !one_part {
        return rb;
    }
    let mut rb = rb;
    for h in [axum::http::header::RANGE, axum::http::header::IF_RANGE] {
        if let Some(v) = player.get(&h) {
            rb = rb.header(h, v.clone());
        }
    }
    rb
}

/// The source's status and range headers on the player's answer: a 206 with its
/// `Content-Range`, and `Accept-Ranges` when the source says it takes ranges.
pub(crate) fn range_head(
    resp: &reqwest::Response,
    out: axum::http::response::Builder,
) -> axum::http::response::Builder {
    let mut out = out.status(resp.status());
    for h in [
        axum::http::header::CONTENT_RANGE,
        axum::http::header::ACCEPT_RANGES,
    ] {
        if let Some(v) = resp.headers().get(&h) {
            out = out.header(h, v.clone());
        }
    }
    out
}

/// The source's 416 for a range past the end, passed to the player as it is with the
/// `Content-Range` that names the file's length (each handler would answer it 502), so the
/// player knows where the file ends.
pub(crate) fn unsatisfiable(resp: &reqwest::Response) -> Option<axum::response::Response> {
    if resp.status() != reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        return None;
    }
    let mut out = axum::response::Response::new(axum::body::Body::empty());
    *out.status_mut() = axum::http::StatusCode::RANGE_NOT_SATISFIABLE;
    if let Some(v) = resp.headers().get(axum::http::header::CONTENT_RANGE) {
        out.headers_mut()
            .insert(axum::http::header::CONTENT_RANGE, v.clone());
    }
    Some(out)
}

/// Send a request whose answer is relayed as it arrives: no total limit of the node's client,
/// the headers within [`RELAY_IDLE`]. The error has no URL in it (it may carry a token).
pub(crate) async fn send(rb: reqwest::RequestBuilder) -> Result<reqwest::Response, String> {
    send_within(rb, RELAY_IDLE).await
}

async fn send_within(rb: reqwest::RequestBuilder, idle: Duration) -> Result<reqwest::Response, String> {
    match tokio::time::timeout(idle, rb.timeout(RELAY_MAX).send()).await {
        Ok(Ok(r)) => Ok(r),
        Ok(Err(e)) => Err(e.without_url().to_string()),
        Err(_) => Err("no answer".into()),
    }
}

/// The answer's body for the player, piece by piece, ending with an error when the source sends
/// nothing for [`RELAY_IDLE`] while it is read.
pub(crate) fn body(resp: reqwest::Response) -> axum::body::Body {
    body_within(resp, RELAY_IDLE)
}

fn body_within(resp: reqwest::Response, idle: Duration) -> axum::body::Body {
    let pieces = futures_util::stream::unfold(Some(Box::pin(resp.bytes_stream())), move |s| async move {
        let mut s = s?;
        match tokio::time::timeout(idle, s.next()).await {
            Ok(Some(Ok(b))) => Some((Ok(b), Some(s))),
            Ok(Some(Err(e))) => Some((Err(std::io::Error::other(e.without_url())), None)),
            Ok(None) => None,
            Err(_) => Some((
                Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "the source sent nothing",
                )),
                None,
            )),
        }
    });
    axum::body::Body::from_stream(pieces)
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A source that sends its body in pieces, `gap` apart, `pieces` times.
    async fn slow_source(pieces: usize, gap: Duration) -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = l.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let _ = sock.read(&mut buf).await;
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: {pieces}\r\nConnection: close\r\n\r\n"
            );
            sock.write_all(head.as_bytes()).await.unwrap();
            for _ in 0..pieces {
                tokio::time::sleep(gap).await;
                if sock.write_all(b"x").await.is_err() {
                    return;
                }
            }
        });
        format!("http://{addr}/song")
    }

    /// A body that takes longer in all than the client's own limit still comes through whole
    /// when each piece comes within the idle limit.
    #[tokio::test]
    async fn a_relay_has_no_total_limit_only_an_idle_one() {
        // The node's client, with its total limit made short here
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(600))
            .build()
            .unwrap();
        let url = slow_source(5, Duration::from_millis(250)).await;
        let idle = Duration::from_millis(500);
        let resp = send_within(client.get(&url), idle).await.unwrap();
        let got = body_within(resp, idle)
            .collect()
            .await
            .expect("the whole body")
            .to_bytes();
        assert_eq!(&got[..], b"xxxxx");
    }

    /// A source that stops sending ends the body with an error after the idle limit.
    #[tokio::test]
    async fn a_relay_ends_when_the_source_goes_quiet() {
        let client = reqwest::Client::new();
        let idle = Duration::from_millis(500);
        let url = slow_source(2, idle * 3).await;
        let resp = send_within(client.get(&url), idle).await.unwrap();
        let t = std::time::Instant::now();
        assert!(body_within(resp, idle).collect().await.is_err());
        assert!(t.elapsed() < idle * 3, "{:?}", t.elapsed());
    }

    /// The player's range goes to the source, and the source's 206 with its range headers comes
    /// back; a request without a range is passed on as before.
    #[tokio::test]
    async fn a_players_range_reaches_the_source_and_its_answer_comes_back() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = l.accept().await.unwrap();
                let mut buf = vec![0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(&buf[..n]).to_lowercase();
                let _ = tx.send(req.clone());
                let reply = if req.contains("range: bytes=20-") {
                    "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */10\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                } else if req.contains("range: bytes=6-") {
                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 6-9/10\r\nAccept-Ranges: bytes\r\nContent-Length: 4\r\nConnection: close\r\n\r\n6789"
                } else {
                    "HTTP/1.1 200 OK\r\nAccept-Ranges: bytes\r\nContent-Length: 10\r\nConnection: close\r\n\r\n0123456789"
                };
                let _ = sock.write_all(reply.as_bytes()).await;
            }
        });
        let client = reqwest::Client::new();
        let url = format!("http://{addr}/song");
        let mut player = axum::http::HeaderMap::new();
        player.insert(axum::http::header::RANGE, "bytes=6-".parse().unwrap());
        let resp = send(ranged(client.get(&url), &player)).await.unwrap();
        assert!(
            rx.recv().await.unwrap().contains("range: bytes=6-"),
            "the range went on"
        );
        let out = range_head(&resp, axum::http::Response::builder())
            .body(body(resp))
            .unwrap();
        assert_eq!(out.status(), 206);
        assert_eq!(out.headers()["content-range"], "bytes 6-9/10");
        assert_eq!(out.headers()["accept-ranges"], "bytes");
        let got = out.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&got[..], b"6789");
        // No range: a whole answer
        let resp = send(ranged(client.get(&url), &axum::http::HeaderMap::new()))
            .await
            .unwrap();
        assert!(!rx.recv().await.unwrap().contains("range:"));
        let out = range_head(&resp, axum::http::Response::builder())
            .body(body(resp))
            .unwrap();
        assert_eq!(out.status(), 200);
        assert!(out.headers().get("content-range").is_none());
        // A range of several parts is not passed on
        let mut several = axum::http::HeaderMap::new();
        several.insert(axum::http::header::RANGE, "bytes=0-1,5-6".parse().unwrap());
        let whole = send(ranged(client.get(&url), &several)).await.unwrap();
        assert!(!rx.recv().await.unwrap().contains("range:"));
        assert!(unsatisfiable(&whole).is_none(), "a 200 is not a 416");
        // A range past the end: the source's 416 and its length go back to the player
        let mut past = axum::http::HeaderMap::new();
        past.insert(axum::http::header::RANGE, "bytes=20-".parse().unwrap());
        let resp = send(ranged(client.get(&url), &past)).await.unwrap();
        assert!(rx.recv().await.unwrap().contains("range: bytes=20-"));
        let out = unsatisfiable(&resp).expect("a 416 for the player");
        assert_eq!(out.status(), 416);
        assert_eq!(out.headers()["content-range"], "bytes */10");
    }
}
