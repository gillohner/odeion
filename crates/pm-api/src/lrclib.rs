//! Lyrics from LRCLIB (lrclib.net, open and keyless) for songs whose files carry none and for
//! streamed songs (UX review, next tier). Synced (LRC) lines when it has them, else plain text.
//! Answers, a "none" too, are kept on the node a week so a song is asked about once.

use crate::state::NodeState;
use crate::types::Lyrics;
use crate::{ApiError, ApiResult};
use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use std::sync::{Arc, OnceLock, RwLock};

const KEEP_SECS: i64 = 7 * 86_400;
/// The most answers kept; the oldest go first.
const KEEP_ROWS: usize = 20_000;
/// The longest title, artist or album asked about (bytes). Anyone who may read the library can
/// ask, and each new question is a row and a request to LRCLIB under the node's contact.
const FIELD_MAX: usize = 300;
const PREFIX: &str = "lyrics.lrclib.";

/// Whether a song's fields are something to ask about: both names there, none too long.
fn askable(title: &str, artist: &str, album: Option<&str>) -> bool {
    !title.is_empty()
        && !artist.is_empty()
        && title.len() <= FIELD_MAX
        && artist.len() <= FIELD_MAX
        && album.is_none_or(|a| a.len() <= FIELD_MAX)
}

/// Old answers go, at most once an hour: rows older than [`KEEP_SECS`] (they would be asked
/// again anyway), then all but the [`KEEP_ROWS`] newest.
fn prune(s: &NodeState) {
    static LAST: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
    let now = pm_library::db::now_unix();
    let last = LAST.load(std::sync::atomic::Ordering::Relaxed);
    if now - last < 3600
        || LAST
            .compare_exchange(
                last,
                now,
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_err()
    {
        return;
    }
    let lib = s.library.lock().expect("library poisoned");
    if let Err(e) = lib.prune_stamped_settings(PREFIX, now - KEEP_SECS, KEEP_ROWS) {
        tracing::warn!(error = %e, "lyrics answers kept: not pruned");
    }
}

fn base() -> &'static RwLock<String> {
    static B: OnceLock<RwLock<String>> = OnceLock::new();
    B.get_or_init(|| RwLock::new("https://lrclib.net".into()))
}

/// Tests point the lookups at a mock.
pub fn set_base(url: &str) {
    *base().write().expect("lrclib base") = url.trim_end_matches('/').to_string();
}

fn key(title: &str, artist: &str, duration_s: u64) -> String {
    let n = |x: &str| x.trim().to_lowercase();
    format!("{PREFIX}{}|{}|{}", n(artist), n(title), duration_s)
}

/// LRCLIB's lyrics for a song: `(text, synced)`, or none. Cached either way.
pub(crate) async fn find(
    s: &NodeState,
    title: &str,
    artist: &str,
    album: Option<&str>,
    duration_ms: Option<u64>,
) -> Option<(String, bool)> {
    let title = title.trim();
    let artist = artist.trim();
    // Outside services hear who asks (the contact in the User-Agent); none set, none asked
    let album = album.map(str::trim).filter(|a| !a.is_empty());
    if !askable(title, artist, album) || s.contact().is_empty() {
        return None;
    }
    let secs = duration_ms.map(|d| (d + 500) / 1000).unwrap_or(0);
    let k = key(title, artist, secs);
    if let Some(v) = s
        .library
        .lock()
        .expect("library poisoned")
        .get_setting(&k)
        .ok()
        .flatten()
    {
        if let Ok(c) = serde_json::from_str::<serde_json::Value>(&v) {
            if pm_library::db::now_unix() - c["at"].as_i64().unwrap_or(0) < KEEP_SECS {
                return c["text"]
                    .as_str()
                    .map(|t| (t.to_string(), c["synced"].as_bool().unwrap_or(false)));
            }
        }
    }
    let mut q: Vec<(&str, String)> = vec![("track_name", title.into()), ("artist_name", artist.into())];
    if let Some(a) = album {
        q.push(("album_name", a.into()));
    }
    if secs > 0 {
        q.push(("duration", secs.to_string()));
    }
    let url = format!("{}/api/get", base().read().expect("lrclib base"));
    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(6),
        s.http_client().get(&url).query(&q).send(),
    )
    .await
    .ok()?
    .ok()?;
    let found = if resp.status().is_success() {
        let v: serde_json::Value = resp.json().await.ok()?;
        let synced = v["syncedLyrics"].as_str().filter(|t| !t.trim().is_empty());
        let plain = v["plainLyrics"].as_str().filter(|t| !t.trim().is_empty());
        synced
            .map(|t| (t.to_string(), true))
            .or_else(|| plain.map(|t| (t.to_string(), false)))
    } else if resp.status().as_u16() == 404 {
        None
    } else {
        // Busy or down: ask again later, keep nothing
        return None;
    };
    let _ = s.library.lock().expect("library poisoned").set_setting(
        &k,
        &serde_json::json!({
            "at": pm_library::db::now_unix(),
            "text": found.as_ref().map(|f| f.0.clone()),
            "synced": found.as_ref().is_some_and(|f| f.1),
        })
        .to_string(),
    );
    prune(s);
    found
}

#[derive(Debug, Deserialize)]
pub struct LyricsQuery {
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

/// `GET /v1/lyrics?title&artist&album&duration_ms`: lyrics for a streamed song.
pub(crate) async fn get(State(s): State<Arc<NodeState>>, Query(q): Query<LyricsQuery>) -> ApiResult<Lyrics> {
    let (text, _) = find(&s, &q.title, &q.artist, q.album.as_deref(), q.duration_ms)
        .await
        .ok_or_else(|| ApiError::not_found("No lyrics found for this song"))?;
    let (lines, synced) = pm_library::lyrics::parse(&text);
    Ok(Json(Lyrics {
        track_id: 0,
        source: "lrclib".into(),
        synced,
        text,
        lines,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_songs_with_names_of_a_sane_length_are_asked_about() {
        assert!(askable("Song", "Artist", None));
        assert!(askable("Song", "Artist", Some("Album")));
        assert!(!askable("", "Artist", None));
        assert!(!askable("Song", "", None));
        let long = "x".repeat(FIELD_MAX + 1);
        assert!(!askable(&long, "Artist", None));
        assert!(!askable("Song", &long, None));
        assert!(!askable("Song", "Artist", Some(&long)));
        assert!(askable(&"x".repeat(FIELD_MAX), "Artist", None));
    }
}
