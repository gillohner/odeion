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
    while let Some(chunk) = resp.chunk().await.map_err(|e| Error::Read(e.to_string()))? {
        if out.len() + chunk.len() > max {
            return Err(Error::TooLong(max));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// [`bytes`] as text, read as UTF-8 with anything else replaced (as `reqwest`'s `text()` does
/// without its `charset` feature, which the workspace does not turn on).
pub(crate) async fn text(resp: reqwest::Response, max: usize) -> Result<String, Error> {
    bytes(resp, max)
        .await
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// [`text`] for the blocking client.
pub(crate) fn text_blocking(resp: reqwest::blocking::Response, max: usize) -> Result<String, Error> {
    use std::io::Read;
    if resp.content_length().is_some_and(|n| n > max as u64) {
        return Err(Error::TooLong(max));
    }
    let mut out = Vec::new();
    resp.take(max as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| Error::Read(e.to_string()))?;
    if out.len() > max {
        return Err(Error::TooLong(max));
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
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

    /// No length given: the body ends when the server closes.
    const OPEN_ENDED: &str = "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n";

    #[tokio::test]
    async fn an_answer_without_a_length_is_cut_off_past_the_limit() {
        let url = serve(OPEN_ENDED, vec![b'a'; 300 << 10]);
        let r = reqwest::get(&url).await.unwrap();
        let e = super::bytes(r, 100 << 10).await.unwrap_err();
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
        let url = serve(OPEN_ENDED, vec![b'#'; 300 << 10]);
        let r = reqwest::blocking::get(&url).unwrap();
        assert_eq!(
            super::text_blocking(r, 100 << 10),
            Err(super::Error::TooLong(100 << 10))
        );
        let url = serve(OPEN_ENDED, b"#EXTM3U\n".to_vec());
        let r = reqwest::blocking::get(&url).unwrap();
        assert_eq!(super::text_blocking(r, 100 << 10).unwrap(), "#EXTM3U\n");
        let url = serve(
            "HTTP/1.1 200 OK\r\nContent-Length: 4294967296\r\nConnection: close\r\n\r\n",
            Vec::new(),
        );
        let r = reqwest::blocking::get(&url).unwrap();
        assert_eq!(
            super::text_blocking(r, 1 << 20),
            Err(super::Error::TooLong(1 << 20))
        );
    }
}
