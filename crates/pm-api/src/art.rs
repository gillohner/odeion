//! Cover art for albums whose files carry none: the Cover Art Archive
//! (coverartarchive.org) by release MBID, then release-group MBID, then Apple's
//! iTunes Search API (official, keyless) by album title and album artist, taking
//! a result only when both match. Fetched lazily on the first cover request,
//! stored in the art directory like embedded art, and remembered as a miss for a
//! week so a library of untagged albums does not hammer either service. The
//! iTunes fallback follows Music Assistant's `itunes_artwork` provider
//! (Apache-2.0; see NOTICE).

use crate::state::NodeState;
use pm_library::AlbumArt;
use pm_model::Mbid;
use std::collections::HashSet;
use std::sync::{Mutex, RwLock};

const MISS_TTL_SECS: i64 = 7 * 24 * 3600;

fn itunes_base() -> &'static RwLock<String> {
    static B: std::sync::OnceLock<RwLock<String>> = std::sync::OnceLock::new();
    B.get_or_init(|| RwLock::new("https://itunes.apple.com".into()))
}

/// Point the iTunes lookups somewhere else (tests).
pub fn set_itunes_base(url: &str) {
    *itunes_base().write().expect("itunes base") = url.trim_end_matches('/').to_string();
}

fn client(s: &NodeState) -> reqwest::Client {
    s.http_client()
}

/// Albums whose cover is being fetched, by art directory and album (one node has one art
/// directory; tests run several nodes in one process).
fn in_flight() -> &'static Mutex<HashSet<(std::path::PathBuf, i64)>> {
    static SET: std::sync::OnceLock<Mutex<HashSet<(std::path::PathBuf, i64)>>> = std::sync::OnceLock::new();
    SET.get_or_init(|| Mutex::new(HashSet::new()))
}

fn ext_for(mime: &str) -> &'static str {
    match mime.split(';').next().unwrap_or("").trim() {
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "jpg",
    }
}

/// Try the archive for `album_id`; `Ok(None)` when the album has no MBIDs,
/// the archive has nothing, a recent miss is remembered, or another fetch
/// for the same album is running.
pub(crate) async fn fetch_missing_cover(
    s: &NodeState,
    album_id: i64,
) -> Result<Option<AlbumArt>, pm_library::LibraryError> {
    let (album, art_dir) = {
        let lib = s.library.lock().expect("library poisoned");
        let Some(album) = lib.get_album(album_id)? else {
            return Ok(None);
        };
        let key = format!("art.caa.miss.{album_id}");
        if let Some(at) = lib.get_setting(&key)?.and_then(|v| v.parse::<i64>().ok()) {
            if pm_library::db::now_unix() - at < MISS_TTL_SECS {
                return Ok(None);
            }
        }
        (album, s.art_dir.clone())
    };
    let flight = (art_dir.clone(), album_id);
    if !in_flight().lock().expect("in-flight lock").insert(flight.clone()) {
        return Ok(None);
    }
    let mut result = fetch(
        s,
        album_id,
        album.release_mbid,
        album.release_group_mbid,
        &art_dir,
    )
    .await;
    if result.is_none() {
        if let Some(artist) = album.album_artist.as_deref() {
            result = from_itunes(s, album_id, &album.title, artist, &art_dir).await;
        }
    }
    in_flight().lock().expect("in-flight lock").remove(&flight);
    let lib = s.library.lock().expect("library poisoned");
    match result {
        Some(art) => {
            lib.set_album_art(&art)?;
            Ok(Some(art))
        }
        None => {
            lib.set_setting(
                &format!("art.caa.miss.{album_id}"),
                &pm_library::db::now_unix().to_string(),
            )?;
            Ok(None)
        }
    }
}

async fn fetch(
    s: &NodeState,
    album_id: i64,
    release: Option<Mbid>,
    group: Option<Mbid>,
    art_dir: &std::path::Path,
) -> Option<AlbumArt> {
    let base = s
        .caa_base
        .read()
        .expect("caa base")
        .trim_end_matches('/')
        .to_string();
    let mut urls = Vec::new();
    if let Some(m) = release {
        urls.push(format!("{base}/release/{m}/front-500"));
    }
    if let Some(m) = group {
        urls.push(format!("{base}/release-group/{m}/front-500"));
    }
    let http = client(s);
    for url in urls {
        if let Some(art) = download(&http, &url, album_id, "caa", "coverartarchive", art_dir).await {
            return Some(art);
        }
    }
    None
}

/// Lower case letters and digits only, with bracketed parts ("(Remastered)", "[Deluxe]") left
/// out, so "Discovery (Remastered 2011)" and "Discovery" compare equal.
fn loose(s: &str) -> String {
    let mut depth = 0u32;
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            c if depth == 0 && c.is_alphanumeric() => out.extend(c.to_lowercase()),
            _ => {}
        }
    }
    out
}

/// Apple's iTunes Search API: the first album whose title and artist match ours, at 600 px.
/// Albums by "Various Artists" are skipped: a title alone picks wrong covers too often.
async fn from_itunes(
    s: &NodeState,
    album_id: i64,
    title: &str,
    artist: &str,
    art_dir: &std::path::Path,
) -> Option<AlbumArt> {
    let (want_title, want_artist) = (loose(title), loose(artist));
    if want_title.is_empty() || want_artist.is_empty() || artist.eq_ignore_ascii_case("Various Artists") {
        return None;
    }
    let base = itunes_base().read().expect("itunes base").clone();
    let http = client(s);
    let resp = http
        .get(format!("{base}/search"))
        .query(&[
            ("term", format!("{artist} {title}").as_str()),
            ("media", "music"),
            ("entity", "album"),
            ("limit", "10"),
        ])
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        tracing::debug!(status = %resp.status(), "itunes search");
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    let art = body["results"].as_array()?.iter().find_map(|r| {
        let ok = loose(r["collectionName"].as_str()?) == want_title
            && loose(r["artistName"].as_str()?) == want_artist;
        ok.then(|| {
            r["artworkUrl100"]
                .as_str()
                .map(|u| u.replace("100x100bb", "600x600bb"))
        })
        .flatten()
    })?;
    download(&http, &art, album_id, "itunes", "itunes", art_dir).await
}

/// Fetch one image into the art directory as `<prefix>-<album>.<ext>`.
async fn download(
    http: &reqwest::Client,
    url: &str,
    album_id: i64,
    prefix: &str,
    source: &str,
    art_dir: &std::path::Path,
) -> Option<AlbumArt> {
    let resp = match http.get(url).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(%url, error = %e, "cover download");
            return None;
        }
    };
    if !resp.status().is_success() {
        return None;
    }
    {
        let mime = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/jpeg")
            .split(';')
            .next()
            .unwrap_or("image/jpeg")
            .trim()
            .to_string();
        if !mime.starts_with("image/") {
            return None;
        }
        let bytes = match resp.bytes().await {
            Ok(b) if !b.is_empty() => b,
            _ => return None,
        };
        // What it is by its bytes, whatever the server called it
        let mime = pm_library::image_type(&bytes)?.to_string();
        if tokio::fs::create_dir_all(art_dir).await.is_err() {
            return None;
        }
        let file = art_dir.join(format!("{prefix}-{album_id}.{}", ext_for(&mime)));
        if write_whole(&file, &bytes).await.is_err() {
            return None;
        }
        Some(AlbumArt {
            album_id,
            file,
            mime,
            source: source.into(),
        })
    }
}

/// Write `bytes` to `to` whole or not at all: into a part file of its own (two writers of one
/// picture never share one), then renamed over `to`. A request that opened the old file keeps
/// reading it to its end; one that opens it later gets the new one complete, never a file being
/// written (which would reach the browser short of its Content-Length).
pub(crate) async fn write_whole(to: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut name = to.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}-{n}.part", std::process::id()));
    let part = to.with_file_name(name);
    let written = match tokio::fs::write(&part, bytes).await {
        Ok(()) => tokio::fs::rename(&part, to).await,
        Err(e) => Err(e),
    };
    if written.is_err() {
        let _ = tokio::fs::remove_file(&part).await;
    }
    written
}

/// Remove the part files a node that stopped mid-write left in the art folder (see
/// [`write_whole`]: each is named after its process, so a later run never writes over it). Run
/// at start, before anything fetches a picture.
pub(crate) fn sweep_parts(art_dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(art_dir) else {
        return;
    };
    for e in entries.flatten() {
        let is_part = e.file_name().to_str().is_some_and(|n| n.ends_with(".part"));
        if is_part && e.file_type().is_ok_and(|t| t.is_file()) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Every picture and relayed file the node serves says it is only that: the browser never guesses
/// another type, and nothing in it runs when it is opened on its own (in a tab).
pub(crate) const PASSIVE_CSP: &str = "default-src 'none'; sandbox";

/// A picture as an answer: its type by its first bytes ([`pm_library::image_type`]: JPEG, PNG,
/// WebP, GIF), never a type stored or sent with it. Anything else is not served.
pub(crate) fn picture(bytes: Vec<u8>, cache: &str) -> Result<axum::response::Response, crate::ApiError> {
    use axum::http::header;
    use axum::response::IntoResponse;
    let mime = pm_library::image_type(&bytes).ok_or_else(|| crate::ApiError::not_found("not a picture"))?;
    Ok((
        [
            (header::CONTENT_TYPE, mime.to_string()),
            (header::CACHE_CONTROL, cache.to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            (header::CONTENT_SECURITY_POLICY, PASSIVE_CSP.to_string()),
        ],
        bytes,
    )
        .into_response())
}

/// The parts a picture file is sent in.
const PICTURE_CHUNK: usize = 64 << 10;

/// A picture file as an answer, as [`picture`] answers bytes (its type by its first bytes, the
/// same headers), but sent as it is read: a folder cover can be a scan of tens of MB, and a page
/// of album tiles asks for many at once. With an `ETag` (its length and change time), so a
/// browser that has it gets a 304 instead of the picture again. The error is the file's own
/// (open, read), or `InvalidData` when it is no picture.
pub(crate) async fn picture_file(
    path: &std::path::Path,
    cache: &'static str,
    asked: &axum::http::HeaderMap,
) -> std::io::Result<axum::response::Response> {
    use axum::http::{header, HeaderValue, StatusCode};
    use axum::response::IntoResponse;
    use futures_util::StreamExt;
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path).await?;
    let meta = file.metadata().await?;
    let len = meta.len();
    let changed = meta
        .modified()
        .ok()
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .unwrap_or_default();
    let etag = format!("\"{len:x}-{:x}.{:x}\"", changed.as_secs(), changed.subsec_nanos());
    let etag = HeaderValue::from_str(&etag).map_err(std::io::Error::other)?;
    let has_it = asked
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag));
    if has_it {
        let cache = HeaderValue::from_static(cache);
        return Ok((
            StatusCode::NOT_MODIFIED,
            [(header::ETAG, etag), (header::CACHE_CONTROL, cache)],
        )
            .into_response());
    }
    let mut head = vec![0u8; 16];
    let mut got = 0;
    while got < head.len() {
        match file.read(&mut head[got..]).await? {
            0 => break,
            n => got += n,
        }
    }
    head.truncate(got);
    let mime = pm_library::image_type(&head)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "not a picture"))?;
    // (the length it had when it was opened: a file that grows meanwhile is not read on)
    let rest = len.saturating_sub(got as u64);
    let first = futures_util::stream::iter([Ok::<_, std::io::Error>(bytes::Bytes::from(head))]);
    // (a read error is the last part: the stream ends there)
    let after = futures_util::stream::unfold(
        Some((file.take(rest), vec![0u8; PICTURE_CHUNK])),
        |state| async move {
            let (mut r, mut buf) = state?;
            match r.read(&mut buf).await {
                Ok(0) => None,
                Ok(n) => Some((Ok(bytes::Bytes::copy_from_slice(&buf[..n])), Some((r, buf)))),
                Err(e) => Some((Err(e), None)),
            }
        },
    );
    let mut resp = axum::response::Response::new(axum::body::Body::from_stream(first.chain(after)));
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(got as u64 + rest));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    h.insert(header::ETAG, etag);
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(PASSIVE_CSP),
    );
    Ok(resp)
}

/// The most a relayed picture may be.
const RELAYED_PICTURE_MAX: usize = 16 << 20;

/// A picture another server sent, answered as [`picture`] does: by its bytes, whatever type the
/// server gave it, and none at all when it is no picture (or larger than any cover).
pub(crate) async fn relayed_picture(
    resp: reqwest::Response,
    cache: &str,
) -> Result<axum::response::Response, crate::ApiError> {
    use futures_util::StreamExt;
    let too_big = || crate::ApiError::not_found("that picture is too large");
    if resp
        .content_length()
        .is_some_and(|n| n as usize > RELAYED_PICTURE_MAX)
    {
        return Err(too_big());
    }
    let mut bytes = Vec::new();
    let mut body = resp.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk =
            chunk.map_err(|e| crate::ApiError::new(axum::http::StatusCode::BAD_GATEWAY, e.to_string()))?;
        if bytes.len() + chunk.len() > RELAYED_PICTURE_MAX {
            return Err(too_big());
        }
        bytes.extend_from_slice(&chunk);
    }
    picture(bytes, cache)
}

/// The type to relay a file from another server as: what it said when that is audio, video or a
/// raster picture, else `fallback` (what the node asked it for).
pub(crate) fn safe_media_type(said: Option<&str>, fallback: &str) -> String {
    let said = said
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let ok = said.starts_with("audio/")
        || said.starts_with("video/")
        || matches!(
            said.as_str(),
            "image/jpeg" | "image/png" | "image/webp" | "image/gif"
        );
    if ok {
        said
    } else {
        fallback.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::loose;

    #[test]
    fn loose_titles() {
        assert_eq!(loose("Discovery (Remastered 2011)"), loose("discovery"));
        assert_eq!(loose("Random Access Memories [Deluxe]"), "randomaccessmemories");
        assert_eq!(loose("AC/DC"), "acdc");
        assert_ne!(loose("Homework"), loose("Discovery"));
    }

    /// A cover file goes out as it is read, in parts, not read whole first (a folder cover can be
    /// a scan of tens of MB, and a page of tiles asks for many at once): every byte, its length
    /// and the headers of [`picture`]; a file that is no picture is refused by its bytes.
    #[tokio::test]
    async fn a_picture_file_is_sent_as_it_is_read() {
        use super::{picture_file, PASSIVE_CSP, PICTURE_CHUNK};
        use http_body_util::BodyExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cover.jpg");
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
        bytes.extend((0..3_000_000u32).map(|i| (i % 251) as u8));
        std::fs::write(&path, &bytes).unwrap();
        let resp = picture_file(&path, "private", &axum::http::HeaderMap::new())
            .await
            .unwrap();
        let h = resp.headers().clone();
        assert_eq!(h["content-type"], "image/jpeg");
        assert_eq!(h["content-length"], bytes.len().to_string().as_str());
        assert_eq!(h["cache-control"], "private");
        assert_eq!(h["x-content-type-options"], "nosniff");
        assert_eq!(h["content-security-policy"], PASSIVE_CSP);
        let mut body = resp.into_body();
        let (mut got, mut largest) = (Vec::new(), 0);
        while let Some(frame) = body.frame().await {
            let data = frame.unwrap().into_data().unwrap();
            largest = largest.max(data.len());
            got.extend_from_slice(&data);
        }
        assert!(
            got == bytes,
            "{} of {} bytes, or other bytes",
            got.len(),
            bytes.len()
        );
        assert!(largest <= PICTURE_CHUNK, "a part of {largest} bytes");
        // Asked again with the ETag it gave: 304, no picture; another ETag: the picture
        let etag = h["etag"].clone();
        let mut again = axum::http::HeaderMap::new();
        again.insert("if-none-match", etag.clone());
        let resp = picture_file(&path, "private", &again).await.unwrap();
        assert_eq!(resp.status(), 304);
        assert_eq!(resp.headers()["etag"], etag);
        assert!(resp.into_body().collect().await.unwrap().to_bytes().is_empty());
        again.insert("if-none-match", "\"other\", W/\"x\"".parse().unwrap());
        let resp = picture_file(&path, "private", &again).await.unwrap();
        assert_eq!(resp.status(), 200);
        // The file changed: another ETag, so the old one gets the picture
        bytes.extend_from_slice(b"more");
        std::fs::write(&path, &bytes).unwrap();
        again.insert("if-none-match", etag.clone());
        let resp = picture_file(&path, "private", &again).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_ne!(resp.headers()["etag"], etag);
        // A picture shorter than the bytes its type is read from
        let tiny = dir.path().join("tiny.gif");
        std::fs::write(&tiny, b"GIF89a").unwrap();
        let resp = picture_file(&tiny, "private", &axum::http::HeaderMap::new())
            .await
            .unwrap();
        assert_eq!(resp.headers()["content-type"], "image/gif");
        let got = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&got[..], b"GIF89a");
        // No picture, whatever its name; no file
        for (name, text) in [
            ("page.jpg", &b"<svg onload=alert(1)>"[..]),
            ("empty.png", &b""[..]),
        ] {
            std::fs::write(dir.path().join(name), text).unwrap();
            let e = picture_file(&dir.path().join(name), "private", &axum::http::HeaderMap::new())
                .await
                .unwrap_err();
            assert_eq!(e.kind(), std::io::ErrorKind::InvalidData, "{name}");
        }
        let (none, no_headers) = (dir.path().join("none.jpg"), axum::http::HeaderMap::new());
        assert!(picture_file(&none, "private", &no_headers).await.is_err());
    }

    #[tokio::test]
    async fn a_picture_is_replaced_whole_while_an_old_reader_reads_on() {
        use std::io::Read;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("caa-7.jpg");
        let old = vec![1u8; 64 * 1024];
        std::fs::write(&path, &old).unwrap();
        // A request already sending the old picture
        let mut reader = std::fs::File::open(&path).unwrap();
        // Two requests write the same missing picture at once
        let (a, b) = (vec![2u8; 200 * 1024], vec![3u8; 100 * 1024]);
        let (ra, rb) = tokio::join!(super::write_whole(&path, &a), super::write_whole(&path, &b));
        ra.unwrap();
        rb.unwrap();
        let mut seen = Vec::new();
        reader.read_to_end(&mut seen).unwrap();
        assert_eq!(seen, old, "the old reader is not cut short or mixed");
        let now = std::fs::read(&path).unwrap();
        assert!(now == a || now == b, "one whole picture, {} bytes", now.len());
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            names,
            vec![std::ffi::OsString::from("caa-7.jpg")],
            "no part file left"
        );
        // A write that cannot happen leaves nothing behind either: no folder to write in, or a
        // part file written whose rename fails (a folder stands at the picture's name)
        let gone = dir.path().join("no-such-dir").join("x.jpg");
        assert!(super::write_whole(&gone, &a).await.is_err());
        let blocked = dir.path().join("caa-8.jpg");
        std::fs::create_dir(&blocked).unwrap();
        std::fs::write(blocked.join("inside"), b"x").unwrap();
        assert!(super::write_whole(&blocked, &a).await.is_err());
        let mut names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["caa-7.jpg", "caa-8.jpg"],
            "no part file left after a failed rename"
        );
        // A node that stopped between write and rename: its part files go at the next start
        std::fs::write(dir.path().join("caa-9.jpg.4242-0.part"), b"half").unwrap();
        std::fs::write(dir.path().join("artist-3.part"), b"half").unwrap();
        super::sweep_parts(dir.path());
        let mut names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["caa-7.jpg", "caa-8.jpg"],
            "part files swept, pictures kept"
        );
        assert!(dir.path().join("caa-8.jpg/inside").exists());
    }
}
