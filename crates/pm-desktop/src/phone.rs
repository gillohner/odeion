//! Phone mode (Android): the app without a node. After a Pubky Ring sign-in, `pm-offline`'s phone
//! backend runs on `127.0.0.1` and the window shows the web UI the app carries, with the person's
//! own TIDAL and Qobuz, played by the app's own player. The choice is remembered
//! (`phone.json` beside `node.json`), so the app opens in phone mode again until the person
//! connects a node (Settings › Connect to a node).
//!
//! The phone page gets the player's and the downloads' commands like the node's page, plus
//! [`PHONE_COMMANDS`], only for its own origin. The backend's key reaches the page the way a
//! node's token does: in `localStorage`, only on that origin.

use crate::AppState;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};

/// The commands only the phone page may call (besides the player's and the downloads').
pub const PHONE_COMMANDS: &[&str] = &["phone_leave"];

/// Phone mode runs on this platform.
pub const AVAILABLE: bool = cfg!(target_os = "android");

/// The person phone mode runs for.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct Saved {
    /// Their Pubky (z32).
    pub pubky: String,
}

/// The running backend: whose it is, its origin and key.
struct Running {
    pubky: String,
    origin: String,
    key: String,
    task: tokio::task::JoinHandle<()>,
    /// The backend itself (leaving phone mode ends its sync and its waits).
    phone: std::sync::Weak<pm_offline::phone::Phone>,
}

impl Running {
    /// Phone mode ends here: the server stops, and the backend's own tasks end
    /// (`Phone::close`). Opened again while a step of it still runs, the same backend comes back
    /// (`Phone::open`): never two for one person's folder.
    fn stop(self) {
        self.task.abort();
        if let Some(p) = self.phone.upgrade() {
            p.close();
        }
    }
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

fn saved_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("phone.json"))
}

/// Phone mode's folder for one person (their Pubky): their sign-ins, vault records and Pubky
/// session, apart from anyone else who used phone mode on this device.
fn dir(app: &AppHandle, pubky: &str) -> Result<PathBuf, String> {
    let base = app.path().app_local_data_dir().map_err(|e| e.to_string())?;
    person_dir(&base, pubky)
}

/// `<base>/phone/<pubky>`; only a Pubky's own letters and digits make a folder name.
fn person_dir(base: &std::path::Path, pubky: &str) -> Result<PathBuf, String> {
    if pubky.is_empty() || !pubky.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err("not a Pubky".into());
    }
    Ok(base.join("phone").join(pubky))
}

/// Where one person's TIDAL tracks are cached while they play: the app's cache folder (the
/// system may empty it).
fn cache_dir(app: &AppHandle, pubky: &str) -> Result<PathBuf, String> {
    let base = app.path().app_cache_dir().map_err(|e| e.to_string())?;
    Ok(person_dir(&base, pubky)?.with_file_name(format!("{pubky}-streams")))
}

pub fn load_saved(path: &std::path::Path) -> Option<Saved> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .filter(|s: &Saved| !s.pubky.is_empty())
}

fn store_saved(path: &std::path::Path, s: Option<&Saved>) -> Result<(), String> {
    match s {
        Some(s) => {
            if let Some(d) = path.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            std::fs::write(path, serde_json::to_vec(s).map_err(|e| e.to_string())?)
                .map_err(|e| format!("Can't remember phone mode: {e}"))
        }
        None => match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        },
    }
}

/// Writes the Pubky session's secret (it works like a password) readable by the app alone.
fn keep_session_secret(dir: &std::path::Path, secret: &str) -> Result<(), String> {
    use std::io::Write;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(dir.join("pubky-session")).map_err(|e| e.to_string())?;
    f.write_all(secret.as_bytes()).map_err(|e| e.to_string())
}

/// The key of the phone page at `origin`, if that is the running backend's.
pub fn key_for(origin: &str) -> Option<String> {
    RUNNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .filter(|r| r.origin == origin)
        .map(|r| r.key.clone())
}

/// The phone page's origin while phone mode runs.
pub fn origin() -> Option<String> {
    RUNNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|r| r.origin.clone())
}

/// Whether phone mode was chosen on this device (the connect page opens it again).
#[tauri::command]
pub fn phone_saved(app: AppHandle) -> Option<Saved> {
    if !AVAILABLE {
        return None;
    }
    saved_path(&app).and_then(|p| load_saved(&p))
}

/// Whether this build offers phone mode (the connect page shows "Use without a node").
#[tauri::command]
pub fn phone_available() -> bool {
    AVAILABLE
}

/// "Use without a node": Pubky Ring first (phone mode keeps settings, sign-ins and data on the
/// person's homeserver), then phone mode. The Ring address reaches the page as `ring-url`.
#[tauri::command]
pub async fn phone_ring(app: AppHandle, state: State<'_, AppState>) -> Result<Option<String>, String> {
    if !AVAILABLE {
        return Err("Phone mode is in the Android app".into());
    }
    let (hs, session) = crate::ring_session(&app, &state, pm_pubky::homeserver::PHONE_CAPABILITIES).await?;
    let saved = Saved {
        pubky: session.public_key().to_string(),
    };
    if let Some(secret) = pm_pubky::homeserver::export_session_secret(&session).await {
        keep_session_secret(&dir(&app, &saved.pubky)?, &secret)?;
    }
    drop(hs);
    let path = saved_path(&app).ok_or("no folder for the app's settings")?;
    store_saved(&path, Some(&saved))?;
    tracing::info!("phone mode: signed in with Pubky Ring");
    open(&app, &state, &saved, false).await
}

/// Where TIDAL is: TIDAL itself, or (Android emulator CI only, feature `phone-mode-fixture`)
/// the runner's stand-in at `ODEION_PHONE_FIXTURE_TIDAL`, with no built-in clients.
fn tidal_conf() -> pm_offline::phone::TidalConf {
    #[cfg(feature = "phone-mode-fixture")]
    {
        let base = env!("ODEION_PHONE_FIXTURE_TIDAL").trim_end_matches('/');
        pm_offline::phone::TidalConf {
            auth_base: base.into(),
            login_base: base.into(),
            v1_base: format!("{base}/v1"),
            browser: None,
            device: None,
        }
    }
    #[cfg(not(feature = "phone-mode-fixture"))]
    Default::default()
}

/// Android emulator CI only (feature `phone-mode-fixture`, tools/android-phone-mode.py): phone
/// mode is chosen and a TIDAL full-tracks session is on the phone before the first start, so the
/// check runs without Pubky Ring or a TIDAL account. Never in releases.
#[cfg(feature = "phone-mode-fixture")]
pub fn seed_fixture(app: &AppHandle) {
    use pm_sources::SessionStore;
    let Some(path) = saved_path(app) else { return };
    if load_saved(&path).is_some() {
        return;
    }
    let fixture = Saved {
        pubky: "o1gg96ewuojmopcjbz8895478wdtxtzzuxnfjjz8o8e77csa1ngo".into(),
    };
    let seeded = dir(app, &fixture.pubky).and_then(|d| {
        let store = pm_offline::phone::store::FileStore::open(&d).map_err(|e| e.to_string())?;
        let st = pm_sources::tidal::full::ModeState {
            enabled: true,
            quality: "high".into(),
            session: Some(pm_sources::tidal::full::Session {
                access_token: "phone-mode-fixture-token".into(),
                expires_at: pm_sources::now_unix() + 30 * 24 * 3600,
                country: "US".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let v = serde_json::to_string(&st).map_err(|e| e.to_string())?;
        store.update(pm_sources::tidal::full::STATE_KEY, &mut |_| Some(v.clone()));
        store_saved(&path, Some(&fixture))
    });
    match seeded {
        Ok(()) => tracing::info!("phone mode fixture: phone mode chosen, a TIDAL session seeded"),
        Err(e) => tracing::warn!("phone mode fixture: {e}"),
    }
}

/// Open phone mode as it was chosen before. `Some`: what the connect page tells the person first
/// (sign-ins set aside: `Phone::set_aside_notice`); phone mode runs, and this again with `seen`
/// (they saw it) shows its page.
#[tauri::command]
pub async fn open_phone(
    app: AppHandle,
    state: State<'_, AppState>,
    seen: Option<bool>,
) -> Result<Option<String>, String> {
    let saved = phone_saved(app.clone()).ok_or("Phone mode isn't set up on this device")?;
    open(&app, &state, &saved, seen.unwrap_or(false)).await
}

/// Starts the backend once and shows its page; or, while it holds sign-ins it couldn't read (set
/// aside: `Phone::set_aside_notice`) that the person hasn't seen yet (`seen`), says so first
/// (`Some`: shown on the connect page, whose Continue opens it again with `seen`).
async fn open(
    app: &AppHandle,
    state: &AppState,
    saved: &Saved,
    seen: bool,
) -> Result<Option<String>, String> {
    // The running backend is this person's and still serves; else a new one (the old one ends)
    let running = {
        let mut r = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        if r.as_ref()
            .is_some_and(|r| r.pubky != saved.pubky || r.task.is_finished())
        {
            if let Some(old) = r.take() {
                old.stop();
            }
        }
        r.as_ref().map(|r| (r.origin.clone(), r.phone.clone()))
    };
    let (origin, phone) = match running {
        Some(o) => o,
        None => {
            let phone = pm_offline::phone::Phone::open(pm_offline::phone::Config {
                dir: dir(app, &saved.pubky)?,
                cache: cache_dir(app, &saved.pubky)?,
                version: env!("CARGO_PKG_VERSION").into(),
                pubky: Some(saved.pubky.clone()),
                tidal: tidal_conf(),
            })
            .map_err(|e| format!("Phone mode can't start: {e}"))?;
            let key = phone.key().to_string();
            let weak = std::sync::Arc::downgrade(&phone);
            let (addr, task) = pm_offline::phone::serve(phone)
                .await
                .map_err(|e| format!("Phone mode can't start: {e}"))?;
            let origin = format!("http://127.0.0.1:{}", addr.port());
            *RUNNING.lock().unwrap_or_else(|e| e.into_inner()) = Some(Running {
                pubky: saved.pubky.clone(),
                origin: origin.clone(),
                key,
                task,
                phone: weak.clone(),
            });
            (origin, weak)
        }
    };
    // Sign-ins set aside (they couldn't be read): said before its page, until seen
    if let Some(p) = phone.upgrade() {
        if seen {
            if let Err(e) = p.set_aside_seen() {
                tracing::warn!("phone mode: {e}");
            }
        } else if let Some(notice) = p.set_aside_notice() {
            return Ok(Some(notice));
        }
    }
    // The navigation guard and the media controls know the local server by this
    state.set_local_origin(Some(origin.clone()));
    #[cfg(target_os = "android")]
    {
        crate::native::allow_page(app, &origin, crate::native::Page::Phone);
        crate::android::start_player_service(app, "Plays your music on this phone".into(), false);
    }
    let page = url::Url::parse(&format!("{origin}/search")).map_err(|e| e.to_string())?;
    app.get_webview_window(crate::MAIN)
        .ok_or("the window is gone")?
        .navigate(page)
        .map_err(|e| e.to_string())?;
    tracing::info!("phone mode: started");
    Ok(None)
}

/// Settings › Connect to a node: phone mode ends (its sign-ins stay on the phone for next time)
/// and the connect page opens. Only the phone page may ask.
#[tauri::command]
pub fn phone_leave(
    app: AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let here = webview
        .url()
        .map(|u| u.origin().ascii_serialization())
        .unwrap_or_default();
    if origin().as_deref() != Some(here.as_str()) {
        return Err("only the phone page leaves phone mode".into());
    }
    if let Some(p) = saved_path(&app) {
        store_saved(&p, None)?;
    }
    if let Some(r) = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).take() {
        r.stop();
    }
    state.set_local_origin(None);
    #[cfg(target_os = "android")]
    let _ = crate::native::native_stop();
    app.get_webview_window(crate::MAIN)
        .ok_or("the window is gone")?
        .navigate(crate::connect_page())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choice_is_remembered_until_the_person_connects_a_node() {
        let dir = std::env::temp_dir().join(format!("pm-phone-saved-{}", std::process::id()));
        let path = dir.join("phone.json");
        assert_eq!(load_saved(&path), None);
        let s = Saved {
            pubky: "o1gg96ewuojmopcjbz8895478wdtxtzzuxnfjjz8o8e77csa1ngo".into(),
        };
        store_saved(&path, Some(&s)).unwrap();
        assert_eq!(load_saved(&path), Some(s));
        store_saved(&path, None).unwrap();
        assert_eq!(load_saved(&path), None);
        store_saved(&path, None).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn each_person_has_a_folder_of_their_own() {
        let base = std::path::Path::new("/data");
        let a = person_dir(base, "o1gg96ewuojmopcjbz8895478wdtxtzzuxnfjjz8o8e77csa1ngo").unwrap();
        let b = person_dir(base, "8pinxxgqs41n4aididenw5apqp1urfmzdztr8jt4abrkdn435ewo").unwrap();
        assert_ne!(a, b);
        assert!(a.starts_with("/data/phone"));
        for bad in ["", "..", "a/b", "../x"] {
            assert!(person_dir(base, bad).is_err(), "{bad}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_pubky_session_is_readable_by_the_app_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("pm-phone-secret-{}", std::process::id()));
        keep_session_secret(&dir, "secret").unwrap();
        let mode = std::fs::metadata(dir.join("pubky-session"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "{mode:o}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
