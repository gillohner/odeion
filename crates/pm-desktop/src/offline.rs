//! Music kept on this device (STEWARD-DOWNLOADS.md), through `pm-offline`: a sync with the node
//! while it is reachable, and the web UI from a local server while it isn't. Each node keeps its
//! own folder (`offline/<node id>/`), so one node's plays, hearts and keeps never reach another.

use crate::settings::Settings;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often the device syncs while connected.
const SYNC_EVERY: Duration = Duration::from_secs(15 * 60);
/// The downloads' limit until the person sets one.
pub const DEFAULT_LIMIT: u64 = 10_000_000_000;
/// Downloads never leave less than this free on the device.
pub const KEEP_FREE: u64 = 1_000_000_000;

/// The local server, for one node's store.
struct Server {
    node_id: String,
    origin: String,
    task: tokio::task::JoinHandle<()>,
}

pub struct Offline {
    /// `offline/` in the app's data folder; each node's store is in a folder under it.
    base: PathBuf,
    /// Every store opened in this run, by node id: the sync loop and the local server share
    /// them, so a node's folder is never open twice (each `Store` holds its own queue lock and
    /// index).
    open: Mutex<HashMap<String, Arc<pm_offline::Store>>>,
    server: tokio::sync::Mutex<Option<Server>>,
    /// `http://127.0.0.1:<port>` while the local server runs; the navigation guard reads it.
    origin: Arc<Mutex<Option<String>>>,
    /// Raised by every [`Offline::spawn_sync`]: an older loop sees it and stops.
    generation: AtomicU64,
    /// One sync at a time, also while a replaced loop finishes its last one.
    syncing: tokio::sync::Mutex<()>,
    /// Where the current (or last) sync stands, for the storage screen.
    progress: Arc<Mutex<pm_offline::Progress>>,
    /// Told when what the device keeps or the sync's progress changed (the page hears it).
    changed: Arc<Mutex<Option<Hook>>>,
}

/// Who hears of changes to the downloads.
type Hook = Arc<dyn Fn() + Send + Sync>;

/// The device's own settings for downloads (`offline/settings.json`).
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Prefs {
    /// The limit in bytes; `None` for none; missing: [`DEFAULT_LIMIT`].
    #[serde(default = "default_limit")]
    limit: Option<u64>,
}

fn default_limit() -> Option<u64> {
    Some(DEFAULT_LIMIT)
}

/// What the storage screen shows (`downloads_state`).
#[derive(serde::Serialize)]
pub struct DownloadsState {
    #[serde(flatten)]
    pub summary: pm_offline::Summary,
    pub limit: Option<u64>,
    pub free: Option<u64>,
    pub keep_free: u64,
    pub progress: pm_offline::Progress,
}

impl Offline {
    /// Offline music in `base`, or `None` when the folder can't be made (offline music is then
    /// off; everything else works).
    pub fn open(base: PathBuf, origin: Arc<Mutex<Option<String>>>) -> Option<Arc<Self>> {
        if let Err(e) = std::fs::create_dir_all(&base) {
            tracing::warn!("offline music is off: {e}");
            return None;
        }
        Some(Arc::new(Self {
            base,
            open: Mutex::default(),
            server: Default::default(),
            origin,
            generation: AtomicU64::new(0),
            syncing: Default::default(),
            progress: Arc::default(),
            changed: Arc::default(),
        }))
    }

    /// Who hears of changes to the downloads (the page, `pmdesktop:downloads`).
    pub fn on_change(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.changed.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(f));
    }

    fn tell(&self) {
        let f = self.changed.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(f) = f {
            f();
        }
    }

    fn prefs(&self) -> Prefs {
        std::fs::read(self.base.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Prefs {
                limit: default_limit(),
            })
    }

    /// The downloads' limit in bytes (`None`: none of their own; the disk's 1 GB still holds).
    pub fn set_limit(&self, limit: Option<u64>) -> Result<(), String> {
        let b = serde_json::to_vec(&Prefs { limit }).map_err(|e| e.to_string())?;
        std::fs::write(self.base.join("settings.json"), b)
            .map_err(|e| format!("Can't save the limit: {e}"))?;
        self.tell();
        Ok(())
    }

    /// What this device keeps from the node with this id, with the limit and the sync's progress.
    pub fn state(&self, node_id: &str) -> DownloadsState {
        let summary = self
            .store(node_id)
            .map(|s| pm_offline::summary(&s.index()))
            .unwrap_or_default();
        DownloadsState {
            summary,
            limit: self.prefs().limit,
            free: pm_offline::sync::free_bytes(&self.base),
            keep_free: KEEP_FREE,
            progress: self.progress.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        }
    }

    /// Keep `item` on the device (or stop keeping it); the caller syncs next.
    pub fn keep(
        &self,
        node_id: &str,
        item: &pm_offline::KeepItem,
        on: bool,
        label: Option<pm_offline::Label>,
    ) -> Result<(), String> {
        let store = self.store(node_id).ok_or("Downloads are off on this device")?;
        store
            .keep(item, on, label)
            .map_err(|e| format!("Can't save it: {e}"))?;
        self.tell();
        Ok(())
    }

    /// Keep nothing: the next sync removes the device's copies (only those).
    pub fn keep_nothing(&self, node_id: &str) -> Result<(), String> {
        let store = self.store(node_id).ok_or("Downloads are off on this device")?;
        store.keep_nothing().map_err(|e| format!("Can't save it: {e}"))?;
        self.tell();
        Ok(())
    }

    /// The store of the node with this id, opened once. `None` without an id (a node that never
    /// answered) or when it can't be opened.
    fn store(&self, node_id: &str) -> Option<Arc<pm_offline::Store>> {
        if node_id.is_empty() {
            return None;
        }
        let mut open = self.open.lock().ok()?;
        if let Some(s) = open.get(node_id) {
            return Some(s.clone());
        }
        match pm_offline::Store::open(pm_offline::node_dir(&self.base, node_id)) {
            Ok(s) => {
                let s = Arc::new(s);
                open.insert(node_id.to_string(), s.clone());
                Some(s)
            }
            Err(e) => {
                tracing::warn!("offline music for this node is off: {e}");
                None
            }
        }
    }

    /// Whether this device keeps music from the node with this id. Doesn't make its folder.
    pub fn has_downloads(&self, node_id: &str) -> bool {
        !node_id.is_empty()
            && pm_offline::node_dir(&self.base, node_id)
                .join("index.json")
                .is_file()
            && self.store(node_id).is_some_and(|s| !s.index().tracks.is_empty())
    }

    /// Start the local server for this node's store (once; again for another node) and give the
    /// address of its Downloads page.
    pub async fn page(&self, node_id: &str) -> Result<String, String> {
        let store = self
            .store(node_id)
            .ok_or("Nothing is downloaded from this node")?;
        let mut server = self.server.lock().await;
        let origin = match server.as_ref() {
            Some(s) if s.node_id == node_id && !s.task.is_finished() => s.origin.clone(),
            _ => {
                if let Some(old) = server.take() {
                    old.task.abort();
                }
                let (addr, task) = pm_offline::server::serve(store)
                    .await
                    .map_err(|e| format!("Can't start offline playback: {e}"))?;
                let origin = format!("http://{addr}");
                *server = Some(Server {
                    node_id: node_id.to_string(),
                    origin: origin.clone(),
                    task,
                });
                origin
            }
        };
        if let Ok(mut cur) = self.origin.lock() {
            *cur = Some(origin.clone());
        }
        Ok(format!("{origin}/downloads"))
    }

    fn options(&self) -> pm_offline::SyncOptions {
        let (progress, changed) = (self.progress.clone(), self.changed.clone());
        pm_offline::SyncOptions {
            cap_bytes: self.prefs().limit,
            keep_free: KEEP_FREE,
            progress: Some(pm_offline::ProgressSink(Arc::new(move |p| {
                *progress.lock().unwrap_or_else(|e| e.into_inner()) = p.clone();
                let f = changed.lock().unwrap_or_else(|e| e.into_inner()).clone();
                if let Some(f) = f {
                    f();
                }
            }))),
            ..Self::device()
        }
    }

    fn device() -> pm_offline::SyncOptions {
        let (name, kind) = match std::env::consts::OS {
            "android" => ("This phone", "android"),
            "ios" => ("This phone", "ios"),
            _ => ("This computer", "desktop"),
        };
        pm_offline::SyncOptions {
            name: name.into(),
            kind: kind.into(),
            ..Default::default()
        }
    }

    /// Sync now and then every [`SYNC_EVERY`], with the settings `current` gives at each round
    /// (the node's address, the tunnel's port and the token may change between rounds). A newer
    /// call replaces this loop; it stops when there is no node any more.
    pub fn spawn_sync(self: &Arc<Self>, current: impl Fn() -> Option<Settings> + Send + 'static) {
        let mine = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                {
                    let _one = me.syncing.lock().await;
                    if me.generation.load(Ordering::SeqCst) != mine {
                        break;
                    }
                    let Some(s) = current() else { break };
                    me.sync_once(&s).await;
                }
                tokio::time::sleep(SYNC_EVERY).await;
                if me.generation.load(Ordering::SeqCst) != mine {
                    break;
                }
            }
        });
    }

    async fn sync_once(&self, s: &Settings) {
        let Some(store) = self.store(&s.node_id) else {
            tracing::info!("offline: no sync without the node's id");
            return;
        };
        match pm_offline::Node::new(s.base(), &s.token) {
            Ok(node) => match pm_offline::sync(&store, &node, &self.options()).await {
                Ok(r) => tracing::info!(
                    downloaded = r.downloaded.len(),
                    removed = r.removed.len(),
                    sent = r.sent,
                    failed = r.failed.len(),
                    full = r.full.as_deref().unwrap_or(""),
                    "offline: synced"
                ),
                Err(e) => tracing::info!("offline: sync stopped: {e}"),
            },
            Err(e) => tracing::warn!("offline: {e}"),
        }
    }
}

/// The node in use and this device's downloads from it.
fn offline_of(state: &crate::AppState) -> Result<(Arc<Offline>, String), String> {
    let o = state.offline.clone().ok_or("Downloads are off on this device")?;
    let node_id = state.current().map(|s| s.node_id).unwrap_or_default();
    if node_id.is_empty() {
        return Err("Connect to your node first".into());
    }
    Ok((o, node_id))
}

/// Sync now (a new loop; the running one ends after its round).
fn sync_now(app: &tauri::AppHandle, o: &Arc<Offline>) {
    use tauri::Manager;
    let app = app.clone();
    o.spawn_sync(move || app.state::<crate::AppState>().current());
}

/// The storage screen: what this device keeps from the node in use, the limit, the sync.
#[tauri::command]
pub fn downloads_state(state: tauri::State<'_, crate::AppState>) -> Result<DownloadsState, String> {
    let (o, node_id) = offline_of(&state)?;
    Ok(o.state(&node_id))
}

/// Download (`on`) a song, a library album or a playlist to this device, or remove its download;
/// `title` and `sub` name it on the storage screen until its songs arrive. A sync starts now.
#[tauri::command]
pub fn downloads_keep(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    kind: String,
    id: String,
    on: bool,
    title: Option<String>,
    sub: Option<String>,
) -> Result<DownloadsState, String> {
    let (o, node_id) = offline_of(&state)?;
    let item =
        pm_offline::KeepItem::parse(&kind, &id).ok_or("Only songs, albums and playlists of your library")?;
    let label = title.map(|t| pm_offline::Label {
        title: t.chars().take(200).collect(),
        sub: sub.unwrap_or_default().chars().take(200).collect(),
    });
    o.keep(&node_id, &item, on, label)?;
    sync_now(&app, &o);
    Ok(o.state(&node_id))
}

/// Remove every download from this device (its copies only; the library is not touched).
#[tauri::command]
pub fn downloads_remove_all(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
) -> Result<DownloadsState, String> {
    let (o, node_id) = offline_of(&state)?;
    o.keep_nothing(&node_id)?;
    sync_now(&app, &o);
    Ok(o.state(&node_id))
}

/// The downloads' limit in bytes (`None`: no limit of their own). A sync starts now, so what was
/// waiting for room comes.
#[tauri::command]
pub fn downloads_limit(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    bytes: Option<u64>,
) -> Result<DownloadsState, String> {
    let (o, node_id) = offline_of(&state)?;
    o.set_limit(bytes.map(|b| b.max(100_000_000)))?;
    sync_now(&app, &o);
    Ok(o.state(&node_id))
}

/// The page hears the downloads changed (`pmdesktop:downloads`, with the storage screen's state).
pub fn push_state(app: &tauri::AppHandle) {
    use tauri::Manager;
    let state = app.state::<crate::AppState>();
    let Ok((o, node_id)) = offline_of(&state) else {
        return;
    };
    let st = o.state(&node_id);
    if let Some(w) = app.get_webview_window(crate::MAIN) {
        let _ = w.eval(format!(
            "window.dispatchEvent(new CustomEvent('pmdesktop:downloads',{{detail:{}}}));",
            serde_json::to_string(&st).unwrap_or_else(|_| "null".into())
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limit_is_ten_gb_until_set_and_kept_across_runs() {
        let dir = tempfile_dir();
        let o = Offline::open(dir.clone(), Arc::default()).unwrap();
        assert_eq!(o.prefs().limit, Some(DEFAULT_LIMIT));
        o.set_limit(Some(5_000_000_000)).unwrap();
        let again = Offline::open(dir.clone(), Arc::default()).unwrap();
        assert_eq!(again.prefs().limit, Some(5_000_000_000));
        again.set_limit(None).unwrap();
        assert_eq!(again.prefs().limit, None);
        let opts = again.options();
        assert_eq!((opts.cap_bytes, opts.keep_free), (None, KEEP_FREE));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keeping_shows_on_the_storage_screen_and_tells_the_page() {
        let dir = tempfile_dir();
        let o = Offline::open(dir.clone(), Arc::default()).unwrap();
        let told = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let t2 = told.clone();
        o.on_change(move || {
            t2.fetch_add(1, Ordering::SeqCst);
        });
        let label = pm_offline::Label {
            title: "Journey".into(),
            sub: "Sefa".into(),
        };
        o.keep("node-1", &pm_offline::KeepItem::Album(7), true, Some(label))
            .unwrap();
        let st = o.state("node-1");
        assert_eq!(st.summary.album_ids, [7]);
        assert_eq!(st.summary.items[0].title, "Journey");
        assert_eq!(st.limit, Some(DEFAULT_LIMIT));
        o.keep_nothing("node-1").unwrap();
        assert!(o.state("node-1").summary.items.is_empty());
        assert_eq!(told.load(Ordering::SeqCst), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A folder of its own for one test.
    fn tempfile_dir() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "pm-desktop-offline-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ))
    }
}
