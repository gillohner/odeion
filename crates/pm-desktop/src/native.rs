//! The page's own player ("This phone") played by the app itself (`pm-app-player`), on Android:
//! the node's stream or file is decoded here and played through the phone's output with the same
//! path choice as the phone's own zone (Auto, bit-perfect, Exclusive USB, shared), instead of
//! through the web view's `<audio>`, which always goes through Android's mixer.
//!
//! The page calls the `native_*` commands (only those: the node's origin gets a capability for
//! them and nothing else, see [`allow_page`]), and hears the player's [`Status`] as a
//! `pmdesktop:native` event. The player takes the phone's output through the endpoint's owner, so
//! it and the node's zone on the same output never play at once.

use pm_app_player::{AppPlayer, Status};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Manager};

/// The commands the node's page (and the offline page) may call. build.rs lists them with the
/// connect page's in the app's manifest; [`allow_page`] allows only these for the page's origin.
pub const PAGE_COMMANDS: &[&str] = &[
    "native_load",
    "native_ahead",
    "native_play",
    "native_pause",
    "native_seek",
    "native_stop",
    "native_status",
];

static PLAYER: OnceLock<AppPlayer> = OnceLock::new();
/// The origins the page may play from (the node's, through the tunnel too; the offline page's).
static ORIGINS: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Whether the player said it plays (PlayerService keeps the app awake for it).
static PLAYING: AtomicBool = AtomicBool::new(false);

/// Whether the app plays the page's music itself on this platform (Android only for now; the
/// desktop app's page keeps `<audio>`).
pub fn on() -> bool {
    cfg!(target_os = "android")
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

/// The page at `origin` may use the player (the node's page, or the offline page): its URLs may
/// play, and its origin gets a capability for [`PAGE_COMMANDS`] only.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn allow_page(app: &AppHandle, origin: &str) {
    if !on() || origin.is_empty() {
        return;
    }
    {
        let mut o = ORIGINS.lock().unwrap_or_else(|e| e.into_inner());
        if o.iter().any(|x| x == origin) {
            return;
        }
        o.push(origin.to_string());
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

/// Whether `url` is on an origin the page may play from.
fn may_play(url: &str, origins: &[String]) -> bool {
    url::Url::parse(url).is_ok_and(|u| {
        matches!(u.scheme(), "http" | "https")
            && origins.iter().any(|o| *o == u.origin().ascii_serialization())
    })
}

fn player() -> Result<&'static AppPlayer, String> {
    PLAYER.get().ok_or_else(|| "the app's player is off".into())
}

fn checked(url: &str) -> Result<(), String> {
    let origins = ORIGINS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if may_play(url, &origins) {
        Ok(())
    } else {
        Err("only the node's own streams play here".into())
    }
}

/// Play `url` from `at_ms`, `ahead` after it. `Err`: it can't play here (the page plays it).
#[tauri::command]
pub async fn native_load(url: String, at_ms: Option<u64>, ahead: Option<String>) -> Result<(), String> {
    checked(&url)?;
    let ahead = ahead.filter(|a| checked(a).is_ok());
    let p = player()?.clone();
    tauri::async_runtime::spawn_blocking(move || p.load(&url, at_ms.unwrap_or(0), ahead.as_deref()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn native_ahead(url: Option<String>) -> Result<(), String> {
    let url = url.filter(|u| checked(u).is_ok());
    player()?.ahead(url.as_deref());
    Ok(())
}

#[tauri::command]
pub async fn native_play() -> Result<(), String> {
    let p = player()?.clone();
    tauri::async_runtime::spawn_blocking(move || p.play())
        .await
        .map_err(|e| e.to_string())?
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
        let o = vec![
            "http://192.168.1.5:7788".to_string(),
            "http://127.0.0.1:41234".to_string(),
        ];
        assert!(may_play(
            "http://192.168.1.5:7788/v1/library/tracks/3/file?token=x",
            &o
        ));
        assert!(may_play("http://127.0.0.1:41234/v1/library/tracks/3/file", &o));
        assert!(!may_play("http://192.168.1.5:7789/v1/library/tracks/3/file", &o));
        assert!(!may_play("http://other.example/x.flac", &o));
        assert!(!may_play("file:///data/x.flac", &o));
        assert!(!may_play("/v1/library/tracks/3/file", &o), "absolute URLs only");
        assert!(!may_play("http://192.168.1.5:7788/x", &[]));
    }

    #[test]
    fn the_event_carries_the_status_as_data() {
        let st = Status {
            url: Some("http://n/x?</script>'\"".into()),
            transport: pm_app_player::Transport::Playing,
            position_ms: 5,
            duration_ms: None,
            ended: 2,
            error: None,
            refused: None,
            taken: false,
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
            error: None,
            refused: None,
            taken: false,
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
