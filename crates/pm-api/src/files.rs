//! The user's own audio on their homeserver's private storage: copy from the
//! node, list, remove broken ones, and bring what this node lacks into a library
//! folder (a background job; files the library already has are matched by content
//! hash and not downloaded again). Private storage only (`/priv/pubky.music/files/`),
//! so uploads never become public (constraint 2) and the node only ever
//! serves its own user (constraint 3).

use crate::state::NodeState;
use crate::types::*;
use crate::{ApiError, ApiResult};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use pm_model::TrackId;
use pm_pubky::homeserver::{RemoteFile, PRIVATE_FILES_DIR};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// How fast copies to the homeserver go (bytes per second, 0 = full speed).
pub const COPY_RATE_KEY: &str = "copy.rate";
/// The default leaves room on a home uplink (often 20–40 Mbit/s) for the node's pages and the
/// music it streams to a browser away from home: a full-speed copy starved both (owner, 2026-10-22).
pub const DEFAULT_COPY_RATE: u64 = 2_500_000;

pub fn copy_rate(s: &NodeState) -> u64 {
    s.library
        .lock()
        .expect("library poisoned")
        .get_setting(COPY_RATE_KEY)
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_COPY_RATE)
}

/// Remote name for a local file: content hash prefix plus the original name,
/// so the same file uploaded twice lands on one path.
pub fn remote_name(bytes: &[u8], local: &Path) -> String {
    remote_name_for(&blake3_hex(bytes), &safe_file_name(local))
}

// Song file names are shared with the phone (pm-appdata)
pub use pm_appdata::files::split_remote_name;
pub(crate) use pm_appdata::files::{remote_name_for, safe_file_name};

fn blake3_hex(bytes: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(bytes);
    hasher.finalize().to_hex().to_string()
}

fn needs_session(s: &NodeState) -> Result<Arc<crate::sync::PubkySync>, ApiError> {
    s.sync_handle().ok_or_else(|| {
        ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "no Pubky session: set `pubky.session_secret_file` in the node config",
        )
    })
}

pub(crate) async fn list(State(s): State<Arc<NodeState>>) -> ApiResult<Vec<RemoteFile>> {
    let sync = needs_session(&s)?;
    let files = sync
        .list_private_files()
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(files))
}

/// Upload a node-local file (by path, or a library track by id).
pub(crate) async fn upload(
    State(s): State<Arc<NodeState>>,
    Json(body): Json<UploadFile>,
) -> ApiResult<UploadedFile> {
    let local: PathBuf = match (body.track_id, body.path.as_deref()) {
        (Some(id), _) => {
            s.library
                .lock()
                .expect("library poisoned")
                .get_track(TrackId(id))?
                .ok_or_else(|| ApiError::not_found(format!("no track {id}")))?
                .file
                .path
        }
        (None, Some(p)) if !p.trim().is_empty() => PathBuf::from(p),
        _ => return Err(ApiError::bad_request("give `track_id` or `path`")),
    };
    if !pm_library::tags::is_supported(&local) {
        return Err(ApiError::bad_request(format!(
            "{} is not a supported audio file",
            local.display()
        )));
    }
    let bytes = tokio::fs::read(&local)
        .await
        .map_err(|e| ApiError::bad_request(format!("read {}: {e}", local.display())))?;
    let size = bytes.len() as u64;
    // The bundled homeserver's disk keeps its reserve, as for every copy
    if let Some(why) = crate::copy_guard::room_of(&s)
        .space()
        .and_then(|sp| sp.too_little(size))
    {
        return Err(ApiError::new(StatusCode::INSUFFICIENT_STORAGE, why));
    }
    let target = crate::copy_guard::target_of(&s)?;
    let remote = format!("{PRIVATE_FILES_DIR}{}", remote_name(&bytes, &local));
    // The same guarded way as a copy: a good file there is never sent over, a failed upload
    // leaves nothing half written
    let mut present = std::collections::HashMap::new();
    match target.stat(&remote).await {
        Ok(Some(sz)) => {
            present.insert(remote.clone(), sz);
        }
        Ok(None) => {}
        Err(e) => return Err(ApiError::new(StatusCode::BAD_GATEWAY, e)),
    }
    let room = crate::copy_guard::room_of(&s);
    use crate::copy_guard::Refused;
    match send_one(&s, target.as_ref(), room.as_ref(), &mut present, &remote, bytes).await {
        Ok(_) => Ok(Json(UploadedFile { path: remote, size })),
        Err(Refused::Space(why)) => Err(ApiError::new(StatusCode::INSUFFICIENT_STORAGE, why)),
        Err(Refused::Full) => {
            crate::storage::note_full(&s, true);
            Err(ApiError::new(
                StatusCode::INSUFFICIENT_STORAGE,
                crate::add_music::FULL,
            ))
        }
        Err(Refused::Short(kept)) => Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            format!("the homeserver kept {kept} of {size} bytes; try again"),
        )),
        Err(Refused::Other(e)) => Err(ApiError::new(StatusCode::BAD_GATEWAY, e)),
    }
}

/// One library file to the homeserver: Ok(true) sent, Ok(false) already there (same content and
/// size). A failed or short upload is deleted there, never left looking like a song. What it
/// sends is added to `present`: a second file of the job with the same name isn't sent again.
pub(crate) async fn copy_one(
    s: &Arc<NodeState>,
    target: &dyn crate::copy_guard::Target,
    room: &dyn crate::copy_guard::Room,
    present: &mut std::collections::HashMap<String, Option<u64>>,
    path: &std::path::Path,
    name: &str,
) -> Result<bool, String> {
    let bytes = crate::copy_jobs::read_file(s, path)
        .await
        .map_err(|e| format!("{name}: can't be read: {e}"))?;
    if bytes.is_empty() {
        return Err(format!("{name}: the file is empty, not copied"));
    }
    let len = bytes.len() as u64;
    // Hashing a big FLAC is CPU work: off the threads that answer requests
    let p = path.to_path_buf();
    let (bytes, remote, hash) = tokio::task::spawn_blocking(move || {
        let hash = blake3_hex(&bytes);
        let r = format!(
            "{PRIVATE_FILES_DIR}{}",
            remote_name_for(&hash, &safe_file_name(&p))
        );
        (bytes, r, hash)
    })
    .await
    .map_err(|e| format!("{name}: {e}"))?;
    remember_hash(s, path, &hash);
    match send_one(s, target, room, present, &remote, bytes).await {
        Ok(sent) => Ok(sent),
        Err(crate::copy_guard::Refused::Space(why)) => Err(format!("{name}: {why}")),
        Err(crate::copy_guard::Refused::Full) => Err(format!("{name}: {}", crate::add_music::FULL)),
        Err(crate::copy_guard::Refused::Short(kept)) => Err(format!(
            "{name}: the homeserver kept {kept} of {len} bytes; run the copy again"
        )),
        Err(crate::copy_guard::Refused::Other(e)) => Err(format!("{name}: {e}")),
    }
}

/// One file's bytes to `remote`: Ok(false) when a good copy is there already. Sent only where
/// nothing good is (what is there now is checked first, unless `present` lists the path with
/// this size: a path the start's listing did not have may have been written since, by another
/// node or an upload, and an upload that failed over it would be removed as half written; one
/// whose check fails is not sent), guarded (the reserve with what is
/// really sent, recorded while in flight, checked after, nothing half written left there), and
/// added to `present` once there.
pub(crate) async fn send_one(
    s: &NodeState,
    target: &dyn crate::copy_guard::Target,
    room: &dyn crate::copy_guard::Room,
    present: &mut std::collections::HashMap<String, Option<u64>>,
    remote: &str,
    bytes: Vec<u8>,
) -> Result<bool, crate::copy_guard::Refused> {
    let len = bytes.len() as u64;
    match present.get(remote) {
        Some(sz) if *sz == Some(len) => return Ok(false),
        // Not listed: a good copy there now is kept; sent when there is none. Not when that
        // cannot be checked: a failed upload's clean-up could remove a good copy written since
        None => match crate::copy_guard::whole_copy_there(target, remote, len).await {
            Ok(true) => {
                present.insert(remote.to_string(), Some(len));
                return Ok(false);
            }
            Ok(false) => {}
            Err(e) => {
                return Err(crate::copy_guard::Refused::Other(format!(
                    "couldn't check the copy there ({e})"
                )))
            }
        },
        // Listed with its size unknown or another: what is there now decides
        Some(_) => match crate::copy_guard::needs_sending(target, remote, len).await {
            Ok(true) => {}
            Ok(false) => return Ok(false),
            Err(e) => {
                return Err(crate::copy_guard::Refused::Other(format!(
                    "couldn't check the copy there ({e})"
                )))
            }
        },
    }
    let rate = copy_rate(s);
    let rate = (rate > 0).then_some(rate);
    crate::copy_guard::upload_guarded(s, target, room, remote, bytes, rate).await?;
    crate::storage::note_full(s, false);
    present.insert(remote.to_string(), Some(len));
    Ok(true)
}

/// `GET /v1/pubky/files/upload/{id}`: a copy job, an import or a bring job.
pub(crate) async fn upload_progress(
    State(s): State<Arc<NodeState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> ApiResult<UploadProgress> {
    if let Some(p) = crate::copy_jobs::find(&s, &id) {
        return Ok(Json(p));
    }
    s.upload_jobs
        .lock()
        .expect("upload jobs")
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or_else(|| ApiError::not_found("no such upload"))
}

/// Only one files job at a time (copy, import or bring): they share the uplink and the
/// homeserver. A copy is queued instead (copy_jobs.rs). None while the start settles.
pub(crate) fn claim_job(s: &NodeState, job: &UploadProgress) -> Result<(), ApiError> {
    claim_job_as(s, job, false)
}

/// `resuming`: the bring job the node resumes at start (`resume_bring`), the only one let in
/// while the start is held; the kept copies wait for it.
pub(crate) fn claim_job_as(s: &NodeState, job: &UploadProgress, resuming: bool) -> Result<(), ApiError> {
    let _admitted = crate::copy_jobs::admit(s);
    if crate::copy_jobs::held(s) {
        if !resuming {
            return Err(crate::copy_jobs::settling());
        }
    } else if let Some(copy) = crate::copy_jobs::busy(s) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!(
                "A copy to your homeserver is {} ({} of {} files); wait for it or cancel it",
                if copy.state == "paused" {
                    "paused"
                } else {
                    "running"
                },
                copy.done + copy.skipped,
                copy.total
            ),
        ));
    }
    let mut jobs = s.upload_jobs.lock().expect("upload jobs");
    jobs.retain(|_, j| !j.finished);
    if let Some(running) = jobs.values().next() {
        let what = if running.kind == "bring" {
            "Bringing files here"
        } else {
            "A copy to your homeserver"
        };
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!(
                "{what} is running ({} of {} files); wait for it or cancel it",
                running.done + running.skipped,
                running.total
            ),
        ));
    }
    jobs.insert(job.id.clone(), job.clone());
    Ok(())
}

fn is_cancelled(s: &NodeState, id: &str) -> bool {
    s.upload_jobs
        .lock()
        .expect("upload jobs")
        .get(id)
        .is_some_and(|j| j.cancelled)
}

/// `POST /v1/pubky/files/jobs/{id}/cancel`: stop after the file in hand (a download stops at
/// once and leaves nothing half-written).
pub(crate) async fn cancel_job(
    State(s): State<Arc<NodeState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> ApiResult<UploadProgress> {
    if let Some(p) = crate::copy_jobs::cancel(&s, &id) {
        return Ok(Json(p));
    }
    let job = {
        let mut jobs = s.upload_jobs.lock().expect("upload jobs");
        let j = jobs
            .get_mut(&id)
            .ok_or_else(|| ApiError::not_found("no such job"))?;
        j.cancelled = true;
        j.clone()
    };
    if job.kind == "bring" {
        // Not started again after a restart; the record stays until the job has cleared its
        // part folder (or, if the node stops first, the next start clears it)
        let lib = s.library.lock().expect("library poisoned");
        if let Some(mut v) = bring_record(&lib).filter(|v| v.is_object()) {
            v["cancelled"] = true.into();
            let _ = lib.set_setting(BRING_RESUME_KEY, &v.to_string());
        }
    }
    Ok(Json(job))
}

fn mtime_of(m: &std::fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The content hash this node already knows for a file (a copy or import hashed it before, and it
/// hasn't changed since).
pub(crate) fn known_hash(s: &NodeState, path: &Path) -> Option<String> {
    let m = std::fs::metadata(path).ok()?;
    s.library
        .lock()
        .expect("library poisoned")
        .cached_file_hash(path, m.len(), mtime_of(&m))
        .ok()
        .flatten()
}

/// Keep a library file's content hash (the copy job just computed it).
pub(crate) fn remember_hash(s: &NodeState, path: &Path, hash: &str) {
    if let Ok(m) = std::fs::metadata(path) {
        let _ =
            s.library
                .lock()
                .expect("library poisoned")
                .store_file_hash(path, m.len(), mtime_of(&m), hash);
    }
}

/// BLAKE3 of a whole file, read in pieces.
fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut f = std::fs::File::open(path)?;
    std::io::copy(&mut f, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}

/// A bring job that was running when the node stopped: `{dest, parts, cancelled?}` (`parts`: the
/// name of its part folder in `dest`, which the next bring clears); the node starts it again,
/// unless it was cancelled (then only its part folder is cleared).
pub const BRING_RESUME_KEY: &str = "files.bring";

/// What the homeserver has and what this node lacks.
struct BringPlan {
    dest: PathBuf,
    /// (remote path, size, local file name).
    fetch: Vec<(String, u64, String)>,
    present: usize,
    present_bytes: u64,
    broken: usize,
    /// Files whose size the homeserver didn't give in time: left out this time, not called
    /// broken (the next bring looks again).
    unchecked: usize,
}

/// The homeserver's files a bring plans with: (path, size, `split_remote_name` parts) of each
/// one to look at, and how many are broken (empty, or a second copy under a doubled name) and
/// unchecked (no size read: unknown, not empty).
type Sorted = (Vec<(String, u64, Option<(String, String)>)>, usize, usize);

fn sort_remote(remote: Vec<pm_pubky::homeserver::RemoteFile>) -> Sorted {
    // Names on the homeserver, to spot a second copy under a doubled prefix
    let names: std::collections::HashSet<String> = remote.iter().map(|f| f.name().to_string()).collect();
    let (mut wanted, mut broken, mut unchecked) = (Vec::new(), 0, 0);
    for f in remote {
        let name = f.name().to_string();
        let parts = split_remote_name(&name).map(|(h, rest)| (h.to_string(), rest.to_string()));
        let doubled = parts.as_ref().is_some_and(|(h, rest)| {
            split_remote_name(rest).is_some_and(|(h2, _)| h2 == h) && names.contains(rest.as_str())
        });
        match f.size {
            _ if doubled => broken += 1,
            Some(0) => broken += 1,
            None => unchecked += 1,
            Some(size) => wanted.push((f.path, size, parts)),
        }
    }
    (wanted, broken, unchecked)
}

/// A file name from elsewhere (the homeserver) as one plain name in a folder: separators of
/// any system (`/`, `\\`), drive and stream markers (`:`) and control characters become `_`, and
/// `.`, `..` or nothing left is no name at all. On Windows `..\\x` or `C:\\x` would otherwise
/// leave the folder.
pub(crate) fn file_name_from(name: &str) -> Option<String> {
    let clean: String = name
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let clean = clean.trim().trim_end_matches('.').to_string();
    (!clean.is_empty() && clean != "." && clean != "..").then_some(clean)
}

/// The folder files are brought into: `From homeserver` in the first library folder, else the
/// node's data folder.
fn default_dest(s: &NodeState) -> PathBuf {
    s.library
        .lock()
        .expect("library poisoned")
        .list_folders()
        .ok()
        .and_then(|f| f.into_iter().find(|(_, on)| *on).map(|(p, _)| p))
        .map(|p| p.join("From homeserver"))
        .unwrap_or_else(|| s.mirror_dir.clone())
}

async fn plan_bring(
    s: &NodeState,
    sync: &crate::sync::PubkySync,
    dest: PathBuf,
) -> Result<BringPlan, String> {
    let remote = sync
        .list_private_files()
        .await
        .map_err(|e| format!("the homeserver did not list its files: {e}"))?;
    let mut plan = BringPlan {
        dest,
        fetch: Vec::new(),
        present: 0,
        present_bytes: 0,
        broken: 0,
        unchecked: 0,
    };
    let (wanted, broken, unchecked) = sort_remote(remote);
    plan.broken = broken;
    plan.unchecked = unchecked;
    // Library files of the same sizes, hashed (kept, so only the first plan reads them)
    let sizes: std::collections::HashSet<u64> = wanted.iter().map(|(_, sz, _)| *sz).collect();
    let candidates = s
        .library
        .lock()
        .expect("library poisoned")
        .files_of_sizes(&sizes)
        .map_err(|e| e.to_string())?;
    let mut have: std::collections::HashSet<String> = std::collections::HashSet::new();
    // The local names this plan gives out
    let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (path, size, mtime) in candidates {
        let cached = s
            .library
            .lock()
            .expect("library poisoned")
            .cached_file_hash(&path, size, mtime)
            .ok()
            .flatten();
        let hash = match cached {
            Some(h) => h,
            None => {
                let p = path.clone();
                let Ok(Ok(h)) = tokio::task::spawn_blocking(move || hash_file(&p)).await else {
                    continue;
                };
                let _ = s
                    .library
                    .lock()
                    .expect("library poisoned")
                    .store_file_hash(&path, size, mtime, &h);
                h
            }
        };
        have.insert(hash[..16].to_string());
    }
    for (path, size, parts) in wanted {
        // A name from the homeserver becomes one plain file name in the folder, never a path
        let Some(full) = file_name_from(path.rsplit('/').next().unwrap_or(&path)) else {
            plan.broken += 1;
            continue;
        };
        let (hash16, original) = parts.unwrap_or_else(|| (String::new(), full.clone()));
        let Some(original) = file_name_from(&original) else {
            plan.broken += 1;
            continue;
        };
        if !hash16.is_empty() && have.contains(&hash16) {
            plan.present += 1;
            plan.present_bytes += size;
            continue;
        }
        match local_name(&plan.dest, &original, &full, size, &mut taken).await {
            Some(local) => plan.fetch.push((path, size, local)),
            // Already brought (maybe not scanned yet)
            None => {
                plan.present += 1;
                plan.present_bytes += size;
            }
        }
    }
    Ok(plan)
}

/// The name a file from the homeserver gets in `dest`, or `None` when the folder has it already
/// (the same name and size). It is the file's `original` name, or the homeserver's `full` name
/// (which starts with its hash) when the original name is another file's: one in the folder,
/// or one this plan already brings (two albums each have a `01 Intro.flac`); when that one is
/// another file's too, the full name with a number in front. `taken` holds the names the plan
/// uses so far, in lower case: on a folder that ignores case, names that differ only in case
/// are one file. No name is given to two files, and none that the folder has for another file.
async fn local_name(
    dest: &Path,
    original: &str,
    full: &str,
    size: u64,
    taken: &mut std::collections::HashSet<String>,
) -> Option<String> {
    let mut n = 0u32;
    loop {
        let name = match n {
            0 => original.to_string(),
            1 => full.to_string(),
            n => format!("{n}-{full}"),
        };
        // (names run out only after 4 billion tries: then no name)
        n = n.checked_add(1)?;
        if taken.contains(&name.to_lowercase()) {
            continue;
        }
        let at = dest.join(&name);
        // (a link that leads nowhere has the name too)
        if tokio::fs::symlink_metadata(&at).await.is_err() {
            taken.insert(name.to_lowercase());
            return Some(name);
        }
        if matches!(tokio::fs::metadata(&at).await, Ok(m) if m.is_file() && m.len() == size) {
            // (the name is this file's: no other file of the plan gets it in another case)
            taken.insert(name.to_lowercase());
            return None;
        }
        // Another file under that name: the next one
    }
}

/// The start of the name of a bring's part folder in its destination: `.odeion-bring-<job id>`,
/// made new for each bring (never one that is there already), readable only to the node.
const PARTS_PREFIX: &str = ".odeion-bring-";

/// Clear the part folder `name` an earlier bring made in `dest` (it stopped midway): only its
/// `<n>.part` files go, then the folder when nothing else is in it. A name that is not a part
/// folder's, or a link at that name, is left alone (a link is never followed).
async fn clear_parts(dest: &Path, name: &str) {
    let ours = name.len() > PARTS_PREFIX.len()
        && name.starts_with(PARTS_PREFIX)
        && Path::new(name).file_name() == Some(std::ffi::OsStr::new(name));
    if !ours {
        return;
    }
    let dir = dest.join(name);
    let at = dir.clone();
    let _ = tokio::task::spawn_blocking(move || clear_numbered_parts(&at)).await;
    let _ = tokio::fs::remove_dir(&dir).await;
}

/// `<n>.part` (digits only): a name a bring gave a part.
fn numbered_part(n: &[u8]) -> bool {
    n.strip_suffix(b".part")
        .is_some_and(|k| !k.is_empty() && k.iter().all(u8::is_ascii_digit))
}

/// Remove the numbered part files in `dir`. On Unix the folder is opened once without following a
/// link, and its entries are listed and removed through that handle, so a folder swapped for a link
/// meanwhile cannot send the removals elsewhere. (Removing a name never touches what a link there
/// leads to.)
#[cfg(unix)]
fn clear_numbered_parts(dir: &Path) {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return;
    };
    // SAFETY: a NUL-terminated path that lives through the call
    let fd = unsafe {
        libc::open(
            c.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return;
    }
    // SAFETY: `fd` is an open folder; on success the stream owns it, on failure it is closed here
    let dirp = unsafe { libc::fdopendir(fd) };
    if dirp.is_null() {
        // SAFETY: `fd` is still ours
        unsafe { libc::close(fd) };
        return;
    }
    let mut names = Vec::new();
    loop {
        // SAFETY: `dirp` is an open stream; an entry is read before the next call
        let ent = unsafe { libc::readdir(dirp) };
        if ent.is_null() {
            break;
        }
        // SAFETY: `d_name` is a NUL-terminated name inside the entry
        let name = unsafe { std::ffi::CStr::from_ptr((*ent).d_name.as_ptr()) };
        if numbered_part(name.to_bytes()) {
            names.push(name.to_owned());
        }
    }
    for n in &names {
        // SAFETY: the stream's folder and a NUL-terminated name
        unsafe { libc::unlinkat(libc::dirfd(dirp), n.as_ptr(), 0) };
    }
    // SAFETY: closes the stream and its folder, once
    unsafe { libc::closedir(dirp) };
}

#[cfg(not(unix))]
fn clear_numbered_parts(dir: &Path) {
    match std::fs::symlink_metadata(dir) {
        Ok(m) if m.is_dir() => {}
        _ => return,
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if numbered_part(e.file_name().to_string_lossy().as_bytes()) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// The bring record ([`BRING_RESUME_KEY`]), if there is one that reads.
fn bring_record(lib: &pm_library::Library) -> Option<serde_json::Value> {
    lib.get_setting(BRING_RESUME_KEY)
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok())
}

/// A record's destination and part folder name.
fn record_parts(v: &serde_json::Value) -> Option<(PathBuf, String)> {
    Some((
        PathBuf::from(v["dest"].as_str()?),
        v["parts"].as_str()?.to_string(),
    ))
}

/// The bring is over: its part folder is cleared and its record goes. `only`: only when the
/// record still names that part folder (a job ending does not remove a later bring's record).
async fn forget_bring(s: &NodeState, only: Option<&str>) {
    let rec = bring_record(&s.library.lock().expect("library poisoned"));
    let Some(rec) = rec else { return };
    if only.is_some_and(|name| rec["parts"].as_str() != Some(name)) {
        return;
    }
    if let Some((dest, name)) = record_parts(&rec) {
        clear_parts(&dest, &name).await;
    }
    let lib = s.library.lock().expect("library poisoned");
    // (unless a bring started meanwhile)
    if bring_record(&lib).is_some_and(|now| now["parts"] == rec["parts"]) {
        let _ = lib.delete_setting(BRING_RESUME_KEY);
    }
}

/// Give a finished download (`part`, a file this bring made) its name (`target`) without
/// replacing a file that got that name since the plan was made: such a file stays, the download
/// is dropped and the error says so (the next bring gives it another name).
async fn place_new(part: &Path, target: &Path) -> std::io::Result<()> {
    use std::io::ErrorKind::AlreadyExists;
    // A link is made only when the name is free, in one step
    let placed = match tokio::fs::hard_link(part, target).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == AlreadyExists => Err(e),
        // A folder that has no links (FAT, some shares): a move that never replaces (Linux),
        // else a copy
        Err(_) => match rename_noreplace(part, target).await {
            Some(r) => r,
            None => copy_new(part, target).await,
        },
    };
    let _ = tokio::fs::remove_file(part).await;
    placed.map_err(|e| {
        if e.kind() == AlreadyExists {
            std::io::Error::new(
                AlreadyExists,
                "a file of that name is there now; it was not replaced",
            )
        } else {
            e
        }
    })
}

/// Move `from` to `to` in one step only when `to` is free (`renameat2` with RENAME_NOREPLACE).
/// `None`: this system or file system has no such move.
async fn rename_noreplace(from: &Path, to: &Path) -> Option<std::io::Result<()>> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = |p: &Path| std::ffi::CString::new(p.as_os_str().as_bytes()).ok();
        let (Some(from), Some(to)) = (c(from), c(to)) else {
            return None;
        };
        let r = tokio::task::spawn_blocking(move || {
            // SAFETY: two NUL-terminated paths that live through the call; no memory is shared
            let rc = unsafe {
                libc::syscall(
                    libc::SYS_renameat2,
                    libc::AT_FDCWD,
                    from.as_ptr(),
                    libc::AT_FDCWD,
                    to.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            };
            if rc == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        })
        .await
        .ok()?;
        match r {
            // No such move here (an old kernel, a file system without it): the copy
            Err(e) if matches!(e.raw_os_error(), Some(libc::EINVAL | libc::ENOSYS | libc::EPERM)) => None,
            r => Some(r),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let _ = (from, to);
        None
    }
}

/// Copy `part` into a file made only when the name `target` is free (one step), never a
/// rename, which would replace a file that appeared since. What a failed copy made goes, and
/// only that: a file another put under the name meanwhile stays. (Between the look and the
/// removal another process could still swap the file; that needs write access to the folder,
/// which can remove or replace any file there anyway.)
async fn copy_new(part: &Path, target: &Path) -> std::io::Result<()> {
    let mut out = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .await?;
    let copied = async {
        let mut from = tokio::fs::File::open(part).await?;
        tokio::io::copy(&mut from, &mut out).await?;
        out.sync_all().await
    }
    .await;
    if copied.is_err() {
        #[cfg(unix)]
        let still_ours = {
            use std::os::unix::fs::MetadataExt;
            match (out.metadata().await, tokio::fs::symlink_metadata(target).await) {
                (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
                _ => false,
            }
        };
        #[cfg(not(unix))]
        let still_ours = true;
        drop(out);
        if still_ours {
            let _ = tokio::fs::remove_file(target).await;
        }
    }
    copied
}

/// `POST /v1/pubky/files/mirror {dest?, dry_run?}`: bring the homeserver's files this node does
/// not have into a library folder, as a background job (progress at `GET …/jobs/{id}`). Files the
/// library already has (same content) are not downloaded; the folder is scanned at the end.
pub(crate) async fn mirror(
    State(s): State<Arc<NodeState>>,
    body: Option<Json<MirrorRequest>>,
) -> ApiResult<UploadProgress> {
    let req = body.map(|Json(b)| b).unwrap_or(MirrorRequest {
        dest: None,
        dry_run: false,
    });
    let sync = needs_session(&s)?;
    let dest = match req.dest.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => {
            let p = PathBuf::from(d);
            if !p.is_absolute() {
                return Err(ApiError::bad_request("give the folder as a full path"));
            }
            p
        }
        None => default_dest(&s),
    };
    let plan = plan_bring(&s, &sync, dest)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e))?;
    let progress = UploadProgress {
        id: crate::tidal::random_urlsafe(9),
        kind: "bring".into(),
        dest: Some(plan.dest.to_string_lossy().into_owned()),
        total: plan.fetch.len(),
        bytes_total: plan.fetch.iter().map(|(_, b, _)| b).sum(),
        skipped: plan.present,
        broken: plan.broken,
        unchecked: plan.unchecked,
        ..Default::default()
    };
    if req.dry_run || plan.fetch.is_empty() {
        let mut p = progress;
        p.finished = !req.dry_run;
        return Ok(Json(p));
    }
    claim_job(&s, &progress)?;
    start_bring(s.clone(), sync, plan, progress.id.clone());
    Ok(Json(progress))
}

fn start_bring(s: Arc<NodeState>, sync: Arc<crate::sync::PubkySync>, plan: BringPlan, id: String) {
    // The part folder of a bring that stopped midway (its record), cleared below
    let earlier = {
        let lib = s.library.lock().expect("library poisoned");
        let earlier = bring_record(&lib).as_ref().and_then(record_parts);
        let parts_name = format!("{PARTS_PREFIX}{id}");
        let _ = lib.set_setting(
            BRING_RESUME_KEY,
            &serde_json::json!({"dest": plan.dest, "parts": parts_name}).to_string(),
        );
        earlier
    };
    tokio::spawn(async move {
        let set = |f: &dyn Fn(&mut UploadProgress)| {
            if let Some(j) = s.upload_jobs.lock().expect("upload jobs").get_mut(&id) {
                f(j);
            }
        };
        let dest = plan.dest.clone();
        // What an earlier bring left (the node stopped) goes first; a part that is a link to a
        // song already in place is only a second name for it
        if let Some((at, name)) = &earlier {
            clear_parts(at, name).await;
        }
        if let Err(e) = tokio::fs::create_dir_all(&dest).await {
            let msg = format!("could not create {}: {e}", dest.display());
            set(&|j| {
                j.errors.push(msg.clone().into());
                j.finished = true;
            });
            return;
        }
        let mut fetched = 0usize;
        // Part files: `.odeion-bring-<id>/<n>.part` in the destination, a folder made new here
        // (never one that was there) that only the node can write in
        let parts = dest.join(format!("{PARTS_PREFIX}{id}"));
        #[allow(unused_mut)] // (only Unix sets a mode)
        let mut mkdir = tokio::fs::DirBuilder::new();
        #[cfg(unix)]
        mkdir.mode(0o700);
        if let Err(e) = mkdir.create(&parts).await {
            let msg = format!("could not create {}: {e}", parts.display());
            set(&|j| {
                j.errors.push(msg.clone().into());
                j.finished = true;
            });
            return;
        }
        let mut fetched_parts = 0u64;
        for (remote, size, local) in plan.fetch {
            if is_cancelled(&s, &id) {
                break;
            }
            set(&|j| j.current = Some(local.clone()));
            let target = dest.join(&local);
            // In a folder of its own, by number: never the name of a file this bring places.
            // Made new by the download (never a file that is there)
            fetched_parts += 1;
            let part = parts.join(format!("{fetched_parts}.part"));
            let rate = copy_rate(&s);
            let mut got = 0u64;
            let r = sync
                .download_private_to(&remote, &part, (rate > 0).then_some(rate), |n| {
                    got += n;
                    set(&|j| j.bytes_done += n);
                    !is_cancelled(&s, &id)
                })
                .await;
            match r {
                Ok(n) if n == size || size == 0 => {
                    if let Err(e) = place_new(&part, &target).await {
                        let msg = format!("{local}: {e}");
                        set(&|j| {
                            j.failed += 1;
                            j.errors.push(msg.clone().into());
                        });
                    } else {
                        fetched += 1;
                        set(&|j| j.done += 1);
                    }
                }
                other => {
                    let _ = tokio::fs::remove_file(&part).await;
                    let msg = match other {
                        Ok(n) => format!("{local}: got {n} of {size} bytes"),
                        Err(e) => format!("{local}: {e}"),
                    };
                    set(&|j| {
                        j.bytes_done = j.bytes_done.saturating_sub(got);
                        if !j.cancelled {
                            j.failed += 1;
                            j.errors.push(msg.clone().into());
                        }
                    });
                }
            }
        }
        // (nothing of an unfinished part is left in it)
        clear_parts(&dest, &format!("{PARTS_PREFIX}{id}")).await;
        // Into the library: a folder outside the library folders becomes one
        if fetched > 0 {
            let inside = s
                .library
                .lock()
                .expect("library poisoned")
                .list_folders()
                .unwrap_or_default()
                .into_iter()
                .any(|(f, on)| on && dest.starts_with(&f));
            if !inside {
                let _ = s.library.lock().expect("library poisoned").add_folder(&dest);
                s.watch_more(&dest);
            }
            // A scan already running would miss the new files: wait for it
            for _ in 0..600 {
                match s.start_scan(vec![dest.clone()]) {
                    Ok(false) => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
                    _ => break,
                }
            }
        }
        // Done or cancelled: not started again (the part folder is clear already)
        forget_bring(&s, Some(&format!("{PARTS_PREFIX}{id}"))).await;
        set(&|j| {
            j.current = None;
            j.finished = true;
            let n = j.errors.len();
            if n > 5 {
                j.errors.drain(..n - 5);
            }
        });
    });
}

/// After a restart: carry on with a bring job the node was running (what already arrived is
/// matched and not fetched again).
pub fn resume_jobs(s: &Arc<NodeState>) {
    // The copies the node had are back in place now (a copy started meanwhile queues behind
    // them); they run once the rest is done
    crate::copy_jobs::resume_after_restart(s);
    let rec = bring_record(&s.library.lock().expect("library poisoned"));
    // A cancelled bring is not started again
    let cancelled = rec.as_ref().is_some_and(|v| v["cancelled"] == true);
    let dest = rec
        .as_ref()
        .filter(|_| !cancelled)
        .and_then(|v| v["dest"].as_str().map(PathBuf::from));
    let s = s.clone();
    tokio::spawn(async move {
        // First what uploads of a stopped node left half written on the homeserver goes: a copy
        // carrying on would write the same paths
        crate::copy_guard::settle_in_flight(&s, 120, std::time::Duration::from_secs(1)).await;
        // A bring job before the copies, which would otherwise find a copy in its place
        if let Some(dest) = dest {
            resume_bring(&s, dest).await;
        } else if cancelled {
            forget_bring(&s, None).await;
        }
        crate::copy_jobs::release_after_restart(&s);
    });
}

/// A bring job the node was running when it stopped carries on.
async fn resume_bring(s: &Arc<NodeState>, dest: PathBuf) {
    let s = s.clone();
    {
        // The Pubky session comes back in the background after start-up
        let mut sync = None;
        for _ in 0..120 {
            if let Some(h) = s.sync_handle() {
                sync = Some(h);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        let Some(sync) = sync else { return };
        let plan = match plan_bring(&s, &sync, dest).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("bringing files here did not resume: {e}");
                return;
            }
        };
        if plan.fetch.is_empty() {
            forget_bring(&s, None).await;
            return;
        }
        let progress = UploadProgress {
            id: crate::tidal::random_urlsafe(9),
            kind: "bring".into(),
            dest: Some(plan.dest.to_string_lossy().into_owned()),
            total: plan.fetch.len(),
            bytes_total: plan.fetch.iter().map(|(_, b, _)| b).sum(),
            skipped: plan.present,
            broken: plan.broken,
            unchecked: plan.unchecked,
            ..Default::default()
        };
        if claim_job_as(&s, &progress, true).is_ok() {
            tracing::info!(
                files = progress.total,
                "bringing files here: carrying on after a restart"
            );
            start_bring(s.clone(), sync, plan, progress.id);
        }
    }
}

/// `DELETE /v1/pubky/files {paths}`: remove private files (broken uploads, doubled copies).
pub(crate) async fn remove(
    State(s): State<Arc<NodeState>>,
    Json(body): Json<RemoveFiles>,
) -> ApiResult<Vec<RemoteFile>> {
    let sync = needs_session(&s)?;
    for p in &body.paths {
        if !p.starts_with(PRIVATE_FILES_DIR) || p.contains("..") {
            return Err(ApiError::bad_request(format!("not one of your music files: {p}")));
        }
    }
    for p in &body.paths {
        sync.delete_private(p)
            .await
            .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, format!("{p}: {e}")))?;
    }
    // Space was freed: no longer "full" until the homeserver says so again
    crate::storage::note_full(&s, false);
    let files = sync
        .list_private_files()
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(files))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A homeserver file name never leaves the folder files are brought into, on any system
    /// (security review 2026-10-30, #6).
    #[test]
    fn homeserver_names_become_plain_file_names() {
        assert_eq!(file_name_from("Song.flac").as_deref(), Some("Song.flac"));
        assert_eq!(
            file_name_from("..\\..\\Windows\\evil.dll").as_deref(),
            Some(".._.._Windows_evil.dll")
        );
        assert_eq!(file_name_from("C:\\x.flac").as_deref(), Some("C__x.flac"));
        assert_eq!(file_name_from("a/b.flac").as_deref(), Some("a_b.flac"));
        assert_eq!(
            file_name_from("song.flac:stream").as_deref(),
            Some("song.flac_stream")
        );
        for bad in ["", ".", "..", " .. ", "...", "\u{0}"] {
            let n = file_name_from(bad);
            assert!(n.is_none() || n.as_deref() == Some("_"), "{bad:?} -> {n:?}");
        }
        for n in ["..\\..\\x", "C:\\x", "../x", "/etc/x"] {
            let f = file_name_from(n).unwrap();
            let p = std::path::Path::new("/dest").join(&f);
            assert_eq!(p.parent(), Some(std::path::Path::new("/dest")), "{n}");
        }
    }

    #[test]
    fn remote_names_are_content_addressed_and_safe() {
        let a = remote_name(b"abc", Path::new("/music/My Song (live).flac"));
        let b = remote_name(b"abc", Path::new("/elsewhere/My Song (live).flac"));
        assert_eq!(a, b);
        assert!(a.ends_with("-My_Song__live_.flac"), "{a}");
        assert_eq!(a.len(), 16 + 1 + "My_Song__live_.flac".len());
        assert_ne!(
            remote_name(b"abd", Path::new("x.flac")),
            remote_name(b"abc", Path::new("x.flac"))
        );
    }

    /// A size the homeserver didn't give in time is unknown, not empty: such a file is left out
    /// for now, never called broken (Settings would offer to remove it); only a size of 0 is.
    #[test]
    fn a_file_with_no_size_read_is_unchecked_not_broken() {
        use pm_pubky::homeserver::RemoteFile;
        let f = |name: &str, size: Option<u64>| RemoteFile {
            path: format!("/priv/pubky.music/files/{name}"),
            size,
        };
        let (wanted, broken, unchecked) = sort_remote(vec![
            f("0123456789abcdef-a.flac", Some(10)),
            f("1123456789abcdef-b.flac", None),
            f("2123456789abcdef-c.flac", Some(0)),
            f("3123456789abcdef-d.flac", None),
        ]);
        let paths: Vec<&str> = wanted.iter().map(|(p, _, _)| p.as_str()).collect();
        assert_eq!(paths, ["/priv/pubky.music/files/0123456789abcdef-a.flac"]);
        assert_eq!((broken, unchecked), (1, 2));
    }

    /// Files from the homeserver that share an original name (a `01 Intro.flac` in two albums)
    /// each get a name of their own in the folder, also when the names differ only in case; a
    /// file that is in the folder under the name is never planned over, and one of the same
    /// name and size counts as brought.
    #[tokio::test]
    async fn files_of_the_same_name_are_brought_under_names_of_their_own() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let mut taken = std::collections::HashSet::new();
        let a = local_name(
            d,
            "01 Intro.flac",
            "aaaaaaaaaaaaaaaa-01_Intro.flac",
            10,
            &mut taken,
        )
        .await;
        let b = local_name(
            d,
            "01 Intro.flac",
            "bbbbbbbbbbbbbbbb-01_Intro.flac",
            20,
            &mut taken,
        )
        .await;
        let c = local_name(
            d,
            "01 INTRO.flac",
            "cccccccccccccccc-01_INTRO.flac",
            30,
            &mut taken,
        )
        .await;
        assert_eq!(a.as_deref(), Some("01 Intro.flac"));
        assert_eq!(b.as_deref(), Some("bbbbbbbbbbbbbbbb-01_Intro.flac"));
        assert_eq!(c.as_deref(), Some("cccccccccccccccc-01_INTRO.flac"));
        // In the folder: another file under the original name, and this file under the full one
        std::fs::write(d.join("Song.flac"), [0u8; 7]).unwrap();
        std::fs::write(d.join("dddddddddddddddd-Song.flac"), [0u8; 9]).unwrap();
        let mut taken = std::collections::HashSet::new();
        assert_eq!(
            local_name(d, "Song.flac", "dddddddddddddddd-Song.flac", 9, &mut taken).await,
            None
        );
        assert!(
            taken.contains("dddddddddddddddd-song.flac"),
            "a brought file's name is its own"
        );
        assert_eq!(
            local_name(d, "Song.flac", "eeeeeeeeeeeeeeee-Song.flac", 7, &mut taken).await,
            None
        );
        // Both names are other files': a third name, and none of the three is given out twice
        let f = local_name(d, "Song.flac", "dddddddddddddddd-Song.flac", 5, &mut taken).await;
        let g = local_name(d, "Song.flac", "dddddddddddddddd-Song.flac", 6, &mut taken).await;
        assert_eq!(f.as_deref(), Some("2-dddddddddddddddd-Song.flac"));
        assert_eq!(g.as_deref(), Some("3-dddddddddddddddd-Song.flac"));
        // A folder under the name is no copy of the file
        std::fs::create_dir(d.join("Live.flac")).unwrap();
        let h = local_name(d, "Live.flac", "ffffffffffffffff-Live.flac", 4096, &mut taken).await;
        assert_eq!(h.as_deref(), Some("ffffffffffffffff-Live.flac"));
    }

    /// Where no link can be made, the download is copied into a file made only when the name
    /// is free: a copy that fails (here: the part is a folder) leaves no file of that name, and a
    /// file of that name stays.
    #[tokio::test]
    async fn without_links_a_failed_copy_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let (part, target) = (dir.path().join("1.part"), dir.path().join("Song.flac"));
        std::fs::create_dir(&part).unwrap();
        assert!(copy_new(&part, &target).await.is_err());
        assert!(!target.exists(), "what the failed copy made is gone");
        std::fs::write(&target, b"the owner's").unwrap();
        let e = copy_new(&part, &target).await.unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&target).unwrap(), b"the owner's");
        // A copy that works
        let (part, other) = (dir.path().join("2.part"), dir.path().join("Other.flac"));
        std::fs::write(&part, b"from the homeserver").unwrap();
        copy_new(&part, &other).await.unwrap();
        assert_eq!(std::fs::read(&other).unwrap(), b"from the homeserver");
    }

    /// The move used where no link can be made never replaces a file.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[tokio::test]
    async fn the_move_without_links_never_replaces_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let (part, target) = (dir.path().join("1.part"), dir.path().join("Song.flac"));
        std::fs::write(&part, b"from the homeserver").unwrap();
        std::fs::write(&target, b"the owner's").unwrap();
        // (a temp folder whose file system has no such move: nothing to test here)
        let Some(r) = rename_noreplace(&part, &target).await else {
            return;
        };
        let e = r.unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&target).unwrap(), b"the owner's");
        assert!(part.exists());
        std::fs::remove_file(&target).unwrap();
        rename_noreplace(&part, &target).await.unwrap().unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"from the homeserver");
        assert!(!part.exists());
    }

    /// Clearing an earlier bring's part folder takes only its numbered part files, follows no
    /// link at the folder's name, and ignores a name that is not a part folder's.
    #[cfg(unix)]
    #[tokio::test]
    async fn clearing_parts_takes_only_a_bring_s_own_part_files() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("dest");
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir_all(dest.join(".odeion-bring-abc")).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(dest.join(".odeion-bring-abc/1.part"), b"x").unwrap();
        std::fs::write(dest.join(".odeion-bring-abc/notes.part.txt"), b"x").unwrap();
        std::fs::write(elsewhere.join("1.part"), b"x").unwrap();
        std::os::unix::fs::symlink(&elsewhere, dest.join(".odeion-bring-link")).unwrap();

        clear_parts(&dest, ".odeion-bring-abc").await;
        assert!(!dest.join(".odeion-bring-abc/1.part").exists());
        assert!(
            dest.join(".odeion-bring-abc/notes.part.txt").exists(),
            "not a part file"
        );
        assert!(dest.join(".odeion-bring-abc").is_dir(), "not empty: kept");

        clear_parts(&dest, ".odeion-bring-link").await;
        assert!(elsewhere.join("1.part").exists(), "a link is not followed");
        assert!(dest.join(".odeion-bring-link").exists());

        for name in ["../elsewhere", ".odeion-bring-", "elsewhere"] {
            clear_parts(&dest, name).await;
            clear_parts(dir.path(), name).await;
        }
        assert!(elsewhere.join("1.part").exists(), "not a part folder's name");

        std::fs::remove_file(dest.join(".odeion-bring-abc/notes.part.txt")).unwrap();
        clear_parts(&dest, ".odeion-bring-abc").await;
        assert!(!dest.join(".odeion-bring-abc").exists(), "empty: gone");
    }

    /// A finished download gets its name only when no file has it: a file that came to be
    /// there since the plan stays as it is, and the download is dropped.
    #[tokio::test]
    async fn a_finished_download_does_not_replace_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let (part, target) = (dir.path().join(".Song.flac.part"), dir.path().join("Song.flac"));
        std::fs::write(&part, b"from the homeserver").unwrap();
        place_new(&part, &target).await.unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"from the homeserver");
        assert!(!part.exists());
        std::fs::write(&part, b"another file of that name").unwrap();
        let e = place_new(&part, &target).await.unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&target).unwrap(), b"from the homeserver");
        assert!(!part.exists(), "the download is dropped");
    }
}
