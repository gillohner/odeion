//! Songs added on the phone, also kept on the person's homeserver (`/priv/pubky.music/files/`,
//! private) under the names a node gives them, so a node finds them there: a setting, off by
//! default, on by default only when the person's homeserver is the one installed with their Odeion
//! node (its `node.json` names that homeserver: `bundled_homeserver`).

use super::account::Account;
use super::{err, Phone};
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use pm_sources::SessionStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The person's choice on this phone (`on` / `off`; none: the default).
const KEY: &str = "phone.files_on_homeserver";
/// A song bigger than this is not copied from the phone.
const MAX_COPY: u64 = 300 * 1024 * 1024;

/// z32 without the `pubky` prefix.
fn z32(k: &str) -> &str {
    k.strip_prefix("pubky").filter(|x| x.len() == 52).unwrap_or(k)
}

/// The person's choice, if they made one.
pub fn chosen(p: &Phone) -> Option<bool> {
    match p.store.get(KEY).as_deref() {
        Some("on") => Some(true),
        Some("off") => Some(false),
        _ => None,
    }
}

/// Whether songs added here go to the homeserver: the person's choice, else on only when their
/// homeserver is their node's own.
pub fn on(p: &Phone) -> bool {
    chosen(p).unwrap_or_else(|| *p.bundled_here.lock().unwrap_or_else(|e| e.into_inner()) == Some(true))
}

/// Whether the person's homeserver is the one installed with their node: their node record's
/// `bundled_homeserver` against the homeserver their Pubky names. `None`: no node record, or not
/// known now.
pub async fn bundled_here(acct: &Account) -> Option<bool> {
    let record = acct.hs.read_node_record(&acct.session).await.ok()??;
    let Some(bundled) = record.bundled_homeserver else {
        return Some(false);
    };
    let theirs = acct
        .hs
        .pubky()
        .get_homeserver_of(&acct.session.public_key())
        .await
        .ok()??;
    Some(z32(&theirs.z32()) == z32(&bundled))
}

/// What a copy pass did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub copied: usize,
    pub left: usize,
}

/// What the phone knows of its copies: songs copied (or found there) by content hash, kept so a
/// song the person removed from their homeserver isn't copied again, and those the homeserver
/// refused for good, with why. `phone/<pubky>/files.json`.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Copies {
    #[serde(default)]
    pub copied: BTreeSet<String>,
    #[serde(default)]
    pub refused: BTreeMap<String, String>,
}

/// The copier's state while the app runs: one pass at a time, the homeserver's files as last
/// listed (content hashes), and when to try again after a failure.
#[derive(Default)]
pub struct Copier {
    running: AtomicBool,
    there: Mutex<Option<HashSet<String>>>,
    retry: Mutex<Option<(Instant, Duration)>>,
}

/// After a failed upload the next pass waits this long, doubled after each failure up to
/// [`BACKOFF_MAX`].
const BACKOFF: Duration = Duration::from_secs(60);
const BACKOFF_MAX: Duration = Duration::from_secs(6 * 60 * 60);

/// How long one song's upload may take: 2 minutes, and 20 more seconds per MB (50 kB/s).
fn upload_for(bytes: u64) -> Duration {
    Duration::from_secs(120 + bytes / 50_000)
}

/// The content hash a file name carries (`<16 hex>-<name>`).
fn hash_of(name: &str) -> Option<&str> {
    pm_appdata::files::split_remote_name(name).map(|(h, _)| h)
}

fn copies_path(p: &Phone) -> std::path::PathBuf {
    p.dir().join("files.json")
}

fn load_copies(p: &Phone) -> Copies {
    std::fs::read(copies_path(p))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_copies(p: &Phone, c: &Copies) {
    let path = copies_path(p);
    let tmp = path.with_extension("json.tmp");
    let ok = serde_json::to_vec(c)
        .ok()
        .is_some_and(|b| std::fs::write(&tmp, b).is_ok() && std::fs::rename(&tmp, &path).is_ok());
    if !ok {
        tracing::warn!("phone mode: what was copied to the homeserver could not be saved");
    }
}

/// Starts a copy pass on its own task, unless one is running: it can take long (big songs, a slow
/// uplink) and the vault and data syncs don't wait for it.
pub fn start(p: &Arc<Phone>, acct: &Arc<Account>) {
    if p.copier.running.swap(true, Ordering::SeqCst) {
        return;
    }
    let (p, acct) = (p.clone(), acct.clone());
    tokio::spawn(async move {
        struct Done<'a>(&'a AtomicBool);
        impl Drop for Done<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _done = Done(&p.copier.running);
        let r = copy(&p, &acct).await;
        if r != Report::default() {
            tracing::info!(?r, "phone mode: songs copied to the homeserver");
        }
    });
}

/// Copies the songs added here that the homeserver doesn't have (by content hash), one at a time,
/// when the setting is on. A song copied before, or refused for good, isn't tried again; after
/// any other failure the pass stops and the next one waits (doubling, up to 6 hours).
pub async fn copy(p: &Phone, acct: &Account) -> Report {
    let known = bundled_here(acct).await;
    if known.is_some() {
        *p.bundled_here.lock().unwrap_or_else(|e| e.into_inner()) = known;
    }
    let mut r = Report::default();
    if !on(p) {
        return r;
    }
    if let Some((at, _)) = *p.copier.retry.lock().unwrap_or_else(|e| e.into_inner()) {
        if Instant::now() < at {
            return r;
        }
    }
    let there: HashSet<String> = match acct.hs.list_private_file_names(&acct.session).await {
        Ok(names) => names
            .iter()
            .filter_map(|n| hash_of(n))
            .map(str::to_string)
            .collect(),
        Err(e) => {
            tracing::warn!("phone mode: the homeserver's files can't be listed now: {e}");
            return r;
        }
    };
    *p.copier.there.lock().unwrap_or_else(|e| e.into_inner()) = Some(there.clone());
    let mut copies = load_copies(p);
    for s in p.songs.list() {
        let Some(h) = hash_of(&s.file).map(str::to_string) else {
            continue;
        };
        if there.contains(&h) {
            if copies.copied.insert(h) {
                save_copies(p, &copies);
            }
            continue;
        }
        // Copied before and since removed there by the person, or refused for good: left as is
        if copies.copied.contains(&h) || copies.refused.contains_key(&h) {
            continue;
        }
        if s.bytes > MAX_COPY {
            r.left += 1;
            continue;
        }
        let path = format!("{}{}", pm_appdata::files::FILES_DIR, s.file);
        let sent = tokio::time::timeout(
            upload_for(s.bytes),
            acct.hs
                .upload_private_file(&acct.session, &path, &p.songs.path_of(&s)),
        )
        .await;
        match sent {
            Ok(Ok(())) => {
                r.copied += 1;
                copies.copied.insert(h.clone());
                save_copies(p, &copies);
                if let Some(t) = p.copier.there.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                    t.insert(h);
                }
                *p.copier.retry.lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
            Ok(Err(e)) if pm_pubky::homeserver::refused_for_good(&e) => {
                tracing::warn!("phone mode: the homeserver refused a song: {e}");
                copies.refused.insert(h, e.to_string());
                save_copies(p, &copies);
                r.left += 1;
            }
            failed => {
                let why = match failed {
                    Ok(Err(e)) => e.to_string(),
                    _ => "it took too long".into(),
                };
                tracing::warn!("phone mode: a song waits to be copied to the homeserver: {why}");
                let mut retry = p.copier.retry.lock().unwrap_or_else(|e| e.into_inner());
                let wait = retry.map(|(_, w)| (w * 2).min(BACKOFF_MAX)).unwrap_or(BACKOFF);
                *retry = Some((Instant::now() + wait, wait));
                r.left += 1;
                break;
            }
        }
    }
    r
}

/// `GET /v1/phone/storage`: whether added songs go to the homeserver, why, and how far.
pub(crate) async fn status(State(p): State<Arc<Phone>>) -> Json<Value> {
    Json(view(&p))
}

/// From what the phone knows (the last pass), without asking the homeserver.
fn view(p: &Phone) -> Value {
    let songs = p.songs.list();
    let copies = load_copies(p);
    let there = p
        .copier
        .there
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_default();
    let (mut uploaded, mut cant) = (0, 0);
    for s in &songs {
        match hash_of(&s.file) {
            Some(h) if there.contains(h) || copies.copied.contains(h) => uploaded += 1,
            Some(h) if copies.refused.contains_key(h) => cant += 1,
            _ if s.bytes > MAX_COPY => cant += 1,
            _ => {}
        }
    }
    json!({
        "on": on(p),
        "chosen": chosen(p),
        "bundled": *p.bundled_here.lock().unwrap_or_else(|e| e.into_inner()),
        "uploaded": uploaded,
        "cant": cant,
        "total": songs.len(),
    })
}

#[derive(Deserialize)]
pub(crate) struct Choice {
    /// `null`: back to the default.
    on: Option<bool>,
}

/// `PUT /v1/phone/storage {on}`.
pub(crate) async fn choose(State(p): State<Arc<Phone>>, Json(c): Json<Choice>) -> Response {
    let v = c.on.map(|on| if on { "on" } else { "off" });
    if let Err(e) = p.store.put(KEY, v) {
        return err(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    p.sync_soon();
    Json(view(&p)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_compare_without_the_pubky_prefix() {
        let k = "8pinxxgqs41n4aididenw5apqp1urfmzdztr8jt4abrkdn435ewo";
        assert_eq!(z32(&format!("pubky{k}")), k);
        assert_eq!(z32(k), k);
    }
}
