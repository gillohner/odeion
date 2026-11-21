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

/// The cache of fetched tracks (`phone/streams/`), emptied at every start.
pub struct Streams {
    dir: PathBuf,
    /// One fetch at a time: the second ask for the same track waits and finds it there.
    fetching: tokio::sync::Mutex<()>,
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
            fetching: tokio::sync::Mutex::new(()),
        })
    }

    fn kept(&self, name: &str) -> Option<PathBuf> {
        ["flac", "m4a"]
            .iter()
            .map(|ext| self.dir.join(format!("{name}.{ext}")))
            .find(|p| p.is_file())
    }

    /// Keeps the newest [`KEEP`] tracks.
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
}

/// Reads `reader` into `<dir>/<name>.<ext>` (through a `.part` file), at most [`MAX_BYTES`].
fn write_track(dir: &FsPath, name: &str, ext: &str, reader: &mut dyn Read) -> Result<PathBuf, String> {
    let part = dir.join(format!("{name}.part"));
    let done = dir.join(format!("{name}.{ext}"));
    let mut f = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let n = std::io::copy(&mut reader.take(MAX_BYTES + 1), &mut f).map_err(|e| e.to_string())?;
    if n > MAX_BYTES {
        let _ = std::fs::remove_file(&part);
        return Err("the track is larger than the phone takes".into());
    }
    if n == 0 {
        let _ = std::fs::remove_file(&part);
        return Err("TIDAL sent nothing".into());
    }
    std::fs::rename(&part, &done).map_err(|e| e.to_string())?;
    Ok(done)
}

fn numeric(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
}

/// `GET /v1/sources/tidal/stream/{id}`: the full track, with ranges.
pub(crate) async fn tidal(
    State(p): State<Arc<Phone>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !numeric(&id) {
        return err(StatusCode::BAD_REQUEST, "bad TIDAL track id");
    }
    let name = format!("tidal-{id}");
    if let Some(f) = p.streams.kept(&name) {
        return crate::server::send_file(&f, &headers).await;
    }
    let _one = p.streams.fetching.lock().await;
    if let Some(f) = p.streams.kept(&name) {
        return crate::server::send_file(&f, &headers).await;
    }
    let p2 = p.clone();
    let fetched = tokio::task::spawn_blocking(move || -> Result<Option<PathBuf>, (StatusCode, String)> {
        let r = pm_sources::tidal::full::open(p2.store.as_ref(), blocking_client(), &tidal::endpoints(&p2), &id);
        pm_sources::tidal::full::record(p2.store.as_ref(), &r);
        let mut full = match r {
            Ok(Some(f)) => f,
            Ok(None) => return Ok(None),
            Err(pm_sources::tidal::full::FullError::Mode(m)) => return Err((StatusCode::PRECONDITION_FAILED, m)),
            Err(pm_sources::tidal::full::FullError::Track(m)) => return Err((StatusCode::NOT_FOUND, m)),
        };
        let ext = if full.mime == "audio/flac" { "flac" } else { "m4a" };
        let path = write_track(&p2.streams.dir, &name, ext, &mut full.reader)
            .map_err(|e| (StatusCode::BAD_GATEWAY, format!("TIDAL: {e}")))?;
        tracing::info!(track = %id, bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0), "phone mode: TIDAL full track fetched");
        Ok(Some(path))
    })
    .await;
    match fetched {
        Ok(Ok(Some(path))) => {
            p.streams.trim();
            crate::server::send_file(&path, &headers).await
        }
        Ok(Ok(None)) => err(
            StatusCode::PRECONDITION_FAILED,
            "Sign in to TIDAL first (Settings › Streaming services)",
        ),
        Ok(Err((status, m))) => err(status, &m),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "the TIDAL fetch stopped"),
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
            write_track(&s.dir, &format!("tidal-{i}"), "flac", &mut r).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
            s.trim();
        }
        assert!(s.kept("tidal-4").is_some() && s.kept("tidal-2").is_some());
        assert!(s.kept("tidal-0").is_none() && s.kept("tidal-1").is_none());
        let mut empty: &[u8] = b"";
        assert!(write_track(&s.dir, "tidal-9", "flac", &mut empty).is_err());
        assert!(s.kept("tidal-9").is_none(), "nothing half-written is served");
        // A new start begins empty
        let again = Streams::open(dir.path()).unwrap();
        assert!(again.kept("tidal-4").is_none());
    }
}
