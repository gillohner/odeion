//! The Odeion app: one native window around the node's web UI (the same UI
//! on every platform), built with Tauri 2 for macOS, Windows, Linux and
//! Android. The window starts on a small connect page (`ui/`): approving in
//! Pubky Ring lets the app find the node's address and token through the
//! owner's homeserver, or an address and token are typed by hand. Once
//! connected the window shows the node's own page.
//!
//! The app is also a player (`player`): it serves this machine's DACs (on
//! Android, whatever the phone plays through) to the node as a `pm-endpoint`
//! and registers them, so they are zones like any other.

#[cfg(target_os = "android")]
mod android;
mod crash;
mod node;
mod offline;
#[cfg(any(desktop, target_os = "android"))]
mod player;
mod ring_wait;
mod settings;
#[cfg(any(desktop, target_os = "android"))]
mod update;
#[cfg(any(test, target_os = "android"))]
mod update_apk;

use serde::Serialize;
use settings::Settings;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;

const MAIN: &str = "main";

/// The capability the app asks Ring for: read the owner's node record.
const RING_CAPS: &str = "/priv/pubky.music/:r";

struct AppState {
    /// Shared with the navigation guard, which runs outside commands.
    settings: Arc<Mutex<Option<Settings>>>,
    path: PathBuf,
    #[cfg(any(desktop, target_os = "android"))]
    player: Option<Arc<player::Player>>,
    /// The iroh tunnel to the node, while the node is reached through it (ADR 0013).
    tunnel: tokio::sync::Mutex<Option<pm_remote::ClientTunnel>>,
    /// Music kept on this device (`pm-offline`); `None` when its folder can't be opened.
    offline: Option<Arc<offline::Offline>>,
    /// The remembered node at the address a node claiming its id answered from on the LAN.
    moved: Mutex<Option<Settings>>,
}

/// Set while the offline page waits for the node to come back (one waiter at a time).
static RETURNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

impl AppState {
    fn current(&self) -> Option<Settings> {
        self.settings.lock().ok()?.clone()
    }

    /// Save `s` and make it the current settings, both under the settings lock: a registration's
    /// [`AppState::store_if_node`] can't write its file between this save and this set.
    fn store(&self, s: Option<Settings>) {
        store_in(&self.settings, &self.path, s);
    }

    /// Store `s` only while the current settings still name `node_url`: the check and the store
    /// under one lock, so a registration that finishes after the app connected to another node
    /// never writes that node's settings over the new one's. Whether it was stored.
    fn store_if_node(&self, node_url: &str, s: Settings) -> bool {
        store_if_node(&self.settings, &self.path, node_url, s)
    }
}

/// [`AppState::store`] on the settings cell itself (also for tests).
fn store_in(cell: &Mutex<Option<Settings>>, path: &std::path::Path, s: Option<Settings>) {
    let mut cur = cell.lock().unwrap_or_else(PoisonError::into_inner);
    if let Err(e) = settings::save(path, s.as_ref()) {
        tracing::warn!("cannot save the app settings: {e}");
    }
    *cur = s;
}

/// [`AppState::store_if_node`] on the settings cell itself (also for tests).
fn store_if_node(
    cell: &Mutex<Option<Settings>>,
    path: &std::path::Path,
    node_url: &str,
    s: Settings,
) -> bool {
    let Ok(mut cur) = cell.lock() else {
        return false;
    };
    if cur.as_ref().map(|c| c.node_url.as_str()) != Some(node_url) {
        return false;
    }
    if let Err(e) = settings::save(path, Some(&s)) {
        tracing::warn!("cannot save the app settings: {e}");
    }
    *cur = Some(s);
    true
}

/// Links that would open a second window (`target="_blank"`, a
/// cross-origin `window.open`) become plain navigations, which the
/// navigation guard sends to the system browser. WebKitGTK does not always
/// raise a new-window request for them, and Android never does. An `about:`
/// window (`about:blank`, origin "null") opens nothing: as a navigation it
/// would replace the app's own page, which the guard allows.
const EXTERNAL_LINKS_JS: &str = "document.addEventListener('click',function(e){var a=e.target&&e.target.closest&&e.target.closest('a[target=_blank]');if(a&&a.href&&!e.defaultPrevented){e.preventDefault();location.assign(a.href);}});var wo=window.open;window.open=function(u){try{var x=new URL(u,location.href);if(x.protocol==='about:')return null;if(x.origin!==location.origin){location.assign(x.href);return null;}}catch(e){}return wo.apply(window,arguments);};";

/// Run before every page: the node's page gets the token in the
/// localStorage key the web UI reads (only on the node's origin), and every
/// page can see it runs inside the app.
fn init_script(s: Option<&Settings>) -> String {
    let (origin, token) = s
        .and_then(|s| Some((s.origin()?, s.token.clone())))
        .unwrap_or_default();
    format!(
        "(function(){{var o={o},t={t};if(o&&location.origin===o){{try{{localStorage.setItem('pm.token',t);}}catch(e){{}}}}window.pmDesktop={{version:{v},platform:{p}}};{EXTERNAL_LINKS_JS}}})();",
        o = serde_json::to_string(&origin).unwrap_or_default(),
        t = serde_json::to_string(&token).unwrap_or_default(),
        v = serde_json::to_string(env!("CARGO_PKG_VERSION")).unwrap_or_default(),
        p = serde_json::to_string(std::env::consts::OS).unwrap_or_default(),
    )
}

/// For a node connected after the window was built: the init script holds
/// the old token, so put the new one in place once and reload.
fn token_sync_script(token: &str) -> String {
    format!(
        "(function(t){{try{{if(localStorage.getItem('pm.token')!==t){{localStorage.setItem('pm.token',t);location.reload();}}}}catch(e){{}}}})({});",
        serde_json::to_string(token).unwrap_or_default()
    )
}

/// Whether reaching the node failed because it wants a new sign-in from this device (it refused
/// the saved one, or there is none): trying again won't help, signing in again does.
fn needs_new_sign_in(error: &str) -> bool {
    error == node::REFUSED || error == node::NEEDS_SIGN_IN
}

/// The app's connect page (`ui/index.html`), where the window starts.
fn connect_page() -> url::Url {
    let page = if cfg!(any(windows, target_os = "android")) {
        "http://tauri.localhost/index.html"
    } else {
        "tauri://localhost/index.html"
    };
    url::Url::parse(page).expect("a fixed URL")
}

/// The app's own pages: `tauri://localhost` on macOS and Linux,
/// `http(s)://tauri.localhost` on Windows and Android.
fn is_app_page(u: &url::Url) -> bool {
    u.scheme() == "tauri" || u.host_str() == Some("tauri.localhost") || u.scheme() == "about"
}

/// Navigation stays on the app's pages and the node's origin; anything else
/// (an "Open on …" link, a streaming service's sign-in) opens in the
/// system browser.
fn allow_navigation(
    app: &AppHandle,
    current: &Arc<Mutex<Option<Settings>>>,
    offline_origin: &Arc<Mutex<Option<String>>>,
    u: &url::Url,
) -> bool {
    tracing::debug!(url = %u, "navigation");
    if is_app_page(u) {
        return true;
    }
    // The offline page (pm-offline's server on 127.0.0.1)
    let here = u.origin().ascii_serialization();
    if offline_origin.lock().ok().and_then(|o| o.clone()).as_deref() == Some(here.as_str()) {
        return true;
    }
    let node_origin = current.lock().ok().and_then(|s| s.as_ref()?.origin());
    if node_origin.as_deref() == Some(u.origin().ascii_serialization().as_str()) {
        return true;
    }
    if matches!(
        u.scheme(),
        "http" | "https" | "mailto" | "pubkyauth" | "pubkyring"
    ) {
        open_external(app, u.as_str());
    }
    false
}

/// Open a link in the system browser or the app that handles it (Pubky Ring for `pubkyauth://`).
/// The opener is a mobile plugin on Android: its call runs on the main thread and waits there for
/// the answer, so it is never called from a main-thread callback (navigation, new-window, sync
/// commands) but from a blocking worker, and the caller returns at once.
fn open_external(app: &AppHandle, url: &str) {
    let app = app.clone();
    let url = url.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = app.opener().open_url(&url, None::<&str>) {
            tracing::warn!(%url, "cannot open externally: {e}");
        }
    });
}

fn show_node(app: &AppHandle, s: &Settings) -> Result<(), String> {
    let w = app
        .get_webview_window(MAIN)
        .ok_or_else(|| "the window is gone".to_string())?;
    let u = url::Url::parse(&format!("{}/", s.base())).map_err(|e| format!("Not an address: {e}"))?;
    w.navigate(u).map_err(|e| e.to_string())
}

/// How the player service runs for a connection: its notification's text, and whether the phone
/// is a zone of the node (the node reaches its player directly: the service runs while connected)
/// or the app plays the music itself (through the tunnel, or without a player: the service ends
/// after a while with nothing playing out of sight). Started either way.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn player_service(node_name: &str, direct: bool, native_player: bool) -> (String, bool) {
    let node = if node_name.is_empty() {
        "your node"
    } else {
        node_name
    };
    if direct && native_player {
        (format!("This phone is a zone of {node}"), true)
    } else {
        (format!("Plays music from {node}"), false)
    }
}

/// Connected: remember the node, show it and register the player.
fn connected(app: &AppHandle, state: &AppState, mut s: Settings, info: &node::Info) -> Result<(), String> {
    take_moved(state);
    if !info.id.is_empty() {
        s.node_id = info.id.clone();
    }
    if let Some(old) = state.current() {
        if old.node_url == s.node_url {
            s.player_address = old.player_address;
        }
    }
    state.store(Some(s.clone()));
    show_node(app, &s)?;
    // Keep this device's downloads in step with the current node (read again at every round)
    if let Some(o) = &state.offline {
        let app = app.clone();
        o.spawn_sync(move || app.state::<AppState>().current());
    }
    // The player service keeps the app playing in the background (from Android 12 a cached app
    // is frozen within seconds): the phone's own zone needs it, and so does the in-app player,
    // through the tunnel too, where the service used to be left out and the music stopped a few
    // seconds after leaving the app (TASK-android-background-playback-1105).
    #[cfg(target_os = "android")]
    {
        let (text, zone) = player_service(&info.name, s.via.is_empty(), state.player.is_some());
        android::start_player_service(app, text, zone);
    }
    // Through the tunnel the node is far away and cannot reach this machine's player.
    #[cfg(any(desktop, target_os = "android"))]
    if s.via.is_empty() {
        tauri::async_runtime::spawn(register_player(app.clone(), s));
    }
    Ok(())
}

/// Register the player with the node in `s`, keep its address, and tell the node's page which
/// output is this app's (`player::output_script`). Again whenever the phone's USB DAC changes, so
/// the node learns it.
#[cfg(any(desktop, target_os = "android"))]
async fn register_player(app: AppHandle, s: Settings) {
    let player = app.state::<AppState>().player.clone();
    let s = player::register(s, player).await;
    let state = app.state::<AppState>();
    let node_url = s.node_url.clone();
    if state.store_if_node(&node_url, s) {
        let script = player::output_script();
        if let (false, Some(w)) = (script.is_empty(), app.get_webview_window(MAIN)) {
            let _ = w.eval(script);
        }
    }
}

/// Whether this device keeps any music (the connect page offers it while the node is away).
#[tauri::command]
fn has_downloads(state: State<'_, AppState>) -> bool {
    let node_id = state.current().map(|s| s.node_id).unwrap_or_default();
    state.offline.as_ref().is_some_and(|o| o.has_downloads(&node_id))
}

/// The node is out of reach: play what this device keeps, and go back to the node as soon as it
/// answers again.
#[tauri::command]
async fn open_offline(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let o = state
        .offline
        .clone()
        .ok_or("Offline music is off on this device")?;
    let node_id = state.current().map(|s| s.node_id).unwrap_or_default();
    if !o.has_downloads(&node_id) {
        return Err("Nothing is downloaded to this device yet".into());
    }
    let page = url::Url::parse(&o.page(&node_id).await?).map_err(|e| e.to_string())?;
    app.get_webview_window(MAIN)
        .ok_or("the window is gone")?
        .navigate(page)
        .map_err(|e| e.to_string())?;
    // The offline app plays in the page too: the service keeps it going in the background
    #[cfg(target_os = "android")]
    android::start_player_service(&app, "Plays the music kept on this phone".into(), false);
    if !RETURNING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let state = app.state::<AppState>();
                let Some(s) = state.current() else { break };
                match reach(&app, &state, s).await {
                    Ok((s, info)) => {
                        if let Err(e) = connected(&app, &state, s, &info) {
                            tracing::warn!("back from offline: {e}");
                        }
                        break;
                    }
                    // The node answers but no longer takes this device's sign-in (a new token, the
                    // one-time change after the update): trying again never helps. Back to the
                    // connect page, which reconnects and signs in again (`open_saved`)
                    Err(e) if needs_new_sign_in(&e) => {
                        tracing::info!("back from offline: the node asks for a new sign-in");
                        if let Some(w) = app.get_webview_window(MAIN) {
                            if let Err(e) = w.navigate(connect_page()) {
                                tracing::warn!("back from offline: {e}");
                            }
                        }
                        break;
                    }
                    Err(_) => {}
                }
            }
            RETURNING.store(false, std::sync::atomic::Ordering::SeqCst);
        });
    }
    Ok(())
}

/// The crash report from last time, for the connect page (`crash.rs`).
#[tauri::command]
fn crash_report() -> Option<String> {
    crash::read()
}

#[tauri::command]
fn crash_dismiss() {
    crash::clear();
}

/// The system share sheet (Android); elsewhere the page copies the text itself.
#[tauri::command]
#[allow(unused_variables)]
fn crash_share(app: AppHandle, text: String) -> Result<(), String> {
    #[cfg(target_os = "android")]
    return android::share_text(&app, text);
    #[cfg(not(target_os = "android"))]
    Err("copy the report instead".into())
}

#[derive(Serialize)]
struct Saved {
    node_url: String,
}

/// The node remembered from last time, if any.
#[tauri::command]
fn saved(state: State<'_, AppState>) -> Option<Saved> {
    state.current().map(|s| Saved { node_url: s.node_url })
}

/// Reconnect to the remembered node: check it answers, then show it. A node that refuses this
/// device's sign-in (it ended there: Make a new token, or the node signed in as someone else) gets
/// a new one, with one approval in Pubky Ring, instead of a dead end.
#[tauri::command]
async fn open_saved(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let s = state.current().ok_or("No node yet")?;
    let again = s.clone();
    match reach(&app, &state, s).await {
        Ok((s, info)) => {
            connected(&app, &state, s, &info)?;
            Ok(info.name)
        }
        // The node refused this device's sign-in, or now asks for one it never had: a new one
        Err(e) if needs_new_sign_in(&e) => {
            if again.owner.is_empty() {
                // Saved before the app knew whose node it is (or connected by address): find the
                // node through the owner's record again, so its sign-in is checked against them
                return ring_connect(app, state).await;
            }
            let mut s = again;
            s.token.clear();
            s.via.clear();
            sign_in_and_connect(&app, &state, s).await
        }
        Err(e) => Err(e),
    }
}

/// Reach the node: at its address; else through the iroh tunnel when its record carries a key
/// (away from home, or moved: the tunnel dials the node by its key, so it proves the node is this
/// one); else, when a node on the LAN claims its id from another address, ask the person
/// ([`MOVED`], then [`follow_moved`]).
async fn reach(app: &AppHandle, state: &AppState, mut s: Settings) -> Result<(Settings, node::Info), String> {
    // TLS (an https node, the iroh relays, pkarr) needs the platform verifier, set up on the UI thread
    #[cfg(target_os = "android")]
    android::tls_ready().await;
    s.via.clear();
    let msg = match node::probe(&s).await {
        Ok(info) => {
            state.tunnel.lock().await.take();
            return Ok((s, info));
        }
        Err(node::ProbeError::Unreachable(msg)) => msg,
        Err(e) => return Err(e.into()),
    };
    let mut moved = None;
    if !s.node_id.is_empty() {
        let found = discover(app).await;
        match after_lan_search(&state.moved, &s, &found).await {
            Some(Ok(done)) => return Ok(done),
            Some(Err(ask)) => moved = Some(ask),
            None => {}
        }
    }
    match through_tunnel(state, s, msg).await {
        Ok(done) => {
            take_moved(state);
            Ok(done)
        }
        Err(e) => Err(moved.unwrap_or(e)),
    }
}

/// The node through the iroh tunnel, when its record carries a key; `msg` says why the address
/// did not do.
async fn through_tunnel(
    state: &AppState,
    mut s: Settings,
    msg: String,
) -> Result<(Settings, node::Info), String> {
    if s.iroh.is_empty() {
        return Err(msg);
    }
    let id: pm_remote::EndpointId = s.iroh.parse().map_err(|_| msg.clone())?;
    let tunnel = pm_remote::ClientTunnel::start(id, &pm_remote::Relays::Public)
        .await
        .map_err(|e| format!("{msg}, and remote access failed: {e}"))?;
    let route = tunnel
        .connect()
        .await
        .map_err(|e| format!("{msg}, and the node did not answer remotely either: {e}"))?;
    tracing::info!(?route, "reaching the node over iroh");
    s.via = format!("http://{}", tunnel.local_addr());
    let info = node::probe(&s).await?;
    if let Some(old) = state.tunnel.lock().await.replace(tunnel) {
        old.close().await;
    }
    Ok((s, info))
}

/// Starts the error that asks the connect page whether to use a new address; the address follows.
const MOVED: &str = "Moved: ";

/// After the LAN search, for a remembered node that did not answer at its address: a node that
/// claims its id from another address. The id is announced in the clear (mDNS, SSDP), so anyone on
/// the network can claim it, and a probe there would hand over the saved token. So nothing is sent
/// there: the address is kept for the person to confirm on the connect page ([`follow_moved`])
/// (security review 2026-10-27, #3).
async fn after_lan_search(
    moved: &Mutex<Option<Settings>>,
    s: &Settings,
    found: &[pm_discovery::FoundNode],
) -> Option<Result<(Settings, node::Info), String>> {
    let n = found.iter().find(|n| n.id == s.node_id && n.url != s.node_url)?;
    tracing::info!(from = %s.node_url, to = %n.url, "a node claims the remembered id at another address; asking first");
    let mut next = s.clone();
    next.node_url = n.url.clone();
    *moved.lock().unwrap_or_else(PoisonError::into_inner) = Some(next);
    Some(Err(format!("{MOVED}{}", n.url)))
}

fn take_moved(state: &AppState) -> Option<Settings> {
    state.moved.lock().unwrap_or_else(PoisonError::into_inner).take()
}

/// The person said yes to the new address the connect page showed: the token goes there now, and
/// the node there must have the remembered id.
#[tauri::command]
async fn follow_moved(
    app: AppHandle,
    state: State<'_, AppState>,
    node_url: String,
) -> Result<String, String> {
    let s = take_moved(&state)
        .filter(|m| m.node_url == node_url)
        .ok_or("That address is no longer on offer. Try again.")?;
    let info = node::probe(&s).await?;
    if !s.node_id.is_empty() && info.id != s.node_id {
        return Err(format!("The node at {} is a different node", s.node_url));
    }
    connected(&app, &state, s, &info)?;
    Ok(info.name)
}

/// Nodes on the LAN (mDNS and SSDP, `pm-discovery`).
async fn discover(app: &AppHandle) -> Vec<pm_discovery::FoundNode> {
    #[cfg(target_os = "android")]
    let lock = android::MulticastLock::acquire(app).await;
    #[cfg(not(target_os = "android"))]
    let _ = app;
    let found =
        tauri::async_runtime::spawn_blocking(|| pm_discovery::browse(std::time::Duration::from_millis(2500)))
            .await
            .unwrap_or_default();
    #[cfg(target_os = "android")]
    drop(lock);
    found
}

/// The connect page's "On this network" list.
#[tauri::command]
async fn discover_nodes(app: AppHandle) -> Vec<pm_discovery::FoundNode> {
    discover(&app).await
}

/// An address and token typed by hand.
#[tauri::command]
async fn connect_manual(
    app: AppHandle,
    state: State<'_, AppState>,
    node_url: String,
    token: String,
    expect_id: Option<String>,
) -> Result<String, String> {
    if node_url.trim().is_empty() {
        return Err("Enter the node's address".into());
    }
    let s = Settings::new(&node_url, &token);
    let info = node::probe(&s).await?;
    // The node found on the network, tried at 127.0.0.1: only when the node there is that one
    if let Some(want) = expect_id.as_deref() {
        if !same_node(want, &info.id) {
            return Err(NOT_THAT_NODE.into());
        }
    }
    connected(&app, &state, s, &info)?;
    Ok(info.name)
}

/// What `connect_manual` answers when the node at the address isn't the one asked for.
const NOT_THAT_NODE: &str = "Another node answers on this computer";

/// Whether the node that answered (`got`, its `/v1/info` id) is the one found on the network
/// (`want`). A node without an id is never taken for another.
fn same_node(want: &str, got: &str) -> bool {
    !want.is_empty() && want == got
}

#[derive(Clone, Serialize)]
struct RingUrl {
    url: String,
    /// The URL as a QR code, for Ring on a phone.
    qr_svg: String,
}

fn qr_svg(data: &str) -> String {
    qrcode::QrCode::new(data.as_bytes())
        .map(|c| {
            c.render::<qrcode::render::svg::Color>()
                .min_dimensions(220, 220)
                .quiet_zone(true)
                .build()
        })
        .unwrap_or_default()
}

/// Sign in with Pubky Ring, asking only to read the node's private folder;
/// the URL goes to the page as a `ring-url` event; on approval the owner's
/// node record gives the address and token.
#[tauri::command]
async fn ring_connect(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    #[cfg(target_os = "android")]
    android::tls_ready().await;
    let hs =
        pm_pubky::homeserver::NodeHomeserver::new(pm_pubky::Network::Mainnet).map_err(|e| e.to_string())?;
    // Ends the wait after ring_wait::DEADLINE, or on the page's Cancel (ring_cancel).
    let stop = ring_wait::Stop::begin(ring_wait::DEADLINE);
    // The newest homeserver records first (the identity this device connected with before, when
    // known), so the grant exchange routes on them and not on the first record a node answered
    let identity = state
        .current()
        .and_then(|c| pm_pubky::homeserver::PublicKey::try_from(c.owner.as_str()).ok());
    let records = hs.fresh_for_sign_in(identity.as_ref(), None, false).await?;
    tracing::info!(?records, "Ring: homeserver records before the sign-in");
    let flow = hs
        .start_login_with("pubky-music-desktop", RING_CAPS)
        .map_err(|e| e.to_string())?;
    // The relay listener runs from here: trips away from now on can stop it.
    let since_trip = ring_wait::trips();
    let url = flow.url().to_string();
    let relay = ring_wait::relay_host(&url);
    let _ = app.emit(
        "ring-url",
        RingUrl {
            qr_svg: qr_svg(&url),
            url,
        },
    );
    // Only on screen and online: Android 15+ cuts a backgrounded app's network (see ring_wait).
    let page = app.clone();
    let session = ring_wait::approval(
        RingFlow {
            flow,
            hs: hs.clone(),
            identity,
        },
        since_trip,
        &relay,
        443,
        stop,
        move |s| {
            let _ = page.emit("ring-state", s);
        },
    )
    .await?;
    let record = hs
        .read_node_record(&session)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("This identity has no node yet: sign the node in once on its web page (Settings → Pubky)")?;
    // The record says where the node is, never how to sign in to it: this device signs in to the
    // node itself with Pubky Ring and gets a credential of its own (ADR 0008)
    let mut s = Settings::new(&record.api_url, "");
    s.iroh = record.iroh;
    // The identity whose record named this node: the node must say it is theirs, and only their
    // approval signs this device in
    s.owner = session.public_key().to_string();
    sign_in_and_connect(&app, &state, s).await
}

/// Reach the node at `s` (its address, else through the iroh tunnel) without a token, sign this
/// device in with Pubky Ring, and show it. A node that asks for no token (on this computer) is
/// shown at once.
async fn sign_in_and_connect(app: &AppHandle, state: &AppState, mut s: Settings) -> Result<String, String> {
    match node::probe(&s).await {
        Ok(info) => {
            state.tunnel.lock().await.take();
            connected(app, state, s, &info)?;
            return Ok(info.name);
        }
        Err(node::ProbeError::NeedsSignIn) => {
            state.tunnel.lock().await.take();
        }
        Err(node::ProbeError::Unreachable(msg)) => s = tunnel_for_sign_in(state, s, msg).await?,
        Err(e) => return Err(e.into()),
    }
    s.token = sign_in_at_node(app, &s).await?;
    let info = node::probe(&s).await?;
    connected(app, state, s, &info)?;
    Ok(info.name)
}

/// The node found through the homeserver, reached over iroh when its address doesn't answer from
/// here, before this device has signed in to it ([`through_tunnel`] for a device that has).
async fn tunnel_for_sign_in(state: &AppState, mut s: Settings, msg: String) -> Result<Settings, String> {
    if s.iroh.is_empty() {
        return Err(msg);
    }
    let id: pm_remote::EndpointId = s.iroh.parse().map_err(|_| msg.clone())?;
    let tunnel = pm_remote::ClientTunnel::start(id, &pm_remote::Relays::Public)
        .await
        .map_err(|e| format!("{msg}, and remote access failed: {e}"))?;
    tunnel
        .connect()
        .await
        .map_err(|e| format!("{msg}, and the node did not answer remotely either: {e}"))?;
    s.via = format!("http://{}", tunnel.local_addr());
    match node::probe(&s).await {
        Ok(_) | Err(node::ProbeError::NeedsSignIn) => {}
        Err(e) => return Err(e.into()),
    }
    if let Some(old) = state.tunnel.lock().await.replace(tunnel) {
        old.close().await;
    }
    Ok(s)
}

/// Signs this device in to the node with Pubky Ring (the node's own sign-in, as its web page
/// does): the QR and link go to the page as a `ring-url` event, like the homeserver step's. Waits
/// until the owner approves, the page's Cancel, or [`ring_wait::DEADLINE`]; polls only while the
/// app is on screen (Android cuts a backgrounded app's network).
async fn sign_in_at_node(app: &AppHandle, s: &Settings) -> Result<String, String> {
    let stop = ring_wait::Stop::begin(ring_wait::DEADLINE);
    let started = node::start_sign_in(s, &s.owner).await?;
    let _ = app.emit(
        "ring-url",
        RingUrl {
            qr_svg: qr_svg(&started.url),
            url: started.url.clone(),
        },
    );
    loop {
        if let Some(why) = stop.reason() {
            return Err(why.to_string());
        }
        if ring_wait::on_screen() {
            if let Some(token) = node::poll_sign_in(s, &started).await? {
                // Cancel pressed while that answer was on its way: not connected
                if let Some(why) = stop.reason() {
                    return Err(why.to_string());
                }
                return Ok(token);
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// A Ring sign-in in progress, for [`ring_wait::approval`].
struct RingFlow {
    flow: pm_pubky::homeserver::LoginFlow,
    /// The client the flow runs on (its records are the ones the exchange routes on).
    hs: pm_pubky::homeserver::NodeHomeserver,
    /// The identity this device connected with before, when known.
    identity: Option<pm_pubky::homeserver::PublicKey>,
}

impl ring_wait::Pending for RingFlow {
    type Done = pm_pubky::homeserver::PubkySession;
    /// An error that came before the poll waited for anything is the relay listener's own (no
    /// grant was sent): see [`ring_wait::at_once`].
    async fn poll(&self) -> Result<Option<Self::Done>, ring_wait::Failed> {
        let (r, at_once) = ring_wait::at_once(self.flow.poll()).await;
        r.map_err(|e| {
            ring_wait::Failed::sort(
                ring_wait::chain(&e),
                at_once,
                e.is_transient(),
                e.is_server_error(),
                &ring_wait::relay_host(self.flow.url()),
            )
        })
    }
    /// A fresh client: nothing it holds comes from the time in the background.
    fn resume(&self) -> Result<Self, String> {
        let hs = pm_pubky::homeserver::NodeHomeserver::new(pm_pubky::Network::Mainnet)
            .map_err(|e| e.to_string())?;
        let flow = hs.resume_login(&self.flow).map_err(|e| e.to_string())?;
        Ok(RingFlow {
            flow,
            hs,
            identity: self.identity.clone(),
        })
    }
    /// Back on screen: the newest records again before the exchange (the ones held from before
    /// the app went away, and pubky's transport choice on them, may be behind).
    async fn returned(&self) -> Result<(), String> {
        let records = self
            .hs
            .fresh_for_sign_in(self.identity.as_ref(), None, false)
            .await?;
        tracing::info!(?records, "Ring: homeserver records on return");
        Ok(())
    }
}

/// The connect page's Cancel: the running Ring sign-in stops waiting (`ring_connect` then fails
/// with [`ring_wait::CANCELLED`]).
#[tauri::command]
fn ring_cancel() {
    ring_wait::cancel();
}

/// Hand a `pubkyauth://` link to Pubky Ring on this device.
/// Async so that it never blocks the IPC thread (the UI thread on desktop) while the opener waits
/// for Android's main thread (see [`open_external`]).
#[tauri::command]
async fn open_ring(app: AppHandle, url: String) -> Result<(), String> {
    if !url.starts_with("pubkyauth://") {
        return Err("not a Pubky Ring link".into());
    }
    tauri::async_runtime::spawn_blocking(move || app.opener().open_url(url, None::<&str>))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Forget the node (the connect page's "Connect to another node").
#[tauri::command]
fn forget(app: AppHandle, state: State<'_, AppState>) {
    state.store(None);
    // No node: the player service ends (and isn't started again when the app comes back)
    #[cfg(target_os = "android")]
    android::stop_player_service(&app);
    #[cfg(not(target_os = "android"))]
    let _ = app;
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // First, while this is the only thread: ODEION_ settings onto the PM_ names the code reads.
    // Not on Android, where the app starts on a thread of a running process and reads no settings.
    #[cfg(not(target_os = "android"))]
    let env_compat = pm_model::brand::apply_env_compat();
    crash::install_hook();
    // WebKitGTK's DMA-BUF renderer crashes the web process on some Linux
    // GPU stacks (NVIDIA, some Wayland compositors); the software path is
    // reliable and fast enough for this UI. An explicit setting wins.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    let filter = || tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    #[cfg(not(target_os = "android"))]
    let _ = tracing_subscriber::fmt().with_env_filter(filter()).try_init();
    #[cfg(target_os = "android")]
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter())
        .with_writer(android::Logcat)
        .with_ansi(false)
        .without_time()
        .try_init();
    #[cfg(not(target_os = "android"))]
    for line in env_compat.log_lines() {
        tracing::info!("{line}");
    }

    let builder = tauri::Builder::default().plugin(tauri_plugin_opener::init());
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init());
    let app = builder
        .invoke_handler(tauri::generate_handler![
            saved,
            open_saved,
            connect_manual,
            follow_moved,
            discover_nodes,
            ring_connect,
            ring_cancel,
            open_ring,
            forget,
            has_downloads,
            open_offline,
            crash_report,
            crash_dismiss,
            crash_share
        ])
        .setup(|app| {
            if let Ok(dir) = app.path().app_local_data_dir() {
                crash::set_dir(dir);
            }
            let path = app.path().app_config_dir()?.join("node.json");
            // ODEION_NODE/ODEION_TOKEN (or PM_NODE/PM_TOKEN) for scripts, then this app's file, then the
            // wry shell's file from before.
            let initial = std::env::var("PM_NODE")
                .ok()
                .map(|n| Settings::new(&n, &std::env::var("PM_TOKEN").unwrap_or_default()))
                .or_else(|| settings::load(&path))
                .or_else(|| settings::legacy_path().and_then(|p| settings::load(&p)));
            #[cfg(any(desktop, target_os = "android"))]
            let player = crash::guarded("the in-app player", player::start).flatten();
            let shared = Arc::new(Mutex::new(initial.clone()));
            let offline_origin: Arc<Mutex<Option<String>>> = Arc::default();
            let offline = app
                .path()
                .app_local_data_dir()
                .ok()
                .and_then(|d| offline::Offline::open(d.join("offline"), offline_origin.clone()));
            app.manage(AppState {
                settings: shared.clone(),
                path,
                #[cfg(any(desktop, target_os = "android"))]
                player,
                tunnel: Default::default(),
                offline,
                moved: Mutex::new(None),
            });

            let handle = app.handle().clone();
            let guard = shared.clone();
            let loads = shared;
            #[allow(unused_mut)]
            let mut builder = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
                .initialization_script(init_script(initial.as_ref()))
                .on_navigation(move |u| allow_navigation(&handle, &guard, &offline_origin, u))
                .on_page_load(move |w, payload| {
                    if !matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                        return;
                    }
                    let s = loads.lock().ok().and_then(|s| s.clone());
                    let node_page = s.as_ref().is_some_and(|s| {
                        s.origin().as_deref() == Some(payload.url().origin().ascii_serialization().as_str())
                    });
                    // Back never leaves the node's page for the connect page before it (which would
                    // connect again and register the phone's player once more)
                    #[cfg(target_os = "android")]
                    android::page_finished(&w, node_page);
                    let Some(s) = s else {
                        return;
                    };
                    if node_page {
                        let _ = w.eval(token_sync_script(&s.token));
                        // Which output is this app's (a phone's USB DAC shows in its Settings)
                        #[cfg(any(desktop, target_os = "android"))]
                        {
                            let script = player::output_script();
                            if !script.is_empty() {
                                let _ = w.eval(script);
                            }
                        }
                    }
                });
            #[cfg(desktop)]
            {
                // `target="_blank"` and `window.open` (every "Open on …" link
                // in the UI): the system browser, never a second app window.
                // Android has no such request; its WebView navigates instead,
                // which the navigation guard above handles.
                let handle = app.handle().clone();
                builder = builder.on_new_window(move |u, _features| {
                    tracing::debug!(url = %u, "new window requested");
                    if matches!(u.scheme(), "http" | "https" | "mailto") {
                        open_external(&handle, u.as_str());
                    }
                    tauri::webview::NewWindowResponse::Deny
                });
                builder = builder
                    .title("Odeion")
                    .inner_size(1280.0, 840.0)
                    .min_inner_size(420.0, 600.0);
            }
            #[allow(unused_variables)]
            let window = builder.build()?;
            #[cfg(any(desktop, target_os = "android"))]
            update::spawn(app.handle());
            #[cfg(target_os = "android")]
            android::init_tls(&window);
            // A USB DAC plugged in or out: the player registers again, so the node and its page learn it
            #[cfg(target_os = "android")]
            {
                // The DAC settings (bit-perfect or shared, the DAC's volume) live with the app
                if let Ok(dir) = app.path().app_local_data_dir() {
                    let _ = std::fs::create_dir_all(&dir);
                    pm_out_local::usb_dac::keep_settings_in(dir.join("usb-dac.json"));
                }
                // A confirmation withdrawn while playing (Android's mixer changed): the node learns it
                let h2 = app.handle().clone();
                pm_out_local::usb_dac::on_change(move || {
                    let app = h2.clone();
                    if let Some(s) = app.state::<AppState>().current() {
                        if s.via.is_empty() {
                            tauri::async_runtime::spawn(register_player(app, s));
                        }
                    }
                });
                let handle = app.handle().clone();
                android::watch_usb_audio(app.handle(), move || {
                    let app = handle.clone();
                    let Some(s) = app.state::<AppState>().current() else {
                        return;
                    };
                    if s.via.is_empty() {
                        tauri::async_runtime::spawn(register_player(app, s));
                    }
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!());
    match app {
        Ok(app) => app.run(|app, event| {
            // The Ring sign-in waits for the screen (ring_wait): Android onPause / onResume. Not on
            // iOS, whose Suspended (willResignActive) and Resumed (willEnterForeground) do not pair.
            #[cfg(target_os = "android")]
            if let tauri::RunEvent::WindowEvent { event, .. } = &event {
                match event {
                    tauri::WindowEvent::Suspended => {
                        ring_wait::left();
                        page_event(app, "pmdesktop:suspended");
                    }
                    tauri::WindowEvent::Resumed => {
                        ring_wait::back();
                        // The page hears it too: the WebView doesn't always tell it it was out of
                        // sight (the in-app player checks its sound came back, app.js)
                        page_event(app, "pmdesktop:resumed");
                        // On screen again: the player service as it was (refused from the background,
                        // or ended after a while with nothing playing)
                        android::resume_player_service(app);
                    }
                    _ => {}
                }
            }
            #[cfg(not(target_os = "android"))]
            let _ = (app, event);
        }),
        Err(e) => {
            tracing::error!("cannot start: {e}");
            std::process::exit(1);
        }
    }
}

/// An event for the page in the main window (`window.dispatchEvent(new Event(name))`).
#[cfg(target_os = "android")]
fn page_event(app: &tauri::AppHandle, name: &str) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.eval(format!("window.dispatchEvent(new Event({name:?}))"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Connecting saves and sets the settings under the settings lock: while it is held, nothing
    /// is written (a late registration's store can't land between this save and this set).
    #[test]
    fn connecting_saves_and_sets_under_the_settings_lock() {
        let dir = std::env::temp_dir().join(format!("pm-desktop-store-in-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("settings.json");
        let cell = std::sync::Arc::new(Mutex::new(None::<Settings>));
        let held = cell.lock().unwrap();
        let writer = {
            let (cell, path) = (cell.clone(), path.clone());
            std::thread::spawn(move || {
                store_in(
                    &cell,
                    &path,
                    Some(Settings {
                        node_url: "http://b:7788".into(),
                        ..Settings::default()
                    }),
                )
            })
        };
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert!(!path.exists(), "saved before the lock was taken");
        drop(held);
        writer.join().unwrap();
        let saved: Settings = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved.node_url, "http://b:7788");
        assert_eq!(cell.lock().unwrap().as_ref().unwrap().node_url, "http://b:7788");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A registration that ends after the app connected to another node never writes that
    /// node's settings over the new one's: the check and the store happen under one lock.
    #[test]
    fn a_late_registration_never_replaces_a_newer_connection() {
        let dir = std::env::temp_dir().join(format!("pm-desktop-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("settings.json");
        let node = |url: &str, player: &str| Settings {
            node_url: url.into(),
            player_address: player.into(),
            ..Settings::default()
        };
        let cell = Mutex::new(Some(node("http://b:7788", "")));
        // A's registration, late: refused, B stays
        assert!(!store_if_node(
            &cell,
            &path,
            "http://a:7788",
            node("http://a:7788", "a-player")
        ));
        assert_eq!(cell.lock().unwrap().as_ref().unwrap().node_url, "http://b:7788");
        assert!(!path.exists(), "nothing written");
        // B's own: stored
        assert!(store_if_node(
            &cell,
            &path,
            "http://b:7788",
            node("http://b:7788", "b-player")
        ));
        assert_eq!(cell.lock().unwrap().as_ref().unwrap().player_address, "b-player");
        let saved: Settings = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved.player_address, "b-player");
        // No side file left behind
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Stands in at the claimed address: answers `/v1/info` as the node would, and hands every
    /// request it gets to the test.
    fn claimant() -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for mut c in l.incoming().flatten() {
                let mut buf = [0u8; 4096];
                let n = c.read(&mut buf).unwrap_or(0);
                let _ = tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
                let body = r#"{"name":"Living room","id":"node-1"}"#;
                let _ = c.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                );
            }
        });
        (url, rx)
    }

    /// The node's id is announced in the clear, so anyone on the LAN can claim it from another
    /// address: nothing goes there, the token least of all, until the person says so (security
    /// review 2026-10-27, #3).
    #[tokio::test]
    async fn a_node_claiming_the_id_elsewhere_gets_no_token_until_the_person_says_so() {
        let (url, seen) = claimant();
        // A test token, built at run time
        let mut s = Settings::new("http://127.0.0.1:9", &["saved", "token"].join("-"));
        s.node_id = "node-1".into();
        let found = vec![pm_discovery::FoundNode {
            id: "node-1".into(),
            name: "Living room".into(),
            url: url.clone(),
            version: String::new(),
            token_required: true,
            via: vec!["mdns".into()],
        }];
        let slot = Mutex::new(None);
        let r = after_lan_search(&slot, &s, &found).await;
        if let Ok(req) = seen.recv_timeout(std::time::Duration::from_millis(300)) {
            panic!("the claimed address got a request before the person said yes:\n{req}");
        }
        match r {
            Some(Err(msg)) => assert_eq!(msg, format!("{MOVED}{url}")),
            other => panic!("expected the page to be asked, got {other:?}"),
        }
        let kept = slot
            .lock()
            .unwrap()
            .clone()
            .expect("the address is kept for the person");
        assert_eq!(
            (kept.node_url.as_str(), kept.token.as_str()),
            (url.as_str(), s.token.as_str())
        );
        // A node at the remembered address, or with another id, is not a move.
        let mut same = found.clone();
        same[0].url = s.node_url.clone();
        assert!(after_lan_search(&slot, &s, &same).await.is_none());
        let mut other = found.clone();
        other[0].id = "node-2".into();
        assert!(after_lan_search(&slot, &s, &other).await.is_none());
    }

    #[test]
    fn the_token_goes_only_to_the_node_origin() {
        let s = Settings::new("http://nas.local:7788", "se\"cret");
        let js = init_script(Some(&s));
        assert!(
            js.contains(r#"var o="http://nas.local:7788",t="se\"cret""#),
            "{js}"
        );
        assert!(js.contains("location.origin===o"));
        // Nothing to hand over before the first connection.
        assert!(init_script(None).contains(r#"var o="",t="""#));
    }

    #[test]
    fn a_blank_window_does_not_replace_the_app_page() {
        // about:blank's origin is "null", never the page's: the override must stop it before the
        // cross-origin branch, whose navigation the guard would allow (an app page)
        let js = init_script(None);
        let blank = js
            .find("x.protocol==='about:')return null;")
            .expect("about: is refused");
        let away = js
            .find("x.origin!==location.origin")
            .expect("other origins go out");
        assert!(blank < away, "{js}");
        assert!(is_app_page(&url::Url::parse("about:blank").unwrap()));
    }

    #[test]
    fn app_pages_are_recognised() {
        for u in [
            "tauri://localhost/index.html",
            "http://tauri.localhost/index.html",
            "about:blank",
        ] {
            assert!(is_app_page(&url::Url::parse(u).unwrap()), "{u}");
        }
        assert!(!is_app_page(&url::Url::parse("https://tidal.com/").unwrap()));
    }

    /// The player service's mode for each connection: a zone of the node when the node reaches
    /// the phone's player directly, else the app playing the music itself, through the tunnel too
    /// (the owner's case: TASK-android-background-playback-1105). This checks the choice only, not
    /// its call sites (`connected`, `open_offline`); both modes run on the emulator
    /// (tools/android-background-probe.py, parts A, B, D and E).
    #[test]
    fn the_player_service_mode_for_each_connection() {
        assert_eq!(
            player_service("NUC", true, true),
            ("This phone is a zone of NUC".to_string(), true)
        );
        // Through the tunnel the node can't reach the phone's player: the app plays itself
        assert_eq!(
            player_service("NUC", false, true),
            ("Plays music from NUC".to_string(), false)
        );
        assert_eq!(
            player_service("", true, false),
            ("Plays music from your node".to_string(), false)
        );
    }

    /// The node found on the network is taken at 127.0.0.1 only when the node there has its id;
    /// a node without an id is never taken for another.
    #[test]
    fn only_the_found_node_is_taken_at_this_computers_address() {
        assert!(same_node("node-1", "node-1"));
        assert!(!same_node("node-1", "node-2"), "another node on this computer");
        assert!(!same_node("node-1", ""), "a node that names no id");
        assert!(!same_node("", ""), "nothing to compare");
    }

    /// Back from offline: a refused or missing sign-in sends the app to its connect page, which
    /// signs in again; a node that doesn't answer is tried again later.
    #[test]
    fn a_refused_sign_in_ends_the_offline_wait() {
        assert!(needs_new_sign_in(node::REFUSED));
        assert!(needs_new_sign_in(node::NEEDS_SIGN_IN));
        assert!(!needs_new_sign_in("Can't reach http://192.168.1.10:7788"));
        assert!(!needs_new_sign_in("The node answered 503"));
        assert!(is_app_page(&connect_page()), "the connect page is the app's own");
        assert!(connect_page().path().ends_with("/index.html"));
    }
}
