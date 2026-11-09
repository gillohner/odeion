//! An HTTP resource as a byte stream symphonia can read and seek: a seek far from where the
//! body is drops it, and the next read asks for the rest from there (`Range: bytes=<pos>-`). A
//! body that breaks off (a mobile link) is asked for again where it broke, a few times. A server
//! that doesn't take ranges is read from the start only, and the decoder cannot seek it.

use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE};
use reqwest::StatusCode;
use std::io::{Read, Seek, SeekFrom};
use std::time::Duration;

/// A seek forward by at most this much reads on in the open body instead of asking again.
const SKIP_IN_BODY: u64 = 256 * 1024;
/// How many times a body that broke off is asked for again (each time from where it broke).
const RETRIES: u32 = 3;

pub struct RangeReader {
    client: Client,
    url: String,
    len: Option<u64>,
    ranges: bool,
    content_type: String,
    pos: u64,
    /// The open body and the offset it is at.
    body: Option<(Response, u64)>,
    /// Pause before asking again (shorter in tests).
    backoff: Duration,
}

impl std::fmt::Debug for RangeReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RangeReader")
            .field("len", &self.len)
            .field("ranges", &self.ranges)
            .field("content_type", &self.content_type)
            .field("pos", &self.pos)
            .finish()
    }
}

/// `bytes 0-99/1234` → 1234.
fn total_of(content_range: &str) -> Option<u64> {
    content_range.rsplit('/').next()?.trim().parse().ok()
}

fn header(r: &Response, name: reqwest::header::HeaderName) -> String {
    r.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

impl RangeReader {
    /// Ask for `url` from its start; an error names what came back instead (`HTTP 404`).
    pub fn open(client: &Client, url: &str) -> Result<Self, String> {
        let resp = client
            .get(url)
            .header(RANGE, "bytes=0-")
            .send()
            .map_err(|e| e.without_url().to_string())?;
        let status = resp.status();
        let (len, ranges) = match status {
            StatusCode::PARTIAL_CONTENT => (total_of(&header(&resp, CONTENT_RANGE)), true),
            StatusCode::OK => (
                header(&resp, CONTENT_LENGTH).parse().ok(),
                header(&resp, ACCEPT_RANGES).eq_ignore_ascii_case("bytes"),
            ),
            other => return Err(format!("HTTP {}", other.as_u16())),
        };
        let content_type = header(&resp, CONTENT_TYPE);
        Ok(Self {
            client: client.clone(),
            url: url.to_string(),
            len,
            ranges,
            content_type,
            pos: 0,
            body: Some((resp, 0)),
            backoff: Duration::from_millis(800),
        })
    }

    /// What the server says it sent (`audio/flac`), without parameters.
    pub fn content_type(&self) -> &str {
        self.content_type.split(';').next().unwrap_or("").trim()
    }

    pub fn len(&self) -> Option<u64> {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == Some(0)
    }

    /// Whether a seek can ask the server for another part.
    pub fn takes_ranges(&self) -> bool {
        self.ranges
    }

    #[cfg(test)]
    fn quick(mut self) -> Self {
        self.backoff = Duration::from_millis(10);
        self
    }

    /// The rest of the resource from `self.pos`.
    fn reopen(&mut self) -> std::io::Result<()> {
        self.body = None;
        if !self.ranges && self.pos != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "the server sends this only from its start",
            ));
        }
        let resp = self
            .client
            .get(&self.url)
            .header(RANGE, format!("bytes={}-", self.pos))
            .send()
            .map_err(|e| std::io::Error::other(e.without_url().to_string()))?;
        match resp.status() {
            StatusCode::PARTIAL_CONTENT => {}
            StatusCode::OK if self.pos == 0 => {}
            StatusCode::RANGE_NOT_SATISFIABLE => {
                // At (or past) the end: nothing more to read
                self.len = Some(self.len.unwrap_or(self.pos).min(self.pos));
                return Ok(());
            }
            other => return Err(std::io::Error::other(format!("HTTP {}", other.as_u16()))),
        }
        self.body = Some((resp, self.pos));
        Ok(())
    }

    fn at_end(&self) -> bool {
        self.len.is_some_and(|l| self.pos >= l)
    }
}

impl Read for RangeReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() || self.at_end() {
            return Ok(0);
        }
        let mut tries = 0;
        loop {
            if self.body.as_ref().is_none_or(|(_, at)| *at != self.pos) {
                if let Err(e) = self.reopen() {
                    // A broken link: once it's back, from here
                    if tries < RETRIES && e.kind() != std::io::ErrorKind::Unsupported {
                        tries += 1;
                        std::thread::sleep(self.backoff * tries);
                        continue;
                    }
                    return Err(e);
                }
                if self.body.is_none() {
                    return Ok(0);
                }
            }
            let (body, at) = self.body.as_mut().expect("opened above");
            match body.read(buf) {
                Ok(0) if self.len.is_none_or(|l| *at >= l) => return Ok(0),
                // Ended short of its length: broke off
                Ok(0) => {}
                Ok(n) => {
                    *at += n as u64;
                    self.pos += n as u64;
                    return Ok(n);
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => tracing::debug!(pos = self.pos, "stream broke off: {e}"),
            }
            self.body = None;
            if !self.ranges || tries >= RETRIES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "the stream broke off",
                ));
            }
            tries += 1;
            std::thread::sleep(self.backoff * tries);
        }
    }
}

impl Seek for RangeReader {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let target = match to {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
            SeekFrom::End(d) => self
                .len
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Unsupported, "length unknown"))?
                .checked_add_signed(d),
        }
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek before the start"))?;
        if target == self.pos {
            return Ok(target);
        }
        if !self.ranges {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "the server sends this only from its start",
            ));
        }
        // A little ahead in the open body: read on to it
        if let Some((body, at)) = self.body.as_mut() {
            if *at == self.pos && target > self.pos && target - self.pos <= SKIP_IN_BODY {
                let want = target - self.pos;
                let got = std::io::copy(&mut body.by_ref().take(want), &mut std::io::sink()).unwrap_or(0);
                *at += got;
                if got != want {
                    self.body = None;
                }
            }
        }
        self.pos = target;
        Ok(target)
    }
}

impl symphonia::core::io::MediaSource for RangeReader {
    fn is_seekable(&self) -> bool {
        self.ranges && self.len.is_some()
    }

    fn byte_len(&self) -> Option<u64> {
        self.len
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// How the test server answers.
    #[derive(Clone, Copy, Default)]
    pub struct Mode {
        /// Answer 200 with the whole body, ignoring Range.
        pub no_ranges: bool,
        /// Break off the first body after this many bytes.
        pub break_at: Option<usize>,
    }

    /// A one-file HTTP server on loopback: `GET` with `Range: bytes=a-` answers 206 from `a`.
    /// Returns its URL and the Range headers it was asked with.
    pub fn serve(body: Vec<u8>, content_type: &str, mode: Mode) -> (String, Arc<Mutex<Vec<String>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/v1/library/tracks/1/file?token=t",
            l.local_addr().unwrap()
        );
        let asked: Arc<Mutex<Vec<String>>> = Arc::default();
        let (asked2, ct) = (asked.clone(), content_type.to_string());
        let broke = Arc::new(AtomicUsize::new(0));
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(s) = s else { continue };
                let (body, asked, ct, broke) = (body.clone(), asked2.clone(), ct.clone(), broke.clone());
                std::thread::spawn(move || {
                    let mut r = BufReader::new(s.try_clone().unwrap());
                    let mut line = String::new();
                    if r.read_line(&mut line).is_err() || line.is_empty() {
                        return;
                    }
                    let mut from = None;
                    loop {
                        let mut h = String::new();
                        if r.read_line(&mut h).is_err() || h.trim().is_empty() {
                            break;
                        }
                        if let Some(v) = h.to_ascii_lowercase().strip_prefix("range:") {
                            let v = v.trim().to_string();
                            asked.lock().unwrap().push(v.clone());
                            from = v
                                .strip_prefix("bytes=")
                                .and_then(|x| x.trim_end_matches('-').parse::<usize>().ok());
                        }
                    }
                    let mut w = s;
                    let len = body.len();
                    let (head, part) = match from {
                        Some(a) if !mode.no_ranges && a >= len => {
                            let _ = write!(w, "HTTP/1.1 416 Range Not Satisfiable\r\ncontent-range: bytes */{len}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                            return;
                        }
                        Some(a) if !mode.no_ranges => (
                            format!("HTTP/1.1 206 Partial Content\r\ncontent-type: {ct}\r\ncontent-range: bytes {a}-{}/{len}\r\ncontent-length: {}\r\naccept-ranges: bytes\r\nconnection: close\r\n\r\n", len - 1, len - a),
                            &body[a..],
                        ),
                        _ => (
                            format!("HTTP/1.1 200 OK\r\ncontent-type: {ct}\r\ncontent-length: {len}\r\nconnection: close\r\n\r\n"),
                            &body[..],
                        ),
                    };
                    let _ = w.write_all(head.as_bytes());
                    match mode.break_at {
                        Some(n) if broke.fetch_add(1, Ordering::SeqCst) == 0 => {
                            let _ = w.write_all(&part[..n.min(part.len())]);
                        }
                        _ => {
                            let _ = w.write_all(part);
                        }
                    }
                });
            }
        });
        (url, asked)
    }

    fn bytes(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 31 % 251) as u8).collect()
    }

    fn client() -> Client {
        Client::builder().build().unwrap()
    }

    #[test]
    fn reads_the_whole_body_and_says_its_length_and_type() {
        let want = bytes(100_000);
        let (url, _) = serve(want.clone(), "audio/flac", Mode::default());
        let mut r = RangeReader::open(&client(), &url).unwrap();
        assert_eq!(r.len(), Some(100_000));
        assert_eq!(r.content_type(), "audio/flac");
        assert!(symphonia::core::io::MediaSource::is_seekable(&r));
        let mut got = Vec::new();
        r.read_to_end(&mut got).unwrap();
        assert_eq!(got, want);
    }

    /// A seek far away asks for the rest from there; a short one reads on; the end is the end.
    #[test]
    fn a_seek_asks_for_the_part_from_there() {
        let want = bytes(2_000_000);
        let (url, asked) = serve(want.clone(), "audio/flac", Mode::default());
        let mut r = RangeReader::open(&client(), &url).unwrap();
        let mut buf = [0u8; 1000];
        r.seek(SeekFrom::Start(1_500_000)).unwrap();
        r.read_exact(&mut buf).unwrap();
        assert_eq!(&buf[..], &want[1_500_000..1_501_000]);
        // Close ahead: no new request
        r.seek(SeekFrom::Current(10_000)).unwrap();
        r.read_exact(&mut buf).unwrap();
        assert_eq!(&buf[..], &want[1_511_000..1_512_000]);
        // Back to the start, then from the end
        r.seek(SeekFrom::Start(0)).unwrap();
        r.read_exact(&mut buf).unwrap();
        assert_eq!(&buf[..], &want[..1000]);
        r.seek(SeekFrom::End(-10)).unwrap();
        let mut tail = Vec::new();
        r.read_to_end(&mut tail).unwrap();
        assert_eq!(tail, &want[want.len() - 10..]);
        assert_eq!(
            *asked.lock().unwrap(),
            vec!["bytes=0-", "bytes=1500000-", "bytes=0-", "bytes=1999990-"]
        );
        assert_eq!(r.read(&mut buf).unwrap(), 0);
    }

    #[test]
    fn a_body_that_breaks_off_is_asked_for_again_where_it_broke() {
        let want = bytes(300_000);
        let mode = Mode {
            break_at: Some(70_000),
            ..Mode::default()
        };
        let (url, asked) = serve(want.clone(), "audio/wav", mode);
        let mut r = RangeReader::open(&client(), &url).unwrap().quick();
        let mut got = Vec::new();
        r.read_to_end(&mut got).unwrap();
        assert_eq!(got, want);
        assert_eq!(*asked.lock().unwrap(), vec!["bytes=0-", "bytes=70000-"]);
    }

    #[test]
    fn a_server_without_ranges_plays_from_the_start_and_cannot_seek() {
        let want = bytes(50_000);
        let mode = Mode {
            no_ranges: true,
            ..Mode::default()
        };
        let (url, _) = serve(want.clone(), "audio/mpeg", mode);
        let mut r = RangeReader::open(&client(), &url).unwrap();
        assert!(!r.takes_ranges());
        assert!(!symphonia::core::io::MediaSource::is_seekable(&r));
        assert!(r.seek(SeekFrom::Start(10)).is_err());
        let mut got = Vec::new();
        r.read_to_end(&mut got).unwrap();
        assert_eq!(got, want);
    }

    #[test]
    fn an_error_answer_says_its_status() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/x", l.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut b = [0u8; 1024];
            let _ = s.read(&mut b);
            let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        });
        assert_eq!(RangeReader::open(&client(), &url).unwrap_err(), "HTTP 404");
    }
}
