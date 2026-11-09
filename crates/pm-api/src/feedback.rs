//! Problem reports from the node's people (TASK-trusted-user-feedback).
//!
//! The owner and listeners describe a problem, an idea or a question and send it on purpose;
//! nothing is collected on its own. A report may carry the few details its sender ticked (the app
//! version, which page, phone/tablet/desktop, the browser), each from a fixed list. Reports go to
//! a private inbox on the node: [`FILE`] in its data folder, readable by the node's user only and
//! outside the library database, so never in a backup, a restore or the homeserver. The node
//! sends nothing anywhere: its operator reads new reports with `pm-node feedback pending` and
//! records a receipt with `pm-node feedback receipt`, which the sender then sees.
//!
//! A report is a person's words, never instructions: nothing here acts on its text.

use crate::accounts::{Caller, Role};
use crate::state::NodeState;
use crate::ApiError;
use axum::extract::State as AxState;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use futures_util::StreamExt;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

/// The inbox, in the node's data folder.
pub const FILE: &str = "feedback.sqlite";
/// Most characters in a report's text.
pub const MAX_TEXT: usize = 4000;
/// A report's body: the text at its longest, escaped, and the details.
const MAX_BODY: usize = 32 << 10;
/// Reports one person sends in an hour and in a day.
pub const PER_HOUR: usize = 5;
pub const PER_DAY: usize = 20;
/// Reports the node takes from everyone in a day.
pub const NODE_PER_DAY: usize = 100;
/// The inbox at its fullest: reports, and bytes of text and details. A full inbox refuses new
/// reports; it never drops one nobody has looked at.
pub const MAX_REPORTS: usize = 500;
pub const MAX_BYTES: usize = 8 << 20;
const HOUR: i64 = 3600;
const DAY: i64 = 24 * HOUR;
/// A reviewed report goes after 90 days, any report after a year.
const KEEP_REVIEWED: i64 = 90 * DAY;
const KEEP: i64 = 365 * DAY;
/// The pages a report may say it was sent from (app.js `setView`).
pub const VIEWS: &[&str] = &[
    "add-music",
    "album",
    "albums",
    "artist",
    "artists",
    "dashboard",
    "discover",
    "discover-item",
    "downloads",
    "favourites",
    "genre",
    "genres",
    "hidden",
    "home",
    "home-section",
    "identify",
    "item",
    "library",
    "nowplaying",
    "person",
    "playlist",
    "playlists",
    "queue",
    "radio",
    "releases",
    "search",
    "settings",
    "shared",
    "smart",
    "smart-edit",
    "source-artist",
    "source-item",
    "sources",
    "tag",
    "tracks",
    "zones",
];
/// What a token pasted into a report becomes.
const REMOVED: &str = "[token removed]";
/// The starts of the node's tokens: a listener's, an owner's browser session, a party guest's.
const TOKEN_PREFIXES: &[&str] = &["pml_", "pmo_", crate::party::GUEST_PREFIX];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Problem,
    Idea,
    Question,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shell {
    Phone,
    Tablet,
    Desktop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Browser {
    Chromium,
    Firefox,
    Safari,
    Other,
}

/// The details a sender ticked, each from a fixed list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Details {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<Shell>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<Browser>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_major: Option<u16>,
}

impl Details {
    fn valid(&self) -> bool {
        let version_ok = self.version.as_deref().is_none_or(|v| {
            (1..=32).contains(&v.len())
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'))
        });
        version_ok && self.view.as_deref().is_none_or(|v| VIEWS.contains(&v))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewReport {
    kind: Kind,
    text: String,
    /// The text as written (cleaned), before tokens come out: only for telling the same report
    /// sent again from an edit, through `Inbox::digest`; never stored.
    #[serde(skip)]
    clean: String,
    client_id: String,
    #[serde(default)]
    details: Option<Details>,
}

/// Where a report is: saved on the node, received by its operator, reviewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Saved,
    Received,
    Reviewed,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            State::Saved => "saved",
            State::Received => "received",
            State::Reviewed => "reviewed",
        }
    }
    fn parse(s: &str) -> Option<State> {
        [State::Saved, State::Received, State::Reviewed]
            .into_iter()
            .find(|x| x.as_str() == s)
    }
}

impl std::str::FromStr for State {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        State::parse(s).ok_or_else(|| "a state is saved, received or reviewed".into())
    }
}

/// A new report as the operator's export shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pending {
    pub id: String,
    pub kind: Kind,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Details>,
    /// `owner` or `listener`.
    pub from: String,
    /// The same short name for every report one person sends; never their key.
    pub person: String,
    pub sent_at: i64,
}

/// One of a person's own reports, as they see it.
#[derive(Debug, Clone, Serialize)]
pub struct Mine {
    pub id: String,
    pub kind: Kind,
    pub sent_at: i64,
    pub state: State,
    /// The start of the text.
    pub preview: String,
}

#[derive(Debug)]
pub enum InboxError {
    /// Too many reports: try again in this many seconds.
    Wait(i64),
    Full,
    /// The sender's sign-in ended before the report was written.
    SignedOut,
    /// The same report id came again with other words: the first version is the one kept.
    Changed,
    /// A receipt that can't be: a state going back, a malformed reference.
    Invalid(&'static str),
    Store(String),
}

impl std::fmt::Display for InboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InboxError::Wait(s) => write!(f, "too many reports; try again in {s} s"),
            InboxError::Full => f.write_str("the inbox is full"),
            InboxError::SignedOut => f.write_str("the sender's sign-in ended"),
            InboxError::Changed => f.write_str("that report was already saved with other words"),
            InboxError::Invalid(why) => f.write_str(why),
            InboxError::Store(e) => write!(f, "inbox: {e}"),
        }
    }
}

impl std::error::Error for InboxError {}

impl From<rusqlite::Error> for InboxError {
    fn from(e: rusqlite::Error) -> Self {
        InboxError::Store(e.to_string())
    }
}

impl From<std::io::Error> for InboxError {
    fn from(e: std::io::Error) -> Self {
        InboxError::Store(e.to_string())
    }
}

/// Unix seconds.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS report (
    id TEXT PRIMARY KEY,
    client_id TEXT NOT NULL,
    reporter TEXT NOT NULL,
    kind TEXT NOT NULL,
    text TEXT NOT NULL,
    details TEXT,
    sent_at INTEGER NOT NULL,
    state TEXT NOT NULL,
    received_at INTEGER,
    reviewed_at INTEGER,
    receipt TEXT,
    digest TEXT,
    UNIQUE (reporter, client_id)
);
CREATE INDEX IF NOT EXISTS report_sent ON report (sent_at);
CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v BLOB NOT NULL);";

/// The node's private inbox.
pub struct Inbox {
    conn: Connection,
    /// This inbox's own key for listeners' short names (made once, kept in the inbox).
    secret: [u8; 32],
}

impl Inbox {
    /// Open the inbox in `dir` if the node has made one: the operator's commands never make it
    /// (made by another user, it would lock the node out of its own inbox).
    pub fn open_existing(dir: &Path) -> Result<Option<Inbox>, InboxError> {
        // Anything there counts, a link too: it is refused below, never taken for no inbox
        if std::fs::symlink_metadata(dir.join(FILE)).is_err() {
            return Ok(None);
        }
        Inbox::open(dir).map(Some)
    }

    /// Open (or make) the inbox in `dir`.
    pub fn open(dir: &Path) -> Result<Inbox, InboxError> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(FILE);
        // Made for the node's user alone, before SQLite opens it, and never through a link: a link
        // at the inbox's place would have reports written into whatever it points to (Kimi review
        // of fc1dca51, M3). The file is looked at and made private through the opened descriptor
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let not_ours = |what: String| InboxError::Store(format!("{FILE} {what}; the inbox isn't opened"));
            let file = match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&path)
            {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(&path)
                    .map_err(|e| not_ours(format!("can't be opened as a file, a link is refused ({e})")))?,
                Err(e) => return Err(e.into()),
            };
            let meta = file.metadata()?;
            if !meta.file_type().is_file() {
                return Err(not_ours("isn't a regular file".into()));
            }
            if meta.permissions().mode() & 0o777 != 0o600 {
                file.set_permissions(std::fs::Permissions::from_mode(0o600)).map_err(|e| {
                    InboxError::Store(format!(
                        "{FILE} can't be made private ({e}): it belongs to another user; run as the node's user"
                    ))
                })?;
            }
            // SQLite's own files beside it: a link there would take report pages elsewhere too
            for ext in ["-journal", "-wal", "-shm"] {
                let side = dir.join(format!("{FILE}{ext}"));
                if let Ok(m) = std::fs::symlink_metadata(&side) {
                    if !m.file_type().is_file() {
                        return Err(not_ours(format!("has a {ext} that isn't a regular file")));
                    }
                }
            }
        }
        let flags = rusqlite::OpenFlags::default() | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let mut conn = Connection::open_with_flags(&path, flags)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // What the inbox lets go of (a purged report, a cleared digest) is overwritten in this
        // file, not left in its free space: whoever copies the file has its key too. It is set per
        // connection, so every opener comes through here. SQLite's journal and the disk itself are
        // beyond it (docs/api.md)
        let overwrites: i64 = conn.pragma_update_and_check(None, "secure_delete", true, |r| r.get(0))?;
        if overwrites != 1 {
            return Err(InboxError::Store(
                "SQLite won't overwrite what the inbox deletes".into(),
            ));
        }
        conn.execute_batch(SCHEMA)?;
        // An inbox made before the digest column: looked at and changed in one go, so two first
        // opens at once can't both try to add it
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !tx
            .prepare("SELECT 1 FROM pragma_table_info('report') WHERE name = 'digest'")?
            .exists([])?
        {
            tx.execute_batch("ALTER TABLE report ADD COLUMN digest TEXT")?;
        }
        tx.commit()?;
        conn.execute(
            "INSERT OR IGNORE INTO meta (k, v) VALUES ('reporter_key', ?1)",
            params![rand::random::<[u8; 32]>().to_vec()],
        )?;
        let key: Vec<u8> = conn.query_row("SELECT v FROM meta WHERE k = 'reporter_key'", [], |row| {
            row.get(0)
        })?;
        let secret = key
            .try_into()
            .map_err(|_| InboxError::Store("the inbox's reporter key is damaged".into()))?;
        Ok(Inbox { conn, secret })
    }

    /// Drop what is past keeping: reviewed reports after 90 days, any after a year.
    pub fn purge(&self, now: i64) -> Result<usize, InboxError> {
        forget_digests(&self.conn, now)?;
        Ok(self.conn.execute(
            "DELETE FROM report WHERE sent_at < ?1 OR (reviewed_at IS NOT NULL AND reviewed_at < ?2)",
            params![now - KEEP, now - KEEP_REVIEWED],
        )?)
    }

    /// Save a report. `Ok((id, false))` for one sent before with the same `client_id`. `still` says
    /// whether its sender is still signed in; it is asked first, once the inbox is this write's.
    fn add(
        &mut self,
        reporter: &str,
        r: &NewReport,
        now: i64,
        still: &dyn Fn() -> bool,
    ) -> Result<(String, bool), InboxError> {
        let details = r
            .details
            .as_ref()
            .filter(|d| **d != Details::default())
            .map(|d| serde_json::to_string(d).expect("serializes"));
        let digest = self.digest(r, details.as_deref());
        // What is past keeping goes first, in a write of its own: the report's write below is
        // undone when it is turned away, and must not take the expiry with it
        self.purge(now)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Asked first, once the inbox is this write's: a sender who signed out meanwhile learns
        // nothing from it (not a retry's id, an edit's 409 or a full window)
        if !still() {
            return Err(InboxError::SignedOut);
        }
        let kind = serde_json::to_value(r.kind).expect("serializes");
        let before: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT id, digest FROM report WHERE reporter = ?1 AND client_id = ?2",
                params![reporter, r.client_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((id, d)) = before {
            // Sent again after a lost answer: the same words, or an edit that would be lost
            if d.as_deref() != Some(digest.as_str()) {
                return Err(InboxError::Changed);
            }
            return Ok((id, false));
        }
        // A full window (one person's hour or day, the node's day) frees a place when its oldest
        // report leaves it
        for (who, window, max) in [
            (Some(reporter), HOUR, PER_HOUR),
            (Some(reporter), DAY, PER_DAY),
            (None, DAY, NODE_PER_DAY),
        ] {
            let mut st = tx.prepare(
                "SELECT sent_at FROM report WHERE (?1 IS NULL OR reporter = ?1) AND sent_at > ?2
                 ORDER BY sent_at",
            )?;
            let times: Vec<i64> = st
                .query_map(params![who, now - window], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            if times.len() >= max {
                return Err(InboxError::Wait((times[times.len() - max] + window - now).max(1)));
            }
        }
        // What waits for the operator; what they took in is safe with them and leaves room
        let (count, bytes): (i64, i64) = tx.query_row(
            "SELECT COUNT(*), COALESCE(SUM(LENGTH(CAST(text AS BLOB)) + COALESCE(LENGTH(details), 0)), 0)
             FROM report WHERE state = 'saved'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let size = r.text.len() + details.as_ref().map_or(0, String::len);
        if count as usize >= MAX_REPORTS || bytes as usize + size > MAX_BYTES {
            return Err(InboxError::Full);
        }
        // And again right before the write: a sign-out during the checks above saves nothing
        if !still() {
            return Err(InboxError::SignedOut);
        }
        let id = format!("fb_{}", hex::encode(rand::random::<[u8; 8]>()));
        tx.execute(
            "INSERT INTO report (id, client_id, reporter, kind, text, details, sent_at, state, digest)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'saved', ?8)",
            params![
                id,
                r.client_id,
                reporter,
                kind.as_str(),
                r.text,
                details,
                now,
                digest
            ],
        )?;
        tx.commit()?;
        Ok((id, true))
    }

    /// A person's own reports, newest first (at most 50).
    fn mine(&self, reporter: &str) -> Result<Vec<Mine>, InboxError> {
        let mut st = self.conn.prepare(
            "SELECT id, kind, sent_at, state, text FROM report WHERE reporter = ?1
             ORDER BY sent_at DESC, rowid DESC LIMIT 50",
        )?;
        let rows = st.query_map(params![reporter], |row| {
            let text: String = row.get(4)?;
            Ok(Mine {
                id: row.get(0)?,
                kind: kind_of(&row.get::<_, String>(1)?),
                sent_at: row.get(2)?,
                state: State::parse(&row.get::<_, String>(3)?).unwrap_or(State::Saved),
                preview: text.chars().take(80).collect(),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The reports no one has received yet, oldest first.
    pub fn pending(&self, now: i64) -> Result<Vec<Pending>, InboxError> {
        self.purge(now)?;
        let mut st = self.conn.prepare(
            "SELECT id, kind, text, details, reporter, sent_at FROM report WHERE state = 'saved'
             ORDER BY sent_at, rowid",
        )?;
        let rows = st.query_map([], |row| {
            let reporter: String = row.get(4)?;
            let details: Option<String> = row.get(3)?;
            Ok(Pending {
                id: row.get(0)?,
                kind: kind_of(&row.get::<_, String>(1)?),
                text: row.get(2)?,
                details: details.and_then(|d| serde_json::from_str(&d).ok()),
                from: if reporter == OWNER { "owner" } else { "listener" }.into(),
                person: if reporter == OWNER {
                    OWNER.into()
                } else {
                    reporter.chars().take(8).collect()
                },
                sent_at: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Record that the operator received or reviewed report `id`, under their own `reference`
    /// (1–64 of `A-Z a-z 0-9 . _ -`). `Ok(false)`: no such report. A state never goes back; the
    /// same receipt again is fine.
    pub fn receipt(&mut self, id: &str, state: State, reference: &str, now: i64) -> Result<bool, InboxError> {
        if !valid_id(id) {
            return Ok(false);
        }
        if !(1..=64).contains(&reference.len())
            || !reference
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        {
            return Err(InboxError::Invalid(
                "a receipt reference is 1–64 letters, digits, '.', '_' or '-'",
            ));
        }
        if state == State::Saved {
            return Err(InboxError::Invalid("a receipt says received or reviewed"));
        }
        // Read and write in one go: two receipts at once never take a state back
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now_state: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT state, receipt FROM report WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((was, receipt)) = now_state else {
            return Ok(false);
        };
        let was = State::parse(&was).unwrap_or(State::Saved);
        if was == state && receipt.as_deref() == Some(reference) {
            return Ok(true);
        }
        if rank(state) <= rank(was) {
            return Err(InboxError::Invalid("a report's state never goes back"));
        }
        let column = match state {
            State::Received => "received_at",
            _ => "reviewed_at",
        };
        tx.execute(
            &format!(
                "UPDATE report SET state = ?1, receipt = ?2, {column} = ?3, digest = NULL WHERE id = ?4"
            ),
            params![state.as_str(), reference, now, id],
        )?;
        tx.commit()?;
        Ok(true)
    }
}

/// How long a report can be sent again after a lost answer and still be told from an edit.
const RESEND: i64 = DAY;

/// A digest is only for telling a resend from an edit: once a report is received or reviewed, or
/// a day old, none is kept, so the inbox holds no lasting check on what was written before tokens
/// came out. A report sent again after that is a new one (409).
fn forget_digests(conn: &Connection, now: i64) -> Result<(), InboxError> {
    conn.execute(
        "UPDATE report SET digest = NULL WHERE digest IS NOT NULL AND (state != 'saved' OR sent_at < ?1)",
        params![now - RESEND],
    )?;
    Ok(())
}

fn rank(s: State) -> u8 {
    match s {
        State::Saved => 0,
        State::Received => 1,
        State::Reviewed => 2,
    }
}

fn kind_of(s: &str) -> Kind {
    serde_json::from_value(serde_json::Value::String(s.into())).unwrap_or(Kind::Problem)
}

/// A report id: `fb_` and 16 hex digits.
pub fn valid_id(id: &str) -> bool {
    id.len() == 19
        && id.starts_with("fb_")
        && id[3..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The owner, as the inbox keeps them.
const OWNER: &str = "owner";

/// Who may send a report: the owner or a listener (their key); party guests and nobody else.
enum Sender {
    Owner,
    Listener(String),
}

fn sender(c: &Caller) -> Option<Sender> {
    match (c.role, c.key.as_deref()) {
        (Role::Owner, _) => Some(Sender::Owner),
        (Role::Listener, Some(k)) => Some(Sender::Listener(k.trim_start_matches("pubky").to_string())),
        _ => None,
    }
}

/// Whether the caller a token gives `now` is still the person it gave `then`: the owner, or the
/// same listener. Not the whole `Caller`: the owner's key can load during a request (the node's
/// identity syncing), which is no sign-out (held Low 3 of the Claude review of 2d2f0a8a).
fn same_person(then: &Caller, now: Option<&Caller>) -> bool {
    match (sender(then), now.and_then(sender)) {
        (Some(Sender::Owner), Some(Sender::Owner)) => true,
        (Some(Sender::Listener(a)), Some(Sender::Listener(b))) => a == b,
        _ => false,
    }
}

impl Inbox {
    /// What a report says, as written: keyed with the inbox's secret, so it tells nothing by
    /// itself. A report sent again is the same one only when this matches; comparing the stored
    /// text instead (tokens out) would tell a sender whether a guess was a token.
    fn digest(&self, r: &NewReport, details: Option<&str>) -> String {
        let mut h = blake3::Hasher::new_keyed(&self.secret);
        h.update(b"pm-feedback-report\0");
        h.update(serde_json::to_string(&r.kind).expect("serializes").as_bytes());
        h.update(b"\0");
        h.update(r.clean.as_bytes());
        h.update(b"\0");
        h.update(details.unwrap_or("").as_bytes());
        h.finalize().to_hex().to_string()
    }

    /// Who sent a report, as the inbox keeps it: the owner, or a listener's key hashed with this
    /// inbox's own secret. The inbox never holds anyone's key, and its short names can't be matched
    /// to a public key or to the same person on another node.
    fn reporter_of(&self, who: &Sender) -> String {
        match who {
            Sender::Owner => OWNER.into(),
            Sender::Listener(k) => blake3::keyed_hash(&self.secret, k.as_bytes())
                .to_hex()
                .to_string(),
        }
    }
}

/// A report's text as kept: one kind of line break, no control characters or direction
/// overrides (a reader sees what was written), no spaces around it.
fn clean(text: &str) -> String {
    let bidi = |c: char| matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}');
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|&c| c == '\n' || c == '\t' || !(c.is_control() || bidi(c)))
        .collect::<String>()
        .trim()
        .to_string()
}

/// The text with every token taken out: the `secrets` given (the sender's own, the node's API
/// token) and anything shaped like one of the node's tokens.
fn scrub(text: &str, secrets: &[String]) -> String {
    // Every range to take out, found in the text as written: each secret (however short) and
    // each token shape; overlapping ranges become one, so no part of either stays
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for s in secrets.iter().filter(|s| !s.is_empty()) {
        ranges.extend(text.match_indices(s.as_str()).map(|(at, m)| (at, at + m.len())));
    }
    let token_char = |b: u8| b.is_ascii_alphanumeric() || b == b'-' || b == b'_';
    for p in TOKEN_PREFIXES {
        for (at, _) in text.match_indices(p) {
            let run = text[at + 4..].bytes().take_while(|&b| token_char(b)).count();
            if run >= 16 {
                ranges.push((at, at + 4 + run));
            }
        }
    }
    ranges.sort_unstable();
    let mut out = String::with_capacity(text.len());
    let mut done = 0;
    let mut i = 0;
    while i < ranges.len() {
        let (from, mut to) = ranges[i];
        while i + 1 < ranges.len() && ranges[i + 1].0 <= to {
            i += 1;
            to = to.max(ranges[i].1);
        }
        out.push_str(&text[done..from]);
        out.push_str(REMOVED);
        done = to;
        i += 1;
    }
    out.push_str(&text[done..]);
    out
}

const EMPTY: &str = "Describe the problem first.";
const TOO_LONG: &str = "That report is too long: at most 4000 characters.";

/// A report's text as it is kept, or why it can't be: cleaned, tokens taken out, then measured.
fn prepare(raw: &str, secrets: &[String]) -> Result<String, &'static str> {
    // Measured before the tokens come out: whether a report is taken never depends on a secret
    // in it (with the node's token at least MIN_TOKEN long, taking one out only shortens it)
    let text = clean(raw);
    match text.chars().count() {
        0 => Err(EMPTY),
        n if n > MAX_TEXT => Err(TOO_LONG),
        _ => Ok(scrub(&text, secrets)),
    }
}

/// The bearer token from the `Authorization` header only: a token in the address ends up in
/// proxy logs and the browser's history, so reports never take one from there.
fn header_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string)
}

const IN_THE_HEADER: &str = "Send the token in the Authorization header.";

fn refuse(status: StatusCode, msg: &str) -> Response {
    ApiError::new(status, msg).into_response()
}

const NO_TOKEN: &str =
    "Reports need the node's API token set: without one the node can't tell who sent them.";
/// The shortest node API token reports work with, in characters (the node warns below it): a
/// shorter one could be guessed from how reports with it in them are taken.
pub const MIN_TOKEN: usize = 16;
const SHORT_TOKEN: &str =
    "Reports need the node's API token to be at least 16 characters long. Ask the node's owner to set a longer one.";
const REPORTS_OFF: &str = "Reports are off on this node for now. Its owner can turn them on.";

/// Why reports are off with a short token: the owner learns why, anyone else only that they are.
fn short_token(c: &Caller) -> &'static str {
    if c.role == Role::Owner {
        SHORT_TOKEN
    } else {
        REPORTS_OFF
    }
}

const NOT_UNDERSTOOD: &str = "The node didn't understand that report. Reload the page and try again.";
const NOT_SAVED: &str = "The node couldn't save the report. Try again.";

/// The node's answer to a saved report.
#[derive(Serialize)]
pub struct Saved {
    pub id: String,
    pub state: State,
}

/// `POST /v1/feedback`: save a report from the owner or a listener.
pub async fn send(
    AxState(s): AxState<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
    req: axum::extract::Request,
) -> Response {
    // Without a token every caller is the owner: no telling who sent a report
    let Some(api_token) = s.token() else {
        return refuse(StatusCode::FORBIDDEN, NO_TOKEN);
    };
    if api_token.chars().count() < MIN_TOKEN {
        return refuse(StatusCode::FORBIDDEN, short_token(&c));
    }
    let Some(token) = header_token(&headers) else {
        return refuse(StatusCode::UNAUTHORIZED, IN_THE_HEADER);
    };
    let Some(who) = sender(&c) else {
        return refuse(StatusCode::FORBIDDEN, "Only the node's people send reports.");
    };
    // The body, bounded, read here: a refusal never repeats it (nor logs it)
    let mut body = req.into_body().into_data_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = body.next().await {
        let Ok(chunk) = chunk else {
            return refuse(StatusCode::BAD_REQUEST, NOT_UNDERSTOOD);
        };
        if bytes.len() + chunk.len() > MAX_BODY {
            return refuse(
                StatusCode::PAYLOAD_TOO_LARGE,
                "That report is too long for the node.",
            );
        }
        bytes.extend_from_slice(&chunk);
    }
    // Still signed in as the same person once the body is here (and again at the write)
    let signed_in = {
        let (s, token, c) = (s.clone(), token.clone(), c.clone());
        move || same_person(&c, crate::accounts::caller(&s, Some(&token)).as_ref())
    };
    if !signed_in() {
        return refuse(StatusCode::UNAUTHORIZED, "missing or wrong bearer token");
    }
    let Ok(mut r) = serde_json::from_slice::<NewReport>(&bytes) else {
        return refuse(StatusCode::BAD_REQUEST, NOT_UNDERSTOOD);
    };
    let Ok(client_id) = uuid::Uuid::parse_str(&r.client_id) else {
        return refuse(StatusCode::BAD_REQUEST, NOT_UNDERSTOOD);
    };
    r.client_id = client_id.hyphenated().to_string();
    if !r.details.as_ref().is_none_or(Details::valid) {
        return refuse(StatusCode::BAD_REQUEST, NOT_UNDERSTOOD);
    }
    r.clean = clean(&r.text);
    r.text = match prepare(&r.text, &[api_token, token]) {
        Ok(text) => text,
        Err(why) => return refuse(StatusCode::BAD_REQUEST, why),
    };
    let dir = s.data_dir();
    let saved = tokio::task::spawn_blocking(move || {
        let mut inbox = Inbox::open(&dir)?;
        let reporter = inbox.reporter_of(&who);
        inbox.add(&reporter, &r, now(), &signed_in)
    })
    .await;
    match saved {
        Ok(Ok((id, new))) => {
            tracing::info!(%id, new, "feedback: report saved");
            let status = if new { StatusCode::CREATED } else { StatusCode::OK };
            (
                status,
                Json(Saved {
                    id,
                    state: State::Saved,
                }),
            )
                .into_response()
        }
        Ok(Err(InboxError::Wait(secs))) => {
            let mut resp = refuse(
                StatusCode::TOO_MANY_REQUESTS,
                "That's a lot of reports at once. Try again later.",
            );
            resp.headers_mut()
                .insert(header::RETRY_AFTER, secs.to_string().parse().expect("digits"));
            resp
        }
        Ok(Err(InboxError::SignedOut)) => refuse(StatusCode::UNAUTHORIZED, "missing or wrong bearer token"),
        Ok(Err(InboxError::Changed)) => refuse(
            StatusCode::CONFLICT,
            "This report was already saved as you first wrote it. Send again to save this version as a new report.",
        ),
        Ok(Err(InboxError::Full)) => refuse(
            StatusCode::INSUFFICIENT_STORAGE,
            "The node's report inbox is full. Tell the node's owner.",
        ),
        Ok(Err(e)) => {
            tracing::warn!("feedback: {e}");
            refuse(StatusCode::INTERNAL_SERVER_ERROR, NOT_SAVED)
        }
        Err(_) => refuse(StatusCode::INTERNAL_SERVER_ERROR, NOT_SAVED),
    }
}

/// `GET /v1/feedback/mine`: the caller's own reports and where each is.
pub async fn mine(
    AxState(s): AxState<Arc<NodeState>>,
    Extension(c): Extension<Caller>,
    headers: HeaderMap,
) -> Response {
    match s.token() {
        None => return refuse(StatusCode::FORBIDDEN, NO_TOKEN),
        Some(t) if t.chars().count() < MIN_TOKEN => return refuse(StatusCode::FORBIDDEN, short_token(&c)),
        Some(_) => {}
    }
    let Some(token) = header_token(&headers) else {
        return refuse(StatusCode::UNAUTHORIZED, IN_THE_HEADER);
    };
    let Some(who) = sender(&c) else {
        return refuse(StatusCode::FORBIDDEN, "Only the node's people send reports.");
    };
    // Still signed in as the same person when the inbox answers, as at a write: a read that waited
    // for the inbox while its sign-in ended shows nothing (Kimi review of fc1dca51, M2)
    let signed_in = {
        let (s, c) = (s.clone(), c.clone());
        move || same_person(&c, crate::accounts::caller(&s, Some(&token)).as_ref())
    };
    if !signed_in() {
        return refuse(StatusCode::UNAUTHORIZED, "missing or wrong bearer token");
    }
    let dir = s.data_dir();
    match tokio::task::spawn_blocking(move || {
        let list = match Inbox::open_existing(&dir)? {
            Some(inbox) => inbox.mine(&inbox.reporter_of(&who))?,
            None => Vec::new(),
        };
        if !signed_in() {
            return Err(InboxError::SignedOut);
        }
        Ok(list)
    })
    .await
    {
        Ok(Ok(list)) => Json(list).into_response(),
        Ok(Err(InboxError::SignedOut)) => refuse(StatusCode::UNAUTHORIZED, "missing or wrong bearer token"),
        Ok(Err(e)) => {
            tracing::warn!("feedback: {e}");
            refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The node couldn't read your reports. Try again.",
            )
        }
        Err(_) => refuse(
            StatusCode::INTERNAL_SERVER_ERROR,
            "The node couldn't read your reports. Try again.",
        ),
    }
}

/// A removed listener's reports go (People › Remove), read or not. Nothing to do (and no inbox
/// made) without one. How many went.
pub fn forget_listener(dir: &Path, key: &str) -> Result<usize, InboxError> {
    let Some(inbox) = Inbox::open_existing(dir)? else {
        return Ok(0);
    };
    let reporter = inbox.reporter_of(&Sender::Listener(key.trim_start_matches("pubky").to_string()));
    Ok(inbox
        .conn
        .execute("DELETE FROM report WHERE reporter = ?1", params![reporter])?)
}

/// Listeners' reports whose sender is no longer on the roster (`listed`, its keys) go: a People ›
/// Remove the inbox was too busy or broken for is finished here (Kimi review of fc1dca51, M1), and
/// so is a restore that brought back a roster without them. The owner's stay. How many went.
pub fn forget_unlisted(dir: &Path, listed: &[String]) -> Result<usize, InboxError> {
    let Some(inbox) = Inbox::open_existing(dir)? else {
        return Ok(0);
    };
    let keep: std::collections::HashSet<String> = listed
        .iter()
        .map(|k| inbox.reporter_of(&Sender::Listener(k.trim_start_matches("pubky").to_string())))
        .chain([OWNER.to_string()])
        .collect();
    let reporters: Vec<String> = inbox
        .conn
        .prepare("SELECT DISTINCT reporter FROM report")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    let mut gone = 0;
    for r in reporters.iter().filter(|r| !keep.contains(*r)) {
        gone += inbox
            .conn
            .execute("DELETE FROM report WHERE reporter = ?1", params![r])?;
    }
    Ok(gone)
}

/// Every hour, and once at start: expired reports go, and the reports of anyone no longer on the
/// roster, whether or not anyone sends or reads one. A roster that can't be read takes none.
pub fn start_housekeeping(s: &Arc<NodeState>) {
    let s = s.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(HOUR as u64));
        loop {
            tick.tick().await;
            let s = s.clone();
            let round = tokio::task::spawn_blocking(move || {
                let dir = s.data_dir();
                if let Err(e) = housekeeping(&dir, now()) {
                    tracing::warn!("feedback: expired reports not removed: {e}");
                }
                match crate::people::listed_keys(&s).map(|keys| forget_unlisted(&dir, &keys)) {
                    Some(Ok(0)) => {}
                    Some(Ok(n)) => tracing::info!(n, "feedback: reports of people no longer listed removed"),
                    Some(Err(e)) => {
                        tracing::warn!("feedback: reports of people no longer listed not removed: {e}")
                    }
                    None => {
                        tracing::warn!("feedback: the roster can't be read; no one's reports removed for it")
                    }
                }
            });
            let _ = round.await;
        }
    });
}

/// One round of [`start_housekeeping`]: nothing to do (and no inbox made) without one.
pub fn housekeeping(dir: &Path, now: i64) -> Result<usize, InboxError> {
    let Some(inbox) = Inbox::open_existing(dir)? else {
        return Ok(0);
    };
    let n = inbox.purge(now)?;
    if n > 0 {
        tracing::info!(n, "feedback: expired reports removed");
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new(text: &str) -> NewReport {
        NewReport {
            kind: Kind::Problem,
            text: text.into(),
            clean: text.into(),
            client_id: uuid::Uuid::new_v4().to_string(),
            details: None,
        }
    }

    /// Held Low 3 of the Claude review of 2d2f0a8a: the owner's key loading during a request is no
    /// sign-out; another listener, a guest or nobody is.
    #[test]
    fn the_same_person_is_who_the_sign_in_names_not_every_field() {
        let owner = |key: Option<&str>| Caller {
            role: Role::Owner,
            key: key.map(Into::into),
        };
        let listener = |key: &str| Caller {
            role: Role::Listener,
            key: Some(key.into()),
        };
        let guest = Caller {
            role: Role::Guest,
            key: None,
        };
        assert!(same_person(&owner(None), Some(&owner(Some("ownerkey")))));
        assert!(same_person(&listener("annkey"), Some(&listener("pubkyannkey"))));
        assert!(!same_person(&listener("annkey"), Some(&listener("benkey"))));
        assert!(!same_person(&listener("annkey"), None));
        assert!(!same_person(&owner(None), Some(&listener("annkey"))));
        assert!(!same_person(&listener("annkey"), Some(&owner(None))));
        assert!(!same_person(&guest, Some(&guest)));
    }

    #[test]
    fn tokens_are_taken_out_and_the_rest_stays() {
        let secrets = vec!["node-api-token-123".to_string(), "short".to_string()];
        assert_eq!(
            scrub(
                "a node-api-token-123 b pml_abcdefghijklmnop1234 c pmo_short d short",
                &secrets
            ),
            "a [token removed] b [token removed] c pmo_[token removed] d [token removed]"
        );
        assert_eq!(scrub("pmg_ABCDEFGHIJKLMNOPQR!", &[]), "[token removed]!");
        // Too short for a token's shape, and no secret: stays
        assert_eq!(scrub("c pmo_tiny d", &[]), "c pmo_tiny d");
        assert_eq!(scrub("ünïcode pml_ stays", &[]), "ünïcode pml_ stays");
        assert_eq!(clean(" a\r\nb\u{1b}[31m\u{2066}c\t "), "a\nb[31mc");
    }

    #[test]
    fn reports_go_once_they_are_past_keeping_even_with_nobody_asking() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(housekeeping(dir.path(), 0).unwrap(), 0);
        assert!(!dir.path().join(FILE).exists(), "no inbox made for nothing");
        let mut inbox = Inbox::open(dir.path()).unwrap();
        let t0 = 1_000_000_000;
        let (old, _) = inbox.add("a", &new("old"), t0, &|| true).unwrap();
        let (reviewed, _) = inbox
            .add("b", &new("reviewed"), t0 + 300 * DAY, &|| true)
            .unwrap();
        let (fresh, _) = inbox.add("c", &new("fresh"), t0 + 300 * DAY, &|| true).unwrap();
        assert!(inbox
            .receipt(&reviewed, State::Reviewed, "r1", t0 + 301 * DAY)
            .unwrap());
        // A year on, the first goes; 90 days after its review, the reviewed one
        assert_eq!(housekeeping(dir.path(), t0 + KEEP + 1).unwrap(), 1);
        assert_eq!(
            housekeeping(dir.path(), t0 + 301 * DAY + KEEP_REVIEWED + 1).unwrap(),
            1
        );
        let left: Vec<String> = inbox
            .pending(t0 + 302 * DAY + KEEP_REVIEWED)
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(left, [fresh]);
        assert!(!left.contains(&old));
    }

    #[test]
    fn a_full_inbox_refuses_and_drops_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut inbox = Inbox::open(dir.path()).unwrap();
        let t = 2_000_000_000;
        // Old enough for no rate limit, young enough to keep
        for i in 0..MAX_REPORTS {
            inbox
                .conn
                .execute(
                    "INSERT INTO report (id, client_id, reporter, kind, text, sent_at, state)
                     VALUES (?1, ?1, 'x', 'problem', 'x', ?2, 'saved')",
                    params![format!("fb_{i:016x}"), t - 2 * DAY],
                )
                .unwrap();
        }
        assert!(matches!(
            inbox.add("y", &new("one more"), t, &|| true),
            Err(InboxError::Full)
        ));
        assert_eq!(inbox.pending(t).unwrap().len(), MAX_REPORTS);
        // Bytes count too
        let dir = tempfile::tempdir().unwrap();
        let mut inbox = Inbox::open(dir.path()).unwrap();
        inbox
            .conn
            .execute(
                "INSERT INTO report (id, client_id, reporter, kind, text, sent_at, state)
                 VALUES ('fb_00000000000000aa', 'c', 'x', 'problem', ?1, ?2, 'saved')",
                params!["x".repeat(MAX_BYTES - 10), t - 2 * DAY],
            )
            .unwrap();
        assert!(matches!(
            inbox.add("y", &new("eleven chars"), t, &|| true),
            Err(InboxError::Full)
        ));
        assert!(inbox.add("y", &new("ten chars!"), t, &|| true).is_ok());
    }

    #[test]
    fn the_node_takes_a_bounded_number_of_reports_a_day() {
        let dir = tempfile::tempdir().unwrap();
        let mut inbox = Inbox::open(dir.path()).unwrap();
        let t = 2_000_000_000;
        for i in 0..NODE_PER_DAY {
            // Many people, spread over the day: no one's own limit applies
            inbox
                .add(&format!("p{i}"), &new("x"), t - DAY + 1 + i as i64 * 60, &|| true)
                .unwrap();
        }
        match inbox.add("someone-new", &new("x"), t, &|| true) {
            Err(InboxError::Wait(s)) => assert_eq!(s, 1, "the oldest leaves the day in a second"),
            other => panic!("{other:?}"),
        }
        // A person's day: 20, spread so the hour's 5 never apply
        let dir = tempfile::tempdir().unwrap();
        let mut inbox = Inbox::open(dir.path()).unwrap();
        for i in 0..PER_DAY {
            inbox
                .add("p", &new("x"), t - DAY + 1 + i as i64 * HOUR, &|| true)
                .unwrap();
        }
        assert!(matches!(
            inbox.add("p", &new("x"), t, &|| true),
            Err(InboxError::Wait(_))
        ));
    }

    #[test]
    fn a_sign_out_just_before_the_write_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut inbox = Inbox::open(dir.path()).unwrap();
        // Checked again inside the write, after any wait for the inbox's lock
        assert!(matches!(
            inbox.add("p", &new("late"), 1_000, &|| false),
            Err(InboxError::SignedOut)
        ));
        assert!(inbox.pending(1_000).unwrap().is_empty());
        assert!(inbox.add("p", &new("in time"), 1_000, &|| true).is_ok());
    }

    #[test]
    fn the_operator_never_makes_an_inbox() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Inbox::open_existing(dir.path()).unwrap().is_none());
        assert!(!dir.path().join(FILE).exists(), "no inbox made by reading");
        Inbox::open(dir.path()).unwrap();
        assert!(Inbox::open_existing(dir.path()).unwrap().is_some());
    }

    #[test]
    fn the_length_is_measured_before_tokens_come_out() {
        // Whether a report is taken never depends on a secret in it
        let token = "0123456789abcdefghij".to_string(); // 20 characters, 15 once taken out
        let at_most = format!("{token}{}", "x".repeat(MAX_TEXT - 20));
        assert!(prepare(&at_most, std::slice::from_ref(&token)).is_ok());
        let over = format!("{token}{}", "x".repeat(MAX_TEXT - 19));
        assert_eq!(prepare(&over, std::slice::from_ref(&token)), Err(TOO_LONG));
        assert_eq!(prepare(" a\r\nb ", &[]), Ok("a\nb".to_string()));
        assert_eq!(prepare(" \u{7} ", &[]), Err(EMPTY));
    }

    #[test]
    fn a_signed_out_sender_learns_nothing_from_the_inbox() {
        // Checked first in the write: a retry, an edit, a full window all answer "signed out"
        let dir = tempfile::tempdir().unwrap();
        let mut inbox = Inbox::open(dir.path()).unwrap();
        let first = new("first");
        inbox.add("p", &first, 1_000, &|| true).unwrap();
        assert!(matches!(
            inbox.add("p", &first, 1_001, &|| false),
            Err(InboxError::SignedOut)
        ));
        let edited = NewReport {
            text: "edited".into(),
            clean: "edited".into(),
            ..first
        };
        assert!(matches!(
            inbox.add("p", &edited, 1_001, &|| false),
            Err(InboxError::SignedOut)
        ));
        for i in 1..PER_HOUR {
            inbox
                .add("p", &new(&format!("n{i}")), 1_000 + i as i64, &|| true)
                .unwrap();
        }
        assert!(matches!(
            inbox.add("p", &new("over"), 1_100, &|| false),
            Err(InboxError::SignedOut)
        ));
    }

    #[test]
    fn a_short_token_is_taken_out_too() {
        let short = "k3y9z".to_string();
        assert_eq!(
            prepare("my token k3y9z here", std::slice::from_ref(&short)),
            Ok("my token [token removed] here".to_string())
        );
        assert_eq!(
            prepare("no secrets", &[String::new()]),
            Ok("no secrets".to_string())
        );
    }

    #[test]
    fn a_secret_inside_a_pasted_token_never_saves_the_token() {
        // The token shapes go first, then the secrets, longest first
        let pasted = "pml_Q7xaW2kPz9LmN4vB8rT1yHcJ6dFgS0EuIoKpZqXwVbM";
        assert_eq!(
            scrub(
                &format!("see {pasted} end"),
                &["a".to_string(), pasted.to_string()]
            ),
            "see [token removed] end"
        );
    }

    #[test]
    fn a_sign_out_during_the_checks_saves_nothing() {
        // Signed in at the first check, out by the write: both are asked
        let dir = tempfile::tempdir().unwrap();
        let mut inbox = Inbox::open(dir.path()).unwrap();
        let asked = std::cell::Cell::new(0);
        let still = || {
            asked.set(asked.get() + 1);
            asked.get() == 1
        };
        assert!(matches!(
            inbox.add("p", &new("late"), 1_000, &still),
            Err(InboxError::SignedOut)
        ));
        assert_eq!(asked.get(), 2);
        assert!(inbox.pending(1_000).unwrap().is_empty());
    }

    #[test]
    fn a_secret_with_a_token_shape_inside_goes_whole() {
        // An operator-chosen token that contains a node token's shape: none of it stays
        let cfg = "Hunter2-Hunter2-pmo_abcdefghijklmnopqrstu".to_string();
        assert_eq!(
            scrub(&format!("x {cfg} y"), std::slice::from_ref(&cfg)),
            "x [token removed] y"
        );
        assert_eq!(
            scrub(&format!("{cfg}{cfg}"), std::slice::from_ref(&cfg)),
            "[token removed]"
        );
    }

    #[test]
    fn a_reports_digest_goes_once_it_cannot_be_sent_again() {
        // It only tells a resend after a lost answer from an edit: gone once received, or after a
        // day, so the inbox never keeps a check on what was written before the tokens came out
        let dir = tempfile::tempdir().unwrap();
        let mut inbox = Inbox::open(dir.path()).unwrap();
        let digests = |inbox: &Inbox| -> i64 {
            inbox
                .conn
                .query_row("SELECT COUNT(*) FROM report WHERE digest IS NOT NULL", [], |r| {
                    r.get(0)
                })
                .unwrap()
        };
        let t = 2_000_000_000;
        let first = new("first");
        let (received, _) = inbox.add("p", &new("second"), t, &|| true).unwrap();
        inbox.add("p", &first, t, &|| true).unwrap();
        assert_eq!(digests(&inbox), 2);
        assert!(inbox.receipt(&received, State::Received, "r1", t + 1).unwrap());
        assert_eq!(digests(&inbox), 1, "received: no resend to tell apart");
        inbox.purge(t + DAY + 1).unwrap();
        assert_eq!(digests(&inbox), 0, "a day on: none kept");
        assert!(matches!(
            inbox.add("p", &first, t + DAY + 2, &|| true),
            Err(InboxError::Changed)
        ));
    }

    #[test]
    fn an_inbox_from_before_the_digest_opens_and_works() {
        let dir = tempfile::tempdir().unwrap();
        let old = Connection::open(dir.path().join(FILE)).unwrap();
        old.execute_batch(
            "CREATE TABLE report (id TEXT PRIMARY KEY, client_id TEXT NOT NULL, reporter TEXT NOT NULL,
                kind TEXT NOT NULL, text TEXT NOT NULL, details TEXT, sent_at INTEGER NOT NULL,
                state TEXT NOT NULL, received_at INTEGER, reviewed_at INTEGER, receipt TEXT,
                UNIQUE (reporter, client_id));
             CREATE TABLE meta (k TEXT PRIMARY KEY, v BLOB NOT NULL);
             INSERT INTO report (id, client_id, reporter, kind, text, sent_at, state)
                VALUES ('fb_00000000000000aa', 'c-old', 'p', 'problem', 'old', 1000, 'saved');",
        )
        .unwrap();
        drop(old);
        // Two opens at once: both see the column, neither fails
        let opened: Vec<_> = (0..2)
            .map(|_| {
                let d = dir.path().to_path_buf();
                std::thread::spawn(move || Inbox::open(&d).map(|_| ()))
            })
            .collect();
        for o in opened {
            o.join().unwrap().unwrap();
        }
        let mut inbox = Inbox::open(dir.path()).unwrap();
        let mut again = new("old");
        again.client_id = "c-old".into();
        // A report from before has no digest: sent again it is a new one (409), never a guess check
        assert!(matches!(
            inbox.add("p", &again, 2000, &|| true),
            Err(InboxError::Changed)
        ));
        assert!(inbox.add("p", &new("new"), 2000, &|| true).unwrap().1);
    }

    /// A resend after the window is turned away (signed in: 409; signed out): what it answers, and
    /// how many digests the inbox still holds afterwards.
    fn a_late_resend(signed_in: bool) -> (Result<(String, bool), InboxError>, i64) {
        let dir = tempfile::tempdir().unwrap();
        let mut inbox = Inbox::open(dir.path()).unwrap();
        let t = 2_000_000_000;
        let first = new("first");
        inbox.add("p", &first, t, &|| true).unwrap();
        let answer = inbox.add("p", &first, t + RESEND + 1, &|| signed_in);
        let kept = inbox
            .conn
            .query_row("SELECT COUNT(*) FROM report WHERE digest IS NOT NULL", [], |r| {
                r.get(0)
            })
            .unwrap();
        (answer, kept)
    }

    #[test]
    fn a_resend_turned_away_as_changed_still_forgets_old_digests() {
        // The expiry stays even when the report itself is turned away: it is not undone with the
        // report's own write
        let (answer, kept) = a_late_resend(true);
        assert!(matches!(answer, Err(InboxError::Changed)));
        assert_eq!(kept, 0);
    }

    #[test]
    fn a_signed_out_resend_still_forgets_old_digests() {
        let (answer, kept) = a_late_resend(false);
        assert!(matches!(answer, Err(InboxError::SignedOut)));
        assert_eq!(kept, 0);
    }

    #[test]
    fn what_the_inbox_lets_go_is_overwritten_in_its_file() {
        // A cleared digest or a purged report must not stay readable in the file's free space:
        // whoever copies the file has the inbox's key too. Short reports sit inside a page; the
        // longest spills onto overflow pages, which are freed another way
        let dir = tempfile::tempdir().unwrap();
        let mut inbox = Inbox::open(dir.path()).unwrap();
        let t = 2_000_000_000;
        let tag = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
        let segment = |n: usize| format!("報告の長い断片{tag}{n:04}。");
        let mut long = String::new();
        let mut n = 0;
        while long.chars().count() < MAX_TEXT {
            long.push_str(&segment(n));
            n += 1;
        }
        let long: String = long.chars().take(MAX_TEXT).collect();
        assert!(long.len() > 4096, "longer than a page: {}", long.len());
        // Found by its pieces: an overflowing text isn't in one run of bytes
        let segments: Vec<String> = (0..n)
            .map(segment)
            .filter(|s| long.contains(s.as_str()))
            .collect();
        let mut gone = Vec::new();
        for i in 0..13 {
            let text = if i == 12 {
                long.clone()
            } else {
                format!("purged-report-{i:02}-{}", uuid::Uuid::new_v4().simple())
            };
            let (id, _) = inbox.add(&format!("p{i}"), &new(&text), t, &|| true).unwrap();
            let digest: String = inbox
                .conn
                .query_row("SELECT digest FROM report WHERE id = ?1", params![id], |r| {
                    r.get(0)
                })
                .unwrap();
            let pieces = if i == 12 { segments.clone() } else { vec![text] };
            gone.push((id, pieces, digest));
        }
        let file = dir.path().join(FILE);
        let found = |pieces: &[String]| {
            let raw = std::fs::read(&file).unwrap();
            pieces
                .iter()
                .filter(|p| raw.windows(p.len()).any(|w| w == p.as_bytes()))
                .count()
        };
        // Control: the file holds what is kept (most of the long one: a piece can straddle pages)
        for (_, pieces, digest) in &gone {
            assert_eq!(found(std::slice::from_ref(digest)), 1);
            assert!(
                found(pieces) * 10 >= pieces.len() * 9,
                "control: the file holds the text"
            );
        }
        for (id, _, _) in &gone {
            assert!(inbox.receipt(id, State::Reviewed, "r1", t + 1).unwrap());
        }
        for (_, _, digest) in &gone {
            assert_eq!(
                found(std::slice::from_ref(digest)),
                0,
                "a cleared digest stays in the file"
            );
        }
        assert_eq!(inbox.purge(t + 2 + KEEP_REVIEWED).unwrap(), 13);
        for (_, pieces, _) in &gone {
            assert_eq!(found(pieces), 0, "a purged report stays in the file");
        }
    }

    /// Claude review of b015b991, Medium: a page the app shows that the inbox doesn't know turns
    /// every report sent from it with details into a 400 (and reloading lands on the same page).
    #[test]
    fn every_page_the_app_shows_is_a_view_a_report_may_name() {
        let js = include_str!("../static/app.js");
        let mut pages: Vec<&str> = js
            .match_indices("setView('")
            .filter_map(|(i, m)| js[i + m.len()..].split('\'').next())
            .collect();
        let nav = js
            .split_once("const NAV_VIEWS = [")
            .and_then(|(_, rest)| rest.split_once(']'))
            .expect("app.js lists its NAV_VIEWS")
            .0;
        pages.extend(nav.split(',').map(|v| v.trim().trim_matches('\'')));
        assert!(pages.len() > 20, "{pages:?}");
        let unknown: Vec<&&str> = pages.iter().filter(|v| !VIEWS.contains(v)).collect();
        assert!(unknown.is_empty(), "pages a report can't name: {unknown:?}");
    }
}
