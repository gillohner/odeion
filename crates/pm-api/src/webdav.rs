//! WebDAV shares as library roots (STEWARD-COMPAT-HA-MA.md §6.4): a Nextcloud, ownCloud or NAS
//! folder scanned into the library like a local folder and played where it is.
//!
//! - **Keys:** a song on a share is `webdav://<share id>/<path inside the share>` in the
//!   library's files table, so missing files and removal work by prefix as for folders.
//! - **Scanning:** the share is listed with `PROPFIND` (one level at a time), and each new or
//!   changed audio file is read with HTTP range requests, so only its tags are fetched
//!   ([`RangeReader`], `pm_library::Scanner::scan_remote`). Shares are rescanned every 6 hours.
//! - **Playing:** zones stream the file; browsers get it relayed. The password stays in the
//!   node (its settings), never in an answer or a log.

use crate::state::NodeState;
use crate::{ApiError, ApiResult};
use axum::extract::{Path as AxPath, State};
use axum::http::StatusCode;
use axum::Json;
use pm_model::text::percent_decode;
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

const SETTING: &str = "webdav.shares";
pub const SCHEME: &str = "webdav://";
const RESCAN_EVERY: Duration = Duration::from_secs(6 * 3600);
/// Listing stops here (a share that big wants a local mount).
const MAX_ENTRIES: usize = 200_000;
/// What a range request fetches at least.
const CHUNK: u64 = 256 * 1024;

/// A share. `Debug` never shows the password, nor credentials in the address.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Share {
    pub id: String,
    pub name: String,
    /// The folder's URL (`https://cloud.example.org/remote.php/dav/files/me/Music/`).
    pub url: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
}

impl std::fmt::Debug for Share {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Share")
            .field("id", &self.id)
            .field("url", &shown_url(&self.url))
            .field("user", &self.user)
            .field("password", &"<set>")
            .finish()
    }
}

impl Share {
    /// Moves a user name and password typed into the address (`http://me:secret@nas/dav/`) to
    /// `user` and `password` and takes them out of the address. With the user field empty, the
    /// address gives the user and its password (a password field stays only when the address has
    /// none), as playback sent them before; with the same user in it, an empty password field
    /// gets the address's. When the fields name another user, they win and the address loses its
    /// credentials all the same. True when the address had any.
    fn take_url_credentials(&mut self) -> bool {
        let Some((url, user, password)) = split_credentials(&self.url) else {
            return false;
        };
        self.url = url;
        if self.user.is_empty() {
            self.user = user;
            if !password.is_empty() {
                self.password = password;
            }
        } else if self.user == user && self.password.is_empty() {
            self.password = password;
        }
        true
    }

    fn base(&self) -> Result<url::Url, String> {
        let mut u = url::Url::parse(&self.url).map_err(|e| e.to_string())?;
        if !u.path().ends_with('/') {
            let p = format!("{}/", u.path());
            u.set_path(&p);
        }
        Ok(u)
    }

    /// The library root of this share.
    pub fn root(&self) -> PathBuf {
        PathBuf::from(format!("{SCHEME}{}", self.id))
    }

    /// The URL of `rel` (a path inside the share, `/`-separated, not encoded).
    pub fn file_url(&self, rel: &str) -> Result<url::Url, String> {
        let mut u = self.base()?;
        {
            let mut segs = u
                .path_segments_mut()
                .map_err(|_| "not a folder URL".to_string())?;
            segs.pop_if_empty();
            for s in rel.split('/').filter(|s| !s.is_empty()) {
                if s == ".." || s == "." {
                    return Err("bad path".into());
                }
                segs.push(s);
            }
        }
        Ok(u)
    }

    /// A password alone (a token, `http://:token@host/` as typed) goes too, with an empty user.
    fn has_credentials(&self) -> bool {
        !self.user.is_empty() || !self.password.is_empty()
    }

    fn auth(&self, rb: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
        if self.has_credentials() {
            rb.basic_auth(&self.user, Some(&self.password))
        } else {
            rb
        }
    }
}

/// `url` without the user name and password typed into it, and those (decoded); `None` when it
/// has none or is not a URL.
fn split_credentials(url: &str) -> Option<(String, String, String)> {
    let mut u = url::Url::parse(url).ok()?;
    if u.username().is_empty() && u.password().is_none() {
        return None;
    }
    let user = percent_decode(u.username());
    let password = percent_decode(u.password().unwrap_or(""));
    u.set_username("").ok()?;
    u.set_password(None).ok()?;
    Some((u.to_string(), user, password))
}

/// A share's address as it may be shown: never with credentials, also when it is not a URL.
fn shown_url(url: &str) -> String {
    if let Some((u, ..)) = split_credentials(url) {
        return u;
    }
    if url::Url::parse(url).is_ok() {
        return url.to_string();
    }
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    match rest[..host_end].rfind('@') {
        Some(at) if scheme.is_empty() => rest[at + 1..].to_string(),
        Some(at) => format!("{scheme}://{}", &rest[at + 1..]),
        None => url.to_string(),
    }
}

pub type Shared = Arc<RwLock<Vec<Share>>>;

/// The shares in the settings. One saved with credentials in its address (before they were
/// taken out on adding) has them moved to its fields, and is saved again that way.
pub fn load(lib: &pm_library::Library) -> Vec<Share> {
    let mut shares: Vec<Share> = lib
        .get_setting(SETTING)
        .ok()
        .flatten()
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
    let mut moved = false;
    for sh in &mut shares {
        moved |= sh.take_url_credentials();
    }
    if moved {
        if let Err(e) = lib.set_setting(SETTING, &serde_json::to_string(&shares).expect("serializes")) {
            tracing::warn!(
                "WebDAV shares: could not save them without the credentials in their addresses: {e}"
            );
        }
    }
    shares
}

fn store(s: &NodeState, shares: &[Share]) -> Result<(), ApiError> {
    s.library
        .lock()
        .expect("library poisoned")
        .set_setting(SETTING, &serde_json::to_string(shares).expect("serializes"))?;
    *s.webdav.write().expect("webdav") = shares.to_vec();
    Ok(())
}

/// A library key's share and path inside it.
pub fn split_key(key: &Path) -> Option<(String, String)> {
    let k = key.to_str()?.strip_prefix(SCHEME)?;
    let (id, rel) = k.split_once('/')?;
    Some((id.to_string(), rel.to_string()))
}

pub fn is_key(key: &Path) -> bool {
    key.to_str().is_some_and(|k| k.starts_with(SCHEME))
}

fn find(shared: &Shared, id: &str) -> Option<Share> {
    shared.read().ok()?.iter().find(|s| s.id == id).cloned()
}

// ---- listing ----

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DavEntry {
    /// The href's path, decoded.
    pub path: String,
    pub dir: bool,
    pub size: u64,
    pub mtime: i64,
}

fn unref(name: &str) -> String {
    match name {
        "amp" => "&".into(),
        "lt" => "<".into(),
        "gt" => ">".into(),
        "quot" => "\"".into(),
        "apos" => "'".into(),
        _ => String::new(),
    }
}

/// A `PROPFIND` multistatus answer as entries (namespaces ignored: `d:href`, `D:href`, `href`).
pub(crate) fn parse_multistatus(xml: &str) -> Result<Vec<DavEntry>, String> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    let mut out = Vec::new();
    let mut cur: Option<DavEntry> = None;
    let mut text = String::new();
    let local = |n: &str| -> String { n.rsplit(':').next().unwrap_or("").to_ascii_lowercase() };
    loop {
        match r.read_event().map_err(|e| format!("not a WebDAV answer: {e}"))? {
            Event::Start(e) | Event::Empty(e) => {
                let name = local(e.name().as_ref());
                match name.as_str() {
                    "response" => {
                        cur = Some(DavEntry {
                            path: String::new(),
                            dir: false,
                            size: 0,
                            mtime: 0,
                        })
                    }
                    "collection" => {
                        if let Some(c) = cur.as_mut() {
                            c.dir = true;
                        }
                    }
                    _ => {}
                }
                text.clear();
            }
            Event::Text(t) => text.push_str(&t.xml_content(quick_xml::XmlVersion::Implicit1_0)),
            Event::GeneralRef(g) => text.push_str(&unref(g.as_ref())),
            Event::End(e) => {
                let name = local(e.name().as_ref());
                let v = std::mem::take(&mut text).trim().to_string();
                if let Some(c) = cur.as_mut() {
                    match name.as_str() {
                        "href" => {
                            // An absolute URL or a path: keep the path
                            let p = url::Url::parse(&v).map(|u| u.path().to_string()).unwrap_or(v);
                            c.path = percent_decode(&p);
                        }
                        "getcontentlength" => c.size = v.parse().unwrap_or(0),
                        "getlastmodified" => {
                            c.mtime = time::OffsetDateTime::parse(
                                &v,
                                &time::format_description::well_known::Rfc2822,
                            )
                            .map(|d| d.unix_timestamp())
                            .unwrap_or(0)
                        }
                        "response" => {
                            if let Some(c) = cur.take() {
                                out.push(c);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// The most of one folder's PROPFIND answer that is read.
const LISTING_MAX: u64 = 32 * 1024 * 1024;

/// A body read whole, but `max` bytes at most: refused when it says or turns out to be bigger.
fn read_capped(resp: reqwest::blocking::Response, max: u64) -> Result<Vec<u8>, String> {
    if resp.content_length().is_some_and(|n| n > max) {
        return Err(format!("the share sent more than the {max} bytes asked for"));
    }
    let mut buf = Vec::new();
    resp.take(max + 1)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    if buf.len() as u64 > max {
        return Err(format!("the share sent more than the {max} bytes asked for"));
    }
    Ok(buf)
}

const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/></d:prop></d:propfind>"#;

fn propfind(share: &Share, url: &url::Url) -> Result<Vec<DavEntry>, String> {
    let m = reqwest::Method::from_bytes(b"PROPFIND").expect("method");
    let resp = share
        .auth(crate::sources::http_client().request(m, url.clone()))
        .header("Depth", "1")
        .header(reqwest::header::CONTENT_TYPE, "application/xml")
        .timeout(Duration::from_secs(60))
        .body(PROPFIND_BODY)
        .send()
        .map_err(|e| e.without_url().to_string())?;
    match resp.status().as_u16() {
        207 | 200 => {}
        401 | 403 => return Err("the share refused this user name or password".into()),
        404 => return Err("the share has no such folder".into()),
        st => return Err(format!("the share answered {st}")),
    }
    let body = read_capped(resp, LISTING_MAX)?;
    parse_multistatus(&String::from_utf8_lossy(&body))
}

/// Every file under the share, as library entries (key, size, time).
pub(crate) fn list_files(share: &Share) -> Result<Vec<pm_library::scan::RemoteEntry>, String> {
    let base = share.base()?;
    let base_path = percent_decode(base.path());
    let mut out = Vec::new();
    let mut queue = vec![base.clone()];
    let mut seen = std::collections::HashSet::new();
    while let Some(dir) = queue.pop() {
        if !seen.insert(dir.path().to_string()) {
            continue;
        }
        for e in propfind(share, &dir)? {
            let Some(rel) = e.path.strip_prefix(&base_path) else {
                continue;
            };
            let rel = rel.trim_matches('/').to_string();
            if e.dir {
                // The folder itself is listed too
                if !rel.is_empty()
                    && share
                        .file_url(&rel)
                        .map(|u| u.path() != dir.path())
                        .unwrap_or(false)
                {
                    let mut u = share.file_url(&rel)?;
                    let p = format!("{}/", u.path().trim_end_matches('/'));
                    u.set_path(&p);
                    queue.push(u);
                }
            } else if !rel.is_empty() {
                out.push(pm_library::scan::RemoteEntry {
                    path: share.root().join(&rel),
                    size: e.size,
                    mtime: e.mtime,
                });
                if out.len() >= MAX_ENTRIES {
                    return Ok(out);
                }
            }
        }
    }
    Ok(out)
}

// ---- reading ----

/// A share's file read with HTTP range requests, for the scanner (tags only).
pub struct RangeReader {
    share: Share,
    url: url::Url,
    len: u64,
    pos: u64,
    buf: Vec<u8>,
    buf_at: u64,
}

impl RangeReader {
    pub fn new(share: Share, url: url::Url, len: u64) -> Self {
        Self {
            share,
            url,
            len,
            pos: 0,
            buf: Vec::new(),
            buf_at: 0,
        }
    }

    fn fill(&mut self, want: usize) -> std::io::Result<()> {
        let end = (self.pos + (want as u64).max(CHUNK))
            .min(self.len)
            .saturating_sub(1);
        let resp = self
            .share
            .auth(crate::sources::http_client().get(self.url.clone()))
            .header(reqwest::header::RANGE, format!("bytes={}-{end}", self.pos))
            .timeout(Duration::from_secs(60))
            .send()
            .map_err(|e| std::io::Error::other(e.without_url().to_string()))?;
        let st = resp.status().as_u16();
        // A range answer is the range asked for; a whole one is the file as listed
        let max = if st == 206 { end + 1 - self.pos } else { self.len };
        let bytes = read_capped(resp, max).map_err(std::io::Error::other)?;
        match st {
            206 => {
                self.buf = bytes;
                self.buf_at = self.pos;
            }
            // No ranges: the whole file came; keep it
            200 => {
                self.buf = bytes;
                self.buf_at = 0;
            }
            st => return Err(std::io::Error::other(format!("the share answered {st}"))),
        }
        Ok(())
    }
}

impl Read for RangeReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.len || out.is_empty() {
            return Ok(0);
        }
        let inside = self.pos >= self.buf_at && self.pos < self.buf_at + self.buf.len() as u64;
        if !inside {
            self.fill(out.len())?;
        }
        let off = (self.pos - self.buf_at) as usize;
        if off >= self.buf.len() {
            return Ok(0);
        }
        let n = out.len().min(self.buf.len() - off);
        out[..n].copy_from_slice(&self.buf[off..off + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for RangeReader {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let p = match to {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::End(n) => self.len as i64 + n,
            SeekFrom::Current(n) => self.pos as i64 + n,
        };
        if p < 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek before the start",
            ));
        }
        self.pos = p as u64;
        Ok(self.pos)
    }
}

impl pm_library::tags::RemoteRead for RangeReader {
    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

/// Scans one share into the library (blocking).
fn scan_share(s: &NodeState, share: &Share) -> Result<pm_library::scan::ScanReport, String> {
    let entries = list_files(share)?;
    // Listed without it; held while files are read and written (no match correction in between)
    let _gate = s.scan_gate();
    let lib = pm_library::Library::open(&s.db_path).map_err(|e| e.to_string())?;
    let scanner = pm_library::Scanner::new(&lib).with_art_dir(&s.art_dir);
    let open = |e: &pm_library::scan::RemoteEntry| -> std::io::Result<Box<dyn pm_library::tags::RemoteRead>> {
        let (_, rel) = split_key(&e.path).ok_or_else(|| std::io::Error::other("bad key"))?;
        let url = share.file_url(&rel).map_err(std::io::Error::other)?;
        Ok(Box::new(RangeReader::new(share.clone(), url, e.size)))
    };
    scanner
        .scan_remote(&share.root(), entries, &open)
        .map_err(|e| e.to_string())
}

/// Opens a library file on a share for a zone: streamed, the credentials kept in the node.
pub(crate) fn open_blocking(
    shared: &Shared,
    key: &Path,
) -> Result<Box<dyn pm_zones::FrameSource>, pm_zones::SourceError> {
    let fail = |reason: String| pm_zones::SourceError::Open {
        uri: key.to_string_lossy().into_owned(),
        reason,
    };
    let (id, rel) = split_key(key).ok_or_else(|| fail("not a WebDAV file".into()))?;
    let mut share = find(shared, &id).ok_or_else(|| fail("that WebDAV share was removed".into()))?;
    // The credentials go as a header, never in the URL: errors name the URL (a decoder's too),
    // and zone state and events carry them to listeners. A request with headers never follows
    // a playlist to another host. Credentials still in the share's address (adding and loading
    // take them out) move to the header as well.
    share.take_url_credentials();
    let url = share.file_url(&rel).map_err(fail)?;
    let mut headers = Vec::new();
    if share.has_credentials() {
        let basic = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("{}:{}", share.user, share.password),
        );
        headers.push(("authorization".to_string(), format!("Basic {basic}")));
    }
    crate::sources::open_http_stream_with(url.as_str(), &headers, None).map_err(|e| match e {
        pm_zones::SourceError::Open { reason, .. } => fail(reason),
        other => other,
    })
}

/// A library file on a share or a drive, as it answers: the caller reads the body in chunks,
/// never whole (the download cache, `downloads::local_original`). A local path is not one.
pub(crate) async fn fetch_library_file(s: &NodeState, path: &Path) -> std::io::Result<reqwest::Response> {
    if crate::drives::is_key(path) {
        return crate::drives::fetch_file(s, path).await;
    }
    if !is_key(path) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "not on this machine, a share or a drive",
        ));
    }
    let (id, rel) = split_key(path).ok_or_else(|| std::io::Error::other("bad key"))?;
    let share = find(&s.webdav, &id).ok_or_else(|| std::io::Error::other("that WebDAV share was removed"))?;
    let url = share.file_url(&rel).map_err(std::io::Error::other)?;
    let mut rb = s.http_client().get(url);
    if share.has_credentials() {
        rb = rb.basic_auth(&share.user, Some(&share.password));
    }
    let resp = crate::downloads::send_for_cache(rb).await?;
    if !resp.status().is_success() {
        return Err(std::io::Error::other(format!(
            "the share answered {}",
            resp.status()
        )));
    }
    Ok(resp)
}

// ---- routes ----

/// A share as Settings shows it (never the password, nor credentials in the address).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareView {
    pub id: String,
    pub name: String,
    pub url: String,
    pub user: String,
    pub tracks: u64,
    pub bytes: u64,
}

fn view(s: &NodeState, sh: &Share) -> ShareView {
    let (tracks, bytes) = s
        .library
        .lock()
        .expect("library poisoned")
        .folder_totals(&sh.root())
        .unwrap_or((0, 0));
    ShareView {
        id: sh.id.clone(),
        name: sh.name.clone(),
        url: shown_url(&sh.url),
        user: sh.user.clone(),
        tracks,
        bytes,
    }
}

/// `GET /v1/library/webdav`.
pub(crate) async fn list(State(s): State<Arc<NodeState>>) -> ApiResult<Vec<ShareView>> {
    let shares = s.webdav.read().expect("webdav").clone();
    Ok(Json(shares.iter().map(|sh| view(&s, sh)).collect()))
}

#[derive(Clone, Deserialize)]
pub struct NewShare {
    pub url: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
}

fn start_scan(s: &Arc<NodeState>, share: Share) {
    let s2 = s.clone();
    // Counted before it is spawned: an automatic run or a correction never misses it
    let scan = s.side_scan();
    tokio::task::spawn_blocking(move || {
        let scanned = scan_share(&s2, &share);
        drop(scan);
        match scanned {
            Ok(r) => {
                tracing::info!(share = %share.name, added = r.files_added, updated = r.files_updated, removed = r.files_removed, errors = r.errors.len(), "WebDAV scan done");
                s2.notify_library();
                crate::identify::after_scan(&s2, r.files_added + r.files_updated);
            }
            Err(e) => {
                tracing::warn!(share = %share.name, "WebDAV scan failed: {e}");
                // Songs the settling left while it held the gate are settled still
                crate::identify::after_scan(&s2, 0);
            }
        }
    });
}

/// `POST /v1/library/webdav {url, name?, user?, password?}`: checks the folder, keeps the share
/// and scans it.
pub(crate) async fn add(State(s): State<Arc<NodeState>>, Json(b): Json<NewShare>) -> ApiResult<ShareView> {
    let u = url::Url::parse(b.url.trim())
        .map_err(|_| ApiError::bad_request("the share's address must be an http(s) URL"))?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err(ApiError::bad_request(
            "the share's address must be an http(s) URL",
        ));
    }
    let mut share = Share {
        id: crate::tidal::random_urlsafe(6)
            .to_lowercase()
            .replace(['-', '_'], "x"),
        name: b
            .name
            .map(|n| n.trim().chars().take(80).collect::<String>())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| u.host_str().unwrap_or("WebDAV").to_string()),
        url: u.to_string(),
        user: b.user.trim().to_string(),
        password: b.password,
    };
    // Kept in the fields, never in the address: Settings shows the address
    share.take_url_credentials();
    let probe = share.clone();
    tokio::task::spawn_blocking(move || probe.base().and_then(|base| propfind(&probe, &base)))
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, format!("WebDAV: {e}")))?;
    let mut shares = s.webdav.read().expect("webdav").clone();
    if shares.iter().any(|x| x.url == share.url && x.user == share.user) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "that share is in the library already",
        ));
    }
    shares.push(share.clone());
    store(&s, &shares)?;
    start_scan(&s, share.clone());
    Ok(Json(view(&s, &share)))
}

/// `POST /v1/library/webdav/{id}/scan`.
pub(crate) async fn rescan(
    State(s): State<Arc<NodeState>>,
    AxPath(id): AxPath<String>,
) -> ApiResult<ShareView> {
    let share = find(&s.webdav, &id).ok_or_else(|| ApiError::not_found("no such share"))?;
    start_scan(&s, share.clone());
    Ok(Json(view(&s, &share)))
}

/// `DELETE /v1/library/webdav/{id}`: forgets the share and its songs in the library.
pub(crate) async fn remove(
    State(s): State<Arc<NodeState>>,
    AxPath(id): AxPath<String>,
) -> ApiResult<serde_json::Value> {
    let mut shares = s.webdav.read().expect("webdav").clone();
    let Some(i) = shares.iter().position(|x| x.id == id) else {
        return Err(ApiError::not_found("no such share"));
    };
    let share = shares.remove(i);
    store(&s, &shares)?;
    let (files, tracks) = s
        .library
        .lock()
        .expect("library poisoned")
        .remove_folder(&share.root())?;
    s.notify_library();
    Ok(Json(
        serde_json::json!({"removed": id, "files": files, "tracks": tracks}),
    ))
}

/// Rescans every share now and then (a share has no change notifications).
pub fn spawn_rescans(s: &Arc<NodeState>) {
    let weak = Arc::downgrade(s);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(RESCAN_EVERY);
        tick.tick().await;
        loop {
            tick.tick().await;
            let Some(s) = weak.upgrade() else { return };
            for sh in s.webdav.read().expect("webdav").clone() {
                start_scan(&s, sh);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_share_answer_is_read_only_up_to_what_was_asked() {
        use std::io::Write;
        // Each connection: 1000 bytes, with its length or without
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/f", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for (i, c) in l.incoming().flatten().enumerate() {
                let mut c = c;
                let mut req = [0u8; 2048];
                let _ = c.read(&mut req);
                let len = if i % 2 == 0 {
                    "Content-Length: 1000\r\n"
                } else {
                    ""
                };
                let head = format!("HTTP/1.1 200 OK\r\n{len}Connection: close\r\n\r\n");
                let _ = c.write_all(head.as_bytes());
                let _ = c.write_all(&[b'x'; 1000]);
            }
        });
        let client = reqwest::blocking::Client::new();
        let get = || client.get(&url).send().unwrap();
        // Says it is bigger: refused; is bigger: refused; fits: read
        assert!(read_capped(get(), 999).is_err());
        assert!(read_capped(get(), 999).is_err());
        assert_eq!(read_capped(get(), 1000).unwrap().len(), 1000);
        assert_eq!(read_capped(get(), 1000).unwrap().len(), 1000);
    }

    #[test]
    fn multistatus_and_keys() {
        let xml = r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:">
          <d:response><d:href>/dav/Music/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>
          <d:response><d:href>/dav/Music/Queen%20%26%20Co/01%20Song.flac</d:href><d:propstat><d:prop><d:resourcetype/>
            <d:getcontentlength>47782</d:getcontentlength><d:getlastmodified>Wed, 22 Oct 2026 10:00:00 GMT</d:getlastmodified></d:prop></d:propstat></d:response>
        </d:multistatus>"#;
        let e = parse_multistatus(xml).unwrap();
        assert_eq!(e.len(), 2);
        assert!(e[0].dir);
        assert_eq!(e[1].path, "/dav/Music/Queen & Co/01 Song.flac");
        assert_eq!((e[1].size, e[1].mtime > 1_790_000_000), (47782, true));
        let sh = Share {
            id: "ab12cd".into(),
            name: "NAS".into(),
            url: "https://nas.local/dav/Music".into(),
            user: "me".into(),
            password: ["not", "shown"].join("-"),
        };
        assert_eq!(
            sh.file_url("Queen & Co/01 Song.flac").unwrap().as_str(),
            "https://nas.local/dav/Music/Queen%20&%20Co/01%20Song.flac"
        );
        assert!(sh.file_url("../etc/passwd").is_err());
        let key = sh.root().join("Queen & Co/01 Song.flac");
        assert_eq!(
            split_key(&key),
            Some(("ab12cd".into(), "Queen & Co/01 Song.flac".into()))
        );
        assert!(!format!("{sh:?}").contains("not-shown"));
    }

    /// A share's password never shows in what a failed playback says (the zone's state and its
    /// events carry that to listeners): not as typed, not percent-encoded. It reaches the share
    /// as a header.
    #[test]
    fn a_failed_playback_never_shows_the_share_password() {
        use std::io::Write;
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/dav/Music/", l.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            for c in l.incoming().flatten() {
                let mut c = c;
                let mut req = [0u8; 4096];
                let n = c.read(&mut req).unwrap_or(0);
                let _ = tx.send(String::from_utf8_lossy(&req[..n]).into_owned());
                // Audio by its type, nothing a decoder reads
                let body = [0u8; 2048];
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: audio/flac\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = c.write_all(head.as_bytes());
                let _ = c.write_all(&body);
            }
        });
        let password = "p@ss word:/x";
        let encoded: String = url::form_urlencoded::byte_serialize(password.as_bytes()).collect();
        let basic = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("me:{password}"),
        );
        // In their own fields, and typed into the share's address
        let in_address = base.replacen("http://", "http://me:p%40ss%20word%3A%2Fx@", 1);
        for (url, user, pass) in [(base.clone(), "me", password), (in_address, "", "")] {
            let share = Share {
                id: "ab12cd".into(),
                name: "NAS".into(),
                url,
                user: user.into(),
                password: pass.into(),
            };
            let key = share.root().join("Queen/01 Song.flac");
            let shared: Shared = Arc::new(RwLock::new(vec![share]));
            let err = match open_blocking(&shared, &key) {
                Ok(_) => panic!("zeros decoded"),
                Err(e) => e,
            };
            let said = format!("{err} {err:?}");
            for secret in [
                password,
                "p%40ss%20word%3A%2Fx",
                "p%40ss",
                encoded.as_str(),
                "p@ss",
            ] {
                assert!(!said.contains(secret), "{secret:?} in {said}");
            }
            assert!(!said.contains("me:"), "{said}");
            let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(
                req.to_ascii_lowercase()
                    .contains(&format!("authorization: basic {}", basic.to_ascii_lowercase())),
                "{req}"
            );
        }
        // A password alone in the address (a token) still goes, with an empty user name
        while rx.try_recv().is_ok() {}
        let token_only = base.replacen("http://", "http://:t%40ken@", 1);
        let share = Share {
            id: "ab12cd".into(),
            name: "NAS".into(),
            url: token_only,
            user: String::new(),
            password: String::new(),
        };
        let key = share.root().join("Queen/01 Song.flac");
        let shared: Shared = Arc::new(RwLock::new(vec![share]));
        let err = match open_blocking(&shared, &key) {
            Ok(_) => panic!("zeros decoded"),
            Err(e) => e,
        };
        let said = format!("{err} {err:?}");
        assert!(!said.contains("t%40ken") && !said.contains("t@ken"), "{said}");
        let basic = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, ":t@ken");
        let mut reqs = vec![rx.recv_timeout(Duration::from_secs(5)).unwrap()];
        reqs.extend(rx.try_iter());
        let want = format!("authorization: basic {}", basic.to_ascii_lowercase());
        assert!(
            reqs.iter().all(|r| r.to_ascii_lowercase().contains(&want)),
            "{reqs:?}"
        );
    }

    /// Reads one whole request (head and body) from a test share's connection.
    fn read_request(c: &mut std::net::TcpStream) -> String {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = c.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            let text = String::from_utf8_lossy(&buf).into_owned();
            if let Some(end) = text.find("\r\n\r\n") {
                let len = text[..end]
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if buf.len() >= end + 4 + len {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&buf).into_owned()
    }

    fn test_node(dir: &Path) -> Arc<NodeState> {
        let db = dir.join("library.db");
        NodeState::new(
            crate::NodeSettings {
                name: "webdav-test".into(),
                version: "0".into(),
                db_path: db.clone(),
                scan_folders: vec![],
                art_dir: dir.join("art"),
                mirror_dir: dir.join("homeserver"),
                pipeline: pm_audio::PipelineConfig::default(),
                token: Some("owner-fixture-token-webdav".into()),
                subsonic_user: "pubky".into(),
                pubky: Default::default(),
                public_url: "http://127.0.0.1:7788".into(),
                upnp: false,
                cast: false,
                slimproto_port: 0,
                session_secret_path: None,
                tidal: Default::default(),
                spotify: Default::default(),
                acoustid_key: String::new(),
                jamendo: Default::default(),
            },
            pm_library::Library::open(&db).unwrap(),
            pm_zones::OutputRegistry::new(),
        )
    }

    /// A share that answers a listing with its folder and anything else with audio (that no
    /// decoder reads), and hands over each request it gets.
    fn test_share() -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::Write;
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let host = l.local_addr().unwrap().to_string();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            for c in l.incoming().flatten() {
                let mut c = c;
                let req = read_request(&mut c);
                let listing = req.starts_with("PROPFIND");
                let _ = tx.send(req);
                let (head, body) = if listing {
                    let xml = r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:"><d:response><d:href>/dav/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response></d:multistatus>"#;
                    (
                        "207 Multi-Status\r\nContent-Type: application/xml",
                        xml.as_bytes().to_vec(),
                    )
                } else {
                    ("200 OK\r\nContent-Type: audio/flac", vec![0u8; 2048])
                };
                let head = format!(
                    "HTTP/1.1 {head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = c.write_all(head.as_bytes());
                let _ = c.write_all(&body);
            }
        });
        (host, rx)
    }

    /// Credentials typed into a share's address are kept in its fields: the saved share, the list
    /// Settings shows and the share's `Debug` have an address without them, and the share still
    /// gets them (checking the folder, playing). A password alone is kept with an empty user.
    #[tokio::test(flavor = "multi_thread")]
    async fn credentials_in_an_address_are_kept_out_of_it() {
        let dir = tempfile::tempdir().unwrap();
        let s = test_node(dir.path());
        let basic = |creds: &str| {
            format!(
                "authorization: basic {}",
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, creds)
                    .to_ascii_lowercase()
            )
        };
        for (typed, user, password, secrets) in [
            ("http://me:p%40w@{host}/dav/", "me", "p@w", ["p%40w", "p@w"]),
            ("http://:tok@{host}/dav/", "", "tok", [":tok@", "tok@"]),
        ] {
            // A share of its own for each, so a scan of the one before is not counted
            let (host, rx) = test_share();
            let typed = typed.replace("{host}", &host);
            let clean = format!("http://{host}/dav/");
            let new = NewShare {
                url: typed.clone(),
                name: None,
                user: String::new(),
                password: String::new(),
            };
            let added = match add(State(s.clone()), Json(new)).await {
                Ok(j) => j.0,
                Err(_) => panic!("{typed} not added"),
            };
            assert_eq!((added.url.as_str(), added.user.as_str()), (clean.as_str(), user));
            let want = basic(&format!("{user}:{password}"));
            // The folder was checked with the credentials
            let probe = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(probe.starts_with("PROPFIND /dav/ "), "{probe}");
            assert!(probe.to_ascii_lowercase().contains(&want), "{probe}");

            // As saved
            let saved: Vec<Share> =
                serde_json::from_str(&s.library.lock().unwrap().get_setting(SETTING).unwrap().unwrap())
                    .unwrap();
            let sh = saved.iter().find(|x| x.id == added.id).unwrap().clone();
            assert_eq!(
                (sh.url.as_str(), sh.user.as_str(), sh.password.as_str()),
                (clean.as_str(), user, password)
            );
            assert_eq!(
                s.webdav.read().unwrap().iter().find(|x| x.id == added.id),
                Some(&sh)
            );
            // As listed, and as a log shows it
            let listed = match list(State(s.clone())).await {
                Ok(j) => serde_json::to_string(&j.0).unwrap(),
                Err(_) => panic!("not listed"),
            };
            let debug = format!("{sh:?}");
            for said in [&listed, &debug] {
                for secret in secrets {
                    assert!(!said.contains(secret), "{secret:?} in {said}");
                }
                assert!(!said.contains('@'), "{said}");
            }

            // Played: still sent, as a header
            let (shared, key) = (s.webdav.clone(), sh.root().join("Queen/01 Song.flac"));
            let _ = tokio::task::spawn_blocking(move || open_blocking(&shared, &key).is_ok())
                .await
                .unwrap();
            let played = std::iter::from_fn(|| rx.recv_timeout(Duration::from_secs(5)).ok())
                .find(|r| r.starts_with("GET "))
                .expect("played");
            assert!(played.starts_with("GET /dav/Queen/01%20Song.flac "), "{played}");
            assert!(played.to_ascii_lowercase().contains(&want), "{played}");
        }
    }

    /// A share saved with credentials in its address (before adding took them out) loses them
    /// when the node loads it, and is saved again without them. A user in the share's fields
    /// wins over another one in the address; with the user field empty, the address gives the
    /// user and its password (a password field stays only when the address has none).
    #[test]
    fn a_share_saved_with_credentials_in_its_address_is_cleaned_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let lib = pm_library::Library::open(&dir.path().join("library.db")).unwrap();
        let plain = Share {
            id: "cc".into(),
            name: "Plain".into(),
            url: "https://nas.local/plain/".into(),
            user: "ann".into(),
            password: "pw".into(),
        };
        let before = serde_json::json!([
            {"id": "aa", "name": "NAS", "url": "http://me:p%40w@nas.local/dav/"},
            {"id": "bb", "name": "Cloud", "url": "https://me:tok@cloud.local/x/", "user": "bob", "password": "hunter2"},
            plain,
            {"id": "dd", "name": "Both", "url": "http://me:p%40w@nas.local/both/", "password": "hunter2"},
            {"id": "ee", "name": "User only", "url": "http://me@nas.local/user/", "password": "hunter2"},
        ]);
        lib.set_setting(SETTING, &before.to_string()).unwrap();
        let shares = load(&lib);
        let got: Vec<(&str, &str, &str)> = shares
            .iter()
            .map(|s| (s.url.as_str(), s.user.as_str(), s.password.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("http://nas.local/dav/", "me", "p@w"),
                ("https://cloud.local/x/", "bob", "hunter2"),
                ("https://nas.local/plain/", "ann", "pw"),
                ("http://nas.local/both/", "me", "p@w"),
                ("http://nas.local/user/", "me", "hunter2"),
            ]
        );
        let stored = lib.get_setting(SETTING).unwrap().unwrap();
        assert_eq!(serde_json::from_str::<Vec<Share>>(&stored).unwrap(), shares);
        for gone in ["p%40w", "me:", "me@", "tok"] {
            assert!(!stored.contains(gone), "{gone:?} in {stored}");
        }
        // Loaded again: nothing to move
        assert_eq!(load(&lib), shares);

        // Shown, a share's address never has credentials, also when it is not a URL
        for (url, shown) in [
            ("http://me:p%40w@nas.local/dav/", "http://nas.local/dav/"),
            ("http://:tok@nas.local/", "http://nas.local/"),
            ("http://nas.local/a@b/", "http://nas.local/a@b/"),
            ("http://me:pw@[bad/x@y", "http://[bad/x@y"),
        ] {
            assert_eq!(shown_url(url), shown, "{url}");
            let sh = Share {
                url: url.into(),
                password: "hunter2".into(),
                ..plain.clone()
            };
            let debug = format!("{sh:?}");
            assert!(debug.contains(shown) && !debug.contains("hunter2"), "{debug}");
        }
    }
}
