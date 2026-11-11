//! The SQLite catalogue.
//!
//! One `Library` wraps one connection behind a mutex, so it is `Send + Sync`
//! and every call is synchronous. The node runs calls on a blocking thread.
//! Full-text search uses FTS5 (compiled into rusqlite's bundled SQLite).

use crate::error::{LibraryError, Result};
use crate::import::ImportedTrack;
use pm_model::AudioFormat;
use pm_model::{Album, Artist, AudioFile, Confidence, Isrc, Mbid, MusicUri, SampleKind, Track, TrackId, Upc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("migrations/0001_init.sql")),
    (2, include_str!("migrations/0002_music_posts.sql")),
    (3, include_str!("migrations/0003_album_art.sql")),
    (4, include_str!("migrations/0004_settings.sql")),
    (5, include_str!("migrations/0005_plays.sql")),
    (6, include_str!("migrations/0006_favourites.sql")),
    (7, include_str!("migrations/0007_tags.sql")),
    (8, include_str!("migrations/0008_genres.sql")),
    (9, include_str!("migrations/0009_lyrics.sql")),
    (10, include_str!("migrations/0010_identify.sql")),
    (11, include_str!("migrations/0011_identify_release.sql")),
    (12, include_str!("migrations/0012_fts_rowid.sql")),
    (13, include_str!("migrations/0013_file_hashes.sql")),
    (14, include_str!("migrations/0014_loudness.sql")),
    (15, include_str!("migrations/0015_mb_matches.sql")),
    (16, include_str!("migrations/0016_bookmarks.sql")),
    (17, include_str!("migrations/0017_library_edits.sql")),
    (18, include_str!("migrations/0018_identify_review.sql")),
    (19, include_str!("migrations/0019_identify_recheck.sql")),
];

pub struct Library {
    conn: Mutex<Connection>,
}

/// How far ahead of the node a device's clock may run for [`Library::record_play_at`] to keep a
/// play at the device's time (seconds).
pub const PLAY_CLOCK_SLACK: i64 = 3600;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LibraryStats {
    pub tracks: u64,
    pub albums: u64,
    pub artists: u64,
    pub files: u64,
    pub missing_files: u64,
    pub total_duration_ms: u64,
}

/// The scanner's view of a stored file.
/// Counts of what a backup holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BackupCounts {
    pub tracks: u64,
    /// Tracks with a canonical identity (tagged, fingerprinted or confirmed).
    pub identified: u64,
    pub albums: u64,
    pub playlists: u64,
    /// Bookmarks. The manifest keeps the older `favourites` name: it is a file format.
    #[serde(rename = "favourites")]
    pub bookmarks: u64,
    pub tags: u64,
    pub plays: u64,
    pub folders: u64,
}

/// The id of "Unknown artist", the songs without any artist ([`Library::unknown_artist`]).
pub const UNKNOWN_ARTIST_ID: i64 = 0;

/// An artist as the browse views list them.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ArtistSummary {
    pub id: i64,
    pub name: String,
    pub mbid: Option<Mbid>,
    pub albums: u64,
    pub tracks: u64,
}

/// An identification suggestion waiting for the owner.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SuggestionRow {
    pub id: i64,
    pub track_id: i64,
    pub recording_mbid: String,
    pub release_mbid: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub release_title: Option<String>,
    pub score: f32,
    /// `fingerprint` or `search`.
    pub source: String,
}

/// One song of a batch confirmation (album review): the suggestion the owner picked, and what
/// they saw, so a suggestion that changed meanwhile is not confirmed by mistake.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConfirmChoice {
    pub track_id: i64,
    pub suggestion_id: i64,
    pub recording_mbid: String,
}

/// A choice that no longer holds: `gone` (the suggestion or song is no more), `changed` (it is
/// for another song or recording now), `identified` (the song got an identity meanwhile) or
/// `duplicate` (the same song twice in one batch).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StaleChoice {
    pub track_id: i64,
    pub suggestion_id: i64,
    pub reason: String,
}

/// How a batch confirmation went: all stored (the songs, in order), or none, with every choice
/// that went stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmReport {
    Confirmed(Vec<i64>),
    Stale(Vec<StaleChoice>),
}

/// What a run found for a song, to store in one step ([`Library::settle`]).
#[derive(Debug, Clone, PartialEq)]
pub enum SettleAnswer {
    Identified {
        recording: MusicUri,
        confidence: Confidence,
        acoustid: Option<String>,
        detail: String,
        release: Option<Mbid>,
        input_key: Option<String>,
    },
    Suggest {
        candidates: Vec<crate::identify::Candidate>,
        input_key: Option<String>,
        /// A run's check: the song's open options become exactly these. Otherwise ("Which song
        /// is this?") they are added to what is open, and the song's rules version stays.
        replace: bool,
    },
    /// `unknown` or `error`.
    State {
        state: String,
        detail: Option<String>,
        input_key: Option<String>,
    },
}

/// What [`Library::settle`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    /// Stored as found.
    Stored,
    /// The song has an identity already (the owner's, its tags, another run's): left as it is.
    Kept,
    /// The recording found is one the owner rejected for the song: not stored (the song is
    /// "unknown").
    Rejected,
    /// Every suggestion was one the owner rejected: the song is "unknown".
    Empty,
    /// The song changed since the run read it (a rescan replaced its file or changed its tags):
    /// the answer is for what it was, so nothing is stored; the next run checks it again.
    Changed,
    /// Checking a waiting song again failed: nothing is written, it keeps its options and the
    /// next run tries again.
    Failed,
}

/// Whether a suggestion still is what the caller saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionCheck {
    Holds,
    Gone,
    Changed,
}

/// What [`Library::rematch`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rematch {
    /// Stored; `old` is the recording the song had (now rejected for it unless kept).
    Done { old: Option<String> },
    /// The song's recording is no longer the one the caller saw: nothing changed.
    Changed,
    /// The song's file's tags name a recording, and they won't be rewritten: nothing changed.
    FromTags,
    /// No such song.
    Gone,
}

/// How much of the library has a canonical identity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IdentityCounts {
    pub total: u64,
    pub identified: u64,
    /// Tracks with suggestions to confirm (not those the owner left as they are).
    pub suggested: u64,
    /// Tracks identification found nothing for (or failed on).
    pub unknown: u64,
    /// Tracks identification has not looked at yet.
    pub unchecked: u64,
    /// Tracks the owner left as they are without an identity ("Leave as is", "Skip all unsure"):
    /// not in `suggested`, not checked again.
    #[serde(default)]
    pub skipped: u64,
    /// Tracks waiting for review that older rules checked (included in `suggested`): the next run
    /// checks them again.
    #[serde(default)]
    pub outdated: u64,
    /// How the identified ones were identified.
    #[serde(default)]
    pub by: IdentifiedBy,
}

/// Identified tracks by how: IDs in the tags, the folder as an album, the fingerprint, or the
/// owner's choice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IdentifiedBy {
    pub tags: u64,
    pub album: u64,
    pub fingerprint: u64,
    pub you: u64,
    /// By a search of their exact title, artist and length.
    #[serde(default)]
    pub title_artist: u64,
    /// Settled without review from the evidence a run stored (`Confidence::Auto`); their IDs
    /// are not written into the files until the owner confirms them.
    #[serde(default)]
    pub auto: u64,
}

/// A bookmark as stored locally (a social v1 bookmark of a canonical music URL).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BookmarkRow {
    pub id: String,
    pub uri: String,
    /// Microseconds.
    pub created_at: i64,
    pub synced_at: Option<i64>,
    pub deleted: bool,
}

/// Roon-style "Focus" on albums by what the files are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Focus {
    /// PCM above 16/48 in a lossless container.
    HiRes,
    Dsd,
    /// 16-bit PCM at 44.1 or 48 kHz in a lossless container.
    Cd,
    Lossless,
    Lossy,
}

const LOSSY_CONTAINERS: &str = "('mp3', 'aac', 'vorbis', 'opus')";

impl Focus {
    /// An SQL predicate over a `files` row aliased `f`.
    fn predicate(self) -> String {
        match self {
            Focus::HiRes => format!(
                "f.sample_kind = 'pcm' AND (f.rate_hz > 48000 OR f.bits > 16) AND f.container NOT IN {LOSSY_CONTAINERS}"
            ),
            Focus::Dsd => "f.sample_kind = 'dsd'".to_string(),
            Focus::Cd => format!(
                "f.sample_kind = 'pcm' AND f.rate_hz <= 48000 AND f.bits <= 16 AND f.container NOT IN {LOSSY_CONTAINERS}"
            ),
            Focus::Lossless => format!("f.container NOT IN {LOSSY_CONTAINERS}"),
            Focus::Lossy => format!("f.container IN {LOSSY_CONTAINERS}"),
        }
    }
}

/// The best file format on an album, for a badge and for Focus.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AlbumFormat {
    pub album_id: i64,
    /// `24/96 FLAC`, `DSD64`, `16/44.1 ALAC`, `MP3`.
    pub label: String,
    pub lossless: bool,
    pub hi_res: bool,
    pub dsd: bool,
}

/// A genre as the browse views list it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GenreSummary {
    pub id: i64,
    pub name: String,
    pub tracks: u64,
    pub albums: u64,
}

/// One of the user's own tags (a v1 PubkyAppTag) as stored locally.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TagRow {
    pub id: String,
    pub uri: String,
    pub label: String,
    /// Microseconds.
    pub created_at: i64,
    pub synced_at: Option<i64>,
    pub deleted: bool,
}

/// One entry of the play history.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlayEntry {
    pub id: i64,
    pub track: Track,
    pub zone: Option<String>,
    pub played_at: i64,
}

/// A track with how often it was played.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlayedTrack {
    pub track: Track,
    pub plays: u64,
    pub last_played_at: i64,
}

/// Where an album's cover lives on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumArt {
    pub album_id: i64,
    pub file: PathBuf,
    pub mime: String,
    /// `embedded` (extracted from a tag) or `folder` (an image next to the files).
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub id: i64,
    pub track_id: TrackId,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub mtime_unix: i64,
    pub missing: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanCounts {
    pub files_seen: u64,
    pub files_added: u64,
    pub files_updated: u64,
    pub files_removed: u64,
}

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// How long a write waits for another connection's transaction before "database is locked".
pub const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// SQLite's busy handler: try again every millisecond, up to [`BUSY_TIMEOUT`]. The built-in
/// timeout sleeps up to 100 ms between tries, so a writer that commits and starts again within a
/// few milliseconds (a scan's batches) kept everyone else out until they gave up.
fn wait_for_writer(tries: i32) -> bool {
    if (tries as u128) >= BUSY_TIMEOUT.as_millis() {
        return false;
    }
    std::thread::sleep(std::time::Duration::from_millis(1));
    true
}

impl Library {
    /// Opens (or makes) the library at `path`. A new one is readable by the node's user alone
    /// (it holds sign-ins); SQLite gives its journal files the same permissions.
    pub fn open(path: &Path) -> Result<Self> {
        // A new one is made empty and 0600 first, so it is never readable by others, not even
        // for the moment between SQLite creating it and the first write
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
            {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => {
                    return Err(LibraryError::Io {
                        path: path.to_path_buf(),
                        source: e,
                    })
                }
            }
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        // A scan writes from its own connection in batches of a few hundred files (and two scans
        // of large shares may run with the API writing too); a writer waits for the others instead
        // of failing with "database is locked".
        conn.busy_handler(Some(wait_for_writer))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;",
        )?;
        let lib = Library {
            conn: Mutex::new(conn),
        };
        lib.migrate()?;
        lib.tidy_file_name_titles()?;
        lib.split_artist_credits()?;
        Ok(lib)
    }

    /// Artists named by a whole credit ("Sefa, Dr. Peacock", scanned before
    /// [`crate::credits`]) become the artists it names; their tracks keep their order of artists.
    /// Artists with a MusicBrainz id are left alone. Idempotent. Returns how many were split.
    fn split_artist_credits(&self) -> Result<usize> {
        let conn = self.lock();
        let rows: Vec<(i64, String)> = conn
            .prepare("SELECT id, name FROM artists WHERE mbid IS NULL")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut split = 0;
        for (artist_id, name) in rows {
            let parts = crate::credits::split_credit(&name);
            if parts.len() < 2 {
                continue;
            }
            split += 1;
            let tracks: Vec<i64> = conn
                .prepare("SELECT DISTINCT track_id FROM track_artists WHERE artist_id = ?1")?
                .query_map(params![artist_id], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?;
            for track in tracks {
                // The track's artists in order, the credit replaced by its parts
                let current: Vec<i64> = conn
                    .prepare("SELECT artist_id FROM track_artists WHERE track_id = ?1 ORDER BY position")?
                    .query_map(params![track], |r| r.get(0))?
                    .collect::<std::result::Result<_, _>>()?;
                let mut next: Vec<i64> = Vec::new();
                for a in current {
                    if a == artist_id {
                        for p in &parts {
                            let id = upsert_artist(
                                &conn,
                                &Artist {
                                    id: None,
                                    name: p.clone(),
                                    mbid: None,
                                },
                            )?;
                            if !next.contains(&id) {
                                next.push(id);
                            }
                        }
                    } else if !next.contains(&a) {
                        next.push(a);
                    }
                }
                conn.execute("DELETE FROM track_artists WHERE track_id = ?1", params![track])?;
                for (pos, a) in next.iter().enumerate() {
                    conn.execute(
                        "INSERT INTO track_artists(track_id, artist_id, position) VALUES (?1, ?2, ?3)",
                        params![track, a, pos as i64],
                    )?;
                }
                let names: Vec<String> = next
                    .iter()
                    .map(|a| {
                        conn.query_row("SELECT name FROM artists WHERE id = ?1", params![a], |r| r.get(0))
                    })
                    .collect::<std::result::Result<_, _>>()?;
                conn.execute(
                    "UPDATE tracks_fts SET artist = ?2 WHERE rowid = ?1",
                    params![track, names.join(" ")],
                )?;
            }
            conn.execute(
                "DELETE FROM artists WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM track_artists WHERE artist_id = ?1)",
                params![artist_id],
            )?;
        }
        if split > 0 {
            tracing::info!(split, "artist credits split into their artists");
        }
        Ok(split)
    }

    /// Tracks still titled by their raw file name (scanned before
    /// [`crate::scan::title_from_file_name`]), or by the tidy form that kept a homeserver hash
    /// prefix (`F80da35cdf86-14 wasted bits`), get the tidy title. Idempotent.
    fn tidy_file_name_titles(&self) -> Result<usize> {
        let conn = self.lock();
        let rows: Vec<(i64, String, String)> = {
            let mut st =
                conn.prepare("SELECT t.id, t.title, f.path FROM tracks t JOIN files f ON f.track_id = t.id")?;
            let it = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            it.collect::<std::result::Result<_, _>>()?
        };
        let mut n = 0;
        for (id, title, path) in rows {
            let stem = Path::new(&path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let hashed = crate::scan::strip_hash_prefix(&stem) != stem;
            if title != stem && !(hashed && title == crate::scan::tidy_stem(&stem)) {
                continue;
            }
            let tidy = crate::scan::title_from_file_name(&stem);
            if tidy.is_empty() || tidy == title {
                continue;
            }
            conn.execute("UPDATE tracks SET title = ?2 WHERE id = ?1", params![id, tidy])?;
            conn.execute(
                "UPDATE tracks_fts SET title = ?2 WHERE rowid = ?1",
                params![id, tidy],
            )?;
            n += 1;
        }
        Ok(n)
    }

    fn migrate(&self) -> Result<()> {
        let mut conn = self.lock();
        let has_table: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='schema_version'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n > 0)?;
        let current: i64 = if has_table {
            conn.query_row("SELECT COALESCE(MAX(version), 0) FROM schema_version", [], |r| {
                r.get(0)
            })?
        } else {
            0
        };
        // An older node on a library a newer one has migrated (a rollback) would run on tables
        // it doesn't know: stop instead, and the owner updates or restores the backup from before
        let known = MIGRATIONS.last().map_or(0, |(v, _)| *v);
        if current > known {
            return Err(LibraryError::NewerSchema {
                found: current,
                known,
            });
        }
        for (version, sql) in MIGRATIONS {
            if *version > current {
                let tx = conn.transaction()?;
                tx.execute_batch(sql)?;
                tx.execute(
                    "INSERT INTO schema_version(version, applied_at) VALUES (?1, ?2)",
                    params![version, now_unix()],
                )?;
                tx.commit()?;
                tracing::info!(version, "applied library schema migration");
            }
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .lock()
            .query_row("SELECT COALESCE(MAX(version), 0) FROM schema_version", [], |r| {
                r.get(0)
            })?)
    }

    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Start grouping writes into one commit (a scan writes thousands of rows; one commit per
    /// file made a first scan of 50,000 tracks wait on the disk for most of its time). Writes
    /// made meanwhile, by the scan or anything else, land in the batch; [`Self::commit_batch`]
    /// commits them. Keep a batch short: commit every few hundred files.
    ///
    /// The batch takes the write lock when it starts (`BEGIN IMMEDIATE`), waiting for other
    /// writers. A plain `BEGIN` reads first and asks for the lock at its first write; in WAL mode
    /// that write fails at once with "database is locked" when another connection committed
    /// since the read, without waiting, and the scan skipped the file.
    pub fn begin_batch(&self) -> Result<()> {
        let conn = self.lock();
        if conn.is_autocommit() {
            conn.execute_batch("BEGIN IMMEDIATE")?;
        }
        Ok(())
    }

    /// Commit what [`Self::begin_batch`] grouped (nothing to do without a batch).
    pub fn commit_batch(&self) -> Result<()> {
        let conn = self.lock();
        if !conn.is_autocommit() {
            conn.execute_batch("COMMIT")?;
        }
        Ok(())
    }

    // ----- tracks -------------------------------------------------------

    /// Insert or update a track, keyed by its file path. Returns the id.
    pub fn upsert_track(&self, track: &Track) -> Result<TrackId> {
        let mut conn = self.lock();
        let tx = conn.savepoint()?;
        let album_id = match &track.album {
            Some(a) => Some(upsert_album(&tx, a)?),
            None => None,
        };
        let path = track.file.path.to_string_lossy().to_string();
        let existing: Option<i64> = tx
            .query_row("SELECT track_id FROM files WHERE path = ?1", params![path], |r| {
                r.get(0)
            })
            .optional()?;
        let (uri, src, score) = match &track.canonical {
            Some((u, c)) => (
                Some(u.to_url()),
                Some(confidence_source(c)),
                Some(c.score() as f64),
            ),
            None => (None, None, None),
        };
        // What identification went by (the file, its tags) before this write
        let before: Option<String> = match existing {
            Some(id) => identify_input_of(&tx, id, &path)?,
            None => None,
        };
        let track_id = match existing {
            Some(id) => {
                tx.execute(
                    // A rescan of a file without IDs in its tags keeps what identification
                    // found (fingerprint, release match, folder match, an automatic pick, the
                    // owner's confirmation).
                    "UPDATE tracks SET title=?2, album_id=?3, track_no=?4, disc_no=?5, duration_ms=?6, recording_mbid=?7, isrc=?8,
                     acoustid=COALESCE(?9, acoustid),
                     canonical_uri=CASE WHEN ?10 IS NULL AND canonical_confidence_source IN ('fingerprint','fingerprint+release','album_fit','tag_match','auto','confirmed_by_user') THEN canonical_uri ELSE ?10 END,
                     canonical_confidence_score=CASE WHEN ?10 IS NULL AND canonical_confidence_source IN ('fingerprint','fingerprint+release','album_fit','tag_match','auto','confirmed_by_user') THEN canonical_confidence_score ELSE ?12 END,
                     canonical_confidence_source=CASE WHEN ?10 IS NULL AND canonical_confidence_source IN ('fingerprint','fingerprint+release','album_fit','tag_match','auto','confirmed_by_user') THEN canonical_confidence_source ELSE ?11 END,
                     replaygain_track_gain_db=?13, replaygain_album_gain_db=?14
                     WHERE id=?1",
                    params![
                        id, track.title, album_id, track.track_no, track.disc_no, track.duration_ms.map(|d| d as i64),
                        track.recording_mbid.map(|m| m.to_string()), track.isrc.as_ref().map(|i| i.to_string()), track.acoustid,
                        uri, src, score, track.replaygain_track_gain_db, track.replaygain_album_gain_db
                    ],
                )?;
                tx.execute("DELETE FROM track_artists WHERE track_id = ?1", params![id])?;
                tx.execute("DELETE FROM track_genres WHERE track_id = ?1", params![id])?;
                id
            }
            None => {
                tx.execute(
                    "INSERT INTO tracks(title, album_id, track_no, disc_no, duration_ms, recording_mbid, isrc, acoustid, canonical_uri,
                     canonical_confidence_source, canonical_confidence_score, replaygain_track_gain_db, replaygain_album_gain_db)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                    params![
                        track.title, album_id, track.track_no, track.disc_no, track.duration_ms.map(|d| d as i64),
                        track.recording_mbid.map(|m| m.to_string()), track.isrc.as_ref().map(|i| i.to_string()), track.acoustid,
                        uri, src, score, track.replaygain_track_gain_db, track.replaygain_album_gain_db
                    ],
                )?;
                tx.last_insert_rowid()
            }
        };
        for (pos, artist) in track.artists.iter().enumerate() {
            let artist_id = upsert_artist(&tx, artist)?;
            tx.execute(
                "INSERT INTO track_artists(track_id, artist_id, position) VALUES (?1, ?2, ?3)",
                params![track_id, artist_id, pos as i64],
            )?;
        }
        for g in &track.genres {
            tx.execute("INSERT OR IGNORE INTO genres(name) VALUES (?1)", params![g])?;
            let gid: i64 = tx.query_row(
                "SELECT id FROM genres WHERE name = ?1 COLLATE NOCASE",
                params![g],
                |r| r.get(0),
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO track_genres(track_id, genre_id) VALUES (?1, ?2)",
                params![track_id, gid],
            )?;
        }
        let f = &track.file;
        tx.execute(
            "INSERT INTO files(track_id, path, size_bytes, mtime_unix, container, sample_kind, rate_hz, bits, channels, duration_ms, missing)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,0)
             ON CONFLICT(path) DO UPDATE SET track_id=excluded.track_id, size_bytes=excluded.size_bytes, mtime_unix=excluded.mtime_unix,
             container=excluded.container, sample_kind=excluded.sample_kind, rate_hz=excluded.rate_hz, bits=excluded.bits,
             channels=excluded.channels, duration_ms=excluded.duration_ms, missing=0",
            params![
                track_id, path, f.size_bytes as i64, f.mtime_unix, f.container, sample_kind_str(f.format.kind),
                f.format.rate_hz, f.format.bits, f.format.channels, f.duration_ms.map(|d| d as i64)
            ],
        )?;
        let artist_text = track
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let mut album_text = track.album.as_ref().map(|a| a.title.clone()).unwrap_or_default();
        for g in &track.genres {
            album_text.push(' ');
            album_text.push_str(g);
        }
        // The file replaced or its tags changed: suggestions found for what it was are dropped (a
        // review page loaded before can't confirm them), and a song without an identity is
        // checked again by the next run. An identity stays, as it always did.
        if before.is_some() && before != identify_input_of(&tx, track_id, &path)? {
            let open: bool = tx.query_row(
                "SELECT canonical_uri IS NULL FROM tracks WHERE id = ?1",
                params![track_id],
                |r| r.get(0),
            )?;
            if open {
                tx.execute(
                    "DELETE FROM identify_suggestions WHERE track_id = ?1",
                    params![track_id],
                )?;
                tx.execute(
                    "DELETE FROM identify_state WHERE track_id = ?1",
                    params![track_id],
                )?;
            }
        }
        tx.execute("DELETE FROM tracks_fts WHERE rowid = ?1", params![track_id])?;
        tx.execute(
            "INSERT INTO tracks_fts(rowid, track_id, title, artist, album) VALUES (?1, ?1, ?2, ?3, ?4)",
            params![track_id, track.title, artist_text, album_text],
        )?;
        tx.commit()?;
        Ok(TrackId(track_id))
    }

    /// A track by id, also when its file is gone (favourites, playlists, plays and devices keep
    /// it; lists leave it out). Its present file when it has one.
    pub fn get_track(&self, id: TrackId) -> Result<Option<Track>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(&format!(
            "{TRACK_SELECT} WHERE t.id = ?1 ORDER BY fi.missing LIMIT 1"
        ))?;
        let row = stmt.query_row(params![id.0], row_to_track_raw).optional()?;
        match row {
            Some(raw) => Ok(Some(finish_track(&conn, raw)?)),
            None => Ok(None),
        }
    }

    /// Tracks ordered by id, keyset-paged: pass the last id seen.
    pub fn list_tracks(&self, after: Option<TrackId>, limit: u32) -> Result<Vec<Track>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(&format!(
            "{} WHERE t.id > ?1 ORDER BY t.id LIMIT ?2",
            present_select()
        ))?;
        let raws: Vec<RawTrack> = stmt
            .query_map(
                params![after.map(|a| a.0).unwrap_or(0), limit as i64],
                row_to_track_raw,
            )?
            .collect::<std::result::Result<_, _>>()?;
        raws.into_iter().map(|raw| finish_track(&conn, raw)).collect()
    }

    /// Tracks by id, `limit` from `offset` (clients that page by offset: Subsonic `search3`).
    pub fn tracks_page(&self, offset: u32, limit: u32) -> Result<Vec<Track>> {
        let conn = self.lock();
        let mut stmt =
            conn.prepare_cached(&format!("{} ORDER BY t.id LIMIT ?1 OFFSET ?2", present_select()))?;
        let raws: Vec<RawTrack> = stmt
            .query_map(params![limit as i64, offset as i64], row_to_track_raw)?
            .collect::<std::result::Result<_, _>>()?;
        raws.into_iter().map(|raw| finish_track(&conn, raw)).collect()
    }

    /// Full-text search over title, artist and album.
    pub fn search_tracks(&self, query: &str, limit: u32) -> Result<Vec<Track>> {
        let q = fts_query(query);
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(&format!(
            "{} JOIN tracks_fts f ON f.rowid = t.id WHERE tracks_fts MATCH ?1 ORDER BY bm25(tracks_fts) LIMIT ?2",
            present_select()
        ))?;
        let raws: Vec<RawTrack> = stmt
            .query_map(params![q, limit as i64], row_to_track_raw)?
            .collect::<std::result::Result<_, _>>()?;
        raws.into_iter().map(|raw| finish_track(&conn, raw)).collect()
    }

    pub fn tracks_by_canonical(&self, uri: &MusicUri) -> Result<Vec<Track>> {
        self.tracks_where("t.canonical_uri = ?1", uri.to_url())
    }

    /// Tracks whose tags carry this ISRC (whatever their canonical identity).
    pub fn tracks_by_isrc(&self, isrc: &str) -> Result<Vec<Track>> {
        self.tracks_where("t.isrc = ?1", isrc.to_ascii_uppercase())
    }

    pub fn tracks_by_mbid(&self, mbid: Mbid) -> Result<Vec<Track>> {
        self.tracks_where("t.recording_mbid = ?1", mbid.to_string())
    }

    fn tracks_where(&self, clause: &str, arg: String) -> Result<Vec<Track>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(&format!(
            "{TRACK_SELECT} WHERE {clause} ORDER BY fi.missing, t.id"
        ))?;
        let raws: Vec<RawTrack> = stmt
            .query_map(params![arg], row_to_track_raw)?
            .collect::<std::result::Result<_, _>>()?;
        raws.into_iter().map(|raw| finish_track(&conn, raw)).collect()
    }

    /// The owner's edit of a file's tags, if any.
    pub fn track_override(&self, path: &Path) -> Result<Option<TrackOverride>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT title, artist, album, album_artist FROM track_overrides WHERE path = ?1",
                params![path.to_string_lossy()],
                |r| {
                    Ok(TrackOverride {
                        title: r.get(0)?,
                        artist: r.get(1)?,
                        album: r.get(2)?,
                        album_artist: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// Edit a song's tags on the node (the file is not touched): the edit is kept for the file,
    /// applied now and whenever the file is read again. Fields left `None` keep the file's value
    /// (the edit replaces the song's earlier one). An edit with nothing in it undoes the song's edit.
    pub fn edit_track(&self, id: TrackId, o: &TrackOverride) -> Result<Option<Track>> {
        let Some(mut t) = self.get_track(id)? else {
            return Ok(None);
        };
        if o.is_empty() {
            return self.undo_edit(id);
        }
        let path = t.file.path.to_string_lossy().to_string();
        {
            let conn = self.lock();
            // What the file says, kept from the first edit on (a rescan refreshes it)
            let orig: Option<(String, String, Option<String>, Option<String>)> = conn
                .query_row(
                    "SELECT orig_title, orig_artists, orig_album, orig_album_artist FROM track_overrides WHERE path = ?1",
                    params![path],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?;
            let had_edit = orig.is_some();
            let (ot, oa, oal, oaa) = orig.unwrap_or_else(|| originals_of(&t));
            conn.execute(
                "INSERT INTO track_overrides(path, title, artist, album, album_artist, orig_title, orig_artists, orig_album, orig_album_artist, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
                 ON CONFLICT(path) DO UPDATE SET title=excluded.title, artist=excluded.artist, album=excluded.album,
                   album_artist=excluded.album_artist, updated_at=excluded.updated_at",
                params![path, o.title, o.artist, o.album, o.album_artist, ot, oa, oal, oaa, now_unix()],
            )?;
            // The new edit goes on what the file says, not on the old edit: a field it leaves
            // out is the file's value again
            if had_edit {
                restore_originals(&mut t, &ot, &oa, oal, oaa);
            }
        }
        let old_album = t.album.as_ref().and_then(|a| a.id);
        o.apply(&mut t);
        self.upsert_track(&t)?;
        self.carry_cover(id, old_album)?;
        self.prune_empty_albums()?;
        self.get_track(id)
    }

    /// A song moved to another album by an edit: the new album takes the old one's cover when it
    /// has none (covers embedded in files are not read again while the file is unchanged).
    fn carry_cover(&self, id: TrackId, old_album: Option<i64>) -> Result<()> {
        let Some(old) = old_album else { return Ok(()) };
        let new = self.get_track(id)?.and_then(|t| t.album.and_then(|a| a.id));
        if let Some(new) = new.filter(|n| *n != old) {
            self.lock().execute(
                "INSERT OR IGNORE INTO album_art(album_id, file, mime, source, set_at)
                 SELECT ?2, file, mime, source, set_at FROM album_art WHERE album_id = ?1",
                params![old, new],
            )?;
        }
        Ok(())
    }

    /// Undo a song's edit: back to what its file says.
    pub fn undo_edit(&self, id: TrackId) -> Result<Option<Track>> {
        let Some(mut t) = self.get_track(id)? else {
            return Ok(None);
        };
        let path = t.file.path.to_string_lossy().to_string();
        let orig: Option<(String, String, Option<String>, Option<String>)> = {
            let conn = self.lock();
            let o = conn
                .query_row(
                    "SELECT orig_title, orig_artists, orig_album, orig_album_artist FROM track_overrides WHERE path = ?1",
                    params![path],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?;
            conn.execute("DELETE FROM track_overrides WHERE path = ?1", params![path])?;
            o
        };
        if let Some((title, artists, album, album_artist)) = orig {
            let old_album = t.album.as_ref().and_then(|a| a.id);
            restore_originals(&mut t, &title, &artists, album, album_artist);
            self.upsert_track(&t)?;
            self.carry_cover(id, old_album)?;
            self.prune_empty_albums()?;
        }
        self.get_track(id)
    }

    /// Keep what a file says under its edit (a rescan read the file again), and give back the
    /// edit to apply on top.
    pub fn refresh_override(&self, track: &Track) -> Result<Option<TrackOverride>> {
        let path = track.file.path.to_string_lossy().to_string();
        let Some(o) = self.track_override(&track.file.path)? else {
            return Ok(None);
        };
        let (ot, oa, oal, oaa) = originals_of(track);
        self.lock().execute(
            "UPDATE track_overrides SET orig_title=?2, orig_artists=?3, orig_album=?4, orig_album_artist=?5 WHERE path=?1",
            params![path, ot, oa, oal, oaa],
        )?;
        Ok(Some(o))
    }

    /// What a file says under its edit, if it has one: title, artist, album, album artist.
    pub fn file_tags(&self, path: &Path) -> Result<Option<TrackOverride>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT orig_title, orig_artists, orig_album, orig_album_artist FROM track_overrides WHERE path = ?1",
                params![path.to_string_lossy()],
                |r| {
                    let artists: String = r.get(1)?;
                    let names: Vec<String> = serde_json::from_str(&artists).unwrap_or_default();
                    Ok(TrackOverride {
                        title: Some(r.get(0)?),
                        artist: Some(names.join(", ")).filter(|a| !a.is_empty()),
                        album: r.get(2)?,
                        album_artist: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// Rename an album on the node (its cover and id stay); every song on it keeps the new name
    /// across rescans. A name another album already has is refused.
    pub fn edit_album(
        &self,
        album_id: i64,
        title: Option<&str>,
        album_artist: Option<Option<&str>>,
    ) -> Result<AlbumEdit> {
        let ids: Vec<i64> = {
            let conn = self.lock();
            let mut stmt = conn.prepare("SELECT id FROM tracks WHERE album_id = ?1")?;
            let ids = stmt
                .query_map(params![album_id], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?;
            ids
        };
        // What each song's file says, read before the rename (its own album name, not the new one)
        let mut before = Vec::with_capacity(ids.len());
        for id in &ids {
            if let Some(t) = self.get_track(TrackId(*id))? {
                let has = self.track_override(&t.file.path)?.is_some();
                before.push((t.file.path.clone(), has, originals_of(&t)));
            }
        }
        {
            let conn = self.lock();
            let taken: Option<i64> = conn
                .query_row(
                    "SELECT b.id FROM albums a JOIN albums b ON b.id != a.id
                       AND b.title = COALESCE(?2, a.title)
                       AND COALESCE(b.album_artist, '') = COALESCE(CASE WHEN ?3 THEN ?4 ELSE a.album_artist END, '')
                       AND COALESCE(b.release_mbid, '') = COALESCE(a.release_mbid, '')
                     WHERE a.id = ?1",
                    params![album_id, title, album_artist.is_some(), album_artist.flatten()],
                    |r| r.get(0),
                )
                .optional()?;
            if taken.is_some() {
                return Ok(AlbumEdit::NameTaken);
            }
            let n = conn.execute(
                "UPDATE albums SET title = COALESCE(?2, title), album_artist = CASE WHEN ?3 THEN ?4 ELSE album_artist END WHERE id = ?1",
                params![album_id, title, album_artist.is_some(), album_artist.flatten()],
            )?;
            if n == 0 {
                return Ok(AlbumEdit::NotFound);
            }
        }
        for (path, has, (ot, oa, oal, oaa)) in before {
            let Some(t) = self
                .file_by_path(&path)?
                .and_then(|f| self.get_track(f.track_id).ok().flatten())
            else {
                continue;
            };
            let (album, album_artist) = match &t.album {
                Some(a) => (Some(a.title.clone()), a.album_artist.clone()),
                None => (None, None),
            };
            let path = path.to_string_lossy().to_string();
            let conn = self.lock();
            if has {
                conn.execute(
                    "UPDATE track_overrides SET album=?2, album_artist=?3, updated_at=?4 WHERE path=?1",
                    params![path, album, album_artist, now_unix()],
                )?;
            } else {
                conn.execute(
                    "INSERT INTO track_overrides(path, title, artist, album, album_artist, orig_title, orig_artists, orig_album, orig_album_artist, updated_at)
                     VALUES (?1,NULL,NULL,?2,?3,?4,?5,?6,?7,?8)",
                    params![path, album, album_artist, ot, oa, oal, oaa, now_unix()],
                )?;
            }
        }
        // Search knows songs by their album too: write them again under the new name
        for id in &ids {
            if let Some(t) = self.get_track(TrackId(*id))? {
                self.upsert_track(&t)?;
            }
        }
        Ok(AlbumEdit::Done)
    }

    /// The file names and content-hash prefixes of the songs taken out of the library (My music
    /// says "taken out" for them, not "not in the library yet").
    pub fn hidden_names_and_hashes(&self) -> Result<std::collections::HashSet<String>> {
        let conn = self.lock();
        let mut out = std::collections::HashSet::new();
        let mut stmt = conn.prepare(
            "SELECT h.path, substr(fh.blake3, 1, 16) FROM hidden_files h LEFT JOIN file_hashes fh ON fh.path = h.path",
        )?;
        for row in stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })? {
            let (path, hash) = row?;
            if let Some(name) = Path::new(&path).file_name() {
                out.insert(name.to_string_lossy().to_string());
            }
            out.extend(hash);
        }
        Ok(out)
    }

    /// Library files by the first 16 hex of their content hash (how the homeserver names the
    /// files the node saved there): `prefix -> track id`.
    pub fn track_ids_by_hash_prefix(&self) -> Result<std::collections::HashMap<String, TrackId>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT substr(h.blake3, 1, 16), f.track_id FROM file_hashes h JOIN files f ON f.path = h.path AND f.missing = 0",
        )?;
        let mut out = std::collections::HashMap::new();
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
            let (prefix, id) = row?;
            out.insert(prefix, TrackId(id));
        }
        Ok(out)
    }

    /// Take a song out of the library: its file stays where it is, scans skip it from now on.
    /// Nothing is deleted: the song's rows stay (plays, matches, lyrics, edits), the file is marked
    /// missing and every track read leaves it out, so bringing it back returns the same song.
    pub fn hide_track(&self, id: TrackId) -> Result<bool> {
        let conn = self.lock();
        let path: Option<String> = conn
            .query_row("SELECT path FROM files WHERE track_id = ?1", params![id.0], |r| {
                r.get(0)
            })
            .optional()?;
        let Some(path) = path else { return Ok(false) };
        conn.execute(
            "INSERT INTO hidden_files(path, hidden_at) VALUES (?1, ?2) ON CONFLICT(path) DO NOTHING",
            params![path, now_unix()],
        )?;
        conn.execute("UPDATE files SET missing = 1 WHERE path = ?1", params![path])?;
        Ok(true)
    }

    /// Whether a file was taken out of the library.
    pub fn is_hidden(&self, path: &Path) -> Result<bool> {
        Ok(self
            .lock()
            .query_row(
                "SELECT 1 FROM hidden_files WHERE path = ?1",
                params![path.to_string_lossy()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// The files taken out of the library, newest first: `(path, hidden_at)`.
    pub fn hidden_files(&self) -> Result<Vec<(String, i64)>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT path, hidden_at FROM hidden_files ORDER BY hidden_at DESC")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// Put a file back: the next scan reads it again.
    /// Back in the library: the song shows again at once (its file counts as there; the next scan
    /// of its folder says otherwise if it is gone).
    pub fn unhide(&self, path: &str) -> Result<bool> {
        let conn = self.lock();
        let n = conn.execute("DELETE FROM hidden_files WHERE path = ?1", params![path])?;
        if n > 0 {
            conn.execute("UPDATE files SET missing = 0 WHERE path = ?1", params![path])?;
        }
        Ok(n > 0)
    }

    /// Library files by their file name (the last part of the path), for joining the
    /// homeserver's private files to the songs they are: `name -> track id`.
    pub fn track_ids_by_file_name(&self) -> Result<std::collections::HashMap<String, TrackId>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT path, track_id FROM files WHERE missing = 0")?;
        let mut out = std::collections::HashMap::new();
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
            let (path, id) = row?;
            if let Some(name) = Path::new(&path).file_name() {
                out.insert(name.to_string_lossy().to_string(), TrackId(id));
            }
        }
        Ok(out)
    }

    pub fn remove_track(&self, id: TrackId) -> Result<bool> {
        let conn = self.lock();
        conn.execute("DELETE FROM tracks_fts WHERE rowid = ?1", params![id.0])?;
        Ok(conn.execute("DELETE FROM tracks WHERE id = ?1", params![id.0])? > 0)
    }

    // ----- albums -------------------------------------------------------

    pub fn list_albums(&self, after: Option<i64>, limit: u32) -> Result<Vec<Album>> {
        self.list_albums_focused(None, after, limit)
    }

    /// Albums with at least one present file matching `focus` (all albums
    /// when `None`).
    pub fn list_albums_focused(
        &self,
        focus: Option<Focus>,
        after: Option<i64>,
        limit: u32,
    ) -> Result<Vec<Album>> {
        let conn = self.lock();
        let filter = match focus {
            Some(f) => format!(
                " AND EXISTS (SELECT 1 FROM tracks t JOIN files f ON f.track_id = t.id WHERE t.album_id = albums.id AND f.missing = 0 AND {})",
                f.predicate()
            ),
            None => format!(" AND {ALBUM_PRESENT}"),
        };
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT id, title, album_artist, release_mbid, release_group_mbid, year, upc, total_discs FROM albums WHERE id > ?1{filter} ORDER BY id LIMIT ?2"
        ))?;
        let v = stmt
            .query_map(params![after.unwrap_or(0), limit as i64], row_to_album)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        v.into_iter().collect()
    }

    /// The best present file per album (highest rate, then bits), labelled.
    pub fn album_formats(&self) -> Result<Vec<AlbumFormat>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT t.album_id, f.container, f.sample_kind, f.rate_hz, f.bits,
                    MAX(f.rate_hz * 100 + f.bits)
             FROM files f JOIN tracks t ON t.id = f.track_id
             WHERE t.album_id IS NOT NULL AND f.missing = 0
             GROUP BY t.album_id ORDER BY t.album_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (album_id, container, kind, rate, bits) = row?;
            out.push(describe_format(album_id, &container, &kind, rate, bits));
        }
        Ok(out)
    }

    /// An album's songs, one row per song: copies of the same song (a homeserver copy, another
    /// format) show once, as their best copy (see `songs_in_order`).
    pub fn album_tracks(&self, album_id: i64) -> Result<Vec<Track>> {
        Ok(songs_only(self.album_songs(album_id)?))
    }

    /// Every track of an album, files gone or not (what an offline device keeps for it).
    pub fn album_track_ids(&self, album_id: i64) -> Result<Vec<i64>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached("SELECT id FROM tracks WHERE album_id = ?1 ORDER BY id")?;
        let v = stmt
            .query_map(params![album_id], |r| r.get(0))?
            .collect::<std::result::Result<Vec<i64>, _>>()?;
        Ok(v)
    }

    /// [`Library::album_tracks`] with each song's other copies.
    pub fn album_songs(&self, album_id: i64) -> Result<Vec<Song>> {
        let conn = self.lock();
        let ids = candidates(
            &conn,
            &format!("SELECT t.id, t.album_id FROM tracks t WHERE t.album_id = ?1 AND {TRACK_PRESENT} ORDER BY t.disc_no, t.track_no, t.id"),
            params![album_id],
        )?;
        songs_in_order(&conn, ids, None)
    }

    // ----- browsing (artists, albums by artist, random) ----------------

    /// Every artist with how many albums and tracks they appear on, by name.
    pub fn list_artists(&self) -> Result<Vec<ArtistSummary>> {
        let conn = self.lock();
        let reps = song_reps(&song_rows(&conn, "ORDER BY t.id", [])?);
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT a.id, a.name, a.mbid, t.id, t.album_id
             FROM artists a
             JOIN track_artists ta ON ta.artist_id = a.id
             JOIN tracks t ON t.id = ta.track_id
             WHERE {TRACK_PRESENT}
             ORDER BY a.name COLLATE NOCASE, a.id"
        ))?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<i64>>(4)?,
            ))
        })?;
        // Artists in name order, each with its albums and songs (copies of a song count once)
        let mut out: Vec<(i64, String, Option<String>, SongCount)> = Vec::new();
        for r in rows {
            let (id, name, mbid, track, album) = r?;
            if out.last().is_none_or(|l| l.0 != id) {
                out.push((id, name, mbid, SongCount::default()));
            }
            if let Some(last) = out.last_mut() {
                last.3.add(&reps, track, album);
            }
        }
        out.into_iter()
            .map(|(id, name, mbid, c)| {
                Ok(ArtistSummary {
                    id,
                    name,
                    mbid: opt_mbid(mbid, "artists.mbid")?,
                    albums: c.albums.len() as u64,
                    tracks: c.songs.len() as u64,
                })
            })
            .collect()
    }

    pub fn get_artist(&self, id: i64) -> Result<Option<ArtistSummary>> {
        Ok(self.list_artists()?.into_iter().find(|a| a.id == id))
    }

    /// Songs with no artist at all, shown as "Unknown artist" (second review: they were in no
    /// artist's list). `None` when every song has one.
    pub fn unknown_artist(&self) -> Result<Option<ArtistSummary>> {
        let conn = self.lock();
        let reps = song_reps(&song_rows(&conn, "ORDER BY t.id", [])?);
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT t.id, t.album_id FROM tracks t
             WHERE NOT EXISTS (SELECT 1 FROM track_artists ta WHERE ta.track_id = t.id) AND {TRACK_PRESENT}"
        ))?;
        let mut count = SongCount::default();
        for r in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?)))? {
            let (track, album) = r?;
            count.add(&reps, track, album);
        }
        Ok((!count.songs.is_empty()).then(|| ArtistSummary {
            id: UNKNOWN_ARTIST_ID,
            name: "Unknown artist".into(),
            mbid: None,
            albums: count.albums.len() as u64,
            tracks: count.songs.len() as u64,
        }))
    }

    /// The songs behind [`Library::unknown_artist`], by album then track number.
    pub fn tracks_without_artist(&self) -> Result<Vec<Track>> {
        let conn = self.lock();
        let ids = candidates(
            &conn,
            &format!(
                "SELECT t.id, t.album_id FROM tracks t LEFT JOIN albums al ON al.id = t.album_id
                 WHERE NOT EXISTS (SELECT 1 FROM track_artists ta WHERE ta.track_id = t.id) AND {TRACK_PRESENT}
                 ORDER BY al.title, t.disc_no, t.track_no, t.title, t.id"
            ),
            [],
        )?;
        Ok(songs_only(songs_in_order(&conn, ids, None)?))
    }

    /// Albums an artist appears on, by year then title.
    pub fn artist_albums(&self, artist_id: i64) -> Result<Vec<Album>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT DISTINCT al.id, al.title, al.album_artist, al.release_mbid, al.release_group_mbid, al.year, al.upc, al.total_discs
             FROM albums al
             JOIN tracks t ON t.album_id = al.id
             JOIN track_artists ta ON ta.track_id = t.id
             WHERE ta.artist_id = ?1 AND EXISTS (SELECT 1 FROM files f WHERE f.track_id = t.id AND +f.missing = 0 AND NOT EXISTS (SELECT 1 FROM hidden_files ph WHERE ph.path = f.path))
             ORDER BY al.year, al.title COLLATE NOCASE",
        )?;
        let v = stmt
            .query_map(params![artist_id], row_to_album)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        v.into_iter().collect()
    }

    pub fn get_album(&self, id: i64) -> Result<Option<Album>> {
        let conn = self.lock();
        let a = conn
            .query_row(
                "SELECT id, title, album_artist, release_mbid, release_group_mbid, year, upc, total_discs FROM albums WHERE id = ?1",
                params![id],
                row_to_album,
            )
            .optional()?;
        a.transpose()
    }

    /// An artist's songs, one row per song ([`Library::album_tracks`]).
    pub fn artist_tracks(&self, artist_id: i64) -> Result<Vec<Track>> {
        Ok(songs_only(self.artist_songs(artist_id)?))
    }

    /// [`Library::artist_tracks`] with each song's other copies.
    pub fn artist_songs(&self, artist_id: i64) -> Result<Vec<Song>> {
        let conn = self.lock();
        let ids = candidates(
            &conn,
            &format!(
                "SELECT t.id, t.album_id FROM tracks t JOIN track_artists ta ON ta.track_id = t.id
                 LEFT JOIN albums al ON al.id = t.album_id
                 WHERE ta.artist_id = ?1 AND {TRACK_PRESENT} ORDER BY al.year, al.title, t.disc_no, t.track_no, t.id"
            ),
            params![artist_id],
        )?;
        songs_in_order(&conn, ids, None)
    }

    pub fn random_tracks(&self, limit: u32) -> Result<Vec<Track>> {
        let conn = self.lock();
        // Songs drawn, not files: a song kept in two copies is no likelier than one kept once
        let reps = song_reps(&song_rows(&conn, "ORDER BY t.id", [])?);
        let songs: std::collections::BTreeSet<i64> = reps.values().copied().collect();
        let list = format!(
            "[{}]",
            songs.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
        );
        let ids = candidates(
            &conn,
            "SELECT t.id, t.album_id FROM json_each(?1) j JOIN tracks t ON t.id = j.value ORDER BY RANDOM() LIMIT ?2",
            params![list, limit as i64],
        )?;
        Ok(songs_only(songs_in_order(&conn, ids, None)?))
    }

    /// Albums newest-first by id (a proxy for "recently added").
    pub fn recent_albums(&self, offset: u32, limit: u32) -> Result<Vec<Album>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(
            &format!(
                "SELECT id, title, album_artist, release_mbid, release_group_mbid, year, upc, total_discs FROM albums
                 WHERE {ALBUM_PRESENT} ORDER BY id DESC LIMIT ?1 OFFSET ?2"
            ),
        )?;
        let v = stmt
            .query_map(params![limit as i64, offset as i64], row_to_album)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        v.into_iter().collect()
    }

    // ----- bookmarks --------------------------------------------------------

    /// Add (or revive) a bookmark; returns the row. Idempotent: a live bookmark keeps its time,
    /// a revived one (cleared, not yet synced) takes the new time.
    pub fn add_bookmark(&self, id: &str, uri: &str, created_at: i64) -> Result<BookmarkRow> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO bookmarks(id, uri, created_at, synced_at, deleted) VALUES (?1, ?2, ?3, NULL, 0)
             ON CONFLICT(uri) DO UPDATE SET deleted = 0,
                 created_at = CASE WHEN bookmarks.deleted = 1 THEN excluded.created_at ELSE bookmarks.created_at END,
                 synced_at = CASE WHEN bookmarks.deleted = 1 THEN NULL ELSE bookmarks.synced_at END",
            params![id, uri, created_at],
        )?;
        drop(conn);
        Ok(self.bookmark_by_uri(uri)?.expect("just inserted"))
    }

    /// Tombstone a bookmark until the remote delete succeeds. `false` when unknown.
    pub fn remove_bookmark(&self, uri: &str) -> Result<bool> {
        let conn = self.lock();
        let n = conn.execute(
            "UPDATE bookmarks SET deleted = 1, synced_at = NULL WHERE uri = ?1 AND deleted = 0",
            params![uri],
        )?;
        Ok(n > 0)
    }

    pub fn bookmark_by_uri(&self, uri: &str) -> Result<Option<BookmarkRow>> {
        let conn = self.lock();
        conn.query_row(
            "SELECT id, uri, created_at, synced_at, deleted FROM bookmarks WHERE uri = ?1",
            params![uri],
            row_to_bookmark,
        )
        .optional()
        .map_err(Into::into)
    }

    /// Live bookmarks, newest first.
    pub fn list_bookmarks(&self) -> Result<Vec<BookmarkRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT id, uri, created_at, synced_at, deleted FROM bookmarks WHERE deleted = 0 ORDER BY created_at DESC",
        )?;
        let v = stmt
            .query_map([], row_to_bookmark)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(v)
    }

    /// Rows to push: never synced, changed since, or tombstoned.
    pub fn unsynced_bookmarks(&self) -> Result<Vec<BookmarkRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT id, uri, created_at, synced_at, deleted FROM bookmarks WHERE synced_at IS NULL ORDER BY created_at",
        )?;
        let v = stmt
            .query_map([], row_to_bookmark)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(v)
    }

    /// Queue a live bookmark for another push (the v1 migration).
    pub fn mark_bookmark_unsynced(&self, id: &str) -> Result<()> {
        self.lock().execute(
            "UPDATE bookmarks SET synced_at = NULL WHERE id = ?1 AND deleted = 0",
            params![id],
        )?;
        Ok(())
    }

    /// Mark pushed; a pushed tombstone is purged.
    pub fn mark_bookmark_synced(&self, id: &str, at_micros: i64) -> Result<()> {
        let conn = self.lock();
        conn.execute("DELETE FROM bookmarks WHERE id = ?1 AND deleted = 1", params![id])?;
        conn.execute(
            "UPDATE bookmarks SET synced_at = ?2 WHERE id = ?1",
            params![id, at_micros],
        )?;
        Ok(())
    }

    /// Tracks that go with `seed` for a library radio: same genres first, then
    /// the same artists, then anything; random within each tier, never the
    /// seed itself or anything in `exclude`. Up to `limit`.
    pub fn similar_tracks(&self, seed: &Track, exclude: &[TrackId], limit: usize) -> Result<Vec<Track>> {
        let mut out: Vec<Track> = Vec::new();
        let mut seen: std::collections::HashSet<i64> = exclude.iter().map(|t| t.0).collect();
        if let Some(id) = seed.id {
            seen.insert(id.0);
        }
        let mut push = |tracks: Vec<Track>, out: &mut Vec<Track>| {
            for t in tracks {
                if out.len() >= limit {
                    break;
                }
                if let Some(id) = t.id {
                    if seen.insert(id.0) {
                        out.push(t);
                    }
                }
            }
        };
        for g in &seed.genres {
            if let Some(genre) = self.genre_by_name(g)? {
                let mut tracks = self.genre_tracks(genre.id, 200)?;
                shuffle(&mut tracks);
                push(tracks, &mut out);
            }
        }
        for a in &seed.artists {
            if let Some(aid) = a.id {
                let mut tracks = self.artist_tracks(aid)?;
                shuffle(&mut tracks);
                push(tracks, &mut out);
            }
        }
        if out.len() < limit {
            push(self.random_tracks((limit * 2) as u32)?, &mut out);
        }
        Ok(out)
    }

    /// Tracks sharing genres with `seeds`, the most shared genres first (random among equals),
    /// never one in `exclude` (an artist's or album's own songs, all of them, not only the seeds);
    /// at most `limit` ids. An item page's "Similar" starts here.
    pub fn tracks_sharing_genres(&self, seeds: &[i64], exclude: &[i64], limit: u32) -> Result<Vec<i64>> {
        if seeds.is_empty() {
            return Ok(Vec::new());
        }
        // Numbers only, so the lists are written into the statement
        let ids = |v: &[i64]| v.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        let seed_list = ids(seeds);
        let list = ids(if exclude.is_empty() { seeds } else { exclude });
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT tg.track_id FROM track_genres tg
             JOIN (SELECT DISTINCT genre_id FROM track_genres WHERE track_id IN ({seed_list})) sg
               ON sg.genre_id = tg.genre_id
             WHERE tg.track_id NOT IN ({list})
             GROUP BY tg.track_id ORDER BY COUNT(*) DESC, random() LIMIT ?1"
        ))?;
        let ids = stmt
            .query_map(params![limit as i64], |r| r.get::<_, i64>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    // ----- backup -----------------------------------------------------------

    /// Write a consistent snapshot of the whole database to `path` (SQLite
    /// `VACUUM INTO`): safe while the node runs, and the copy is a plain
    /// database file that `restore` or `Library::open` can use. The target
    /// must not exist.
    pub fn backup_to(&self, path: &Path) -> Result<()> {
        if path.exists() {
            return Err(LibraryError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::new(std::io::ErrorKind::AlreadyExists, "exists"),
            });
        }
        let conn = self.lock();
        conn.execute("VACUUM INTO ?1", params![path.to_string_lossy()])?;
        Ok(())
    }

    /// Rewrite the database file in place and fold the write-ahead log into it: afterwards the
    /// file alone holds everything, and rows deleted before (sign-ins taken out of a backup copy)
    /// are gone from it, not just marked free.
    pub fn vacuum_into_place(&self) -> Result<()> {
        self.lock()
            .execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    /// What a backup carries, counted (shown before a restore and compared after it).
    pub fn backup_counts(&self) -> Result<BackupCounts> {
        let conn = self.lock();
        let n = |sql: &str| -> Result<u64> { Ok(conn.query_row(sql, [], |r| r.get::<_, i64>(0))? as u64) };
        Ok(BackupCounts {
            tracks: n("SELECT COUNT(*) FROM tracks")?,
            identified: n("SELECT COUNT(*) FROM tracks WHERE canonical_uri IS NOT NULL")?,
            albums: n("SELECT COUNT(*) FROM albums")?,
            playlists: n(
                "SELECT COUNT(*) FROM music_posts WHERE kind = 'pubky.music/playlist' AND deleted = 0",
            )?,
            bookmarks: n("SELECT COUNT(*) FROM bookmarks WHERE deleted = 0")?,
            tags: n("SELECT COUNT(*) FROM tags WHERE deleted = 0")?,
            plays: n("SELECT COUNT(*) FROM plays")?,
            folders: n("SELECT COUNT(*) FROM folders")?,
        })
    }

    /// Replace the whole database with the one at `path` while the node runs (SQLite's backup
    /// API copies it into this connection), then bring it up to this version's schema. Check the
    /// file first by opening it with [`Library::open`]: a broken one must never get this far.
    pub fn restore_from(&self, path: &Path) -> Result<()> {
        {
            let mut conn = self.lock();
            conn.restore(rusqlite::MAIN_DB, path, None::<fn(rusqlite::backup::Progress)>)?;
        }
        self.migrate()?;
        self.tidy_file_name_titles()?;
        Ok(())
    }

    /// Music that moved (a new disk, another machine): every file under `from` now lives under
    /// `to`. Tracks keep their ids, so matches, playlists, history and tags stay with them.
    /// Returns the number of files moved.
    pub fn move_folder(&self, from: &Path, to: &Path) -> Result<u64> {
        let from = from.to_string_lossy().trim_end_matches('/').to_string();
        let to = to.to_string_lossy().trim_end_matches('/').to_string();
        let mut conn = self.lock();
        let tx = conn.savepoint()?;
        let n = tx.execute(
            "UPDATE files SET path = ?2 || substr(path, length(?1) + 1)
             WHERE path = ?1 OR substr(path, 1, length(?1) + 1) = ?1 || '/'",
            params![from, to],
        )?;
        tx.execute(
            "UPDATE album_art SET file = ?2 || substr(file, length(?1) + 1)
             WHERE source = 'folder' AND substr(file, 1, length(?1) + 1) = ?1 || '/'",
            params![from, to],
        )?;
        // The owner's edits and the songs taken out move with their files
        for table in ["track_overrides", "hidden_files"] {
            tx.execute(
                &format!(
                    "UPDATE {table} SET path = ?2 || substr(path, length(?1) + 1)
                     WHERE path = ?1 OR substr(path, 1, length(?1) + 1) = ?1 || '/'"
                ),
                params![from, to],
            )?;
        }
        tx.execute("DELETE FROM folders WHERE path = ?1", params![from])?;
        tx.execute(
            "INSERT OR IGNORE INTO folders(path, enabled) VALUES (?1, 1)",
            params![to],
        )?;
        tx.commit()?;
        Ok(n as u64)
    }

    /// Forget covers whose image file is gone (a restore on another machine: the extracted
    /// covers live in the node's data folder, which a backup leaves out). Returns how many.
    pub fn drop_missing_art(&self) -> Result<u64> {
        let conn = self.lock();
        let rows: Vec<(i64, String)> = conn
            .prepare("SELECT album_id, file FROM album_art")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut n = 0;
        for (id, file) in rows {
            if !Path::new(&file).exists() {
                conn.execute("DELETE FROM album_art WHERE album_id = ?1", params![id])?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// Albums without a cover, each with one of its files that is present.
    pub fn albums_without_art(&self) -> Result<Vec<(i64, PathBuf)>> {
        let conn = self.lock();
        let v = conn
            .prepare(
                "SELECT al.id, MIN(f.path) FROM albums al
                 JOIN tracks t ON t.album_id = al.id
                 JOIN files f ON f.track_id = t.id AND f.missing = 0
                 LEFT JOIN album_art aa ON aa.album_id = al.id
                 WHERE aa.album_id IS NULL GROUP BY al.id",
            )?
            .query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, PathBuf::from(r.get::<_, String>(1)?)))
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(v)
    }

    /// Every present library file whose path starts with `prefix` (a folder ending in its separator,
    /// a share's or a drive's key ending in `/`), by path: its size and the first 16 hex digits of
    /// its content hash when one is kept for that size. The range on the path uses its index.
    pub fn files_under(&self, prefix: &str) -> Result<Vec<(String, u64, Option<String>)>> {
        let Some(last) = prefix.chars().last() else {
            return Ok(Vec::new());
        };
        // Every path with this prefix sorts before the prefix with its last character raised by one
        let upper = format!(
            "{}{}",
            &prefix[..prefix.len() - last.len_utf8()],
            char::from_u32(last as u32 + 1).unwrap_or(char::MAX)
        );
        let conn = self.lock();
        let v = conn
            .prepare(
                "SELECT f.path, f.size_bytes, substr(h.blake3, 1, 16) FROM files f
                 LEFT JOIN file_hashes h ON h.path = f.path AND h.size_bytes = f.size_bytes
                 WHERE f.missing = 0 AND f.path >= ?1 AND f.path < ?2 ORDER BY f.path",
            )?
            .query_map(params![prefix, upper], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64, r.get(2)?))
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(v)
    }

    /// Every present file under `root` with its album's id and title. The scanner uses it to
    /// find albums that one folder's files were split into.
    pub fn album_files_under(&self, root: &Path) -> Result<Vec<(PathBuf, i64, String)>> {
        let mut prefix = root.to_string_lossy().to_string();
        if !prefix.ends_with(std::path::MAIN_SEPARATOR) {
            prefix.push(std::path::MAIN_SEPARATOR);
        }
        let conn = self.lock();
        let v = conn
            .prepare(
                "SELECT f.path, al.id, al.title FROM files f
                 JOIN tracks t ON t.id = f.track_id
                 JOIN albums al ON al.id = t.album_id
                 WHERE f.missing = 0 AND f.path LIKE ?1 ESCAPE '\\'",
            )?
            .query_map(params![format!("{}%", like_escape(&prefix))], |r| {
                Ok((PathBuf::from(r.get::<_, String>(0)?), r.get(1)?, r.get(2)?))
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(v)
    }

    /// Drop albums no track belongs to any more (their tracks moved to another album when
    /// re-read), with their covers. Returns how many albums went.
    pub fn prune_empty_albums(&self) -> Result<u64> {
        let conn = self.lock();
        conn.execute(
            "DELETE FROM album_art WHERE album_id NOT IN (SELECT DISTINCT album_id FROM tracks WHERE album_id IS NOT NULL)",
            [],
        )?;
        Ok(conn.execute(
            "DELETE FROM albums WHERE id NOT IN (SELECT DISTINCT album_id FROM tracks WHERE album_id IS NOT NULL)",
            [],
        )? as u64)
    }

    // ----- lyrics -----------------------------------------------------------

    /// Store (or clear, with `None`) the lyrics of a track; `source` is
    /// `"lrc"` for a sidecar file or `"tag"` for embedded text.
    pub fn set_lyrics(&self, track: TrackId, lyrics: Option<(&str, &str)>) -> Result<()> {
        let conn = self.lock();
        match lyrics {
            Some((text, source)) => {
                conn.execute(
                    "INSERT INTO lyrics(track_id, text, source) VALUES (?1, ?2, ?3)
                     ON CONFLICT(track_id) DO UPDATE SET text = excluded.text, source = excluded.source",
                    params![track.0, text, source],
                )?;
            }
            None => {
                conn.execute("DELETE FROM lyrics WHERE track_id = ?1", params![track.0])?;
            }
        }
        Ok(())
    }

    /// The stored lyrics of a track as `(text, source)`.
    pub fn lyrics(&self, track: TrackId) -> Result<Option<(String, String)>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT text, source FROM lyrics WHERE track_id = ?1",
                params![track.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    // ----- measured loudness ------------------------------------------------

    /// Local files of tracks whose tags have no ReplayGain and that have no current
    /// measurement (none, or the file changed since): `(track, path, size, mtime)`.
    pub fn tracks_needing_loudness(&self, limit: u32) -> Result<Vec<(TrackId, PathBuf, i64, i64)>> {
        let conn = self.lock();
        let mut st = conn.prepare(
            "SELECT t.id, f.path, f.size_bytes, f.mtime_unix FROM tracks t
             JOIN files f ON f.track_id = t.id AND f.missing = 0 AND f.sample_kind = 'pcm'
             LEFT JOIN track_loudness l ON l.track_id = t.id
             WHERE t.replaygain_track_gain_db IS NULL
               AND f.path NOT LIKE '%://%'
               AND (l.track_id IS NULL OR l.size_bytes != f.size_bytes OR l.mtime_unix != f.mtime_unix)
             ORDER BY t.id LIMIT ?1",
        )?;
        let rows = st
            .query_map(params![limit], |r| {
                Ok((
                    TrackId(r.get(0)?),
                    PathBuf::from(r.get::<_, String>(1)?),
                    r.get(2)?,
                    r.get(3)?,
                ))
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// Record a measurement of `track`'s file as it was (`size`, `mtime`).
    pub fn set_loudness(&self, track: TrackId, lufs: f64, peak: f64, size: i64, mtime: i64) -> Result<()> {
        self.lock().execute(
            "INSERT INTO track_loudness(track_id, lufs, peak, size_bytes, mtime_unix, measured_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(track_id) DO UPDATE SET lufs = excluded.lufs, peak = excluded.peak,
               size_bytes = excluded.size_bytes, mtime_unix = excluded.mtime_unix,
               measured_at = excluded.measured_at",
            params![track.0, lufs, peak, size, mtime, now_unix()],
        )?;
        Ok(())
    }

    /// `track`'s current measurement: `(lufs, peak)`, while its file is unchanged. (A file that
    /// could not be measured is kept with a negative peak, so it isn't tried again; it has none.)
    pub fn loudness(&self, track: TrackId) -> Result<Option<(f64, f64)>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT l.lufs, l.peak FROM track_loudness l
                 JOIN files f ON f.track_id = l.track_id
                 WHERE l.track_id = ?1 AND f.size_bytes = l.size_bytes AND f.mtime_unix = l.mtime_unix
                   AND l.peak >= 0",
                params![track.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// The album's loudness from its tracks' measurements, `(lufs, peak)`: the duration-weighted
    /// energy mean and the highest peak. `None` unless every track on it is measured (and none
    /// has ReplayGain tags, which would mix two references).
    pub fn album_loudness(&self, album_id: i64) -> Result<Option<(f64, f64)>> {
        let conn = self.lock();
        let mut st = conn.prepare(
            "SELECT t.replaygain_track_gain_db, l.lufs, l.peak, COALESCE(t.duration_ms, 1)
             FROM tracks t
             LEFT JOIN files f ON f.track_id = t.id
             LEFT JOIN track_loudness l ON l.track_id = t.id
               AND l.size_bytes = f.size_bytes AND l.mtime_unix = f.mtime_unix
             WHERE t.album_id = ?1",
        )?;
        /// A track's ReplayGain tag, measured loudness and peak, and duration.
        type Row = (Option<f64>, Option<f64>, Option<f64>, i64);
        let rows: Vec<Row> = st
            .query_map(params![album_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?
            .collect::<std::result::Result<_, _>>()?;
        if rows.is_empty() {
            return Ok(None);
        }
        let (mut energy, mut weight, mut peak) = (0f64, 0f64, 0f64);
        for (rg, lufs, p, ms) in rows {
            let (None, Some(lufs), Some(p)) = (rg, lufs, p) else {
                return Ok(None);
            };
            if p < 0.0 {
                return Ok(None);
            }
            let w = ms.max(1) as f64;
            energy += w * 10f64.powf(lufs / 10.0);
            weight += w;
            peak = peak.max(p);
        }
        Ok(Some((10.0 * (energy / weight).log10(), peak)))
    }

    /// Local tracks without ReplayGain tags: `(measured, could not be measured, still to measure)`.
    pub fn loudness_counts(&self) -> Result<(u64, u64, u64)> {
        let conn = self.lock();
        let (measured, failed): (i64, i64) = conn.query_row(
            "SELECT COALESCE(SUM(l.peak >= 0), 0), COALESCE(SUM(l.peak < 0), 0)
             FROM track_loudness l JOIN files f ON f.track_id = l.track_id
             WHERE f.size_bytes = l.size_bytes AND f.mtime_unix = l.mtime_unix",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let todo: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tracks t
             JOIN files f ON f.track_id = t.id AND f.missing = 0 AND f.sample_kind = 'pcm'
             LEFT JOIN track_loudness l ON l.track_id = t.id
             WHERE t.replaygain_track_gain_db IS NULL AND f.path NOT LIKE '%://%'
               AND (l.track_id IS NULL OR l.size_bytes != f.size_bytes OR l.mtime_unix != f.mtime_unix)",
            [],
            |r| r.get(0),
        )?;
        Ok((measured as u64, failed as u64, todo as u64))
    }

    // ----- genres -----------------------------------------------------------

    /// Every genre with track and album counts, by name.
    pub fn list_genres(&self) -> Result<Vec<GenreSummary>> {
        let conn = self.lock();
        let reps = song_reps(&song_rows(&conn, "ORDER BY t.id", [])?);
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT g.id, g.name, t.id, t.album_id
             FROM genres g JOIN track_genres tg ON tg.genre_id = g.id JOIN tracks t ON t.id = tg.track_id
             WHERE {TRACK_PRESENT}
             ORDER BY g.name COLLATE NOCASE, g.id"
        ))?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        })?;
        let mut out: Vec<(i64, String, SongCount)> = Vec::new();
        for r in rows {
            let (id, name, track, album) = r?;
            if out.last().is_none_or(|l| l.0 != id) {
                out.push((id, name, SongCount::default()));
            }
            if let Some(last) = out.last_mut() {
                last.2.add(&reps, track, album);
            }
        }
        Ok(out
            .into_iter()
            .map(|(id, name, c)| GenreSummary {
                id,
                name,
                tracks: c.songs.len() as u64,
                albums: c.albums.len() as u64,
            })
            .collect())
    }

    pub fn get_genre(&self, id: i64) -> Result<Option<GenreSummary>> {
        Ok(self.list_genres()?.into_iter().find(|g| g.id == id))
    }

    pub fn genre_by_name(&self, name: &str) -> Result<Option<GenreSummary>> {
        Ok(self
            .list_genres()?
            .into_iter()
            .find(|g| g.name.eq_ignore_ascii_case(name.trim())))
    }

    /// Albums with at least one track of the genre, by artist then title.
    pub fn genre_albums(&self, genre_id: i64) -> Result<Vec<Album>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT DISTINCT al.id, al.title, al.album_artist, al.release_mbid, al.release_group_mbid, al.year, al.upc, al.total_discs
             FROM albums al JOIN tracks t ON t.album_id = al.id JOIN track_genres tg ON tg.track_id = t.id
             WHERE tg.genre_id = ?1 AND EXISTS (SELECT 1 FROM files f WHERE f.track_id = t.id AND +f.missing = 0 AND NOT EXISTS (SELECT 1 FROM hidden_files ph WHERE ph.path = f.path)) ORDER BY al.album_artist COLLATE NOCASE, al.year, al.title COLLATE NOCASE",
        )?;
        let v = stmt
            .query_map(params![genre_id], row_to_album)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        v.into_iter().collect()
    }

    pub fn genre_tracks(&self, genre_id: i64, limit: u32) -> Result<Vec<Track>> {
        let conn = self.lock();
        let ids = candidates(
            &conn,
            &format!(
                "SELECT t.id, t.album_id FROM tracks t JOIN track_genres tg ON tg.track_id = t.id
                 LEFT JOIN albums al ON al.id = t.album_id
                 WHERE tg.genre_id = ?1 AND {TRACK_PRESENT}
                 ORDER BY al.album_artist COLLATE NOCASE, al.title COLLATE NOCASE, t.disc_no, t.track_no, t.id"
            ),
            params![genre_id],
        )?;
        Ok(songs_only(songs_in_order(&conn, ids, Some(limit as usize))?))
    }

    // ----- tags -------------------------------------------------------------

    /// Add (or revive) a tag; idempotent on `(uri, label)`.
    pub fn add_tag(&self, id: &str, uri: &str, label: &str, created_at: i64) -> Result<TagRow> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO tags(id, uri, label, created_at, synced_at, deleted) VALUES (?1, ?2, ?3, ?4, NULL, 0)
             ON CONFLICT(uri, label) DO UPDATE SET deleted = 0,
                 synced_at = CASE WHEN tags.deleted = 1 THEN NULL ELSE tags.synced_at END",
            params![id, uri, label, created_at],
        )?;
        conn.query_row(
            "SELECT id, uri, label, created_at, synced_at, deleted FROM tags WHERE uri = ?1 AND label = ?2",
            params![uri, label],
            row_to_tag,
        )
        .map_err(Into::into)
    }

    /// Tombstone a tag until the remote delete succeeds. `false` when unknown.
    pub fn remove_tag(&self, uri: &str, label: &str) -> Result<bool> {
        let conn = self.lock();
        let n = conn.execute(
            "UPDATE tags SET deleted = 1, synced_at = NULL WHERE uri = ?1 AND label = ?2 AND deleted = 0",
            params![uri, label],
        )?;
        Ok(n > 0)
    }

    /// Live tags, newest first; for one URI when given.
    pub fn list_tags(&self, uri: Option<&str>) -> Result<Vec<TagRow>> {
        let conn = self.lock();
        let v = match uri {
            Some(u) => {
                let mut stmt = conn.prepare_cached(
                    "SELECT id, uri, label, created_at, synced_at, deleted FROM tags WHERE deleted = 0 AND uri = ?1 ORDER BY created_at DESC",
                )?;
                let v = stmt
                    .query_map(params![u], row_to_tag)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                v
            }
            None => {
                let mut stmt = conn.prepare_cached(
                    "SELECT id, uri, label, created_at, synced_at, deleted FROM tags WHERE deleted = 0 ORDER BY created_at DESC",
                )?;
                let v = stmt
                    .query_map([], row_to_tag)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                v
            }
        };
        Ok(v)
    }

    pub fn unsynced_tags(&self) -> Result<Vec<TagRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT id, uri, label, created_at, synced_at, deleted FROM tags WHERE synced_at IS NULL ORDER BY created_at",
        )?;
        let v = stmt
            .query_map([], row_to_tag)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(v)
    }

    /// Mark pushed; a pushed tombstone is purged.
    pub fn mark_tag_synced(&self, id: &str, at_micros: i64) -> Result<()> {
        let conn = self.lock();
        conn.execute("DELETE FROM tags WHERE id = ?1 AND deleted = 1", params![id])?;
        conn.execute(
            "UPDATE tags SET synced_at = ?2 WHERE id = ?1",
            params![id, at_micros],
        )?;
        Ok(())
    }

    /// Every tag id the node holds, removed ones included (a sync skips downloading these).
    pub fn tag_ids(&self) -> Result<std::collections::HashSet<String>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached("SELECT id FROM tags")?;
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(ids)
    }

    pub fn tag_by_id(&self, id: &str) -> Result<Option<TagRow>> {
        let conn = self.lock();
        conn.query_row(
            "SELECT id, uri, label, created_at, synced_at, deleted FROM tags WHERE id = ?1",
            params![id],
            row_to_tag,
        )
        .optional()
        .map_err(Into::into)
    }

    /// The album with this release MBID, if any.
    pub fn album_by_release_mbid(&self, mbid: Mbid) -> Result<Option<Album>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT id, title, album_artist, release_mbid, release_group_mbid, year, upc, total_discs FROM albums WHERE release_mbid = ?1 LIMIT 1",
        )?;
        let v = stmt
            .query_map(params![mbid.to_string()], row_to_album)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        v.into_iter().next().transpose()
    }

    /// The album with this UPC / barcode, if any.
    pub fn album_by_upc(&self, upc: &Upc) -> Result<Option<Album>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT id, title, album_artist, release_mbid, release_group_mbid, year, upc, total_discs FROM albums WHERE upc = ?1 LIMIT 1",
        )?;
        let v = stmt
            .query_map(params![upc.to_string()], row_to_album)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        v.into_iter().next().transpose()
    }

    /// The album with this release-group MBID, if any.
    pub fn album_by_release_group_mbid(&self, mbid: Mbid) -> Result<Option<Album>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT id, title, album_artist, release_mbid, release_group_mbid, year, upc, total_discs FROM albums WHERE release_group_mbid = ?1 LIMIT 1",
        )?;
        let v = stmt
            .query_map(params![mbid.to_string()], row_to_album)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        v.into_iter().next().transpose()
    }

    /// The artist with this MBID, if any.
    pub fn artist_by_mbid(&self, mbid: Mbid) -> Result<Option<ArtistSummary>> {
        let id: Option<i64> = {
            let conn = self.lock();
            conn.query_row(
                "SELECT id FROM artists WHERE mbid = ?1 LIMIT 1",
                params![mbid.to_string()],
                |r| r.get(0),
            )
            .optional()?
        };
        match id {
            Some(id) => self.get_artist(id),
            None => Ok(None),
        }
    }

    /// Tracks whose file name (last path segment) is `name`.
    pub fn tracks_by_file_name(&self, name: &str) -> Result<Vec<Track>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(&format!(
            "{TRACK_SELECT} WHERE fi.path LIKE ?1 ESCAPE '\\' AND fi.missing = 0 LIMIT 20"
        ))?;
        let escaped = name.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        let raws: Vec<RawTrack> = stmt
            .query_map(params![format!("%/{escaped}")], row_to_track_raw)?
            .collect::<std::result::Result<_, _>>()?;
        raws.into_iter().map(|raw| finish_track(&conn, raw)).collect()
    }

    /// The library track for a title and (optional) artist: the first search
    /// hit whose title matches exactly, preferring one by that artist.
    pub fn find_by_title_artist(&self, title: &str, artist: Option<&str>) -> Result<Option<Track>> {
        let title = title.trim();
        if title.is_empty() {
            return Ok(None);
        }
        let artist = artist.unwrap_or("").trim();
        let q = if artist.is_empty() {
            title.to_string()
        } else {
            format!("{title} {artist}")
        };
        let mut hits = self.search_tracks(&q, 20)?;
        if hits.is_empty() && !artist.is_empty() {
            hits = self.search_tracks(title, 20)?;
        }
        let norm = |s: &str| s.trim().to_lowercase();
        let first_artist = artist.split([',', ';', '&']).next().unwrap_or("").trim();
        Ok(hits
            .iter()
            .find(|t| {
                norm(&t.title) == norm(title)
                    && (artist.is_empty()
                        || t.artists
                            .iter()
                            .any(|a| norm(&a.name) == norm(artist) || norm(&a.name) == norm(first_artist)))
            })
            .or_else(|| hits.iter().find(|t| norm(&t.title) == norm(title)))
            .cloned())
    }

    /// Match an imported playlist entry: exact path, then file name, then
    /// title and artist.
    pub fn resolve_import(&self, t: &ImportedTrack) -> Result<Option<Track>> {
        if let Some(p) = t.path.as_deref().and_then(crate::import::local_path) {
            if let Some(row) = self.file_by_path(&p)? {
                if let Some(track) = self.get_track(row.track_id)? {
                    return Ok(Some(track));
                }
            }
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                let hits = self.tracks_by_file_name(name)?;
                if hits.len() == 1 {
                    return Ok(hits.into_iter().next());
                }
                if let Some(h) = hits.into_iter().find(|h| h.title.eq_ignore_ascii_case(&t.title)) {
                    return Ok(Some(h));
                }
            }
        }
        self.find_by_title_artist(&t.title, t.artist.as_deref())
    }

    /// Record that `track` started playing on `zone`; returns the entry id.
    /// Every played track's play count and last start (unix seconds).
    pub fn play_stats(&self) -> Result<std::collections::HashMap<i64, (u64, i64)>> {
        let conn = self.lock();
        let mut stmt =
            conn.prepare("SELECT track_id, COUNT(*), MAX(played_at) FROM plays GROUP BY track_id")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                (r.get::<_, i64>(1)? as u64, r.get::<_, i64>(2)?),
            ))
        })?;
        let mut out = std::collections::HashMap::new();
        for row in rows {
            let (id, v) = row?;
            out.insert(id, v);
        }
        Ok(out)
    }

    /// When a track last started (unix seconds), if ever.
    pub fn last_played(&self, track: TrackId) -> Result<Option<i64>> {
        Ok(self.lock().query_row(
            "SELECT MAX(played_at) FROM plays WHERE track_id = ?1",
            params![track.0],
            |r| r.get::<_, Option<i64>>(0),
        )?)
    }

    pub fn record_play(&self, track: TrackId, zone: Option<&str>) -> Result<i64> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO plays(track_id, zone, played_at) VALUES (?1, ?2, ?3)",
            params![track.0, zone, now_unix()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// A play at a given time (a device that played offline reports it later). The same track,
    /// zone and time again is not recorded twice (a resent report), checked on the device's own
    /// time before anything is changed; returns whether it was new. A device clock up to
    /// [`PLAY_CLOCK_SLACK`] ahead is kept as it is, so its resent reports still match; a time
    /// further in the future counts as now.
    pub fn record_play_at(&self, track: TrackId, zone: Option<&str>, at: i64) -> Result<bool> {
        let conn = self.lock();
        let seen: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM plays WHERE track_id = ?1 AND zone IS ?2 AND played_at = ?3)",
            params![track.0, zone, at],
            |r| r.get(0),
        )?;
        if seen {
            return Ok(false);
        }
        let now = now_unix();
        let at = if at > now + PLAY_CLOCK_SLACK { now } else { at };
        let n = conn.execute(
            "INSERT INTO plays(track_id, zone, played_at) SELECT ?1, ?2, ?3
             WHERE NOT EXISTS (SELECT 1 FROM plays WHERE track_id = ?1 AND zone IS ?2 AND played_at = ?3)",
            params![track.0, zone, at],
        )?;
        Ok(n > 0)
    }

    /// Newest plays first.
    pub fn history(&self, limit: u32) -> Result<Vec<PlayEntry>> {
        let conn = self.lock();
        let sql = format!(
            "{} JOIN plays p ON p.track_id = t.id ORDER BY p.played_at DESC, p.id DESC LIMIT ?1",
            track_select_with("p.id, p.zone, p.played_at")
        );
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows: Vec<(RawTrack, i64, Option<String>, i64)> = stmt
            .query_map(params![limit as i64], |r| {
                Ok((
                    row_to_track_raw(r)?,
                    r.get(TRACK_COLUMNS)?,
                    r.get(TRACK_COLUMNS + 1)?,
                    r.get(TRACK_COLUMNS + 2)?,
                ))
            })?
            .collect::<std::result::Result<_, _>>()?;
        rows.into_iter()
            .map(|(raw, id, zone, played_at)| {
                Ok(PlayEntry {
                    id,
                    track: finish_track(&conn, raw)?,
                    zone,
                    played_at,
                })
            })
            .collect()
    }

    /// Plays before `before` (unix seconds; all when `None`), newest first, at most `limit`.
    pub fn plays_before(&self, before: Option<i64>, limit: u32) -> Result<Vec<PlayEntry>> {
        let conn = self.lock();
        let sql = format!(
            "{} JOIN plays p ON p.track_id = t.id WHERE p.played_at < ?1 ORDER BY p.played_at DESC, p.id DESC LIMIT ?2",
            track_select_with("p.id, p.zone, p.played_at")
        );
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows: Vec<(RawTrack, i64, Option<String>, i64)> = stmt
            .query_map(params![before.unwrap_or(i64::MAX), limit as i64], |r| {
                Ok((
                    row_to_track_raw(r)?,
                    r.get(TRACK_COLUMNS)?,
                    r.get(TRACK_COLUMNS + 1)?,
                    r.get(TRACK_COLUMNS + 2)?,
                ))
            })?
            .collect::<std::result::Result<_, _>>()?;
        rows.into_iter()
            .map(|(raw, id, zone, played_at)| {
                Ok(PlayEntry {
                    id,
                    track: finish_track(&conn, raw)?,
                    zone,
                    played_at,
                })
            })
            .collect()
    }

    /// Every play since `since` (unix seconds; all when `None`), as `(track id, played_at)`: no
    /// track is read, so counting a long history stays cheap (the stats).
    pub fn play_times_since(&self, since: Option<i64>) -> Result<Vec<(i64, i64)>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached("SELECT track_id, played_at FROM plays WHERE played_at >= ?1")?;
        let rows = stmt
            .query_map([since.unwrap_or(i64::MIN)], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// Forget every play (the node's own history; nothing of it is published). Returns how many.
    pub fn clear_plays(&self) -> Result<usize> {
        Ok(self.lock().execute("DELETE FROM plays", [])?)
    }

    /// Tracks by play count, most played first.
    pub fn most_played_tracks(&self, limit: u32) -> Result<Vec<PlayedTrack>> {
        let conn = self.lock();
        let sql = format!(
            "{} JOIN (SELECT track_id, COUNT(*) AS n, MAX(played_at) AS last FROM plays GROUP BY track_id) p
             ON p.track_id = t.id ORDER BY p.n DESC, p.last DESC LIMIT ?1",
            track_select_with("p.n, p.last")
        );
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows: Vec<(RawTrack, i64, i64)> = stmt
            .query_map(params![limit as i64], |r| {
                Ok((
                    row_to_track_raw(r)?,
                    r.get(TRACK_COLUMNS)?,
                    r.get(TRACK_COLUMNS + 1)?,
                ))
            })?
            .collect::<std::result::Result<_, _>>()?;
        rows.into_iter()
            .map(|(raw, n, last)| {
                Ok(PlayedTrack {
                    track: finish_track(&conn, raw)?,
                    plays: n.max(0) as u64,
                    last_played_at: last,
                })
            })
            .collect()
    }

    /// Albums whose tracks played most recently, newest first.
    pub fn recently_played_albums(&self, limit: u32) -> Result<Vec<Album>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(
            &format!("SELECT al.id, al.title, al.album_artist, al.release_mbid, al.release_group_mbid, al.year, al.upc, al.total_discs
             FROM albums al JOIN (SELECT t.album_id AS album_id, MAX(p.played_at) AS last FROM plays p
                 JOIN tracks t ON t.id = p.track_id WHERE t.album_id IS NOT NULL GROUP BY t.album_id) r
             ON r.album_id = al.id WHERE {ALBUM_PRESENT_AL} ORDER BY r.last DESC LIMIT ?1"),
        )?;
        let v = stmt
            .query_map(params![limit as i64], row_to_album)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        v.into_iter().collect()
    }

    /// Albums by total plays of their tracks, most played first.
    pub fn most_played_albums(&self, limit: u32) -> Result<Vec<Album>> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(
            &format!("SELECT al.id, al.title, al.album_artist, al.release_mbid, al.release_group_mbid, al.year, al.upc, al.total_discs
             FROM albums al JOIN (SELECT t.album_id AS album_id, COUNT(*) AS n, MAX(p.played_at) AS last FROM plays p
                 JOIN tracks t ON t.id = p.track_id WHERE t.album_id IS NOT NULL GROUP BY t.album_id) r
             ON r.album_id = al.id WHERE {ALBUM_PRESENT_AL} ORDER BY r.n DESC, r.last DESC LIMIT ?1"),
        )?;
        let v = stmt
            .query_map(params![limit as i64], row_to_album)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        v.into_iter().collect()
    }

    /// Plays of one track.
    pub fn play_count(&self, track: TrackId) -> Result<u64> {
        let conn = self.lock();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM plays WHERE track_id = ?1",
            params![track.0],
            |r| r.get(0),
        )?;
        Ok(n.max(0) as u64)
    }

    pub fn search_albums(&self, query: &str, limit: u32) -> Result<Vec<Album>> {
        let like = format!("%{}%", query.trim());
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT id, title, album_artist, release_mbid, release_group_mbid, year, upc, total_discs FROM albums
             WHERE title LIKE ?1 OR COALESCE(album_artist, '') LIKE ?1 ORDER BY title COLLATE NOCASE LIMIT ?2",
        )?;
        let v = stmt
            .query_map(params![like, limit as i64], row_to_album)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        v.into_iter().collect()
    }

    // ----- settings (JSON key/value) ---------------------------------------

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .lock()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    /// How many bytes the setting `key` holds (`None`: no such setting), from the row's header:
    /// the value itself is not read, so a caller can refuse an oversized one before it is.
    pub fn setting_bytes(&self, key: &str) -> Result<Option<u64>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT octet_length(value) FROM settings WHERE key = ?1",
                [key],
                |r| r.get::<_, Option<i64>>(0),
            )
            .optional()?
            .map(|n| u64::try_from(n.unwrap_or(0)).unwrap_or(0)))
    }

    // ---- identification (crate::identify) ----

    /// A cheap content key for the fingerprint cache: size plus BLAKE3 of
    /// the first and last MiB. Renames and retags of the same audio keep it
    /// only when the bytes at both ends are unchanged; good enough to avoid
    /// a second AcoustID lookup for a moved file.
    pub fn file_key(path: &std::path::Path) -> std::io::Result<String> {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(path)?;
        let len = f.metadata()?.len();
        let mut h = blake3::Hasher::new();
        h.update(&len.to_le_bytes());
        let mut buf = vec![0u8; 1 << 20];
        let n = f.read(&mut buf)?;
        h.update(&buf[..n]);
        if len > (2 << 20) {
            f.seek(SeekFrom::End(-(1 << 20)))?;
            let n = f.read(&mut buf)?;
            h.update(&buf[..n]);
        }
        Ok(h.finalize().to_hex()[..32].to_string())
    }

    /// The cached fingerprint and AcoustID answer for a file key.
    pub fn cached_fingerprint(&self, key: &str) -> Result<Option<(u64, String, Option<String>)>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT duration_secs, fingerprint, lookup_json FROM fingerprints WHERE file_key = ?1",
                params![key],
                |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?, r.get(2)?)),
            )
            .optional()?)
    }

    /// Whether any AcoustID answer is cached (else no file needs reading for one).
    pub fn has_cached_lookups(&self) -> Result<bool> {
        Ok(self.lock().query_row(
            "SELECT EXISTS(SELECT 1 FROM fingerprints WHERE lookup_json IS NOT NULL)",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn save_fingerprint(&self, key: &str, duration_secs: u64, fingerprint: &str) -> Result<()> {
        self.lock().execute(
            "INSERT INTO fingerprints(file_key, duration_secs, fingerprint) VALUES (?1, ?2, ?3)
             ON CONFLICT(file_key) DO UPDATE SET duration_secs = excluded.duration_secs, fingerprint = excluded.fingerprint",
            params![key, duration_secs as i64, fingerprint],
        )?;
        Ok(())
    }

    pub fn save_lookup(&self, key: &str, json: &str) -> Result<()> {
        self.lock().execute(
            "UPDATE fingerprints SET lookup_json = ?2, looked_up_at = ?3 WHERE file_key = ?1",
            params![key, json, now_unix()],
        )?;
        Ok(())
    }

    /// Set a track's canonical identity found after the scan (fingerprint,
    /// release match, the owner's confirmation). Tag-derived identities are
    /// set by the scanner; this never overwrites one of those, and only the owner replaces the
    /// owner's own choice.
    pub fn set_identity(
        &self,
        track: i64,
        uri: &MusicUri,
        confidence: Confidence,
        acoustid: Option<&str>,
    ) -> Result<bool> {
        let n = self.lock().execute(
            "UPDATE tracks SET canonical_uri = ?2, canonical_confidence_source = ?3, canonical_confidence_score = ?4,
             acoustid = COALESCE(?5, acoustid)
             WHERE id = ?1 AND (canonical_confidence_source IS NULL OR canonical_confidence_source NOT IN ('tagged_mbid','tagged_isrc'))
               AND (?3 = 'confirmed_by_user' OR COALESCE(canonical_confidence_source, '') <> 'confirmed_by_user')",
            params![track, uri.to_url(), confidence_source(&confidence), confidence.score() as f64, acoustid],
        )?;
        Ok(n > 0)
    }

    pub fn set_identify_state(&self, track: i64, state: &str, detail: Option<&str>) -> Result<()> {
        self.lock().execute(
            "INSERT INTO identify_state(track_id, state, detail, checked_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(track_id) DO UPDATE SET state = excluded.state, detail = excluded.detail, checked_at = excluded.checked_at",
            params![track, state, detail, now_unix()],
        )?;
        Ok(())
    }

    /// Remember the release a match was disambiguated to.
    pub fn set_identify_release(&self, track: i64, release: &Mbid) -> Result<()> {
        self.lock().execute(
            "UPDATE identify_state SET release_mbid = ?2 WHERE track_id = ?1",
            params![track, release.to_string()],
        )?;
        Ok(())
    }

    /// Tracks whose identity identification found (not their tags), with the
    /// release when known: candidates for "write IDs into my files".
    /// Tracks credited to more than one artist (a file whose tags may name them as one).
    pub fn tracks_with_several_artists(&self) -> Result<Vec<Track>> {
        let ids: Vec<i64> = {
            let conn = self.lock();
            let mut stmt = conn.prepare(
                "SELECT ta.track_id FROM track_artists ta JOIN files f ON f.track_id = ta.track_id AND f.missing = 0
                 GROUP BY ta.track_id HAVING COUNT(*) > 1 ORDER BY ta.track_id",
            )?;
            let v = stmt
                .query_map([], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?;
            v
        };
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(t) = self.get_track(TrackId(id))? {
                out.push(t);
            }
        }
        Ok(out)
    }

    pub fn identified_not_tagged(&self) -> Result<Vec<(Track, Option<Mbid>)>> {
        let tracks = self.tracks_where(
            "t.canonical_confidence_source IN ('fingerprint','fingerprint+release','album_fit','tag_match','confirmed_by_user') AND t.canonical_uri LIKE ?1 || '%'",
            "https://musicbrainz.org/recording/".into(),
        )?;
        let conn = self.lock();
        Ok(tracks
            .into_iter()
            .map(|t| {
                let rel: Option<String> = t.id.and_then(|id| {
                    conn.query_row(
                        "SELECT release_mbid FROM identify_state WHERE track_id = ?1",
                        params![id.0],
                        |r| r.get(0),
                    )
                    .optional()
                    .ok()
                    .flatten()
                    .flatten()
                });
                let rel = rel.and_then(|r| Mbid::parse(&r).ok());
                (t, rel)
            })
            .collect())
    }

    /// Tracks without a canonical identity that identification has not
    /// looked at yet (with `retry`, also those it found nothing for or
    /// failed on; suggestions wait for the owner either way, and songs the owner left as they are
    /// stay so).
    pub fn tracks_to_identify(&self, retry: bool) -> Result<Vec<Track>> {
        let clause = if retry {
            "t.canonical_uri IS NULL AND ?1 = '' AND COALESCE((SELECT state FROM identify_state s WHERE s.track_id = t.id), '') NOT IN ('suggested', 'skipped')"
        } else {
            "t.canonical_uri IS NULL AND ?1 = '' AND NOT EXISTS (SELECT 1 FROM identify_state s WHERE s.track_id = t.id)"
        };
        self.tracks_where(clause, String::new())
    }

    /// Every track, in one query (identification groups them by folder).
    pub fn all_tracks(&self) -> Result<Vec<Track>> {
        self.tracks_where("?1 = ''", String::new())
    }

    /// Audio files in the folder of `path` that the library knows.
    pub fn tracks_in_folder(&self, dir: &std::path::Path) -> Result<Vec<Track>> {
        let prefix = format!("{}/", dir.to_string_lossy().trim_end_matches('/'));
        let all = self.tracks_where("fi.path LIKE ?1 || '%'", prefix)?;
        Ok(all
            .into_iter()
            .filter(|t| t.file.path.parent() == Some(dir))
            .collect())
    }

    pub fn add_suggestions(&self, track: i64, candidates: &[crate::identify::Candidate]) -> Result<()> {
        insert_suggestions(&self.lock(), track, candidates)
    }
}

/// Add suggestions for a song, leaving out what the owner said "Not this" to.
fn insert_suggestions(
    conn: &Connection,
    track: i64,
    candidates: &[crate::identify::Candidate],
) -> Result<()> {
    for c in candidates {
        // What the owner said "Not this" to stays out
        if rejected(conn, track, &c.recording.to_string())? {
            continue;
        }
        conn.execute(
                "INSERT INTO identify_suggestions(track_id, recording_mbid, release_mbid, title, artist, release_title, score, source, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(track_id, recording_mbid) DO UPDATE SET
                   score = CASE
                     WHEN identify_suggestions.source = excluded.source THEN MAX(identify_suggestions.score, excluded.score)
                     WHEN excluded.source = 'fingerprint' THEN excluded.score
                     WHEN identify_suggestions.source = 'fingerprint' THEN identify_suggestions.score
                     ELSE MAX(identify_suggestions.score, excluded.score) END,
                   source = CASE WHEN excluded.source = 'fingerprint' THEN excluded.source ELSE identify_suggestions.source END,
                   release_mbid = COALESCE(excluded.release_mbid, release_mbid),
                   title = COALESCE(excluded.title, title), artist = COALESCE(excluded.artist, artist),
                   release_title = COALESCE(excluded.release_title, release_title)",
                params![
                    track,
                    c.recording.to_string(),
                    c.release.map(|r| r.to_string()),
                    c.title,
                    c.artist,
                    c.release_title,
                    c.score as f64,
                    c.source,
                    now_unix()
                ],
            )?;
    }
    Ok(())
}

impl Library {
    /// Open suggestions, grouped by track (best first within a track).
    /// One track's open suggestions, best first.
    /// A song's open options, also when the owner left it as it is (its own page lists them).
    pub fn suggestions_for(&self, track: i64) -> Result<Vec<SuggestionRow>> {
        self.suggestions_for_tracks(&[track])
    }

    pub fn suggestions(&self) -> Result<Vec<SuggestionRow>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.track_id, s.recording_mbid, s.release_mbid, s.title, s.artist, s.release_title, s.score, s.source
             FROM identify_suggestions s JOIN tracks t ON t.id = s.track_id
             WHERE t.canonical_uri IS NULL AND NOT EXISTS (SELECT 1 FROM identify_state k WHERE k.track_id = t.id AND k.state = 'skipped')
             ORDER BY t.album_id, t.disc_no, t.track_no, s.track_id, s.score DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SuggestionRow {
                    id: r.get(0)?,
                    track_id: r.get(1)?,
                    recording_mbid: r.get(2)?,
                    release_mbid: r.get(3)?,
                    title: r.get(4)?,
                    artist: r.get(5)?,
                    release_title: r.get(6)?,
                    score: r.get::<_, f64>(7)? as f32,
                    source: r.get(8)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The owner confirms a suggestion: the track gets that recording as its
    /// identity (`confirmed_by_user`) and its other suggestions go.
    /// `None` when there is no such suggestion, or its song got an identity meanwhile.
    pub fn accept_suggestion(&self, id: i64) -> Result<Option<(i64, MusicUri)>> {
        let row: Option<(i64, String)> = self
            .lock()
            .query_row(
                "SELECT track_id, recording_mbid FROM identify_suggestions WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((track, mbid)) = row else { return Ok(None) };
        let choice = ConfirmChoice {
            track_id: track,
            suggestion_id: id,
            recording_mbid: mbid,
        };
        let uri = recording_uri(&choice.recording_mbid)?;
        match self.confirm_suggestions(&[choice])? {
            ConfirmReport::Confirmed(_) => Ok(Some((track, uri))),
            ConfirmReport::Stale(_) => Ok(None),
        }
    }

    /// The owner confirms several suggestions at once (album review). Each choice must still be
    /// open as the owner saw it: the suggestion there, for that song and recording, the song
    /// without an identity, each song once. Then every song gets its recording
    /// (`confirmed_by_user`) and loses its other suggestions, all in one transaction; otherwise
    /// nothing is stored and the stale choices say why.
    pub fn confirm_suggestions(&self, choices: &[ConfirmChoice]) -> Result<ConfirmReport> {
        immediately(&mut self.lock(), |conn| {
            let tx = conn.savepoint()?;
            let mut stale = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let mut uris = Vec::with_capacity(choices.len());
            for c in choices {
                let why = |reason: &str| StaleChoice {
                    track_id: c.track_id,
                    suggestion_id: c.suggestion_id,
                    reason: reason.into(),
                };
                if !seen.insert(c.track_id) {
                    stale.push(why("duplicate"));
                    continue;
                }
                let row: Option<(i64, String)> = tx
                    .query_row(
                        "SELECT track_id, recording_mbid FROM identify_suggestions WHERE id = ?1",
                        params![c.suggestion_id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                let Some((track, rec)) = row else {
                    stale.push(why("gone"));
                    continue;
                };
                if track != c.track_id || rec != c.recording_mbid {
                    stale.push(why("changed"));
                    continue;
                }
                let identified: Option<Option<String>> = tx
                    .query_row(
                        "SELECT canonical_uri FROM tracks WHERE id = ?1",
                        params![track],
                        |r| r.get(0),
                    )
                    .optional()?;
                match identified {
                    None => stale.push(why("gone")),
                    Some(Some(_)) => stale.push(why("identified")),
                    Some(None) => uris.push((track, recording_uri(&rec)?)),
                }
            }
            if !stale.is_empty() {
                // Dropping the savepoint rolls back (nothing was written)
                return Ok(ConfirmReport::Stale(stale));
            }
            let now = now_unix();
            for (track, uri) in &uris {
                tx.execute(
                "UPDATE tracks SET canonical_uri = ?2, canonical_confidence_source = 'confirmed_by_user', canonical_confidence_score = ?3
                 WHERE id = ?1 AND canonical_uri IS NULL",
                params![track, uri.to_url(), Confidence::ConfirmedByUser.score() as f64],
            )?;
                tx.execute(
                    "DELETE FROM identify_suggestions WHERE track_id = ?1",
                    params![track],
                )?;
                tx.execute(
                "INSERT INTO identify_state(track_id, state, detail, checked_at) VALUES (?1, 'identified', 'confirmed_by_user', ?2)
                 ON CONFLICT(track_id) DO UPDATE SET state = excluded.state, detail = excluded.detail, checked_at = excluded.checked_at",
                params![track, now],
            )?;
            }
            tx.commit()?;
            Ok(ConfirmReport::Confirmed(
                uris.into_iter().map(|(t, _)| t).collect(),
            ))
        })
    }

    /// Songs with open suggestions and their file's path, in one cheap query (album review
    /// groups them by folder, then reads only the page's songs).
    pub fn suggested_track_paths(&self) -> Result<Vec<(i64, String)>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT t.id, f.path FROM tracks t JOIN files f ON f.track_id = t.id AND f.missing = 0
             WHERE t.canonical_uri IS NULL AND EXISTS (SELECT 1 FROM identify_suggestions s WHERE s.track_id = t.id)
               AND NOT EXISTS (SELECT 1 FROM identify_state k WHERE k.track_id = t.id AND k.state = 'skipped')
             ORDER BY t.id",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// The open suggestions of these songs, best first within a song.
    pub fn suggestions_for_tracks(&self, tracks: &[i64]) -> Result<Vec<SuggestionRow>> {
        let conn = self.lock();
        let mut out = Vec::new();
        for chunk in tracks.chunks(500) {
            let marks = vec!["?"; chunk.len()].join(",");
            let mut stmt = conn.prepare(&format!(
                "SELECT id, track_id, recording_mbid, release_mbid, title, artist, release_title, score, source
                 FROM identify_suggestions WHERE track_id IN ({marks}) ORDER BY track_id, score DESC, id"
            ))?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
                Ok(SuggestionRow {
                    id: r.get(0)?,
                    track_id: r.get(1)?,
                    recording_mbid: r.get(2)?,
                    release_mbid: r.get(3)?,
                    title: r.get(4)?,
                    artist: r.get(5)?,
                    release_title: r.get(6)?,
                    score: r.get::<_, f64>(7)? as f32,
                    source: r.get(8)?,
                })
            })?;
            for row in rows {
                out.push(row?);
            }
        }
        Ok(out)
    }

    /// Songs with open suggestions, in one query (album review groups them by folder).
    pub fn suggested_tracks(&self) -> Result<Vec<Track>> {
        self.tracks_where(
            "t.canonical_uri IS NULL AND ?1 = '' AND EXISTS (SELECT 1 FROM identify_suggestions s WHERE s.track_id = t.id) AND NOT EXISTS (SELECT 1 FROM identify_state k WHERE k.track_id = t.id AND k.state = 'skipped')",
            String::new(),
        )
    }

    /// "Leave as is" for songs waiting for review: each is kept without an identity, out of
    /// review and of every later run, its options kept for [`Self::unskip_review`]. Songs with an
    /// identity, nothing to review or already left are not changed. How many were left.
    pub fn skip_review(&self, tracks: &[i64]) -> Result<usize> {
        immediately(&mut self.lock(), |conn| {
            let tx = conn.savepoint()?;
            let mut n = 0;
            for t in tracks {
                n += tx.execute(
                    "INSERT INTO identify_state(track_id, state, detail, checked_at)
                     SELECT t.id, 'skipped', NULL, ?2 FROM tracks t
                     WHERE t.id = ?1 AND t.canonical_uri IS NULL
                       AND EXISTS (SELECT 1 FROM identify_suggestions s WHERE s.track_id = t.id)
                     ON CONFLICT(track_id) DO UPDATE SET state = 'skipped', detail = NULL
                     WHERE identify_state.state <> 'skipped'",
                    params![t, now_unix()],
                )?;
            }
            tx.commit()?;
            Ok(n)
        })
    }

    /// Undo [`Self::skip_review`]: the songs wait for review again with their options (none left:
    /// "unknown"). How many came back.
    pub fn unskip_review(&self, tracks: &[i64]) -> Result<usize> {
        immediately(&mut self.lock(), |conn| {
            let tx = conn.savepoint()?;
            let mut n = 0;
            for t in tracks {
                n += tx.execute(
                    "UPDATE identify_state SET state = CASE
                       WHEN EXISTS (SELECT 1 FROM identify_suggestions s WHERE s.track_id = ?1) THEN 'suggested'
                       ELSE 'unknown' END
                     WHERE track_id = ?1 AND state = 'skipped'",
                    params![t],
                )?;
            }
            tx.commit()?;
            Ok(n)
        })
    }

    /// Songs the owner left as they are, with their file's path (to find them by album). Songs
    /// whose file is offline or gone are listed too, so "bring back all" reaches every one.
    pub fn skipped_track_paths(&self) -> Result<Vec<(i64, String)>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT t.id, COALESCE(MIN(f.path), '') FROM tracks t
             JOIN identify_state s ON s.track_id = t.id
             LEFT JOIN files f ON f.track_id = t.id
             WHERE t.canonical_uri IS NULL AND s.state = 'skipped'
             GROUP BY t.id ORDER BY t.id",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// Whether songs wait for review that older rules checked (the node checks them again once).
    pub fn has_outdated_suggestions(&self) -> Result<bool> {
        Ok(self.lock().query_row(
            &format!(
                "SELECT EXISTS (SELECT 1 FROM tracks t JOIN identify_state s ON s.track_id = t.id
                   WHERE t.canonical_uri IS NULL AND s.state = 'suggested' AND COALESCE(s.rules, 0) < ?1 AND {TRACK_PRESENT})"
            ),
            params![crate::identify::RULES_VERSION],
            |r| r.get(0),
        )?)
    }

    /// The songs [`Self::has_outdated_suggestions`] finds: waiting for review, checked by older
    /// rules, with a file present. The start's automatic run checks only these.
    pub fn outdated_suggestion_ids(&self) -> Result<std::collections::HashSet<i64>> {
        let conn = self.lock();
        let mut st = conn.prepare(&format!(
            "SELECT t.id FROM tracks t JOIN identify_state s ON s.track_id = t.id
             WHERE t.canonical_uri IS NULL AND s.state = 'suggested' AND COALESCE(s.rules, 0) < ?1 AND {TRACK_PRESENT}"
        ))?;
        let ids = st
            .query_map(params![crate::identify::RULES_VERSION], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(ids)
    }

    /// One suggestion by id, whatever its song's state.
    pub fn suggestion_by_id(&self, id: i64) -> Result<Option<SuggestionRow>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT id, track_id, recording_mbid, release_mbid, title, artist, release_title, score, source
                 FROM identify_suggestions WHERE id = ?1",
                params![id],
                |r| {
                    Ok(SuggestionRow {
                        id: r.get(0)?,
                        track_id: r.get(1)?,
                        recording_mbid: r.get(2)?,
                        release_mbid: r.get(3)?,
                        title: r.get(4)?,
                        artist: r.get(5)?,
                        release_title: r.get(6)?,
                        score: r.get::<_, f64>(7)? as f32,
                        source: r.get(8)?,
                    })
                },
            )
            .optional()?)
    }

    /// How long identification reuses a MusicBrainz answer (30 days).
    pub const IDENTIFY_MB_TTL_SECS: i64 = 30 * 24 * 3600;
    /// The most MusicBrainz answers identification keeps (oldest go first).
    pub const IDENTIFY_MB_MAX: i64 = 50_000;

    /// A MusicBrainz answer kept no earlier than `since`: `Some(None)` is a kept "nothing there".
    pub fn identify_mb_cached(&self, key: &str, since: i64) -> Result<Option<Option<String>>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT body FROM identify_mb_cache WHERE key = ?1 AND fetched_at >= ?2",
                params![key, since],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn save_identify_mb(&self, key: &str, body: Option<&str>) -> Result<()> {
        self.lock().execute(
            "INSERT INTO identify_mb_cache(key, body, fetched_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET body = excluded.body, fetched_at = excluded.fetched_at",
            params![key, body, now_unix()],
        )?;
        Ok(())
    }

    /// Keep identification's caches bounded: MusicBrainz answers older than their time or beyond
    /// the `max` newest, and content keys of files the library no longer has. Rows dropped.
    pub fn prune_identify_cache(&self, now: i64, max: i64) -> Result<usize> {
        let conn = self.lock();
        let mut n = conn.execute(
            "DELETE FROM identify_mb_cache WHERE fetched_at < ?1",
            params![now - Self::IDENTIFY_MB_TTL_SECS],
        )?;
        n += conn.execute(
            "DELETE FROM identify_mb_cache WHERE key IN (SELECT key FROM identify_mb_cache ORDER BY fetched_at DESC LIMIT -1 OFFSET ?1)",
            params![max.max(0)],
        )?;
        n += conn.execute(
            "DELETE FROM identify_file_keys WHERE path NOT IN (SELECT path FROM files)",
            [],
        )?;
        Ok(n)
    }

    /// A file's content key kept while its size and time are these.
    pub fn cached_file_key(&self, path: &Path, size: u64, mtime: i64) -> Result<Option<String>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT file_key FROM identify_file_keys WHERE path = ?1 AND size = ?2 AND mtime = ?3",
                params![path.to_string_lossy(), size as i64, mtime],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn save_file_key(&self, path: &Path, size: u64, mtime: i64, key: &str) -> Result<()> {
        self.lock().execute(
            "INSERT INTO identify_file_keys(path, size, mtime, file_key) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET size = excluded.size, mtime = excluded.mtime, file_key = excluded.file_key",
            params![path.to_string_lossy(), size as i64, mtime, key],
        )?;
        Ok(())
    }

    /// Remember what a song looked like when it was checked.
    pub fn set_identify_input(&self, track: i64, input_key: &str) -> Result<()> {
        self.lock().execute(
            "UPDATE identify_state SET input_key = ?2 WHERE track_id = ?1",
            params![track, input_key],
        )?;
        Ok(())
    }

    /// Songs a plain run compares with what they look like now (no identity): "not found" songs
    /// (unknown or error) with what they looked like when checked (those checked before this was
    /// kept have none and are not compared), and songs waiting for review, with or without it
    /// (one an earlier version of identification left waiting is checked again once: owner,
    /// 2026-11-04, most of them settle now).
    pub fn identify_inputs(&self) -> Result<std::collections::HashMap<i64, Option<String>>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT s.track_id, s.input_key FROM identify_state s JOIN tracks t ON t.id = s.track_id
             WHERE t.canonical_uri IS NULL AND ((s.state IN ('unknown','error') AND s.input_key IS NOT NULL) OR s.state = 'suggested')",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// Store what a run found for a song in one step, against what holds now: a song with an
    /// identity (the owner's choice made meanwhile, its tags, another run) is left as it is
    /// (`Kept`); a recording the owner rejected for it is not stored (`Rejected`, the song
    /// "unknown"); suggestions leave out rejected recordings, and none left makes the song
    /// "unknown" (`Empty`), never "suggested" with nothing to review. Identity, state, release and
    /// what the song looked like are written together or not at all.
    pub fn settle(&self, track: i64, answer: &SettleAnswer) -> Result<Settled> {
        self.settle_checked(track, answer, None)
    }

    /// [`Self::settle`], for an answer the run found for the song as it was when it read it
    /// (`read`, from [`Self::input_generations`]): when a rescan replaced the file or changed the
    /// tags since, the answer is stale and nothing is stored (`Changed`).
    pub fn settle_checked(&self, track: i64, answer: &SettleAnswer, read: Option<&str>) -> Result<Settled> {
        immediately(&mut self.lock(), |conn| {
            if let Some(read) = read {
                if identify_input_by_track(conn, track)?.as_deref() != Some(read) {
                    return Ok(Settled::Changed);
                }
            }
            let tx = conn.savepoint()?;
            let identified: Option<bool> = tx
                .query_row(
                    "SELECT canonical_uri IS NOT NULL FROM tracks WHERE id = ?1",
                    params![track],
                    |r| r.get(0),
                )
                .optional()?;
            // A song gone meanwhile, or with an identity: nothing to do (the savepoint rolls back)
            if identified != Some(false) {
                return Ok(Settled::Kept);
            }
            let (now, open_before): (Option<String>, i64) = tx.query_row(
                "SELECT (SELECT state FROM identify_state WHERE track_id = ?1),
                        (SELECT COUNT(*) FROM identify_suggestions WHERE track_id = ?1)",
                params![track],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            // The owner left it as it is ("Leave as is"): no run changes that. "Which song is
            // this?" still stores what it finds, for the song's page; the song stays left
            if now.as_deref() == Some("skipped") {
                if let SettleAnswer::Suggest {
                    candidates,
                    replace: false,
                    ..
                } = answer
                {
                    insert_suggestions(&tx, track, candidates)?;
                    tx.commit()?;
                    return Ok(Settled::Stored);
                }
                return Ok(Settled::Kept);
            }
            let rules = crate::identify::RULES_VERSION;
            // `rules` None: the song's rules version stays (options added, not checked again)
            let put_state_as = |state: &str,
                                detail: Option<&str>,
                                release: Option<String>,
                                input: Option<&str>,
                                rules: Option<u32>| {
                tx.execute(
                "INSERT INTO identify_state(track_id, state, detail, checked_at, release_mbid, input_key, rules) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(track_id) DO UPDATE SET state = excluded.state, detail = excluded.detail, checked_at = excluded.checked_at,
                   release_mbid = excluded.release_mbid, input_key = COALESCE(excluded.input_key, identify_state.input_key),
                   rules = COALESCE(excluded.rules, identify_state.rules)",
                params![track, state, detail, now_unix(), release, input, rules],
            )
            };
            let put_state =
                |state: &str, detail: Option<&str>, release: Option<String>, input: Option<&str>| {
                    put_state_as(state, detail, release, input, Some(rules))
                };
            let done = match answer {
                SettleAnswer::Identified {
                    recording,
                    confidence,
                    acoustid,
                    detail,
                    release,
                    input_key,
                } => {
                    let rec = match recording {
                        MusicUri::Recording { mbid } => mbid.to_string(),
                        other => other.to_url(),
                    };
                    if rejected(&tx, track, &rec)? && open_before > 0 {
                        // Checked again: its open options still wait for the owner
                        put_state("suggested", None, None, input_key.as_deref())?;
                        Settled::Kept
                    } else if rejected(&tx, track, &rec)? {
                        put_state("unknown", Some("rejected by you"), None, input_key.as_deref())?;
                        Settled::Rejected
                    } else {
                        tx.execute(
                        "UPDATE tracks SET canonical_uri = ?2, canonical_confidence_source = ?3, canonical_confidence_score = ?4,
                         acoustid = COALESCE(?5, acoustid) WHERE id = ?1 AND canonical_uri IS NULL",
                        params![
                            track,
                            recording.to_url(),
                            confidence_source(confidence),
                            confidence.score() as f64,
                            acoustid
                        ],
                    )?;
                        put_state(
                            "identified",
                            Some(detail),
                            release.map(|r| r.to_string()),
                            input_key.as_deref(),
                        )?;
                        // Options an earlier run left for it are settled with it
                        tx.execute(
                            "DELETE FROM identify_suggestions WHERE track_id = ?1",
                            params![track],
                        )?;
                        Settled::Stored
                    }
                }
                SettleAnswer::Suggest {
                    candidates,
                    input_key,
                    replace,
                } => {
                    let recheck = *replace && open_before > 0;
                    if recheck {
                        // Checked again: options older rules offered and these don't go; one that
                        // stays keeps its id, so a review page showing it still acts on it
                        let keep: Vec<String> = candidates.iter().map(|c| c.recording.to_string()).collect();
                        let marks = vec!["?"; keep.len()].join(",");
                        let mut args: Vec<&dyn rusqlite::ToSql> = vec![&track];
                        args.extend(keep.iter().map(|k| k as &dyn rusqlite::ToSql));
                        tx.execute(
                            &format!("DELETE FROM identify_suggestions WHERE track_id = ?1 AND recording_mbid NOT IN ({marks})"),
                            args.as_slice(),
                        )?;
                    }
                    insert_suggestions(&tx, track, candidates)?;
                    let open: i64 = tx.query_row(
                        "SELECT COUNT(*) FROM identify_suggestions WHERE track_id = ?1",
                        params![track],
                        |r| r.get(0),
                    )?;
                    // Options added to open ones ("Which song is this?") aren't a check under
                    // these rules: the song keeps its version, so a run still checks it again
                    let stamp = (*replace || open_before == 0).then_some(rules);
                    // Not a check under these rules: not its inputs either, so a run checks it again
                    let input = stamp.and(input_key.as_deref());
                    if open > 0 {
                        put_state_as("suggested", None, None, input, stamp)?;
                        Settled::Stored
                    } else {
                        let why = (!candidates.is_empty()).then_some("suggestions rejected");
                        put_state("unknown", why, None, input_key.as_deref())?;
                        Settled::Empty
                    }
                }
                SettleAnswer::State {
                    state,
                    detail: _,
                    input_key,
                } if open_before > 0 => {
                    // Checked again and nothing new: an error leaves it to a later run (nothing
                    // is written); "nothing found" keeps the options older rules offered, for the
                    // owner, and marks it checked by these rules
                    if state == "error" {
                        return Ok(Settled::Failed);
                    }
                    put_state("suggested", None, None, input_key.as_deref())?;
                    tx.commit()?;
                    return Ok(Settled::Kept);
                }
                SettleAnswer::State {
                    state,
                    detail,
                    input_key,
                } => {
                    put_state(state, detail.as_deref(), None, input_key.as_deref())?;
                    Settled::Stored
                }
            };
            tx.commit()?;
            Ok(done)
        })
    }

    /// What identification goes by for every song (its file's size and time and its tags, see
    /// [`Self::settle_checked`]), by track, read when a run starts.
    pub fn input_generations(&self) -> Result<std::collections::HashMap<i64, String>> {
        let ids: Vec<(i64, String)> = {
            let conn = self.lock();
            let mut stmt = conn.prepare("SELECT track_id, path FROM files WHERE missing = 0")?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<std::result::Result<_, _>>()?;
            rows
        };
        let conn = self.lock();
        let mut out = std::collections::HashMap::new();
        for (id, path) in ids {
            if let Some(g) = identify_input_of(&conn, id, &path)? {
                out.insert(id, g);
            }
        }
        Ok(out)
    }

    /// Whether suggestion `id` still is what the caller saw: there, and for `track` and
    /// `recording` when given (an id from a stale page names nothing else: ids are never reused).
    pub fn check_suggestion(
        &self,
        id: i64,
        track: Option<i64>,
        recording: Option<&str>,
    ) -> Result<SuggestionCheck> {
        check_suggestion_on(&self.lock(), id, track, recording)
    }

    /// "Not this", when the suggestion still is what the caller saw ([`Self::check_suggestion`]).
    pub fn reject_suggestion_checked(
        &self,
        id: i64,
        track: Option<i64>,
        recording: Option<&str>,
    ) -> Result<SuggestionCheck> {
        immediately(&mut self.lock(), |conn| {
            let check = check_suggestion_on(conn, id, track, recording)?;
            if check == SuggestionCheck::Holds {
                reject_on(conn, id)?;
            }
            Ok(check)
        })
    }

    /// Whether a song has a canonical identity (tags, a run, the owner).
    pub fn has_identity(&self, track: i64) -> Result<bool> {
        Ok(self
            .lock()
            .query_row(
                "SELECT canonical_uri IS NOT NULL FROM tracks WHERE id = ?1",
                params![track],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(false))
    }

    /// Whether the owner said "Not this" to `recording` for this song.
    pub fn is_rejected(&self, track: i64, recording: &str) -> Result<bool> {
        rejected(&self.lock(), track, recording)
    }

    /// Drop one suggestion; when it was the last for its track, the track is
    /// "unknown" until the next retry.
    pub fn reject_suggestion(&self, id: i64) -> Result<bool> {
        immediately(&mut self.lock(), |conn| reject_on(conn, id))
    }

    /// How identified the library is.
    pub fn identity_counts(&self) -> Result<IdentityCounts> {
        let conn = self.lock();
        let q = |sql: &str| -> rusqlite::Result<u64> {
            conn.query_row(sql, [], |r| r.get::<_, i64>(0)).map(|n| n as u64)
        };
        Ok(IdentityCounts {
            total: q("SELECT COUNT(*) FROM tracks")?,
            identified: q("SELECT COUNT(*) FROM tracks WHERE canonical_uri IS NOT NULL")?,
            suggested: q(
                "SELECT COUNT(DISTINCT s.track_id) FROM identify_suggestions s JOIN tracks t ON t.id = s.track_id WHERE t.canonical_uri IS NULL AND NOT EXISTS (SELECT 1 FROM identify_state k WHERE k.track_id = t.id AND k.state = 'skipped')",
            )?,
            skipped: q(
                "SELECT COUNT(*) FROM tracks t JOIN identify_state s ON s.track_id = t.id WHERE t.canonical_uri IS NULL AND s.state = 'skipped'",
            )?,
            outdated: conn.query_row(
                &format!(
                    "SELECT COUNT(*) FROM tracks t JOIN identify_state s ON s.track_id = t.id
                     WHERE t.canonical_uri IS NULL AND s.state = 'suggested' AND COALESCE(s.rules, 0) < ?1 AND {TRACK_PRESENT}"
                ),
                params![crate::identify::RULES_VERSION],
                |r| r.get::<_, i64>(0),
            )? as u64,
            unknown: q(
                "SELECT COUNT(*) FROM tracks t JOIN identify_state s ON s.track_id = t.id WHERE t.canonical_uri IS NULL AND s.state IN ('unknown','error')",
            )?,
            unchecked: q(
                "SELECT COUNT(*) FROM tracks t WHERE t.canonical_uri IS NULL AND NOT EXISTS (SELECT 1 FROM identify_state s WHERE s.track_id = t.id)",
            )?,
            by: IdentifiedBy {
                tags: q("SELECT COUNT(*) FROM tracks WHERE canonical_uri IS NOT NULL AND canonical_confidence_source IN ('tagged_mbid','tagged_isrc')")?,
                album: q("SELECT COUNT(*) FROM tracks WHERE canonical_uri IS NOT NULL AND canonical_confidence_source = 'album_fit'")?,
                fingerprint: q("SELECT COUNT(*) FROM tracks WHERE canonical_uri IS NOT NULL AND canonical_confidence_source IN ('fingerprint','fingerprint+release')")?,
                you: q("SELECT COUNT(*) FROM tracks WHERE canonical_uri IS NOT NULL AND canonical_confidence_source = 'confirmed_by_user'")?,
                title_artist: q("SELECT COUNT(*) FROM tracks WHERE canonical_uri IS NOT NULL AND canonical_confidence_source = 'tag_match'")?,
                auto: q("SELECT COUNT(*) FROM tracks WHERE canonical_uri IS NOT NULL AND canonical_confidence_source = 'auto'")?,
            },
        })
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.lock().execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // ---- automatic settling and corrections ("Wrong match?") ----

    /// The release each song identified by a run, its tags' IDs or the owner is on (not an
    /// automatic pick: those are never evidence for another), by track: the folder evidence of
    /// automatic settling.
    pub fn identified_releases(&self) -> Result<std::collections::HashMap<i64, String>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT s.track_id, s.release_mbid FROM identify_state s JOIN tracks t ON t.id = s.track_id
             WHERE t.canonical_uri IS NOT NULL AND s.release_mbid IS NOT NULL
               AND COALESCE(t.canonical_confidence_source, '') <> 'auto'",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// Songs the owner rejected any recording for.
    pub fn tracks_with_rejections(&self) -> Result<std::collections::HashSet<i64>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT DISTINCT track_id FROM identify_rejections")?;
        let rows = stmt
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// An automatic pick for a song waiting for review (`Confidence::Auto`), stored only when it
    /// still holds: the song without an identity and waiting (not left as it is), `recording` one
    /// of its open options and never rejected, and its file and tags what the caller read
    /// (`read`, from [`Self::input_generations`]). Its options stay (the "Wrong match?" view lists
    /// them); its IDs are not written into the file. Whether it was stored.
    pub fn settle_auto(
        &self,
        track: i64,
        recording: &Mbid,
        release: Option<&Mbid>,
        score: f32,
        detail: &str,
        read: Option<&str>,
    ) -> Result<bool> {
        immediately(&mut self.lock(), |conn| {
            // Its file there, and what it was when the caller read it (a song whose file is gone,
            // or that changed since, is not settled from what was read)
            let Some(read) = read else {
                return Ok(false);
            };
            if identify_input_by_track(conn, track)?.as_deref() != Some(read) {
                return Ok(false);
            }
            let rec = recording.to_string();
            let holds: bool = conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM tracks t JOIN identify_state s ON s.track_id = t.id
                   WHERE t.id = ?1 AND t.canonical_uri IS NULL AND s.state = 'suggested'
                     AND EXISTS (SELECT 1 FROM files f WHERE f.track_id = t.id AND f.missing = 0)
                     AND EXISTS (SELECT 1 FROM identify_suggestions o WHERE o.track_id = t.id AND o.recording_mbid = ?2))",
                params![track, rec],
                |r| r.get(0),
            )?;
            // Any recording the owner rejected for the song leaves it to them
            let turned_down: bool = conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM identify_rejections WHERE track_id = ?1)",
                params![track],
                |r| r.get(0),
            )?;
            if !holds || turned_down {
                return Ok(false);
            }
            let tx = conn.savepoint()?;
            let confidence = Confidence::Auto { score };
            tx.execute(
                "UPDATE tracks SET canonical_uri = ?2, canonical_confidence_source = 'auto', canonical_confidence_score = ?3
                 WHERE id = ?1 AND canonical_uri IS NULL",
                params![
                    track,
                    MusicUri::Recording { mbid: *recording }.to_url(),
                    confidence.score() as f64
                ],
            )?;
            tx.execute(
                "UPDATE identify_state SET state = 'identified', detail = ?2, checked_at = ?3, release_mbid = ?4, rules = ?5
                 WHERE track_id = ?1",
                params![
                    track,
                    detail,
                    now_unix(),
                    release.map(|r| r.to_string()),
                    crate::identify::RULES_VERSION
                ],
            )?;
            tx.commit()?;
            Ok(true)
        })
    }

    /// The owner corrects a song's match ("Wrong match?"): `choice` another recording (and its
    /// release when the owner picked one), the current one (an automatic pick becomes theirs), or
    /// `None`, "Not matched". Done only while the song's recording is still `expect` (what the
    /// page showed; `None`: no identity). The recording replaced or taken away is rejected for
    /// the song, so no later run, retry or automatic settling brings it back; the chosen one is
    /// the owner's (`confirmed_by_user`), an earlier rejection of it lifted. "Not matched" leaves
    /// the song without an identity and out of later runs, as "Leave as is" does. A song whose
    /// file's tags name a recording is changed only with `tags_follow` (the caller rewrites the
    /// file's IDs): else the next scan would read the old one back.
    pub fn rematch(
        &self,
        track: i64,
        expect: Option<&str>,
        choice: Option<(&Mbid, Option<&Mbid>)>,
        tags_follow: bool,
    ) -> Result<Rematch> {
        immediately(&mut self.lock(), |conn| {
            let row: Option<(Option<String>, Option<String>, Option<String>)> = conn
                .query_row(
                    "SELECT t.canonical_uri, t.canonical_confidence_source,
                       COALESCE(t.recording_mbid, (SELECT 'written' FROM identify_state s WHERE s.track_id = t.id AND s.detail = 'written into the file'))
                     FROM tracks t WHERE t.id = ?1",
                    params![track],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((uri, source, tagged)) = row else {
                return Ok(Rematch::Gone);
            };
            let current = uri.as_deref().map(|u| match MusicUri::parse(u) {
                Ok(MusicUri::Recording { mbid }) => mbid.to_string(),
                _ => u.to_string(),
            });
            if current.as_deref() != expect {
                return Ok(Rematch::Changed);
            }
            let from_tags =
                tagged.is_some() || matches!(source.as_deref(), Some("tagged_mbid" | "tagged_isrc"));
            if from_tags && !tags_follow {
                return Ok(Rematch::FromTags);
            }
            let tx = conn.savepoint()?;
            let now = now_unix();
            let chosen = choice.map(|(r, _)| r.to_string());
            if let Some(old) = current.as_deref().filter(|o| Some(*o) != chosen.as_deref()) {
                tx.execute(
                    "INSERT OR IGNORE INTO identify_rejections(track_id, recording_mbid, rejected_at) VALUES (?1, ?2, ?3)",
                    params![track, old, now],
                )?;
                tx.execute(
                    "DELETE FROM identify_suggestions WHERE track_id = ?1 AND recording_mbid = ?2",
                    params![track, old],
                )?;
            }
            match choice {
                Some((rec, release)) => {
                    let same = current.as_deref() == Some(rec.to_string().as_str());
                    tx.execute(
                        "DELETE FROM identify_rejections WHERE track_id = ?1 AND recording_mbid = ?2",
                        params![track, rec.to_string()],
                    )?;
                    tx.execute(
                        "UPDATE tracks SET canonical_uri = ?2, canonical_confidence_source = 'confirmed_by_user', canonical_confidence_score = ?3
                         WHERE id = ?1",
                        params![
                            track,
                            MusicUri::Recording { mbid: *rec }.to_url(),
                            Confidence::ConfirmedByUser.score() as f64
                        ],
                    )?;
                    tx.execute(
                        "DELETE FROM identify_suggestions WHERE track_id = ?1",
                        params![track],
                    )?;
                    // The same recording keeps its release unless the owner named one
                    tx.execute(
                        "INSERT INTO identify_state(track_id, state, detail, checked_at, release_mbid, rules) VALUES (?1, 'identified', 'confirmed_by_user', ?2, ?3, ?4)
                         ON CONFLICT(track_id) DO UPDATE SET state = excluded.state, detail = excluded.detail, checked_at = excluded.checked_at,
                           release_mbid = CASE WHEN ?5 THEN COALESCE(excluded.release_mbid, identify_state.release_mbid) ELSE excluded.release_mbid END,
                           rules = excluded.rules",
                        params![
                            track,
                            now,
                            release.map(|r| r.to_string()),
                            crate::identify::RULES_VERSION,
                            same
                        ],
                    )?;
                }
                None => {
                    tx.execute(
                        "UPDATE tracks SET canonical_uri = NULL, canonical_confidence_source = NULL, canonical_confidence_score = NULL
                         WHERE id = ?1",
                        params![track],
                    )?;
                    tx.execute(
                        "INSERT INTO identify_state(track_id, state, detail, checked_at, release_mbid, rules) VALUES (?1, 'skipped', 'not matched by you', ?2, NULL, ?3)
                         ON CONFLICT(track_id) DO UPDATE SET state = excluded.state, detail = excluded.detail, checked_at = excluded.checked_at,
                           release_mbid = NULL, rules = excluded.rules",
                        params![track, now, crate::identify::RULES_VERSION],
                    )?;
                }
            }
            tx.commit()?;
            Ok(Rematch::Done { old: current })
        })
    }

    /// A song's match as a correction sees it: its recording (`None`: no match), and whether its
    /// file's tags name a recording (read by the scan, or written there by "Write IDs").
    pub fn match_now(&self, track: i64) -> Result<Option<(Option<String>, bool)>> {
        let row: Option<(Option<String>, Option<String>, bool)> = self
            .lock()
            .query_row(
                "SELECT t.canonical_uri, t.canonical_confidence_source,
                   t.recording_mbid IS NOT NULL OR EXISTS (SELECT 1 FROM identify_state s WHERE s.track_id = t.id AND s.detail = 'written into the file')
                 FROM tracks t WHERE t.id = ?1",
                params![track],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(row.map(|(uri, source, tagged)| {
            let current = uri.as_deref().map(|u| match MusicUri::parse(u) {
                Ok(MusicUri::Recording { mbid }) => mbid.to_string(),
                _ => u.to_string(),
            });
            let from_tags = tagged || matches!(source.as_deref(), Some("tagged_mbid" | "tagged_isrc"));
            (current, from_tags)
        }))
    }

    /// After the node rewrote a song's ID tags itself (a correction): the file as the scan will
    /// find it now (its size and time, so the rewrite is no change of the song's), and the
    /// recording its tags name now. What the owner chose (a match, or none) stands.
    pub fn note_rewritten(
        &self,
        track: i64,
        path: &Path,
        size: u64,
        mtime: i64,
        recording: Option<&Mbid>,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE files SET size_bytes = ?3, mtime_unix = ?4 WHERE track_id = ?1 AND path = ?2",
            params![track, path.to_string_lossy(), size as i64, mtime],
        )?;
        conn.execute(
            "UPDATE tracks SET recording_mbid = ?2 WHERE id = ?1",
            params![track, recording.map(|m| m.to_string())],
        )?;
        Ok(())
    }

    /// The release identification stored for a song, when one was singled out.
    pub fn identify_release(&self, track: i64) -> Result<Option<String>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT release_mbid FROM identify_state WHERE track_id = ?1",
                params![track],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    /// "Not found" songs (nothing found, or the check failed) last checked before `before`, with
    /// their file present, oldest first, at most `limit`: the weekly retry's songs.
    pub fn stale_not_found(&self, before: i64, limit: usize) -> Result<Vec<i64>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT t.id FROM tracks t JOIN identify_state s ON s.track_id = t.id
             WHERE t.canonical_uri IS NULL AND s.state IN ('unknown','error') AND s.checked_at < ?1 AND {TRACK_PRESENT}
             ORDER BY s.checked_at, t.id LIMIT ?2"
        ))?;
        let rows = stmt
            .query_map(params![before, limit as i64], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// AcoustID's answer kept for the file at `path` (by its last content key), without reading
    /// the file: the automatic settling's look at lengths and release kinds.
    pub fn lookup_for_path(&self, path: &Path) -> Result<Option<String>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT f.lookup_json FROM identify_file_keys k JOIN fingerprints f ON f.file_key = k.file_key
                 WHERE k.path = ?1",
                params![path.to_string_lossy()],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    /// When AcoustID's answer kept for a file key was asked (`None`: none kept, or before this was
    /// recorded).
    pub fn lookup_time(&self, key: &str) -> Result<Option<i64>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT looked_up_at FROM fingerprints WHERE file_key = ?1 AND lookup_json IS NOT NULL",
                params![key],
                |r| r.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten())
    }

    // ---- content hashes (files on the homeserver) ----

    /// Library files present on disk whose size is one of `sizes`: (path, size, mtime).
    pub fn files_of_sizes(&self, sizes: &std::collections::HashSet<u64>) -> Result<Vec<(PathBuf, u64, i64)>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT path, size_bytes, mtime_unix FROM files WHERE missing = 0")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                PathBuf::from(r.get::<_, String>(0)?),
                r.get::<_, i64>(1)? as u64,
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let row = row?;
            if sizes.contains(&row.1) {
                out.push(row);
            }
        }
        Ok(out)
    }

    /// The kept hash of a file, when its size and mtime are unchanged.
    pub fn cached_file_hash(&self, path: &Path, size: u64, mtime: i64) -> Result<Option<String>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT blake3 FROM file_hashes WHERE path = ?1 AND size_bytes = ?2 AND mtime_unix = ?3",
                params![path.to_string_lossy(), size as i64, mtime],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn store_file_hash(&self, path: &Path, size: u64, mtime: i64, hash: &str) -> Result<()> {
        self.lock().execute(
            "INSERT INTO file_hashes(path, size_bytes, mtime_unix, blake3) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET size_bytes = excluded.size_bytes,
             mtime_unix = excluded.mtime_unix, blake3 = excluded.blake3",
            params![path.to_string_lossy(), size as i64, mtime, hash],
        )?;
        Ok(())
    }

    pub fn delete_setting(&self, key: &str) -> Result<bool> {
        Ok(self
            .lock()
            .execute("DELETE FROM settings WHERE key = ?1", [key])?
            > 0)
    }

    /// All settings whose key starts with `prefix`.
    pub fn settings_with_prefix(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        let conn = self.lock();
        let mut st = conn.prepare("SELECT key, value FROM settings WHERE key LIKE ?1 || '%' ORDER BY key")?;
        let rows = st.query_map([prefix], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    // ----- album art -----------------------------------------------------

    pub fn album_art(&self, album_id: i64) -> Result<Option<AlbumArt>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT album_id, file, mime, source FROM album_art WHERE album_id = ?1",
                [album_id],
                |r| {
                    Ok(AlbumArt {
                        album_id: r.get(0)?,
                        file: PathBuf::from(r.get::<_, String>(1)?),
                        mime: r.get(2)?,
                        source: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn set_album_art(&self, art: &AlbumArt) -> Result<()> {
        self.lock().execute(
            "INSERT INTO album_art(album_id, file, mime, source, set_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(album_id) DO UPDATE SET file = excluded.file, mime = excluded.mime,
             source = excluded.source, set_at = excluded.set_at",
            params![
                art.album_id,
                art.file.to_string_lossy().into_owned(),
                art.mime,
                art.source,
                now_unix()
            ],
        )?;
        Ok(())
    }

    // ----- stats and folders -------------------------------------------

    pub fn stats(&self) -> Result<LibraryStats> {
        let conn = self.lock();
        let count =
            |sql: &str| -> Result<u64> { Ok(conn.query_row(sql, [], |r| r.get::<_, i64>(0))? as u64) };
        Ok(LibraryStats {
            // Songs and albums the lists show: copies of a song once, gone files left out
            tracks: {
                let reps = song_reps(&song_rows(&conn, "ORDER BY t.id", [])?);
                reps.values().collect::<std::collections::HashSet<_>>().len() as u64
            },
            albums: count(&format!("SELECT COUNT(*) FROM albums WHERE {ALBUM_PRESENT}"))?,
            artists: count(&format!(
                "SELECT COUNT(DISTINCT ta.artist_id) FROM track_artists ta JOIN tracks t ON t.id = ta.track_id WHERE {TRACK_PRESENT}"
            ))?,
            files: count("SELECT COUNT(*) FROM files WHERE missing = 0")?,
            missing_files: count("SELECT COUNT(*) FROM files WHERE missing = 1")?,
            total_duration_ms: count("SELECT COALESCE(SUM(duration_ms), 0) FROM files WHERE missing = 0")?,
        })
    }

    pub fn add_folder(&self, path: &Path) -> Result<()> {
        self.lock().execute(
            "INSERT OR IGNORE INTO folders(path, enabled) VALUES (?1, 1)",
            params![path.to_string_lossy()],
        )?;
        Ok(())
    }

    /// Forget a library folder: its files, the tracks that lived only there,
    /// and the albums and artists left empty. Returns `(files, tracks)` removed.
    pub fn remove_folder(&self, root: &Path) -> Result<(u64, u64)> {
        let mut conn = self.lock();
        let tx = conn.savepoint()?;
        let root_s = root.to_string_lossy().into_owned();
        let prefix = format!("{}/", root_s.trim_end_matches('/'));
        let escaped = prefix
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let files = tx.execute(
            "DELETE FROM files WHERE path = ?1 OR path LIKE ?2 ESCAPE '\\'",
            params![root_s, format!("{escaped}%")],
        )? as u64;
        tx.execute(
            "DELETE FROM plays WHERE track_id IN (SELECT id FROM tracks WHERE id NOT IN (SELECT track_id FROM files))",
            [],
        )?;
        tx.execute(
            "DELETE FROM track_artists WHERE track_id IN (SELECT id FROM tracks WHERE id NOT IN (SELECT track_id FROM files))",
            [],
        )?;
        tx.execute(
            "DELETE FROM tracks_fts WHERE rowid IN (SELECT id FROM tracks WHERE id NOT IN (SELECT track_id FROM files))",
            [],
        )?;
        let tracks = tx.execute(
            "DELETE FROM tracks WHERE id NOT IN (SELECT track_id FROM files)",
            [],
        )? as u64;
        tx.execute(
            "DELETE FROM album_art WHERE album_id NOT IN (SELECT DISTINCT album_id FROM tracks WHERE album_id IS NOT NULL)",
            [],
        )?;
        tx.execute(
            "DELETE FROM albums WHERE id NOT IN (SELECT DISTINCT album_id FROM tracks WHERE album_id IS NOT NULL)",
            [],
        )?;
        tx.execute(
            "DELETE FROM artists WHERE id NOT IN (SELECT DISTINCT artist_id FROM track_artists)",
            [],
        )?;
        tx.execute(
            "DELETE FROM genres WHERE id NOT IN (SELECT DISTINCT genre_id FROM track_genres)",
            [],
        )?;
        tx.execute("DELETE FROM folders WHERE path = ?1", params![root_s])?;
        tx.commit()?;
        Ok((files, tracks))
    }

    pub fn list_folders(&self) -> Result<Vec<(PathBuf, bool)>> {
        let conn = self.lock();
        let mut stmt = conn.prepare("SELECT path, enabled FROM folders ORDER BY path")?;
        let v = stmt
            .query_map([], |r| {
                Ok((PathBuf::from(r.get::<_, String>(0)?), r.get::<_, i64>(1)? != 0))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(v)
    }

    /// Tracks and bytes of the present files under `root` (a library folder), for the storage
    /// status line.
    pub fn folder_totals(&self, root: &Path) -> Result<(u64, u64)> {
        let mut prefix = root.to_string_lossy().to_string();
        if !prefix.ends_with(std::path::MAIN_SEPARATOR) {
            prefix.push(std::path::MAIN_SEPARATOR);
        }
        let conn = self.lock();
        Ok(conn.query_row(
            "SELECT COUNT(DISTINCT track_id), COALESCE(SUM(size_bytes), 0) FROM files WHERE missing = 0 AND path LIKE ?1 ESCAPE '\\'",
            params![format!("{}%", like_escape(&prefix))],
            |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
        )?)
    }

    // ----- scanner support ----------------------------------------------

    pub fn file_by_path(&self, path: &Path) -> Result<Option<FileRow>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT id, track_id, path, size_bytes, mtime_unix, missing FROM files WHERE path = ?1",
                params![path.to_string_lossy()],
                |r| {
                    Ok(FileRow {
                        id: r.get(0)?,
                        track_id: TrackId(r.get(1)?),
                        path: PathBuf::from(r.get::<_, String>(2)?),
                        size_bytes: r.get::<_, i64>(3)? as u64,
                        mtime_unix: r.get(4)?,
                        missing: r.get::<_, i64>(5)? != 0,
                    })
                },
            )
            .optional()?)
    }

    pub fn begin_scan(&self, root: &Path) -> Result<i64> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO scans(root, started_at) VALUES (?1, ?2)",
            params![root.to_string_lossy(), now_unix()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn finish_scan(&self, scan_id: i64, counts: &ScanCounts) -> Result<()> {
        self.lock().execute(
            "UPDATE scans SET finished_at=?2, files_seen=?3, files_added=?4, files_updated=?5, files_removed=?6 WHERE id=?1",
            params![scan_id, now_unix(), counts.files_seen as i64, counts.files_added as i64, counts.files_updated as i64, counts.files_removed as i64],
        )?;
        Ok(())
    }

    pub fn mark_file_seen(&self, path: &Path, scan_id: i64) -> Result<()> {
        self.lock().execute(
            "UPDATE files SET last_seen_scan_id = ?2, missing = 0 WHERE path = ?1",
            params![path.to_string_lossy(), scan_id],
        )?;
        Ok(())
    }

    /// Flag every file under `root` that this scan did not see. Returns how
    /// many changed state.
    pub fn mark_missing_under(&self, root: &Path, scan_id: i64) -> Result<u64> {
        let mut prefix = root.to_string_lossy().to_string();
        if !prefix.ends_with(std::path::MAIN_SEPARATOR) {
            prefix.push(std::path::MAIN_SEPARATOR);
        }
        let n = self.lock().execute(
            "UPDATE files SET missing = 1 WHERE missing = 0 AND path LIKE ?1 ESCAPE '\\' AND (last_seen_scan_id IS NULL OR last_seen_scan_id <> ?2)",
            params![format!("{}%", like_escape(&prefix)), scan_id],
        )?;
        Ok(n as u64)
    }
}

/// The owner's edit of a song's tags (`None`: the file's value stays).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TrackOverride {
    #[serde(default)]
    pub title: Option<String>,
    /// One credit, as typed ("A, B" names two artists, as in tags).
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub album_artist: Option<String>,
}

impl TrackOverride {
    /// Blank fields count as not given.
    pub fn cleaned(self) -> Self {
        let c = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        Self {
            title: c(self.title),
            artist: c(self.artist),
            album: c(self.album),
            album_artist: c(self.album_artist),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.artist.is_none() && self.album.is_none() && self.album_artist.is_none()
    }

    /// Put the edit on a track read from its file.
    pub fn apply(&self, t: &mut Track) {
        if let Some(v) = &self.title {
            t.title = v.clone();
        }
        if let Some(v) = &self.artist {
            t.artists = crate::credits::split_all(std::slice::from_ref(v))
                .into_iter()
                .map(|name| pm_model::Artist {
                    id: None,
                    name,
                    mbid: None,
                })
                .collect();
        }
        // An album artist alone, on a song without an album, makes no album
        if self.album.is_some() || (self.album_artist.is_some() && t.album.is_some()) {
            let a = t.album.get_or_insert_with(|| pm_model::Album {
                id: None,
                title: String::new(),
                album_artist: None,
                release_mbid: None,
                release_group_mbid: None,
                year: None,
                upc: None,
                total_discs: None,
            });
            a.id = None;
            if let Some(v) = &self.album {
                a.title = v.clone();
            }
            if let Some(v) = &self.album_artist {
                a.album_artist = Some(v.clone());
            }
        }
    }
}

/// What [`Library::edit_album`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlbumEdit {
    Done,
    NotFound,
    /// Another album already has that name (and album artist).
    NameTaken,
}

/// Put back what a file says (kept under its edit) on a track.
fn restore_originals(
    t: &mut Track,
    title: &str,
    artists: &str,
    album: Option<String>,
    album_artist: Option<String>,
) {
    t.title = title.to_string();
    t.artists = original_artists(artists);
    match (album, t.album.as_mut()) {
        (Some(title), Some(a)) => {
            a.id = None;
            a.title = title;
            a.album_artist = album_artist;
        }
        (None, _) => t.album = None,
        (Some(_), None) => {}
    }
}

/// One artist as a file says it, kept under an edit: the name and its MusicBrainz id (artists are
/// told apart by both, so an undo without the id would make a second artist).
#[derive(serde::Serialize, serde::Deserialize)]
struct OriginalArtist {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mbid: Option<String>,
}

/// The artists kept under an edit (JSON): objects with their ids, or plain names.
fn original_artists(json: &str) -> Vec<pm_model::Artist> {
    if let Ok(list) = serde_json::from_str::<Vec<OriginalArtist>>(json) {
        return list
            .into_iter()
            .map(|a| pm_model::Artist {
                id: None,
                name: a.name,
                mbid: a.mbid.and_then(|m| Mbid::parse(&m).ok()),
            })
            .collect();
    }
    serde_json::from_str::<Vec<String>>(json)
        .unwrap_or_default()
        .into_iter()
        .map(|name| pm_model::Artist {
            id: None,
            name,
            mbid: None,
        })
        .collect()
}

/// What a track's file says, as kept under an edit: title, artists (JSON, with their ids),
/// album, album artist.
fn originals_of(t: &Track) -> (String, String, Option<String>, Option<String>) {
    let artists: Vec<OriginalArtist> = t
        .artists
        .iter()
        .map(|a| OriginalArtist {
            name: a.name.clone(),
            mbid: a.mbid.map(|m| m.to_string()),
        })
        .collect();
    (
        t.title.clone(),
        serde_json::to_string(&artists).unwrap_or_else(|_| "[]".into()),
        t.album.as_ref().map(|a| a.title.clone()),
        t.album.as_ref().and_then(|a| a.album_artist.clone()),
    )
}

/// Run `f` holding the write lock from its start (`BEGIN IMMEDIATE`, waiting for another writer
/// through the busy handler), unless a batch already holds it. A transaction that reads first
/// and writes later asks for the lock only at its first write, and in WAL mode that write fails
/// at once ("database is locked") when another connection holds it or committed meanwhile:
/// [`Library::begin_batch`] says the same of scans.
fn immediately<T>(conn: &mut Connection, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
    let began = conn.is_autocommit();
    if began {
        conn.execute_batch("BEGIN IMMEDIATE")?;
    }
    let r = f(conn);
    if began {
        let end = match &r {
            Ok(_) => conn.execute_batch("COMMIT"),
            Err(_) => conn.execute_batch("ROLLBACK"),
        };
        if let Err(e) = end {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(e.into());
        }
    }
    r
}

fn check_suggestion_on(
    conn: &Connection,
    id: i64,
    track: Option<i64>,
    recording: Option<&str>,
) -> Result<SuggestionCheck> {
    let row: Option<(i64, String)> = conn
        .query_row(
            "SELECT track_id, recording_mbid FROM identify_suggestions WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(match row {
        None => SuggestionCheck::Gone,
        Some((t, r)) if track.is_some_and(|x| x != t) || recording.is_some_and(|x| x != r) => {
            SuggestionCheck::Changed
        }
        Some(_) => SuggestionCheck::Holds,
    })
}

/// "Not this" for suggestion `id`, in the caller's transaction: remembered (no later run, retry
/// or cached answer brings it back), the option dropped. A song left with none is "unknown"; an
/// automatic pick of that very recording (settled meanwhile from its kept options) is taken back,
/// the song waiting again with what is left.
fn reject_on(conn: &Connection, id: i64) -> Result<bool> {
    let row: Option<(i64, String)> = conn
        .query_row(
            "SELECT track_id, recording_mbid FROM identify_suggestions WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((track, recording)) = row else {
        return Ok(false);
    };
    conn.execute(
        "INSERT OR IGNORE INTO identify_rejections(track_id, recording_mbid, rejected_at) VALUES (?1, ?2, ?3)",
        params![track, recording, now_unix()],
    )?;
    conn.execute("DELETE FROM identify_suggestions WHERE id = ?1", params![id])?;
    let url = recording_uri(&recording).map(|u| u.to_url()).unwrap_or_default();
    let taken_back = conn.execute(
        "UPDATE tracks SET canonical_uri = NULL, canonical_confidence_source = NULL, canonical_confidence_score = NULL
         WHERE id = ?1 AND canonical_uri = ?2 AND canonical_confidence_source = 'auto'",
        params![track, url],
    )? > 0;
    let (left, open): (i64, bool) = conn.query_row(
        "SELECT (SELECT COUNT(*) FROM identify_suggestions WHERE track_id = ?1),
                COALESCE((SELECT canonical_uri IS NULL FROM tracks WHERE id = ?1), 0)",
        params![track],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    // (an identified song's kept options, an automatic pick's, leave its state as it is)
    if open && (left == 0 || taken_back) {
        let (state, detail) = if left == 0 {
            ("unknown", Some("suggestions rejected"))
        } else {
            ("suggested", None)
        };
        conn.execute(
            "INSERT INTO identify_state(track_id, state, detail, checked_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(track_id) DO UPDATE SET state = excluded.state, detail = excluded.detail, checked_at = excluded.checked_at,
               release_mbid = NULL",
            params![track, state, detail, now_unix()],
        )?;
    }
    Ok(true)
}

fn rejected(conn: &Connection, track: i64, recording: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM identify_rejections WHERE track_id = ?1 AND recording_mbid = ?2)",
        params![track, recording],
        |r| r.get(0),
    )?)
}

/// What identification goes by for a song's file: its size and time, and the tags it is matched
/// by (title, artists, album, album artist, the album's release id, disc, number, length).
fn identify_input_of(conn: &Connection, track: i64, path: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT f.size_bytes, f.mtime_unix, t.title, t.track_no, t.disc_no, t.duration_ms, a.title, a.album_artist, a.release_mbid,
               (SELECT group_concat(name, char(31)) FROM (SELECT ar.name AS name FROM track_artists ta JOIN artists ar ON ar.id = ta.artist_id
                  WHERE ta.track_id = t.id ORDER BY ta.position))
             FROM tracks t LEFT JOIN files f ON f.track_id = t.id AND f.path = ?2 LEFT JOIN albums a ON a.id = t.album_id
             WHERE t.id = ?1",
            params![track, path],
            |r| {
                Ok(format!(
                    "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
                    r.get::<_, Option<i64>>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, Option<String>>(9)?,
                ))
            },
        )
        .optional()?)
}

/// [`identify_input_of`] for a song's present file.
fn identify_input_by_track(conn: &Connection, track: i64) -> Result<Option<String>> {
    let path: Option<String> = conn
        .query_row(
            "SELECT path FROM files WHERE track_id = ?1 AND missing = 0 ORDER BY rowid LIMIT 1",
            params![track],
            |r| r.get(0),
        )
        .optional()?;
    match path {
        Some(p) => identify_input_of(conn, track, &p),
        None => Ok(None),
    }
}

fn recording_uri(mbid: &str) -> Result<MusicUri> {
    let mbid = Mbid::parse(mbid).map_err(|_| LibraryError::Corrupt {
        column: "identify_suggestions.recording_mbid",
        value: mbid.to_string(),
    })?;
    Ok(MusicUri::Recording { mbid })
}

fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// Turn free text into an FTS5 query: each word a prefix term, ANDed.
fn fts_query(input: &str) -> String {
    input
        .split_whitespace()
        .map(|w| w.replace('"', ""))
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn confidence_source(c: &Confidence) -> &'static str {
    match c {
        Confidence::TaggedMbid => "tagged_mbid",
        Confidence::TaggedIsrc => "tagged_isrc",
        Confidence::Fingerprint { .. } => "fingerprint",
        Confidence::FingerprintRelease { .. } => "fingerprint+release",
        Confidence::AlbumFit { .. } => "album_fit",
        Confidence::ConfirmedByUser => "confirmed_by_user",
        Confidence::TagMatch { .. } => "tag_match",
        Confidence::Auto { .. } => "auto",
        Confidence::Fuzzy { .. } => "fuzzy",
    }
}

fn confidence_from(source: &str, score: f64) -> Result<Confidence> {
    Ok(match source {
        "tagged_mbid" => Confidence::TaggedMbid,
        "tagged_isrc" => Confidence::TaggedIsrc,
        "fingerprint" => Confidence::Fingerprint { score: score as f32 },
        "fingerprint+release" => Confidence::FingerprintRelease { score: score as f32 },
        "album_fit" => Confidence::AlbumFit { score: score as f32 },
        "confirmed_by_user" => Confidence::ConfirmedByUser,
        "tag_match" => Confidence::TagMatch { score: score as f32 },
        "auto" => Confidence::Auto { score: score as f32 },
        "fuzzy" => Confidence::Fuzzy { score: score as f32 },
        other => {
            return Err(LibraryError::Corrupt {
                column: "canonical_confidence_source",
                value: other.to_string(),
            })
        }
    })
}

fn describe_format(album_id: i64, container: &str, kind: &str, rate: i64, bits: i64) -> AlbumFormat {
    let lossy = crate::tags::is_lossy_container(container);
    let dsd = kind == "dsd";
    let container_label = match container {
        "flac" => "FLAC",
        "alac" | "m4a" | "mp4" | "isomp4" => "ALAC",
        "wav" | "wave" | "riff" => "WAV",
        "aiff" | "aif" => "AIFF",
        "dsf" => "DSF",
        "dff" => "DFF",
        "mp3" => "MP3",
        "aac" => "AAC",
        "vorbis" | "ogg" => "Vorbis",
        "opus" => "Opus",
        other => other,
    };
    let label = if dsd {
        let mult = (rate as f64 / 44_100.0).round().max(1.0) as i64;
        format!("DSD{mult}")
    } else if lossy {
        container_label.to_string()
    } else {
        let khz = rate as f64 / 1000.0;
        let khz = if khz.fract() == 0.0 {
            format!("{khz:.0}")
        } else {
            format!("{khz:.1}")
        };
        format!("{bits}/{khz} {container_label}")
    };
    AlbumFormat {
        album_id,
        label,
        lossless: !lossy,
        hi_res: !lossy && !dsd && (rate > 48_000 || bits > 16),
        dsd,
    }
}

fn sample_kind_str(k: SampleKind) -> &'static str {
    match k {
        SampleKind::Pcm => "pcm",
        SampleKind::Dsd => "dsd",
    }
}

fn upsert_artist(tx: &Connection, a: &Artist) -> Result<i64> {
    let mbid = a.mbid.map(|m| m.to_string());
    tx.execute(
        "INSERT INTO artists(name, mbid, sort_name) VALUES (?1, ?2, ?3) ON CONFLICT(name, COALESCE(mbid, '')) DO UPDATE SET sort_name = COALESCE(excluded.sort_name, artists.sort_name)",
        params![a.name, mbid, Option::<String>::None],
    )?;
    Ok(tx.query_row(
        "SELECT id FROM artists WHERE name = ?1 AND COALESCE(mbid, '') = COALESCE(?2, '')",
        params![a.name, mbid],
        |r| r.get(0),
    )?)
}

fn upsert_album(tx: &Connection, a: &Album) -> Result<i64> {
    let release = a.release_mbid.map(|m| m.to_string());
    tx.execute(
        "INSERT INTO albums(title, album_artist, release_mbid, release_group_mbid, year, upc, total_discs) VALUES (?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(title, COALESCE(album_artist, ''), COALESCE(release_mbid, '')) DO UPDATE SET
         release_group_mbid = COALESCE(excluded.release_group_mbid, albums.release_group_mbid),
         year = COALESCE(excluded.year, albums.year), upc = COALESCE(excluded.upc, albums.upc),
         total_discs = COALESCE(excluded.total_discs, albums.total_discs)",
        params![a.title, a.album_artist, release, a.release_group_mbid.map(|m| m.to_string()), a.year, a.upc.as_ref().map(|u| u.to_string()), a.total_discs],
    )?;
    Ok(tx.query_row(
        "SELECT id FROM albums WHERE title = ?1 AND COALESCE(album_artist, '') = COALESCE(?2, '') AND COALESCE(release_mbid, '') = COALESCE(?3, '')",
        params![a.title, a.album_artist, release],
        |r| r.get(0),
    )?)
}

const TRACK_SELECT: &str = "SELECT t.id, t.title, t.album_id, t.track_no, t.disc_no, t.duration_ms, t.recording_mbid, t.isrc, t.acoustid,
    t.canonical_uri, t.canonical_confidence_source, t.canonical_confidence_score, t.replaygain_track_gain_db, t.replaygain_album_gain_db,
    fi.path, fi.size_bytes, fi.mtime_unix, fi.container, fi.sample_kind, fi.rate_hz, fi.bits, fi.channels, fi.duration_ms,
    al.id, al.title, al.album_artist, al.release_mbid, al.release_group_mbid, al.year, al.upc, al.total_discs
    FROM tracks t
    JOIN files fi ON fi.track_id = t.id AND NOT EXISTS (SELECT 1 FROM hidden_files h WHERE h.path = fi.path)
    LEFT JOIN albums al ON al.id = t.album_id";

/// A track (aliased `t`) has a present file.
const TRACK_PRESENT: &str = "EXISTS (SELECT 1 FROM files pf WHERE pf.track_id = t.id AND +pf.missing = 0 AND NOT EXISTS (SELECT 1 FROM hidden_files ph WHERE ph.path = pf.path))";

/// An album (the `albums` table unaliased) has a song with a present file.
const ALBUM_PRESENT: &str = "EXISTS (SELECT 1 FROM tracks pt JOIN files pf ON pf.track_id = pt.id AND +pf.missing = 0 AND NOT EXISTS (SELECT 1 FROM hidden_files ph WHERE ph.path = pf.path) WHERE pt.album_id = albums.id)";

/// [`ALBUM_PRESENT`] for an album aliased `al`.
const ALBUM_PRESENT_AL: &str = "EXISTS (SELECT 1 FROM tracks pt JOIN files pf ON pf.track_id = pt.id AND +pf.missing = 0 AND NOT EXISTS (SELECT 1 FROM hidden_files ph WHERE ph.path = pf.path) WHERE pt.album_id = al.id)";

/// A song row: the copy played by default and, when there are several, every copy.
#[derive(Debug, Clone, PartialEq)]
pub struct Song {
    pub track: Track,
    /// Every present copy, the default first. Empty when the song has one copy.
    pub versions: Vec<TrackVersion>,
}

/// One copy of a song, as a version chooser lists it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TrackVersion {
    pub track_id: i64,
    /// `library` (a file in a library folder) or `homeserver` (the copy kept on the homeserver).
    pub source: String,
    pub container: String,
    /// `24/96 FLAC`, `MP3`, as album formats are labelled.
    pub format: String,
    pub rate_hz: u32,
    pub bits: u8,
    pub size_bytes: u64,
    pub lossless: bool,
    /// The copy the song row plays.
    pub best: bool,
}

/// A copy kept on the homeserver rather than a file of the owner's own: a path in the
/// homeserver's private storage, or the node's mirror of it (`<16 hex of the hash>-<name>`).
pub fn is_homeserver_copy(path: &Path) -> bool {
    if path.to_string_lossy().contains("/priv/pubky.music/files/") {
        return true;
    }
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
        let b = n.as_bytes();
        b.len() > 17
            && b[16] == b'-'
            && b[..16]
                .iter()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
            && b[..16].iter().any(|c| (b'a'..=b'f').contains(c))
    })
}

fn songs_only(songs: Vec<Song>) -> Vec<Track> {
    songs.into_iter().map(|s| s.track).collect()
}

/// Which copy plays by default: a local file over a homeserver copy, then lossless, then the
/// highest rate × bits, then the larger file, then the oldest track. Smaller is better.
fn copy_rank(t: &Track) -> (bool, bool, std::cmp::Reverse<u64>, std::cmp::Reverse<u64>, i64) {
    let f = &t.file;
    (
        is_homeserver_copy(&f.path),
        crate::tags::is_lossy_container(&f.container),
        std::cmp::Reverse(f.format.rate_hz as u64 * f.format.bits as u64),
        std::cmp::Reverse(f.size_bytes),
        t.id.map(|i| i.0).unwrap_or(i64::MAX),
    )
}

fn version_of(t: &Track, best: bool) -> TrackVersion {
    let f = &t.file;
    let label = describe_format(
        0,
        &f.container,
        sample_kind_str(f.format.kind),
        f.format.rate_hz as i64,
        f.format.bits as i64,
    );
    TrackVersion {
        track_id: t.id.map(|i| i.0).unwrap_or_default(),
        source: if is_homeserver_copy(&f.path) {
            "homeserver"
        } else {
            "library"
        }
        .into(),
        container: f.container.clone(),
        format: label.label,
        rate_hz: f.format.rate_hz,
        bits: f.format.bits,
        size_bytes: f.size_bytes,
        lossless: label.lossless,
        best,
    }
}

/// What tells copies of a song apart, for one present track.
struct SongRow {
    id: i64,
    album: Option<i64>,
    disc: u32,
    no: Option<u32>,
    title: String,
    hash: Option<String>,
    duration_ms: Option<i64>,
}

/// [`SongRow`]s of present tracks; `tail` adds conditions (`AND …`) and the order.
fn song_rows<P: rusqlite::Params>(conn: &Connection, tail: &str, p: P) -> Result<Vec<SongRow>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT t.id, t.album_id, t.disc_no, t.track_no, t.title,
            (SELECT h.blake3 FROM files hf JOIN file_hashes h ON h.path = hf.path
                 AND h.size_bytes = hf.size_bytes AND h.mtime_unix = hf.mtime_unix
             WHERE hf.track_id = t.id AND +hf.missing = 0 AND NOT EXISTS (SELECT 1 FROM hidden_files ph WHERE ph.path = hf.path) LIMIT 1),
            t.duration_ms
         FROM tracks t WHERE {TRACK_PRESENT} {tail}"
    ))?;
    let v = stmt
        .query_map(p, |r| {
            Ok(SongRow {
                id: r.get(0)?,
                album: r.get(1)?,
                disc: r.get::<_, Option<u32>>(2)?.unwrap_or(1),
                no: r.get(3)?,
                title: r.get(4)?,
                hash: r.get(5)?,
                duration_ms: r.get(6)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(v)
}

/// Which song each track is (owner, 2026-11-04: "we should generally not show it twice"):
/// within one album, tracks with the same disc, number and title, or files with the same
/// content, are one song. Maps every track id to the first (by row order) track of its song.
fn song_reps(rows: &[SongRow]) -> std::collections::HashMap<i64, i64> {
    use std::collections::HashMap;
    fn root(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let mut parent: Vec<usize> = (0..rows.len()).collect();
    let mut by_key: HashMap<(i64, u32, u32, String), Vec<usize>> = HashMap::new();
    let mut by_hash: HashMap<(i64, &str), usize> = HashMap::new();
    for (i, r) in rows.iter().enumerate() {
        let Some(album) = r.album else { continue };
        let mut joined = Vec::with_capacity(2);
        if let Some(no) = r.no {
            let title = r.title.trim_matches(' ').to_ascii_lowercase();
            let same = by_key.entry((album, r.disc, no, title)).or_default();
            // The same length too: two discs without disc tags may each have a track 1 "Intro"
            joined.extend(
                same.iter()
                    .copied()
                    .filter(|&j| close_lengths(rows[j].duration_ms, r.duration_ms)),
            );
            same.push(i);
        }
        if let Some(h) = r.hash.as_deref() {
            joined.push(*by_hash.entry((album, h)).or_insert(i));
        }
        for j in joined {
            let (a, b) = (root(&mut parent, i), root(&mut parent, j));
            if a != b {
                parent[a.max(b)] = a.min(b);
            }
        }
    }
    (0..rows.len())
        .map(|i| (rows[i].id, rows[root(&mut parent, i)].id))
        .collect()
}

/// Whether two copies are the same length, within 3 seconds (a length unknown on either side
/// counts as the same).
fn close_lengths(a: Option<i64>, b: Option<i64>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => (a - b).abs() <= 3_000,
        _ => true,
    }
}

/// Albums and songs counted for a summary; copies of a song count once.
#[derive(Default)]
struct SongCount {
    albums: std::collections::HashSet<i64>,
    songs: std::collections::HashSet<i64>,
}

impl SongCount {
    fn add(&mut self, reps: &std::collections::HashMap<i64, i64>, track: i64, album: Option<i64>) {
        self.songs.insert(reps.get(&track).copied().unwrap_or(track));
        if let Some(a) = album {
            self.albums.insert(a);
        }
    }
}

/// `(track id, album id)` rows of a listing query, in its order.
fn candidates<P: rusqlite::Params>(conn: &Connection, sql: &str, p: P) -> Result<Vec<(i64, Option<i64>)>> {
    let mut stmt = conn.prepare_cached(sql)?;
    let v = stmt
        .query_map(p, |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(v)
}

/// Per album: which song each track is, and each song's tracks.
type AlbumSongs = (
    std::collections::HashMap<i64, i64>,
    std::collections::HashMap<i64, Vec<i64>>,
);

/// One row per song for a listing: walks the listing's tracks in order and keeps the first
/// `limit` songs (all when `None`), each with every present copy on its album, whether the
/// listing had that copy or not. A song plays its best copy ([`copy_rank`]).
fn songs_in_order(
    conn: &Connection,
    listed: Vec<(i64, Option<i64>)>,
    limit: Option<usize>,
) -> Result<Vec<Song>> {
    use std::collections::{HashMap, HashSet};
    // Per album: which song each track is, and each song's tracks
    let mut albums: HashMap<i64, AlbumSongs> = HashMap::new();
    let mut seen: HashSet<(Option<i64>, i64)> = HashSet::new();
    let mut picked: Vec<Vec<i64>> = Vec::new();
    for (id, album) in listed {
        if limit.is_some_and(|l| picked.len() >= l) {
            break;
        }
        let Some(album) = album else {
            if seen.insert((None, id)) {
                picked.push(vec![id]);
            }
            continue;
        };
        let (reps, members) = &*match albums.entry(album) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let rows = song_rows(conn, "AND t.album_id = ?1 ORDER BY t.id", params![album])?;
                let reps = song_reps(&rows);
                let mut members: HashMap<i64, Vec<i64>> = HashMap::new();
                for r in &rows {
                    members.entry(reps[&r.id]).or_default().push(r.id);
                }
                e.insert((reps, members))
            }
        };
        let rep = reps.get(&id).copied().unwrap_or(id);
        if seen.insert((Some(album), rep)) {
            picked.push(members.get(&rep).cloned().unwrap_or_else(|| vec![id]));
        }
    }
    let ids: Vec<i64> = picked.iter().flatten().copied().collect();
    let mut tracks: HashMap<i64, Track> = HashMap::new();
    for chunk in ids.chunks(500) {
        let list = format!(
            "[{}]",
            chunk.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
        );
        let mut stmt = conn.prepare_cached(&format!(
            "{} WHERE t.id IN (SELECT value FROM json_each(?1)) ORDER BY t.id",
            present_select()
        ))?;
        let raws: Vec<RawTrack> = stmt
            .query_map(params![list], row_to_track_raw)?
            .collect::<std::result::Result<_, _>>()?;
        for raw in raws {
            let t = finish_track(conn, raw)?;
            if let Some(id) = t.id {
                tracks.entry(id.0).or_insert(t);
            }
        }
    }
    let mut out = Vec::with_capacity(picked.len());
    for group in picked {
        let mut copies: Vec<Track> = group.iter().filter_map(|i| tracks.remove(i)).collect();
        if copies.is_empty() {
            continue;
        }
        copies.sort_by_key(copy_rank);
        let versions = if copies.len() > 1 {
            copies
                .iter()
                .enumerate()
                .map(|(k, t)| version_of(t, k == 0))
                .collect()
        } else {
            Vec::new()
        };
        let track = copies.swap_remove(0);
        out.push(Song { track, versions });
    }
    Ok(out)
}

/// [`TRACK_SELECT`] with present files only: what lists show (owner, 2026-11-04: a song whose
/// file is gone is left out). Reads by id keep [`TRACK_SELECT`].
fn present_select() -> String {
    TRACK_SELECT.replacen(
        "JOIN files fi ON fi.track_id = t.id AND",
        "JOIN files fi ON fi.track_id = t.id AND +fi.missing = 0 AND",
        1,
    )
}

/// Number of columns `TRACK_SELECT` yields (track 14, file 9, album 8).
const TRACK_COLUMNS: usize = 31;

/// `TRACK_SELECT` with extra columns appended to the select list.
fn track_select_with(extra: &str) -> String {
    TRACK_SELECT.replacen("\n    FROM tracks t", &format!(", {extra}\n    FROM tracks t"), 1)
}

struct RawTrack {
    track: Track,
    strings: RawStrings,
}

fn opt_mbid(v: Option<String>, column: &'static str) -> Result<Option<Mbid>> {
    v.map(|s| Mbid::parse(&s).map_err(|_| LibraryError::Corrupt { column, value: s }))
        .transpose()
}

fn row_to_track_raw(r: &Row<'_>) -> rusqlite::Result<RawTrack> {
    // Column access is checked; semantic parsing happens in finish_track.
    let get_str = |i: usize| r.get::<_, Option<String>>(i);
    let canonical = match (get_str(9)?, get_str(10)?, r.get::<_, Option<f64>>(11)?) {
        (Some(uri), Some(src), Some(score)) => Some((uri, src, score)),
        _ => None,
    };
    let album = match r.get::<_, Option<i64>>(23)? {
        Some(id) => Some(Album {
            id: Some(id),
            title: r.get(24)?,
            album_artist: get_str(25)?,
            release_mbid: None,
            release_group_mbid: None,
            year: r.get(28)?,
            upc: None,
            total_discs: r.get(30)?,
        }),
        None => None,
    };
    let kind = match r.get::<_, String>(18)?.as_str() {
        "dsd" => SampleKind::Dsd,
        _ => SampleKind::Pcm,
    };
    let format = AudioFormat {
        kind,
        rate_hz: r.get(19)?,
        bits: r.get(20)?,
        channels: r.get(21)?,
    };
    let file = AudioFile {
        path: PathBuf::from(r.get::<_, String>(14)?),
        size_bytes: r.get::<_, i64>(15)? as u64,
        mtime_unix: r.get(16)?,
        container: r.get(17)?,
        format,
        duration_ms: r.get::<_, Option<i64>>(22)?.map(|d| d as u64),
    };
    let mut track = Track {
        id: Some(TrackId(r.get(0)?)),
        title: r.get(1)?,
        artists: Vec::new(),
        album,
        track_no: r.get(3)?,
        disc_no: r.get(4)?,
        duration_ms: r.get::<_, Option<i64>>(5)?.map(|d| d as u64),
        file,
        recording_mbid: None,
        isrc: None,
        acoustid: get_str(8)?,
        canonical: None,
        replaygain_track_gain_db: r.get(12)?,
        replaygain_album_gain_db: r.get(13)?,
        genres: Vec::new(),
    };
    let raw_strings = RawStrings {
        recording_mbid: get_str(6)?,
        isrc: get_str(7)?,
        canonical,
        release_mbid: get_str(26)?,
        release_group_mbid: get_str(27)?,
        upc: get_str(29)?,
    };
    track.id = Some(TrackId(r.get(0)?));
    Ok(RawTrack {
        track,
        strings: raw_strings,
    })
}

struct RawStrings {
    recording_mbid: Option<String>,
    isrc: Option<String>,
    canonical: Option<(String, String, f64)>,
    release_mbid: Option<String>,
    release_group_mbid: Option<String>,
    upc: Option<String>,
}

fn finish_track(conn: &Connection, raw: RawTrack) -> Result<Track> {
    let RawTrack { mut track, strings } = raw;
    track.recording_mbid = opt_mbid(strings.recording_mbid, "recording_mbid")?;
    track.isrc = strings
        .isrc
        .map(|s| {
            Isrc::parse(&s).map_err(|_| LibraryError::Corrupt {
                column: "isrc",
                value: s,
            })
        })
        .transpose()?;
    track.canonical = match strings.canonical {
        Some((uri, src, score)) => {
            let uri = MusicUri::parse(&uri).map_err(|_| LibraryError::Corrupt {
                column: "canonical_uri",
                value: uri,
            })?;
            Some((uri, confidence_from(&src, score)?))
        }
        None => None,
    };
    if let Some(album) = track.album.as_mut() {
        album.release_mbid = opt_mbid(strings.release_mbid, "release_mbid")?;
        album.release_group_mbid = opt_mbid(strings.release_group_mbid, "release_group_mbid")?;
        album.upc = strings
            .upc
            .map(|s| {
                Upc::parse(&s).map_err(|_| LibraryError::Corrupt {
                    column: "upc",
                    value: s,
                })
            })
            .transpose()?;
    }
    let id = track.id.expect("loaded track has id").0;
    let mut stmt = conn.prepare_cached(
        "SELECT a.id, a.name, a.mbid FROM track_artists ta JOIN artists a ON a.id = ta.artist_id WHERE ta.track_id = ?1 ORDER BY ta.position",
    )?;
    let artists = stmt
        .query_map(params![id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (aid, name, mbid) in artists {
        track.artists.push(Artist {
            id: Some(aid),
            name,
            mbid: opt_mbid(mbid, "artists.mbid")?,
        });
    }
    let mut stmt = conn.prepare_cached(
        "SELECT g.name FROM track_genres tg JOIN genres g ON g.id = tg.genre_id WHERE tg.track_id = ?1 ORDER BY g.name COLLATE NOCASE",
    )?;
    track.genres = stmt
        .query_map(params![id], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(track)
}

/// Fisher–Yates with a time-seeded xorshift; good enough for a radio shuffle.
fn shuffle<T>(v: &mut [T]) {
    let mut x = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9)
        | 1;
    for i in (1..v.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.swap(i, (x % (i as u64 + 1)) as usize);
    }
}

fn row_to_tag(r: &Row<'_>) -> rusqlite::Result<TagRow> {
    Ok(TagRow {
        id: r.get(0)?,
        uri: r.get(1)?,
        label: r.get(2)?,
        created_at: r.get(3)?,
        synced_at: r.get(4)?,
        deleted: r.get::<_, i64>(5)? != 0,
    })
}

fn row_to_bookmark(r: &Row<'_>) -> rusqlite::Result<BookmarkRow> {
    Ok(BookmarkRow {
        id: r.get(0)?,
        uri: r.get(1)?,
        created_at: r.get(2)?,
        synced_at: r.get(3)?,
        deleted: r.get::<_, i64>(4)? != 0,
    })
}

fn row_to_album(r: &Row<'_>) -> rusqlite::Result<Result<Album>> {
    let release: Option<String> = r.get(3)?;
    let rg: Option<String> = r.get(4)?;
    let upc: Option<String> = r.get(6)?;
    let base = Album {
        id: Some(r.get(0)?),
        title: r.get(1)?,
        album_artist: r.get(2)?,
        release_mbid: None,
        release_group_mbid: None,
        year: r.get(5)?,
        upc: None,
        total_discs: r.get(7)?,
    };
    Ok((|| {
        let mut a = base;
        a.release_mbid = opt_mbid(release, "release_mbid")?;
        a.release_group_mbid = opt_mbid(rg, "release_group_mbid")?;
        a.upc = upc
            .map(|s| {
                Upc::parse(&s).map_err(|_| LibraryError::Corrupt {
                    column: "upc",
                    value: s,
                })
            })
            .transpose()?;
        Ok(a)
    })())
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    pub(crate) const MB: &str = "b1a9c0e9-d987-4042-ae91-78d6a3267d69";

    pub(crate) fn sample_track(path: &str, title: &str) -> Track {
        Track {
            id: None,
            title: title.into(),
            artists: vec![Artist {
                id: None,
                name: "Daft Punk".into(),
                mbid: Some(Mbid::parse(MB).unwrap()),
            }],
            album: Some(Album {
                id: None,
                title: "Random Access Memories".into(),
                album_artist: Some("Daft Punk".into()),
                release_mbid: Some(Mbid::parse(MB).unwrap()),
                release_group_mbid: None,
                year: Some(2013),
                upc: Some(Upc::parse("888837168618").unwrap()),
                total_discs: Some(1),
            }),
            track_no: Some(8),
            disc_no: Some(1),
            duration_ms: Some(369_000),
            file: AudioFile {
                path: PathBuf::from(path),
                size_bytes: 123,
                mtime_unix: 1_700_000_000,
                container: "flac".into(),
                format: AudioFormat::pcm(44_100, 16, 2),
                duration_ms: Some(369_000),
            },
            recording_mbid: Some(Mbid::parse(MB).unwrap()),
            isrc: Some(Isrc::parse("USQX91300108").unwrap()),
            acoustid: None,
            canonical: Some((
                MusicUri::Recording {
                    mbid: Mbid::parse(MB).unwrap(),
                },
                Confidence::TaggedMbid,
            )),
            replaygain_track_gain_db: Some(-7.35),
            replaygain_album_gain_db: None,
            genres: vec!["Disco".into(), "Electronic".into()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::{sample_track, MB};
    use super::*;

    #[test]
    fn a_settings_size_is_its_bytes_without_reading_it() {
        let lib = Library::open_in_memory().unwrap();
        assert_eq!(lib.setting_bytes("none").unwrap(), None);
        lib.set_setting("ascii", "0123456789").unwrap();
        assert_eq!(lib.setting_bytes("ascii").unwrap(), Some(10));
        // Bytes, not characters: "é" is two bytes in UTF-8
        lib.set_setting("accent", "é").unwrap();
        assert_eq!(lib.setting_bytes("accent").unwrap(), Some(2));
        lib.set_setting("empty", "").unwrap();
        assert_eq!(lib.setting_bytes("empty").unwrap(), Some(0));
    }

    #[test]
    fn tracks_sharing_genres_rank_by_shared_genres() {
        let lib = Library::open_in_memory().unwrap();
        let add = |path: &str, genres: &[&str]| {
            let mut t = sample_track(path, path);
            t.recording_mbid = None;
            t.isrc = None;
            t.canonical = None;
            t.genres = genres.iter().map(|g| g.to_string()).collect();
            lib.upsert_track(&t).unwrap().0
        };
        let seed = add("/m/seed.flac", &["Disco", "Funk"]);
        let both = add("/m/both.flac", &["Funk", "Disco"]);
        let one = add("/m/one.flac", &["Disco"]);
        let none = add("/m/none.flac", &["Jazz"]);
        let got = lib.tracks_sharing_genres(&[seed], &[], 10).unwrap();
        assert_eq!(
            got,
            vec![both, one],
            "most shared first, never the seed, nothing unrelated"
        );
        assert!(!got.contains(&none));
        assert!(lib.tracks_sharing_genres(&[], &[], 10).unwrap().is_empty());
        // everything in `exclude` stays out, seeds or not
        assert_eq!(
            lib.tracks_sharing_genres(&[seed], &[seed, both], 10).unwrap(),
            vec![one]
        );
        assert_eq!(lib.tracks_sharing_genres(&[seed], &[], 1).unwrap(), vec![both]);
    }

    #[test]
    fn format_labels() {
        let f = describe_format(1, "flac", "pcm", 96_000, 24);
        assert_eq!(
            (f.label.as_str(), f.hi_res, f.lossless, f.dsd),
            ("24/96 FLAC", true, true, false)
        );
        let f = describe_format(1, "flac", "pcm", 44_100, 16);
        assert_eq!((f.label.as_str(), f.hi_res), ("16/44.1 FLAC", false));
        let f = describe_format(1, "dsf", "dsd", 5_644_800, 1);
        assert_eq!((f.label.as_str(), f.dsd, f.hi_res), ("DSD128", true, false));
        let f = describe_format(1, "mp3", "pcm", 44_100, 16);
        assert_eq!((f.label.as_str(), f.lossless), ("MP3", false));
    }

    #[test]
    fn titles_from_hashed_file_names_lose_the_hash_on_open() {
        let lib = Library::open_in_memory().unwrap();
        let old = lib
            .upsert_track(&sample_track(
                "/hs/f80da35cdf86-14_wasted_bits.flac",
                "F80da35cdf86-14 wasted bits",
            ))
            .unwrap();
        let raw = lib
            .upsert_track(&sample_track(
                "/hs/0d4c7c38c2e2e811-Donna.flac",
                "0d4c7c38c2e2e811-Donna",
            ))
            .unwrap();
        let tagged = lib
            .upsert_track(&sample_track("/hs/f80da35cdf86-Other.flac", "A tagged title"))
            .unwrap();
        let plain = lib
            .upsert_track(&sample_track(
                "/music/123456789012-Song.flac",
                "123456789012-Song",
            ))
            .unwrap();
        assert_eq!(lib.tidy_file_name_titles().unwrap(), 2);
        let title = |id| lib.get_track(id).unwrap().unwrap().title;
        assert_eq!(title(old), "Wasted bits");
        assert_eq!(title(raw), "Donna");
        assert_eq!(title(tagged), "A tagged title", "a tag's title is kept");
        assert_eq!(title(plain), "123456789012-Song", "digits only are not a hash");
        assert_eq!(lib.tidy_file_name_titles().unwrap(), 0, "idempotent");
    }

    #[test]
    fn songs_without_an_artist_are_unknown_artist() {
        let lib = Library::open_in_memory().unwrap();
        assert!(lib.unknown_artist().unwrap().is_none());
        lib.upsert_track(&sample_track("/music/a.flac", "With")).unwrap();
        let mut bare = sample_track("/music/b.flac", "Without");
        bare.artists.clear();
        lib.upsert_track(&bare).unwrap();
        let u = lib.unknown_artist().unwrap().expect("one song without an artist");
        assert_eq!((u.id, u.tracks, u.albums), (UNKNOWN_ARTIST_ID, 1, 1));
        let t = lib.tracks_without_artist().unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].title, "Without");
        // The shared list stays the real artists only
        assert!(lib
            .list_artists()
            .unwrap()
            .iter()
            .all(|a| a.id != UNKNOWN_ARTIST_ID));
    }

    #[test]
    fn focus_filters_albums_by_their_files() {
        let lib = Library::open_in_memory().unwrap();
        let mut cd = sample_track("/music/cd.flac", "CD");
        cd.album.as_mut().unwrap().title = "CD album".into();
        let mut hires = sample_track("/music/hires.flac", "Hi");
        hires.album.as_mut().unwrap().title = "Hi-res album".into();
        hires.file.format.rate_hz = 96_000;
        hires.file.format.bits = 24;
        let mut lossy = sample_track("/music/l.mp3", "Lo");
        lossy.album.as_mut().unwrap().title = "MP3 album".into();
        lossy.file.container = "mp3".into();
        for t in [&cd, &hires, &lossy] {
            lib.upsert_track(t).unwrap();
        }
        let titles = |focus| -> Vec<String> {
            lib.list_albums_focused(focus, None, 100)
                .unwrap()
                .into_iter()
                .map(|a| a.title)
                .collect()
        };
        assert_eq!(titles(None).len(), 3);
        assert_eq!(titles(Some(Focus::HiRes)), ["Hi-res album"]);
        assert_eq!(titles(Some(Focus::Cd)), ["CD album"]);
        assert_eq!(titles(Some(Focus::Lossy)), ["MP3 album"]);
        assert_eq!(titles(Some(Focus::Lossless)).len(), 2);
        assert!(titles(Some(Focus::Dsd)).is_empty());
        let formats = lib.album_formats().unwrap();
        assert_eq!(formats.len(), 3);
        assert!(formats.iter().any(|f| f.label == "24/96 FLAC" && f.hi_res));
    }

    #[test]
    fn backup_is_a_usable_copy() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(&dir.path().join("library.db")).unwrap();
        let id = lib
            .upsert_track(&sample_track("/music/a.flac", "Get Lucky"))
            .unwrap();
        lib.set_lyrics(id, Some(("la la", "tag"))).unwrap();
        let copy = dir.path().join("backup.db");
        lib.backup_to(&copy).unwrap();
        assert!(lib.backup_to(&copy).is_err(), "never overwrites");
        let restored = Library::open(&copy).unwrap();
        assert_eq!(restored.stats().unwrap().tracks, 1);
        assert_eq!(restored.get_track(id).unwrap().unwrap().title, "Get Lucky");
        assert_eq!(restored.lyrics(id).unwrap().unwrap().0, "la la");
    }

    /// The search index is keyed by track id: a rescan replaces the entry (one hit, new title),
    /// and a batch commits everything written inside it.
    #[test]
    fn search_index_follows_rescans_inside_a_batch() {
        let lib = Library::open_in_memory().unwrap();
        lib.begin_batch().unwrap();
        let id = lib
            .upsert_track(&sample_track("/music/a.flac", "Get Lucky"))
            .unwrap();
        let again = lib
            .upsert_track(&sample_track("/music/a.flac", "Get Luckier"))
            .unwrap();
        lib.commit_batch().unwrap();
        assert_eq!(id, again);
        assert!(lib.search_tracks("lucky", 10).unwrap().is_empty());
        let hits = lib.search_tracks("luckier", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, Some(id));
        lib.commit_batch().unwrap(); // nothing open: nothing to do
    }

    /// A library scanned before credits were split: "Sefa, Dr. Peacock" becomes two artists,
    /// "Dr. Peacock" is shared with the track that named him alone, and nothing is left behind.
    #[test]
    fn old_credits_split_into_artists() {
        let lib = Library::open_in_memory().unwrap();
        let mut a = sample_track("/m/a.flac", "Flowing River");
        a.artists = vec![Artist {
            id: None,
            name: "Sefa, Dr. Peacock".into(),
            mbid: None,
        }];
        let mut b = sample_track("/m/b.flac", "Solo");
        b.artists = vec![Artist {
            id: None,
            name: "Dr. Peacock".into(),
            mbid: None,
        }];
        let ta = lib.upsert_track(&a).unwrap();
        lib.upsert_track(&b).unwrap();
        assert_eq!(lib.split_artist_credits().unwrap(), 1);
        let names: Vec<String> = lib.list_artists().unwrap().into_iter().map(|a| a.name).collect();
        assert_eq!(names, vec!["Dr. Peacock", "Sefa"]);
        let t = lib.get_track(ta).unwrap().unwrap();
        assert_eq!(
            t.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            vec!["Sefa", "Dr. Peacock"]
        );
        assert_eq!(lib.split_artist_credits().unwrap(), 0, "once is enough");
        assert_eq!(lib.search_tracks("peacock", 10).unwrap().len(), 2);
    }

    #[test]
    fn a_library_from_a_newer_version_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        let known = {
            let lib = Library::open(&path).unwrap();
            let v = lib.schema_version().unwrap();
            lib.lock()
                .execute(
                    "INSERT INTO schema_version(version, applied_at) VALUES (?1, 0)",
                    params![v + 1],
                )
                .unwrap();
            v
        };
        match Library::open(&path) {
            Err(LibraryError::NewerSchema { found, known: k }) => {
                assert_eq!((found, k), (known + 1, known));
            }
            other => panic!("expected NewerSchema, got {:?}", other.map(|_| ())),
        }
    }

    #[test]
    fn migrate_and_roundtrip() {
        let lib = Library::open_in_memory().unwrap();
        assert_eq!(lib.schema_version().unwrap(), 19);
        let t = sample_track("/music/a.flac", "Get Lucky");
        let id = lib.upsert_track(&t).unwrap();
        let back = lib.get_track(id).unwrap().unwrap();
        assert_eq!(back.title, "Get Lucky");
        assert_eq!(back.artists.len(), 1);
        assert_eq!(back.artists[0].mbid, Some(Mbid::parse(MB).unwrap()));
        assert_eq!(back.album.as_ref().unwrap().title, "Random Access Memories");
        assert_eq!(back.album.as_ref().unwrap().upc, t.album.as_ref().unwrap().upc);
        assert_eq!(back.recording_mbid, t.recording_mbid);
        assert_eq!(back.isrc, t.isrc);
        assert_eq!(back.canonical, t.canonical);
        assert_eq!(back.file.format, AudioFormat::pcm(44_100, 16, 2));
        assert_eq!(back.replaygain_track_gain_db, Some(-7.35));

        // Upsert by path keeps the id and updates fields.
        let mut t2 = t.clone();
        t2.title = "Get Lucky (Radio Edit)".into();
        let id2 = lib.upsert_track(&t2).unwrap();
        assert_eq!(id, id2);
        assert_eq!(
            lib.get_track(id).unwrap().unwrap().title,
            "Get Lucky (Radio Edit)"
        );
        let stats = lib.stats().unwrap();
        assert_eq!(stats.tracks, 1);
        assert_eq!(stats.albums, 1);
        assert_eq!(stats.artists, 1);
        assert_eq!(stats.total_duration_ms, 369_000);
    }

    #[test]
    fn search_list_and_lookup() {
        let lib = Library::open_in_memory().unwrap();
        lib.upsert_track(&sample_track("/m/1.flac", "Get Lucky")).unwrap();
        lib.upsert_track(&sample_track("/m/2.flac", "Instant Crush"))
            .unwrap();
        let mut plain = sample_track("/m/3.wav", "Untitled");
        plain.artists.clear();
        plain.album = None;
        plain.canonical = None;
        plain.recording_mbid = None;
        plain.isrc = None;
        lib.upsert_track(&plain).unwrap();

        let hits = lib.search_tracks("lucky", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Get Lucky");
        let hits = lib.search_tracks("daft", 10).unwrap();
        assert_eq!(hits.len(), 2);
        let hits = lib.search_tracks("random inst", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(lib.search_tracks("", 10).unwrap().is_empty());

        let page1 = lib.list_tracks(None, 2).unwrap();
        assert_eq!(page1.len(), 2);
        let page2 = lib.list_tracks(page1.last().and_then(|t| t.id), 2).unwrap();
        assert_eq!(page2.len(), 1);
        assert_eq!(page2[0].title, "Untitled");
        assert!(page2[0].album.is_none());
        assert!(page2[0].artists.is_empty());

        let uri = MusicUri::Recording {
            mbid: Mbid::parse(MB).unwrap(),
        };
        assert_eq!(lib.tracks_by_canonical(&uri).unwrap().len(), 2);
        assert_eq!(lib.tracks_by_mbid(Mbid::parse(MB).unwrap()).unwrap().len(), 2);
        let albums = lib.list_albums(None, 10).unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(lib.album_tracks(albums[0].id.unwrap()).unwrap().len(), 2);
        assert!(lib.remove_track(TrackId(1)).unwrap());
        assert_eq!(lib.stats().unwrap().tracks, 2);
        assert!(lib.search_tracks("lucky", 10).unwrap().is_empty());
    }

    #[test]
    fn folders_and_missing() {
        let lib = Library::open_in_memory().unwrap();
        lib.add_folder(Path::new("/music")).unwrap();
        lib.add_folder(Path::new("/music")).unwrap();
        assert_eq!(lib.list_folders().unwrap().len(), 1);
        lib.upsert_track(&sample_track("/music/a.flac", "A")).unwrap();
        lib.upsert_track(&sample_track("/music/b.flac", "B")).unwrap();
        lib.upsert_track(&sample_track("/other/c.flac", "C")).unwrap();
        let scan = lib.begin_scan(Path::new("/music")).unwrap();
        lib.mark_file_seen(Path::new("/music/a.flac"), scan).unwrap();
        assert_eq!(lib.mark_missing_under(Path::new("/music"), scan).unwrap(), 1);
        let s = lib.stats().unwrap();
        assert_eq!(s.missing_files, 1);
        assert_eq!(s.files, 2);
        let f = lib.file_by_path(Path::new("/music/b.flac")).unwrap().unwrap();
        assert!(f.missing);
        lib.finish_scan(
            scan,
            &ScanCounts {
                files_seen: 1,
                files_removed: 1,
                ..Default::default()
            },
        )
        .unwrap();
        // A re-upsert clears the flag.
        lib.upsert_track(&sample_track("/music/b.flac", "B")).unwrap();
        assert!(
            !lib.file_by_path(Path::new("/music/b.flac"))
                .unwrap()
                .unwrap()
                .missing
        );
    }

    #[test]
    fn fts_query_building() {
        assert_eq!(fts_query("daft punk"), "\"daft\"* \"punk\"*");
        assert_eq!(fts_query("  \"quoted\" "), "\"quoted\"*");
        assert_eq!(fts_query(""), "");
    }
}

#[cfg(test)]
mod play_history_tests {
    use super::tests_support::sample_track;
    use super::*;

    #[test]
    fn plays_made_offline_keep_their_time_and_count_once() {
        let lib = Library::open_in_memory().unwrap();
        let a = lib.upsert_track(&sample_track("/m/a.flac", "A")).unwrap();
        let then = now_unix() - 3600;
        assert!(lib.record_play_at(a, Some("app"), then).unwrap());
        // The same report again (the answer got lost): not a second play
        assert!(!lib.record_play_at(a, Some("app"), then).unwrap());
        // From a device whose clock runs a few minutes ahead: its time, so a resend still matches
        let ahead = now_unix() + 300;
        assert!(lib.record_play_at(a, Some("app"), ahead).unwrap());
        assert!(!lib.record_play_at(a, Some("app"), ahead).unwrap());
        // Far in the future: now
        assert!(lib.record_play_at(a, None, now_unix() + 86_400).unwrap());
        let h = lib.history(10).unwrap();
        assert_eq!(h.len(), 3);
        assert_eq!(h[0].played_at, ahead);
        assert!(h[1].played_at <= now_unix());
        assert_eq!(h[2].played_at, then);
        assert_eq!(lib.play_count(a).unwrap(), 3);
    }

    #[test]
    fn plays_feed_history_and_counts() {
        let lib = Library::open_in_memory().unwrap();
        let a = lib.upsert_track(&sample_track("/m/a.flac", "A")).unwrap();
        let b = lib.upsert_track(&sample_track("/m/b.flac", "B")).unwrap();
        assert!(lib.history(10).unwrap().is_empty());
        assert!(lib.recently_played_albums(10).unwrap().is_empty());
        lib.record_play(a, Some("zone-1")).unwrap();
        lib.record_play(b, None).unwrap();
        lib.record_play(a, Some("zone-1")).unwrap();
        let h = lib.history(10).unwrap();
        assert_eq!(h.len(), 3);
        assert_eq!(h[0].track.id, Some(a));
        assert_eq!(h[0].zone.as_deref(), Some("zone-1"));
        assert_eq!(h[1].track.id, Some(b));
        assert_eq!(h[1].zone, None);
        assert_eq!(lib.history(1).unwrap().len(), 1);
        let most = lib.most_played_tracks(10).unwrap();
        assert_eq!(most[0].track.id, Some(a));
        assert_eq!(most[0].plays, 2);
        assert_eq!(most[1].plays, 1);
        assert_eq!(lib.play_count(a).unwrap(), 2);
        let albums = lib.recently_played_albums(10).unwrap();
        assert_eq!(albums.len(), 1, "both tracks share one album");
        assert_eq!(albums[0].title, "Random Access Memories");
        assert_eq!(lib.most_played_albums(10).unwrap().len(), 1);
        // Removing the track removes its plays.
        assert!(lib.remove_track(a).unwrap());
        assert_eq!(lib.history(10).unwrap().len(), 1);
    }

    #[test]
    fn bookmarks_lifecycle() {
        let lib = Library::open_in_memory().unwrap();
        let t = lib.upsert_track(&sample_track("/m/a.flac", "A")).unwrap();
        let track = lib.get_track(t).unwrap().unwrap();
        let uri = "https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69";
        assert!(lib.list_bookmarks().unwrap().is_empty());
        let row = lib.add_bookmark("id1", uri, 1_000).unwrap();
        assert_eq!(row.synced_at, None);
        assert_eq!(lib.unsynced_bookmarks().unwrap().len(), 1);
        lib.mark_bookmark_synced("id1", 2_000).unwrap();
        assert!(lib.unsynced_bookmarks().unwrap().is_empty());
        // Adding again is a no-op that keeps the sync state.
        let again = lib.add_bookmark("id1", uri, 5_000).unwrap();
        assert_eq!(again.created_at, 1_000);
        assert_eq!(again.synced_at, Some(2_000));
        assert_eq!(lib.list_bookmarks().unwrap().len(), 1);
        // Remove → tombstone → push → purged.
        assert!(lib.remove_bookmark(uri).unwrap());
        assert!(!lib.remove_bookmark(uri).unwrap());
        assert!(lib.list_bookmarks().unwrap().is_empty());
        let pending = lib.unsynced_bookmarks().unwrap();
        assert!(pending[0].deleted);
        lib.mark_bookmark_synced("id1", 3_000).unwrap();
        assert!(lib.bookmark_by_uri(uri).unwrap().is_none());
        // Re-adding after a purge starts fresh.
        let fresh = lib.add_bookmark("id1", uri, 9_000).unwrap();
        assert_eq!(fresh.created_at, 9_000);
        // Hearted again while the clear waits to be pushed: the new time
        assert!(lib.remove_bookmark(uri).unwrap());
        let revived = lib.add_bookmark("id1", uri, 12_000).unwrap();
        assert_eq!(revived.created_at, 12_000);
        assert!(!revived.deleted);
        // Album / artist lookups by MBID.
        let album = track.album.as_ref().unwrap();
        let found = lib
            .album_by_release_mbid(album.release_mbid.unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(found.title, album.title);
        let artist = lib
            .artist_by_mbid(track.artists[0].mbid.unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(artist.name, "Daft Punk");
    }
}

#[cfg(test)]
mod tag_tests {
    use super::*;

    #[test]
    fn tags_lifecycle() {
        let lib = Library::open_in_memory().unwrap();
        let uri = "https://musicbrainz.org/recording/b1a9c0e9-d987-4042-ae91-78d6a3267d69";
        let t = lib.add_tag("t1", uri, "disco", 1_000).unwrap();
        assert_eq!(t.label, "disco");
        lib.add_tag("t2", uri, "funk", 2_000).unwrap();
        lib.add_tag(
            "t3",
            "https://musicbrainz.org/release/833f00e1-781f-4edd-90e4-e52712618862",
            "disco",
            3_000,
        )
        .unwrap();
        assert_eq!(lib.list_tags(Some(uri)).unwrap().len(), 2);
        assert_eq!(lib.list_tags(None).unwrap().len(), 3);
        assert_eq!(lib.unsynced_tags().unwrap().len(), 3);
        lib.mark_tag_synced("t1", 5_000).unwrap();
        let again = lib.add_tag("t1", uri, "disco", 9_000).unwrap();
        assert_eq!(again.synced_at, Some(5_000), "re-adding keeps the sync state");
        assert!(lib.remove_tag(uri, "disco").unwrap());
        assert!(!lib.remove_tag(uri, "disco").unwrap());
        assert_eq!(lib.list_tags(Some(uri)).unwrap().len(), 1);
        let pending = lib.unsynced_tags().unwrap();
        assert!(pending.iter().any(|t| t.id == "t1" && t.deleted));
        lib.mark_tag_synced("t1", 6_000).unwrap();
        assert!(
            lib.tag_by_id("t1").unwrap().is_none(),
            "pushed tombstones are purged"
        );
    }
}

#[cfg(test)]
mod folder_tests {
    use super::tests_support::sample_track;
    use super::*;

    #[test]
    fn remove_folder_drops_its_tracks_and_empty_albums() {
        let lib = Library::open_in_memory().unwrap();
        lib.add_folder(Path::new("/m/a")).unwrap();
        lib.add_folder(Path::new("/m/b")).unwrap();
        let a = lib.upsert_track(&sample_track("/m/a/one.flac", "One")).unwrap();
        let b = lib.upsert_track(&sample_track("/m/b/two.flac", "Two")).unwrap();
        let mut other = sample_track("/m/b/three.flac", "Three");
        other.album.as_mut().unwrap().title = "Other".into();
        other.album.as_mut().unwrap().release_mbid = None;
        let c = lib.upsert_track(&other).unwrap();
        lib.record_play(a, None).unwrap();
        assert_eq!(lib.stats().unwrap().albums, 2);
        let (files, tracks) = lib.remove_folder(Path::new("/m/b")).unwrap();
        assert_eq!((files, tracks), (2, 2));
        assert!(lib.get_track(a).unwrap().is_some());
        assert!(lib.get_track(b).unwrap().is_none());
        assert!(lib.get_track(c).unwrap().is_none());
        assert_eq!(
            lib.stats().unwrap().albums,
            1,
            "the album only track three used is gone"
        );
        assert_eq!(lib.list_folders().unwrap().len(), 1);
        assert!(
            lib.search_tracks("Three", 10).unwrap().is_empty(),
            "FTS rows go too"
        );
        assert_eq!(lib.history(10).unwrap().len(), 1);
        // A prefix that is not a path boundary is left alone.
        lib.add_folder(Path::new("/m")).unwrap();
        let (files, _) = lib.remove_folder(Path::new("/m/a_other")).unwrap();
        assert_eq!(files, 0);
        assert!(lib.get_track(a).unwrap().is_some());
    }
}

#[cfg(test)]
mod genre_tests {
    use super::tests_support::sample_track;
    use super::*;

    #[test]
    fn genres_are_stored_browsable_and_searchable() {
        let lib = Library::open_in_memory().unwrap();
        let a = lib.upsert_track(&sample_track("/m/a.flac", "A")).unwrap();
        let mut jazz = sample_track("/m/b.flac", "B");
        jazz.genres = vec!["jazz".into()];
        jazz.album.as_mut().unwrap().title = "Blue".into();
        jazz.album.as_mut().unwrap().release_mbid = None;
        let b = lib.upsert_track(&jazz).unwrap();
        let t = lib.get_track(a).unwrap().unwrap();
        assert_eq!(t.genres, vec!["Disco", "Electronic"]);
        let genres = lib.list_genres().unwrap();
        let names: Vec<&str> = genres.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, vec!["Disco", "Electronic", "jazz"]);
        let disco = genres.iter().find(|g| g.name == "Disco").unwrap();
        assert_eq!((disco.tracks, disco.albums), (1, 1));
        assert_eq!(
            lib.genre_albums(disco.id).unwrap()[0].title,
            "Random Access Memories"
        );
        assert!(lib.genre_tracks(disco.id, 0).unwrap().is_empty(), "limit 0");
        assert_eq!(lib.genre_tracks(disco.id, 10).unwrap()[0].id, Some(a));
        assert!(
            lib.search_tracks("jazz", 10)
                .unwrap()
                .iter()
                .any(|t| t.id == Some(b)),
            "genre words match in search"
        );
        // Re-upserting with another genre replaces, case-insensitively de-duplicated.
        let mut again = sample_track("/m/a.flac", "A");
        again.genres = vec!["DISCO".into()];
        lib.upsert_track(&again).unwrap();
        let genres = lib.list_genres().unwrap();
        assert_eq!(
            genres.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            vec!["Disco", "jazz"]
        );
        assert!(lib.genre_by_name("disco").unwrap().is_some());
        lib.add_folder(Path::new("/m")).unwrap();
        lib.remove_folder(Path::new("/m")).unwrap();
        assert!(
            lib.list_genres().unwrap().is_empty(),
            "orphan genres go with their tracks"
        );
    }
}

#[cfg(test)]
mod identify_tests {
    use super::tests_support::{sample_track, MB};
    use super::*;
    use crate::identify::Candidate;

    const BOHEMIAN: &str = "b1a9c0e9-d987-4042-ae91-78d6a3267d69";

    /// A scanned file whose tags carry no IDs.
    fn untagged(path: &str, title: &str) -> Track {
        let mut t = sample_track(path, title);
        t.recording_mbid = None;
        t.isrc = None;
        t.canonical = None;
        t
    }

    #[test]
    fn suggestions_need_confirmation_and_identities_survive_rescans() {
        let lib = Library::open_in_memory().unwrap();
        let t = untagged("/music/Queen/11.flac", "Bohemian Rhapsody");
        let id = lib.upsert_track(&t).unwrap().0;
        assert_eq!(lib.tracks_to_identify(false).unwrap().len(), 1);
        let c = |mbid: &str, score: f32| Candidate {
            recording: Mbid::parse(mbid).unwrap(),
            release: None,
            title: Some("Bohemian Rhapsody".into()),
            artist: Some("Queen".into()),
            release_title: None,
            score,
            source: "fingerprint".into(),
        };
        lib.add_suggestions(
            id,
            &[c(BOHEMIAN, 0.95), c("ebf79ba5-085e-48d2-9eb8-2d992fbf0f6d", 0.93)],
        )
        .unwrap();
        lib.set_identify_state(id, "suggested", None).unwrap();
        // A suggestion is not an identity.
        assert!(lib.get_track(TrackId(id)).unwrap().unwrap().canonical.is_none());
        let counts = lib.identity_counts().unwrap();
        assert_eq!((counts.identified, counts.suggested, counts.unchecked), (0, 1, 0));
        assert!(
            lib.tracks_to_identify(true).unwrap().is_empty(),
            "suggestions wait for the owner"
        );
        let rows = lib.suggestions().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].recording_mbid, BOHEMIAN, "best first");
        let (tid, uri) = lib.accept_suggestion(rows[0].id).unwrap().unwrap();
        assert_eq!(tid, id);
        assert_eq!(
            uri.to_url(),
            format!("https://musicbrainz.org/recording/{BOHEMIAN}")
        );
        let tr = lib.get_track(TrackId(id)).unwrap().unwrap();
        assert_eq!(tr.canonical.as_ref().unwrap().1, Confidence::ConfirmedByUser);
        assert!(lib.suggestions().unwrap().is_empty());
        // A rescan of the same untagged file keeps the confirmed identity.
        lib.upsert_track(&t).unwrap();
        assert_eq!(
            lib.get_track(TrackId(id)).unwrap().unwrap().canonical.unwrap().1,
            Confidence::ConfirmedByUser
        );
        assert_eq!(lib.tracks_by_canonical(&uri).unwrap().len(), 1);
        // Tags with an MBID win over identification and are never overwritten by it.
        let mut tagged = t.clone();
        let other = Mbid::parse("ebf79ba5-085e-48d2-9eb8-2d992fbf0f6d").unwrap();
        tagged.canonical = Some((MusicUri::Recording { mbid: other }, Confidence::TaggedMbid));
        lib.upsert_track(&tagged).unwrap();
        assert!(!lib
            .set_identity(id, &uri, Confidence::Fingerprint { score: 0.99 }, Some("a"))
            .unwrap());
        assert_eq!(
            lib.get_track(TrackId(id)).unwrap().unwrap().canonical.unwrap().1,
            Confidence::TaggedMbid
        );
    }

    #[test]
    fn rejecting_the_last_suggestion_leaves_the_track_unknown() {
        let lib = Library::open_in_memory().unwrap();
        let id = lib.upsert_track(&untagged("/m/a.flac", "A")).unwrap().0;
        lib.add_suggestions(
            id,
            &[Candidate {
                recording: Mbid::parse(BOHEMIAN).unwrap(),
                release: None,
                title: None,
                artist: None,
                release_title: None,
                score: 0.6,
                source: "search".into(),
            }],
        )
        .unwrap();
        let s = lib.suggestions().unwrap();
        assert!(lib.reject_suggestion(s[0].id).unwrap());
        assert_eq!(lib.identity_counts().unwrap().unknown, 1);
        assert_eq!(
            lib.tracks_to_identify(true).unwrap().len(),
            1,
            "retry looks again"
        );
        assert!(lib.tracks_to_identify(false).unwrap().is_empty());
    }

    #[test]
    fn folder_tracks_and_file_keys() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open_in_memory().unwrap();
        let p = |n: &str| dir.path().join(n).to_string_lossy().to_string();
        lib.upsert_track(&sample_track(&p("a.flac"), "A")).unwrap();
        lib.upsert_track(&sample_track(&p("b.flac"), "B")).unwrap();
        lib.upsert_track(&sample_track(&p("sub/c.flac"), "C")).unwrap();
        assert_eq!(lib.tracks_in_folder(dir.path()).unwrap().len(), 2);
        std::fs::write(dir.path().join("x.bin"), vec![7u8; 3 << 20]).unwrap();
        let k1 = Library::file_key(&dir.path().join("x.bin")).unwrap();
        std::fs::write(dir.path().join("y.bin"), vec![7u8; 3 << 20]).unwrap();
        assert_eq!(k1, Library::file_key(&dir.path().join("y.bin")).unwrap());
        std::fs::write(dir.path().join("y.bin"), vec![8u8; 3 << 20]).unwrap();
        assert_ne!(k1, Library::file_key(&dir.path().join("y.bin")).unwrap());
    }

    /// The owner's edits stay on the node: applied at once, kept when the file is read again,
    /// undone without a rescan; an album renamed keeps its id; a song taken out stays out.
    #[test]
    fn edits_and_removals_stay_on_the_node() {
        let lib = Library::open_in_memory().unwrap();
        let id = lib
            .upsert_track(&sample_track("/m/lucky.flac", "Get Lucky"))
            .unwrap();
        let o = TrackOverride {
            title: Some("Get Lucky (radio edit)".into()),
            artist: Some("Daft Punk, Pharrell Williams".into()),
            ..Default::default()
        };
        let t = lib.edit_track(id, &o).unwrap().unwrap();
        assert_eq!(t.title, "Get Lucky (radio edit)");
        let names: Vec<&str> = t.artists.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Daft Punk", "Pharrell Williams"]);
        assert_eq!(
            t.album.as_ref().unwrap().title,
            "Random Access Memories",
            "album untouched"
        );
        // The file is read again (a rescan): the edit stays on top
        let mut again = sample_track("/m/lucky.flac", "Get Lucky");
        again.id = Some(id);
        lib.refresh_override(&again).unwrap().unwrap().apply(&mut again);
        lib.upsert_track(&again).unwrap();
        assert_eq!(
            lib.get_track(id).unwrap().unwrap().title,
            "Get Lucky (radio edit)"
        );
        // Undo: back to what the file says
        let t = lib.undo_edit(id).unwrap().unwrap();
        assert_eq!(t.title, "Get Lucky");
        assert_eq!(t.artists.len(), 1);
        assert!(lib.track_override(Path::new("/m/lucky.flac")).unwrap().is_none());
        // An empty edit undoes too
        lib.edit_track(
            id,
            &TrackOverride {
                title: Some("X".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let t = lib.edit_track(id, &TrackOverride::default()).unwrap().unwrap();
        assert_eq!(t.title, "Get Lucky");
        // A field the next edit leaves out goes back to what the file says
        lib.edit_track(
            id,
            &TrackOverride {
                title: Some("X".into()),
                artist: Some("Y".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let t = lib
            .edit_track(
                id,
                &TrackOverride {
                    artist: Some("Y".into()),
                    ..Default::default()
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!((t.title.as_str(), t.artists[0].name.as_str()), ("Get Lucky", "Y"));
        assert_eq!(
            lib.file_tags(Path::new("/m/lucky.flac"))
                .unwrap()
                .unwrap()
                .title
                .as_deref(),
            Some("Get Lucky")
        );
        let t = lib.undo_edit(id).unwrap().unwrap();
        // Rename the album: same id, every song on it keeps the name
        let album = t.album.unwrap().id.unwrap();
        assert_eq!(lib.edit_album(album, Some("RAM"), None).unwrap(), AlbumEdit::Done);
        let t = lib.get_track(id).unwrap().unwrap();
        assert_eq!(t.album.as_ref().unwrap().id, Some(album));
        assert_eq!(t.album.as_ref().unwrap().title, "RAM");
        assert_eq!(
            lib.track_override(Path::new("/m/lucky.flac"))
                .unwrap()
                .unwrap()
                .album
                .as_deref(),
            Some("RAM")
        );
        assert_eq!(
            lib.edit_album(999_999, Some("x"), None).unwrap(),
            AlbumEdit::NotFound
        );
        // A name another album has is refused, not merged
        let mut other = sample_track("/m/other.flac", "Other");
        if let Some(a) = other.album.as_mut() {
            a.title = "Taken".into();
        }
        lib.upsert_track(&other).unwrap();
        assert_eq!(
            lib.edit_album(album, Some("Taken"), None).unwrap(),
            AlbumEdit::NameTaken
        );
        // Undo after the rename: back to the file's own album, not to no album
        let t = lib.undo_edit(id).unwrap().unwrap();
        assert_eq!(
            t.album.as_ref().map(|a| a.title.as_str()),
            Some("Random Access Memories")
        );
        // and rename again for what follows
        let album = t.album.unwrap().id.unwrap();
        assert_eq!(lib.edit_album(album, Some("RAM"), None).unwrap(), AlbumEdit::Done);
        // Search finds the songs under the album's new name
        assert!(lib
            .search_tracks("RAM", 10)
            .unwrap()
            .iter()
            .any(|t| t.id == Some(id)));
        // Take it out: nothing is deleted, the song is left out of every read, scans skip it
        lib.record_play(id, None).unwrap();
        assert!(lib.hide_track(id).unwrap());
        assert!(lib.is_hidden(Path::new("/m/lucky.flac")).unwrap());
        assert!(lib.get_track(id).unwrap().is_none());
        assert_eq!(lib.stats().unwrap().files, 1, "only the other song is left");
        assert_eq!(lib.hidden_files().unwrap()[0].0, "/m/lucky.flac");
        assert!(lib.unhide("/m/lucky.flac").unwrap());
        assert!(!lib.is_hidden(Path::new("/m/lucky.flac")).unwrap());
        // back at once, before any scan (a running scan or a remote folder may not read it soon)
        assert!(lib.get_track(id).unwrap().is_some(), "back without a scan");
        assert_eq!(lib.stats().unwrap().files, 2);
        // the next scan sees the file: the same song, with its plays and its edit
        lib.mark_file_seen(Path::new("/m/lucky.flac"), 0).unwrap();
        let back = lib.get_track(id).unwrap().unwrap();
        assert_eq!(back.album.as_ref().map(|a| a.title.as_str()), Some("RAM"));
        assert!(
            lib.history(10).unwrap().iter().any(|p| p.track.id == Some(id)),
            "the plays stay"
        );
        // File names join homeserver files to songs
        lib.upsert_track(&sample_track("/hs/0123456789abcdef-song.flac", "Song"))
            .unwrap();
        assert!(lib
            .track_ids_by_file_name()
            .unwrap()
            .contains_key("0123456789abcdef-song.flac"));
        // ... and by the content hash the homeserver names a file the node saved there
        let id = lib
            .upsert_track(&sample_track("/music/Artist/song.flac", "Saved"))
            .unwrap();
        lib.store_file_hash(
            Path::new("/music/Artist/song.flac"),
            123,
            1_700_000_000,
            &format!("fedcba9876543210{}", "0".repeat(48)),
        )
        .unwrap();
        assert_eq!(
            lib.track_ids_by_hash_prefix().unwrap().get("fedcba9876543210"),
            Some(&id)
        );
    }

    #[test]
    fn edits_keep_artist_ids_covers_and_moved_paths() {
        let lib = Library::open_in_memory().unwrap();
        let id = lib
            .upsert_track(&sample_track("/m/a/lucky.flac", "Get Lucky"))
            .unwrap();
        let artist = lib.get_track(id).unwrap().unwrap().artists[0].id;
        // A second edit, then undo: the same artist, with its MusicBrainz id
        let edit = |title: &str| TrackOverride {
            title: Some(title.into()),
            artist: Some("Someone".into()),
            ..Default::default()
        };
        lib.edit_track(id, &edit("X")).unwrap();
        lib.edit_track(id, &edit("Y")).unwrap();
        let t = lib.undo_edit(id).unwrap().unwrap();
        assert_eq!(t.artists[0].id, artist, "not a second Daft Punk");
        assert_eq!(t.artists[0].mbid, Some(Mbid::parse(MB).unwrap()));
        // The originals kept as plain names (before ids were kept) still restore
        assert_eq!(original_artists(r#"["A","B"]"#).len(), 2);
        // An edit that moves the song to another album takes the cover along, and so does undo
        let album = t.album.as_ref().unwrap().id.unwrap();
        lib.set_album_art(&AlbumArt {
            album_id: album,
            file: PathBuf::from("/m/a/cover.jpg"),
            mime: "image/jpeg".into(),
            source: "folder".into(),
        })
        .unwrap();
        let t = lib
            .edit_track(
                id,
                &TrackOverride {
                    album: Some("Another".into()),
                    ..Default::default()
                },
            )
            .unwrap()
            .unwrap();
        let moved = t.album.unwrap().id.unwrap();
        assert_ne!(moved, album);
        assert_eq!(
            lib.album_art(moved).unwrap().map(|a| a.file),
            Some(PathBuf::from("/m/a/cover.jpg"))
        );
        let t = lib.undo_edit(id).unwrap().unwrap();
        assert!(lib.album_art(t.album.unwrap().id.unwrap()).unwrap().is_some());
        // An album artist alone makes no album for a song without one
        let mut single = sample_track("/m/a/single.flac", "Single");
        single.album = None;
        TrackOverride {
            album_artist: Some("Someone".into()),
            ..Default::default()
        }
        .apply(&mut single);
        assert!(single.album.is_none());
        // A moved folder takes its edits and its removed songs along
        lib.edit_track(id, &edit("Z")).unwrap();
        let gone = lib.upsert_track(&sample_track("/m/a/gone.flac", "Gone")).unwrap();
        assert!(lib.hide_track(gone).unwrap());
        lib.move_folder(Path::new("/m/a"), Path::new("/m/b")).unwrap();
        assert!(lib.is_hidden(Path::new("/m/b/gone.flac")).unwrap());
        assert!(!lib.is_hidden(Path::new("/m/a/gone.flac")).unwrap());
        assert!(lib
            .track_override(Path::new("/m/b/lucky.flac"))
            .unwrap()
            .is_some());
        assert_eq!(lib.undo_edit(id).unwrap().unwrap().title, "Get Lucky");
    }

    /// Migration 15: the favourites table and its node-only settings become bookmarks, rows kept.
    #[test]
    fn favourites_become_bookmarks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            for (version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v <= 14) {
                conn.execute_batch(sql).unwrap();
                conn.execute(
                    "INSERT INTO schema_version(version, applied_at) VALUES (?1, 0)",
                    params![version],
                )
                .unwrap();
            }
            conn.execute(
                "INSERT INTO favourites(id, uri, created_at, synced_at, deleted) VALUES ('f1', 'https://musicbrainz.org/release/1', 7, 9, 0)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO settings(key, value) VALUES ('favourites.local', '[3]'), ('favourites.local.albums', '[4]')",
                [],
            )
            .unwrap();
        }
        let lib = Library::open(&path).unwrap();
        assert_eq!(lib.schema_version().unwrap(), MIGRATIONS.last().unwrap().0);
        let rows = lib.list_bookmarks().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].id.as_str(), rows[0].created_at, rows[0].synced_at),
            ("f1", 7, Some(9))
        );
        assert_eq!(
            lib.get_setting("bookmarks.local").unwrap().as_deref(),
            Some("[3]")
        );
        assert_eq!(
            lib.get_setting("bookmarks.local.albums").unwrap().as_deref(),
            Some("[4]")
        );
        assert_eq!(lib.get_setting("favourites.local").unwrap(), None);
        assert_eq!(lib.backup_counts().unwrap().bookmarks, 1);
    }

    /// A song its folder identified keeps that identity when its file changes (tags fixed,
    /// ReplayGain written) and is read again, and is one "Write IDs" writes.
    #[test]
    fn a_folder_match_survives_a_rescan_of_a_changed_file() {
        let lib = Library::open_in_memory().unwrap();
        let mut t = untagged("/music/Band/Album/01 Song.flac", "Song");
        let id = lib.upsert_track(&t).unwrap().0;
        let uri = MusicUri::Recording {
            mbid: Mbid::parse(BOHEMIAN).unwrap(),
        };
        assert!(lib
            .set_identity(id, &uri, Confidence::AlbumFit { score: 0.95 }, None)
            .unwrap());
        lib.set_identify_state(id, "identified", Some("album_fit"))
            .unwrap();
        assert_eq!(lib.identified_not_tagged().unwrap().len(), 1);
        // The file changed on disk: the scanner reads it again, still without IDs in its tags
        t.file.size_bytes += 512;
        t.file.mtime_unix += 60;
        t.replaygain_track_gain_db = Some(-7.5);
        lib.upsert_track(&t).unwrap();
        let (kept, conf) = lib.get_track(TrackId(id)).unwrap().unwrap().canonical.unwrap();
        assert_eq!(kept, uri);
        assert_eq!(conf, Confidence::AlbumFit { score: 0.95 });
        assert_eq!(lib.identity_counts().unwrap().by.album, 1);
        let todo = lib.identified_not_tagged().unwrap();
        assert_eq!(todo.len(), 1, "Write IDs writes folder matches");
        assert_eq!(todo[0].0.id, Some(TrackId(id)));
        // Nothing cached yet, then an AcoustID answer
        assert!(!lib.has_cached_lookups().unwrap());
        lib.save_fingerprint("k", 3, "AQ").unwrap();
        assert!(
            !lib.has_cached_lookups().unwrap(),
            "a fingerprint not looked up yet"
        );
        lib.save_lookup("k", "{}").unwrap();
        assert!(lib.has_cached_lookups().unwrap());
    }

    fn cand(mbid: &str, title: &str, score: f32, source: &str) -> Candidate {
        Candidate {
            recording: Mbid::parse(mbid).unwrap(),
            release: None,
            title: Some(title.into()),
            artist: Some("Band".into()),
            release_title: Some("Album".into()),
            score,
            source: source.into(),
        }
    }

    const R: [&str; 5] = [
        "00000000-0000-4000-8000-000000000001",
        "00000000-0000-4000-8000-000000000002",
        "00000000-0000-4000-8000-000000000003",
        "00000000-0000-4000-8000-000000000004",
        "00000000-0000-4000-8000-000000000005",
    ];

    /// Album review confirms several songs at once: every choice is checked against what is
    /// open now (the same suggestion, for that song, of that recording, the song still without an
    /// identity) and either all are stored, in one transaction, or none.
    #[test]
    fn a_batch_confirmation_is_all_or_nothing_and_checks_every_choice() {
        let lib = Library::open_in_memory().unwrap();
        let t: Vec<i64> = (1..=5)
            .map(|i| {
                lib.upsert_track(&untagged(&format!("/m/A/{i:02}.flac"), &format!("Song {i}")))
                    .unwrap()
                    .0
            })
            .collect();
        for (i, id) in t.iter().enumerate() {
            lib.add_suggestions(*id, &[cand(R[i], &format!("Song {}", i + 1), 0.9, "album")])
                .unwrap();
            lib.set_identify_state(*id, "suggested", None).unwrap();
        }
        // Song 3 also sounds like another recording: a real conflict, two options
        lib.add_suggestions(t[2], &[cand(R[4], "Song 3 (other take)", 0.92, "fingerprint")])
            .unwrap();
        let open = |track: i64| -> Vec<SuggestionRow> { lib.suggestions_for(track).unwrap() };
        let choice = |track: i64, rec: &str| {
            let row = open(track).into_iter().find(|r| r.recording_mbid == rec).unwrap();
            ConfirmChoice {
                track_id: track,
                suggestion_id: row.id,
                recording_mbid: rec.into(),
            }
        };
        // Two songs confirmed together
        let r = lib
            .confirm_suggestions(&[choice(t[0], R[0]), choice(t[1], R[1])])
            .unwrap();
        assert_eq!(r, ConfirmReport::Confirmed(vec![t[0], t[1]]));
        for (id, rec) in [(t[0], R[0]), (t[1], R[1])] {
            let tr = lib.get_track(TrackId(id)).unwrap().unwrap();
            let (uri, conf) = tr.canonical.unwrap();
            assert_eq!(uri.to_url(), format!("https://musicbrainz.org/recording/{rec}"));
            assert_eq!(conf, Confidence::ConfirmedByUser);
            assert!(open(id).is_empty());
        }
        // Song 4 got an identity meanwhile (a run, another tab): the batch with it stores nothing,
        // song 3's conflict included, and says which choice went stale and why
        let c3 = choice(t[2], R[2]);
        let c4 = choice(t[3], R[3]);
        lib.set_identity(
            t[3],
            &MusicUri::Recording {
                mbid: Mbid::parse(R[3]).unwrap(),
            },
            Confidence::Fingerprint { score: 0.97 },
            None,
        )
        .unwrap();
        let r = lib.confirm_suggestions(&[c3.clone(), c4.clone()]).unwrap();
        assert_eq!(
            r,
            ConfirmReport::Stale(vec![StaleChoice {
                track_id: t[3],
                suggestion_id: c4.suggestion_id,
                reason: "identified".into()
            }])
        );
        assert!(lib.get_track(TrackId(t[2])).unwrap().unwrap().canonical.is_none());
        assert_eq!(open(t[2]).len(), 2, "song 3 still has both options");
        // A suggestion that is gone, one for another song, one naming another recording, and the
        // same song twice: each refused, nothing stored
        let c5 = choice(t[4], R[4]);
        let mut wrong_song = c5.clone();
        wrong_song.track_id = t[0];
        let mut wrong_rec = c3.clone();
        wrong_rec.recording_mbid = R[4].into();
        let r = lib
            .confirm_suggestions(&[
                ConfirmChoice {
                    suggestion_id: 999_999,
                    ..c5.clone()
                },
                wrong_song,
                wrong_rec,
            ])
            .unwrap();
        let ConfirmReport::Stale(why) = r else {
            panic!("{r:?}")
        };
        let reasons: Vec<&str> = why.iter().map(|w| w.reason.as_str()).collect();
        assert_eq!(reasons, vec!["gone", "changed", "changed"]);
        let r = lib
            .confirm_suggestions(&[c3.clone(), choice(t[2], R[4])])
            .unwrap();
        let ConfirmReport::Stale(why) = r else {
            panic!("{r:?}")
        };
        assert_eq!(why[0].reason, "duplicate");
        assert!(lib.get_track(TrackId(t[2])).unwrap().unwrap().canonical.is_none());
        assert!(lib.get_track(TrackId(t[4])).unwrap().unwrap().canonical.is_none());
        // The owner's choice of the conflict, alone: stored
        assert_eq!(
            lib.confirm_suggestions(&[c3]).unwrap(),
            ConfirmReport::Confirmed(vec![t[2]])
        );
        // A failure half way leaves nothing: a trigger refuses the second song's state
        lib.lock()
            .execute_batch(&format!(
                "CREATE TRIGGER no_state BEFORE UPDATE ON identify_state WHEN NEW.track_id = {}
                 BEGIN SELECT RAISE(ABORT, 'disk I/O error'); END;",
                t[4]
            ))
            .unwrap();
        let t6 = lib.upsert_track(&untagged("/m/A/06.flac", "Song 6")).unwrap().0;
        lib.add_suggestions(t6, &[cand(R[0], "Song 6", 0.9, "album")])
            .unwrap();
        lib.set_identify_state(t6, "suggested", None).unwrap();
        assert!(lib.confirm_suggestions(&[choice(t6, R[0]), c5]).is_err());
        assert!(
            lib.get_track(TrackId(t6)).unwrap().unwrap().canonical.is_none(),
            "rolled back"
        );
        assert_eq!(open(t6).len(), 1);
    }

    /// "Not this" is remembered for the song: a later run, a retry or a cached answer never
    /// suggests that recording again for it, nor makes it its identity; and nothing a run finds
    /// replaces the owner's own choice.
    #[test]
    fn rejections_and_the_owners_choices_survive_later_runs() {
        let lib = Library::open_in_memory().unwrap();
        let id = lib.upsert_track(&untagged("/m/x.flac", "X")).unwrap().0;
        lib.add_suggestions(
            id,
            &[cand(R[0], "X", 0.8, "search"), cand(R[1], "X", 0.7, "search")],
        )
        .unwrap();
        let rows = lib.suggestions_for(id).unwrap();
        assert!(lib.reject_suggestion(rows[0].id).unwrap());
        assert!(lib.is_rejected(id, R[0]).unwrap());
        assert!(!lib.is_rejected(id, R[1]).unwrap());
        // The same answer again (a retry, the cache): the rejected one stays out
        lib.add_suggestions(
            id,
            &[
                cand(R[0], "X", 0.99, "fingerprint"),
                cand(R[1], "X", 0.7, "search"),
            ],
        )
        .unwrap();
        let left: Vec<String> = lib
            .suggestions_for(id)
            .unwrap()
            .into_iter()
            .map(|r| r.recording_mbid)
            .collect();
        assert_eq!(left, vec![R[1].to_string()]);
        // Rejecting the last one: unknown, and a legacy retry still looks at it again
        let last = lib.suggestions_for(id).unwrap()[0].id;
        assert!(lib.reject_suggestion(last).unwrap());
        assert_eq!(lib.tracks_to_identify(true).unwrap().len(), 1);
        // The owner picks one (another song): no run's answer replaces it
        let other = lib.upsert_track(&untagged("/m/y.flac", "Y")).unwrap().0;
        lib.add_suggestions(other, &[cand(R[2], "Y", 0.8, "search")])
            .unwrap();
        let s = lib.suggestions_for(other).unwrap()[0].id;
        lib.accept_suggestion(s).unwrap().unwrap();
        let uri = |r: &str| MusicUri::Recording {
            mbid: Mbid::parse(r).unwrap(),
        };
        assert!(!lib
            .set_identity(other, &uri(R[3]), Confidence::Fingerprint { score: 0.99 }, None)
            .unwrap());
        assert!(!lib
            .set_identity(other, &uri(R[3]), Confidence::AlbumFit { score: 0.95 }, None)
            .unwrap());
        let (kept, conf) = lib.get_track(TrackId(other)).unwrap().unwrap().canonical.unwrap();
        assert_eq!((kept, conf), (uri(R[2]), Confidence::ConfirmedByUser));
        // A suggestion accepted for a song that got an identity meanwhile is refused
        let third = lib.upsert_track(&untagged("/m/z.flac", "Z")).unwrap().0;
        lib.add_suggestions(third, &[cand(R[4], "Z", 0.8, "search")])
            .unwrap();
        let s = lib.suggestions_for(third).unwrap()[0].id;
        lib.set_identity(third, &uri(R[3]), Confidence::Fingerprint { score: 0.95 }, None)
            .unwrap();
        assert_eq!(lib.accept_suggestion(s).unwrap(), None);
        assert_eq!(
            lib.get_track(TrackId(third))
                .unwrap()
                .unwrap()
                .canonical
                .unwrap()
                .0,
            uri(R[3])
        );
    }

    fn rec_uri(r: &str) -> MusicUri {
        MusicUri::Recording {
            mbid: Mbid::parse(r).unwrap(),
        }
    }

    fn state_of(lib: &Library, track: i64) -> Option<(String, Option<String>, Option<String>)> {
        lib.lock()
            .query_row(
                "SELECT state, detail, release_mbid FROM identify_state WHERE track_id = ?1",
                params![track],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .unwrap()
    }

    /// Fix round 1, item 1: a run's answer is stored in one guarded step, against what holds
    /// now: a recording the owner rejected meanwhile is not stored; a song the owner confirmed
    /// meanwhile keeps the owner's recording, state and (no) release; suggestions for it are not
    /// added; only a real transition reports `Stored`.
    #[test]
    fn a_run_answer_is_settled_in_one_guarded_step() {
        let lib = Library::open_in_memory().unwrap();
        let rel = Mbid::parse(R[4]).unwrap();
        let t: Vec<i64> = (1..=5)
            .map(|i| {
                lib.upsert_track(&untagged(&format!("/m/S/{i}.flac"), &format!("S{i}")))
                    .unwrap()
                    .0
            })
            .collect();
        let found = |track: i64| SettleAnswer::Identified {
            recording: rec_uri(R[0]),
            confidence: Confidence::AlbumFit { score: 0.95 },
            acoustid: None,
            detail: "album_fit".into(),
            release: Some(rel),
            input_key: Some(format!("k{track}")),
        };
        // The owner rejects A while the run decides
        lib.add_suggestions(t[0], &[cand(R[0], "S1", 0.9, "search")])
            .unwrap();
        let sid = lib.suggestions_for(t[0]).unwrap()[0].id;
        assert!(lib.reject_suggestion(sid).unwrap());
        assert_eq!(lib.settle(t[0], &found(t[0])).unwrap(), Settled::Rejected);
        assert!(lib.get_track(TrackId(t[0])).unwrap().unwrap().canonical.is_none());
        assert_ne!(state_of(&lib, t[0]).unwrap().0, "identified");
        // The owner confirms B while the run decides A
        lib.add_suggestions(t[1], &[cand(R[1], "S2", 0.9, "search")])
            .unwrap();
        let sid = lib.suggestions_for(t[1]).unwrap()[0].id;
        lib.accept_suggestion(sid).unwrap().unwrap();
        assert_eq!(lib.settle(t[1], &found(t[1])).unwrap(), Settled::Kept);
        let (uri, conf) = lib.get_track(TrackId(t[1])).unwrap().unwrap().canonical.unwrap();
        assert_eq!((uri, conf), (rec_uri(R[1]), Confidence::ConfirmedByUser));
        assert_eq!(
            state_of(&lib, t[1]).unwrap(),
            ("identified".into(), Some("confirmed_by_user".into()), None),
            "no release of the run's recording on the owner's"
        );
        let suggest = SettleAnswer::Suggest {
            candidates: vec![cand(R[2], "x", 0.9, "fingerprint")],
            input_key: None,
            replace: true,
        };
        assert_eq!(lib.settle(t[1], &suggest).unwrap(), Settled::Kept);
        assert!(lib.suggestions_for(t[1]).unwrap().is_empty());
        assert_eq!(state_of(&lib, t[1]).unwrap().0, "identified");
        let unknown = SettleAnswer::State {
            state: "unknown".into(),
            detail: None,
            input_key: None,
        };
        assert_eq!(lib.settle(t[1], &unknown).unwrap(), Settled::Kept);
        assert_eq!(state_of(&lib, t[1]).unwrap().0, "identified");
        // Nothing in the way: stored, with its release and what the song looked like
        assert_eq!(lib.settle(t[2], &found(t[2])).unwrap(), Settled::Stored);
        assert_eq!(
            state_of(&lib, t[2]).unwrap(),
            ("identified".into(), Some("album_fit".into()), Some(R[4].into()))
        );
        assert_eq!(
            lib.get_track(TrackId(t[2]))
                .unwrap()
                .unwrap()
                .canonical
                .unwrap()
                .0,
            rec_uri(R[0])
        );
        // Suggestions: the rejected one stays out; none left is "unknown", not "suggested"
        lib.add_suggestions(t[3], &[cand(R[3], "S4", 0.9, "search")])
            .unwrap();
        let sid = lib.suggestions_for(t[3]).unwrap()[0].id;
        assert!(lib.reject_suggestion(sid).unwrap());
        let both = SettleAnswer::Suggest {
            candidates: vec![
                cand(R[3], "S4", 0.9, "fingerprint"),
                cand(R[2], "S4b", 0.9, "fingerprint"),
            ],
            input_key: Some("k4".into()),
            replace: true,
        };
        assert_eq!(lib.settle(t[3], &both).unwrap(), Settled::Stored);
        assert_eq!(lib.suggestions_for(t[3]).unwrap().len(), 1);
        assert_eq!(state_of(&lib, t[3]).unwrap().0, "suggested");
        // A song left "suggested" with nothing open (an older run) whose answer is all rejected
        lib.set_identify_state(t[4], "suggested", None).unwrap();
        lib.add_suggestions(t[4], &[cand(R[3], "S5", 0.9, "search")])
            .unwrap();
        let sid = lib.suggestions_for(t[4]).unwrap()[0].id;
        lib.lock()
            .execute(
                "INSERT INTO identify_rejections(track_id, recording_mbid, rejected_at) VALUES (?1, ?2, 0)",
                params![t[4], R[3]],
            )
            .unwrap();
        lib.lock()
            .execute("DELETE FROM identify_suggestions WHERE id = ?1", params![sid])
            .unwrap();
        let rejected_only = SettleAnswer::Suggest {
            candidates: vec![cand(R[3], "S5", 0.9, "search")],
            input_key: None,
            replace: true,
        };
        assert_eq!(lib.settle(t[4], &rejected_only).unwrap(), Settled::Empty);
        assert_eq!(state_of(&lib, t[4]).unwrap().0, "unknown");
        assert!(lib
            .tracks_to_identify(true)
            .unwrap()
            .iter()
            .any(|x| x.id == Some(TrackId(t[4]))));
    }

    /// Fix round 1, item 6: a suggestion's id is never given to another one (deleting the newest,
    /// or all, of them); a stale "Not this" or accept names nothing, and the caller's expectation
    /// (song, recording) is checked.
    #[test]
    fn suggestion_ids_are_never_reused_and_expectations_are_checked() {
        let lib = Library::open_in_memory().unwrap();
        let a = lib.upsert_track(&untagged("/m/I/a.flac", "A")).unwrap().0;
        let b = lib.upsert_track(&untagged("/m/I/b.flac", "B")).unwrap().0;
        lib.add_suggestions(a, &[cand(R[0], "A", 0.9, "search")]).unwrap();
        let old = lib.suggestions_for(a).unwrap()[0].id;
        assert!(lib.reject_suggestion(old).unwrap());
        lib.add_suggestions(b, &[cand(R[1], "B", 0.9, "search")]).unwrap();
        let new = lib.suggestions_for(b).unwrap()[0].id;
        assert_ne!(new, old, "the newest id deleted is not given again");
        // A stale tab's "Not this" on the old id: nothing to reject
        assert!(!lib.reject_suggestion(old).unwrap());
        assert!(!lib.is_rejected(b, R[1]).unwrap());
        // Expectations: the right song and recording, else refused
        assert_eq!(
            lib.check_suggestion(new, Some(a), None).unwrap(),
            SuggestionCheck::Changed
        );
        assert_eq!(
            lib.check_suggestion(new, Some(b), Some(R[0])).unwrap(),
            SuggestionCheck::Changed
        );
        assert_eq!(
            lib.check_suggestion(new, Some(b), Some(R[1])).unwrap(),
            SuggestionCheck::Holds
        );
        assert_eq!(
            lib.check_suggestion(old, None, None).unwrap(),
            SuggestionCheck::Gone
        );
        // All deleted: still new ids
        lib.lock()
            .execute("DELETE FROM identify_suggestions", [])
            .unwrap();
        lib.add_suggestions(a, &[cand(R[2], "A", 0.9, "search")]).unwrap();
        assert!(lib.suggestions_for(a).unwrap()[0].id > new);
    }

    /// Fix round 1, item 5: a song's open suggestions are for the file and tags they were found
    /// with. A rescan that finds the file replaced or its tags changed drops them (and the song is
    /// checked again), so a review page loaded before can't confirm them: the batch is stale.
    #[test]
    fn a_changed_file_or_tags_drop_the_songs_open_suggestions() {
        let lib = Library::open_in_memory().unwrap();
        let mut t = untagged("/m/C/01.flac", "One");
        let id = lib.upsert_track(&t).unwrap().0;
        t.id = Some(TrackId(id));
        let open = |lib: &Library| lib.suggestions_for(id).unwrap();
        let suggest = |lib: &Library| {
            lib.add_suggestions(id, &[cand(R[0], "One", 0.9, "fingerprint")])
                .unwrap();
            lib.set_identify_state(id, "suggested", None).unwrap();
            let row = lib.suggestions_for(id).unwrap()[0].clone();
            ConfirmChoice {
                track_id: id,
                suggestion_id: row.id,
                recording_mbid: row.recording_mbid,
            }
        };
        let choice = suggest(&lib);
        // The same file read again: kept
        lib.upsert_track(&t).unwrap();
        assert_eq!(open(&lib).len(), 1);
        // Replaced at the same path (another size and time): dropped, the batch is stale
        t.file.size_bytes += 1000;
        t.file.mtime_unix += 5;
        lib.upsert_track(&t).unwrap();
        assert!(open(&lib).is_empty());
        assert!(state_of(&lib, id).is_none(), "checked again by the next run");
        let sid = choice.suggestion_id;
        assert_eq!(
            lib.confirm_suggestions(&[choice]).unwrap(),
            ConfirmReport::Stale(vec![StaleChoice {
                track_id: id,
                suggestion_id: sid,
                reason: "gone".into()
            }])
        );
        assert!(lib.get_track(TrackId(id)).unwrap().unwrap().canonical.is_none());
        // Its tags corrected: dropped too
        let choice = suggest(&lib);
        t.title = "One (fixed)".into();
        lib.upsert_track(&t).unwrap();
        assert!(open(&lib).is_empty());
        let ConfirmReport::Stale(why) = lib.confirm_suggestions(&[choice]).unwrap() else {
            panic!("stale expected")
        };
        assert_eq!(why[0].reason, "gone");
    }

    /// PR #250 review (high): a run's answer is for the song as it was when the run read it. A
    /// rescan that replaced its file or changed its tags meanwhile makes the answer stale: it is
    /// not stored (nor written into the new file); the song is checked again.
    #[test]
    fn an_answer_for_a_song_that_changed_meanwhile_is_not_stored() {
        let lib = Library::open_in_memory().unwrap();
        let mut t = untagged("/m/G/01.flac", "One");
        let id = lib.upsert_track(&t).unwrap().0;
        t.id = Some(TrackId(id));
        let read = lib.input_generations().unwrap().get(&id).cloned();
        assert!(read.is_some());
        let found = SettleAnswer::Identified {
            recording: rec_uri(R[0]),
            confidence: Confidence::TagMatch { score: 0.9 },
            acoustid: None,
            detail: "tag_match".into(),
            release: None,
            input_key: Some("k".into()),
        };
        // The file is replaced while the run works on it
        t.file.size_bytes += 10;
        t.file.mtime_unix += 1;
        lib.upsert_track(&t).unwrap();
        assert_eq!(
            lib.settle_checked(id, &found, read.as_deref()).unwrap(),
            Settled::Changed
        );
        assert!(lib.get_track(TrackId(id)).unwrap().unwrap().canonical.is_none());
        assert!(state_of(&lib, id).is_none(), "checked again by the next run");
        // Its tags change: the same
        let read = lib.input_generations().unwrap().get(&id).cloned();
        t.title = "One (fixed)".into();
        lib.upsert_track(&t).unwrap();
        assert_eq!(
            lib.settle_checked(id, &found, read.as_deref()).unwrap(),
            Settled::Changed
        );
        // Nothing changed since it was read: stored
        let read = lib.input_generations().unwrap().get(&id).cloned();
        assert_eq!(
            lib.settle_checked(id, &found, read.as_deref()).unwrap(),
            Settled::Stored
        );
        assert!(lib.get_track(TrackId(id)).unwrap().unwrap().canonical.is_some());
    }

    /// PR #250 review (low): the release id in a song's tags is part of what identification goes
    /// by: correcting only it drops the song's open suggestions, as other tag changes do.
    #[test]
    fn a_corrected_release_id_drops_open_suggestions() {
        let lib = Library::open_in_memory().unwrap();
        let mut t = untagged("/m/R/01.flac", "Rel");
        let id = lib.upsert_track(&t).unwrap().0;
        t.id = Some(TrackId(id));
        lib.add_suggestions(id, &[cand(R[0], "Rel", 0.9, "fingerprint")])
            .unwrap();
        lib.set_identify_state(id, "suggested", None).unwrap();
        let album = t.album.as_mut().expect("the sample track has an album");
        album.release_mbid = Some(Mbid::parse(R[3]).unwrap());
        lib.upsert_track(&t).unwrap();
        assert!(lib.suggestions_for(id).unwrap().is_empty());
    }

    /// An album as on the owner's NUC (2026-11-04): each song once on the NAS and once as an old
    /// homeserver copy whose file was deleted. The album must show each song once.
    fn album_with_gone_copies(lib: &Library, n: u32) -> (i64, Vec<i64>, Vec<i64>) {
        let (mut present, mut gone) = (Vec::new(), Vec::new());
        for no in 1..=n {
            let mut t = sample_track(
                &format!("/music/Roon/Crossroad/{no:02}.flac"),
                &format!("Song {no}"),
            );
            t.track_no = Some(no);
            t.duration_ms = Some(60_000);
            present.push(lib.upsert_track(&t).unwrap().0);
            t.file.path = PathBuf::from(format!(
                "/homeserver/data/files/u/priv/pubky.music/files/{:016x}-{no:02}.flac",
                no
            ));
            gone.push(lib.upsert_track(&t).unwrap().0);
        }
        let scan = lib.begin_scan(Path::new("/homeserver")).unwrap();
        lib.mark_missing_under(Path::new("/homeserver"), scan).unwrap();
        let album = lib
            .get_track(TrackId(present[0]))
            .unwrap()
            .unwrap()
            .album
            .unwrap()
            .id
            .unwrap();
        (album, present, gone)
    }

    #[test]
    fn songs_whose_files_are_gone_leave_album_artist_and_counts() {
        let lib = Library::open_in_memory().unwrap();
        let (album, present, gone) = album_with_gone_copies(&lib, 4);
        let tracks = lib.album_tracks(album).unwrap();
        let ids: Vec<i64> = tracks.iter().map(|t| t.id.unwrap().0).collect();
        assert_eq!(ids, present, "each song once, the present copy");
        let total: u64 = tracks.iter().filter_map(|t| t.duration_ms).sum();
        assert_eq!(total, 4 * 60_000);
        let artist = tracks[0].artists[0].id.unwrap();
        assert_eq!(lib.artist_tracks(artist).unwrap().len(), 4);
        let summary = lib.get_artist(artist).unwrap().unwrap();
        assert_eq!((summary.albums, summary.tracks), (1, 4));
        let genres = lib.list_genres().unwrap();
        assert!(genres.iter().all(|g| g.tracks == 4), "{genres:?}");
        // PR #268 review (Claude H1): read by id it is still there, so favourites, playlists,
        // plays and device downloads that name it keep it; no list shows it
        for id in &gone {
            assert!(
                lib.get_track(TrackId(*id)).unwrap().is_some(),
                "a gone copy stays readable by id"
            );
        }
        let listed: Vec<i64> = lib
            .list_tracks(None, 100)
            .unwrap()
            .iter()
            .map(|t| t.id.unwrap().0)
            .collect();
        assert_eq!(listed, present);
        let stats = lib.stats().unwrap();
        assert_eq!((stats.tracks, stats.albums, stats.artists), (4, 1, 1));
        // the rows stay: plays and ids are kept
        lib.record_play(TrackId(gone[0]), None).unwrap();
        assert_eq!(lib.play_count(TrackId(gone[0])).unwrap(), 1);
        assert!(lib
            .file_by_path(Path::new(&format!(
                "/homeserver/data/files/u/priv/pubky.music/files/{:016x}-01.flac",
                1
            )))
            .unwrap()
            .is_some());
    }

    #[test]
    fn an_album_with_no_present_song_leaves_the_lists_and_comes_back() {
        let lib = Library::open_in_memory().unwrap();
        let mut t = sample_track("/nas/Gone/01.flac", "Gone");
        t.album.as_mut().unwrap().title = "Gone album".into();
        t.artists[0].name = "Gone artist".into();
        t.artists[0].mbid = None;
        let id = lib.upsert_track(&t).unwrap().0;
        let album = lib
            .get_track(TrackId(id))
            .unwrap()
            .unwrap()
            .album
            .unwrap()
            .id
            .unwrap();
        let artist = lib.get_track(TrackId(id)).unwrap().unwrap().artists[0]
            .id
            .unwrap();
        lib.record_play(TrackId(id), None).unwrap();
        let scan = lib.begin_scan(Path::new("/nas")).unwrap();
        lib.mark_missing_under(Path::new("/nas"), scan).unwrap();
        let listed = |lib: &Library| {
            (
                lib.list_albums(None, 100)
                    .unwrap()
                    .iter()
                    .any(|a| a.id == Some(album)),
                lib.recent_albums(0, 100)
                    .unwrap()
                    .iter()
                    .any(|a| a.id == Some(album)),
                lib.recently_played_albums(10)
                    .unwrap()
                    .iter()
                    .any(|a| a.id == Some(album)),
                lib.most_played_albums(10)
                    .unwrap()
                    .iter()
                    .any(|a| a.id == Some(album)),
                lib.list_artists().unwrap().iter().any(|a| a.id == artist),
                lib.artist_albums(artist).unwrap().len(),
                lib.album_tracks(album).unwrap().len(),
            )
        };
        assert_eq!(listed(&lib), (false, false, false, false, false, 0, 0));
        // the file is found again: the song is back as it was
        let scan = lib.begin_scan(Path::new("/nas")).unwrap();
        lib.mark_file_seen(Path::new("/nas/Gone/01.flac"), scan).unwrap();
        assert_eq!(listed(&lib), (true, true, true, true, true, 1, 1));
        assert_eq!(lib.album_tracks(album).unwrap()[0].id, Some(TrackId(id)));
        assert_eq!(lib.play_count(TrackId(id)).unwrap(), 1);
    }

    #[test]
    fn copies_of_a_song_are_one_row_with_versions() {
        let lib = Library::open_in_memory().unwrap();
        let mut t = sample_track("/m/Album/01.flac", "Song");
        t.track_no = Some(1);
        let mut add = |path: &str, container: &str, rate: u32, bits: u8, size: u64| {
            t.file.path = PathBuf::from(path);
            t.file.container = container.into();
            t.file.format = AudioFormat::pcm(rate, bits, 2);
            t.file.size_bytes = size;
            lib.upsert_track(&t).unwrap().0
        };
        // a homeserver copy (first, so ids alone would pick it), an MP3 and a hi-res FLAC
        let home = add(
            "/data/homeserver/1698088e5519dff7-01.flac",
            "flac",
            96_000,
            24,
            900,
        );
        let mp3 = add("/m/Album/01.mp3", "mp3", 44_100, 16, 100);
        let flac = add("/m/Album/01 (hi-res).flac", "flac", 96_000, 24, 800);
        let other = {
            let mut o = sample_track("/m/Album/02.flac", "Other");
            o.track_no = Some(2);
            lib.upsert_track(&o).unwrap().0
        };
        let album = lib
            .get_track(TrackId(flac))
            .unwrap()
            .unwrap()
            .album
            .unwrap()
            .id
            .unwrap();
        let songs = lib.album_songs(album).unwrap();
        assert_eq!(songs.len(), 2, "one row per song");
        assert_eq!(
            songs[0].track.id,
            Some(TrackId(flac)),
            "local over homeserver, then quality"
        );
        let v: Vec<(i64, &str, bool)> = songs[0]
            .versions
            .iter()
            .map(|v| (v.track_id, v.source.as_str(), v.best))
            .collect();
        assert_eq!(
            v,
            vec![
                (flac, "library", true),
                (mp3, "library", false),
                (home, "homeserver", false)
            ]
        );
        assert_eq!(songs[0].versions[0].format, "24/96 FLAC");
        assert!(!songs[0].versions[1].lossless);
        assert_eq!(songs[1].track.id, Some(TrackId(other)));
        assert!(songs[1].versions.is_empty(), "one copy: no versions");
        let artist = songs[0].track.artists[0].id.unwrap();
        assert_eq!(lib.artist_tracks(artist).unwrap().len(), 2);
        assert_eq!(lib.get_artist(artist).unwrap().unwrap().tracks, 2);
        assert!(lib.list_genres().unwrap().iter().all(|g| g.tracks == 2));
        assert_eq!(lib.random_tracks(10).unwrap().len(), 2);
        // every copy stays a playable track of its own
        assert!(lib.get_track(TrackId(home)).unwrap().is_some());
    }

    #[test]
    fn same_content_under_another_title_is_one_song() {
        let lib = Library::open_in_memory().unwrap();
        let mut a = sample_track("/m/Album/01.flac", "Song");
        a.track_no = Some(1);
        let a_id = lib.upsert_track(&a).unwrap().0;
        let mut b = a.clone();
        b.title = "01".into();
        b.track_no = None;
        b.file.path = PathBuf::from("/m/copies/01.flac");
        let b_id = lib.upsert_track(&b).unwrap().0;
        for p in ["/m/Album/01.flac", "/m/copies/01.flac"] {
            lib.store_file_hash(Path::new(p), 123, 1_700_000_000, "abc")
                .unwrap();
        }
        let album = lib
            .get_track(TrackId(a_id))
            .unwrap()
            .unwrap()
            .album
            .unwrap()
            .id
            .unwrap();
        let songs = lib.album_songs(album).unwrap();
        assert_eq!(songs.len(), 1);
        assert_eq!(songs[0].versions.len(), 2);
        assert!(songs[0].versions.iter().any(|v| v.track_id == b_id));
        // PR #268 review (Codex M1): counts go by the same songs as the lists
        let artist = songs[0].track.artists[0].id.unwrap();
        assert_eq!(lib.get_artist(artist).unwrap().unwrap().tracks, 1);
        assert!(lib.list_genres().unwrap().iter().all(|g| g.tracks == 1));
    }

    /// PR #268 review (Codex M2): a limit counts songs, not copies, and a song keeps every copy
    /// even when the limit falls between them.
    #[test]
    fn a_limit_counts_songs_and_keeps_every_copy() {
        let lib = Library::open_in_memory().unwrap();
        for no in 1..=4 {
            let mut t = sample_track(&format!("/m/A/{no:02}.mp3"), &format!("Song {no}"));
            t.track_no = Some(no);
            t.file.container = "mp3".into();
            lib.upsert_track(&t).unwrap();
            t.file.path = PathBuf::from(format!("/m/A/{no:02}.flac"));
            t.file.container = "flac".into();
            lib.upsert_track(&t).unwrap();
        }
        let genre = lib.list_genres().unwrap()[0].id;
        let got = lib.genre_tracks(genre, 3).unwrap();
        assert_eq!(got.len(), 3, "three songs, not three copies");
        assert!(
            got.iter().all(|t| t.file.container == "flac"),
            "each plays its best copy"
        );
        assert_eq!(lib.random_tracks(3).unwrap().len(), 3);
        assert_eq!(lib.random_tracks(10).unwrap().len(), 4);
    }

    #[test]
    fn search_still_finds_present_songs_only_and_is_not_grouped() {
        let lib = Library::open_in_memory().unwrap();
        let (_, present, _) = album_with_gone_copies(&lib, 2);
        let found: Vec<i64> = lib
            .search_tracks("Song", 10)
            .unwrap()
            .iter()
            .map(|t| t.id.unwrap().0)
            .collect();
        let mut sorted = found.clone();
        sorted.sort();
        assert_eq!(sorted, present);
    }

    #[test]
    fn homeserver_copies_are_told_apart() {
        assert!(is_homeserver_copy(Path::new(
            "/homeserver/data/files/u/priv/pubky.music/files/x-01.flac"
        )));
        assert!(is_homeserver_copy(Path::new(
            "/data/homeserver/1698088e5519dff7-01.flac"
        )));
        assert!(!is_homeserver_copy(Path::new("/music/Roon/01 Song.flac")));
        assert!(!is_homeserver_copy(Path::new("/music/1698088E5519DFF7-01.flac")));
        assert!(!is_homeserver_copy(Path::new(
            "/music/2024010112345678-live.flac"
        )));
    }

    /// PR #268 review (Claude L1): two discs without disc tags each with a track 1 "Intro" are
    /// two songs; the same song in two formats is one.
    #[test]
    fn songs_of_other_lengths_are_not_merged() {
        let lib = Library::open_in_memory().unwrap();
        let mut t = sample_track("/m/A/CD1/01.flac", "Intro");
        t.track_no = Some(1);
        t.disc_no = None;
        t.duration_ms = Some(60_000);
        let one = lib.upsert_track(&t).unwrap().0;
        t.file.path = PathBuf::from("/m/A/CD2/01.flac");
        t.duration_ms = Some(95_000);
        let two = lib.upsert_track(&t).unwrap().0;
        t.file.path = PathBuf::from("/m/A/CD1/01.mp3");
        t.file.container = "mp3".into();
        t.duration_ms = Some(60_400);
        lib.upsert_track(&t).unwrap();
        let album = lib
            .get_track(TrackId(one))
            .unwrap()
            .unwrap()
            .album
            .unwrap()
            .id
            .unwrap();
        let songs = lib.album_songs(album).unwrap();
        let ids: Vec<i64> = songs.iter().map(|s| s.track.id.unwrap().0).collect();
        assert_eq!(ids, vec![one, two]);
        assert_eq!(songs[0].versions.len(), 2);
        assert!(songs[1].versions.is_empty());
    }

    /// PR #268 review (Claude L3): a stored hash counts only for the file as it was hashed.
    #[test]
    fn a_stale_hash_does_not_merge_songs() {
        let lib = Library::open_in_memory().unwrap();
        let mut a = sample_track("/m/A/01.flac", "One");
        a.track_no = Some(1);
        let a_id = lib.upsert_track(&a).unwrap().0;
        let mut b = sample_track("/m/A/02.flac", "Two");
        b.track_no = Some(2);
        lib.upsert_track(&b).unwrap();
        lib.store_file_hash(Path::new("/m/A/01.flac"), 123, 1_700_000_000, "same")
            .unwrap();
        // hashed when it had another size: the file changed since
        lib.store_file_hash(Path::new("/m/A/02.flac"), 999, 1_700_000_000, "same")
            .unwrap();
        let album = lib
            .get_track(TrackId(a_id))
            .unwrap()
            .unwrap()
            .album
            .unwrap()
            .id
            .unwrap();
        assert_eq!(lib.album_songs(album).unwrap().len(), 2);
    }

    /// PR #268 delta review (Codex M2): a hidden file is no present song: it takes no place in a
    /// limited list and no count.
    #[test]
    fn a_hidden_file_takes_no_place_in_a_list() {
        let lib = Library::open_in_memory().unwrap();
        let mut a = sample_track("/m/A/01.flac", "One");
        a.track_no = Some(1);
        let hidden = lib.upsert_track(&a).unwrap();
        let mut b = sample_track("/m/A/02.flac", "Two");
        b.track_no = Some(2);
        let shown = lib.upsert_track(&b).unwrap();
        lib.hide_track(hidden).unwrap();
        // as if a scan had seen the file again without asking whether it is hidden
        lib.lock().execute("UPDATE files SET missing = 0", []).unwrap();
        let genre = lib.list_genres().unwrap()[0].clone();
        assert_eq!(genre.tracks, 1);
        let got = lib.genre_tracks(genre.id, 1).unwrap();
        assert_eq!(got.iter().map(|t| t.id).collect::<Vec<_>>(), vec![Some(shown)]);
        assert_eq!(lib.random_tracks(5).unwrap().len(), 1);
        assert_eq!(lib.stats().unwrap().tracks, 1);
    }

    /// PR #268 delta review (Codex M1): Shuffle draws songs, so a song with copies is not likelier.
    #[test]
    fn shuffle_draws_songs_not_copies() {
        let lib = Library::open_in_memory().unwrap();
        let mut a = sample_track("/m/A/01.flac", "Many");
        a.track_no = Some(1);
        for n in 0..9 {
            a.file.path = PathBuf::from(format!("/m/A/01-{n}.flac"));
            lib.upsert_track(&a).unwrap();
        }
        let mut b = sample_track("/m/A/02.flac", "Once");
        b.track_no = Some(2);
        let once = lib.upsert_track(&b).unwrap();
        let hits = (0..400)
            .filter(|_| lib.random_tracks(1).unwrap()[0].id == Some(once))
            .count();
        // a fair draw gives about 200; drawing copies would give about 40
        assert!((120..=280).contains(&hits), "{hits}");
    }

    /// PR #268 CI (speed): the presence checks ran through the index on `files.missing`, one
    /// scan of every present file per track (11 s for 5,000 tracks). They must use `files_track`.
    #[test]
    fn whole_library_counts_stay_fast() {
        let lib = Library::open_in_memory().unwrap();
        lib.begin_batch().unwrap();
        for n in 0..3000u32 {
            let mut t = sample_track(&format!("/m/A{}/{n}.flac", n / 10), &format!("S{n}"));
            t.album.as_mut().unwrap().title = format!("Album {}", n / 10);
            t.track_no = Some(n % 10 + 1);
            t.recording_mbid = None;
            t.isrc = None;
            t.canonical = None;
            lib.upsert_track(&t).unwrap();
        }
        lib.commit_batch().unwrap();
        let t0 = std::time::Instant::now();
        assert_eq!(lib.stats().unwrap().tracks, 3000);
        lib.list_artists().unwrap();
        lib.list_genres().unwrap();
        lib.random_tracks(50).unwrap();
        assert_eq!(lib.list_albums(None, 1000).unwrap().len(), 300);
        assert_eq!(lib.recent_albums(0, 1000).unwrap().len(), 300);
        let took = t0.elapsed();
        // a few tens of ms here; the quadratic plan took seconds
        assert!(took < std::time::Duration::from_secs(2), "{took:?}");
    }

    /// A new library file is readable by the node's user alone: it holds sign-ins.
    #[cfg(unix)]
    #[test]
    fn a_new_library_is_the_nodes_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.sqlite");
        let _lib = Library::open(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    fn rules_of(lib: &Library, track: i64) -> Option<i64> {
        lib.lock()
            .query_row(
                "SELECT rules FROM identify_state WHERE track_id = ?1",
                params![track],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// A song an older version of the rules left waiting, as on a node from before them: its
    /// options, "suggested", no input key and no rules.
    fn left_by_older_rules(lib: &Library, path: &str, options: &[Candidate]) -> i64 {
        let id = lib.upsert_track(&untagged(path, "X")).unwrap().0;
        lib.add_suggestions(id, options).unwrap();
        lib.set_identify_state(id, "suggested", None).unwrap();
        assert_eq!(rules_of(lib, id), None);
        id
    }

    /// Checking a waiting song again (owner, 2026-11-06): the options become what the rules find
    /// now (one that stays keeps its id); an error changes nothing, so a later run tries again;
    /// "nothing found" keeps the options for the owner and marks the song checked; a rejected pick
    /// leaves it waiting; every check stores the rules' version, and older ones count as outdated.
    #[test]
    fn checking_a_waiting_song_again_replaces_or_keeps_its_options() {
        let lib = Library::open_in_memory().unwrap();
        let ids = |lib: &Library, t: i64| -> Vec<(i64, String)> {
            lib.suggestions_for(t)
                .unwrap()
                .into_iter()
                .map(|r| (r.id, r.recording_mbid))
                .collect()
        };
        let a = left_by_older_rules(
            &lib,
            "/m/a.flac",
            &[cand(R[0], "X", 0.8, "search"), cand(R[1], "X", 0.7, "search")],
        );
        assert_eq!(lib.identity_counts().unwrap().outdated, 1);
        assert!(lib.has_outdated_suggestions().unwrap());
        let kept_id = ids(&lib, a).into_iter().find(|(_, r)| r == R[1]).unwrap().0;
        let now = SettleAnswer::Suggest {
            candidates: vec![cand(R[1], "X", 0.75, "search"), cand(R[2], "X", 0.7, "search")],
            input_key: Some("k".into()),
            replace: true,
        };
        assert_eq!(lib.settle(a, &now).unwrap(), Settled::Stored);
        let after = ids(&lib, a);
        assert_eq!(after.len(), 2, "{after:?}");
        assert!(
            after.contains(&(kept_id, R[1].to_string())),
            "kept its id: {after:?}"
        );
        assert!(after.iter().any(|(_, r)| r == R[2]));
        assert!(!after.iter().any(|(_, r)| r == R[0]), "an older option is gone");
        assert_eq!(rules_of(&lib, a), Some(crate::identify::RULES_VERSION as i64));
        let c = lib.identity_counts().unwrap();
        assert_eq!((c.suggested, c.outdated), (1, 0));
        assert!(!lib.has_outdated_suggestions().unwrap());

        // An error: nothing written, the song still outdated for the next run
        let b = left_by_older_rules(&lib, "/m/b.flac", &[cand(R[3], "X", 0.8, "search")]);
        let err = SettleAnswer::State {
            state: "error".into(),
            detail: Some("MusicBrainz answered 503".into()),
            input_key: Some("k".into()),
        };
        assert_eq!(lib.settle(b, &err).unwrap(), Settled::Failed);
        assert_eq!(state_of(&lib, b).unwrap().0, "suggested");
        assert_eq!(rules_of(&lib, b), None);
        assert_eq!(ids(&lib, b).len(), 1);
        // Options added ("Which song is this?"): the open ones stay, the song stays outdated
        let added = SettleAnswer::Suggest {
            candidates: vec![cand(R[4], "X", 0.6, "search")],
            input_key: None,
            replace: false,
        };
        assert_eq!(lib.settle(b, &added).unwrap(), Settled::Stored);
        assert_eq!(ids(&lib, b).len(), 2, "added to the open option");
        assert_eq!(rules_of(&lib, b), None);
        assert_eq!(lib.identity_counts().unwrap().outdated, 1);
        // Nothing found: the options stay for the owner, the song is marked checked
        let nothing = SettleAnswer::State {
            state: "unknown".into(),
            detail: None,
            input_key: Some("k".into()),
        };
        assert_eq!(lib.settle(b, &nothing).unwrap(), Settled::Kept);
        assert_eq!(state_of(&lib, b).unwrap().0, "suggested");
        assert_eq!(rules_of(&lib, b), Some(crate::identify::RULES_VERSION as i64));
        assert_eq!(ids(&lib, b).len(), 2);

        // The rules pick a recording the owner rejected for it: it keeps waiting with what's open
        let c = left_by_older_rules(
            &lib,
            "/m/c.flac",
            &[cand(R[0], "X", 0.8, "search"), cand(R[4], "X", 0.8, "search")],
        );
        let r4 = ids(&lib, c).into_iter().find(|(_, r)| r == R[4]).unwrap().0;
        assert!(lib.reject_suggestion(r4).unwrap());
        let pick = SettleAnswer::Identified {
            recording: rec_uri(R[4]),
            confidence: Confidence::TagMatch { score: 0.9 },
            acoustid: None,
            detail: "tag_match".into(),
            release: None,
            input_key: Some("k".into()),
        };
        assert_eq!(lib.settle(c, &pick).unwrap(), Settled::Kept);
        assert_eq!(state_of(&lib, c).unwrap().0, "suggested");
        assert!(lib.get_track(TrackId(c)).unwrap().unwrap().canonical.is_none());
        // A clear pick settles it, its options gone
        let pick = SettleAnswer::Identified {
            recording: rec_uri(R[0]),
            confidence: Confidence::TagMatch { score: 0.9 },
            acoustid: None,
            detail: "tag_match".into(),
            release: None,
            input_key: Some("k".into()),
        };
        assert_eq!(lib.settle(c, &pick).unwrap(), Settled::Stored);
        assert!(ids(&lib, c).is_empty());
        assert_eq!(rules_of(&lib, c), Some(crate::identify::RULES_VERSION as i64));
    }

    /// "Leave as is" (owner, 2026-11-06): left songs are out of review, the counts and every run,
    /// keep their options, and come back with them; songs with an identity or nothing open are not
    /// touched; a run's answer for a left song changes nothing.
    #[test]
    fn songs_left_as_they_are_stay_out_of_review_and_runs_until_brought_back() {
        let lib = Library::open_in_memory().unwrap();
        let a = left_by_older_rules(&lib, "/m/one/a.flac", &[cand(R[0], "X", 0.8, "search")]);
        let b = left_by_older_rules(&lib, "/m/one/b.flac", &[cand(R[1], "X", 0.8, "search")]);
        let none = lib.upsert_track(&untagged("/m/two/c.flac", "C")).unwrap().0;
        lib.set_identify_state(none, "unknown", None).unwrap();
        assert_eq!(lib.skip_review(&[a, none, 999]).unwrap(), 1);
        assert_eq!(lib.skip_review(&[a]).unwrap(), 0, "already left");
        let c = lib.identity_counts().unwrap();
        assert_eq!((c.suggested, c.skipped, c.outdated), (1, 1, 1));
        let listed: Vec<i64> = lib
            .suggested_track_paths()
            .unwrap()
            .into_iter()
            .map(|(i, _)| i)
            .collect();
        assert_eq!(listed, vec![b]);
        assert!(lib.suggestions().unwrap().iter().all(|r| r.track_id == b));
        assert_eq!(
            lib.suggested_tracks()
                .unwrap()
                .iter()
                .map(|t| t.id.unwrap().0)
                .collect::<Vec<_>>(),
            vec![b]
        );
        assert_eq!(lib.skipped_track_paths().unwrap().len(), 1);
        // Its file offline: still found, so "bring back all" reaches it
        let missing = |m: i64| {
            lib.lock()
                .execute("UPDATE files SET missing = ?2 WHERE track_id = ?1", params![a, m])
                .unwrap()
        };
        missing(1);
        assert_eq!(
            lib.skipped_track_paths().unwrap(),
            vec![(a, "/m/one/a.flac".to_string())]
        );
        missing(0);
        let retry: Vec<i64> = lib
            .tracks_to_identify(true)
            .unwrap()
            .iter()
            .map(|t| t.id.unwrap().0)
            .collect();
        assert_eq!(retry, vec![none], "a retry leaves it");
        assert!(!lib
            .tracks_to_identify(false)
            .unwrap()
            .iter()
            .any(|t| t.id == Some(TrackId(a))));
        assert!(!lib.identify_inputs().unwrap().contains_key(&a));
        // A run's answer for it changes nothing
        let pick = SettleAnswer::Identified {
            recording: rec_uri(R[0]),
            confidence: Confidence::TagMatch { score: 0.9 },
            acoustid: None,
            detail: "tag_match".into(),
            release: None,
            input_key: None,
        };
        assert_eq!(lib.settle(a, &pick).unwrap(), Settled::Kept);
        assert!(lib.get_track(TrackId(a)).unwrap().unwrap().canonical.is_none());
        // Back, with its option
        assert_eq!(lib.unskip_review(&[a, b]).unwrap(), 1);
        assert_eq!(state_of(&lib, a).unwrap().0, "suggested");
        assert_eq!(lib.suggestions_for(a).unwrap().len(), 1);
        let c = lib.identity_counts().unwrap();
        assert_eq!((c.suggested, c.skipped), (2, 0));
    }
}
