//! Songs added on the phone, also kept on the person's homeserver (`/priv/pubky.music/files/`,
//! private) under the names a node gives them, so a node finds them there: a setting, off by
//! default, on by default only when the person's homeserver is the one installed with their Odeion
//! node (its `node.json` names that homeserver: `bundled_homeserver`).

use super::account::Account;
use super::{err, Phone};
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use pm_pubky::homeserver::HomeserverError;
use pm_sources::SessionStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
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
    /// Asked for again while a pass ran: one more pass after it.
    again: AtomicBool,
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
        // Songs added meanwhile get the next pass
        p.copier.again.store(true, Ordering::SeqCst);
        return;
    }
    let (weak, acct) = (Arc::downgrade(p), acct.clone());
    tokio::spawn(async move {
        // Done (or phone mode left): another pass may start
        struct Done(Weak<Phone>);
        impl Drop for Done {
            fn drop(&mut self) {
                if let Some(p) = self.0.upgrade() {
                    p.copier.running.store(false, Ordering::SeqCst);
                }
            }
        }
        let _done = Done(weak.clone());
        loop {
            if let Some(p) = weak.upgrade() {
                p.copier.again.store(false, Ordering::SeqCst);
            }
            let known = bundled_here(&acct).await;
            let r = match weak.upgrade() {
                Some(p) => {
                    if known.is_some() {
                        *p.bundled_here.lock().unwrap_or_else(|e| e.into_inner()) = known;
                    }
                    drop(p);
                    copy_while(&weak, &*acct).await
                }
                None => break,
            };
            if r != Report::default() {
                tracing::info!(?r, "phone mode: songs copied to the homeserver");
            }
            if !weak
                .upgrade()
                .is_some_and(|p| p.copier.again.load(Ordering::SeqCst))
            {
                break;
            }
        }
    });
}

/// Where songs are copied to: the person's homeserver ([`Account`]), or a stand-in in tests.
pub trait Remote: Sync {
    /// The names of the files there (complete, or an error).
    fn names(&self) -> impl std::future::Future<Output = Result<Vec<String>, HomeserverError>> + Send;
    /// Puts the file at `local` at `path`, read as it goes.
    fn upload(
        &self,
        path: &str,
        local: &std::path::Path,
    ) -> impl std::future::Future<Output = Result<(), HomeserverError>> + Send;
}

impl Remote for Account {
    async fn names(&self) -> Result<Vec<String>, HomeserverError> {
        self.hs.list_private_file_names(&self.session).await
    }
    async fn upload(&self, path: &str, local: &std::path::Path) -> Result<(), HomeserverError> {
        self.hs.upload_private_file(&self.session, path, local).await
    }
}

/// Copies the songs added here that the homeserver doesn't have (by content hash), one at a time,
/// when the setting is on. A song copied before, or refused for good, isn't tried again; one that
/// takes too long waits for the next pass and the others go on; after any other failure the pass
/// stops and the next one waits (doubling, up to 6 hours).
pub async fn copy(p: &Arc<Phone>, acct: &Account) -> Report {
    let known = bundled_here(acct).await;
    if known.is_some() {
        *p.bundled_here.lock().unwrap_or_else(|e| e.into_inner()) = known;
    }
    copy_while(&Arc::downgrade(p), acct).await
}

/// How long the next pass waits after a failure, given the last wait.
fn next_wait(last: Option<Duration>) -> Duration {
    last.map(|w| (w * 2).min(BACKOFF_MAX)).unwrap_or(BACKOFF)
}

/// [`copy`] with `remote`, holding phone mode only between uploads: leaving it, or turning the
/// setting off, ends the pass before the next song.
pub async fn copy_while(weak: &Weak<Phone>, remote: &impl Remote) -> Report {
    let mut r = Report::default();
    let Some(p) = weak.upgrade() else { return r };
    if !on(&p) {
        return r;
    }
    if let Some((at, _)) = *p.copier.retry.lock().unwrap_or_else(|e| e.into_inner()) {
        if Instant::now() < at {
            return r;
        }
    }
    // The songs that may need copying; the homeserver is asked only when there are some
    let copies = load_copies(&p);
    let todo: Vec<_> = p
        .songs
        .list()
        .into_iter()
        .filter_map(|s| {
            let h = hash_of(&s.file)?.to_string();
            (!copies.copied.contains(&h) && !copies.refused.contains_key(&h)).then_some((s, h))
        })
        .collect();
    if todo.is_empty() {
        return r;
    }
    drop(p);
    let there: HashSet<String> = match remote.names().await {
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
    let Some(p) = weak.upgrade() else { return r };
    *p.copier.there.lock().unwrap_or_else(|e| e.into_inner()) = Some(there.clone());
    drop(p);
    for (s, h) in todo {
        let Some(p) = weak.upgrade() else { break };
        // Turned off meanwhile: no more uploads
        if !on(&p) {
            break;
        }
        if there.contains(&h) {
            let mut copies = load_copies(&p);
            if copies.copied.insert(h) {
                save_copies(&p, &copies);
            }
            continue;
        }
        let local = p.songs.path_of(&s);
        // Too big, or no longer on the phone: the others go on
        if s.bytes > MAX_COPY || !local.is_file() {
            r.left += 1;
            continue;
        }
        drop(p);
        let path = format!("{}{}", pm_appdata::files::FILES_DIR, s.file);
        let sent = tokio::time::timeout(upload_for(s.bytes), remote.upload(&path, &local)).await;
        let Some(p) = weak.upgrade() else { break };
        match sent {
            Ok(Ok(())) => {
                r.copied += 1;
                let mut copies = load_copies(&p);
                copies.copied.insert(h.clone());
                save_copies(&p, &copies);
                if let Some(t) = p.copier.there.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                    t.insert(h);
                }
                *p.copier.retry.lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
            Ok(Err(e)) if pm_pubky::homeserver::refused_for_good(&e) => {
                tracing::warn!("phone mode: the homeserver refused a song: {e}");
                let mut copies = load_copies(&p);
                copies.refused.insert(h, e.to_string());
                save_copies(&p, &copies);
                r.left += 1;
            }
            // Too slow for this one: it waits for the next pass, the others go on
            Err(_) => {
                tracing::warn!("phone mode: a song took too long to copy to the homeserver");
                r.left += 1;
            }
            Ok(Err(e)) => {
                tracing::warn!("phone mode: a song waits to be copied to the homeserver: {e}");
                let mut retry = p.copier.retry.lock().unwrap_or_else(|e| e.into_inner());
                let wait = next_wait(retry.map(|(_, w)| w));
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
    // A new choice tries the songs the homeserver refused again (it may have room now)
    let mut copies = load_copies(&p);
    if !copies.refused.is_empty() {
        copies.refused.clear();
        save_copies(&p, &copies);
    }
    *p.copier.retry.lock().unwrap_or_else(|e| e.into_inner()) = None;
    p.sync_soon();
    Json(view(&p)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// A homeserver stand-in: the names it lists, what was uploaded, the answers to give.
    #[derive(Default)]
    struct Fake {
        names: Vec<String>,
        listed: AtomicUsize,
        uploaded: Mutex<Vec<String>>,
        /// Status codes to answer the next uploads with (none: taken); 0: a transport error.
        answers: Mutex<std::collections::VecDeque<u16>>,
        /// Called at each upload (the person turning the setting off, say).
        during: Option<Box<dyn Fn() + Send + Sync>>,
    }

    impl Remote for Fake {
        async fn names(&self) -> Result<Vec<String>, HomeserverError> {
            self.listed.fetch_add(1, Ordering::SeqCst);
            Ok(self.names.clone())
        }
        async fn upload(&self, path: &str, _local: &std::path::Path) -> Result<(), HomeserverError> {
            if let Some(f) = &self.during {
                f();
            }
            match self.answers.lock().unwrap().pop_front() {
                None => {
                    self.uploaded.lock().unwrap().push(path.to_string());
                    Ok(())
                }
                Some(0) => Err(HomeserverError::Transport("connection reset".into())),
                Some(code) => Err(HomeserverError::Pubky(pubky::Error::Request(
                    pubky::errors::RequestError::Server {
                        status: pubky::StatusCode::from_u16(code).unwrap(),
                        message: String::new(),
                    },
                ))),
            }
        }
    }

    /// Phone mode with the setting on and `n` songs added (distinct files).
    fn phone(n: usize) -> (tempfile::TempDir, Arc<Phone>, Vec<crate::phone::songs::Song>) {
        let dir = tempfile::tempdir().unwrap();
        let p = Phone::open(super::super::Config {
            dir: dir.path().to_path_buf(),
            cache: dir.path().join("streams"),
            version: "test".into(),
            pubky: None,
            tidal: Default::default(),
        })
        .unwrap();
        p.store.put(KEY, Some("on")).unwrap();
        let songs = [
            "60_mono_audio.flac",
            "14_wasted_bits.flac",
            "61_predictor_overflow_check_16-bit.flac",
            "63_predictor_overflow_check_24-bit.flac",
        ]
        .iter()
        .take(n)
        .map(|f| {
            let part = dir.path().join("x.part");
            std::fs::copy(
                format!("{}/../pm-audio/tests/fixtures/{f}", env!("CARGO_MANIFEST_DIR")),
                &part,
            )
            .unwrap();
            p.songs.store(&part, f).unwrap()
        })
        .collect();
        (dir, p, songs)
    }

    fn uploaded(f: &Fake) -> usize {
        f.uploaded.lock().unwrap().len()
    }

    #[tokio::test]
    async fn with_nothing_to_copy_the_homeserver_is_not_asked() {
        let (_d, p, _) = phone(0);
        let f = Fake::default();
        assert_eq!(copy_while(&Arc::downgrade(&p), &f).await, Report::default());
        assert_eq!(f.listed.load(Ordering::SeqCst), 0);
        // Copied: the next passes don't list either
        let (_d, p, _) = phone(1);
        let f = Fake::default();
        assert_eq!(copy_while(&Arc::downgrade(&p), &f).await.copied, 1);
        assert_eq!(copy_while(&Arc::downgrade(&p), &f).await, Report::default());
        assert_eq!(f.listed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_song_gone_from_the_phone_is_skipped_and_the_others_go_on() {
        let (_d, p, songs) = phone(3);
        std::fs::remove_file(p.songs.path_of(&songs[0])).unwrap();
        let f = Fake::default();
        let r = copy_while(&Arc::downgrade(&p), &f).await;
        assert_eq!(r, Report { copied: 2, left: 1 });
        assert_eq!(uploaded(&f), 2);
    }

    #[tokio::test]
    async fn turned_off_during_a_pass_no_more_songs_go() {
        let (_d, p, _) = phone(3);
        let p2 = p.clone();
        let f = Fake {
            during: Some(Box::new(move || p2.store.put(KEY, Some("off")).unwrap())),
            ..Default::default()
        };
        let r = copy_while(&Arc::downgrade(&p), &f).await;
        assert_eq!(r.copied, 1, "the one under way ends, no other starts");
        assert_eq!(uploaded(&f), 1);
    }

    #[tokio::test]
    async fn a_refused_song_is_set_aside_until_a_new_choice() {
        let (_d, p, _) = phone(3);
        let f = Fake::default();
        f.answers.lock().unwrap().push_back(413);
        let r = copy_while(&Arc::downgrade(&p), &f).await;
        assert_eq!(r, Report { copied: 2, left: 1 }, "the others go on");
        assert_eq!(load_copies(&p).refused.len(), 1);
        // Not tried again by itself
        assert_eq!(copy_while(&Arc::downgrade(&p), &f).await, Report::default());
        assert_eq!(view(&p)["cant"], 1);
        // A new choice in Settings tries it again
        choose(State(p.clone()), Json(Choice { on: Some(true) })).await;
        assert!(load_copies(&p).refused.is_empty());
        assert_eq!(copy_while(&Arc::downgrade(&p), &f).await.copied, 1);
        assert_eq!(uploaded(&f), 3);
    }

    #[tokio::test]
    async fn a_failure_stops_the_pass_and_the_next_one_waits() {
        let (_d, p, _) = phone(2);
        let f = Fake::default();
        f.answers.lock().unwrap().push_back(0);
        let r = copy_while(&Arc::downgrade(&p), &f).await;
        assert_eq!(r, Report { copied: 0, left: 1 });
        // Too soon: nothing asked, nothing sent
        assert_eq!(copy_while(&Arc::downgrade(&p), &f).await, Report::default());
        assert_eq!(f.listed.load(Ordering::SeqCst), 1);
        // A new choice doesn't wait
        choose(State(p.clone()), Json(Choice { on: Some(true) })).await;
        assert_eq!(copy_while(&Arc::downgrade(&p), &f).await.copied, 2);
    }

    #[test]
    fn the_wait_doubles_up_to_six_hours() {
        assert_eq!(next_wait(None), BACKOFF);
        assert_eq!(next_wait(Some(BACKOFF)), BACKOFF * 2);
        assert_eq!(next_wait(Some(BACKOFF_MAX)), BACKOFF_MAX);
        assert_eq!(next_wait(Some(BACKOFF_MAX / 2 + BACKOFF)), BACKOFF_MAX);
    }

    #[test]
    fn keys_compare_without_the_pubky_prefix() {
        let k = "8pinxxgqs41n4aididenw5apqp1urfmzdztr8jt4abrkdn435ewo";
        assert_eq!(z32(&format!("pubky{k}")), k);
        assert_eq!(z32(k), k);
    }
}
