//! TIDAL and Qobuz for Odeion, in one place for the node (`pm-api`) and the app's phone mode
//! (`pm-offline`'s phone backend): one copy of each provider.
//!
//! What lives here is what both sides do the same way: the sign-in steps, the catalogue calls and
//! their mapping to [`types`], stream URLs and the readers that fetch them. What differs is behind
//! small stores each side implements:
//!
//! * [`SessionStore`]: where one person's sign-in state for a service is kept. The node keeps it in
//!   its settings per person (`Who`), the phone in a file in the app's data.
//! * [`qobuz::AppCache`]: where Qobuz's web-player app values are kept once found (the node: for
//!   everyone on it; the phone: for its one person).
//!
//! The node's HTTP handlers, its per-person rules, the vault and the search cache stay in
//! `pm-api`; nothing here knows about them.

mod client_mask;
pub mod dash;
pub mod hls;
pub mod pages;
pub mod qobuz;
pub mod releases;
pub mod search;
pub mod segmented;
pub mod tidal;
pub mod types;

pub use reqwest::StatusCode;

/// A provider call that did not work: the HTTP status the node answers with and a sentence for
/// people (never a token or a URL with one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceError {
    pub status: StatusCode,
    pub message: String,
}

impl SourceError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
    pub fn not_found(what: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, what)
    }
    pub fn bad_request(what: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, what)
    }
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SourceError {}

/// Where one person's sign-in state for a service is kept, as JSON text under a key
/// (`tidal.unofficial`, `qobuz.unofficial`). The node implements it over its settings for the
/// person asking; the phone over a file.
pub trait SessionStore: Send + Sync {
    /// The value under `key`, if any.
    fn get(&self, key: &str) -> Option<String>;
    /// Read, change and write `key` as one step (the node holds its library lock across it, so a
    /// renewal never writes over a sign-in made meanwhile). `f` gets the value there now and
    /// answers the value to write, or `None` to leave it as it is.
    fn update(&self, key: &str, f: &mut dyn FnMut(Option<String>) -> Option<String>);
}

/// A [`SessionStore`] in memory (tests, and a phone that has nothing saved yet).
#[derive(Default)]
pub struct MemoryStore(std::sync::Mutex<std::collections::HashMap<String, String>>);

impl SessionStore for MemoryStore {
    fn get(&self, key: &str) -> Option<String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).get(key).cloned()
    }

    fn update(&self, key: &str, f: &mut dyn FnMut(Option<String>) -> Option<String>) {
        let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = f(m.get(key).cloned()) {
            m.insert(key.to_string(), v);
        }
    }
}

/// Seconds since the Unix epoch.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// A urlencoded form body (reqwest is built without its form feature).
pub fn form(pairs: &[(&str, &str)]) -> String {
    let mut ser = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        ser.append_pair(k, v);
    }
    ser.finish()
}

/// `n thing` / `n things`.
pub fn plural(n: u64, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}
