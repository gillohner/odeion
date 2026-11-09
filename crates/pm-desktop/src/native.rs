//! The page's own player ("This phone") played by the app itself (`pm-app-player`), on Android:
//! the node's stream or file is decoded here and played through the phone's output with the same
//! path choice as the phone's own zone (Auto, bit-perfect, Exclusive USB, shared), instead of
//! through the web view's `<audio>`, which always goes through Android's mixer.
//!
//! The page calls the `native_*` commands (only those: the node's origin gets a capability for
//! them and nothing else, see [`allow_page`]), and hears the player's [`Status`] as a
//! `pmdesktop:native` event. The player takes the phone's output through the endpoint's owner, so
//! it and the node's zone on the same output never play at once.

use pm_app_player::{AppPlayer, LoadError, Status};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Manager};

/// The commands the node's page (and the offline page) may call: the player's and the
/// downloads' (src/offline.rs). build.rs lists them with the connect page's in the app's
/// manifest; [`allow_page`] allows only these for the page's origin.
pub const PAGE_COMMANDS: &[&str] = &[
    "native_load",
    "native_ahead",
    "native_play",
    "native_pause",
    "native_seek",
    "native_stop",
    "native_status",
    "downloads_state",
    "downloads_keep",
    "downloads_remove_all",
    "downloads_limit",
];

static PLAYER: OnceLock<AppPlayer> = OnceLock::new();
/// The origins the page may play from: the node's in use (its address, or the tunnel's) and the
/// offline page's. Each is replaced when it changes, so an old node's page plays nothing here.
static ORIGINS: Mutex<Origins> = Mutex::new(Origins {
    node: None,
    offline: None,
});

struct Origins {
    node: Option<String>,
    offline: Option<String>,
}

impl Origins {
    fn has(&self, origin: &str) -> bool {
        [&self.node, &self.offline]
            .into_iter()
            .flatten()
            .any(|o| o == origin)
    }
}

/// Which page [`allow_page`] is for.
#[derive(Clone, Copy)]
pub enum Page {
    Node,
    Offline,
}
/// Whether the player said it plays (PlayerService keeps the app awake for it).
static PLAYING: AtomicBool = AtomicBool::new(false);

/// Whether the app plays the page's music itself on this platform (Android only for now; the
/// desktop app's page keeps `<audio>`).
pub fn on() -> bool {
    cfg!(target_os = "android") && PLAYER.get().is_some()
}

/// The player on the endpoint's output (the phone has one), with the endpoint's owner.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn start(app: &AppHandle, endpoint: &Arc<pm_endpoint::Endpoint>) {
    let Some(output) = endpoint
        .outputs
        .list()
        .first()
        .and_then(|o| endpoint.outputs.get(&o.id))
    else {
        tracing::warn!("no output for the app's own player");
        return;
    };
    let player = AppPlayer::new(output, endpoint.owner().clone());
    let handle = app.clone();
    player.on_change(Arc::new(move |st: &Status| push(&handle, st)));
    if PLAYER.set(player).is_err() {
        tracing::debug!("the app's own player was already there");
    }
}

/// The page hears the player (and the player service whether it plays).
fn push(app: &AppHandle, st: &Status) {
    let playing = st.transport == pm_app_player::Transport::Playing;
    if PLAYING.swap(playing, Ordering::SeqCst) != playing {
        #[cfg(target_os = "android")]
        crate::android::app_playing(app, playing);
        tracing::info!("{}", log_line(st));
    } else if playing && LOGGED.swap(st.position_ms / 5_000, Ordering::SeqCst) != st.position_ms / 5_000 {
        // Where it is every 5 s (tools/android-endpoint.py measures the pace from these)
        tracing::info!(position_ms = st.position_ms, "the app's own player at");
    }
    if let Some(w) = app.get_webview_window(crate::MAIN) {
        let _ = w.eval(event_script(st));
    }
}

/// Which 5 s stretch the position was last logged in.
static LOGGED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);

/// "the app's own player: playing /v1/library/tracks/3/file, 44.1 kHz/16 → 44.1 kHz/16, path
/// bit_perfect, to FiiO KA17 (bit_perfect)" (the URL's path only: its query has the token).
fn log_line(st: &Status) -> String {
    let what = st
        .url
        .as_deref()
        .and_then(|u| url::Url::parse(u).ok())
        .map(|u| u.path().to_string())
        .unwrap_or_default();
    let path = st.now_playing.as_ref().map(|np| {
        format!(
            ", {} -> {}, path {}",
            np.source_format,
            np.output_format,
            serde_json::to_value(np.signal_path.headline())
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default()
        )
    });
    let dev = st.device.as_ref().map(|d| {
        format!(
            ", to {} ({:?}{})",
            d.name,
            d.mode,
            if d.confirmed { ", confirmed" } else { "" }
        )
    });
    format!(
        "the app's own player: {} {what}{}{}{}",
        serde_json::to_value(st.transport)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default(),
        path.unwrap_or_default(),
        dev.unwrap_or_default(),
        st.error.as_deref().map(|e| format!(" ({e})")).unwrap_or_default(),
    )
}

fn event_script(st: &Status) -> String {
    format!(
        "window.dispatchEvent(new CustomEvent('pmdesktop:native',{{detail:{}}}));",
        serde_json::to_string(st).unwrap_or_else(|_| "null".into())
    )
}

/// A page (re)loads: the player stops with the page that played it, as `<audio>` would.
pub fn page_changed() {
    if let Some(p) = PLAYER.get() {
        if p.status().url.is_some() {
            p.stop();
        }
    }
}

/// The page at `origin` may use the player (the node's page, or the offline page, each in place
/// of the one before): its own URLs may play, and its origin gets a capability for
/// [`PAGE_COMMANDS`] only (Tauri keeps a capability once added; the commands check the origin).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn allow_page(app: &AppHandle, origin: &str, page: Page) {
    if !on() || origin.is_empty() {
        return;
    }
    {
        let mut o = ORIGINS.lock().unwrap_or_else(|e| e.into_inner());
        let had = o.has(origin);
        let slot = match page {
            Page::Node => &mut o.node,
            Page::Offline => &mut o.offline,
        };
        *slot = Some(origin.to_string());
        if had {
            return;
        }
    }
    let mut cap = tauri::ipc::CapabilityBuilder::new(format!("page-player-{origin}"))
        .remote(format!("{origin}/*"))
        .window(crate::MAIN);
    for c in PAGE_COMMANDS {
        cap = cap.permission(format!("allow-{}", c.replace('_', "-")));
    }
    if let Err(e) = app.add_capability(cap) {
        tracing::warn!(%origin, "the page can't reach the app's player: {e}");
    }
}

/// Whether the page at `page` may play `url`: the URL is on the page's own origin, and that is
/// the node's in use or the offline page's.
fn may_play(url: &str, page: &str, origins: &Origins) -> bool {
    let origin = |s: &str| {
        url::Url::parse(s)
            .ok()
            .map(|u| (u.scheme().to_string(), u.origin().ascii_serialization()))
    };
    match (origin(url), origin(page)) {
        (Some((scheme, u)), Some((_, p))) => {
            matches!(scheme.as_str(), "http" | "https") && u == p && origins.has(&u)
        }
        _ => false,
    }
}

fn player() -> Result<&'static AppPlayer, String> {
    PLAYER.get().ok_or_else(|| "the app's player is off".into())
}

fn checked(url: &str, page: &tauri::Webview) -> Result<(), String> {
    let page = page.url().map(|u| u.to_string()).unwrap_or_default();
    if may_play(url, &page, &ORIGINS.lock().unwrap_or_else(|e| e.into_inner())) {
        Ok(())
    } else {
        Err("only the node's own streams play here".into())
    }
}

fn failed(message: impl Into<String>) -> LoadError {
    LoadError {
        refused: false,
        overtaken: false,
        message: message.into(),
    }
}

/// Not for the app's player at all (it is off, or the URL isn't the page's node's): the page
/// plays it itself.
fn not_here(message: impl Into<String>) -> LoadError {
    LoadError {
        refused: true,
        overtaken: false,
        message: message.into(),
    }
}

/// Play `url` from `at_ms`, `ahead` after it. `Err` with `refused`: something the app can't play
/// itself (the page plays it); else why it didn't start (the page tries again).
#[tauri::command]
pub async fn native_load(
    webview: tauri::Webview,
    url: String,
    at_ms: Option<u64>,
    ahead: Option<String>,
) -> Result<(), LoadError> {
    checked(&url, &webview).map_err(not_here)?;
    let ahead = ahead.filter(|a| checked(a, &webview).is_ok());
    let p = player().map_err(not_here)?.clone();
    tauri::async_runtime::spawn_blocking(move || p.load(&url, at_ms.unwrap_or(0), ahead.as_deref()))
        .await
        .map_err(|e| failed(e.to_string()))?
}

#[tauri::command]
pub fn native_ahead(webview: tauri::Webview, url: Option<String>) -> Result<(), String> {
    let url = url.filter(|u| checked(u, &webview).is_ok());
    player()?.ahead(url.as_deref());
    Ok(())
}

#[tauri::command]
pub async fn native_play() -> Result<(), LoadError> {
    let p = player().map_err(failed)?.clone();
    tauri::async_runtime::spawn_blocking(move || p.play())
        .await
        .map_err(|e| failed(e.to_string()))?
}

#[tauri::command]
pub fn native_pause() -> Result<(), String> {
    player()?.pause();
    Ok(())
}

#[tauri::command]
pub fn native_seek(ms: u64) -> Result<(), String> {
    player()?.seek(ms);
    Ok(())
}

#[tauri::command]
pub fn native_stop() -> Result<(), String> {
    player()?.stop();
    Ok(())
}

#[tauri::command]
pub fn native_status() -> Result<Status, String> {
    Ok(player()?.status())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_pages_own_origins_play() {
        let o = Origins {
            node: Some("http://192.168.1.5:7788".into()),
            offline: Some("http://127.0.0.1:41234".into()),
        };
        let node = "http://192.168.1.5:7788/album/lib-7";
        let offline = "http://127.0.0.1:41234/downloads";
        assert!(may_play(
            "http://192.168.1.5:7788/v1/library/tracks/3/file?token=x",
            node,
            &o
        ));
        assert!(may_play(
            "http://127.0.0.1:41234/v1/library/tracks/3/file",
            offline,
            &o
        ));
        // Another origin than the page's own, even an allowed one
        assert!(!may_play(
            "http://127.0.0.1:41234/v1/library/tracks/3/file",
            node,
            &o
        ));
        assert!(!may_play(
            "http://192.168.1.5:7789/v1/library/tracks/3/file",
            node,
            &o
        ));
        assert!(!may_play(
            "http://other.example/x.flac",
            "http://other.example/",
            &o
        ));
        assert!(!may_play("file:///data/x.flac", node, &o));
        assert!(
            !may_play("/v1/library/tracks/3/file", node, &o),
            "absolute URLs only"
        );
        // An old node's page, after another node was connected
        let now = Origins {
            node: Some("http://192.168.1.9:7788".into()),
            offline: None,
        };
        assert!(!may_play("http://192.168.1.5:7788/x", node, &now));
    }

    #[test]
    fn the_event_carries_the_status_as_data() {
        let st = Status {
            url: Some("http://n/x?</script>'\"".into()),
            transport: pm_app_player::Transport::Playing,
            position_ms: 5,
            duration_ms: None,
            ended: 2,
            advanced: 1,
            error: None,
            refused: None,
            taken: false,
            broke: false,
            now_playing: None,
            output: pm_app_player::OutputId::new("cpal", "Speaker or headphones"),
            device: None,
        };
        let s = event_script(&st);
        assert!(s.starts_with("window.dispatchEvent(new CustomEvent('pmdesktop:native',{detail:{"));
        assert!(s.contains(r#""url":"http://n/x?</script>'\"""#), "{s}");
        assert!(s.contains(r#""transport":"playing""#) && s.contains(r#""ended":2"#));
    }

    #[test]
    fn the_log_line_names_the_path_never_the_token() {
        let f = pm_model::AudioFormat::pcm(44_100, 16, 2);
        let mut path = pm_model::SignalPath::new();
        path.push(pm_model::PathStage::passthrough("decode", f));
        let st = Status {
            url: Some("http://192.168.1.5:7788/v1/library/tracks/3/file?token=secret-token".into()),
            transport: pm_app_player::Transport::Playing,
            position_ms: 0,
            duration_ms: Some(1000),
            ended: 0,
            advanced: 0,
            error: None,
            refused: None,
            taken: false,
            broke: false,
            now_playing: Some(pm_app_player::Playing {
                source_format: f,
                output_format: f,
                signal_path: path,
                format: None,
            }),
            output: pm_app_player::OutputId::new("cpal", "Speaker or headphones"),
            device: None,
        };
        let l = log_line(&st);
        assert!(
            l.starts_with("the app's own player: playing /v1/library/tracks/3/file, "),
            "{l}"
        );
        assert!(l.contains("path bit_perfect"), "{l}");
        assert!(!l.contains("secret-token"), "{l}");
    }

    /// Every page command is in the app's manifest (build.rs), or its capability names nothing.
    #[test]
    fn the_manifest_lists_the_page_commands() {
        let build = include_str!("../build.rs");
        for c in PAGE_COMMANDS {
            assert!(build.contains(&format!("\"{c}\"")), "{c} missing from build.rs");
        }
    }
}
