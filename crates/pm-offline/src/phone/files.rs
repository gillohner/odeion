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
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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

/// The person's choice, if they made one. Settings left out when the sign-ins file couldn't be
/// read ([`super::store::SETTINGS_SET_ASIDE`]) may have held an "off": until a new choice, off
/// (songs never go to the homeserver against it).
pub fn chosen(p: &Phone) -> Option<bool> {
    match p.store.get(KEY).as_deref() {
        Some("on") => Some(true),
        Some("off") => Some(false),
        _ if p.store.get(super::store::SETTINGS_SET_ASIDE).is_some() => Some(false),
        _ => None,
    }
}

/// Whether the copy reads as off only because settings were lost with sign-ins that couldn't be
/// read ([`chosen`]), not by a choice of the person's.
pub(crate) fn off_until_chosen(p: &Phone) -> bool {
    p.store.get(KEY).is_none() && p.store.get(super::store::SETTINGS_SET_ASIDE).is_some()
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
    let record = within(acct.hs.read_node_record(&acct.session)).await.ok()??;
    let Some(bundled) = record.bundled_homeserver else {
        return Some(false);
    };
    let theirs = tokio::time::timeout(
        pm_pubky::homeserver::OBJECT_TIMEOUT,
        acct.hs.pubky().get_homeserver_of(&acct.session.public_key()),
    )
    .await
    .ok()?
    .ok()??;
    Some(z32(&theirs.z32()) == z32(&bundled))
}

/// `f` within the time one homeserver object may take ([`OBJECT_TIMEOUT`]): a stalled connection
/// never holds the copier.
///
/// [`OBJECT_TIMEOUT`]: pm_pubky::homeserver::OBJECT_TIMEOUT
async fn within<T>(
    f: impl std::future::Future<Output = Result<T, HomeserverError>>,
) -> Result<T, HomeserverError> {
    within_for(pm_pubky::homeserver::OBJECT_TIMEOUT, f).await
}

async fn within_for<T>(
    limit: Duration,
    f: impl std::future::Future<Output = Result<T, HomeserverError>>,
) -> Result<T, HomeserverError> {
    tokio::time::timeout(limit, f).await.unwrap_or_else(|_| {
        Err(HomeserverError::Transport(
            "the homeserver didn't answer in time".into(),
        ))
    })
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
    /// Songs that took too long to copy (by content hash): each waits on its own, doubling.
    slow: Mutex<HashMap<String, (Instant, Duration)>>,
    /// One change at a time to `files.json` (the copier and a new choice).
    file: Mutex<()>,
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

/// What the phone knows of its copies; with no file yet, nothing. One that can't be read, or
/// isn't its JSON, is an error and is left as it is: read as empty, songs the person removed from
/// their homeserver would be copied there again. The copier then stops until it reads.
fn load_copies(p: &Phone) -> std::io::Result<Copies> {
    let path = copies_path(p);
    let bad = |e: &dyn std::fmt::Display| {
        std::io::Error::other(format!(
            "what was copied to the homeserver ({}) can't be read ({e}); left as it is",
            path.display()
        ))
    };
    match std::fs::read(&path) {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| bad(&e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Copies::default()),
        Err(e) => Err(bad(&e)),
    }
}

/// Written whole, on the disk before it replaces the old one.
fn save_copies(p: &Phone, c: &Copies) -> std::io::Result<()> {
    use std::io::Write;
    let path = copies_path(p);
    let tmp = path.with_extension("json.tmp");
    let b = serde_json::to_vec(c).map_err(std::io::Error::other)?;
    let write = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&b)?;
        f.sync_all()?;
        std::fs::rename(&tmp, &path)
    };
    write().inspect_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        tracing::warn!("phone mode: what was copied to the homeserver could not be saved: {e}");
    })
}

/// One change to what the phone knows of its copies, read and saved; an error (logged) when
/// either fails, and nothing changed.
fn change_copies(p: &Phone, f: impl FnOnce(&mut Copies) -> bool) -> std::io::Result<()> {
    let _one = p.copier.file.lock().unwrap_or_else(|e| e.into_inner());
    let mut c = load_copies(p).inspect_err(|e| tracing::warn!("phone mode: {e}"))?;
    if f(&mut c) {
        save_copies(p, &c)?;
    }
    Ok(())
}

/// The copier stops after a failure of the phone's own (its record of copies), as after a failed
/// upload: the next pass waits.
fn wait_after_failure(p: &Phone) {
    let mut retry = p.copier.retry.lock().unwrap_or_else(|e| e.into_inner());
    let wait = next_wait(retry.map(|(_, w)| w));
    *retry = Some((Instant::now() + wait, wait));
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

/// A copy pass soon (a song added, the setting changed), with no vault or data sync.
pub(crate) fn copy_soon(p: &Arc<Phone>) {
    let p = p.clone();
    tokio::spawn(async move {
        if let Some(a) = p.account().await {
            start(&p, &a);
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
    /// Whether the file at `path` is there (`None`: not), and its size when the homeserver says
    /// (`Some(None)`: there, its size not given).
    fn size(
        &self,
        path: &str,
    ) -> impl std::future::Future<Output = Result<Option<Option<u64>>, HomeserverError>> + Send;
    /// Removes the file at `path`.
    fn delete(&self, path: &str) -> impl std::future::Future<Output = Result<(), HomeserverError>> + Send;
}

impl Remote for Account {
    async fn names(&self) -> Result<Vec<String>, HomeserverError> {
        self.hs.list_private_file_names(&self.session).await
    }
    async fn upload(&self, path: &str, local: &std::path::Path) -> Result<(), HomeserverError> {
        self.hs.upload_private_file(&self.session, path, local).await
    }
    async fn size(&self, path: &str) -> Result<Option<Option<u64>>, HomeserverError> {
        self.hs.stat_private(&self.session, path).await
    }
    async fn delete(&self, path: &str) -> Result<(), HomeserverError> {
        self.hs.delete_private(&self.session, path).await
    }
}

/// Copies the songs added here that the homeserver doesn't have (by content hash), one at a time,
/// when the setting is on. A song counts as copied once the homeserver says it holds all of it (a
/// short copy is removed and sent again later). One found there whose size it doesn't give counts
/// as there, as on the node (`copy_guard::needs_sending`): never sent over, nor removed (it may be
/// whole); one it took and then gives no size of isn't counted until a later pass finds it there.
/// A size it can't say at all stops the pass. A song copied before, or refused for good, isn't
/// tried again; one that takes too long waits for the next pass and the others go on; after any
/// other failure the pass stops and the next one waits (doubling, up to 6 hours).
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
    copy_within(weak, remote, pm_pubky::homeserver::OBJECT_TIMEOUT).await
}

/// [`copy_while`], each size and removal on the homeserver within `limit`: a call that stalls
/// never holds the copier.
async fn copy_within(weak: &Weak<Phone>, remote: &impl Remote, limit: Duration) -> Report {
    let mut r = Report::default();
    let Some(p) = weak.upgrade() else { return r };
    // Off, or phone mode left
    if !on(&p) || p.closed() {
        return r;
    }
    if let Some((at, _)) = *p.copier.retry.lock().unwrap_or_else(|e| e.into_inner()) {
        if Instant::now() < at {
            return r;
        }
    }
    // The songs that may need copying now; the homeserver is asked only when there are some. Too
    // big, gone from the phone, or waiting after being too slow: left for later
    // Can't be read: nothing is copied (a song copied before would go again)
    let copies = match load_copies(&p) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("phone mode: no songs are copied: {e}");
            return r;
        }
    };
    let now = Instant::now();
    let mut todo = Vec::new();
    for s in p.songs.list() {
        let Some(h) = hash_of(&s.file).map(str::to_string) else {
            continue;
        };
        if copies.copied.contains(&h) || copies.refused.contains_key(&h) {
            continue;
        }
        let waits = p
            .copier
            .slow
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&h)
            .is_some_and(|(at, _)| now < *at);
        if s.bytes > MAX_COPY || !p.songs.path_of(&s).is_file() || waits {
            r.left += 1;
            continue;
        }
        todo.push((s, h));
    }
    if todo.is_empty() {
        return r;
    }
    drop(p);
    // Each content hash there, with the names it has
    let there: HashMap<String, Vec<String>> = match remote.names().await {
        Ok(names) => {
            let mut m: HashMap<String, Vec<String>> = HashMap::new();
            for n in names {
                if let Some(h) = hash_of(&n) {
                    m.entry(h.to_string()).or_default().push(n);
                }
            }
            m
        }
        Err(e) => {
            tracing::warn!("phone mode: the homeserver's files can't be listed now: {e}");
            return r;
        }
    };
    let Some(p) = weak.upgrade() else { return r };
    *p.copier.there.lock().unwrap_or_else(|e| e.into_inner()) = Some(there.keys().cloned().collect());
    drop(p);
    for (s, h) in todo {
        let Some(p) = weak.upgrade() else { break };
        // Turned off (or phone mode left) meanwhile: no more uploads
        if !on(&p) || p.closed() {
            break;
        }
        let local = p.songs.path_of(&s);
        drop(p);
        let path = format!("{}{}", pm_appdata::files::FILES_DIR, s.file);
        // There already, whole (a copy cut short by a failure or a kill is not): copied. A short
        // one under the phone's own name is replaced. One whose size the homeserver doesn't give
        // counts as there, as on the node (`copy_guard::needs_sending`); one whose size it can't
        // say now may be whole: never sent over, nor removed, and the pass stops
        let mut whole = false;
        let mut unknown = None;
        for name in there.get(&h).into_iter().flatten() {
            let at = format!("{}{name}", pm_appdata::files::FILES_DIR);
            match within_for(limit, remote.size(&at)).await {
                Ok(Some(Some(n))) if n == s.bytes => {
                    whole = true;
                    break;
                }
                Ok(Some(Some(_))) if at == path => {
                    let _ = within_for(limit, remote.delete(&at)).await;
                }
                Ok(Some(None)) => unknown = unknown.or(Some(true)),
                Err(e) => {
                    tracing::warn!(
                        "phone mode: the size of a song on the homeserver can't be asked now: {e}"
                    );
                    unknown = Some(false);
                }
                _ => {}
            }
        }
        match unknown.filter(|_| !whole) {
            // There, its size not given: there, as on the node
            Some(true) => whole = true,
            // Its size couldn't be asked: the pass stops as after a failure
            Some(false) => {
                if let Some(p) = weak.upgrade() {
                    wait_after_failure(&p);
                }
                r.left += 1;
                break;
            }
            None => {}
        }
        if whole {
            let Some(p) = weak.upgrade() else { break };
            if change_copies(&p, |c| c.copied.insert(h)).is_err() {
                wait_after_failure(&p);
                r.left += 1;
                break;
            }
            continue;
        }
        let sent = tokio::time::timeout(upload_for(s.bytes), remote.upload(&path, &local)).await;
        // Taken: copied only once the homeserver says it holds the whole song (an answer that came
        // early, or a copy cut on the way, is not); a short copy goes, and the song is sent again
        let stored = match &sent {
            Ok(Ok(())) => Some(within_for(limit, remote.size(&path)).await),
            _ => None,
        };
        if let Some(Ok(Some(Some(n)))) = stored {
            if n != s.bytes {
                let _ = within_for(limit, remote.delete(&path)).await;
            }
        }
        if !matches!(sent, Ok(Ok(()))) {
            // Whatever part of it arrived goes (a later pass sends it whole), judged now: the
            // listing is from the pass's start, and a whole copy that landed under this name
            // meanwhile (the node's) stays, as does one whose size the homeserver doesn't say
            if let Ok(Some(Some(n))) = within_for(limit, remote.size(&path)).await {
                if n != s.bytes {
                    let _ = within_for(limit, remote.delete(&path)).await;
                }
            }
        }
        let Some(p) = weak.upgrade() else { break };
        // One song waits on its own (doubling), the others go on
        let wait_alone = |h: String| {
            let mut slow = p.copier.slow.lock().unwrap_or_else(|e| e.into_inner());
            let wait = next_wait(slow.get(&h).map(|(_, w)| *w));
            slow.insert(h, (Instant::now() + wait, wait));
        };
        match (sent, stored) {
            (Ok(Ok(())), Some(Ok(Some(Some(n))))) if n == s.bytes => {
                p.copier.slow.lock().unwrap_or_else(|e| e.into_inner()).remove(&h);
                if let Some(t) = p.copier.there.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                    t.insert(h.clone());
                }
                // Not kept: the pass stops (the next one finds it there whole, and keeps it then)
                if change_copies(&p, |c| c.copied.insert(h)).is_err() {
                    wait_after_failure(&p);
                    r.left += 1;
                    break;
                }
                r.copied += 1;
                *p.copier.retry.lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
            // There, its size not given: not counted yet, never sent again (the next pass finds it
            // there and counts it, as the node does)
            (Ok(Ok(())), Some(Ok(Some(None)))) => {
                tracing::warn!(
                    "phone mode: the homeserver doesn't say the size of a song it took; it is left as it is"
                );
                p.copier.slow.lock().unwrap_or_else(|e| e.into_inner()).remove(&h);
                r.left += 1;
            }
            // Short there (removed above), or not there: sent again later
            (Ok(Ok(())), Some(Ok(_))) => {
                tracing::warn!(
                    "phone mode: a song the homeserver took isn't there whole; it is sent again later"
                );
                wait_alone(h);
                r.left += 1;
            }
            // Its size couldn't be asked: not counted yet (the next pass finds it whole, or sends it
            // again), and the pass stops as after a failure
            (Ok(Ok(())), _) => {
                tracing::warn!("phone mode: a song copied to the homeserver couldn't be checked there yet");
                wait_after_failure(&p);
                r.left += 1;
                break;
            }
            (Ok(Err(e)), _) if pm_pubky::homeserver::refused_for_good(&e) => {
                tracing::warn!("phone mode: the homeserver refused a song: {e}");
                r.left += 1;
                if change_copies(&p, |c| c.refused.insert(h, e.to_string()).is_none()).is_err() {
                    wait_after_failure(&p);
                    break;
                }
            }
            // Too slow for this one: it waits on its own (doubling), the others go on
            (Err(_), _) => {
                tracing::warn!("phone mode: a song took too long to copy to the homeserver");
                wait_alone(h);
                r.left += 1;
            }
            (Ok(Err(e)), _) => {
                tracing::warn!("phone mode: a song waits to be copied to the homeserver: {e}");
                wait_after_failure(&p);
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
    // Can't be read: not counted, and said (nothing is copied until it reads)
    let (copies, unreadable) = match load_copies(p) {
        Ok(c) => (c, false),
        Err(_) => (Copies::default(), true),
    };
    let (mut uploaded, mut cant) = (0, 0);
    for s in &songs {
        match hash_of(&s.file) {
            // Only what a pass found there whole, or there with no size given (as on a node)
            Some(h) if copies.copied.contains(h) => uploaded += 1,
            Some(h) if copies.refused.contains_key(h) => cant += 1,
            _ if s.bytes > MAX_COPY => cant += 1,
            _ => {}
        }
    }
    json!({
        "on": on(p),
        "chosen": chosen(p),
        "bundled": *p.bundled_here.lock().unwrap_or_else(|e| e.into_inner()),
        "uploaded": if unreadable { Value::Null } else { json!(uploaded) },
        "cant": if unreadable { Value::Null } else { json!(cant) },
        "total": songs.len(),
        "unreadable": unreadable,
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
    // What was copied can't be read: nothing changes (the copier can't run anyway)
    if let Err(e) = load_copies(&p) {
        return err(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    // (A choice made now is the person's: settings lost before no longer stand in for it)
    let saved = p.store.change_if(|d| {
        d.put(KEY, v);
        d.put(super::store::SETTINGS_SET_ASIDE, None);
        Ok(true)
    });
    if let Err(e) = saved {
        return err(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    // A new choice tries the songs the homeserver refused again (it may have room now; not
    // kept, they are tried at the next choice)
    let _ = change_copies(&p, |c| {
        let had = !c.refused.is_empty();
        c.refused.clear();
        had
    });
    *p.copier.retry.lock().unwrap_or_else(|e| e.into_inner()) = None;
    copy_soon(&p);
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
        /// The size of each file there (what was uploaded, or set by a test).
        sizes: Mutex<HashMap<String, u64>>,
        deleted: Mutex<Vec<String>>,
        /// Size and removal requests never answer.
        stalls: bool,
        /// The size of what is at the path after a failed upload: a part of it, or a whole copy
        /// another device put there meanwhile (none: nothing).
        left_there: Option<u64>,
        /// The size of what is there after an upload the homeserver took (none: all of it).
        kept_of_upload: Option<u64>,
        /// Files there whose size the homeserver doesn't give.
        no_size: Mutex<HashSet<String>>,
        /// What was uploaded is listed by the next passes.
        lists_uploads: bool,
        /// Uploads tried (taken or not).
        tried: AtomicUsize,
    }

    impl Remote for Fake {
        async fn names(&self) -> Result<Vec<String>, HomeserverError> {
            self.listed.fetch_add(1, Ordering::SeqCst);
            let mut names = self.names.clone();
            if self.lists_uploads {
                for u in self.uploaded.lock().unwrap().iter() {
                    names.push(u.trim_start_matches(pm_appdata::files::FILES_DIR).to_string());
                }
            }
            Ok(names)
        }
        async fn upload(&self, path: &str, local: &std::path::Path) -> Result<(), HomeserverError> {
            self.tried.fetch_add(1, Ordering::SeqCst);
            if let Some(f) = &self.during {
                f();
            }
            match self.answers.lock().unwrap().pop_front() {
                None => {
                    self.uploaded.lock().unwrap().push(path.to_string());
                    let n = std::fs::metadata(local).map(|m| m.len()).unwrap_or(0);
                    let n = self.kept_of_upload.unwrap_or(n);
                    self.sizes.lock().unwrap().insert(path.to_string(), n);
                    Ok(())
                }
                Some(code) => {
                    if let Some(n) = self.left_there {
                        self.sizes.lock().unwrap().insert(path.to_string(), n);
                    }
                    Err(match code {
                        0 => HomeserverError::Transport("connection reset".into()),
                        _ => HomeserverError::Pubky(pubky::Error::Request(
                            pubky::errors::RequestError::Server {
                                status: pubky::StatusCode::from_u16(code).unwrap(),
                                message: String::new(),
                            },
                        )),
                    })
                }
            }
        }
        async fn size(&self, path: &str) -> Result<Option<Option<u64>>, HomeserverError> {
            if self.stalls {
                std::future::pending::<()>().await;
            }
            if self.no_size.lock().unwrap().contains(path) {
                return Ok(Some(None));
            }
            Ok(self.sizes.lock().unwrap().get(path).copied().map(Some))
        }
        async fn delete(&self, path: &str) -> Result<(), HomeserverError> {
            if self.stalls {
                std::future::pending::<()>().await;
            }
            self.deleted.lock().unwrap().push(path.to_string());
            self.sizes.lock().unwrap().remove(path);
            Ok(())
        }
    }

    impl Fake {
        fn listing(names: Vec<String>) -> Self {
            Fake {
                names,
                ..Default::default()
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
    async fn a_record_of_copies_that_cant_be_read_copies_nothing() {
        let (_d, p, _) = phone(2);
        // One song copied before (the person may have removed it from their homeserver since),
        // then files.json is cut short (a power cut)
        let f = Fake::default();
        let first = Copies {
            copied: BTreeSet::from([hash_of(&p.songs.list()[0].file).unwrap().to_string()]),
            ..Default::default()
        };
        save_copies(&p, &first).unwrap();
        let whole = std::fs::read(copies_path(&p)).unwrap();
        let cut = &whole[..whole.len() - 3];
        std::fs::write(copies_path(&p), cut).unwrap();
        // Nothing is copied (the removed song isn't sent again), the file stays as it is
        let r = copy_while(&Arc::downgrade(&p), &f).await;
        assert_eq!(r.copied, 0);
        assert_eq!(uploaded(&f), 0);
        assert_eq!(std::fs::read(copies_path(&p)).unwrap(), cut);
        // The page is told it can't be read, never "none uploaded"
        assert_eq!(view(&p)["unreadable"], true);
        assert!(view(&p)["uploaded"].is_null());
        // A new choice doesn't write over it either, and isn't made (the page keeps its switch)
        let r = choose(State(p.clone()), Json(Choice { on: Some(false) })).await;
        assert_eq!(r.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(std::fs::read(copies_path(&p)).unwrap(), cut);
        assert_eq!(chosen(&p), Some(true));
    }

    #[tokio::test]
    async fn a_copy_the_phone_cant_note_stops_the_copier() {
        let (_d, p, _) = phone(2);
        // files.json can't be written now (a folder where it is written first)
        std::fs::create_dir(copies_path(&p).with_extension("json.tmp")).unwrap();
        let f = Fake::default();
        let r = copy_while(&Arc::downgrade(&p), &f).await;
        // The first song went and couldn't be noted: not counted, and no more songs go
        assert_eq!(r, Report { copied: 0, left: 1 });
        assert_eq!(uploaded(&f), 1);
        // The next pass waits, as after a failure
        assert_eq!(copy_while(&Arc::downgrade(&p), &f).await, Report::default());
        assert_eq!(uploaded(&f), 1);
    }

    #[tokio::test]
    async fn a_song_gone_from_the_phone_is_skipped_and_the_others_go_on() {
        let (_d, p, songs) = phone(3);
        std::fs::remove_file(p.songs.path_of(&songs[0])).unwrap();
        let f = Fake::default();
        let r = copy_while(&Arc::downgrade(&p), &f).await;
        assert_eq!(r, Report { copied: 2, left: 1 });
        assert_eq!(uploaded(&f), 2);
        // Left as it is, it doesn't make every later pass ask the homeserver
        assert_eq!(
            copy_while(&Arc::downgrade(&p), &f).await,
            Report { copied: 0, left: 1 }
        );
        assert_eq!(f.listed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_song_too_slow_to_copy_waits_on_its_own() {
        let (_d, p, songs) = phone(2);
        let f = Fake::default();
        // As if its upload had timed out just now
        let h = hash_of(&songs[0].file).unwrap().to_string();
        p.copier
            .slow
            .lock()
            .unwrap()
            .insert(h.clone(), (Instant::now() + BACKOFF, BACKOFF));
        let r = copy_while(&Arc::downgrade(&p), &f).await;
        assert_eq!(r, Report { copied: 1, left: 1 }, "the other goes on");
        // Asked again at once (songs added meanwhile): it still waits, nothing is sent
        assert_eq!(
            copy_while(&Arc::downgrade(&p), &f).await,
            Report { copied: 0, left: 1 }
        );
        assert_eq!(uploaded(&f), 1);
        // Its wait over, it goes
        p.copier.slow.lock().unwrap().insert(h, (Instant::now(), BACKOFF));
        assert_eq!(copy_while(&Arc::downgrade(&p), &f).await.copied, 1);
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
        assert_eq!(load_copies(&p).unwrap().refused.len(), 1);
        // Not tried again by itself
        assert_eq!(copy_while(&Arc::downgrade(&p), &f).await, Report::default());
        assert_eq!(view(&p)["cant"], 1);
        // A new choice in Settings tries it again
        choose(State(p.clone()), Json(Choice { on: Some(true) })).await;
        assert!(load_copies(&p).unwrap().refused.is_empty());
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

    #[tokio::test]
    async fn a_copy_cut_short_is_sent_again_whole() {
        let (_d, p, songs) = phone(2);
        let path = |s: &crate::phone::songs::Song| format!("{}{}", pm_appdata::files::FILES_DIR, s.file);
        // The first is there whole under another name (the node's), the second only partly under
        // its own (an upload cut by a kill)
        let h0 = hash_of(&songs[0].file).unwrap();
        let theirs = format!("{h0}-01_From_The_Node.flac");
        let f = Fake::listing(vec![theirs.clone(), songs[1].file.clone()]);
        f.sizes.lock().unwrap().insert(
            format!("{}{theirs}", pm_appdata::files::FILES_DIR),
            songs[0].bytes,
        );
        f.sizes.lock().unwrap().insert(path(&songs[1]), 10);
        let r = copy_while(&Arc::downgrade(&p), &f).await;
        assert_eq!(r.copied, 1, "only the short one is sent");
        assert_eq!(*f.uploaded.lock().unwrap(), [path(&songs[1])]);
        assert_eq!(
            *f.deleted.lock().unwrap(),
            [path(&songs[1])],
            "the short copy went first"
        );
        assert_eq!(load_copies(&p).unwrap().copied.len(), 2);
    }

    #[tokio::test]
    async fn a_failed_upload_leaves_nothing_behind() {
        let (_d, p, songs) = phone(1);
        // Cut off after 10 bytes
        let f = Fake {
            left_there: Some(10),
            ..Default::default()
        };
        f.answers.lock().unwrap().push_back(0);
        copy_while(&Arc::downgrade(&p), &f).await;
        let path = format!("{}{}", pm_appdata::files::FILES_DIR, songs[0].file);
        assert_eq!(*f.deleted.lock().unwrap(), [path]);
        assert!(load_copies(&p).unwrap().copied.is_empty());
    }

    #[tokio::test]
    async fn a_homeserver_that_never_answers_doesnt_hold_the_copier() {
        let never = std::future::pending::<Result<(), HomeserverError>>();
        assert!(within_for(Duration::from_millis(20), never).await.is_err());
        // Its size never comes after the upload: not counted as copied (it can't be checked), and
        // the pass ends
        let (_d, p, _songs) = phone(1);
        let f = Fake {
            stalls: true,
            ..Default::default()
        };
        let r = tokio::time::timeout(
            Duration::from_secs(5),
            copy_within(&Arc::downgrade(&p), &f, Duration::from_millis(20)),
        )
        .await
        .expect("the pass ends");
        assert_eq!(uploaded(&f), 1);
        assert_eq!(r, Report { copied: 0, left: 1 });
        assert!(load_copies(&p).unwrap().copied.is_empty());
    }

    #[tokio::test]
    async fn a_copy_whose_size_cant_be_asked_is_never_sent_over() {
        // There under the phone's own name (it may be whole), its size never comes: nothing is
        // sent over it (an upload that failed would leave it cut short, and then remove it), the
        // pass ends and the next one waits
        let (_d, p, songs) = phone(1);
        let f = Fake {
            names: vec![songs[0].file.clone()],
            stalls: true,
            ..Default::default()
        };
        let r = tokio::time::timeout(
            Duration::from_secs(5),
            copy_within(&Arc::downgrade(&p), &f, Duration::from_millis(20)),
        )
        .await
        .expect("the pass ends");
        assert_eq!(f.tried.load(Ordering::SeqCst), 0, "nothing sent over it");
        assert_eq!(r, Report { copied: 0, left: 1 });
        assert!(p.copier.retry.lock().unwrap().is_some(), "the next pass waits");
        assert!(load_copies(&p).unwrap().copied.is_empty());
    }

    #[tokio::test]
    async fn a_copy_the_homeserver_holds_short_is_removed_and_sent_again() {
        let (_d, p, songs) = phone(2);
        let path = |s: &crate::phone::songs::Song| format!("{}{}", pm_appdata::files::FILES_DIR, s.file);
        // Each upload is answered "taken", but only 10 bytes of it are there
        let f = Fake {
            kept_of_upload: Some(10),
            ..Default::default()
        };
        let r = copy_while(&Arc::downgrade(&p), &f).await;
        assert_eq!(r, Report { copied: 0, left: 2 }, "neither counted");
        assert!(load_copies(&p).unwrap().copied.is_empty());
        assert_eq!(view(&p)["uploaded"], 0);
        assert_eq!(*f.deleted.lock().unwrap(), [path(&songs[0]), path(&songs[1])]);
        // Each waits on its own; its wait over, it is sent again, whole this time: copied
        assert_eq!(
            copy_while(&Arc::downgrade(&p), &f).await,
            Report { copied: 0, left: 2 }
        );
        p.copier.slow.lock().unwrap().clear();
        let whole = Fake::default();
        assert_eq!(copy_while(&Arc::downgrade(&p), &whole).await.copied, 2);
        assert_eq!(
            *whole.uploaded.lock().unwrap(),
            [path(&songs[0]), path(&songs[1])]
        );
        assert_eq!(load_copies(&p).unwrap().copied.len(), 2);
    }

    #[tokio::test]
    async fn a_failed_upload_keeps_a_copy_it_couldnt_check() {
        let (_d, p, songs) = phone(1);
        let path = format!("{}{}", pm_appdata::files::FILES_DIR, songs[0].file);
        // Not there when the pass listed; after the failed upload something is there, its size
        // not given: kept
        let f = Fake::default();
        f.answers.lock().unwrap().push_back(0);
        f.no_size.lock().unwrap().insert(path.clone());
        copy_while(&Arc::downgrade(&p), &f).await;
        assert_eq!(f.tried.load(Ordering::SeqCst), 1);
        assert!(
            f.deleted.lock().unwrap().is_empty(),
            "{:?}",
            f.deleted.lock().unwrap()
        );
    }

    #[tokio::test]
    async fn a_copy_whose_size_isnt_given_is_never_sent_over_nor_removed() {
        let (_d, p, songs) = phone(1);
        let path = format!("{}{}", pm_appdata::files::FILES_DIR, songs[0].file);
        // There under the phone's own name, whole perhaps: the homeserver doesn't give its size.
        // Were it sent over, an upload that fails (as here) would leave it cut short and remove it
        let f = Fake {
            names: vec![songs[0].file.clone()],
            left_there: Some(10),
            ..Default::default()
        };
        f.no_size.lock().unwrap().insert(path.clone());
        f.answers.lock().unwrap().push_back(0);
        let r = copy_while(&Arc::downgrade(&p), &f).await;
        assert_eq!(f.tried.load(Ordering::SeqCst), 0, "nothing sent over it");
        assert!(f.deleted.lock().unwrap().is_empty());
        // There, as on the node: counted, so never asked about again
        assert_eq!(r, Report::default());
        assert_eq!(load_copies(&p).unwrap().copied.len(), 1);
        assert_eq!(view(&p)["uploaded"], 1);
        copy_while(&Arc::downgrade(&p), &f).await;
        assert_eq!(f.tried.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_song_taken_whose_size_isnt_given_is_not_sent_again() {
        let (_d, p, songs) = phone(1);
        let path = format!("{}{}", pm_appdata::files::FILES_DIR, songs[0].file);
        // Taken, and then there with no size given (the next passes list it)
        let f = Fake {
            lists_uploads: true,
            ..Default::default()
        };
        f.no_size.lock().unwrap().insert(path.clone());
        assert_eq!(
            copy_while(&Arc::downgrade(&p), &f).await,
            Report { copied: 0, left: 1 }
        );
        assert_eq!(f.tried.load(Ordering::SeqCst), 1);
        // Later passes (any wait over) leave it as it is: never sent again, never removed
        for _ in 0..3 {
            p.copier.slow.lock().unwrap().clear();
            *p.copier.retry.lock().unwrap() = None;
            copy_while(&Arc::downgrade(&p), &f).await;
        }
        assert_eq!(f.tried.load(Ordering::SeqCst), 1, "sent once");
        assert!(f.deleted.lock().unwrap().is_empty());
        // Found there by the next pass: counted, as on the node
        assert_eq!(load_copies(&p).unwrap().copied.len(), 1);
    }

    #[tokio::test]
    async fn a_whole_copy_put_there_during_a_failed_upload_stays() {
        let (_d, p, songs) = phone(1);
        // Not there when the pass listed; the node puts its whole copy under the same name while
        // the phone's upload of it fails
        let f = Fake {
            left_there: Some(songs[0].bytes),
            ..Default::default()
        };
        f.answers.lock().unwrap().push_back(0);
        copy_while(&Arc::downgrade(&p), &f).await;
        assert!(
            f.deleted.lock().unwrap().is_empty(),
            "{:?}",
            f.deleted.lock().unwrap()
        );
        // The next pass finds it whole: copied, nothing sent
        *p.copier.retry.lock().unwrap() = None;
        let f2 = Fake::listing(vec![songs[0].file.clone()]);
        f2.sizes.lock().unwrap().insert(
            format!("{}{}", pm_appdata::files::FILES_DIR, songs[0].file),
            songs[0].bytes,
        );
        copy_while(&Arc::downgrade(&p), &f2).await;
        assert_eq!(uploaded(&f2), 0);
        assert_eq!(load_copies(&p).unwrap().copied.len(), 1);
    }

    #[tokio::test]
    async fn a_choice_lost_with_sign_ins_that_cant_be_read_is_off() {
        // The person turned copies to the homeserver off; then the sign-ins file is cut short
        // (it holds the choice too) and set aside at the next start, on a homeserver that is
        // their node's (where "not chosen" means on)
        let (d, p, _) = phone(1);
        p.store.put(KEY, Some("off")).unwrap();
        drop(p);
        std::fs::write(
            d.path().join(super::super::store::FILE),
            br#"{"phone.files_on_homeserver":"of"#,
        )
        .unwrap();
        let p = Phone::open(super::super::Config {
            dir: d.path().to_path_buf(),
            cache: d.path().join("streams"),
            version: "test".into(),
            pubky: None,
            tidal: Default::default(),
        })
        .unwrap();
        *p.bundled_here.lock().unwrap() = Some(true);
        assert_eq!(chosen(&p), Some(false));
        assert!(!on(&p), "off until the person chooses again");
        let f = Fake::default();
        assert_eq!(copy_while(&Arc::downgrade(&p), &f).await, Report::default());
        assert_eq!(
            f.tried.load(Ordering::SeqCst),
            0,
            "nothing sent against the choice"
        );
        // They are told, with the sign-ins set aside
        let told = p.set_aside_notice().unwrap();
        assert!(
            told.ends_with(
                "Keeping the songs you add on your homeserver is off until you choose again in Settings."
            ),
            "{told}"
        );
        // A new choice is the person's, "back to the default" too (on, with their node's
        // homeserver)
        let back = choose(State(p.clone()), Json(Choice { on: None })).await;
        assert_eq!(back.status(), axum::http::StatusCode::OK);
        assert_eq!(chosen(&p), None);
        assert!(on(&p));
        assert_eq!(copy_while(&Arc::downgrade(&p), &f).await.copied, 1);
    }

    #[test]
    fn a_choice_that_reads_stands_when_another_setting_is_lost() {
        // A setting that doesn't read is left out; the copy choice beside it reads: it stands,
        // and the notice doesn't say the copy is off
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join(super::super::store::FILE),
            br#"{"phone.files_on_homeserver":"on","qobuz.app":7}"#,
        )
        .unwrap();
        let p = Phone::open(super::super::Config {
            dir: d.path().to_path_buf(),
            cache: d.path().join("streams"),
            version: "test".into(),
            pubky: None,
            tidal: Default::default(),
        })
        .unwrap();
        assert_eq!(chosen(&p), Some(true));
        let told = p.set_aside_notice().unwrap();
        assert!(
            told.starts_with("Some settings on this phone couldn't be read."),
            "{told}"
        );
        assert!(!told.contains("is off"), "{told}");
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
