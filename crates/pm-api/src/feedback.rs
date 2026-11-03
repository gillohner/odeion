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
use axum::extract::{Query, State as AxState};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use futures_util::StreamExt;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
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
    /// A receipt that can't be: a state going back, a malformed reference.
    Invalid(&'static str),
    Store(String),
}

impl std::fmt::Display for InboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InboxError::Wait(s) => write!(f, "too many reports; try again in {s} s"),
            InboxError::Full => f.write_str("the inbox is full"),
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
    UNIQUE (reporter, client_id)
);
CREATE INDEX IF NOT EXISTS report_sent ON report (sent_at);";

/// The node's private inbox.
pub struct Inbox {
    conn: Connection,
}

impl Inbox {
    /// Open (or make) the inbox in `dir`.
    pub fn open(dir: &Path) -> Result<Inbox, InboxError> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(FILE);
        // Made for the node's user alone, before SQLite opens it
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
            {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        let conn = Connection::open(&path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        Ok(Inbox { conn })
    }

    /// Drop what is past keeping: reviewed reports after 90 days, any after a year.
    pub fn purge(&self, now: i64) -> Result<usize, InboxError> {
        Ok(self.conn.execute(
            "DELETE FROM report WHERE sent_at < ?1 OR (reviewed_at IS NOT NULL AND reviewed_at < ?2)",
            params![now - KEEP, now - KEEP_REVIEWED],
        )?)
    }

    /// Save a report. `Ok((id, false))` for one sent before with the same `client_id`.
    fn add(&mut self, reporter: &str, r: &NewReport, now: i64) -> Result<(String, bool), InboxError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM report WHERE sent_at < ?1 OR (reviewed_at IS NOT NULL AND reviewed_at < ?2)",
            params![now - KEEP, now - KEEP_REVIEWED],
        )?;
        let before: Option<String> = tx
            .query_row(
                "SELECT id FROM report WHERE reporter = ?1 AND client_id = ?2",
                params![reporter, r.client_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = before {
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
        let details = r
            .details
            .as_ref()
            .filter(|d| **d != Details::default())
            .map(|d| serde_json::to_string(d).expect("serializes"));
        let (count, bytes): (i64, i64) = tx.query_row(
            "SELECT COUNT(*), COALESCE(SUM(LENGTH(CAST(text AS BLOB)) + COALESCE(LENGTH(details), 0)), 0) FROM report",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let size = r.text.len() + details.as_ref().map_or(0, String::len);
        if count as usize >= MAX_REPORTS || bytes as usize + size > MAX_BYTES {
            return Err(InboxError::Full);
        }
        let id = format!("fb_{}", hex::encode(rand::random::<[u8; 8]>()));
        tx.execute(
            "INSERT INTO report (id, client_id, reporter, kind, text, details, sent_at, state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'saved')",
            params![
                id,
                r.client_id,
                reporter,
                serde_json::to_value(r.kind).expect("serializes").as_str(),
                r.text,
                details,
                now
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
    pub fn receipt(&self, id: &str, state: State, reference: &str, now: i64) -> Result<bool, InboxError> {
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
        let now_state: Option<(String, Option<String>)> = self
            .conn
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
        self.conn.execute(
            &format!("UPDATE report SET state = ?1, receipt = ?2, {column} = ?3 WHERE id = ?4"),
            params![state.as_str(), reference, now, id],
        )?;
        Ok(true)
    }
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

/// Who sent a report, as the inbox keeps it: the owner, or a listener's key hashed (so the inbox
/// never holds anyone's key).
const OWNER: &str = "owner";

fn reporter(c: &Caller) -> Option<String> {
    match (c.role, c.key.as_deref()) {
        (Role::Owner, _) => Some(OWNER.into()),
        (Role::Listener, Some(k)) => {
            let mut h = Sha256::new();
            h.update(b"pm-feedback-reporter:");
            h.update(k.trim_start_matches("pubky").as_bytes());
            Some(hex::encode(h.finalize()))
        }
        _ => None,
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
    let mut t = text.to_string();
    for s in secrets.iter().filter(|s| s.len() >= 8) {
        t = t.replace(s.as_str(), REMOVED);
    }
    let token_char = |b: u8| b.is_ascii_alphanumeric() || b == b'-' || b == b'_';
    let mut out = String::with_capacity(t.len());
    let mut rest = t.as_str();
    while let Some(at) = TOKEN_PREFIXES.iter().filter_map(|p| rest.find(p)).min() {
        let tail = &rest[at + 4..];
        let run = tail.bytes().take_while(|&b| token_char(b)).count();
        if run >= 16 {
            out.push_str(&rest[..at]);
            out.push_str(REMOVED);
            rest = &tail[run..];
        } else {
            out.push_str(&rest[..at + 4]);
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

fn refuse(status: StatusCode, msg: &str) -> Response {
    ApiError::new(status, msg).into_response()
}

const NO_TOKEN: &str =
    "Reports need the node's API token set: without one the node can't tell who sent them.";
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
    Query(query): Query<HashMap<String, String>>,
    req: axum::extract::Request,
) -> Response {
    // Without a token every caller is the owner: no telling who sent a report
    let Some(api_token) = s.token() else {
        return refuse(StatusCode::FORBIDDEN, NO_TOKEN);
    };
    let Some(who) = reporter(&c) else {
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
    // Still signed in as the same person once the body is here
    let token = crate::bearer(&headers, &query);
    if crate::accounts::caller(&s, token.as_deref()).as_ref() != Some(&c) {
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
    let text = clean(&r.text);
    let chars = text.chars().count();
    if chars == 0 {
        return refuse(StatusCode::BAD_REQUEST, "Describe the problem first.");
    }
    if chars > MAX_TEXT {
        return refuse(
            StatusCode::BAD_REQUEST,
            "That report is too long: at most 4000 characters.",
        );
    }
    let secrets: Vec<String> = [Some(api_token), token].into_iter().flatten().collect();
    r.text = scrub(&text, &secrets);
    let dir = s.data_dir();
    let saved = tokio::task::spawn_blocking(move || Inbox::open(&dir)?.add(&who, &r, now())).await;
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
pub async fn mine(AxState(s): AxState<Arc<NodeState>>, Extension(c): Extension<Caller>) -> Response {
    if s.token().is_none() {
        return refuse(StatusCode::FORBIDDEN, NO_TOKEN);
    }
    let Some(who) = reporter(&c) else {
        return refuse(StatusCode::FORBIDDEN, "Only the node's people send reports.");
    };
    let dir = s.data_dir();
    match tokio::task::spawn_blocking(move || {
        if !dir.join(FILE).exists() {
            return Ok(Vec::new());
        }
        Inbox::open(&dir)?.mine(&who)
    })
    .await
    {
        Ok(Ok(list)) => Json(list).into_response(),
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

/// Drop expired reports every hour, whether or not anyone sends or reads one.
pub fn start_housekeeping(s: &Arc<NodeState>) {
    let dir = s.data_dir();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(HOUR as u64));
        loop {
            tick.tick().await;
            let dir = dir.clone();
            let _ = tokio::task::spawn_blocking(move || housekeeping(&dir, now())).await;
        }
    });
}

/// One round of [`start_housekeeping`]: nothing to do (and no inbox made) without one.
pub fn housekeeping(dir: &Path, now: i64) -> Result<usize, InboxError> {
    if !dir.join(FILE).exists() {
        return Ok(0);
    }
    let n = Inbox::open(dir)?.purge(now)?;
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
            client_id: uuid::Uuid::new_v4().to_string(),
            details: None,
        }
    }

    #[test]
    fn tokens_are_taken_out_and_the_rest_stays() {
        let secrets = vec!["node-api-token-123".to_string(), "short".to_string()];
        assert_eq!(
            scrub(
                "a node-api-token-123 b pml_abcdefghijklmnop1234 c pmo_short d short",
                &secrets
            ),
            "a [token removed] b [token removed] c pmo_short d short"
        );
        assert_eq!(scrub("pmg_ABCDEFGHIJKLMNOPQR!", &[]), "[token removed]!");
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
        let (old, _) = inbox.add("a", &new("old"), t0).unwrap();
        let (reviewed, _) = inbox.add("b", &new("reviewed"), t0 + 300 * DAY).unwrap();
        let (fresh, _) = inbox.add("c", &new("fresh"), t0 + 300 * DAY).unwrap();
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
            inbox.add("y", &new("one more"), t),
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
            inbox.add("y", &new("eleven chars"), t),
            Err(InboxError::Full)
        ));
        assert!(inbox.add("y", &new("ten chars!"), t).is_ok());
    }

    #[test]
    fn the_node_takes_a_bounded_number_of_reports_a_day() {
        let dir = tempfile::tempdir().unwrap();
        let mut inbox = Inbox::open(dir.path()).unwrap();
        let t = 2_000_000_000;
        for i in 0..NODE_PER_DAY {
            // Many people, spread over the day: no one's own limit applies
            inbox
                .add(&format!("p{i}"), &new("x"), t - DAY + 1 + i as i64 * 60)
                .unwrap();
        }
        match inbox.add("someone-new", &new("x"), t) {
            Err(InboxError::Wait(s)) => assert_eq!(s, 1, "the oldest leaves the day in a second"),
            other => panic!("{other:?}"),
        }
        // A person's day: 20, spread so the hour's 5 never apply
        let dir = tempfile::tempdir().unwrap();
        let mut inbox = Inbox::open(dir.path()).unwrap();
        for i in 0..PER_DAY {
            inbox.add("p", &new("x"), t - DAY + 1 + i as i64 * HOUR).unwrap();
        }
        assert!(matches!(inbox.add("p", &new("x"), t), Err(InboxError::Wait(_))));
    }
}
