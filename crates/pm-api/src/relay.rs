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
    let pieces = futures_util::stream::unfold(Some(Box::pin(resp.bytes_stream())), |s| async move {
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
}
