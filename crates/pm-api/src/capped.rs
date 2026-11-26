//! Another server's answer read whole, up to a size.
//!
//! `reqwest`'s `bytes()` and `text()` keep whatever a server sends; the node's clients stop after a
//! time, not after a size. Where the address comes from outside (a feed, a station, a picture, a
//! server the owner named), these read at most `max` bytes and fail beyond that.

/// Why an answer was not read whole.
#[derive(Debug, PartialEq)]
pub(crate) enum Error {
    /// Longer than this many bytes.
    TooLong(usize),
    /// The read itself failed.
    Read(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::TooLong(max) => write!(f, "the answer is longer than {} KiB", max >> 10),
            Error::Read(e) => f.write_str(e),
        }
    }
}

impl From<Error> for String {
    fn from(e: Error) -> String {
        e.to_string()
    }
}

/// The body of `resp`; an error when it says, or turns out, to be longer than `max` bytes.
pub(crate) async fn bytes(mut resp: reqwest::Response, max: usize) -> Result<Vec<u8>, Error> {
    if resp.content_length().is_some_and(|n| n > max as u64) {
        return Err(Error::TooLong(max));
    }
    let mut out = Vec::new();
    // Without the address (none on a body error today; kept should one come): a media server's
    // carries its sign-in in the query
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| Error::Read(e.without_url().to_string()))?
    {
        if out.len() + chunk.len() > max {
            return Err(Error::TooLong(max));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// Answers of the media servers the owner signs into (Jellyfin, Plex, OpenSubsonic,
/// Audiobookshelf), at most: a whole library's listing fits many times over.
pub(crate) const MEDIA_SERVER_MAX: usize = 64 << 20;

/// [`bytes`] parsed as JSON.
pub(crate) async fn json<T: serde::de::DeserializeOwned>(
    resp: reqwest::Response,
    max: usize,
) -> Result<T, Error> {
    let body = bytes(resp, max).await?;
    serde_json::from_slice(&body).map_err(|e| Error::Read(format!("not JSON: {e}")))
}

/// [`bytes`] as text, read as UTF-8 with anything else replaced (as `reqwest`'s `text()` does
/// without its `charset` feature, which the workspace does not turn on).
pub(crate) async fn text(resp: reqwest::Response, max: usize) -> Result<String, Error> {
    bytes(resp, max)
        .await
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// [`bytes`] for the blocking client, within one overall wait.
///
/// The blocking client's timeout bounds each body read on its own (see `sources.rs`), so a
/// server that sends a byte now and then would hold a reader of many small reads for as long as
/// it likes. reqwest's own `bytes()`/`text()`/`json()` read under one wait; this keeps one too:
/// [`BLOCKING_WAIT`] for the whole body, checked after every read, so it ends at most one read
/// wait past it.
fn bytes_blocking(resp: reqwest::blocking::Response, max: usize) -> Result<Vec<u8>, Error> {
    bytes_blocking_within(resp, max, BLOCKING_WAIT)
}

/// [`bytes_blocking`] with its overall wait given.
fn bytes_blocking_within(
    resp: reqwest::blocking::Response,
    max: usize,
    wait: std::time::Duration,
) -> Result<Vec<u8>, Error> {
    use std::io::Read;
    if resp.content_length().is_some_and(|n| n > max as u64) {
        return Err(Error::TooLong(max));
    }
    let deadline = std::time::Instant::now() + wait;
    let mut body = resp.take(max as u64 + 1);
    let mut out = Vec::new();
    let mut chunk = [0u8; 16 << 10];
    loop {
        match body.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(Error::Read(e.to_string())),
        }
        if out.len() > max {
            return Err(Error::TooLong(max));
        }
        if std::time::Instant::now() > deadline {
            return Err(Error::Read("the answer took too long".into()));
        }
    }
    Ok(out)
}

/// The whole-body wait of [`json_blocking`] (HLS playlists, read as text, have the same reader in
/// pm-sources: `capped.rs` there): the stream clients' wait
/// (`sources::STREAM_WAIT`), which reqwest's blocking `text()`/`json()` gave the whole body.
const BLOCKING_WAIT: std::time::Duration = crate::sources::STREAM_WAIT;

/// [`json`] for the blocking client.
pub(crate) fn json_blocking<T: serde::de::DeserializeOwned>(
    resp: reqwest::blocking::Response,
    max: usize,
) -> Result<T, Error> {
    let body = bytes_blocking(resp, max)?;
    serde_json::from_slice(&body).map_err(|e| Error::Read(format!("not JSON: {e}")))
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    /// A server on a free port that answers every connection with `head` and then `body`, and
    /// closes.
    fn serve(head: &'static str, body: Vec<u8>) -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for c in l.incoming() {
                let Ok(mut c) = c else { return };
                let mut req = [0u8; 4096];
                let _ = c.read(&mut req);
                if c.write_all(head.as_bytes()).is_err() {
                    continue;
                }
                let _ = c.write_all(&body);
            }
        });
        url
    }

    /// A server on a free port whose answer has no length and never ends: it writes until the
    /// reader goes away. A reader that keeps the whole body before checking its size never
    /// returns here, so the tests below wait for it with a timeout.
    fn serve_endless() -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for c in l.incoming() {
                let Ok(mut c) = c else { return };
                std::thread::spawn(move || {
                    let mut req = [0u8; 4096];
                    let _ = c.read(&mut req);
                    if c.write_all(OPEN_ENDED.as_bytes()).is_err() {
                        return;
                    }
                    // 16 KiB every 2 ms at most: a reader that keeps it all holds no more than
                    // ~160 MiB by BLOCKING_WAIT (20 s), ~80 MiB by STOPS_WITHIN
                    let chunk = [b'a'; 16 << 10];
                    while c.write_all(&chunk).is_ok() {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                });
            }
        });
        url
    }

    /// Long enough for a capped read of a few hundred KiB, far too short to fill memory.
    const STOPS_WITHIN: std::time::Duration = std::time::Duration::from_secs(10);

    /// No length given: the body ends when the server closes.
    const OPEN_ENDED: &str = "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n";

    #[tokio::test]
    async fn an_answer_without_a_length_is_cut_off_past_the_limit() {
        let url = serve_endless();
        let r = reqwest::get(&url).await.unwrap();
        let e = tokio::time::timeout(STOPS_WITHIN, super::bytes(r, 100 << 10))
            .await
            .expect("the read stops at the limit, not at the end of the body")
            .unwrap_err();
        assert_eq!(e, super::Error::TooLong(100 << 10));
        let r = reqwest::get(&url).await.unwrap();
        let e = tokio::time::timeout(STOPS_WITHIN, super::json::<serde_json::Value>(r, 100 << 10))
            .await
            .expect("the JSON read stops at the limit too")
            .unwrap_err();
        assert_eq!(e, super::Error::TooLong(100 << 10));
        assert_eq!(e.to_string(), "the answer is longer than 100 KiB");
        // Exactly the limit is still whole
        let url = serve(OPEN_ENDED, vec![b'a'; 100 << 10]);
        let r = reqwest::get(&url).await.unwrap();
        assert_eq!(super::bytes(r, 100 << 10).await.unwrap().len(), 100 << 10);
        let url = serve(OPEN_ENDED, "Grüße".as_bytes().to_vec());
        let r = reqwest::get(&url).await.unwrap();
        assert_eq!(super::text(r, 100).await.unwrap(), "Grüße");
    }

    #[tokio::test]
    async fn a_length_over_the_limit_is_refused_before_the_body() {
        let url = serve(
            "HTTP/1.1 200 OK\r\nContent-Length: 4294967296\r\nConnection: close\r\n\r\n",
            Vec::new(),
        );
        let r = reqwest::get(&url).await.unwrap();
        assert_eq!(
            super::bytes(r, 1 << 20).await,
            Err(super::Error::TooLong(1 << 20))
        );
    }

    #[test]
    fn the_blocking_reader_stops_at_the_limit_too() {
        // A whole-body read of the endless answer ends in this client's timeout (reqwest's
        // text()/json()) or in BLOCKING_WAIT (a read loop without the limit): a Read error, not
        // TooLong
        let client = reqwest::blocking::Client::builder()
            .timeout(STOPS_WITHIN)
            .build()
            .unwrap();
        let url = serve_endless();
        let r = client.get(&url).send().unwrap();
        assert_eq!(
            super::bytes_blocking(r, 100 << 10),
            Err(super::Error::TooLong(100 << 10))
        );
        let r = client.get(&url).send().unwrap();
        assert_eq!(
            super::json_blocking::<serde_json::Value>(r, 100 << 10),
            Err(super::Error::TooLong(100 << 10))
        );
        let url = serve(OPEN_ENDED, b"#EXTM3U\n".to_vec());
        let r = reqwest::blocking::get(&url).unwrap();
        assert_eq!(super::bytes_blocking(r, 100 << 10).unwrap(), b"#EXTM3U\n");
        let url = serve(
            "HTTP/1.1 200 OK\r\nContent-Length: 4294967296\r\nConnection: close\r\n\r\n",
            Vec::new(),
        );
        let r = reqwest::blocking::get(&url).unwrap();
        assert_eq!(
            super::bytes_blocking(r, 1 << 20),
            Err(super::Error::TooLong(1 << 20))
        );
    }

    /// A server that sends a byte now and then (each read well inside the client's timeout)
    /// cannot hold a blocking read past its overall wait.
    #[test]
    fn a_trickling_answer_ends_at_the_blocking_wait() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for c in l.incoming() {
                let Ok(mut c) = c else { return };
                std::thread::spawn(move || {
                    let mut req = [0u8; 4096];
                    let _ = c.read(&mut req);
                    if c.write_all(OPEN_ENDED.as_bytes()).is_err() {
                        return;
                    }
                    while c.write_all(b"a").is_ok() {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                });
            }
        });
        let client = reqwest::blocking::Client::builder()
            .timeout(STOPS_WITHIN)
            .build()
            .unwrap();
        let r = client.get(&url).send().unwrap();
        // On a thread: without the overall wait this read would never end
        let (tx, rx) = std::sync::mpsc::channel();
        let started = std::time::Instant::now();
        std::thread::spawn(move || {
            let _ = tx.send(super::bytes_blocking_within(
                r,
                1 << 20,
                std::time::Duration::from_secs(1),
            ));
        });
        let got = rx
            .recv_timeout(STOPS_WITHIN)
            .expect("the read ends at its overall wait");
        assert_eq!(got, Err(super::Error::Read("the answer took too long".into())));
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    /// A media server's answer is read as JSON up to its size, and a failed read does not name
    /// the address (a Subsonic server's carries the sign-in in its query). reqwest's body errors
    /// carry no address today; `without_url` keeps it so if that changes.
    #[tokio::test]
    async fn json_is_read_up_to_its_size_and_errors_leave_the_address_out() {
        let url = serve(OPEN_ENDED, br#"{"ok": [1, 2]}"#.to_vec());
        let r = reqwest::get(&url).await.unwrap();
        let v: serde_json::Value = super::json(r, 100).await.unwrap();
        assert_eq!(v["ok"][1], 2);
        let url = serve(OPEN_ENDED, vec![b' '; 300]);
        let r = reqwest::get(&url).await.unwrap();
        let e = super::json::<serde_json::Value>(r, 100).await.unwrap_err();
        assert_eq!(e, super::Error::TooLong(100));
        // Says 100 bytes, sends 10, closes
        let url = serve(
            "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n",
            b"{\"ok\": 1,".to_vec(),
        );
        let r = reqwest::get(format!("{url}rest/ping?u=me&p=s3cret"))
            .await
            .unwrap();
        let e = super::json::<serde_json::Value>(r, 1000).await.unwrap_err();
        assert!(matches!(e, super::Error::Read(_)), "{e:?}");
        assert!(!e.to_string().contains("s3cret"), "{e}");
    }
}
