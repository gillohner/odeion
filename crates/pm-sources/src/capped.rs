//! Another server's answer read whole, up to a size, for the blocking client (HLS playlists).
//!
//! The same rule as the node's own reader (pm-api `capped::text_blocking`): at most `max` bytes,
//! and one overall wait for the whole body, so neither an endless nor a trickling answer holds a
//! reader for long. `reqwest`'s blocking `text()` keeps whatever a server sends.

/// The whole-body wait: the stream clients' wait (pm-api `sources::STREAM_WAIT`), which
/// reqwest's blocking `text()` gave the whole body.
const BLOCKING_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

/// The body of `resp` as text (UTF-8, anything else replaced); an error when it says, or turns
/// out, to be longer than `max` bytes, or takes longer than [`BLOCKING_WAIT`] to arrive.
pub(crate) fn text_blocking(resp: reqwest::blocking::Response, max: usize) -> Result<String, String> {
    bytes_blocking_within(resp, max, BLOCKING_WAIT).map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn too_long(max: usize) -> String {
    format!("the answer is longer than {} KiB", max >> 10)
}

/// The body of `resp`, at most `max` bytes, within `wait` (checked after every read, so it ends
/// at most one read wait past it).
fn bytes_blocking_within(
    resp: reqwest::blocking::Response,
    max: usize,
    wait: std::time::Duration,
) -> Result<Vec<u8>, String> {
    use std::io::Read;
    if resp.content_length().is_some_and(|n| n > max as u64) {
        return Err(too_long(max));
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
            Err(e) => return Err(e.to_string()),
        }
        if out.len() > max {
            return Err(too_long(max));
        }
        if std::time::Instant::now() > deadline {
            return Err("the answer took too long".into());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    /// A server on a free port that answers every connection with `head`, then writes `body`
    /// once, or (`endless`) 16 KiB every 2 ms until the reader goes away.
    fn serve(head: &'static str, body: Vec<u8>, endless: bool) -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for c in l.incoming() {
                let Ok(mut c) = c else { return };
                let body = body.clone();
                std::thread::spawn(move || {
                    let mut req = [0u8; 4096];
                    let _ = c.read(&mut req);
                    if c.write_all(head.as_bytes()).is_err() {
                        return;
                    }
                    if !endless {
                        let _ = c.write_all(&body);
                        return;
                    }
                    let chunk = [b'a'; 16 << 10];
                    while c.write_all(&chunk).is_ok() {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                });
            }
        });
        url
    }

    const OPEN_ENDED: &str = "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n";

    #[test]
    fn a_playlist_is_read_up_to_its_size_and_no_further() {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap();
        let r = client
            .get(serve(OPEN_ENDED, b"#EXTM3U\n".to_vec(), false))
            .send()
            .unwrap();
        assert_eq!(super::text_blocking(r, 100 << 10).unwrap(), "#EXTM3U\n");
        // An endless answer stops at the limit, not at the client's timeout
        let r = client.get(serve(OPEN_ENDED, Vec::new(), true)).send().unwrap();
        let started = std::time::Instant::now();
        assert_eq!(
            super::text_blocking(r, 100 << 10),
            Err("the answer is longer than 100 KiB".to_string())
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        // A length over the limit is refused before the body
        let head = "HTTP/1.1 200 OK\r\nContent-Length: 4294967296\r\nConnection: close\r\n\r\n";
        let r = client.get(serve(head, Vec::new(), false)).send().unwrap();
        assert_eq!(
            super::text_blocking(r, 1 << 20),
            Err("the answer is longer than 1024 KiB".to_string())
        );
    }
}
