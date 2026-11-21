//! Streams in phone mode, for the app's own player and the page's `<audio>`: the routes the node's
//! page uses (`/v1/sources/{tidal|qobuz}/stream/{id}`), on this server's own origin.
//!
//! - **TIDAL:** the full track (`pm_sources::tidal::full::open`: one file, or DASH segments read
//!   in order) is written once to the app's cache and served from there with byte ranges, so the
//!   player seeks in it like a file. A few recent tracks are kept; the rest are removed. Nothing
//!   encrypted is ever opened, and nothing is kept past the cache: streams are not downloads.
//! - **Qobuz:** the signed file URL, relayed as it comes (as the node relays it).

use super::{err, qobuz, tidal, Phone};
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use std::io::Read;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, OnceLock};

/// The most one track may be (a long hi-res track is a few hundred MB).
const MAX_BYTES: u64 = 300 * 1024 * 1024;
/// How many tracks the cache keeps (the one playing and the next).
const KEEP: usize = 3;
/// A fetch nobody reads any more (the person moved on) stops after this.
const UNREAD_FOR: std::time::Duration = std::time::Duration::from_secs(5);

/// The blocking client the full-track readers use (built once, off the async runtime).
pub(crate) fn blocking_client() -> &'static reqwest::blocking::Client {
    static C: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("an HTTP client")
    })
}

/// How long a first ask waits for the whole track before it is sent as it arrives (the app's
/// player gives up on an answer after 30 s; a whole track answers with ranges, so it seeks).
const WHOLE_WITHIN_MS: u64 = 12_000;

/// Where one track's fetch is.
#[derive(Clone, Debug)]
enum Fetch {
    /// Being written to this file (of this media type); this many bytes so far.
    Running(PathBuf, u64, &'static str),
    /// Whole, in the cache.
    Done(PathBuf),
    /// Didn't work: the answer to give.
    Failed(StatusCode, String),
}

/// The cache of fetched tracks (`streams/` in the app's cache), emptied at every start, and the
/// fetches on their way: one per track, whoever asked first; a later ask (a retry, the player
/// reading again) follows the same one.
pub struct Streams {
    dir: PathBuf,
    fetches: std::sync::Mutex<std::collections::HashMap<String, tokio::sync::watch::Receiver<Fetch>>>,
    /// [`WHOLE_WITHIN_MS`] (tests shorten it).
    whole_within_ms: std::sync::atomic::AtomicU64,
}

impl Streams {
    pub fn open(dir: &FsPath) -> std::io::Result<Self> {
        if dir.is_dir() {
            for e in std::fs::read_dir(dir)?.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            fetches: Default::default(),
            whole_within_ms: WHOLE_WITHIN_MS.into(),
        })
    }

    /// How long a first ask waits for the whole track (tests: shorter).
    pub fn set_whole_within(&self, d: std::time::Duration) {
        self.whole_within_ms
            .store(d.as_millis() as u64, std::sync::atomic::Ordering::Relaxed);
    }

    fn kept(&self, name: &str) -> Option<PathBuf> {
        ["flac", "m4a"]
            .iter()
            .map(|ext| self.dir.join(format!("{name}.{ext}")))
            .find(|p| p.is_file())
    }

    /// Keeps the newest [`KEEP`] whole tracks (never one being written).
    fn trim(&self) {
        let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x != "part"))
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        files.sort();
        while files.len() > KEEP {
            let (_, p) = files.remove(0);
            let _ = std::fs::remove_file(p);
        }
    }

    /// The fetch of `id`: the one on its way, or a new one (`start` runs it on a blocking thread
    /// and reports through the sender).
    fn fetch(
        &self,
        id: &str,
        start: impl FnOnce(Arc<tokio::sync::watch::Sender<Fetch>>, Arc<std::sync::atomic::AtomicBool>)
            + Send
            + 'static,
    ) -> tokio::sync::watch::Receiver<Fetch> {
        let mut f = self.fetches.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(rx) = f.get(id) {
            if !matches!(&*rx.borrow(), Fetch::Done(_) | Fetch::Failed(..)) {
                return rx.clone();
            }
        }
        let (tx, rx) = tokio::sync::watch::channel(Fetch::Running(PathBuf::new(), 0, "audio/flac"));
        let tx = Arc::new(tx);
        f.insert(id.to_string(), rx.clone());
        // Nobody reading it for UNREAD_FOR (only the map's receiver left): the fetch stops at its
        // next chunk
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (watch_tx, watch_stop) = (tx.clone(), stop.clone());
        tokio::spawn(async move {
            let mut alone = None::<tokio::time::Instant>;
            loop {
                if matches!(&*watch_tx.borrow(), Fetch::Done(_) | Fetch::Failed(..)) {
                    return;
                }
                if watch_tx.receiver_count() > 1 {
                    alone = None;
                } else if alone.get_or_insert_with(tokio::time::Instant::now).elapsed() >= UNREAD_FOR {
                    watch_stop.store(true, std::sync::atomic::Ordering::Relaxed);
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
        });
        tokio::task::spawn_blocking(move || start(tx, stop));
        rx
    }
}

/// A unique name for one fetch's file being written.
fn part_name(name: &str) -> String {
    let mut b = [0u8; 8];
    let _ = getrandom::fill(&mut b);
    format!(
        "{name}.{}.part",
        b.iter().map(|x| format!("{x:02x}")).collect::<String>()
    )
}

/// Reads `reader` into `<dir>/<name>.<ext>` through a `.part` file of this fetch's own, at most
/// [`MAX_BYTES`], telling `progress` the bytes written as they come.
fn write_track(
    dir: &FsPath,
    name: &str,
    ext: &str,
    reader: &mut dyn Read,
    progress: &dyn Fn(&FsPath, u64),
    keep_on: &dyn Fn() -> bool,
) -> Result<PathBuf, String> {
    use std::io::Write;
    let part = dir.join(part_name(name));
    let done = dir.join(format!("{name}.{ext}"));
    let mut f = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut n: u64 = 0;
    let fail = |e: String| {
        let _ = std::fs::remove_file(&part);
        e
    };
    loop {
        if !keep_on() {
            return Err(fail("nobody plays it any more".into()));
        }
        let k = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(k) => k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(fail(e.to_string())),
        };
        n += k as u64;
        if n > MAX_BYTES {
            return Err(fail("the track is larger than the phone takes".into()));
        }
        f.write_all(&buf[..k]).map_err(|e| fail(e.to_string()))?;
        f.flush().map_err(|e| fail(e.to_string()))?;
        progress(&part, n);
    }
    if n == 0 {
        return Err(fail("TIDAL sent nothing".into()));
    }
    std::fs::rename(&part, &done).map_err(|e| fail(e.to_string()))?;
    Ok(done)
}

fn numeric(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
}

/// The track as it is written: from the start, each part as soon as it is in the file, to the end
/// (no length, no ranges: this play can't seek; the next one, from the whole file, can).
fn following(rx: tokio::sync::watch::Receiver<Fetch>, file: tokio::fs::File, mime: &'static str) -> Response {
    let body = futures_util::stream::unfold((Some(file), rx, false), |(file, mut rx, ended)| async move {
        use tokio::io::AsyncReadExt;
        if ended {
            return None;
        }
        let mut file = file?;
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let k = file.read(&mut buf).await.ok()?;
            if k > 0 {
                buf.truncate(k);
                return Some((Ok::<_, std::io::Error>(buf), (Some(file), rx, false)));
            }
            // At the end of what is written: the track's end, or wait for more
            let state = rx.borrow_and_update().clone();
            match state {
                Fetch::Done(_) => {
                    // Every byte was written before Done: what is left of the (same, renamed)
                    // file is the rest
                    let mut rest = Vec::new();
                    file.read_to_end(&mut rest).await.ok()?;
                    return (!rest.is_empty()).then(|| (Ok(rest), (None, rx, true)));
                }
                // The song must not just end: the player says it broke off
                Fetch::Failed(_, m) => {
                    return Some((Err(std::io::Error::other(m)), (None, rx, true)));
                }
                Fetch::Running(..) => rx.changed().await.ok()?,
            }
        }
    });
    let mut r = Response::new(Body::from_stream(body));
    r.headers_mut()
        .insert(header::CONTENT_TYPE, axum::http::HeaderValue::from_static(mime));
    r
}

/// `GET /v1/sources/tidal/stream/{id}`: the full track. Whole in the cache: with ranges. Still on
/// its way after [`WHOLE_WITHIN_MS`]: as it arrives.
pub(crate) async fn tidal(
    State(p): State<Arc<Phone>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !numeric(&id) {
        return err(StatusCode::BAD_REQUEST, "bad TIDAL track id");
    }
    // A cached track plays only while the person is signed in to TIDAL here
    if !tidal::streams(&p) {
        return err(
            StatusCode::PRECONDITION_FAILED,
            "Sign in to TIDAL first (Settings › Streaming services)",
        );
    }
    let name = format!("tidal-{id}");
    if let Some(f) = p.streams.kept(&name) {
        return crate::server::send_file(&f, &headers).await;
    }
    // The session as phone mode keeps it fresh (renewed under the vault's lock where the vault
    // has it, never beside a renewal running there): the stream itself never renews
    let sess = match tidal::session(&p).await {
        Ok(s) => s,
        Err(e) => return super::source_err(e),
    };
    let quality = pm_sources::tidal::full::quality_of(&pm_sources::tidal::full::load(p.store.as_ref()));
    let p2 = p.clone();
    let id2 = id.clone();
    let mut rx = p.streams.fetch(&id, move |tx, stop| {
        let r = pm_sources::tidal::full::open_as(blocking_client(), &tidal::endpoints(&p2), &sess, quality, &id2)
            .map(Some);
        pm_sources::tidal::full::record(p2.store.as_ref(), &r);
        let end = match r {
            Ok(Some(mut full)) => {
                let (ext, mime) = if full.mime == "audio/flac" {
                    ("flac", "audio/flac")
                } else {
                    ("m4a", "audio/mp4")
                };
                let progress = |part: &FsPath, n: u64| {
                    let _ = tx.send(Fetch::Running(part.to_path_buf(), n, mime));
                };
                let keep_on = || !stop.load(std::sync::atomic::Ordering::Relaxed);
                match write_track(&p2.streams.dir, &name, ext, &mut full.reader, &progress, &keep_on) {
                    Ok(path) => {
                        tracing::info!(track = %id2, bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0), "phone mode: TIDAL full track fetched");
                        p2.streams.trim();
                        Fetch::Done(path)
                    }
                    Err(e) => Fetch::Failed(StatusCode::BAD_GATEWAY, format!("TIDAL: {e}")),
                }
            }
            Ok(None) => Fetch::Failed(
                StatusCode::PRECONDITION_FAILED,
                "Sign in to TIDAL first (Settings › Streaming services)".into(),
            ),
            Err(pm_sources::tidal::full::FullError::Mode(m)) => Fetch::Failed(StatusCode::PRECONDITION_FAILED, m),
            Err(pm_sources::tidal::full::FullError::Track(m)) => Fetch::Failed(StatusCode::NOT_FOUND, m),
        };
        let _ = tx.send(end);
    });
    // Whole within a few seconds (most tracks): with ranges; else as it arrives
    let deadline = tokio::time::sleep(std::time::Duration::from_millis(
        p.streams
            .whole_within_ms
            .load(std::sync::atomic::Ordering::Relaxed),
    ));
    tokio::pin!(deadline);
    loop {
        let state = rx.borrow_and_update().clone();
        match state {
            Fetch::Done(path) => return crate::server::send_file(&path, &headers).await,
            Fetch::Failed(status, m) => return err(status, &m),
            Fetch::Running(part, n, mime) if n > 0 && deadline.is_elapsed() => {
                // Only a read from the start follows a fetch; a range waits for the whole track
                if headers
                    .get(header::RANGE)
                    .is_none_or(|r| r.as_bytes() == b"bytes=0-")
                {
                    // (renamed meanwhile when whole: the next look sees Done)
                    if let Ok(f) = tokio::fs::File::open(&part).await {
                        return following(rx, f, mime);
                    }
                }
            }
            Fetch::Running(..) => {}
        }
        tokio::select! {
            changed = rx.changed() => if changed.is_err() {
                return err(StatusCode::INTERNAL_SERVER_ERROR, "the TIDAL fetch stopped");
            },
            _ = &mut deadline, if !deadline.is_elapsed() => {}
        }
    }
}

/// `GET /v1/sources/qobuz/stream/{id}`: the track, relayed so the session and the app values never
/// reach the page.
pub(crate) async fn qobuz(State(p): State<Arc<Phone>>, Path(id): Path<String>) -> Response {
    if !numeric(&id) {
        return err(StatusCode::BAD_REQUEST, "bad Qobuz id");
    }
    let (app, sess) = match qobuz::ready(&p) {
        Ok(x) => x,
        Err(e) => return super::source_err(e),
    };
    let st = qobuz::state(&p);
    let format = pm_sources::qobuz::format_id(&st);
    let r = pm_sources::qobuz::file_url(&p.http, p.store.as_ref(), app, &sess.token, &id, format).await;
    let url = match r {
        Ok(u) => u,
        Err(pm_sources::qobuz::StreamError::Mode(m)) => {
            qobuz::mark_down(&p, &m);
            return err(StatusCode::BAD_GATEWAY, &m);
        }
        Err(pm_sources::qobuz::StreamError::Track(m)) => return err(StatusCode::NOT_FOUND, &m),
        Err(pm_sources::qobuz::StreamError::Unavailable(m)) => {
            return err(StatusCode::SERVICE_UNAVAILABLE, &m)
        }
    };
    // The track itself takes as long as it takes: no overall timeout on this one
    let resp = match reqwest::Client::new().get(&url).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            return err(
                StatusCode::BAD_GATEWAY,
                &format!("Qobuz: the stream answered {}", r.status()),
            )
        }
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("Qobuz: {}", e.without_url())),
    };
    let len = resp.content_length();
    let mut out = Response::builder()
        .header(header::CONTENT_TYPE, pm_sources::qobuz::content_type_for(format))
        .header(header::CACHE_CONTROL, "private, max-age=600");
    if let Some(n) = len {
        out = out.header(header::CONTENT_LENGTH, n);
    }
    out.body(Body::from_stream(resp.bytes_stream()))
        .unwrap_or_else(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "the stream can't be sent"))
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_track_is_written_whole_and_the_cache_keeps_the_newest() {
        let dir = tempfile::tempdir().unwrap();
        let s = Streams::open(dir.path()).unwrap();
        for i in 0..5 {
            let mut r: &[u8] = b"fLaC....";
            write_track(&s.dir, &format!("tidal-{i}"), "flac", &mut r, &|_, _| {}, &|| {
                true
            })
            .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
            s.trim();
        }
        assert!(s.kept("tidal-4").is_some() && s.kept("tidal-2").is_some());
        assert!(s.kept("tidal-0").is_none() && s.kept("tidal-1").is_none());
        let mut empty: &[u8] = b"";
        assert!(write_track(&s.dir, "tidal-9", "flac", &mut empty, &|_, _| {}, &|| true).is_err());
        assert!(s.kept("tidal-9").is_none(), "nothing half-written is served");
        // A new start begins empty
        let again = Streams::open(dir.path()).unwrap();
        assert!(again.kept("tidal-4").is_none());
    }
}
