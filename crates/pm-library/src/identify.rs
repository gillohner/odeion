//! Identifying local files so they join Pinakes (steward identify brief).
//!
//! Strongest evidence first, each stored with its [`Confidence`]:
//! 1. IDs embedded in tags ([`crate::canonical::assign`], at scan time);
//! 2. the folder as an album: one release for the whole folder, each file
//!    matched by position, length and title ([`crate::album_fit`]);
//! 3. for the rest, a Chromaprint fingerprint looked up on AcoustID: one recording at or
//!    above [`Confidence::FINGERPRINT_ACCEPT`] is accepted; several are
//!    disambiguated by the **folder as a release** (Picard's approach: the
//!    release whose track count, positions and durations fit the folder);
//! 4. otherwise the candidates are kept as *suggestions* for the owner to
//!    confirm, never used as identity on their own.
//!
//! The pure decision logic lives here and is tested against AcoustID-shaped
//! answers; the HTTP client is [`AcoustIdClient`] (feature `musicbrainz`).

use pm_model::{Confidence, Mbid};
use serde::{Deserialize, Serialize};

/// Durations within this many seconds count as the same take.
pub const DURATION_SLACK_SECS: f64 = 3.0;

// ---- AcoustID lookup answer (`meta=recordings releasegroups releases tracks`) ----

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct LookupResponse {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub results: Vec<AcoustIdResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<LookupError>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct LookupError {
    #[serde(default)]
    pub code: i64,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AcoustIdResult {
    pub id: String,
    pub score: f32,
    #[serde(default)]
    pub recordings: Vec<AcRecording>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AcRecording {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    /// Seconds.
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub artists: Vec<AcArtist>,
    #[serde(default)]
    pub releases: Vec<AcRelease>,
    #[serde(default)]
    pub releasegroups: Vec<AcReleaseGroup>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AcArtist {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AcReleaseGroup {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    /// The group's primary type (`Album`, `Single`, `EP`…), when AcoustID says.
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Its secondary types (`Compilation`, `Live`…).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secondarytypes: Vec<String>,
    #[serde(default)]
    pub releases: Vec<AcRelease>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AcRelease {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub track_count: Option<u32>,
    #[serde(default)]
    pub medium_count: Option<u32>,
    #[serde(default)]
    pub mediums: Vec<AcMedium>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AcMedium {
    #[serde(default)]
    pub position: u32,
    #[serde(default)]
    pub track_count: Option<u32>,
    #[serde(default)]
    pub tracks: Vec<AcTrack>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AcTrack {
    #[serde(default)]
    pub position: u32,
    #[serde(default)]
    pub id: Option<String>,
}

impl AcRecording {
    /// Releases whether AcoustID nested them under release groups or not.
    pub fn all_releases(&self) -> impl Iterator<Item = &AcRelease> {
        self.releases
            .iter()
            .chain(self.releasegroups.iter().flat_map(|g| g.releases.iter()))
    }

    pub fn artist(&self) -> Option<String> {
        (!self.artists.is_empty()).then(|| {
            self.artists
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
    }
}

impl AcRelease {
    /// Total tracks on the release (every medium).
    pub fn total_tracks(&self) -> Option<u32> {
        self.track_count.or_else(|| {
            let sum: u32 = self.mediums.iter().filter_map(|m| m.track_count).sum();
            (sum > 0).then_some(sum)
        })
    }
}

// ---- what we know about the file and its folder ----

/// The file's own position and length, from tags and the decoder.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FileFacts {
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub duration_secs: Option<f64>,
}

fn duration_fits(rec: &AcRecording, file: &FileFacts) -> bool {
    match (rec.duration, file.duration_secs) {
        (Some(a), Some(b)) => (a - b).abs() <= DURATION_SLACK_SECS,
        // Nothing to compare: do not let a missing length rule a release out.
        _ => true,
    }
}

/// Whether `rel` looks like the folder: same total track count and this
/// file at its disc/track position on it.
pub fn release_fits(rel: &AcRelease, file: &FileFacts, folder_len: usize) -> bool {
    let Some(track_no) = file.track_no else {
        return false;
    };
    if folder_len > 0 && rel.total_tracks() != Some(folder_len as u32) {
        return false;
    }
    let disc = file.disc_no.unwrap_or(1);
    rel.mediums
        .iter()
        .filter(|m| m.position == disc || (rel.mediums.len() == 1 && m.position == 0))
        .any(|m| m.tracks.iter().any(|t| t.position == track_no))
}

/// The release the folder most likely is: the one the most files fit at
/// their positions (strong matches only). `None` when no release fits at
/// least half of the folder, or two releases tie.
pub fn best_folder_release(files: &[(FileFacts, &[AcoustIdResult])], folder_len: usize) -> Option<String> {
    let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
    for (facts, results) in files {
        let mut seen: std::collections::BTreeSet<&str> = Default::default();
        for r in results
            .iter()
            .filter(|r| r.score >= Confidence::FINGERPRINT_ACCEPT)
        {
            for rec in &r.recordings {
                if !duration_fits(rec, facts) {
                    continue;
                }
                for rel in rec.all_releases() {
                    if release_fits(rel, facts, folder_len) && seen.insert(rel.id.as_str()) {
                        *counts.entry(rel.id.as_str()).or_default() += 1;
                    }
                }
            }
        }
    }
    let mut ranked: Vec<(&str, usize)> = counts.into_iter().collect();
    ranked.sort_by_key(|x| std::cmp::Reverse(x.1));
    let (best, n) = *ranked.first()?;
    if ranked.get(1).is_some_and(|x| x.1 == n) {
        return None;
    }
    let need = folder_len.div_ceil(2).max(1);
    (n >= need).then(|| best.to_string())
}

/// The releases the folder may be: the one of [`best_folder_release`]; or, when several tie,
/// all of them provided they put the same recordings at every file's place (editions that differ
/// only in what doesn't change the recordings: a remaster, another country's pressing). Tied
/// editions that disagree on any file decide nothing (empty).
pub fn best_folder_releases(files: &[(FileFacts, &[AcoustIdResult])], folder_len: usize) -> Vec<String> {
    if let Some(one) = best_folder_release(files, folder_len) {
        return vec![one];
    }
    let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
    for (facts, results) in files {
        let mut seen: std::collections::BTreeSet<&str> = Default::default();
        for rec in strong_fitting(results, facts) {
            for rel in rec.all_releases() {
                if release_fits(rel, facts, folder_len) && seen.insert(rel.id.as_str()) {
                    *counts.entry(rel.id.as_str()).or_default() += 1;
                }
            }
        }
    }
    let Some(top) = counts.values().max().copied() else {
        return Vec::new();
    };
    if top < folder_len.div_ceil(2).max(1) {
        return Vec::new();
    }
    let tied: Vec<&str> = counts
        .iter()
        .filter(|(_, n)| **n == top)
        .map(|(r, _)| *r)
        .collect();
    // The recordings each tied release puts at each file's place
    let at =
        |rel: &str, facts: &FileFacts, results: &[AcoustIdResult]| -> std::collections::BTreeSet<String> {
            strong_fitting(results, facts)
                .filter(|rec| {
                    rec.all_releases()
                        .any(|r| r.id == rel && release_fits(r, facts, folder_len))
                })
                .map(|rec| rec.id.clone())
                .collect()
        };
    let same = files.iter().all(|(facts, results)| {
        let first = at(tied[0], facts, results);
        tied[1..].iter().all(|r| at(r, facts, results) == first)
    });
    if same {
        tied.into_iter().map(str::to_string).collect()
    } else {
        Vec::new()
    }
}

/// The strong recordings whose length fits the file.
fn strong_fitting<'a>(
    results: &'a [AcoustIdResult],
    facts: &'a FileFacts,
) -> impl Iterator<Item = &'a AcRecording> {
    results
        .iter()
        .filter(|r| r.score >= Confidence::FINGERPRINT_ACCEPT)
        .flat_map(|r| r.recordings.iter())
        .filter(move |rec| duration_fits(rec, facts))
}

/// Bump when identification's decisions change enough that "not found" songs are worth checking
/// again (part of [`input_key`]).
pub const INPUT_VERSION: u32 = 2;

/// What a song looked like to identification: its file (path, size, time), the tags it is matched
/// by, and the node's services as the caller describes them (`services`: their addresses, whether
/// MusicBrainz can be asked, a hash of the AcoustID key, never the key). Only this hash is stored.
/// A "not found" song whose key changes is checked again by a plain run.
pub fn input_key(t: &pm_model::Track, services: &str) -> String {
    let artists: Vec<&str> = t.artists.iter().map(|a| a.name.as_str()).collect();
    let album = t.album.as_ref();
    let text = format!(
        "v{INPUT_VERSION}|{services}|{}|{}|{}|{}|{}|{}|{}|{}|{:?}|{:?}|{:?}",
        t.file.path.display(),
        t.file.size_bytes,
        t.file.mtime_unix,
        t.title,
        artists.join("\u{1f}"),
        album.map(|a| a.title.as_str()).unwrap_or_default(),
        album.and_then(|a| a.album_artist.as_deref()).unwrap_or_default(),
        album
            .and_then(|a| a.release_mbid)
            .map(|m| m.to_string())
            .unwrap_or_default(),
        t.disc_no,
        t.track_no,
        t.duration_ms.or(t.file.duration_ms),
    );
    blake3::hash(text.as_bytes()).to_hex()[..32].to_string()
}

// ---- the decision ----

/// One candidate recording for the owner to confirm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub recording: Mbid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<Mbid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_title: Option<String>,
    pub score: f32,
    /// `fingerprint` (AcoustID), `search` (MusicBrainz by tags) or `album` (the folder as a
    /// release, when the fingerprint names another song).
    pub source: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Canonical: one recording, by fingerprint (and maybe the release).
    Identified {
        recording: Mbid,
        release: Option<Mbid>,
        confidence: Confidence,
        acoustid: String,
    },
    /// Several candidates (or only weak ones): the owner decides.
    Suggest(Vec<Candidate>),
    /// AcoustID knows nothing about this audio, or knows it without a
    /// MusicBrainz recording (the caller then tries a search by tags).
    Unknown,
}

struct Seen<'a> {
    rec: &'a AcRecording,
    score: f32,
    acoustid: &'a str,
}

fn distinct(results: &[AcoustIdResult], min: f32) -> Vec<Seen<'_>> {
    let mut out: Vec<Seen> = Vec::new();
    for r in results.iter().filter(|r| r.score >= min) {
        for rec in &r.recordings {
            if Mbid::parse(&rec.id).is_err() {
                continue;
            }
            match out.iter_mut().find(|s| s.rec.id == rec.id) {
                Some(s) if s.score < r.score => {
                    s.score = r.score;
                    s.acoustid = &r.id;
                }
                Some(_) => {}
                None => out.push(Seen {
                    rec,
                    score: r.score,
                    acoustid: &r.id,
                }),
            }
        }
    }
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out
}

/// Whether the recording fits the file's place on some release (`None`: on none), and the
/// release when that is settled: the folder's when the recording is on it, else the only one
/// that fits. Several that fit and none preferred: the release stays open (the first in
/// AcoustID's answer is no evidence).
fn fitting_release<'a>(
    rec: &'a AcRecording,
    file: &FileFacts,
    folder_len: usize,
    preferred: Option<&str>,
) -> Option<Option<&'a AcRelease>> {
    if !duration_fits(rec, file) {
        return None;
    }
    let mut fits: Vec<&AcRelease> = Vec::new();
    for rel in rec
        .all_releases()
        .filter(|rel| release_fits(rel, file, folder_len))
    {
        if !fits.iter().any(|f| f.id == rel.id) {
            fits.push(rel);
        }
    }
    if fits.is_empty() {
        return None;
    }
    if let Some(on) = preferred.and_then(|p| fits.iter().find(|rel| rel.id == p)) {
        return Some(Some(*on));
    }
    Some((fits.len() == 1).then(|| fits[0]))
}

fn candidate(s: &Seen, release: Option<&AcRelease>) -> Candidate {
    Candidate {
        recording: Mbid::parse(&s.rec.id).expect("filtered"),
        release: release.and_then(|r| Mbid::parse(&r.id).ok()),
        title: s.rec.title.clone(),
        artist: s.rec.artist(),
        release_title: release.and_then(|r| r.title.clone()),
        score: s.score,
        source: "fingerprint".into(),
    }
}

/// Decide what AcoustID's answer means for one file. `folder_len` is the
/// number of audio files in its folder; `preferred_release` the folder's
/// release from [`best_folder_release`], when there is one.
pub fn decide(
    results: &[AcoustIdResult],
    file: &FileFacts,
    folder_len: usize,
    preferred_release: Option<&str>,
) -> Decision {
    decide_core(results, file, folder_len, preferred_release, None)
}

/// [`decide`], and with the file's tags: several strong recordings that are all the file's song
/// (one audio, several MusicBrainz recordings) are not a question ([`same_audio`]).
fn decide_core(
    results: &[AcoustIdResult],
    file: &FileFacts,
    folder_len: usize,
    preferred_release: Option<&str>,
    tags: Option<&crate::same_song::FileTags>,
) -> Decision {
    let strong = distinct(results, Confidence::FINGERPRINT_ACCEPT);
    if strong.is_empty() {
        let weak = distinct(results, 0.5);
        if weak.is_empty() {
            return Decision::Unknown;
        }
        return Decision::Suggest(weak.iter().take(5).map(|s| candidate(s, None)).collect());
    }
    // Each recording: whether it fits the file's place on a release, and which release when
    // that is settled
    type Fit<'a> = Option<Option<&'a AcRelease>>;
    let with_release: Vec<(&Seen, Fit)> = strong
        .iter()
        .map(|s| (s, fitting_release(s.rec, file, folder_len, preferred_release)))
        .collect();
    let identified = |s: &Seen, fit: Fit| Decision::Identified {
        recording: Mbid::parse(&s.rec.id).expect("filtered"),
        release: fit.flatten().and_then(|r| Mbid::parse(&r.id).ok()),
        confidence: if fit.is_some() {
            Confidence::FingerprintRelease { score: s.score }
        } else {
            Confidence::Fingerprint { score: s.score }
        },
        acoustid: s.acoustid.to_string(),
    };
    if with_release.len() == 1 {
        let (s, fit) = with_release[0];
        return identified(s, fit);
    }
    // Several recordings fit the audio: the folder decides, when it can.
    let on_preferred: Vec<&(&Seen, Fit)> = with_release
        .iter()
        .filter(|(_, fit)| {
            fit.flatten()
                .is_some_and(|r| Some(r.id.as_str()) == preferred_release)
        })
        .collect();
    if on_preferred.len() == 1 {
        let (s, fit) = on_preferred[0];
        return identified(s, *fit);
    }
    let fitting: Vec<&(&Seen, Fit)> = with_release.iter().filter(|(_, fit)| fit.is_some()).collect();
    if fitting.len() == 1 {
        let (s, fit) = fitting[0];
        return identified(s, *fit);
    }
    // One audio under several recordings of the file's song: the most logical one
    if let Some(s) = tags.and_then(|t| same_audio(&strong, file, folder_len, t)) {
        let fit = fitting_release(s.rec, file, folder_len, preferred_release);
        return identified(s, fit);
    }
    Decision::Suggest(
        with_release
            .iter()
            .take(5)
            .map(|(s, fit)| candidate(s, fit.flatten()))
            .collect(),
    )
}

/// [`decide`] with the folder's releases from [`best_folder_releases`]: with tied editions that
/// agree on the recordings, the one strong recording at the file's place on them is identified
/// (`fingerprint+release`, the edition left open); anything else is decided as before.
pub fn decide_among(
    results: &[AcoustIdResult],
    file: &FileFacts,
    folder_len: usize,
    preferred: &[String],
) -> Decision {
    if preferred.len() <= 1 {
        return decide(results, file, folder_len, preferred.first().map(String::as_str));
    }
    let strong = distinct(results, Confidence::FINGERPRINT_ACCEPT);
    let on: Vec<&Seen> = strong
        .iter()
        .filter(|s| {
            duration_fits(s.rec, file)
                && s.rec
                    .all_releases()
                    .any(|r| preferred.contains(&r.id) && release_fits(r, file, folder_len))
        })
        .collect();
    if let [s] = on[..] {
        return Decision::Identified {
            recording: Mbid::parse(&s.rec.id).expect("filtered"),
            release: None,
            confidence: Confidence::FingerprintRelease { score: s.score },
            acoustid: s.acoustid.to_string(),
        };
    }
    decide(results, file, folder_len, None)
}

/// [`decide_among`] with the file's tags: several strong recordings that are all the file's song
/// (one audio under several MusicBrainz recordings) are not a question; the most logical one is
/// identified.
pub fn decide_with_tags(
    results: &[AcoustIdResult],
    file: &FileFacts,
    folder_len: usize,
    preferred: &[String],
    tags: Option<&crate::same_song::FileTags>,
) -> Decision {
    if preferred.len() <= 1 {
        return decide_core(
            results,
            file,
            folder_len,
            preferred.first().map(String::as_str),
            tags,
        );
    }
    match decide_among(results, file, folder_len, preferred) {
        d @ Decision::Identified { .. } => d,
        _ => decide_core(results, file, folder_len, None, tags),
    }
}

/// The recording to identify when every strong recording is the file's song (its title, version
/// and artist, a length within 3 s): one audio under several MusicBrainz recordings. A recording
/// only on live releases is set aside for a file that isn't live. In order: one that fits the
/// folder at the file's place, one on a release named like the file's album, one on the plainest
/// release (studio album, then single or EP), one on most releases, the lowest id. `None` when
/// any strong recording is another song, another version or another artist, or has no title.
fn same_audio<'a>(
    strong: &'a [Seen<'a>],
    file: &FileFacts,
    folder_len: usize,
    tags: &crate::same_song::FileTags,
) -> Option<&'a Seen<'a>> {
    use crate::same_song::{
        album_says_version, file_is_live, named_like_album, plain_kind, same_artist, same_song,
    };
    let live_file = file_is_live(tags);
    let named = |s: &Seen| {
        s.rec
            .all_releases()
            .any(|r| r.title.as_deref().is_some_and(|t| named_like_album(tags, t)))
            || s.rec
                .releasegroups
                .iter()
                .any(|g| g.title.as_deref().is_some_and(|t| named_like_album(tags, t)))
    };
    let mut pool: Vec<&Seen> = Vec::new();
    for s in strong {
        // Full evidence for every recording: its title and version, a length within 3 s, the
        // file's artist; anything missing or different and the owner decides
        let title = s.rec.title.as_deref()?;
        let near = matches!((s.rec.duration, file.duration_secs), (Some(a), Some(b)) if (a - b).abs() <= DURATION_SLACK_SECS);
        let artist = s.rec.artist()?;
        if !same_song(tags, title) || !near || !same_artist(&tags.artist, &artist) {
            return None;
        }
        let groups = &s.rec.releasegroups;
        let live = |g: &AcReleaseGroup| g.secondarytypes.iter().any(|t| t.eq_ignore_ascii_case("live"));
        let live_only = !groups.is_empty() && groups.iter().all(live);
        let live_ok = live_file
            || groups
                .iter()
                .any(|g| live(g) && g.title.as_deref().is_some_and(|t| named_like_album(tags, t)));
        if live_only && !live_ok {
            continue;
        }
        pool.push(s);
    }
    let any_named = pool.iter().any(|s| named(s));
    // An album that says it is another version: only a recording on that very release
    if album_says_version(tags) && !any_named {
        return None;
    }
    // Several, none on the file's album: only when their titles say the same
    if !any_named && pool.len() > 1 {
        let title = |s: &Seen| s.rec.title.clone().unwrap_or_default().to_lowercase();
        if pool.iter().any(|s| title(s) != title(pool[0])) {
            return None;
        }
    }
    let key = |s: &Seen| {
        let fits = fitting_release(s.rec, file, folder_len, None).is_some();
        let plain = s
            .rec
            .releasegroups
            .iter()
            .map(|g| plain_kind(g.kind.as_deref(), &g.secondarytypes))
            .max()
            .unwrap_or(0);
        (
            std::cmp::Reverse(fits),
            std::cmp::Reverse(named(s)),
            std::cmp::Reverse(plain),
            std::cmp::Reverse(s.rec.all_releases().count()),
        )
    };
    pool.into_iter()
        .min_by(|a, b| key(a).cmp(&key(b)).then_with(|| a.rec.id.cmp(&b.rec.id)))
}

// ---- the AcoustID web service ----

#[cfg(feature = "musicbrainz")]
pub use client::{AcoustIdClient, AcoustIdConfig};

#[cfg(feature = "musicbrainz")]
mod client {
    use super::*;
    use crate::{LibraryError, Result};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::sync::Mutex;

    pub const DEFAULT_BASE: &str = "https://api.acoustid.org";

    /// AcoustID's error code for an application key it doesn't know ("invalid API key").
    const INVALID_KEY: i64 = 4;

    #[derive(Debug, Clone)]
    pub struct AcoustIdConfig {
        pub base_url: String,
        /// The application key (acoustid.org/new-application).
        pub api_key: String,
        /// AcoustID allows three requests a second.
        pub min_interval: Duration,
        pub user_agent: String,
    }

    impl AcoustIdConfig {
        pub fn new(api_key: impl Into<String>, user_agent: impl Into<String>) -> Self {
            Self {
                base_url: DEFAULT_BASE.into(),
                api_key: api_key.into(),
                min_interval: Duration::from_millis(340),
                user_agent: user_agent.into(),
            }
        }
    }

    #[derive(Clone)]
    pub struct AcoustIdClient {
        http: reqwest::Client,
        config: AcoustIdConfig,
        last: Arc<Mutex<Option<Instant>>>,
    }

    impl AcoustIdClient {
        pub fn new(config: AcoustIdConfig) -> Result<Self> {
            let http = reqwest::Client::builder()
                .user_agent(config.user_agent.clone())
                .timeout(Duration::from_secs(20))
                .build()
                .map_err(|e| LibraryError::AcoustId(e.to_string()))?;
            Ok(Self {
                http,
                config,
                last: Arc::new(Mutex::new(None)),
            })
        }

        /// The application key this client sends.
        pub fn api_key(&self) -> &str {
            &self.config.api_key
        }

        /// Look a fingerprint up. Returns the raw JSON (for the cache) and
        /// the parsed results.
        pub async fn lookup(
            &self,
            fingerprint: &str,
            duration_secs: u64,
        ) -> Result<(String, Vec<AcoustIdResult>)> {
            {
                let mut last = self.last.lock().await;
                if let Some(t) = *last {
                    let e = t.elapsed();
                    if e < self.config.min_interval {
                        tokio::time::sleep(self.config.min_interval - e).await;
                    }
                }
                *last = Some(Instant::now());
            }
            let url = format!("{}/v2/lookup", self.config.base_url.trim_end_matches('/'));
            let duration = duration_secs.to_string();
            let form = [
                ("client", self.config.api_key.as_str()),
                ("format", "json"),
                ("meta", "recordings releasegroups releases tracks compress"),
                ("duration", duration.as_str()),
                ("fingerprint", fingerprint),
            ];
            let body: String = url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(form)
                .finish();
            let resp = self
                .http
                .post(&url)
                .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(body)
                .send()
                .await
                .map_err(|e| LibraryError::AcoustId(e.to_string()))?;
            let status = resp.status();
            let body = resp
                .text()
                .await
                .map_err(|e| LibraryError::AcoustId(e.to_string()))?;
            let parsed: LookupResponse =
                serde_json::from_str(&body).map_err(|e| LibraryError::AcoustId(format!("{status}: {e}")))?;
            if parsed.status != "ok" {
                return Err(match parsed.error {
                    Some(e) if e.code == INVALID_KEY => LibraryError::AcoustIdKey(e.message),
                    Some(e) => LibraryError::AcoustId(format!("{} (code {})", e.message, e.code)),
                    None => LibraryError::AcoustId(status.to_string()),
                });
            }
            Ok((body, parsed.results))
        }

        /// Whether AcoustID refuses this client's key: one lookup of a fingerprint too short to
        /// match anything. `Some(why)` only for that answer; an AcoustID out of reach, or any
        /// other answer, says nothing about the key.
        pub async fn key_refused(&self) -> Option<String> {
            match self.lookup("AQAAAA", 30).await {
                Err(LibraryError::AcoustIdKey(why)) => Some(why),
                _ => None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<AcoustIdResult> {
        let s = std::fs::read_to_string(format!(
            "{}/tests/fixtures/acoustid/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let r: LookupResponse = serde_json::from_str(&s).unwrap();
        assert_eq!(r.status, "ok");
        r.results
    }

    const BOHEMIAN: &str = "b1a9c0e9-d987-4042-ae91-78d6a3267d69";
    const BOHEMIAN_DUP: &str = "ebf79ba5-085e-48d2-9eb8-2d992fbf0f6d";
    const OPERA_1975: &str = "1a5b6a49-4c4f-3d18-9e3d-1c4d1b0a7a3e";

    #[test]
    fn one_strong_recording_is_identified() {
        let r = fixture("single.json");
        let file = FileFacts {
            track_no: Some(11),
            disc_no: Some(1),
            duration_secs: Some(355.0),
        };
        match decide(&r, &file, 1, None) {
            Decision::Identified {
                recording,
                confidence,
                acoustid,
                ..
            } => {
                assert_eq!(recording.to_string(), BOHEMIAN);
                assert!(matches!(confidence, Confidence::Fingerprint { score } if score > 0.95));
                assert_eq!(acoustid, "9ff43b6a-4f16-427c-93c2-92307ca505e0");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn below_the_threshold_is_only_a_suggestion() {
        let mut r = fixture("single.json");
        r[0].score = 0.89;
        match decide(&r, &FileFacts::default(), 1, None) {
            Decision::Suggest(c) => {
                assert_eq!(c.len(), 1);
                assert_eq!(c[0].recording.to_string(), BOHEMIAN);
            }
            other => panic!("0.89 must not be identified: {other:?}"),
        }
        r[0].score = 0.9;
        assert!(matches!(
            decide(&r, &FileFacts::default(), 1, None),
            Decision::Identified { .. }
        ));
        assert_eq!(decide(&[], &FileFacts::default(), 1, None), Decision::Unknown);
    }

    #[test]
    fn several_recordings_are_suggestions_without_a_folder() {
        let r = fixture("multi.json");
        let file = FileFacts {
            track_no: None,
            disc_no: None,
            duration_secs: Some(355.0),
        };
        match decide(&r, &file, 0, None) {
            Decision::Suggest(c) => {
                let ids: Vec<String> = c.iter().map(|c| c.recording.to_string()).collect();
                assert!(ids.contains(&BOHEMIAN.to_string()) && ids.contains(&BOHEMIAN_DUP.to_string()));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_folder_as_a_release_disambiguates() {
        let r = fixture("multi.json");
        // A 12-track folder; this file is track 11 on disc 1 and 5:55 long:
        // only the 1975 album (12 tracks, Bohemian Rhapsody at 11) fits.
        let file = FileFacts {
            track_no: Some(11),
            disc_no: Some(1),
            duration_secs: Some(355.0),
        };
        let folder: Vec<(FileFacts, &[AcoustIdResult])> = vec![(file, &r[..])];
        // One file alone is not half of a 12-track folder.
        assert_eq!(best_folder_release(&folder, 12), None);
        match decide(&r, &file, 12, None) {
            Decision::Identified {
                recording,
                release,
                confidence,
                ..
            } => {
                assert_eq!(recording.to_string(), BOHEMIAN);
                assert_eq!(release.unwrap().to_string(), OPERA_1975);
                assert!(matches!(confidence, Confidence::FingerprintRelease { .. }));
            }
            other => panic!("{other:?}"),
        }
        // Wrong position: nothing fits, so the owner chooses.
        let elsewhere = FileFacts {
            track_no: Some(3),
            ..file
        };
        assert!(matches!(decide(&r, &elsewhere, 12, None), Decision::Suggest(_)));
        // A length off by more than 3 s rules a take out.
        let longer = FileFacts {
            duration_secs: Some(366.0),
            ..file
        };
        assert!(matches!(decide(&r, &longer, 12, None), Decision::Suggest(_)));
    }

    #[test]
    fn the_folder_release_wins_by_majority() {
        let r = fixture("multi.json");
        let mut files = Vec::new();
        let facts: Vec<FileFacts> = (0..7)
            .map(|_| FileFacts {
                track_no: Some(11),
                disc_no: Some(1),
                duration_secs: Some(355.0),
            })
            .collect();
        for f in &facts {
            files.push((*f, &r[..]));
        }
        assert_eq!(best_folder_release(&files, 12).as_deref(), Some(OPERA_1975));
        match decide(&r, &facts[0], 12, Some(OPERA_1975)) {
            Decision::Identified { release, .. } => assert_eq!(release.unwrap().to_string(), OPERA_1975),
            other => panic!("{other:?}"),
        }
    }

    /// A real AcoustID answer (recorded with the node's key): it parses, and a file at track 11
    /// of a 12-file folder is that recording on a 12-track release of A Night at the Opera. Many
    /// 12-track editions have it at 11, so which one stays open (owner, 2026-11-03: the first in
    /// AcoustID's answer is no evidence); the folder's other songs settle it ([`decide_among`]).
    #[test]
    fn a_recorded_answer_parses_and_finds_the_release() {
        let r = fixture("recorded-bohemian-trackid.json");
        assert_eq!(r.len(), 1);
        let rec = &r[0].recordings[0];
        assert_eq!(rec.id, BOHEMIAN);
        assert!(rec.releasegroups.len() > 100);
        let file = FileFacts {
            track_no: Some(11),
            disc_no: Some(1),
            duration_secs: Some(355.0),
        };
        let mut fitting: Vec<&str> = rec
            .all_releases()
            .filter(|x| release_fits(x, &file, 12))
            .map(|x| x.id.as_str())
            .collect();
        fitting.sort_unstable();
        fitting.dedup();
        assert!(fitting.len() > 1, "several 12-track editions: {}", fitting.len());
        match decide(&r, &file, 12, None) {
            Decision::Identified {
                recording,
                release,
                confidence,
                acoustid,
            } => {
                assert_eq!(recording.to_string(), BOHEMIAN);
                assert!(matches!(confidence, Confidence::FingerprintRelease { .. }));
                assert_eq!(acoustid, "84eed5cd-5b1f-4f5c-8c92-2686d2877862");
                assert_eq!(release, None, "which edition stays open");
            }
            other => panic!("{other:?}"),
        }
        // The folder pointing at one of them settles it
        match decide(&r, &file, 12, Some(fitting[0])) {
            Decision::Identified { release, .. } => assert_eq!(release.unwrap().to_string(), fitting[0]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn suggestions_never_count_as_identity() {
        let c = Confidence::Fuzzy { score: 0.99 };
        assert!(!c.publishable());
        assert_eq!(c.label(), "suggested");
        assert!(Confidence::ConfirmedByUser.publishable());
        assert!(Confidence::FingerprintRelease { score: 0.92 }.publishable());
        assert_eq!(
            Confidence::FingerprintRelease { score: 0.92 }.label(),
            "fingerprint+release"
        );
    }

    const R1: &str = "11111111-1111-4111-8111-111111111111";
    const R2: &str = "22222222-2222-4222-8222-222222222222";
    const R9: &str = "99999999-9999-4999-8999-999999999999";

    fn on(id: &str, total: u32, pos: u32) -> AcRelease {
        AcRelease {
            id: id.into(),
            title: Some(format!("Release {}", &id[..2])),
            track_count: Some(total),
            mediums: vec![AcMedium {
                position: 1,
                track_count: Some(total),
                tracks: vec![AcTrack {
                    position: pos,
                    id: None,
                }],
            }],
            ..Default::default()
        }
    }

    fn recording(i: u32, releases: Vec<AcRelease>) -> AcRecording {
        AcRecording {
            id: format!("00000000-0000-4000-8000-{i:012}"),
            title: Some(format!("Song {i}")),
            duration: Some(200.0 + i as f64),
            releases,
            ..Default::default()
        }
    }

    fn facts(i: u32) -> FileFacts {
        FileFacts {
            track_no: Some(i),
            disc_no: Some(1),
            duration_secs: Some(200.0 + i as f64),
        }
    }

    /// Two editions tie for the folder (every file sits at its place on both): when both put the
    /// same recording at every file's place, the folder still says which recording each file is,
    /// and a third release that only fits by chance is ruled out. When the editions disagree on
    /// any file, nothing is decided by them.
    #[test]
    fn tied_editions_with_the_same_recordings_still_decide() {
        let folder = |second: &dyn Fn(u32) -> u32| -> Vec<Vec<AcoustIdResult>> {
            (1..=4)
                .map(|i| {
                    let mut recs = vec![recording(i, vec![on(R1, 4, i)])];
                    let j = second(i);
                    if j == i {
                        recs[0].releases.push(on(R2, 4, i));
                    } else {
                        // Another recording as long, at the same place on the second edition
                        let mut other = recording(j, vec![on(R2, 4, i)]);
                        other.duration = Some(200.0 + i as f64);
                        recs.push(other);
                    }
                    if i == 1 {
                        // Another recording as long, first on a 4-track best-of
                        let mut chance = recording(50, vec![on(R9, 4, 1)]);
                        chance.duration = Some(201.0);
                        recs.push(chance);
                    }
                    vec![AcoustIdResult {
                        id: format!("acoustid-{i}"),
                        score: 0.95,
                        recordings: recs,
                    }]
                })
                .collect()
        };
        let same = folder(&|i| i);
        let pairs: Vec<(FileFacts, &[AcoustIdResult])> = same
            .iter()
            .enumerate()
            .map(|(k, r)| (facts(k as u32 + 1), &r[..]))
            .collect();
        // The old rule: a tie decides nothing
        assert_eq!(best_folder_release(&pairs, 4), None);
        let preferred = best_folder_releases(&pairs, 4);
        assert_eq!(preferred, vec![R1.to_string(), R2.to_string()]);
        match decide_among(&same[0], &facts(1), 4, &preferred) {
            Decision::Identified {
                recording,
                release,
                confidence,
                ..
            } => {
                assert_eq!(recording.to_string(), "00000000-0000-4000-8000-000000000001");
                assert_eq!(release, None, "which edition stays open");
                assert!(matches!(confidence, Confidence::FingerprintRelease { .. }));
            }
            other => panic!("{other:?}"),
        }
        // Without the folder the chance fit makes it a choice (as before)
        assert!(matches!(
            decide(&same[0], &facts(1), 4, None),
            Decision::Suggest(_)
        ));
        // The second edition has another recording at place 3: the tie decides nothing
        let differ = folder(&|i| if i == 3 { 33 } else { i });
        let pairs: Vec<(FileFacts, &[AcoustIdResult])> = differ
            .iter()
            .enumerate()
            .map(|(k, r)| (facts(k as u32 + 1), &r[..]))
            .collect();
        assert!(best_folder_releases(&pairs, 4).is_empty());
        assert!(matches!(
            decide_among(&differ[0], &facts(1), 4, &[]),
            Decision::Suggest(_)
        ));
        assert!(matches!(
            decide_among(&differ[2], &facts(3), 4, &[]),
            Decision::Suggest(_)
        ));
        // One release ahead: as before
        let one: Vec<(FileFacts, &[AcoustIdResult])> =
            vec![(facts(1), &same[0][..]), (facts(2), &same[1][..])];
        assert_eq!(
            best_folder_releases(&one, 2),
            Vec::<String>::new(),
            "a 4-track release is no 2-file folder"
        );
    }

    /// Without a release the folder points at, a recording on several releases that each fit the
    /// file's place gets no release: the first in AcoustID's answer is no evidence.
    #[test]
    fn several_fitting_releases_leave_the_release_open() {
        let results = vec![AcoustIdResult {
            id: "a".into(),
            score: 0.97,
            recordings: vec![recording(1, vec![on(R1, 4, 1), on(R2, 4, 1)])],
        }];
        match decide(&results, &facts(1), 4, None) {
            Decision::Identified {
                release, confidence, ..
            } => {
                assert_eq!(release, None);
                assert!(matches!(confidence, Confidence::FingerprintRelease { .. }));
            }
            other => panic!("{other:?}"),
        }
        // One that fits: that one
        let results = vec![AcoustIdResult {
            id: "a".into(),
            score: 0.97,
            recordings: vec![recording(1, vec![on(R1, 4, 1), on(R2, 5, 1)])],
        }];
        assert!(matches!(
            decide(&results, &facts(1), 4, None),
            Decision::Identified { release: Some(r), .. } if r.to_string() == R1
        ));
    }

    fn grouped(
        i: u32,
        title: &str,
        kind: &str,
        secondary: &[&str],
        rel_title: &str,
        total: u32,
        pos: u32,
    ) -> AcRecording {
        serde_json::from_value(serde_json::json!({
            "id": format!("00000000-0000-4000-8000-{i:012}"), "title": title, "duration": 232.0,
            "artists": [{"name": "Mike + The Mechanics"}],
            "releasegroups": [{"id": format!("g{i}"), "title": rel_title, "type": kind, "secondarytypes": secondary,
                "releases": [{"id": format!("00000000-0000-4000-9000-{i:012}"), "title": rel_title, "track_count": total,
                    "mediums": [{"position": 1, "track_count": total, "tracks": [{"position": pos}]}]}]}]
        }))
        .unwrap()
    }

    /// REV2: one fingerprint, several recordings of the same song (same title, version and
    /// artist as the file): not a question for the owner. The most logical one is identified;
    /// recordings of other songs keep it a question; a live-only one is not the studio file's.
    #[test]
    fn one_audio_under_several_recordings_is_identified() {
        let tags = crate::same_song::FileTags {
            title: "Over My Shoulder".into(),
            artist: "Mike + The Mechanics".into(),
            album: Some("Beggar on a Beach of Gold".into()),
            duration_ms: Some(232_000),
        };
        let file = FileFacts {
            track_no: Some(2),
            disc_no: Some(1),
            duration_secs: Some(232.0),
        };
        let answer = |recs: Vec<AcRecording>| {
            vec![AcoustIdResult {
                id: "a".into(),
                score: 0.96,
                recordings: recs,
            }]
        };
        let same = answer(vec![
            grouped(3, "Over My Shoulder", "Album", &["Compilation"], "Hits", 18, 3),
            grouped(
                2,
                "Over My Shoulder",
                "Album",
                &[],
                "Beggar on a Beach of Gold",
                10,
                2,
            ),
            grouped(1, "Over My Shoulder", "Single", &[], "Over My Shoulder", 2, 1),
        ]);
        // Without the file's tags: a question, as before
        assert!(matches!(decide(&same, &file, 12, None), Decision::Suggest(_)));
        match decide_with_tags(&same, &file, 12, &[], Some(&tags)) {
            Decision::Identified {
                recording,
                release,
                confidence,
                ..
            } => {
                assert_eq!(
                    recording.to_string(),
                    "00000000-0000-4000-8000-000000000002",
                    "the one on the file's album"
                );
                assert_eq!(release, None, "no release fits a folder of 12");
                assert!(matches!(confidence, Confidence::Fingerprint { .. }));
            }
            other => panic!("{other:?}"),
        }
        // A live-only recording among them is set aside; the studio ones still decide
        let with_live = answer(vec![
            grouped(4, "Over My Shoulder", "Album", &["Live"], "Live", 15, 7),
            grouped(
                2,
                "Over My Shoulder",
                "Album",
                &[],
                "Beggar on a Beach of Gold",
                10,
                2,
            ),
            grouped(5, "Over My Shoulder", "Album", &["Compilation"], "Hits", 18, 3),
        ]);
        assert!(
            matches!(decide_with_tags(&with_live, &file, 12, &[], Some(&tags)), Decision::Identified { recording, .. } if recording.to_string().ends_with("000000000002"))
        );
        // Another song among them: a question
        let other = answer(vec![
            grouped(
                2,
                "Over My Shoulder",
                "Album",
                &[],
                "Beggar on a Beach of Gold",
                10,
                2,
            ),
            grouped(6, "Silent Running", "Album", &[], "Mike + The Mechanics", 9, 1),
        ]);
        assert!(matches!(
            decide_with_tags(&other, &file, 12, &[], Some(&tags)),
            Decision::Suggest(_)
        ));
        // Only live recordings for a studio file: a question
        let live_only = answer(vec![
            grouped(4, "Over My Shoulder", "Album", &["Live"], "Live", 15, 7),
            grouped(7, "Over My Shoulder", "Album", &["Live"], "Live Too", 15, 7),
        ]);
        assert!(matches!(
            decide_with_tags(&live_only, &file, 12, &[], Some(&tags)),
            Decision::Suggest(_)
        ));
    }

    /// PR #250 review: one audio under several recordings is identified only on full evidence:
    /// each recording's length (present, within 3 s) and artist (present, the file's), and no
    /// qualifier the file doesn't have.
    #[test]
    fn same_audio_needs_length_artist_and_the_same_version() {
        let tags = crate::same_song::FileTags {
            title: "Over My Shoulder".into(),
            artist: "Mike + The Mechanics".into(),
            album: Some("Beggar on a Beach of Gold".into()),
            duration_ms: Some(232_000),
        };
        let file = FileFacts {
            track_no: Some(2),
            disc_no: Some(1),
            duration_secs: Some(232.0),
        };
        let answer = |recs: Vec<AcRecording>| {
            vec![AcoustIdResult {
                id: "a".into(),
                score: 0.96,
                recordings: recs,
            }]
        };
        let base = || {
            vec![
                grouped(
                    2,
                    "Over My Shoulder",
                    "Album",
                    &[],
                    "Beggar on a Beach of Gold",
                    10,
                    2,
                ),
                grouped(3, "Over My Shoulder", "Album", &["Compilation"], "Hits", 18, 3),
            ]
        };
        assert!(matches!(
            decide_with_tags(&answer(base()), &file, 12, &[], Some(&tags)),
            Decision::Identified { .. }
        ));
        let mut no_len = base();
        no_len[1].duration = None;
        assert!(
            matches!(
                decide_with_tags(&answer(no_len), &file, 12, &[], Some(&tags)),
                Decision::Suggest(_)
            ),
            "a length to compare"
        );
        let mut no_artist = base();
        no_artist[1].artists.clear();
        assert!(
            matches!(
                decide_with_tags(&answer(no_artist), &file, 12, &[], Some(&tags)),
                Decision::Suggest(_)
            ),
            "an artist to compare"
        );
        let mut other = base();
        other[1].title = Some("Over My Shoulder (Alternate Take)".into());
        assert!(
            matches!(
                decide_with_tags(&answer(other), &file, 12, &[], Some(&tags)),
                Decision::Suggest(_)
            ),
            "another take"
        );
        // "Live Forever" says nothing about a live recording
        let mut t2 = tags.clone();
        t2.album = Some("Live Forever".into());
        let live = answer(vec![
            grouped(
                4,
                "Over My Shoulder",
                "Album",
                &["Live"],
                "Live at Wembley",
                15,
                7,
            ),
            grouped(5, "Over My Shoulder", "Album", &["Live"], "Live Too", 15, 7),
        ]);
        assert!(matches!(
            decide_with_tags(&live, &file, 12, &[], Some(&t2)),
            Decision::Suggest(_)
        ));
    }
}
