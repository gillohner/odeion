//! The "Identify library" job (steward identify brief), a ladder per song, cheapest first: the
//! IDs in the tags (at scan time), the folder as an album (one MusicBrainz release for the whole
//! folder, see [`pm_library::album_fit`]), a Chromaprint fingerprint (cached by file key, decoded
//! on every core) looked up on AcoustID (3 requests a second) and decided folder by folder with
//! the folder as a release, and last a MusicBrainz search by the tags. What stays ambiguous, or
//! where two methods disagree, becomes suggestions for the owner. At the end of every run, songs
//! waiting for review whose stored evidence is clear on its own are settled as `auto`
//! ([`pm_library::auto_settle`]): used in the node, never written into files until the owner
//! confirms them. The job resumes: tracks already looked at are skipped unless `retry` is set.
//! It also runs by itself: after a scan that added or changed songs ([`after_scan`]), once a week
//! for "not found" songs whose kept answers expired, and at start for songs older rules left
//! waiting ([`spawn_recheck`]); "Match new songs automatically" (on by default) turns the first
//! two off.

use crate::state::NodeState;
use crate::{ApiError, ApiResult};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use futures_util::StreamExt;
use pm_library::album_fit::{
    fit_release, folder_album, release_candidates_ranked, release_query, settle_editions, AlbumFit,
    FolderAlbum, FolderFile, Matched,
};
use pm_library::identify::{
    best_folder_releases, decide_with_tags, AcoustIdResult, Candidate, Decision, FileFacts,
};
use pm_library::{IdentityCounts, Library, SuggestionRow};
use pm_model::{Confidence, Mbid, MusicUri, Track};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Default)]
pub struct AcoustIdSettings {
    pub api_key: String,
    pub base_url: String,
    /// The key from the config, env or build; a key entered in Settings (`acoustid.api_key`)
    /// is used instead, and clearing it goes back to this one.
    pub configured: String,
}

/// A key entered in Settings › Library (second review: it needed a config-file edit).
pub const KEY_SETTING: &str = "acoustid.api_key";

/// Where the current (or last) run stands.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct IdentifyProgress {
    pub running: bool,
    pub total: usize,
    pub done: usize,
    pub identified: usize,
    pub suggested: usize,
    pub unknown: usize,
    pub errors: usize,
    /// This run's identified songs by how (`album_fit`, `fingerprint`, `fingerprint+release`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub identified_by: BTreeMap<String, usize>,
    /// Songs waiting for review (this run's or earlier ones') its end settled automatically, by
    /// rule (`R1`, `R1b`, `R2`, `R3`); not in `identified`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub settled: BTreeMap<String, usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// AcoustID refused the key during this run (why, in its words): nothing more was
    /// fingerprinted. Cleared when the key changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acoustid_refused: Option<String>,
    /// Why the run ended before its last song (the next run carries on from there).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    /// The node stopped while this run went on (it never finished): the next run carries on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub interrupted: bool,
    /// What the run had to do, and what it took from earlier runs instead.
    #[serde(default, skip_serializing_if = "RunWork::is_empty")]
    pub work: RunWork,
}

/// A run's work: what it read, decoded and asked, and what it reused from earlier runs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RunWork {
    /// Files read for their content key (2 MiB each).
    pub file_reads: usize,
    /// Content keys taken from the library (the file's size and time unchanged).
    pub file_keys_reused: usize,
    /// Files decoded for a fingerprint.
    pub decoded: usize,
    /// AcoustID lookups sent.
    pub acoustid_asked: usize,
    /// AcoustID answers taken from the library.
    pub lookups_reused: usize,
    /// MusicBrainz requests sent.
    pub mb_asked: usize,
    /// MusicBrainz answers taken from the library.
    pub mb_reused: usize,
    /// Files known not to fingerprint (their audio, unchanged), not decoded again.
    #[serde(default)]
    pub failures_reused: usize,
}

impl RunWork {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

fn work(s: &NodeState, f: impl FnOnce(&mut RunWork)) {
    f(&mut s.identify.lock().expect("identify lock").work);
}

/// How a run goes: `retry` looks again at "not found" songs; the pacing is MusicBrainz's and
/// AcoustID's (tests and the replay benchmark pace nothing, see [`RunOptions::quick`]).
#[derive(Debug, Clone)]
pub struct RunOptions {
    pub retry: bool,
    pub mb_interval: std::time::Duration,
    pub acoustid_interval: std::time::Duration,
    /// The most MusicBrainz answers kept (default [`Library::IDENTIFY_MB_MAX`]); held when
    /// the run ends, not only when the next one starts.
    pub mb_cache_max: Option<i64>,
    /// Only songs waiting for review that older rules checked (the start's automatic run): no
    /// new song is fingerprinted, and IDs are written only for these.
    pub outdated_only: bool,
    /// Only these songs (the weekly retry of "not found" songs).
    pub only: Option<Vec<i64>>,
    /// AcoustID answers kept from before this time are asked again (the weekly retry); the
    /// fingerprint itself is reused.
    pub refresh_before: Option<i64>,
}

impl RunOptions {
    pub fn new(retry: bool) -> Self {
        Self {
            retry,
            mb_interval: std::time::Duration::from_millis(1000),
            acoustid_interval: std::time::Duration::from_millis(340),
            mb_cache_max: None,
            outdated_only: false,
            only: None,
            refresh_before: None,
        }
    }

    /// The weekly retry: these "not found" songs, their AcoustID answers from before `before`
    /// asked again.
    pub fn weekly(songs: Vec<i64>, before: i64) -> Self {
        Self {
            only: Some(songs),
            refresh_before: Some(before),
            ..Self::new(true)
        }
    }

    /// The start's automatic run: [`Self::outdated_only`].
    pub fn recheck() -> Self {
        Self {
            outdated_only: true,
            ..Self::new(false)
        }
    }

    /// Unpaced, against mock services only (tests, the replay benchmark).
    pub fn quick(retry: bool) -> Self {
        Self {
            retry,
            mb_interval: std::time::Duration::ZERO,
            acoustid_interval: std::time::Duration::ZERO,
            mb_cache_max: None,
            outdated_only: false,
            only: None,
            refresh_before: None,
        }
    }
}

/// Where identification stands, in one word for the Identify card (`IdentifyStatus::phase`):
/// - `empty`: the library has no songs;
/// - `new`: songs nobody has checked yet (never run, or songs added since);
/// - `running`;
/// - `interrupted`: the node stopped during the last run;
/// - `failed`: the last run stopped early on an error (`progress.stopped` says why);
/// - `needs_review`: finished, and matches wait for the owner (`counts.suggested`);
/// - `ready`: finished, nothing to review.
pub fn phase(p: &IdentifyProgress, c: &IdentityCounts) -> &'static str {
    if p.running {
        "running"
    } else if c.total == 0 {
        "empty"
    } else if p.interrupted {
        "interrupted"
    } else if p.stopped.is_some() {
        "failed"
    } else if c.suggested > 0 {
        "needs_review"
    } else if c.unchecked > 0 {
        "new"
    } else {
        "ready"
    }
}

/// The last run's progress, kept in the library's settings: after a restart the card still knows
/// how it ended, or that it never did.
const LAST_RUN_KEY: &str = "identify.last_run";

fn remember_run(s: &NodeState, p: &IdentifyProgress) {
    let Ok(json) = serde_json::to_string(p) else {
        return;
    };
    if let Err(e) = s
        .library
        .lock()
        .expect("library poisoned")
        .set_setting(LAST_RUN_KEY, &json)
    {
        tracing::debug!("identify: the run's progress not kept: {e}");
    }
}

/// The run this process knows, else the last one the library remembers (a run it remembers as
/// going on was cut off by a restart).
fn progress_now(s: &NodeState) -> IdentifyProgress {
    let p = s.identify.lock().expect("identify lock").clone();
    if p.running || p.started_at.is_some() {
        return p;
    }
    let kept = s
        .library
        .lock()
        .expect("library poisoned")
        .get_setting(LAST_RUN_KEY)
        .ok()
        .flatten()
        .and_then(|j| serde_json::from_str::<IdentifyProgress>(&j).ok());
    let Some(mut kept) = kept else { return p };
    if kept.running {
        kept.running = false;
        kept.interrupted = true;
    }
    // Kept for this process: a key refusal or a run started meanwhile replace it as before
    let mut now = s.identify.lock().expect("identify lock");
    if now.running || now.started_at.is_some() {
        return now.clone();
    }
    kept.acoustid_refused = now.acoustid_refused.take().or(kept.acoustid_refused);
    *now = kept.clone();
    kept
}

/// `GET /v1/library/identify`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentifyStatus {
    pub progress: IdentifyProgress,
    pub counts: IdentityCounts,
    /// Where identification stands, see [`phase`].
    #[serde(default)]
    pub phase: String,
    /// Whether an AcoustID key is configured (fingerprint lookups).
    pub acoustid: bool,
    /// Whether that key was entered on the page (it can be changed or removed there).
    pub acoustid_own: bool,
    /// The key's state for the Identify card: `missing` (none), `refused` (the last run's
    /// AcoustID answer, in `acoustid_detail`) or `ok`.
    pub acoustid_state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acoustid_detail: Option<String>,
    /// "Write IDs into my files" (off by default).
    pub write_ids: bool,
    /// "Match new songs automatically" (on by default): a run after each scan that added or
    /// changed songs, and the weekly retry of "not found" songs.
    #[serde(default)]
    pub auto_match: bool,
}

/// `POST /v1/library/identify/key/check`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct KeyCheck {
    /// The key to ask AcoustID about; none checks the key in use.
    #[serde(default)]
    pub acoustid_key: Option<String>,
}

/// What a check found: `ok`, and the words for the page either way.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyCheckResult {
    pub ok: bool,
    pub detail: String,
}

/// Where to get a key, as the page and the answers say it.
pub const KEY_FROM: &str = "get a free one at acoustid.org/new-application";

#[derive(Debug, Clone, Default, Deserialize)]
pub struct IdentifyStart {
    /// Look again at tracks a previous run found nothing for.
    #[serde(default)]
    pub retry: bool,
}

/// `PUT /v1/library/identify/settings`.
#[derive(Debug, Clone, Deserialize)]
pub struct IdentifySettings {
    #[serde(default)]
    pub write_ids: Option<bool>,
    /// "Match new songs automatically".
    #[serde(default)]
    pub auto_match: Option<bool>,
    /// Your own AcoustID key; empty removes it.
    #[serde(default)]
    pub acoustid_key: Option<String>,
}

/// `POST /v1/library/identify/write`: what was written.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WriteReport {
    pub written: usize,
    /// Files left as they were, with the reason (format, read-only share, …).
    pub skipped: Vec<WriteSkip>,
    /// Where the tag backups are.
    pub backup_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteSkip {
    pub track_id: i64,
    pub path: String,
    pub reason: String,
}

const WRITE_IDS_KEY: &str = "library.write_ids";

/// "Match new songs automatically": `false` turns it off; on unless so.
const AUTO_KEY: &str = "identify.auto";

pub(crate) fn auto_match_on(s: &NodeState) -> bool {
    s.library
        .lock()
        .expect("library poisoned")
        .get_setting(AUTO_KEY)
        .ok()
        .flatten()
        .is_none_or(|v| v != "false")
}

fn write_ids_on(s: &NodeState) -> bool {
    s.library
        .lock()
        .expect("library poisoned")
        .get_setting(WRITE_IDS_KEY)
        .ok()
        .flatten()
        .is_some_and(|v| v == "true")
}

/// Tag backups live next to the node's other data.
pub(crate) fn backup_dir(s: &NodeState) -> PathBuf {
    s.art_dir
        .parent()
        .map(|p| p.join("tag-backups"))
        .unwrap_or_else(|| PathBuf::from("tag-backups"))
}

/// Write the IDs identification found into the files (all such tracks, or
/// `only`); each file's ID tags are backed up first. Blocking.
pub fn write_found_ids(s: &NodeState, only: Option<&[i64]>) -> Result<WriteReport, ApiError> {
    let found = s
        .library
        .lock()
        .expect("library poisoned")
        .identified_not_tagged()?;
    let dir = backup_dir(s);
    let mut report = WriteReport {
        backup_dir: dir.display().to_string(),
        ..Default::default()
    };
    for (t, release) in found {
        let id = t.id.map(|i| i.0).unwrap_or_default();
        if only.is_some_and(|o| !o.contains(&id)) {
            continue;
        }
        let Some((MusicUri::Recording { mbid }, _)) = t.canonical.clone() else {
            continue;
        };
        // One writer at a time, and the song still what it was when listed (a correction made
        // meanwhile wins)
        let _one = id_writes();
        if !still_matched(s, id, &mbid) {
            continue;
        }
        let ids = pm_library::tagwrite::Ids {
            recording: mbid,
            release,
            acoustid: t.acoustid.clone(),
        };
        match pm_library::tagwrite::write_ids(&t.file.path, &ids, &dir) {
            Ok(_) => {
                report.written += 1;
                let _ = s.library.lock().expect("library poisoned").set_identify_state(
                    id,
                    "identified",
                    Some("written into the file"),
                );
            }
            Err(e) => report.skipped.push(WriteSkip {
                track_id: id,
                path: t.file.path.display().to_string(),
                reason: e.to_string(),
            }),
        }
    }
    Ok(report)
}

/// `PUT /v1/library/identify/settings {write_ids}`.
pub(crate) async fn put_settings(
    State(s): State<Arc<NodeState>>,
    Json(body): Json<IdentifySettings>,
) -> ApiResult<IdentifyStatus> {
    if let Some(w) = body.write_ids {
        s.library
            .lock()
            .expect("library poisoned")
            .set_setting(WRITE_IDS_KEY, if w { "true" } else { "false" })?;
    }
    if let Some(a) = body.auto_match {
        s.library
            .lock()
            .expect("library poisoned")
            .set_setting(AUTO_KEY, if a { "true" } else { "false" })?;
    }
    if let Some(k) = body.acoustid_key {
        let k = k.trim().to_string();
        if k.len() > 64 || k.chars().any(|c| !c.is_ascii_alphanumeric()) {
            return Err(ApiError::bad_request(
                "That doesn't look like an AcoustID key (letters and digits, from acoustid.org/new-application)",
            ));
        }
        // One lookup with the new key: one AcoustID doesn't know is refused here, not found out
        // song by song (an AcoustID out of reach doesn't stop the key being saved)
        if !k.is_empty() {
            let base = s.acoustid.read().expect("acoustid lock").base_url.clone();
            if let Some(why) = acoustid_client(&s, &k, &base)?.key_refused().await {
                return Err(ApiError::bad_request(format!(
                    "AcoustID refused this key ({why}). Copy the API key of your application on acoustid.org, or register one at acoustid.org/new-application"
                )));
            }
        }
        {
            let lib = s.library.lock().expect("library poisoned");
            if k.is_empty() {
                lib.delete_setting(KEY_SETTING)?;
            } else {
                lib.set_setting(KEY_SETTING, &k)?;
            }
        }
        {
            let mut a = s.acoustid.write().expect("acoustid lock");
            a.api_key = if k.is_empty() { a.configured.clone() } else { k };
        }
        // After the key changed: a run's refusal of the old key, set meanwhile, is cleared too
        s.identify.lock().expect("identify lock").acoustid_refused = None;
    }
    Ok(Json(status(&s)?))
}

/// `POST /v1/library/identify/key/check {acoustid_key?}`: asks AcoustID whether it takes the
/// key (one lookup), saving nothing; the key in use when none is given. Always 200 `{ok, detail}`:
/// a refused key, a missing one and an AcoustID out of reach each in their own words.
pub(crate) async fn check_key(
    State(s): State<Arc<NodeState>>,
    Json(body): Json<KeyCheck>,
) -> ApiResult<KeyCheckResult> {
    let answer = |ok: bool, detail: String| Ok(Json(KeyCheckResult { ok, detail }));
    let (key, base) = {
        let a = s.acoustid.read().expect("acoustid lock");
        (
            body.acoustid_key
                .as_deref()
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .unwrap_or(&a.api_key)
                .to_string(),
            a.base_url.clone(),
        )
    };
    if key.is_empty() {
        return answer(false, format!("Key missing: {KEY_FROM}."));
    }
    if key.len() > 64 || key.chars().any(|c| !c.is_ascii_alphanumeric()) {
        return answer(
            false,
            "That doesn't look like an AcoustID key (letters and digits, from acoustid.org/new-application)"
                .into(),
        );
    }
    let client = acoustid_client(&s, &key, &base)?;
    // AcoustID looks at the key before the fingerprint: only its "invalid API key" (code 4) refuses
    // the key; any other answer of its own (the dummy fingerprint draws "invalid fingerprint",
    // code 3) means it took the key. Only no answer at all is "couldn't reach"
    match client.lookup("AQAAAA", 30).await {
        Err(pm_library::LibraryError::AcoustIdKey(why)) => answer(
            false,
            format!("AcoustID refused this key ({why}). Check it, or {KEY_FROM}."),
        ),
        Ok(_) => answer(true, "OK: AcoustID accepts this key.".into()),
        Err(pm_library::LibraryError::AcoustId(m)) if m.contains("(code ") => {
            answer(true, "OK: AcoustID accepts this key.".into())
        }
        Err(e) => answer(false, format!("Couldn't reach AcoustID: {e}. Try again later.")),
    }
}

/// `POST /v1/library/identify/write`: write the IDs found so far into the files now.
pub(crate) async fn write_now(State(s): State<Arc<NodeState>>) -> ApiResult<WriteReport> {
    let st = s.clone();
    let r = tokio::task::spawn_blocking(move || write_found_ids(&st, None))
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))??;
    Ok(Json(r))
}

/// One suggestion with the track it is for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestionView {
    #[serde(flatten)]
    pub row: SuggestionRow,
    pub track: Track,
}

fn status(s: &NodeState) -> Result<IdentifyStatus, ApiError> {
    let (counts, acoustid_own) = {
        let lib = s.library.lock().expect("library poisoned");
        let own = lib.get_setting(KEY_SETTING)?.is_some_and(|k| !k.is_empty());
        (lib.identity_counts()?, own)
    };
    // Its own statement: the run counts while it holds the library, so never hold both
    let progress = progress_now(s);
    let acoustid = !s.acoustid.read().expect("acoustid lock").api_key.is_empty();
    let (acoustid_state, acoustid_detail) = if !acoustid {
        ("missing", None)
    } else if let Some(why) = &progress.acoustid_refused {
        ("refused", Some(why.clone()))
    } else {
        ("ok", None)
    };
    Ok(IdentifyStatus {
        phase: phase(&progress, &counts).into(),
        progress,
        counts,
        acoustid,
        acoustid_own,
        acoustid_state: acoustid_state.into(),
        acoustid_detail,
        write_ids: write_ids_on(s),
        auto_match: auto_match_on(s),
    })
}

pub(crate) async fn get_status(State(s): State<Arc<NodeState>>) -> ApiResult<IdentifyStatus> {
    Ok(Json(status(&s)?))
}

/// `POST /v1/library/identify {retry?}`: start the job (no-op while running).
pub(crate) async fn start(
    State(s): State<Arc<NodeState>>,
    body: Option<Json<IdentifyStart>>,
) -> ApiResult<IdentifyStatus> {
    launch(&s, RunOptions::new(body.map(|b| b.retry).unwrap_or(false)));
    Ok(Json(status(&s)?))
}

/// Start a run in the background; false (and nothing started) while one is going.
fn launch(s: &Arc<NodeState>, opts: RunOptions) -> bool {
    {
        let mut p = s.identify.lock().expect("identify lock");
        if p.running {
            return false;
        }
        *p = IdentifyProgress {
            running: true,
            started_at: Some(pm_library::db::now_unix()),
            ..Default::default()
        };
    }
    let started = s.identify.lock().expect("identify lock").clone();
    remember_run(s, &started);
    let st = s.clone();
    tokio::spawn(async move {
        if let Err(e) = run_with(&st, opts).await {
            tracing::warn!("identify: {}", e.message);
            st.identify.lock().expect("identify lock").stopped = Some(e.message);
        }
        let ended = {
            let mut p = st.identify.lock().expect("identify lock");
            p.running = false;
            p.finished_at = Some(pm_library::db::now_unix());
            p.clone()
        };
        remember_run(&st, &ended);
        let _ = st.events.send(crate::types::ApiEvent::LibraryChanged);
    });
    true
}

/// How long the node lets its start settle before it checks old suggestions again.
const RECHECK_AFTER_START: std::time::Duration = std::time::Duration::from_secs(60);

/// At start: when songs wait for review that older rules checked, check them again once under
/// the current rules (owner, 2026-11-06: most of them are the same song on several editions, which
/// the current rules settle). After the start's library scan, only when MusicBrainz can be asked
/// and no run is going. The run checks only those songs (nothing new is fingerprinted); songs it
/// settles are marked checked by these rules, so the next start finds nothing to do; with "Write
/// IDs" on, the IDs found for them are written as after any run.
///
/// Then, every hour, the weekly retry of "not found" songs (`weekly_retry`).
pub fn spawn_recheck(s: &Arc<NodeState>) {
    let s = s.clone();
    tokio::spawn(async move {
        tokio::time::sleep(RECHECK_AFTER_START).await;
        recheck_when_idle(&s, std::time::Duration::from_secs(5)).await;
        loop {
            weekly_retry(&s);
            tokio::time::sleep(WEEKLY_LOOK).await;
        }
    });
}

/// Whether a scan or a run is going (an automatic run waits for both).
fn busy(s: &NodeState) -> bool {
    s.is_scanning()
        || s.side_scans.load(std::sync::atomic::Ordering::SeqCst) > 0
        || s.identify.lock().expect("identify lock").running
}

/// [`spawn_recheck`] without the wait at start: once no scan and no run is going, start the run
/// if there is something to check again. Whether it started one.
pub(crate) async fn recheck_when_idle(s: &Arc<NodeState>, poll: std::time::Duration) -> bool {
    loop {
        while busy(s) {
            tokio::time::sleep(poll).await;
        }
        let outdated = s
            .library
            .lock()
            .expect("library poisoned")
            .has_outdated_suggestions()
            .unwrap_or(false);
        if !outdated || s.contact().trim().is_empty() {
            return false;
        }
        if launch(s, RunOptions::recheck()) {
            tracing::info!("identify: checking songs that wait for review again under the current rules");
            return true;
        }
        // Another run took the slot meanwhile (an automatic one after the start's scan): after it
    }
}

/// How often an automatic run waiting for a scan or another run looks again.
const AUTO_POLL: std::time::Duration = if cfg!(test) {
    std::time::Duration::from_millis(10)
} else {
    std::time::Duration::from_secs(5)
};

/// Tests: automatic runs after a scan only for the nodes that ask for them (the others scan
/// without a run reaching for any service).
#[cfg(test)]
pub(crate) static AUTO_RUNS_FOR: std::sync::Mutex<Vec<std::sync::Weak<NodeState>>> =
    std::sync::Mutex::new(Vec::new());

/// A scan added or changed `changed` songs: with "Match new songs automatically" on and a
/// MusicBrainz contact, a run for them starts once no scan and no other run is going. The same
/// job and pacing as Identify: a plain run, which checks songs never checked and songs whose file
/// or tags changed, nothing else. One waiter at a time; scans that end while it waits are covered
/// by its run, and one that ends after its run started gets a run of its own after it.
pub fn after_scan(s: &Arc<NodeState>, changed: u64) {
    if changed == 0 {
        return;
    }
    #[cfg(test)]
    {
        let asked = AUTO_RUNS_FOR.lock().expect("auto runs");
        if !asked
            .iter()
            .any(|w| w.upgrade().is_some_and(|a| Arc::ptr_eq(&a, s)))
        {
            return;
        }
    }
    let Ok(rt) = tokio::runtime::Handle::try_current() else {
        return;
    };
    if s.identify_waiting.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let s = s.clone();
    rt.spawn(async move {
        run_after_scan(&s, AUTO_POLL).await;
    });
}

/// [`after_scan`]'s waiter: whether it started a run.
pub(crate) async fn run_after_scan(s: &Arc<NodeState>, poll: std::time::Duration) -> bool {
    use std::sync::atomic::Ordering::SeqCst;
    loop {
        while busy(s) {
            tokio::time::sleep(poll).await;
        }
        // From here a scan that ends starts a waiter of its own
        s.identify_waiting.store(false, SeqCst);
        if !auto_match_on(s) || s.contact().trim().is_empty() || !plain_run_has_work(s) {
            return false;
        }
        if launch(s, RunOptions::new(false)) {
            tracing::info!("identify: matching the songs a scan added or changed");
            return true;
        }
        // A run started meanwhile: wait for it, unless another waiter does
        if s.identify_waiting.swap(true, SeqCst) {
            return false;
        }
    }
}

/// Whether a plain run has songs to check: never checked, or a "not found" or waiting song whose
/// file, tags or the node's services changed (as [`run_with`] picks them). An automatic run with
/// nothing to do would only replace the last run's summary.
fn plain_run_has_work(s: &NodeState) -> bool {
    let settings = s.acoustid.read().expect("acoustid lock").clone();
    let services = services_key(s, &settings, !s.contact().trim().is_empty());
    let lib = s.library.lock().expect("library poisoned");
    match lib.tracks_to_identify(false) {
        Ok(t) if t.is_empty() => {}
        _ => return true,
    }
    let Ok(kept) = lib.identify_inputs() else {
        return true;
    };
    if kept.is_empty() {
        return false;
    }
    let Ok(all) = lib.all_tracks() else {
        return true;
    };
    all.iter().any(|t| {
        t.id.is_some_and(|id| {
            kept.get(&id.0)
                .is_some_and(|k| k.as_deref() != Some(pm_library::identify::input_key(t, &services).as_str()))
        })
    })
}

/// When the weekly retry last started (unix seconds).
const WEEKLY_KEY: &str = "identify.weekly_at";
const WEEK_SECS: i64 = 7 * 24 * 3600;
/// How often the node looks whether the weekly retry is due.
const WEEKLY_LOOK: std::time::Duration = std::time::Duration::from_secs(3600);
/// Songs one weekly retry checks at most (oldest first; the next week takes the next ones).
pub const WEEKLY_MAX: usize = 500;

/// Once a week, with "Match new songs automatically" on and a MusicBrainz contact, while nothing
/// else runs: one run over "not found" songs last checked before their kept MusicBrainz answers
/// expired (30 days), at most [`WEEKLY_MAX`], their AcoustID answers from then asked again.
/// Whether it started one.
pub(crate) fn weekly_retry(s: &Arc<NodeState>) -> bool {
    if !auto_match_on(s) || s.contact().trim().is_empty() || busy(s) {
        return false;
    }
    let now = pm_library::db::now_unix();
    let before = now - Library::IDENTIFY_MB_TTL_SECS;
    let songs = {
        let lib = s.library.lock().expect("library poisoned");
        let last = lib
            .get_setting(WEEKLY_KEY)
            .ok()
            .flatten()
            .and_then(|v| v.parse::<i64>().ok());
        if last.is_some_and(|t| now - t < WEEK_SECS) {
            return false;
        }
        lib.stale_not_found(before, WEEKLY_MAX).unwrap_or_default()
    };
    if songs.is_empty() {
        return false;
    }
    let n = songs.len();
    if !launch(s, RunOptions::weekly(songs, before)) {
        return false;
    }
    if let Err(e) = s
        .library
        .lock()
        .expect("library poisoned")
        .set_setting(WEEKLY_KEY, &now.to_string())
    {
        tracing::debug!("identify: the weekly retry's time not kept: {e}");
    }
    tracing::info!(songs = n, "identify: the weekly look at songs not found");
    true
}

/// Fingerprints decoded at once: the machine's cores, at most 8 (each decodes up to two minutes
/// of audio, from a share over the network too).
fn fingerprint_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, 8)
}

/// Releases a folder's search may look up before the folder goes to the fingerprint.
const RELEASES_PER_FOLDER: usize = 2;

/// Tests: something to do right after a run has read the library (a rescan landing there), for
/// the node it is registered for.
#[cfg(test)]
#[allow(clippy::type_complexity)]
pub(crate) static AFTER_SNAPSHOT: std::sync::Mutex<Vec<(usize, Box<dyn Fn() + Send>)>> =
    std::sync::Mutex::new(Vec::new());

#[cfg(test)]
fn after_snapshot(s: &Arc<NodeState>) {
    let key = Arc::as_ptr(s) as usize;
    for (k, f) in AFTER_SNAPSHOT.lock().expect("hooks").iter() {
        if *k == key {
            f();
        }
    }
}

/// What the lanes share: the songs this run identified, and what each song looks like now.
struct RunCtx {
    found: Mutex<Vec<i64>>,
    inputs: HashMap<i64, String>,
    /// What each song was when the run read it ([`Library::input_generations`]): an answer for a
    /// song that changed since is not stored.
    read: HashMap<i64, String>,
}

/// A folder that says it is an album, for the album lane.
struct AlbumWork {
    album: FolderAlbum,
    /// Every file of the folder (identified or not): the folder as a release.
    files: Vec<FolderFile>,
    /// Its songs still to identify.
    tracks: Vec<Track>,
}

/// A folder's songs for the fingerprint.
struct Folder {
    tracks: Vec<Track>,
    /// Audio files in the folder.
    len: usize,
    /// The release the rest of the folder fits, when it fits one.
    release: Option<String>,
}

/// A song the fingerprint left open, for the search by tags.
struct Miss {
    track: Track,
    outcome: Outcome,
}

/// Run identification to the end. Public for tests and the CLI.
///
/// Each song goes down a ladder, cheapest first, and stops at the first sure answer: the IDs in
/// its tags (settled at scan time, never here), its folder as an album (one MusicBrainz search
/// for the whole folder, every file matched by position, length and title), its fingerprint
/// (decoded on every core, looked up on AcoustID three a second), and last a MusicBrainz search
/// by its tags (suggestions only). The three lanes run at once: the fingerprint starts on the
/// folders that are no album while the album lane asks MusicBrainz, and takes what the album
/// lane leaves; the search waits for the album lane, which shares MusicBrainz's one request a
/// second and settles a dozen songs a request.
pub async fn run(s: &Arc<NodeState>, retry: bool) -> Result<(), ApiError> {
    run_with(s, RunOptions::new(retry)).await
}

/// [`run`] with its options (tests and the replay benchmark pace nothing).
pub async fn run_with(s: &Arc<NodeState>, opts: RunOptions) -> Result<(), ApiError> {
    let retry = opts.retry;
    {
        // Counted afresh (a run started from the page was reset already, and is marked running)
        let mut p = s.identify.lock().expect("identify lock");
        *p = IdentifyProgress {
            running: p.running,
            started_at: p.started_at.or(Some(pm_library::db::now_unix())),
            ..Default::default()
        };
    }
    let settings = s.acoustid.read().expect("acoustid lock").clone();
    let mb = mb_client(s, opts.mb_interval);
    let cache_max = opts.mb_cache_max.unwrap_or(Library::IDENTIFY_MB_MAX);
    let (mut todo, all, kept, read) = {
        let lib = s.library.lock().expect("library poisoned");
        // The caches stay bounded (a cleanup failing only keeps more); not while a scan writes:
        // the run's first write then is a song's, which waits for the scan
        if !s.is_scanning() {
            if let Err(e) = lib.prune_identify_cache(pm_library::db::now_unix(), cache_max) {
                tracing::debug!("identify: caches not pruned: {e}");
            }
        }
        (
            if opts.outdated_only || opts.only.is_some() {
                Vec::new()
            } else {
                lib.tracks_to_identify(retry)?
            },
            lib.all_tracks()?,
            lib.identify_inputs()?,
            // What each song is (file and tags), in the same read as the songs themselves: a
            // rescan after it makes the run's answer stale, and settling sees that
            lib.input_generations()?,
        )
    };
    #[cfg(test)]
    after_snapshot(s);
    // What each song looks like now; a "not found" song that changed since it was checked (its
    // tags, its file, an AcoustID key or a MusicBrainz contact added) is checked again
    let services = services_key(s, &settings, mb.is_some());
    let inputs: HashMap<i64, String> = all
        .iter()
        .filter_map(|t| {
            let id = t.id?.0;
            Some((id, pm_library::identify::input_key(t, &services)))
        })
        .collect();
    if let Some(only) = &opts.only {
        // The weekly retry: these songs, nothing else
        let only: std::collections::HashSet<i64> = only.iter().copied().collect();
        todo = all
            .iter()
            .filter(|t| t.id.is_some_and(|id| only.contains(&id.0)))
            .cloned()
            .collect();
    } else if opts.outdated_only {
        // The start's automatic run: the songs older rules left waiting, nothing else
        let outdated = s
            .library
            .lock()
            .expect("library poisoned")
            .outdated_suggestion_ids()?;
        todo = all
            .iter()
            .filter(|t| t.id.is_some_and(|id| outdated.contains(&id.0)))
            .cloned()
            .collect();
    } else {
        // Songs waiting for review are among them (older rules left them, or they changed): a
        // retry ("Identify again") checks them too, not only a plain run
        let have: std::collections::HashSet<i64> = todo.iter().filter_map(|t| t.id.map(|i| i.0)).collect();
        let changed: Vec<Track> = all
            .iter()
            .filter(|t| {
                t.id.is_some_and(|id| {
                    !have.contains(&id.0)
                        && kept
                            .get(&id.0)
                            .is_some_and(|k| k.as_deref() != inputs.get(&id.0).map(String::as_str))
                })
            })
            .cloned()
            .collect();
        todo.extend(changed);
    }
    s.identify.lock().expect("identify lock").total = todo.len();
    let folder_of = |t: &Track| t.file.path.parent().map(PathBuf::from).unwrap_or_default();
    let mut by_folder: BTreeMap<PathBuf, Vec<Track>> = BTreeMap::new();
    for t in todo {
        by_folder.entry(folder_of(&t)).or_default().push(t);
    }
    let mut in_folder: HashMap<PathBuf, Vec<FolderFile>> = HashMap::new();
    for t in &all {
        let dir = folder_of(t);
        if by_folder.contains_key(&dir) {
            in_folder.entry(dir).or_default().push(folder_file(t));
        }
    }
    drop(all);
    let client = if settings.api_key.is_empty() {
        None
    } else {
        let mut c = pm_library::identify::AcoustIdConfig::new(&settings.api_key, s.user_agent());
        c.base_url = settings.base_url.clone();
        c.min_interval = opts.acoustid_interval;
        Some(pm_library::identify::AcoustIdClient::new(c).map_err(ApiError::from)?)
    };
    let found = RunCtx {
        found: Mutex::new(Vec::new()),
        inputs,
        read,
    };
    // Folders that are no album go to the fingerprint at once; albums to the album lane
    let (to_fp, from_albums) = mpsc::unbounded_channel::<Folder>();
    let mut albums = Vec::new();
    for (dir, tracks) in by_folder {
        let files = in_folder
            .remove(&dir)
            .unwrap_or_else(|| tracks.iter().map(folder_file).collect());
        match folder_album(&files).filter(|_| mb.is_some()) {
            Some(album) => albums.push(AlbumWork { album, files, tracks }),
            None => {
                let _ = to_fp.send(Folder {
                    len: files.len().max(tracks.len()),
                    tracks,
                    release: None,
                });
            }
        }
    }
    let (to_search, misses) = mpsc::unbounded_channel::<Miss>();
    let (albums_done, after_albums) = tokio::sync::oneshot::channel::<()>();
    let album_lane = async {
        let r = album_lane(s, mb.as_ref(), albums, &to_fp, &found).await;
        drop(to_fp);
        let _ = albums_done.send(());
        r
    };
    let lanes = tokio::try_join!(
        album_lane,
        fingerprint_lane(
            s,
            client.as_ref(),
            from_albums,
            to_search,
            &found,
            opts.refresh_before
        ),
        search_lane(s, mb.as_ref(), misses, after_albums, &found),
    );
    let identified_now = found.found.into_inner().expect("found lock");
    // Songs waiting for review whose stored evidence is clear on its own (this run's and earlier
    // ones'): settled as `auto`, never written into the files here; only with "Match new songs
    // automatically" on (off, the owner decides every unsure song)
    let settling = if auto_match_on(s) {
        auto_settle_pass(s).await
    } else {
        Ok(SettleReport::default())
    };
    match settling {
        Ok(r) if r.settled > 0 => {
            tracing::info!(settled = r.settled, by = ?r.by_rule, "identify: unsure songs settled automatically");
            let mut p = s.identify.lock().expect("identify lock");
            for (rule, n) in r.by_rule {
                *p.settled.entry(rule).or_default() += n;
            }
        }
        Ok(_) => {}
        Err(e) => tracing::warn!("identify: unsure songs not settled automatically: {}", e.message),
    }
    // "Write IDs into my files", when the owner turned it on: also for the songs a run that
    // stopped early identified before it stopped
    if !identified_now.is_empty() && write_ids_on(s) {
        let st = s.clone();
        let only = identified_now.clone();
        let written = tokio::task::spawn_blocking(move || write_found_ids(&st, Some(&only)))
            .await
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
            .and_then(|r| r);
        match written {
            Ok(r) => tracing::info!(
                written = r.written,
                skipped = r.skipped.len(),
                "identify: IDs written into files"
            ),
            // The run's own error is the one to report
            Err(e) if lanes.is_err() => tracing::warn!("identify: IDs not written: {}", e.message),
            Err(e) => return Err(e),
        }
    }
    // The caches stay within their bounds when the run ends too (waiting through a scan)
    if let Err(e) = write_waiting(s, |lib| {
        lib.prune_identify_cache(pm_library::db::now_unix(), cache_max)
    })
    .await
    {
        tracing::debug!("identify: caches not pruned: {}", e.message);
    }
    lanes?;
    Ok(())
}

/// What a song's check depended on in the node's services: AcoustID's address and key, and
/// MusicBrainz's address and whether a contact lets it be asked. The key goes in only hashed,
/// and only into [`pm_library::identify::input_key`]'s own hash: nothing of it is stored.
fn services_key(s: &NodeState, acoustid: &AcoustIdSettings, mb_on: bool) -> String {
    let key = if acoustid.api_key.is_empty() {
        String::new()
    } else {
        blake3::hash(format!("pm-identify-acoustid-key\u{1f}{}", acoustid.api_key).as_bytes()).to_hex()[..16]
            .to_string()
    };
    let mb_base = s.mb_base.read().expect("mb lock").clone();
    format!(
        "acoustid={}|{key}|musicbrainz={mb_on}|{}",
        acoustid.base_url.trim_end_matches('/'),
        mb_base.trim_end_matches('/')
    )
}

fn folder_file(t: &Track) -> FolderFile {
    FolderFile {
        track: t.id.map(|i| i.0).unwrap_or_default(),
        title: t.title.clone(),
        artist: t.artists.first().map(|a| a.name.clone()).unwrap_or_default(),
        album: t.album.as_ref().map(|a| a.title.clone()),
        album_artist: t.album.as_ref().and_then(|a| a.album_artist.clone()),
        disc_no: t.disc_no,
        track_no: t.track_no,
        duration_ms: t.duration_ms.or(t.file.duration_ms),
        release: t.album.as_ref().and_then(|a| a.release_mbid),
    }
}

/// MusicBrainz, when the owner gave a contact (it requires one). Every client shares the
/// process's one gate per host ([`pm_library::musicbrainz::take_turn`]).
fn mb_client(
    s: &NodeState,
    interval: std::time::Duration,
) -> Option<pm_library::musicbrainz::MusicBrainzClient> {
    let contact = s.contact();
    if contact.trim().is_empty() {
        return None;
    }
    let mut cfg = pm_library::musicbrainz::MusicBrainzConfig::new(contact);
    cfg.base_url = s.mb_base.read().expect("mb lock").clone();
    cfg.min_interval = interval;
    pm_library::musicbrainz::MusicBrainzClient::new(cfg).ok()
}

/// Each album folder against MusicBrainz: what fits is identified, the rest goes on to the
/// fingerprint (with the release the folder fits, when it fits one).
async fn album_lane(
    s: &NodeState,
    mb: Option<&pm_library::musicbrainz::MusicBrainzClient>,
    albums: Vec<AlbumWork>,
    next: &mpsc::UnboundedSender<Folder>,
    found: &RunCtx,
) -> Result<(), ApiError> {
    let Some(mb) = mb else {
        return Ok(());
    };
    // A disc per folder asks for its album once
    let mut searches: HashMap<String, Option<serde_json::Value>> = HashMap::new();
    let mut releases: HashMap<Mbid, Option<serde_json::Value>> = HashMap::new();
    // An earlier run's fingerprints are a second opinion, read by file content; with none
    // cached, no file is read for one
    let second_opinion = s
        .library
        .lock()
        .expect("library poisoned")
        .has_cached_lookups()
        .unwrap_or(true);
    let workers = fingerprint_workers();
    for w in albums {
        let fit = match fit_folder(s, mb, &w, &mut searches, &mut releases).await {
            Ok(f) => f,
            Err(e) => {
                tracing::debug!(album = %w.album.title, "identify: the album lookup failed ({e}); the fingerprint decides");
                None
            }
        };
        let mut left = Vec::new();
        if let Some(f) = fit.as_ref() {
            let mut matched = Vec::new();
            for t in w.tracks {
                let id = t.id.map(|i| i.0).unwrap_or_default();
                if let Some(c) = f.conflicts.iter().find(|c| c.track == id) {
                    // Editions that fit as well name different recordings: the owner chooses
                    let options = c
                        .options
                        .iter()
                        .map(|o| Candidate {
                            recording: o.recording,
                            release: Some(o.release),
                            title: Some(o.title.clone()),
                            artist: Some(w.album.artist.clone()),
                            release_title: o.release_title.clone(),
                            score: f.score(),
                            source: "album".into(),
                        })
                        .collect();
                    settle(s, &t, Outcome::Suggest(options), found).await?;
                    continue;
                }
                match f.matched.iter().find(|m| m.track == id) {
                    Some(m) => matched.push((t, m.clone())),
                    None => left.push(t),
                }
            }
            let album = &w.album;
            let mut decided = std::pin::pin!(futures_util::stream::iter(matched)
                .map(|(t, m)| async move {
                    let outcome = album_outcome(s, &t, album, f, &m, second_opinion).await;
                    (t, outcome)
                })
                .buffered(workers));
            while let Some((t, outcome)) = decided.next().await {
                settle(s, &t, outcome, found).await?;
            }
        } else {
            left = w.tracks;
        }
        if !left.is_empty() {
            let _ = next.send(Folder {
                tracks: left,
                len: w.files.len(),
                // Tied editions: the fingerprint finds out which (if any) on its own
                release: fit.filter(|f| f.editions == 1).map(|f| f.release.to_string()),
            });
        }
    }
    Ok(())
}

/// The release a folder fits best: the one its tags name, else the search's nearest few.
async fn fit_folder(
    s: &NodeState,
    mb: &pm_library::musicbrainz::MusicBrainzClient,
    w: &AlbumWork,
    searches: &mut HashMap<String, Option<serde_json::Value>>,
    releases: &mut HashMap<Mbid, Option<serde_json::Value>>,
) -> Result<Option<AlbumFit>, String> {
    let ids: Vec<(Mbid, u64)> = match w.album.release {
        Some(r) => vec![(r, 0)],
        None => {
            let q = release_query(&w.album);
            let answer = match searches.get(&q) {
                Some(a) => a.clone(),
                None => {
                    let a = mb_get(s, mb, "release", &[("query", q.as_str()), ("limit", "10")]).await?;
                    searches.insert(q, a.clone());
                    a
                }
            };
            answer
                .map(|a| release_candidates_ranked(&w.album, w.files.len(), &a, RELEASES_PER_FOLDER))
                .unwrap_or_default()
        }
    };
    // Every candidate (at most RELEASES_PER_FOLDER) is looked at: an edition further in size
    // may fit as well (a multi-disc edition fits by the discs the folder holds), with other
    // recordings
    let mut fits: Vec<AlbumFit> = Vec::new();
    for (id, _) in ids {
        let rel = match releases.get(&id) {
            Some(r) => r.clone(),
            None => {
                // With its release group: the release's kind (album, single, compilation…)
                let r = mb_get(
                    s,
                    mb,
                    &format!("release/{id}"),
                    &[("inc", "recordings+artist-credits+release-groups")],
                )
                .await?;
                releases.insert(id, r.clone());
                r
            }
        };
        if let Some(f) = rel.as_ref().and_then(|r| fit_release(&w.files, r)) {
            fits.push(f);
        }
    }
    Ok(settle_editions(fits, &w.album))
}

/// A song its folder's release names. A fingerprint looked up before (by an earlier run) is a
/// second opinion: agreeing raises the confidence; naming other recordings only (another song,
/// or another recording of this one) leaves both for the owner to confirm.
async fn album_outcome(
    s: &NodeState,
    t: &Track,
    album: &FolderAlbum,
    fit: &AlbumFit,
    m: &Matched,
    second_opinion: bool,
) -> Outcome {
    let results = if second_opinion {
        cached_results(s, t).await.unwrap_or_default()
    } else {
        Vec::new()
    };
    let strong: Vec<(f32, &str, &pm_library::identify::AcRecording)> = results
        .iter()
        .filter(|r| r.score >= Confidence::FINGERPRINT_ACCEPT)
        .flat_map(|r| r.recordings.iter().map(move |rec| (r.score, r.id.as_str(), rec)))
        .collect();
    let agrees = strong
        .iter()
        .find(|(_, _, rec)| rec.id == m.recording.to_string());
    // A sure fingerprint that names none of the folder's recording is another recording: maybe of
    // this song (a live take, another master), never settled by the title
    let other_song = !strong.is_empty() && agrees.is_none();
    // Tied editions agreeing on the recording: which edition stays open
    let release = (fit.editions == 1).then_some(fit.release);
    let mine = Candidate {
        recording: m.recording,
        release,
        title: Some(m.title.clone()),
        artist: Some(album.artist.clone()),
        // Tied editions: none of their titles stands for the release
        release_title: release.and(fit.release_title.clone()),
        score: fit.score(),
        source: "album".into(),
    };
    if other_song {
        let mut c = vec![mine];
        for (score, _, rec) in strong.iter().take(4) {
            let Ok(recording) = Mbid::parse(&rec.id) else {
                continue;
            };
            c.push(Candidate {
                recording,
                release: None,
                title: rec.title.clone(),
                artist: rec.artist(),
                release_title: None,
                score: *score,
                source: "fingerprint".into(),
            });
        }
        return Outcome::Suggest(c);
    }
    let (score, acoustid, detail) = match agrees {
        Some((fp, acoustid, _)) => (
            (1.0 - (1.0 - fit.score()) * (1.0 - fp)).min(0.99),
            Some(acoustid.to_string()),
            "album_fit, the fingerprint agrees",
        ),
        None if fit.editions > 1 => (fit.score(), None, "album_fit, editions agree"),
        None => (fit.score(), None, "album_fit"),
    };
    Outcome::Identified {
        recording: m.recording,
        release,
        confidence: Confidence::AlbumFit { score },
        acoustid,
        detail: detail.into(),
    }
}

/// What an earlier run's AcoustID lookup said about the file, without decoding it.
async fn cached_results(s: &NodeState, t: &Track) -> Option<Vec<AcoustIdResult>> {
    let key = content_key(s, t).await.ok()?;
    let (_, _, json) = s
        .library
        .lock()
        .expect("library poisoned")
        .cached_fingerprint(&key)
        .ok()??;
    let r: pm_library::identify::LookupResponse = serde_json::from_str(&json?).ok()?;
    Some(r.results)
}

/// The fingerprint lane: songs are fingerprinted on every core and looked up as they come, and
/// each folder is decided once all its songs are in (the folder as a release disambiguates).
async fn fingerprint_lane(
    s: &NodeState,
    client: Option<&pm_library::identify::AcoustIdClient>,
    folders: mpsc::UnboundedReceiver<Folder>,
    misses: mpsc::UnboundedSender<Miss>,
    found: &RunCtx,
    refresh_before: Option<i64>,
) -> Result<(), ApiError> {
    struct Meta {
        len: usize,
        release: Option<String>,
        songs: usize,
    }
    let workers = fingerprint_workers();
    let decoding = tokio::sync::Semaphore::new(workers);
    let gate = KeyGate::default();
    let folders =
        futures_util::stream::unfold(folders, |mut rx| async move { rx.recv().await.map(|f| (f, rx)) });
    let looked = folders
        .flat_map(|f| {
            let meta = Arc::new(Meta {
                len: f.len,
                release: f.release,
                songs: f.tracks.len(),
            });
            futures_util::stream::iter(f.tracks.into_iter().map(move |t| (meta.clone(), t)))
        })
        .map(|(meta, t)| {
            let decoding = &decoding;
            let gate = &gate;
            async move {
                let r = lookup(s, client, &t, decoding, gate, refresh_before).await;
                (meta, t, r)
            }
        })
        // Lookups wait for AcoustID's pace while more songs decode
        .buffered(workers * 2 + 4);
    let mut looked = std::pin::pin!(looked);
    let mut folder: Vec<(Track, FileFacts, Result<Vec<AcoustIdResult>, String>)> = Vec::new();
    while let Some((meta, t, r)) = looked.next().await {
        let facts = FileFacts {
            track_no: t.track_no,
            disc_no: t.disc_no,
            duration_secs: t.duration_ms.or(t.file.duration_ms).map(|ms| ms as f64 / 1000.0),
        };
        folder.push((t, facts, r));
        if folder.len() < meta.songs {
            continue;
        }
        let mut looked_up = std::mem::take(&mut folder);
        // What the owner rejected for a song is no recording to decide on (its same-audio
        // sibling may still be)
        {
            let lib = s.library.lock().expect("library poisoned");
            for (t, _, r) in looked_up.iter_mut() {
                let id = t.id.map(|i| i.0).unwrap_or_default();
                if let Ok(results) = r {
                    for res in results.iter_mut() {
                        res.recordings
                            .retain(|rec| !lib.is_rejected(id, &rec.id).unwrap_or(false));
                    }
                    results.retain(|res| !res.recordings.is_empty());
                }
            }
        }
        let pairs: Vec<(FileFacts, &[AcoustIdResult])> = looked_up
            .iter()
            .filter_map(|(_, f, r)| r.as_ref().ok().map(|r| (*f, r.as_slice())))
            .collect();
        let preferred: Vec<String> = match meta.release.clone() {
            Some(r) => vec![r],
            None => best_folder_releases(&pairs, meta.len),
        };
        for (t, facts, r) in looked_up {
            let outcome = match r {
                Err(e) => Outcome::Error(e),
                Ok(results) => {
                    match decide_with_tags(&results, &facts, meta.len, &preferred, Some(&file_tags(&t))) {
                        Decision::Identified {
                            recording,
                            release,
                            confidence,
                            acoustid,
                        } => Outcome::Identified {
                            recording,
                            release,
                            confidence,
                            acoustid: Some(acoustid),
                            detail: confidence.label().into(),
                        },
                        Decision::Suggest(c) => Outcome::Suggest(c),
                        Decision::Unknown => Outcome::Unknown,
                    }
                }
            };
            match outcome {
                // Nothing from the fingerprint: the search by tags has a go
                Outcome::Unknown | Outcome::Error(_) => {
                    let _ = misses.send(Miss { track: t, outcome });
                }
                o => settle(s, &t, o, found).await?,
            }
        }
    }
    Ok(())
}

/// What a file says about itself, for choosing among recordings of its song.
fn file_tags(t: &Track) -> pm_library::same_song::FileTags {
    pm_library::same_song::FileTags {
        title: t.title.clone(),
        artist: t.artists.first().map(|a| a.name.clone()).unwrap_or_default(),
        album: t.album.as_ref().map(|a| a.title.clone()),
        duration_ms: t.duration_ms.or(t.file.duration_ms),
    }
}

/// The last step, once the album lane is done with MusicBrainz: a search by the tags.
async fn search_lane(
    s: &NodeState,
    mb: Option<&pm_library::musicbrainz::MusicBrainzClient>,
    mut misses: mpsc::UnboundedReceiver<Miss>,
    after_albums: tokio::sync::oneshot::Receiver<()>,
    found: &RunCtx,
) -> Result<(), ApiError> {
    let _ = after_albums.await;
    while let Some(m) = misses.recv().await {
        let found_now = match mb {
            Some(mb) => search_hits(s, mb, &m.track, true).await,
            None => Ok(Vec::new()),
        };
        let outcome = match found_now {
            Ok(hits) => {
                // What the owner rejected for this song is never picked
                let id = m.track.id.map(|i| i.0).unwrap_or_default();
                let hits: Vec<pm_library::musicbrainz::RecordingHit> = {
                    let lib = s.library.lock().expect("library poisoned");
                    hits.into_iter()
                        .filter(|h| !lib.is_rejected(id, &h.mbid.to_string()).unwrap_or(false))
                        .collect()
                };
                match pm_library::same_song::pick_exact_hit(&file_tags(&m.track), &hits) {
                    // Exactly the file's song (on one or several recordings): identified
                    Some(p) => Outcome::Identified {
                        recording: p.recording,
                        release: p.release,
                        confidence: Confidence::TagMatch {
                            score: TAG_MATCH_SCORE,
                        },
                        acoustid: None,
                        detail: "tag_match".into(),
                    },
                    None => {
                        let c = rank_track_hits(&m.track, hits);
                        if c.is_empty() {
                            m.outcome
                        } else if matches!(m.outcome, Outcome::Error(_)) {
                            // The fingerprint failed: the search's options are added to what the
                            // song has, and it isn't marked checked (its fingerprint options stay,
                            // and a later run asks the fingerprint again)
                            settle_with(s, &m.track, Outcome::Suggest(c), found, false).await?;
                            continue;
                        } else {
                            Outcome::Suggest(c)
                        }
                    }
                }
            }
            // MusicBrainz failed: an error (a later run asks again), unless the fingerprint said more
            Err(e) => match m.outcome {
                Outcome::Unknown => Outcome::Error(e),
                o => o,
            },
        };
        settle(s, &m.track, outcome, found).await?;
    }
    Ok(())
}

/// Store what a song's ladder found and count it.
async fn settle(s: &NodeState, t: &Track, outcome: Outcome, found: &RunCtx) -> Result<(), ApiError> {
    settle_with(s, t, outcome, found, true).await
}

/// [`settle`]; `replace` false: options found are added to the song's open ones and the song
/// isn't marked checked (part of its check failed).
async fn settle_with(
    s: &NodeState,
    t: &Track,
    outcome: Outcome,
    found: &RunCtx,
    replace: bool,
) -> Result<(), ApiError> {
    use pm_library::db::{SettleAnswer, Settled};
    let id = t.id.map(|i| i.0).unwrap_or_default();
    let input_key = found.inputs.get(&id).cloned();
    let answer = match &outcome {
        Outcome::Identified {
            recording,
            release,
            confidence,
            acoustid,
            detail,
        } => SettleAnswer::Identified {
            recording: MusicUri::Recording { mbid: *recording },
            confidence: *confidence,
            acoustid: acoustid.clone(),
            detail: detail.clone(),
            release: *release,
            input_key,
        },
        Outcome::Suggest(c) => SettleAnswer::Suggest {
            candidates: c.clone(),
            input_key,
            replace,
        },
        Outcome::Unknown => SettleAnswer::State {
            state: "unknown".into(),
            detail: None,
            input_key,
        },
        Outcome::Error(e) => SettleAnswer::State {
            state: "error".into(),
            detail: Some(e.clone()),
            input_key,
        },
    };
    // One step, against what holds when it is written (the owner may confirm or reject while a
    // busy library makes the run wait): the owner's word stands, and only what was stored counts
    let read = found.read.get(&id).map(String::as_str);
    let settled = write_waiting(s, |lib| lib.settle_checked(id, &answer, read)).await?;
    if settled == Settled::Stored && matches!(outcome, Outcome::Identified { .. }) {
        // Found once its identity is stored: its IDs may be written into the file
        found.found.lock().expect("found lock").push(id);
    }
    // The library first, then the count, never both at once: `status` holds the progress while
    // it reads the library
    let mut p = s.identify.lock().expect("identify lock");
    p.done += 1;
    match (settled, outcome) {
        // Kept: the owner's or another identity; Changed: checked again by the next run
        (Settled::Kept | Settled::Changed, _) => {}
        // Checking a waiting song again failed: it keeps its options for the next run
        (Settled::Failed, o) => {
            p.errors += 1;
            if let Outcome::Error(e) = o {
                p.last_error = Some(e);
            }
        }
        (Settled::Rejected | Settled::Empty, _) => p.unknown += 1,
        (Settled::Stored, Outcome::Identified { confidence, .. }) => {
            p.identified += 1;
            *p.identified_by.entry(confidence.label().to_string()).or_default() += 1;
        }
        (Settled::Stored, Outcome::Suggest(_)) => p.suggested += 1,
        (Settled::Stored, Outcome::Unknown) => p.unknown += 1,
        (Settled::Stored, Outcome::Error(e)) => {
            p.errors += 1;
            p.last_error = Some(e);
        }
    }
    // Now and then the run's progress is kept, so a restart knows how far it got
    let kept = p.done.is_multiple_of(250).then(|| p.clone());
    drop(p);
    if let Some(k) = kept {
        remember_run(s, &k);
    }
    Ok(())
}

/// What an automatic-settling pass did: songs looked at, settled (by rule), and why the others
/// stay for the owner (by reason). Counts only: no title, path or id.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SettleReport {
    pub looked: usize,
    pub settled: usize,
    pub by_rule: BTreeMap<String, usize>,
    pub stay: BTreeMap<String, usize>,
    /// What in particular kept them, by `reason/detail` (`version/title:live`, `artist/various`,
    /// `different_songs/tags_row:strong`…; words of a closed list only).
    #[serde(default)]
    pub why: BTreeMap<String, usize>,
}

/// Settle songs waiting for review whose stored evidence is clear on its own
/// ([`pm_library::auto_settle`]): their open options, their tags, and the release their folder is
/// on (its songs identified by a run, their tags or the owner; never an automatic pick, so the
/// order songs are looked at changes nothing). Reads only the library, asks no service. Songs the
/// owner left as they are, or rejected a recording for, stay. Each pick is stored only while it
/// holds ([`Library::settle_auto`]): a confirmation, rejection or rescan meanwhile wins.
pub async fn auto_settle_pass(s: &NodeState) -> Result<SettleReport, ApiError> {
    use pm_library::auto_settle::{decide_why, folder_release, with_answer, Opt, Song, Verdict};
    let (waiting, rows, releases, rejected, read, all) = {
        let lib = s.library.lock().expect("library poisoned");
        (
            lib.suggested_tracks()?,
            lib.suggestions()?,
            lib.identified_releases()?,
            lib.tracks_with_rejections()?,
            lib.input_generations()?,
            lib.all_tracks()?,
        )
    };
    let mut report = SettleReport {
        looked: waiting.len(),
        ..Default::default()
    };
    if waiting.is_empty() {
        return Ok(report);
    }
    let folder_of = |t: &Track| t.file.path.parent().map(PathBuf::from).unwrap_or_default();
    let wanted: std::collections::HashSet<PathBuf> = waiting.iter().map(folder_of).collect();
    // Each folder's votes: the release of its songs identified (not automatically), else the
    // release their album's tags name
    let mut votes: HashMap<PathBuf, Vec<String>> = HashMap::new();
    for t in &all {
        let dir = folder_of(t);
        if !wanted.contains(&dir) {
            continue;
        }
        let id = t.id.map(|i| i.0).unwrap_or_default();
        // (an automatic pick's own release is never evidence; its album's tags are, as before
        // it was settled, so what one pass settled changes nothing for the next)
        let tagged = t
            .album
            .as_ref()
            .and_then(|a| a.release_mbid)
            .map(|m| m.to_string());
        let auto = matches!(t.canonical, Some((_, Confidence::Auto { .. })));
        let vote = if t.canonical.is_some() && !auto {
            releases.get(&id).cloned().or(tagged)
        } else {
            tagged
        };
        if let Some(v) = vote {
            votes.entry(dir).or_default().push(v);
        }
    }
    drop(all);
    let folder: HashMap<PathBuf, Option<String>> = votes
        .into_iter()
        .map(|(dir, v)| {
            let r = folder_release(v.iter().map(String::as_str));
            (dir, r)
        })
        .collect();
    let mut options: HashMap<i64, Vec<Opt>> = HashMap::new();
    let mut stored_rows: HashMap<i64, Vec<SuggestionRow>> = HashMap::new();
    for r in &rows {
        options.entry(r.track_id).or_default().push(Opt::from(r));
    }
    for r in rows {
        stored_rows.entry(r.track_id).or_default().push(r);
    }
    for t in &waiting {
        let id = t.id.map(|i| i.0).unwrap_or_default();
        let mut opts = options.remove(&id).unwrap_or_default();
        // A song whose file is gone (nothing kept of what it is now) is not settled
        let Some(was) = read.get(&id).map(String::as_str) else {
            *report.stay.entry("no_file".to_string()).or_default() += 1;
            continue;
        };
        // The run's kept AcoustID answer, read from the library (no file is read): its every sure
        // recording, with lengths and release kinds
        let kept = s
            .library
            .lock()
            .expect("library poisoned")
            .lookup_for_path(&t.file.path)
            .ok()
            .flatten()
            .and_then(|j| serde_json::from_str::<pm_library::identify::LookupResponse>(&j).ok());
        let answer = kept.is_some();
        if let Some(kept) = kept {
            let none = |_: &str| false;
            opts = with_answer(opts, &kept.results, &none);
        }
        let tags = file_tags(t);
        let fr = folder.get(&folder_of(t)).cloned().flatten();
        let (verdict, detail) = decide_why(&Song {
            tags: &tags,
            options: &opts,
            folder_release: fr.as_deref(),
            rejected: rejected.contains(&id),
            answer,
        });
        let (recording, release, score, rule) = match verdict {
            Verdict::Stay(why) => {
                *report.stay.entry(why.name().to_string()).or_default() += 1;
                if let Some(d) = detail {
                    *report.why.entry(format!("{}/{d}", why.name())).or_default() += 1;
                }
                continue;
            }
            Verdict::Pick {
                recording,
                release,
                score,
                rule,
            } => (recording, release, score, rule),
        };
        let (Ok(recording), release) = (
            Mbid::parse(&recording),
            release.and_then(|r| Mbid::parse(&r).ok()),
        ) else {
            *report.stay.entry("unreadable".to_string()).or_default() += 1;
            continue;
        };
        let detail = format!("auto: {}", rule.name());
        // The evidence it was decided on, checked again where it is stored
        let signature =
            pm_library::db::options_signature(stored_rows.get(&id).map(Vec::as_slice).unwrap_or_default());
        let dir = folder_of(t);
        let pick = pm_library::db::AutoPick {
            recording: &recording,
            release: release.as_ref(),
            score,
            detail: &detail,
            read: Some(was),
            options: &signature,
            folder: fr.as_deref().map(|r| (dir.as_path(), r)),
        };
        // Not while a scan writes (it may hold what it read before this pick): left to the next
        // pass (and a scan that changes the song's file reopens an automatic pick anyway)
        let stored = write_waiting(s, |lib| match s.scan_gate.try_lock() {
            Err(std::sync::TryLockError::WouldBlock) => Ok(pm_library::AutoSettle::Scanning),
            _ => lib.settle_auto(id, &pick),
        })
        .await?;
        if stored == pm_library::AutoSettle::Stored {
            report.settled += 1;
            *report.by_rule.entry(rule.name().to_string()).or_default() += 1;
        } else {
            *report.stay.entry("changed".to_string()).or_default() += 1;
            *report
                .why
                .entry(format!("changed/{}", stored.name()))
                .or_default() += 1;
        }
    }
    Ok(report)
}

enum Outcome {
    Identified {
        recording: Mbid,
        release: Option<Mbid>,
        confidence: Confidence,
        acoustid: Option<String>,
        /// How, in a word or two (kept with the state).
        detail: String,
    },
    Suggest(Vec<Candidate>),
    Unknown,
    Error(String),
}

/// Fingerprint (cached by file key) and look up (cached answer, else AcoustID when a key is
/// set). Without a key, or once AcoustID refused it, only an answer looked up before counts:
/// nothing is decoded for nothing.
/// What a run's fingerprint lane knows of its AcoustID key.
#[derive(Default)]
struct KeyGate {
    /// AcoustID answered the key (see [`lookup`]).
    answered: tokio::sync::Mutex<bool>,
    /// AcoustID refused the run's key: nothing more is fingerprinted. The run's own, not the
    /// progress shown: a key saved meanwhile clears that, but the run keeps its client.
    refused: std::sync::atomic::AtomicBool,
}

async fn lookup(
    s: &NodeState,
    client: Option<&pm_library::identify::AcoustIdClient>,
    t: &Track,
    decoding: &tokio::sync::Semaphore,
    gate: &KeyGate,
    refresh_before: Option<i64>,
) -> Result<Vec<AcoustIdResult>, String> {
    let path = t.file.path.clone();
    let key = content_key(s, t).await?;
    let (cached, asked_at) = {
        let lib = s.library.lock().expect("library poisoned");
        (
            lib.cached_fingerprint(&key).map_err(|e| e.to_string())?,
            if refresh_before.is_some() {
                lib.lookup_time(&key).map_err(|e| e.to_string())?
            } else {
                None
            },
        )
    };
    // The weekly retry asks AcoustID again about an answer kept from before `refresh_before`
    // (with a key to ask it with; else the kept answer stands)
    let expired =
        client.is_some() && refresh_before.is_some_and(|before| asked_at.is_none_or(|at| at < before));
    if let Some((_, _, Some(j))) = cached.as_ref().filter(|_| !expired) {
        let r: pm_library::identify::LookupResponse = serde_json::from_str(j).map_err(|e| e.to_string())?;
        work(s, |w| w.lookups_reused += 1);
        return Ok(r.results);
    }
    // Without a key, or once AcoustID refused it (this run), a fingerprint answers nothing
    let refused = || gate.refused.load(std::sync::atomic::Ordering::SeqCst);
    let client = client.filter(|_| !refused());
    let Some(client) = client else {
        return Ok(Vec::new());
    };
    // Until AcoustID has answered the key, one song at a time: a key it refuses costs one
    // song's decoding, not one per core
    let mut probe = {
        let a = gate.answered.lock().await;
        (!*a).then_some(a)
    };
    if refused() {
        return Ok(Vec::new());
    }
    let (duration, fp) = match cached {
        // This audio didn't fingerprint before (too short, not decodable): the same answer
        Some((_, fp, _)) if fp.starts_with(UNFINGERPRINTABLE) => {
            work(s, |w| w.failures_reused += 1);
            return Err(fp[UNFINGERPRINTABLE.len()..].to_string());
        }
        Some((d, fp, _)) => (d, fp),
        None => {
            if t.file.format.kind == pm_model::SampleKind::Dsd {
                return Err("DSD files are not fingerprinted".into());
            }
            let _turn = decoding.acquire().await.map_err(|e| e.to_string())?;
            // Refused while this song waited for its turn
            if refused() {
                return Ok(Vec::new());
            }
            let p = path.clone();
            work(s, |w| w.decoded += 1);
            let f = match tokio::task::spawn_blocking(move || pm_audio::fingerprint::fingerprint_file(&p))
                .await
                .map_err(|e| e.to_string())?
            {
                Ok(f) => f,
                Err(e) => {
                    let why = format!("fingerprint: {e}");
                    // What the audio itself makes impossible is kept by its content (a changed
                    // file has another key); a file that couldn't be read is tried again
                    let lasting =
                        !matches!(e, pm_audio::AudioError::Open { .. } | pm_audio::AudioError::Io(_));
                    if lasting {
                        if let Err(e) = s.library.lock().expect("library poisoned").save_fingerprint(
                            &key,
                            0,
                            &format!("{UNFINGERPRINTABLE}{why}"),
                        ) {
                            tracing::debug!("identify: fingerprint failure not kept: {e}");
                        }
                    }
                    return Err(why);
                }
            };
            // The cache saves work, nothing more: a busy library doesn't fail the song
            if let Err(e) = s.library.lock().expect("library poisoned").save_fingerprint(
                &key,
                f.duration_secs,
                &f.fingerprint,
            ) {
                tracing::debug!("identify: fingerprint not cached: {e}");
            }
            (f.duration_secs, f.fingerprint)
        }
    };
    // Refused while this song was fingerprinted
    if refused() {
        return Ok(Vec::new());
    }
    work(s, |w| w.acoustid_asked += 1);
    let looked = client.lookup(&fp, duration).await;
    if let Some(a) = probe.as_mut() {
        **a = !matches!(looked, Err(pm_library::LibraryError::AcoustIdKey(_)));
    }
    let (raw, results) = match looked {
        Ok(r) => r,
        Err(pm_library::LibraryError::AcoustIdKey(why)) => {
            tracing::warn!("identify: AcoustID refused the key ({why}); nothing more is fingerprinted");
            gate.refused.store(true, std::sync::atomic::Ordering::SeqCst);
            // Shown only while it is still the saved key: a key saved meanwhile was checked when
            // it was saved, and is not this refusal
            let mut p = s.identify.lock().expect("identify lock");
            if s.acoustid.read().expect("acoustid lock").api_key == client.api_key() {
                p.acoustid_refused = Some(why);
            }
            return Ok(Vec::new());
        }
        Err(e) => return Err(e.to_string()),
    };
    if let Err(e) = s
        .library
        .lock()
        .expect("library poisoned")
        .save_lookup(&key, &raw)
    {
        tracing::debug!("identify: AcoustID answer not cached: {e}");
    }
    Ok(results)
}

/// The fingerprint kept for audio that can't be fingerprinted, before the reason (a real
/// fingerprint is URL-safe base64, never this).
const UNFINGERPRINTABLE: &str = "!";

/// A file's content key ([`Library::file_key`]): kept while the file's size and modification
/// time stay the same (one look at its metadata, nothing read), else read and kept.
async fn content_key(s: &NodeState, t: &Track) -> Result<String, String> {
    let path = t.file.path.clone();
    let stat = tokio::task::spawn_blocking({
        let p = path.clone();
        move || std::fs::metadata(&p)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{}: {e}", path.display()))?;
    let size = stat.len();
    let mtime = stat
        .modified()
        .ok()
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or_default();
    let kept = s
        .library
        .lock()
        .expect("library poisoned")
        .cached_file_key(&path, size, mtime)
        .ok()
        .flatten();
    if let Some(k) = kept {
        work(s, |w| w.file_keys_reused += 1);
        return Ok(k);
    }
    work(s, |w| w.file_reads += 1);
    let key = tokio::task::spawn_blocking({
        let p = path.clone();
        move || Library::file_key(&p)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{}: {e}", path.display()))?;
    // The cache saves work, nothing more: not kept while a scan writes (no wait for it)
    if !s.is_scanning() {
        if let Err(e) = s
            .library
            .lock()
            .expect("library poisoned")
            .save_file_key(&path, size, mtime, &key)
        {
            tracing::debug!("identify: content key not kept: {e}");
        }
    }
    Ok(key)
}

/// The key a MusicBrainz answer is kept under: the service's address, the path and the query
/// (no credentials: MusicBrainz takes none).
fn mb_key(s: &NodeState, path: &str, query: &[(&str, &str)]) -> String {
    let base = s.mb_base.read().expect("mb lock").clone();
    let q: Vec<String> = query.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{}/{path}?{}", base.trim_end_matches('/'), q.join("&"))
}

/// A MusicBrainz answer: the one an earlier run kept (within [`Library::IDENTIFY_MB_TTL_SECS`]),
/// else asked now (paced through the process's one gate) and kept. A failed request is not kept,
/// so the next run asks again.
async fn mb_get(
    s: &NodeState,
    mb: &pm_library::musicbrainz::MusicBrainzClient,
    path: &str,
    query: &[(&str, &str)],
) -> Result<Option<serde_json::Value>, String> {
    let key = mb_key(s, path, query);
    let since = pm_library::db::now_unix() - Library::IDENTIFY_MB_TTL_SECS;
    let kept = s
        .library
        .lock()
        .expect("library poisoned")
        .identify_mb_cached(&key, since)
        .ok()
        .flatten();
    if let Some(body) = kept {
        // A kept answer that no longer parses is asked again
        let parsed = match body {
            None => Some(None),
            Some(b) => serde_json::from_str::<serde_json::Value>(&b).ok().map(Some),
        };
        if let Some(v) = parsed {
            work(s, |w| w.mb_reused += 1);
            return Ok(v);
        }
    }
    work(s, |w| w.mb_asked += 1);
    let v = mb.get_json(path, query).await.map_err(|e| e.to_string())?;
    keep_mb(s, &key, v.as_ref());
    Ok(v)
}

/// Keep a MusicBrainz answer for later runs (only what identification reads of a release).
fn keep_mb(s: &NodeState, key: &str, v: Option<&serde_json::Value>) {
    // Not while a scan writes: an answer kept saves a request, never worth waiting for the scan
    if s.is_scanning() {
        return;
    }
    let body = v.map(|v| pm_library::album_fit::compact_answer(v).to_string());
    if let Err(e) = s
        .library
        .lock()
        .expect("library poisoned")
        .save_identify_mb(key, body.as_deref())
    {
        tracing::debug!("identify: MusicBrainz answer not kept: {e}");
    }
}

fn acoustid_client(
    s: &NodeState,
    key: &str,
    base_url: &str,
) -> Result<pm_library::identify::AcoustIdClient, ApiError> {
    let mut cfg = pm_library::identify::AcoustIdConfig::new(key, s.user_agent());
    cfg.base_url = base_url.to_string();
    pm_library::identify::AcoustIdClient::new(cfg).map_err(ApiError::from)
}

/// How long the job waits for a write while another connection holds the library (a scan
/// writing) before it stops; each try waits the connection's busy timeout first.
const BUSY_PATIENCE: std::time::Duration = std::time::Duration::from_secs(600);

/// A library write that waits through a busy database instead of ending the run (it stopped at
/// the first "database is locked" while a scan ran): tried again once the scan is done (or a
/// second later, when no scan runs), for up to [`BUSY_PATIENCE`].
async fn write_waiting<T>(
    s: &NodeState,
    write: impl Fn(&Library) -> pm_library::Result<T>,
) -> Result<T, ApiError> {
    let start = std::time::Instant::now();
    loop {
        let r = write(&s.library.lock().expect("library poisoned"));
        match r {
            Err(e) if e.is_busy() && start.elapsed() < BUSY_PATIENCE => {
                tracing::info!("identify waits for the library (another writer holds it): {e}");
                // Without the library meanwhile: a try holds it through SQLite's busy wait, and
                // every request that reads the library would wait with it
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    if !s.is_scanning() || start.elapsed() >= BUSY_PATIENCE {
                        break;
                    }
                }
            }
            Err(e) if e.is_busy() => {
                return Err(ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!(
                        "the library stayed busy for {} minutes (a scan writing?): {e}",
                        BUSY_PATIENCE.as_secs() / 60
                    ),
                ))
            }
            r => return r.map_err(ApiError::from),
        }
    }
}

/// MusicBrainz recordings matching the tags' artist and title (± 2 s): suggestions only, ranked
/// by [`rank_hits`]. A file with no artist gets none: a title alone matches strangers' songs.
async fn search_suggestions(s: &NodeState, t: &Track) -> Result<Vec<Candidate>, String> {
    let Some(mb) = mb_client(s, std::time::Duration::from_millis(1000)) else {
        return Ok(Vec::new());
    };
    // Asked now (the owner's "Which song is this?"): not from the library's kept answers
    search_suggestions_with(s, &mb, t, false).await
}

/// The score a tag match is stored with: an exact title, artist and length on MusicBrainz's
/// strongest hits names the song (and may be published), below a fingerprint's sure answer.
pub const TAG_MATCH_SCORE: f32 = 0.9;

/// [`search_suggestions`] with a client; `reuse` takes MusicBrainz's answers kept by earlier runs.
async fn search_suggestions_with(
    s: &NodeState,
    mb: &pm_library::musicbrainz::MusicBrainzClient,
    t: &Track,
    reuse: bool,
) -> Result<Vec<Candidate>, String> {
    Ok(rank_track_hits(t, search_hits(s, mb, t, reuse).await?))
}

/// The suggestions for a track from search hits ([`rank_hits`]).
fn rank_track_hits(t: &Track, hits: Vec<pm_library::musicbrainz::RecordingHit>) -> Vec<Candidate> {
    rank_hits(
        &t.title,
        t.album.as_ref().map(|a| a.title.as_str()),
        t.duration_ms,
        hits,
    )
}

/// MusicBrainz's recordings for a track's artist and title (± 2 s, then without the length);
/// `reuse` takes the answers kept by earlier runs.
async fn search_hits(
    s: &NodeState,
    mb: &pm_library::musicbrainz::MusicBrainzClient,
    t: &Track,
    reuse: bool,
) -> Result<Vec<pm_library::musicbrainz::RecordingHit>, String> {
    let artist = t.artists.first().map(|a| a.name.clone()).unwrap_or_default();
    if t.title.trim().is_empty() || artist.trim().is_empty() {
        return Ok(Vec::new());
    }
    let (narrow, wide) = pm_library::musicbrainz::recording_hit_queries(&artist, &t.title, t.duration_ms);
    let get = |q: String| async move {
        let query = [("query", q.as_str()), ("limit", "10")];
        let v = if reuse {
            mb_get(s, mb, "recording", &query).await?
        } else {
            work(s, |w| w.mb_asked += 1);
            let v = mb
                .get_json("recording", &query)
                .await
                .map_err(|e| e.to_string())?;
            keep_mb(s, &mb_key(s, "recording", &query), v.as_ref());
            v
        };
        Ok::<_, String>(
            v.map(|v| pm_library::musicbrainz::parse_search_hits(&v))
                .unwrap_or_default(),
        )
    };
    let mut found = match narrow {
        Some(q) => get(q).await?,
        None => Vec::new(),
    };
    if found.is_empty() {
        found = get(wide).await?;
    }
    Ok(found)
}

/// A title as people compare them: lower case, without "(Live)", "[Remastered]" or
/// " - 2011 Remaster", and with punctuation as spaces.
fn plain_title(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0u32;
    let s = s.split(" - ").next().unwrap_or(s);
    for c in s.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            c if depth == 0 => out.extend(if c.is_alphanumeric() {
                c.to_lowercase().collect::<Vec<_>>()
            } else {
                vec![' ']
            }),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether two titles name the same song: equal when plain, or one is the other with a few
/// words more ("Bohemian Rhapsody" and "Bohemian Rhapsody Live Aid").
fn same_title(a: &str, b: &str) -> Option<bool> {
    let (a, b) = (plain_title(a), plain_title(b));
    if a.is_empty() || b.is_empty() {
        return None;
    }
    if a == b {
        return Some(true);
    }
    let (short, long) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };
    (long.starts_with(short.as_str()) && short.len() * 10 >= long.len() * 6).then_some(false)
}

/// Rank MusicBrainz's hits for a file (second review #5, #6):
/// - a hit must carry the file's title (MusicBrainz also returns songs that only mention it);
/// - of the releases a recording is on, the one named like the file's album wins, else the
///   first studio release; bootlegs, live albums and compilations rank lower unless the file's
///   album is one;
/// - a length more than 3 s off, a title with extra words or another album lower the score.
fn rank_hits(
    title: &str,
    album: Option<&str>,
    duration_ms: Option<u64>,
    hits: Vec<pm_library::musicbrainz::RecordingHit>,
) -> Vec<Candidate> {
    let album = album.map(plain_title).filter(|a| !a.is_empty());
    let mut out: Vec<Candidate> = hits
        .into_iter()
        .filter(|h| h.score >= 80)
        .filter_map(|h| {
            let exact = same_title(title, &h.title)?;
            let near = match (h.length_ms, duration_ms) {
                (Some(a), Some(b)) => a.abs_diff(b) <= 3000,
                _ => true,
            };
            let on_album = album
                .as_ref()
                .and_then(|al| h.releases.iter().find(|r| plain_title(&r.title) == *al));
            let pick = on_album
                .or_else(|| h.releases.iter().find(|r| !r.is_secondary()))
                .or(h.releases.first());
            let (release, release_title) = match pick {
                Some(r) => (r.mbid, Some(r.title.clone())),
                None => (h.release_mbid, h.release.clone()),
            };
            let mut score = h.score as f32 / 100.0;
            if !near {
                score *= 0.7;
            }
            if !exact {
                score *= 0.9;
            }
            if album.is_some() && on_album.is_none() {
                score *= 0.9;
            }
            if on_album.is_none() && pick.is_some_and(|r| r.is_secondary()) {
                score *= 0.85;
            }
            Some(Candidate {
                recording: h.mbid,
                release,
                title: Some(h.title.clone()),
                artist: Some(h.artist.clone()),
                release_title,
                score,
                source: "search".into(),
            })
        })
        .collect();
    // Best first, ties by recording id: which five are kept never follows MusicBrainz's order
    out.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.recording.cmp(&b.recording))
    });
    // One choice per title, artist and release: identical-looking rows only confuse
    let mut seen = std::collections::HashSet::new();
    out.retain(|c| {
        seen.insert((
            c.title.as_deref().unwrap_or_default().to_lowercase(),
            c.artist.as_deref().unwrap_or_default().to_lowercase(),
            c.release_title.clone(),
        ))
    });
    out.truncate(5);
    out
}

/// `GET /v1/library/suggestions`: open suggestions with their tracks.
pub(crate) async fn suggestions(State(s): State<Arc<NodeState>>) -> ApiResult<Vec<SuggestionView>> {
    let lib = s.library.lock().expect("library poisoned");
    let rows = lib.suggestions()?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(track) = lib.get_track(pm_model::TrackId(row.track_id))? {
            out.push(SuggestionView { row, track });
        }
    }
    Ok(Json(out))
}

/// "Which song is this?" for one track: what MusicBrainz suggests, searched now.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrackCandidates {
    pub track: Track,
    pub suggestions: Vec<pm_library::SuggestionRow>,
    /// MusicBrainz is asked only with a contact (Settings, or Get started on Home).
    pub needs_contact: bool,
    /// What this search found (also for a song with a match, where nothing is stored; and for
    /// `q`, a search by the owner's words, never stored): picked with `…/tracks/{id}/match`.
    #[serde(default)]
    pub found: Vec<Candidate>,
}

/// `GET /v1/library/tracks/{id}/candidates?q=`: `q`, the owner's own words for MusicBrainz.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CandidatesQuery {
    #[serde(default)]
    pub q: Option<String>,
}

/// The longest search the owner may type.
const MAX_QUERY: usize = 200;

/// MusicBrainz's recordings for the owner's words (the "Wrong match?" search box): its strongest
/// hits, as options to pick from, never stored.
async fn words_search(s: &NodeState, q: &str) -> Result<Vec<Candidate>, String> {
    let Some(mb) = mb_client(s, std::time::Duration::from_millis(1000)) else {
        return Ok(Vec::new());
    };
    let query = [("query", q), ("limit", "10")];
    work(s, |w| w.mb_asked += 1);
    let v = mb
        .get_json("recording", &query)
        .await
        .map_err(|e| e.to_string())?;
    let hits = v
        .map(|v| pm_library::musicbrainz::parse_search_hits(&v))
        .unwrap_or_default();
    Ok(hits
        .into_iter()
        .filter(|h| h.score >= 50)
        .map(|h| Candidate {
            recording: h.mbid,
            release: h.release_mbid,
            title: Some(h.title),
            artist: Some(h.artist),
            release_title: h.release,
            score: h.score as f32 / 100.0,
            source: "search".into(),
        })
        .collect())
}

/// `GET /v1/library/tracks/{id}/candidates`: search MusicBrainz by the track's tags now (with
/// and, failing that, without its length) and list the suggestions to pick from; picking one is
/// `POST /v1/library/suggestions/{id}/accept`.
pub(crate) async fn track_candidates(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<i64>,
    axum::extract::Query(want): axum::extract::Query<CandidatesQuery>,
) -> ApiResult<TrackCandidates> {
    let track = s
        .library
        .lock()
        .expect("library poisoned")
        .get_track(pm_model::TrackId(id))?
        .ok_or_else(|| ApiError::not_found(format!("no track {id}")))?;
    let needs_contact = s.contact().trim().is_empty();
    let words = want
        .q
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(|q| q.chars().take(MAX_QUERY).collect::<String>());
    if let (false, Some(q)) = (needs_contact, words.as_deref()) {
        let found = words_search(&s, q)
            .await
            .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, format!("MusicBrainz: {e}")))?;
        let suggestions = s.library.lock().expect("library poisoned").suggestions_for(id)?;
        return Ok(Json(TrackCandidates {
            track,
            suggestions,
            needs_contact,
            found,
        }));
    }
    let mut found_now = Vec::new();
    if !needs_contact {
        let found = search_suggestions(&s, &track)
            .await
            .map_err(|e| ApiError::new(axum::http::StatusCode::BAD_GATEWAY, format!("MusicBrainz: {e}")))?;
        found_now = found.clone();
        // Stored like a run's answer: what the owner rejected stays out, and the song waits for
        // review only with something open (none: "unknown", so a retry looks at it again; a
        // song with an identity is left as it is)
        if !found.is_empty()
            || s.library
                .lock()
                .expect("library poisoned")
                .suggestions_for(id)?
                .is_empty()
        {
            s.library.lock().expect("library poisoned").settle(
                id,
                &pm_library::db::SettleAnswer::Suggest {
                    candidates: found,
                    input_key: None,
                    replace: false,
                },
            )?;
        }
    }
    let suggestions = s.library.lock().expect("library poisoned").suggestions_for(id)?;
    // What the owner rejected for the song is not offered again
    let found: Vec<Candidate> = {
        let lib = s.library.lock().expect("library poisoned");
        found_now
            .into_iter()
            .filter(|c| !lib.is_rejected(id, &c.recording.to_string()).unwrap_or(false))
            .collect()
    };
    Ok(Json(TrackCandidates {
        track,
        suggestions,
        needs_contact,
        found,
    }))
}

/// `POST /v1/library/suggestions/{id}/accept`: the owner confirms; the
/// track's identity becomes that recording (`confirmed_by_user`). A song that got an identity
/// meanwhile keeps it (409).
pub(crate) async fn accept(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<i64>,
    axum::extract::Query(want): axum::extract::Query<Expect>,
) -> ApiResult<Track> {
    let (track, t) = {
        let lib = s.library.lock().expect("library poisoned");
        if lib.check_suggestion(id, want.track_id, want.recording_mbid.as_deref())?
            == pm_library::db::SuggestionCheck::Changed
        {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "That suggestion changed since the page was loaded; nothing was confirmed. Reload to see it.",
            ));
        }
        let Some((track, _)) = lib.accept_suggestion(id)? else {
            if lib.suggestion_by_id(id)?.is_some() {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "This song got an identity meanwhile; it was kept. Reload to see it.",
                ));
            }
            return Err(ApiError::not_found(format!("no suggestion {id}")));
        };
        let t = lib
            .get_track(pm_model::TrackId(track))?
            .ok_or_else(|| ApiError::not_found("track gone"))?;
        (track, t)
    };
    if write_ids_on(&s) {
        let st = s.clone();
        let _ = tokio::task::spawn_blocking(move || write_found_ids(&st, Some(&[track]))).await;
    }
    let _ = s.events.send(crate::types::ApiEvent::LibraryChanged);
    Ok(Json(t))
}

/// Songs one batch confirmation takes at most (an album box set is a few hundred).
pub const MAX_CONFIRM: usize = 500;
/// Albums one page of album review lists at most.
pub const MAX_REVIEW_ALBUMS: usize = 200;
/// Songs one page of album review stops at (whole albums: a page has at least one, and an album
/// is never split, so a page may run over by the last album's songs).
pub const MAX_REVIEW_SONGS: usize = 200;
/// A song's only option is offered ticked from this score up (when it comes from the folder or the
/// fingerprint, never a search by tags alone).
pub const PRESELECT: f32 = 0.85;

/// One song in album review: its own options (best first, each with its evidence: source, score,
/// release), `single` (one option) or `conflict` (several recordings), and `pick`, the option
/// offered ticked (the owner still confirms it); a conflict is never picked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSong {
    pub track: ReviewTrack,
    pub options: Vec<SuggestionRow>,
    pub status: String,
    #[serde(default)]
    pub pick: Option<i64>,
    /// Why a single option is not ticked: `weak`, `search` or `other_release`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<String>,
    /// The song's match now ("Wrong match?" view only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<CurrentMatch>,
}

/// A song's match as the "Wrong match?" view shows it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CurrentMatch {
    pub recording_mbid: String,
    #[serde(default)]
    pub release_mbid: Option<String>,
    /// How it was found (`album_fit`, `fingerprint`, `tag_match`, `auto`, `confirmed_by_user`,
    /// `tagged_mbid`…).
    pub source: String,
    pub score: f32,
    /// Settled automatically: not written into the file until confirmed.
    pub auto: bool,
    /// The file's tags name a recording: corrected here only with "Write IDs into my files" on
    /// (the file's IDs are rewritten), else in the file.
    pub from_tags: bool,
}

/// A song as album review shows it: what the page needs of the track, not the whole record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewTrack {
    pub id: i64,
    pub title: String,
    pub artists: Vec<ReviewName>,
    #[serde(default)]
    pub track_no: Option<u32>,
    #[serde(default)]
    pub disc_no: Option<u32>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    pub file: ReviewFile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewName {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewFile {
    pub path: String,
}

impl From<&Track> for ReviewTrack {
    fn from(t: &Track) -> Self {
        Self {
            id: t.id.map(|i| i.0).unwrap_or_default(),
            title: t.title.clone(),
            artists: t
                .artists
                .iter()
                .map(|a| ReviewName { name: a.name.clone() })
                .collect(),
            track_no: t.track_no,
            disc_no: t.disc_no,
            duration_ms: t.duration_ms.or(t.file.duration_ms),
            file: ReviewFile {
                path: t.file.path.display().to_string(),
            },
        }
    }
}

/// The release at least half of an album's songs have an option on (shown, never chosen by).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRelease {
    pub mbid: String,
    #[serde(default)]
    pub title: Option<String>,
    pub songs: usize,
}

/// One folder of songs waiting for review, as an album.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewAlbum {
    /// The folder (the album's key in the page).
    pub key: String,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub release: Option<ReviewRelease>,
    pub songs: Vec<ReviewSong>,
    /// Songs offered ticked.
    pub ready: usize,
    /// Songs whose options disagree.
    pub conflicts: usize,
}

/// `GET /v1/library/suggestions/albums`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewPage {
    pub albums: Vec<ReviewAlbum>,
    /// Where the next page starts: ask for `after=` this folder (`None`: no more).
    #[serde(default)]
    pub next_after: Option<String>,
    /// Songs waiting, in every album.
    pub songs: usize,
    pub total_albums: usize,
    pub offset: usize,
    #[serde(default)]
    pub next: Option<usize>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReviewQuery {
    /// The albums after this folder (the last one shown): albums confirmed meanwhile make no
    /// page skip one. Takes the place of `offset`.
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
}

fn review_song(track: &Track, options: Vec<SuggestionRow>, release: Option<&str>) -> ReviewSong {
    let single = options.len() == 1;
    let (pick, hold) = match options.first().filter(|_| single) {
        None => (None, None),
        Some(o) if o.source == "search" => (None, Some("search")),
        Some(o) if o.score < PRESELECT => (None, Some("weak")),
        Some(o) if release.is_some() && o.release_mbid.as_deref().is_some_and(|r| Some(r) != release) => {
            (None, Some("other_release"))
        }
        Some(o) => (Some(o.id), None),
    };
    ReviewSong {
        track: ReviewTrack::from(track),
        options,
        status: if single { "single" } else { "conflict" }.into(),
        pick,
        hold: hold.map(str::to_string),
        current: None,
    }
}

/// The release at least half of the songs have an option on, when one stands out.
fn album_release(songs: &[(Track, Vec<SuggestionRow>)]) -> Option<ReviewRelease> {
    let mut on: BTreeMap<&str, (usize, Option<&str>)> = BTreeMap::new();
    for (_, options) in songs {
        let mut seen = std::collections::HashSet::new();
        for o in options {
            if let Some(r) = o.release_mbid.as_deref().filter(|r| seen.insert(*r)) {
                let e = on.entry(r).or_default();
                e.0 += 1;
                e.1 = e.1.or(o.release_title.as_deref());
            }
        }
    }
    let mut ranked: Vec<(&str, (usize, Option<&str>))> = on.into_iter().collect();
    ranked.sort_by_key(|(_, (n, _))| std::cmp::Reverse(*n));
    let (mbid, (n, title)) = *ranked.first()?;
    if ranked.get(1).is_some_and(|x| x.1 .0 == n) || n * 2 < songs.len() {
        return None;
    }
    Some(ReviewRelease {
        mbid: mbid.to_string(),
        title: title.map(str::to_string),
        songs: n,
    })
}

/// `GET /v1/library/suggestions/albums?offset&limit`: open suggestions as albums (a folder each),
/// a page of at most [`MAX_REVIEW_ALBUMS`] albums (50 by default).
pub(crate) async fn review_albums(
    State(s): State<Arc<NodeState>>,
    axum::extract::Query(q): axum::extract::Query<ReviewQuery>,
) -> ApiResult<ReviewPage> {
    // Grouped from ids and paths alone; only the page's songs are read whole (a backlog of
    // thousands answers as quickly as a few)
    let paths = s
        .library
        .lock()
        .expect("library poisoned")
        .suggested_track_paths()?;
    let songs = paths.len();
    let mut by_folder: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    for (id, path) in paths {
        let folder = std::path::Path::new(&path)
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        by_folder.entry(folder).or_default().push(id);
    }
    let total_albums = by_folder.len();
    let offset = match &q.after {
        Some(after) => by_folder
            .range::<String, _>((
                std::ops::Bound::Excluded(after.clone()),
                std::ops::Bound::Unbounded,
            ))
            .count(),
        None => 0,
    };
    let offset = match &q.after {
        Some(_) => total_albums - offset,
        None => q.offset.unwrap_or(0).min(total_albums),
    };
    let limit = q.limit.unwrap_or(50).clamp(1, MAX_REVIEW_ALBUMS);
    let mut page: Vec<(String, Vec<i64>)> = Vec::new();
    let mut on_page = 0;
    for (folder, ids) in by_folder.into_iter().skip(offset) {
        if page.len() >= limit || (!page.is_empty() && on_page >= MAX_REVIEW_SONGS) {
            break;
        }
        on_page += ids.len();
        page.push((folder, ids));
    }
    let page_len = page.len();
    let last_key = page.last().map(|(k, _)| k.clone());
    let (tracks, rows) = {
        let lib = s.library.lock().expect("library poisoned");
        let ids: Vec<i64> = page.iter().flat_map(|(_, ids)| ids.iter().copied()).collect();
        let mut tracks: HashMap<i64, Track> = HashMap::new();
        for id in &ids {
            if let Some(t) = lib.get_track(pm_model::TrackId(*id))? {
                tracks.insert(*id, t);
            }
        }
        (tracks, lib.suggestions_for_tracks(&ids)?)
    };
    let mut options: HashMap<i64, Vec<SuggestionRow>> = HashMap::new();
    for r in rows {
        options.entry(r.track_id).or_default().push(r);
    }
    let albums: Vec<ReviewAlbum> = page
        .into_iter()
        .filter_map(|(key, ids)| {
            let mut list: Vec<(Track, Vec<SuggestionRow>)> = ids
                .iter()
                .filter_map(|id| Some((tracks.get(id)?.clone(), options.remove(id)?)))
                .collect();
            if list.is_empty() {
                return None;
            }
            list.sort_by(|(a, _), (b, _)| {
                (a.disc_no.unwrap_or(1), a.track_no.unwrap_or(u32::MAX), &a.title).cmp(&(
                    b.disc_no.unwrap_or(1),
                    b.track_no.unwrap_or(u32::MAX),
                    &b.title,
                ))
            });
            let release = album_release(&list);
            let first = &list[0].0;
            let album = first.album.as_ref().map(|a| a.title.clone());
            let artist = first
                .album
                .as_ref()
                .and_then(|a| a.album_artist.clone())
                .or_else(|| first.artists.first().map(|a| a.name.clone()));
            let songs: Vec<ReviewSong> = list
                .into_iter()
                .map(|(t, o)| review_song(&t, o, release.as_ref().map(|r| r.mbid.as_str())))
                .collect();
            Some(ReviewAlbum {
                key,
                album,
                artist,
                ready: songs.iter().filter(|x| x.pick.is_some()).count(),
                conflicts: songs.iter().filter(|x| x.status == "conflict").count(),
                release,
                songs,
            })
        })
        .collect();
    let shown = offset + page_len;
    let next = (shown < total_albums).then_some(shown);
    let next_after = next.and(last_key);
    Ok(Json(ReviewPage {
        albums,
        next_after,
        songs,
        total_albums,
        offset,
        next,
    }))
}

/// `GET /v1/library/identify/fix?tracks=1,2,3`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FixQuery {
    /// Song ids, comma-separated (at most [`MAX_CONFIRM`]).
    #[serde(default)]
    pub tracks: String,
}

/// A song's match as [`CurrentMatch`] says it (`None`: no match).
fn current_match(t: &Track, release: Option<String>) -> Option<CurrentMatch> {
    let (uri, c) = t.canonical.as_ref()?;
    let recording_mbid = match uri {
        MusicUri::Recording { mbid } => mbid.to_string(),
        other => other.to_url(),
    };
    Some(CurrentMatch {
        recording_mbid,
        release_mbid: release,
        source: c.label().to_string(),
        score: c.score(),
        auto: matches!(c, Confidence::Auto { .. }),
        from_tags: t.recording_mbid.is_some() || matches!(c, Confidence::TaggedMbid | Confidence::TaggedIsrc),
    })
}

/// `GET /v1/library/identify/fix?tracks=`: "Wrong match?" for these songs (one song, or an
/// album's): the album review's shape, a folder per album, each song with its match now
/// (`current`) and its open options (an automatic pick keeps the others it was chosen from).
/// More are found with `…/tracks/{id}/candidates`; a choice is `POST …/tracks/{id}/match`.
pub(crate) async fn fix_view(
    State(s): State<Arc<NodeState>>,
    axum::extract::Query(q): axum::extract::Query<FixQuery>,
) -> ApiResult<ReviewPage> {
    let mut ids: Vec<i64> = Vec::new();
    for part in q.tracks.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let id = part
            .parse::<i64>()
            .map_err(|_| ApiError::bad_request("tracks: song ids, comma-separated"))?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return Err(ApiError::bad_request("tracks: name at least one song"));
    }
    if ids.len() > MAX_CONFIRM {
        return Err(ApiError::bad_request(format!(
            "At most {MAX_CONFIRM} songs at once"
        )));
    }
    let (tracks, rows) = {
        let lib = s.library.lock().expect("library poisoned");
        let mut tracks = Vec::new();
        for id in &ids {
            if let Some(t) = lib.get_track(pm_model::TrackId(*id))? {
                let release = lib.identify_release(*id)?;
                tracks.push((t, release));
            }
        }
        (tracks, lib.suggestions_for_tracks(&ids)?)
    };
    let mut options: HashMap<i64, Vec<SuggestionRow>> = HashMap::new();
    for r in rows {
        options.entry(r.track_id).or_default().push(r);
    }
    let mut by_folder: BTreeMap<String, Vec<(Track, Option<String>)>> = BTreeMap::new();
    for (t, release) in tracks {
        let folder = t
            .file
            .path
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        by_folder.entry(folder).or_default().push((t, release));
    }
    let songs: usize = by_folder.values().map(Vec::len).sum();
    let albums: Vec<ReviewAlbum> = by_folder
        .into_iter()
        .map(|(key, mut list)| {
            list.sort_by(|(a, _), (b, _)| {
                (a.disc_no.unwrap_or(1), a.track_no.unwrap_or(u32::MAX), &a.title).cmp(&(
                    b.disc_no.unwrap_or(1),
                    b.track_no.unwrap_or(u32::MAX),
                    &b.title,
                ))
            });
            let first = &list[0].0;
            let album = first.album.as_ref().map(|a| a.title.clone());
            let artist = first
                .album
                .as_ref()
                .and_then(|a| a.album_artist.clone())
                .or_else(|| first.artists.first().map(|a| a.name.clone()));
            let songs: Vec<ReviewSong> = list
                .into_iter()
                .map(|(t, release)| {
                    let id = t.id.map(|i| i.0).unwrap_or_default();
                    let opts = options.remove(&id).unwrap_or_default();
                    let current = current_match(&t, release);
                    let mut song = review_song(&t, opts, None);
                    // Nothing is offered ticked here: the owner picks
                    song.pick = None;
                    song.hold = None;
                    if current.is_some() {
                        song.status = "current".into();
                    }
                    song.current = current;
                    song
                })
                .collect();
            ReviewAlbum {
                key,
                album,
                artist,
                release: None,
                ready: 0,
                conflicts: songs.iter().filter(|x| x.status == "conflict").count(),
                songs,
            }
        })
        .collect();
    let total_albums = albums.len();
    Ok(Json(ReviewPage {
        albums,
        next_after: None,
        songs,
        total_albums,
        offset: 0,
        next: None,
    }))
}

/// `POST /v1/library/tracks/{id}/match`: the owner's correction ("Wrong match?").
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MatchBody {
    /// The recording that is this song (another one, or the current one to keep it).
    #[serde(default)]
    pub recording_mbid: Option<String>,
    /// Its release, when the owner picked an option that names one.
    #[serde(default)]
    pub release_mbid: Option<String>,
    /// "Not matched": none of them.
    #[serde(default)]
    pub none: bool,
    /// The recording the page showed as the song's match (absent: no match).
    #[serde(default)]
    pub expect: Option<String>,
}

/// `POST /v1/library/tracks/{id}/match {recording_mbid? | none, release_mbid?, expect}`: the
/// owner corrects a song's match. Another recording becomes the song's (`confirmed_by_user`), the
/// current one kept becomes the owner's (an automatic pick turns into theirs), or "Not matched"
/// takes the match away and leaves the song as it is (out of later runs). The recording replaced
/// or taken away is rejected for the song: no run, retry or automatic settling brings it back.
/// 409 when the song's match is no longer `expect`, or when the file's tags name a recording and
/// "Write IDs into my files" is off (the next scan would read it back). With Write IDs on, the
/// file's ID tags are replaced or removed (backed up first); off, the file is not touched. Pubky tags already published on the old recording stay on it.
pub(crate) async fn set_match(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<i64>,
    Json(body): Json<MatchBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let parse = |v: &str, what: &str| {
        Mbid::parse(v).map_err(|_| ApiError::bad_request(format!("{what}: not a MusicBrainz id")))
    };
    let choice = match (body.recording_mbid.as_deref(), body.none) {
        (Some(r), false) => Some((
            parse(r, "recording_mbid")?,
            body.release_mbid
                .as_deref()
                .map(|r| parse(r, "release_mbid"))
                .transpose()?,
        )),
        (None, true) => None,
        _ => {
            return Err(ApiError::bad_request(
                "Name the recording (recording_mbid) or say none of them (none: true)",
            ))
        }
    };
    let expect = body
        .expect
        .as_deref()
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(str::to_string);
    // With "Write IDs into my files" on, the file's IDs follow the correction, also for a song
    // whose tags name a recording; off, such a song's tags would bring the old one back
    let follow = write_ids_on(&s);
    let st = s.clone();
    let out = tokio::task::spawn_blocking(move || correct(&st, id, expect, choice, follow))
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))??;
    let _ = s.events.send(crate::types::ApiEvent::LibraryChanged);
    Ok(Json(out))
}

/// How long a correction waits for a scan going to end before it gives up (409).
const CORRECTION_WAIT: std::time::Duration = if cfg!(test) {
    std::time::Duration::from_secs(2)
} else {
    std::time::Duration::from_secs(30)
};

/// One ID write into a file at a time (a correction, "Write IDs" after a run or on the page), each
/// checking what the song is right before it writes: two writers never leave a file naming what
/// the other one replaced.
static ID_WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn id_writes() -> std::sync::MutexGuard<'static, ()> {
    ID_WRITES.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether a song's match is still `recording`, found by identification or the owner (not an
/// automatic pick, not its tags): checked by every ID writer right before it writes.
pub(crate) fn still_matched(s: &NodeState, id: i64, recording: &Mbid) -> bool {
    s.library
        .lock()
        .expect("library poisoned")
        .get_track(pm_model::TrackId(id))
        .ok()
        .flatten()
        .and_then(|now| now.canonical)
        .is_some_and(|(u, c)| {
            u == MusicUri::Recording { mbid: *recording }
                && !matches!(
                    c,
                    Confidence::Auto { .. } | Confidence::TaggedMbid | Confidence::TaggedIsrc
                )
        })
}

/// What a correction answers when the song's match is not what the page showed, or can't be
/// changed here.
fn not_corrected(r: &pm_library::Rematch, id: i64) -> ApiError {
    match r {
        pm_library::Rematch::Gone => ApiError::not_found(format!("no track {id}")),
        pm_library::Rematch::FromTags => ApiError::new(
            StatusCode::CONFLICT,
            "This song's file names its recording in its tags: turn on Write IDs into my files to change it here, or correct the tags.",
        ),
        _ => ApiError::new(
            StatusCode::CONFLICT,
            "This song's match changed since the page was loaded; nothing was changed. Reload to see it.",
        ),
    }
}

/// [`set_match`]'s work (blocking), one correction at a time with every ID write: checked
/// against the library, then (`follow`) the file's IDs replaced or removed, then the library
/// changed in one step that checks again. A file whose tags name a recording and that can't take
/// the change changes nothing (the next scan would read the old recording back); when the
/// library's step finds the song changed meanwhile, the file is put back from its backup.
fn correct(
    s: &NodeState,
    id: i64,
    expect: Option<String>,
    choice: Option<(Mbid, Option<Mbid>)>,
    follow: bool,
) -> Result<serde_json::Value, ApiError> {
    use pm_library::Rematch;
    let _one = id_writes();
    // No scan alongside it: a scan that read the file before could store the old recording after
    // the correction. It waits for the scan gate a while; scans wait for it in turn
    let _gate = {
        let since = std::time::Instant::now();
        loop {
            match s.scan_gate.try_lock() {
                Ok(g) => break g,
                Err(std::sync::TryLockError::Poisoned(e)) => break e.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) if since.elapsed() < CORRECTION_WAIT => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(std::sync::TryLockError::WouldBlock) => {
                    return Err(ApiError::new(
                        StatusCode::CONFLICT,
                        "A library scan is going; nothing was changed. Try again when it is done.",
                    ))
                }
            }
        }
    };
    let (track, now, release_kept, choice) = {
        let lib = s.library.lock().expect("library poisoned");
        // A release only when the folder's own fit names it (the album lane's option): a search
        // hit's first release is MusicBrainz's answer order, no evidence of the edition
        let choice = match choice {
            Some((r, Some(rel))) if !lib.folder_fit_names(id, &r, &rel)? => Some((r, None)),
            c => c,
        };
        (
            lib.get_track(pm_model::TrackId(id))?,
            lib.match_now(id)?,
            lib.identify_release(id)?,
            choice,
        )
    };
    let (Some(track), Some((current, from_tags))) = (track, now) else {
        return Err(not_corrected(&Rematch::Gone, id));
    };
    if current.as_deref() != expect.as_deref() {
        return Err(not_corrected(&Rematch::Changed, id));
    }
    if from_tags && !follow {
        return Err(not_corrected(&Rematch::FromTags, id));
    }
    let dir = backup_dir(s);
    let mut backup: Option<PathBuf> = None;
    let mut skipped: Option<String> = None;
    if follow {
        let kept = choice.is_some_and(|(r, _)| current.as_deref() == Some(r.to_string().as_str()));
        let ids = choice.map(|(recording, release)| pm_library::tagwrite::Ids {
            recording,
            release: release.or_else(|| {
                kept.then(|| release_kept.as_deref().and_then(|r| Mbid::parse(r).ok()))
                    .flatten()
            }),
            acoustid: track.acoustid.clone(),
        });
        // A song left without a match: an ISRC in its tags would name it again
        let drop_isrc = choice.is_none() && track.isrc.is_some();
        match pm_library::tagwrite::replace_ids(&track.file.path, ids.as_ref(), drop_isrc, &dir) {
            Ok(b) => backup = Some(b),
            Err(e) if from_tags => {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    format!("The IDs in this song's file couldn't be changed ({e}), so nothing was: correct the tags in the file."),
                ))
            }
            Err(e) => skipped = Some(e.to_string()),
        }
    }
    // The rewritten file as the scan will find it, noted in the correction's own step: the
    // rewrite is no change of the song's, so the owner's choice (a match, or none) stands
    let stamp = backup.as_ref().and_then(|_| {
        let path = &track.file.path;
        let meta = std::fs::metadata(path).ok()?;
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
            .max(pm_library::lyrics::sidecar_mtime(path).unwrap_or(0));
        Some((path.as_path(), meta.len(), mtime))
    });
    let done = s.library.lock().expect("library poisoned").rematch(
        id,
        expect.as_deref(),
        choice.as_ref().map(|(r, rel)| (r, rel.as_ref())),
        follow,
        stamp,
    );
    let old = match done {
        Ok(Rematch::Done { old }) => old,
        other => {
            // The file goes back to what it was
            if let Some(b) = &backup {
                let back = std::fs::read(b)
                    .ok()
                    .and_then(|j| serde_json::from_slice::<pm_library::tagwrite::Backup>(&j).ok());
                match back.map(|b| pm_library::tagwrite::restore(&b)) {
                    Some(Ok(())) => {}
                    Some(Err(e)) => {
                        tracing::warn!("identify: a file's IDs not put back after a refused correction: {e}")
                    }
                    None => {
                        tracing::warn!("identify: a file's ID backup unreadable after a refused correction")
                    }
                }
            }
            return Err(match other {
                Ok(r) => not_corrected(&r, id),
                Err(e) => ApiError::from(e),
            });
        }
    };
    let track = s
        .library
        .lock()
        .expect("library poisoned")
        .get_track(pm_model::TrackId(id))?
        .ok_or_else(|| ApiError::not_found("track gone"))?;
    let mut out = serde_json::json!({"track": track, "replaced": old});
    if follow {
        out["files"] = serde_json::json!({
            "written": backup.is_some(),
            "skipped": skipped,
            "backup_dir": dir.display().to_string(),
        });
    }
    Ok(out)
}

/// `POST /v1/library/suggestions/confirm {choices: [{track_id, suggestion_id, recording_mbid}]}`.
#[derive(Debug, Clone, Deserialize)]
pub struct ConfirmBody {
    pub choices: Vec<pm_library::ConfirmChoice>,
}

/// Most albums one "Leave as is" names.
const MAX_SKIP_ALBUMS: usize = 500;
/// Most songs one "Leave as is" names by id.
const MAX_SKIP_TRACKS: usize = 5_000;

/// `POST /v1/library/suggestions/skip` and `…/unskip`: which songs, exactly one of: albums of the
/// review page (their `key`), song ids, or every one.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SkipBody {
    #[serde(default)]
    pub albums: Vec<String>,
    #[serde(default)]
    pub track_ids: Vec<i64>,
    #[serde(default)]
    pub all: bool,
}

/// The songs a [`SkipBody`] names among `paths` (songs and their file's path).
fn skip_targets(body: &SkipBody, paths: Vec<(i64, String)>) -> Result<Vec<i64>, ApiError> {
    let given = [!body.albums.is_empty(), !body.track_ids.is_empty(), body.all];
    if given.iter().filter(|g| **g).count() != 1 {
        return Err(ApiError::bad_request(
            "Name the songs one way: albums, track_ids or all",
        ));
    }
    if body.albums.len() > MAX_SKIP_ALBUMS || body.track_ids.len() > MAX_SKIP_TRACKS {
        return Err(ApiError::bad_request(format!(
            "At most {MAX_SKIP_ALBUMS} albums or {MAX_SKIP_TRACKS} songs at once"
        )));
    }
    if !body.track_ids.is_empty() {
        return Ok(body.track_ids.clone());
    }
    let albums: std::collections::HashSet<&str> = body.albums.iter().map(String::as_str).collect();
    Ok(paths
        .into_iter()
        .filter(|(_, path)| {
            body.all
                || albums.contains(
                    std::path::Path::new(path)
                        .parent()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                        .as_str(),
                )
        })
        .map(|(id, _)| id)
        .collect())
}

/// `POST /v1/library/suggestions/skip {albums | track_ids | all}`: "Leave as is" / "Skip all
/// unsure". The songs stay without an identity, out of review and of later runs; they play and
/// show as before, and nothing is written into files. Undone by `…/unskip`.
pub(crate) async fn skip(
    State(s): State<Arc<NodeState>>,
    Json(body): Json<SkipBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let paths = if body.track_ids.is_empty() {
        s.library
            .lock()
            .expect("library poisoned")
            .suggested_track_paths()?
    } else {
        Vec::new()
    };
    let ids = skip_targets(&body, paths)?;
    let n = write_waiting(&s, |lib| lib.skip_review(&ids)).await?;
    let counts = s.library.lock().expect("library poisoned").identity_counts()?;
    let _ = s.events.send(crate::types::ApiEvent::LibraryChanged);
    Ok(Json(serde_json::json!({"skipped": n, "counts": counts})))
}

/// `POST /v1/library/suggestions/unskip {albums | track_ids | all}`: the songs left as they are
/// wait for review again, with their options.
pub(crate) async fn unskip(
    State(s): State<Arc<NodeState>>,
    Json(body): Json<SkipBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let paths = if body.track_ids.is_empty() {
        s.library
            .lock()
            .expect("library poisoned")
            .skipped_track_paths()?
    } else {
        Vec::new()
    };
    let ids = skip_targets(&body, paths)?;
    let n = write_waiting(&s, |lib| lib.unskip_review(&ids)).await?;
    let counts = s.library.lock().expect("library poisoned").identity_counts()?;
    let _ = s.events.send(crate::types::ApiEvent::LibraryChanged);
    Ok(Json(serde_json::json!({"restored": n, "counts": counts})))
}

/// `POST /v1/library/suggestions/confirm`: the owner confirms the songs they chose, at most
/// [`MAX_CONFIRM`]. All are stored in one transaction, or none: a choice that went stale (the
/// suggestion gone or changed, the song identified meanwhile, a song twice) is a 409 listing them.
pub(crate) async fn confirm(
    State(s): State<Arc<NodeState>>,
    Json(body): Json<ConfirmBody>,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    if body.choices.is_empty() {
        return Err(ApiError::bad_request("Choose at least one song to confirm"));
    }
    if body.choices.len() > MAX_CONFIRM {
        return Err(ApiError::bad_request(format!(
            "At most {MAX_CONFIRM} songs at once"
        )));
    }
    let report = s
        .library
        .lock()
        .expect("library poisoned")
        .confirm_suggestions(&body.choices)?;
    match report {
        pm_library::ConfirmReport::Stale(stale) => Ok((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "Some songs changed since the page was loaded, so nothing was confirmed. Reload and check them again.",
                "stale": stale,
            })),
        )
            .into_response()),
        pm_library::ConfirmReport::Confirmed(tracks) => {
            // The identities are stored; writing the IDs into the files ("Write IDs" on) comes
            // after, and what it couldn't do is said, not hidden (it undoes nothing)
            let files = if write_ids_on(&s) {
                let st = s.clone();
                let only = tracks.clone();
                match tokio::task::spawn_blocking(move || write_found_ids(&st, Some(&only))).await {
                    Ok(Ok(r)) => Some(serde_json::json!({"written": r.written, "skipped": r.skipped})),
                    Ok(Err(e)) => Some(serde_json::json!({"written": 0, "skipped": [], "error": e.message})),
                    Err(e) => Some(serde_json::json!({"written": 0, "skipped": [], "error": e.to_string()})),
                }
            } else {
                None
            };
            let _ = s.events.send(crate::types::ApiEvent::LibraryChanged);
            let mut out = serde_json::json!({"confirmed": tracks, "count": tracks.len()});
            if let Some(f) = files {
                out["files"] = f;
            }
            Ok(Json(out).into_response())
        }
    }
}

/// `DELETE /v1/library/suggestions/{id}`: not this one.
pub(crate) async fn reject(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<i64>,
    axum::extract::Query(want): axum::extract::Query<Expect>,
) -> Result<StatusCode, ApiError> {
    let check = s
        .library
        .lock()
        .expect("library poisoned")
        .reject_suggestion_checked(id, want.track_id, want.recording_mbid.as_deref())?;
    match check {
        pm_library::db::SuggestionCheck::Holds => Ok(StatusCode::NO_CONTENT),
        pm_library::db::SuggestionCheck::Changed => Err(ApiError::new(
            StatusCode::CONFLICT,
            "That suggestion changed since the page was loaded; nothing was rejected. Reload to see it.",
        )),
        pm_library::db::SuggestionCheck::Gone => Err(ApiError::not_found(format!("no suggestion {id}"))),
    }
}

/// What the caller saw of a suggestion (`?track_id&recording_mbid`, both optional): a single
/// accept or "Not this" whose suggestion is now for another song or recording is refused (409).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Expect {
    #[serde(default)]
    pub track_id: Option<i64>,
    #[serde(default)]
    pub recording_mbid: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_library::musicbrainz::{HitRelease, RecordingHit};
    use pm_model::Mbid;

    const R1: &str = "b1a9c0e9-d987-4042-ae91-78d6a3267d69";
    const R2: &str = "0383dadf-2a4e-4d10-a46a-e9e041da8eb3";
    const R3: &str = "5b11f4ce-a62d-471e-81fc-a69a8278c7da";

    fn rel(title: &str, id: &str, status: &str, kinds: &[&str]) -> HitRelease {
        HitRelease {
            title: title.into(),
            mbid: Mbid::parse(id).ok(),
            status: status.into(),
            kinds: kinds.iter().map(|k| k.to_string()).collect(),
        }
    }

    fn hit(id: &str, title: &str, length_ms: u64, releases: Vec<HitRelease>) -> RecordingHit {
        RecordingHit {
            mbid: Mbid::parse(id).unwrap(),
            score: 100,
            title: title.into(),
            artist: "Queen".into(),
            release: releases.first().map(|r| r.title.clone()),
            release_mbid: releases.first().and_then(|r| r.mbid),
            length_ms: Some(length_ms),
            disambiguation: None,
            releases,
        }
    }

    #[test]
    fn the_files_album_and_studio_releases_come_first() {
        // Second review #5: a live bootleg first, "Greatest Hits II" above "A Night at the Opera"
        let hits = vec![
            hit(
                R1,
                "Death on Two Legs",
                223_000,
                vec![rel("Ultimate Houston 1977", R1, "Bootleg", &["Album", "Live"])],
            ),
            hit(
                R2,
                "Death on Two Legs (Dedicated to…)",
                225_000,
                vec![
                    rel("Greatest Hits II", R2, "Official", &["Album", "Compilation"]),
                    rel("A Night at the Opera", R3, "Official", &["Album"]),
                ],
            ),
        ];
        let c = rank_hits(
            "Death on Two Legs",
            Some("A Night at the Opera"),
            Some(223_000),
            hits,
        );
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].recording, Mbid::parse(R2).unwrap());
        assert_eq!(c[0].release_title.as_deref(), Some("A Night at the Opera"));
        assert_eq!(c[0].release, Mbid::parse(R3).ok());
        assert!(c[1].score < c[0].score);
        assert_eq!(c[1].release_title.as_deref(), Some("Ultimate Houston 1977"));
        // Without an album tag, the studio release is picked over the compilation
        let hits = vec![hit(
            R2,
            "Death on Two Legs",
            223_000,
            vec![
                rel("Greatest Hits II", R2, "Official", &["Album", "Compilation"]),
                rel("A Night at the Opera", R3, "Official", &["Album"]),
            ],
        )];
        let c = rank_hits("Death on Two Legs", None, Some(223_000), hits);
        assert_eq!(c[0].release_title.as_deref(), Some("A Night at the Opera"));
        assert_eq!(c[0].score, 1.0);
    }

    #[test]
    fn a_hit_must_carry_the_files_title() {
        // Second review #6: a long, unrelated title that only mentions the file's
        let hits = vec![
            hit(
                R1,
                "HOLY SHIT EACH EAR HAS ITS OWN TRACK?! (fuck mono audio)",
                5_000,
                vec![],
            ),
            hit(R2, "Mono Audio", 5_000, vec![]),
            hit(R3, "Mono Audio Test Tone For Speakers", 5_000, vec![]),
        ];
        let c = rank_hits("Mono audio", None, Some(5_000), hits);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].recording, Mbid::parse(R2).unwrap());
    }

    #[test]
    fn plain_titles() {
        assert_eq!(
            plain_title("Bohemian Rhapsody - Remastered 2011"),
            "bohemian rhapsody"
        );
        assert_eq!(
            plain_title("Get Lucky (Radio Edit) [feat. Pharrell]"),
            "get lucky"
        );
        assert_eq!(same_title("Don't Stop Me Now", "Don’t Stop Me Now"), Some(true));
        assert_eq!(
            same_title("Bohemian Rhapsody", "Bohemian Rhapsody (Live Aid)"),
            Some(true)
        );
        assert_eq!(same_title("Love", "Love of My Life"), None);
    }
}
