//! Copying the library to the homeserver's private storage (TASK-copy-library-folders): folders,
//! albums, songs or everything, as jobs that run in the background one at a time, wait their turn
//! in a queue, can be paused, cancelled and retried, and carry on after a restart.
//!
//! - **Kept** in the setting [`JOBS_KEY`]: the job running or paused now, the queue and the last
//!   job's summary. A job keeps what was picked, not the file list: that is read again from the
//!   library (by path) and `after` is the last path it got through, so a pause or a restart carries
//!   on with the files after it, also when the library's list changed meanwhile.
//! - **Guarded** as every copy is (copy_guard.rs): the disk's reserve before each file, failures
//!   in a row, audio only, nothing half written. A file that can't be read is a failure.
//! - **Seen** through `GET /v1/pubky/files/jobs` and the owner's `jobs` events.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::types::{ApiEvent, JobError, JobsView, UploadProgress};
use crate::{ApiError, ApiResult, NodeState};

/// Where the copy jobs are kept between restarts.
pub const JOBS_KEY: &str = "files.copy_jobs";

/// Errors a job keeps to show (all are counted).
const KEEP_ERRORS: usize = 50;
/// How often a running job is saved and announced.
const TICK: std::time::Duration = std::time::Duration::from_secs(2);
/// The window `speed_bps` is measured over.
const SPEED_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);
/// How long the homeserver's list counts as known for the folder tree.
const LIST_FRESH: std::time::Duration = std::time::Duration::from_secs(300);

/// What a copy job takes: as `POST /v1/pubky/files/upload` gives it, or the failed files of an
/// earlier job (a retry).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Selection {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub track_ids: Vec<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album_id: Option<i64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all: bool,
    /// A retry: these files, with their sizes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<(String, u64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopyJob {
    pub progress: UploadProgress,
    pub selection: Selection,
    /// The last file (by path, the selection's order) it got through: it carries on after it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    /// Retry failed made a job of this one's files already.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    retried: bool,
    /// The files that failed, for Retry failed.
    #[serde(default)]
    pub failed_files: Vec<(String, u64)>,
    /// Bytes sent (not skipped), and samples of it for the speed.
    #[serde(default)]
    sent_bytes: u64,
    #[serde(skip)]
    samples: VecDeque<(Instant, u64)>,
}

/// The node's copy jobs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Queue {
    pub current: Option<CopyJob>,
    #[serde(default)]
    pub queued: VecDeque<CopyJob>,
    pub last: Option<CopyJob>,
    /// Finished jobs of this run, for `GET …/upload/{id}` after the next one finished.
    #[serde(skip)]
    history: VecDeque<UploadProgress>,
    #[serde(skip)]
    runner: bool,
    /// Loaded after a restart, held until what the stopped node left is settled.
    #[serde(skip)]
    held: bool,
    #[serde(skip)]
    ticked: Option<Instant>,
}

/// What the homeserver had when last listed: content-hash prefixes (the folder tree's "already
/// there").
#[derive(Debug, Default)]
pub struct Listed {
    at: Option<Instant>,
    hashes: HashSet<String>,
    refreshing: bool,
}

/// On `NodeState`: the queue, its save order, and the homeserver list the tree counts with.
#[derive(Debug, Default)]
pub struct CopyJobs {
    pub queue: Mutex<Queue>,
    persist: Mutex<()>,
    /// Held while a files job is let in (a copy, an import, bringing files here): its check
    /// and its start happen together, so two never start side by side.
    admission: Mutex<()>,
    pub listed: Mutex<Listed>,
    /// Tests: what the quota leaves on the homeserver (bytes), in place of asking it.
    pub quota_seam: Mutex<Option<u64>>,
}

fn now() -> i64 {
    pm_library::db::now_unix()
}

// ---- keeping and telling ----------------------------------------------------------------------

/// Writes the queue to the library. Taken in order: a later state never loses to an earlier one.
fn save(s: &NodeState) {
    let _order = s.copy_jobs.persist.lock().expect("copy jobs persist");
    let json = {
        let q = s.copy_jobs.queue.lock().expect("copy jobs");
        serde_json::to_string(&*q).expect("serializes")
    };
    let _ = s
        .library
        .lock()
        .expect("library poisoned")
        .set_setting(JOBS_KEY, &json);
}

/// What the job chip shows: the copy running or paused, else an import or bring job running; the
/// queue; the last copy's summary.
pub fn view(s: &NodeState) -> JobsView {
    let other = s
        .upload_jobs
        .lock()
        .expect("upload jobs")
        .values()
        .find(|j| !j.finished)
        .cloned()
        .map(|mut j| {
            j.state = "running".into();
            j
        });
    let q = s.copy_jobs.queue.lock().expect("copy jobs");
    JobsView {
        current: q.current.as_ref().map(|j| j.progress.clone()).or(other),
        queued: q.queued.iter().map(|j| j.progress.clone()).collect(),
        last: q.last.as_ref().map(|j| j.progress.clone()),
    }
}

/// Tells the owner's pages (`jobs` events).
pub fn notify(s: &NodeState) {
    let _ = s.events.send(ApiEvent::Jobs {
        jobs: Box::new(view(s)),
    });
}

/// Saves and tells, at most every [`TICK`] while a job runs.
fn tick(s: &NodeState) {
    let due = {
        let mut q = s.copy_jobs.queue.lock().expect("copy jobs");
        let due = q.ticked.is_none_or(|t| t.elapsed() >= TICK);
        if due {
            q.ticked = Some(Instant::now());
        }
        due
    };
    if due {
        save(s);
        notify(s);
    }
}

fn changed(s: &NodeState) {
    save(s);
    notify(s);
}

/// Changes the current job when it is `id`; whether it was.
fn update(s: &NodeState, id: &str, f: impl FnOnce(&mut CopyJob)) -> bool {
    let mut q = s.copy_jobs.queue.lock().expect("copy jobs");
    match q.current.as_mut() {
        Some(j) if j.progress.id == id => {
            f(j);
            true
        }
        _ => false,
    }
}

fn state_of(s: &NodeState, id: &str) -> Option<String> {
    let q = s.copy_jobs.queue.lock().expect("copy jobs");
    q.current
        .as_ref()
        .filter(|j| j.progress.id == id)
        .map(|j| j.progress.state.clone())
}

/// Taken while a files job checks that nothing else runs and starts (copy_jobs, `files::claim_job`,
/// `add_music::claim`).
pub fn admit(s: &NodeState) -> std::sync::MutexGuard<'_, ()> {
    s.copy_jobs.admission.lock().expect("admission")
}

/// A copy is running, paused or waiting: an import or bring job waits for it.
pub fn busy(s: &NodeState) -> Option<UploadProgress> {
    let q = s.copy_jobs.queue.lock().expect("copy jobs");
    // Held after a restart: a bring job resuming goes first, the copy waits for it
    if q.held {
        return None;
    }
    q.current
        .as_ref()
        .or(q.queued.front())
        .map(|j| j.progress.clone())
}

/// An import or bring job (upload_jobs) running: a copy waits for it.
fn other_running(s: &NodeState) -> bool {
    s.upload_jobs
        .lock()
        .expect("upload jobs")
        .values()
        .any(|j| !j.finished)
}

/// A copy job by id: current, queued, last or finished earlier in this run.
pub fn find(s: &NodeState, id: &str) -> Option<UploadProgress> {
    let q = s.copy_jobs.queue.lock().expect("copy jobs");
    q.current
        .iter()
        .chain(q.queued.iter())
        .chain(q.last.iter())
        .map(|j| &j.progress)
        .chain(q.history.iter())
        .find(|p| p.id == id)
        .cloned()
}

// ---- what a selection is --------------------------------------------------------------------

/// A folder's path as the library's file paths start (`…/` added).
fn as_prefix(p: &str) -> String {
    let p = p.trim();
    if p.ends_with('/') {
        p.to_string()
    } else {
        format!("{p}/")
    }
}

/// The library files a selection stands for, by path, each once, with its size.
pub(crate) fn resolve(s: &NodeState, sel: &Selection) -> Result<Vec<(PathBuf, u64)>, ApiError> {
    let lib = s.library.lock().expect("library poisoned");
    let mut out: Vec<(String, u64)> = sel.files.clone();
    if sel.all {
        let mut after = None;
        loop {
            let page = lib.list_tracks(after, 500)?;
            after = page.last().and_then(|t| t.id);
            let n = page.len();
            out.extend(
                page.into_iter()
                    .map(|t| (t.file.path.to_string_lossy().into_owned(), t.file.size_bytes)),
            );
            if n < 500 {
                break;
            }
        }
    }
    if let Some(a) = sel.album_id {
        out.extend(
            lib.album_tracks(a)?
                .into_iter()
                .map(|t| (t.file.path.to_string_lossy().into_owned(), t.file.size_bytes)),
        );
    }
    for id in &sel.track_ids {
        if let Some(t) = lib.get_track(pm_model::TrackId(*id))? {
            out.push((t.file.path.to_string_lossy().into_owned(), t.file.size_bytes));
        }
    }
    for p in &sel.paths {
        out.extend(
            lib.files_under(&as_prefix(p))?
                .into_iter()
                .map(|(path, size, _)| (path, size)),
        );
    }
    out.sort();
    out.dedup_by(|a, b| a.0 == b.0);
    Ok(out.into_iter().map(|(p, n)| (PathBuf::from(p), n)).collect())
}

/// A few words for what was picked.
fn label_of(s: &NodeState, sel: &Selection) -> String {
    let n = |k: usize, one: &str, many: &str| {
        if k == 1 {
            format!("1 {one}")
        } else {
            format!("{k} {many}")
        }
    };
    if sel.all {
        return "Your library".into();
    }
    if !sel.files.is_empty() {
        return format!("Retry: {}", n(sel.files.len(), "file", "files"));
    }
    if let Some(a) = sel.album_id {
        let title = s
            .library
            .lock()
            .expect("library poisoned")
            .album_tracks(a)
            .ok()
            .and_then(|t| t.into_iter().find_map(|t| t.album.map(|al| al.title)));
        return title.unwrap_or_else(|| "An album".into());
    }
    if let [one] = sel.paths.as_slice() {
        let name = one.trim_end_matches('/').rsplit('/').next().unwrap_or(one);
        return name.to_string();
    }
    let mut parts = Vec::new();
    if !sel.paths.is_empty() {
        parts.push(n(sel.paths.len(), "folder", "folders"));
    }
    if !sel.track_ids.is_empty() {
        parts.push(n(sel.track_ids.len(), "song", "songs"));
    }
    parts.join(", ")
}

/// A library file on a share or a drive (read through its source), not on this machine's disk.
fn is_remote(path: &Path) -> bool {
    crate::webdav::is_key(path) || crate::drives::is_key(path)
}

/// A library file's bytes: from the disk, or through its share or drive.
pub(crate) async fn read_file(s: &NodeState, path: &Path) -> std::io::Result<Vec<u8>> {
    if is_remote(path) {
        let resp = crate::webdav::fetch_library_file(s, path).await?;
        return resp
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(std::io::Error::other);
    }
    tokio::fs::read(path).await
}

// ---- what the homeserver has --------------------------------------------------------------

/// Keeps the homeserver's list for the tree (`present`): from a copy that listed it anyway, or a
/// refresh.
fn remember_listed(s: &NodeState, paths: impl Iterator<Item = String>) {
    let hashes = paths
        .filter_map(|p| {
            let name = p.rsplit('/').next()?.to_string();
            crate::files::split_remote_name(&name).map(|(h, _)| h.to_string())
        })
        .collect();
    let mut l = s.copy_jobs.listed.lock().expect("listed");
    l.hashes = hashes;
    l.at = Some(Instant::now());
}

/// The hash prefixes the homeserver has, when known; asks it again in the background when the
/// list is old or missing (`None` meanwhile, the first time).
pub(crate) fn listed_hashes(s: &Arc<NodeState>) -> Option<HashSet<String>> {
    // Without a homeserver session there is nothing to ask
    if crate::copy_guard::target_of(s).is_err() {
        return None;
    }
    let (known, stale) = {
        let mut l = s.copy_jobs.listed.lock().expect("listed");
        let stale = l.at.is_none_or(|t| t.elapsed() >= LIST_FRESH);
        let known = l.at.map(|_| l.hashes.clone());
        if stale && !l.refreshing {
            l.refreshing = true;
            (known, true)
        } else {
            (known, false)
        }
    };
    if stale {
        let s = s.clone();
        tokio::spawn(async move {
            if let Ok(t) = crate::copy_guard::target_of(&s) {
                if let Ok(list) = t.list().await {
                    remember_listed(&s, list.into_iter().map(|f| f.path));
                }
            }
            s.copy_jobs.listed.lock().expect("listed").refreshing = false;
        });
    }
    known
}

/// What a selection would send: files already there (by a remembered hash, at the same size)
/// are skipped and not counted in the bytes needed.
fn plan_of(
    s: &NodeState,
    files: &[(PathBuf, u64)],
    listed: Option<&HashMap<String, Option<u64>>>,
) -> (usize, u64, u64) {
    let (mut skipped, mut bytes_skipped, mut needed) = (0, 0, 0);
    for (p, n) in files {
        if crate::copy_guard::is_apple_double(p) {
            continue;
        }
        let there = listed.is_some_and(|present| {
            crate::files::known_hash(s, p).is_some_and(|h| {
                let remote = format!(
                    "{}{}",
                    pm_pubky::homeserver::PRIVATE_FILES_DIR,
                    crate::files::remote_name_for(&h, &crate::files::safe_file_name(p))
                );
                present.get(&remote).is_some_and(|sz| *sz == Some(*n))
            })
        });
        if there {
            skipped += 1;
            bytes_skipped += n;
        } else {
            needed += n;
        }
    }
    (skipped, bytes_skipped, needed)
}

// ---- starting -----------------------------------------------------------------------------

/// `POST /v1/pubky/files/upload {paths | track_ids | album_id | all, dry_run?}`: a copy job.
/// `202 {id, state: running | queued, …}`; a dry run `200` with the totals; `507 {error,
/// needed_bytes, free_bytes, …}` when it wouldn't fit (the disk's reserve, the homeserver quota).
pub(crate) async fn start(
    State(s): State<Arc<NodeState>>,
    Json(body): Json<crate::types::UploadRequest>,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    let sel = Selection {
        paths: body
            .paths
            .iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect(),
        track_ids: body.track_ids.clone(),
        album_id: body.album_id,
        all: body.all,
        files: Vec::new(),
    };
    let s2 = s.clone();
    let sel2 = sel.clone();
    let files = tokio::task::spawn_blocking(move || resolve(&s2, &sel2))
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))??;
    if files.is_empty() {
        return Err(ApiError::bad_request("nothing to upload"));
    }
    let mut progress = UploadProgress {
        id: crate::tidal::random_urlsafe(9),
        kind: "copy".into(),
        total: files.len(),
        bytes_total: files.iter().map(|(_, b)| b).sum(),
        label: Some(label_of(&s, &sel)),
        ..Default::default()
    };
    let target = crate::copy_guard::target_of(&s);
    let listed: Option<HashMap<String, Option<u64>>> = match &target {
        Ok(t) => t.list().await.ok().map(|l| {
            remember_listed(&s, l.iter().map(|f| f.path.clone()));
            l.into_iter().map(|f| (f.path, f.size)).collect()
        }),
        Err(_) => None,
    };
    let (skipped, bytes_skipped, needed) = {
        let (s2, files2) = (s.clone(), files.clone());
        tokio::task::spawn_blocking(move || plan_of(&s2, &files2, listed.as_ref()))
            .await
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };
    progress.skipped = skipped;
    progress.bytes_skipped = Some(bytes_skipped);
    let room = crate::copy_guard::room_of(&s);
    let mut fits = crate::copy_guard::plan(room.as_ref(), &mut progress, needed);
    // The owner's quota on the bundled homeserver, as `/v1/storage` reads it
    if let Some(left) = crate::storage::quota_left(&s).await {
        progress.free_bytes = Some(progress.free_bytes.map_or(left, |f| f.min(left)));
        if fits.is_ok() && needed > left {
            fits = Err(ApiError::new(
                StatusCode::INSUFFICIENT_STORAGE,
                format!(
                    "not enough room on your homeserver: {} needed, {} left in your quota",
                    crate::copy_guard::gb(needed),
                    crate::copy_guard::gb(left)
                ),
            ));
        }
    }
    let refused = |p: &UploadProgress, e: ApiError| {
        let mut v = serde_json::to_value(p).expect("serializes");
        v["error"] = e.message.into();
        (e.status, Json(v)).into_response()
    };
    if body.dry_run {
        progress.skipped = skipped;
        return Ok(match fits {
            Ok(()) => Json(progress).into_response(),
            Err(e) => refused(&progress, e),
        });
    }
    if let Err(e) = fits {
        return Ok(refused(&progress, e));
    }
    target?;
    // The plan's skips are found again as the job goes
    progress.skipped = 0;
    progress.bytes_skipped = None;
    let job = CopyJob {
        progress,
        selection: sel,
        after: None,
        retried: false,
        failed_files: Vec::new(),
        sent_bytes: 0,
        samples: VecDeque::new(),
    };
    let out = enqueue(&s, job);
    Ok((StatusCode::ACCEPTED, Json(out)).into_response())
}

/// Puts a job in line: it runs now when nothing else does, else it is queued.
fn enqueue(s: &Arc<NodeState>, mut job: CopyJob) -> UploadProgress {
    let admitted = admit(s);
    let other = other_running(s);
    let out = {
        let mut q = s.copy_jobs.queue.lock().expect("copy jobs");
        if q.current.is_none() && q.queued.is_empty() && !other {
            job.progress.state = "running".into();
            job.progress.started_at = Some(now());
            let p = job.progress.clone();
            q.current = Some(job);
            p
        } else {
            job.progress.state = "queued".into();
            let p = job.progress.clone();
            q.queued.push_back(job);
            p
        }
    };
    drop(admitted);
    changed(s);
    ensure_runner(s);
    out
}

// ---- running ------------------------------------------------------------------------------

/// Starts the node's runner unless it is going.
fn ensure_runner(s: &Arc<NodeState>) {
    {
        let mut q = s.copy_jobs.queue.lock().expect("copy jobs");
        if q.runner {
            return;
        }
        q.runner = true;
    }
    let s = s.clone();
    tokio::spawn(async move { run(s).await });
}

enum Next {
    Run(String),
    Finish(String),
    Wait,
    Exit,
}

/// The runner: the current job, then the queue in order, one at a time; it ends when nothing is
/// left to run (a paused job waits for Resume, which starts it again).
async fn run(s: Arc<NodeState>) {
    loop {
        let admitted = admit(&s);
        let other = other_running(&s);
        let mut started = false;
        let next = {
            let mut q = s.copy_jobs.queue.lock().expect("copy jobs");
            match q
                .current
                .as_ref()
                .map(|j| (j.progress.id.clone(), j.progress.state.clone()))
            {
                _ if q.held => {
                    q.runner = false;
                    Next::Exit
                }
                Some((_, st)) if st == "running" && other => Next::Wait,
                Some((id, st)) if st == "running" => Next::Run(id),
                Some((id, st)) if st == "cancelled" => Next::Finish(id),
                Some(_) => {
                    q.runner = false;
                    Next::Exit
                }
                None if q.queued.is_empty() => {
                    q.runner = false;
                    Next::Exit
                }
                None if other => Next::Wait,
                None => {
                    let mut j = q.queued.pop_front().expect("not empty");
                    j.progress.state = "running".into();
                    j.progress.started_at = Some(now());
                    let id = j.progress.id.clone();
                    q.current = Some(j);
                    started = true;
                    Next::Run(id)
                }
            }
        };
        drop(admitted);
        if started {
            changed(&s);
        }
        match next {
            Next::Run(id) => run_job(&s, &id).await,
            Next::Finish(id) => finish(&s, &id, "cancelled"),
            Next::Wait => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
            Next::Exit => return,
        }
    }
}

/// Ends the current job `id` as `state` and keeps its summary as the last one.
fn finish(s: &NodeState, id: &str, state: &str) {
    {
        let mut q = s.copy_jobs.queue.lock().expect("copy jobs");
        if q.current.as_ref().is_none_or(|j| j.progress.id != id) {
            return;
        }
        let mut j = q.current.take().expect("checked");
        let p = &mut j.progress;
        p.state = state.into();
        p.finished = true;
        p.cancelled = state == "cancelled";
        p.finished_at = Some(now());
        p.current = None;
        p.current_folder = None;
        p.speed_bps = None;
        p.eta_s = None;
        q.history.push_back(p.clone());
        if q.history.len() > 20 {
            q.history.pop_front();
        }
        q.last = Some(j);
    }
    changed(s);
}

/// Stops the current job `id` early, saying why (`state` failed). `left`: the files it never
/// tried, which Retry sends with the failed ones.
fn stop(s: &NodeState, id: &str, why: String, full: bool, left: usize) {
    update(s, id, |j| {
        j.progress.failed_retryable = j.failed_files.len() + left;
        j.progress.errors.push(why.clone().into());
        j.progress.errors_count += 1;
        j.progress.stopped_reason = Some(why.clone());
        j.progress.full = full;
        j.progress.error = Some(why);
    });
    finish(s, id, "failed");
}

/// What became of one file.
enum Outcome {
    Sent,
    Skipped,
    NotAudio,
    Failed(String),
    Full(String),
    Space(String),
}

/// Waits for the homeserver session (after a restart it comes back in the background): up to
/// two minutes, or until the job is paused or cancelled.
async fn target_for(s: &Arc<NodeState>, id: &str) -> Option<Arc<dyn crate::copy_guard::Target>> {
    for _ in 0..120 {
        if let Ok(t) = crate::copy_guard::target_of(s) {
            return Some(t);
        }
        if state_of(s, id).as_deref() != Some("running") {
            return None;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    None
}

async fn run_job(s: &Arc<NodeState>, id: &str) {
    let Some((sel, after)) = ({
        let q = s.copy_jobs.queue.lock().expect("copy jobs");
        q.current
            .as_ref()
            .filter(|j| j.progress.id == id)
            .map(|j| (j.selection.clone(), j.after.clone()))
    }) else {
        return;
    };
    let files = {
        let s2 = s.clone();
        match tokio::task::spawn_blocking(move || resolve(&s2, &sel)).await {
            Ok(Ok(f)) => f,
            Ok(Err(e)) => return stop(s, id, e.message, false, 0),
            Err(e) => return stop(s, id, e.to_string(), false, 0),
        }
    };
    // Carries on after the last file it got through (the list is in path order)
    let start = after.as_deref().map_or(0, |a| {
        files.partition_point(|(p, _)| p.to_string_lossy().as_ref() <= a)
    });
    let left = files.len() - start;
    update(s, id, |j| {
        j.progress.total = files.len();
        j.progress.bytes_total = files.iter().map(|(_, b)| b).sum();
    });
    let Some(target) = target_for(s, id).await else {
        if state_of(s, id).as_deref() == Some("running") {
            stop(
                s,
                id,
                "sign in to your homeserver first (Settings › Pubky)".into(),
                false,
                left,
            );
        }
        return;
    };
    let mut present: HashMap<String, Option<u64>> = match target.list().await {
        Ok(l) => {
            remember_listed(s, l.iter().map(|f| f.path.clone()));
            l.into_iter().map(|f| (f.path, f.size)).collect()
        }
        Err(e) => {
            return stop(
                s,
                id,
                format!("the homeserver did not list its files: {e}"),
                false,
                left,
            )
        }
    };
    let room = crate::copy_guard::room_of(s);
    let mut streak = crate::copy_guard::Streak::default();
    for (i, (path, size)) in files.iter().enumerate().skip(start) {
        // Paused or cancelled: after the file in hand, nothing more
        if state_of(s, id).as_deref() != Some("running") {
            save(s);
            notify(s);
            return;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let folder = path.parent().map(|p| p.to_string_lossy().into_owned());
        update(s, id, |j| {
            j.progress.current = Some(name.clone());
            j.progress.current_folder = folder;
        });
        // A share's or drive's song was read as audio when the library scanned it
        let audio = if is_remote(path) {
            Ok(true)
        } else {
            crate::copy_guard::is_audio(path).await
        };
        let outcome = match audio {
            Ok(false) => Outcome::NotAudio,
            Err(e) => Outcome::Failed(e),
            Ok(true) => {
                match crate::files::copy_one(s, target.as_ref(), room.as_ref(), &mut present, path, &name)
                    .await
                {
                    Ok(true) => Outcome::Sent,
                    Ok(false) => Outcome::Skipped,
                    Err(e) if crate::add_music::is_full_error(&e) => Outcome::Full(e),
                    Err(e) if crate::copy_guard::is_space_stop(&e) => Outcome::Space(e),
                    Err(e) => Outcome::Failed(e),
                }
            }
        };
        let p = path.to_string_lossy().into_owned();
        let at = Instant::now();
        update(s, id, |j| {
            // A file the reserve stopped before was never tried: the job isn't past it
            if !matches!(outcome, Outcome::Space(_)) {
                j.after = Some(p.clone());
            }
            let pr = &mut j.progress;
            match &outcome {
                Outcome::Sent => {
                    pr.done += 1;
                    j.sent_bytes += size;
                }
                Outcome::Skipped => pr.skipped += 1,
                Outcome::NotAudio => pr.not_audio += 1,
                Outcome::Failed(e) | Outcome::Full(e) => {
                    pr.failed += 1;
                    pr.errors.push(JobError {
                        path: p.clone(),
                        reason: e.clone(),
                    });
                    pr.errors_count += 1;
                    let n = pr.errors.len();
                    if n > KEEP_ERRORS {
                        pr.errors.drain(..n - KEEP_ERRORS);
                    }
                    j.failed_files.push((p.clone(), *size));
                    pr.failed_retryable = j.failed_files.len();
                }
                Outcome::Space(_) => {}
            }
            // A file left out counts as gone through: the bar reaches the end
            if !matches!(outcome, Outcome::Space(_)) {
                pr.bytes_done += size;
            }
            j.samples.push_back((at, j.sent_bytes));
            while j
                .samples
                .front()
                .is_some_and(|(t, _)| at.duration_since(*t) > SPEED_WINDOW)
            {
                j.samples.pop_front();
            }
            let pr = &mut j.progress;
            if let (Some((t0, b0)), Some((t1, b1))) = (j.samples.front(), j.samples.back()) {
                let secs = t1.duration_since(*t0).as_secs_f64();
                if secs >= 1.0 {
                    let bps = ((b1 - b0) as f64 / secs) as u64;
                    pr.speed_bps = Some(bps);
                    pr.eta_s = (bps > 0).then(|| pr.bytes_total.saturating_sub(pr.bytes_done) / bps);
                }
            }
        });
        match outcome {
            Outcome::Full(e) => {
                crate::storage::note_full(s, true);
                let left = files.len() - i - 1;
                return stop(
                    s,
                    id,
                    format!("{e}; the files after it were not copied"),
                    true,
                    left,
                );
            }
            Outcome::Space(e) => return stop(s, id, e, false, files.len() - i),
            Outcome::Failed(e) => {
                if let Some(why) = streak.failed(&e) {
                    return stop(s, id, why, false, files.len() - i - 1);
                }
            }
            _ => streak.ok(),
        }
        tick(s);
    }
    finish(s, id, "finished");
}

// ---- controls -----------------------------------------------------------------------------

fn no_job() -> ApiError {
    ApiError::not_found("no such copy")
}

/// `POST /v1/pubky/files/upload/{id}/pause`: after the file in hand. It stays paused, also over a
/// restart, until Resume.
pub(crate) async fn pause(
    State(s): State<Arc<NodeState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> ApiResult<UploadProgress> {
    let p = {
        let mut q = s.copy_jobs.queue.lock().expect("copy jobs");
        if q.queued.iter().any(|j| j.progress.id == id) {
            return Err(ApiError::new(StatusCode::CONFLICT, "it hasn't started yet"));
        }
        let j = q
            .current
            .as_mut()
            .filter(|j| j.progress.id == id)
            .ok_or_else(no_job)?;
        if j.progress.state == "running" {
            j.progress.state = "paused".into();
            j.progress.speed_bps = None;
            j.progress.eta_s = None;
        }
        j.progress.clone()
    };
    changed(&s);
    Ok(Json(p))
}

/// `POST /v1/pubky/files/upload/{id}/resume`.
pub(crate) async fn resume(
    State(s): State<Arc<NodeState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> ApiResult<UploadProgress> {
    let p = {
        let mut q = s.copy_jobs.queue.lock().expect("copy jobs");
        let j = q
            .current
            .as_mut()
            .filter(|j| j.progress.id == id)
            .ok_or_else(no_job)?;
        if j.progress.state == "paused" {
            j.progress.state = "running".into();
            j.samples.clear();
        }
        j.progress.clone()
    };
    changed(&s);
    ensure_runner(&s);
    Ok(Json(p))
}

/// `POST /v1/pubky/files/upload/{id}/cancel` (also `…/jobs/{id}/cancel`): a running copy stops
/// after the file in hand, a paused one at once, a queued one leaves the queue. `None` when `id`
/// is no copy job (an import or a bring job).
pub(crate) fn cancel(s: &Arc<NodeState>, id: &str) -> Option<UploadProgress> {
    let out = {
        let mut q = s.copy_jobs.queue.lock().expect("copy jobs");
        if let Some(j) = q.current.as_mut().filter(|j| j.progress.id == id) {
            if !j.progress.finished {
                j.progress.state = "cancelled".into();
                j.progress.cancelled = true;
            }
            Some(j.progress.clone())
        } else if let Some(i) = q.queued.iter().position(|j| j.progress.id == id) {
            let mut j = q.queued.remove(i).expect("found");
            j.progress.state = "cancelled".into();
            j.progress.cancelled = true;
            j.progress.finished = true;
            q.history.push_back(j.progress.clone());
            Some(j.progress)
        } else {
            None
        }
    };
    if out.is_some() {
        changed(s);
        ensure_runner(s);
    }
    out
}

pub(crate) async fn cancel_route(
    State(s): State<Arc<NodeState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> ApiResult<UploadProgress> {
    cancel(&s, &id).map(Json).ok_or_else(no_job)
}

/// `POST /v1/pubky/files/upload/{id}/retry`: a new job, queued, of the files the last copy
/// couldn't send: those that failed and, when it stopped early (failures in a row, the homeserver
/// full, the disk's reserve), every file it never got to. Once per copy.
pub(crate) async fn retry(
    State(s): State<Arc<NodeState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<(StatusCode, Json<UploadProgress>), ApiError> {
    let (mut files, stopped, sel, after) = {
        let q = s.copy_jobs.queue.lock().expect("copy jobs");
        if q.current.as_ref().is_some_and(|j| j.progress.id == id) {
            return Err(ApiError::new(StatusCode::CONFLICT, "that copy is still going"));
        }
        let j = q
            .last
            .as_ref()
            .filter(|j| j.progress.id == id)
            .ok_or_else(no_job)?;
        if j.retried {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "that copy was retried already",
            ));
        }
        (
            j.failed_files.clone(),
            j.progress.state == "failed",
            j.selection.clone(),
            j.after.clone(),
        )
    };
    if stopped {
        let s2 = s.clone();
        let all = tokio::task::spawn_blocking(move || resolve(&s2, &sel))
            .await
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))??;
        files.extend(
            all.into_iter()
                .map(|(p, n)| (p.to_string_lossy().into_owned(), n))
                // The failed ones are all before it
                .filter(|(p, _)| after.as_deref().is_none_or(|a| p.as_str() > a)),
        );
    }
    if files.is_empty() {
        return Err(ApiError::new(StatusCode::CONFLICT, "nothing failed in that copy"));
    }
    {
        let mut q = s.copy_jobs.queue.lock().expect("copy jobs");
        if let Some(j) = q.last.as_mut().filter(|j| j.progress.id == id) {
            j.retried = true;
            j.progress.failed_retryable = 0;
        }
    }
    let sel = Selection {
        files,
        ..Default::default()
    };
    let job = CopyJob {
        progress: UploadProgress {
            id: crate::tidal::random_urlsafe(9),
            kind: "copy".into(),
            total: sel.files.len(),
            bytes_total: sel.files.iter().map(|(_, b)| b).sum(),
            label: Some(label_of(&s, &sel)),
            ..Default::default()
        },
        selection: sel,
        after: None,
        retried: false,
        failed_files: Vec::new(),
        sent_bytes: 0,
        samples: VecDeque::new(),
    };
    Ok((StatusCode::ACCEPTED, Json(enqueue(&s, job))))
}

/// `GET /v1/pubky/files/jobs`: `{current?, queued, last?}`.
pub(crate) async fn jobs(State(s): State<Arc<NodeState>>) -> ApiResult<JobsView> {
    Ok(Json(view(&s)))
}

/// `DELETE /v1/pubky/files/jobs/last`: the summary is dismissed.
pub(crate) async fn dismiss_last(State(s): State<Arc<NodeState>>) -> ApiResult<JobsView> {
    s.copy_jobs.queue.lock().expect("copy jobs").last = None;
    changed(&s);
    Ok(Json(view(&s)))
}

/// At start: the copy jobs a stopped node had, back in place (held: nothing runs before
/// [`release_after_restart`]). A running one then carries on from where it got, a paused one
/// waits for Resume, the queue after them.
pub fn resume_after_restart(s: &Arc<NodeState>) {
    let kept = s
        .library
        .lock()
        .expect("library poisoned")
        .get_setting(JOBS_KEY)
        .ok()
        .flatten()
        .and_then(|j| serde_json::from_str::<Queue>(&j).ok());
    let Some(kept) = kept else { return };
    // Cancelled just before the node stopped: it ends now, and doesn't hold the queue
    let cancelled = kept
        .current
        .as_ref()
        .filter(|j| j.progress.state == "cancelled")
        .map(|j| j.progress.id.clone());
    let go = kept
        .current
        .as_ref()
        .is_some_and(|j| j.progress.state == "running")
        || ((kept.current.is_none() || cancelled.is_some()) && !kept.queued.is_empty());
    {
        let mut q = s.copy_jobs.queue.lock().expect("copy jobs");
        q.current = kept.current;
        q.queued = kept.queued;
        q.last = kept.last;
        q.held = true;
    }
    if let Some(id) = cancelled {
        finish(s, &id, "cancelled");
    }
    if go {
        tracing::info!("a copy to the homeserver carries on after the restart");
    }
}

/// After the start's settling (what a stopped node half wrote is cleaned up first, never while a
/// copy writes the same paths): the kept copies run.
pub fn release_after_restart(s: &Arc<NodeState>) {
    s.copy_jobs.queue.lock().expect("copy jobs").held = false;
    ensure_runner(s);
}

// ---- the folder tree --------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TreeQuery {
    /// The folder to open; none: the roots.
    #[serde(default)]
    pub path: Option<String>,
    /// Paging: the children after this name.
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// One folder of the tree, counted from the library (never a walk of the disk).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct TreeNode {
    pub path: String,
    pub name: String,
    pub files: u64,
    pub bytes: u64,
    /// Files whose content the homeserver has; left out while its list isn't known yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub present: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub present_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub present_pending: bool,
    /// No folders inside.
    pub leaf: bool,
}

/// `GET /v1/library/tree`: one level.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct TreeLevel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub children: Vec<TreeNode>,
    pub files: u64,
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub present: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub present_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub present_pending: bool,
    /// More children after these: ask again with `after=<next>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

/// Counts of files under one folder.
#[derive(Default)]
struct Tally {
    files: u64,
    bytes: u64,
    present: u64,
    present_bytes: u64,
    folders: bool,
}

impl Tally {
    fn add(&mut self, size: u64, hash: &Option<String>, there: Option<&HashSet<String>>) {
        self.files += 1;
        self.bytes += size;
        if hash
            .as_ref()
            .is_some_and(|h| there.is_some_and(|t| t.contains(h)))
        {
            self.present += 1;
            self.present_bytes += size;
        }
    }

    fn node(&self, path: String, name: String, known: bool) -> TreeNode {
        TreeNode {
            path,
            name,
            files: self.files,
            bytes: self.bytes,
            present: known.then_some(self.present),
            present_bytes: known.then_some(self.present_bytes),
            present_pending: !known,
            leaf: !self.folders,
        }
    }
}

/// The library's roots: its folders, WebDAV shares and cloud-drive folders, as (path, name).
fn roots(s: &NodeState) -> Result<Vec<(String, String)>, ApiError> {
    let mut out: Vec<(String, String)> = crate::folder_list_now(s)?
        .into_iter()
        .map(|(p, _)| (p.clone(), p))
        .collect();
    for sh in s.webdav.read().expect("webdav").iter() {
        out.push((format!("{}{}", crate::webdav::SCHEME, sh.id), sh.name.clone()));
    }
    for r in s.drives.read().expect("drives").roots.iter() {
        out.push((r.key().to_string_lossy().into_owned(), r.name.clone()));
    }
    // By name, the order `after` pages in
    out.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    Ok(out)
}

/// `GET /v1/library/tree?path=&after=&limit=`: the folders inside `path` (the roots without
/// one), each with its files and bytes from the library and how many the homeserver has.
pub(crate) async fn tree(
    State(s): State<Arc<NodeState>>,
    axum::extract::Query(q): axum::extract::Query<TreeQuery>,
) -> ApiResult<TreeLevel> {
    let there = listed_hashes(&s);
    let s2 = s.clone();
    let out = tokio::task::spawn_blocking(move || level(&s2, &q, there.as_ref()))
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))??;
    Ok(Json(out))
}

fn level(s: &NodeState, q: &TreeQuery, there: Option<&HashSet<String>>) -> Result<TreeLevel, ApiError> {
    let known = there.is_some();
    let limit = q.limit.unwrap_or(200).clamp(1, 1000);
    let mut whole = Tally::default();
    let mut kids: Vec<TreeNode> = Vec::new();
    match q.path.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        None => {
            for (path, name) in roots(s)? {
                let rows = s
                    .library
                    .lock()
                    .expect("library poisoned")
                    .files_under(&as_prefix(&path))?;
                let mut t = Tally::default();
                for (p, size, h) in &rows {
                    t.add(*size, h, there);
                    whole.add(*size, h, there);
                    t.folders |= p[as_prefix(&path).len()..].contains('/');
                }
                kids.push(t.node(path, name, known));
            }
        }
        Some(path) => {
            let prefix = as_prefix(path);
            let rows = s.library.lock().expect("library poisoned").files_under(&prefix)?;
            let mut by: std::collections::BTreeMap<String, Tally> = Default::default();
            for (p, size, h) in &rows {
                whole.add(*size, h, there);
                if let Some((name, rest)) = p[prefix.len()..].split_once('/') {
                    let t = by.entry(name.to_string()).or_default();
                    t.add(*size, h, there);
                    t.folders |= rest.contains('/');
                }
            }
            kids = by
                .into_iter()
                .map(|(name, t)| t.node(format!("{prefix}{name}"), name, known))
                .collect();
        }
    }
    if let Some(after) = q.after.as_deref() {
        kids.retain(|k| k.name.as_str() > after);
    }
    let next = (kids.len() > limit).then(|| kids[limit - 1].name.clone());
    kids.truncate(limit);
    let w = whole.node(String::new(), String::new(), known);
    Ok(TreeLevel {
        path: q.path.clone(),
        children: kids,
        files: w.files,
        bytes: w.bytes,
        present: w.present,
        present_bytes: w.present_bytes,
        present_pending: w.present_pending,
        next,
    })
}
