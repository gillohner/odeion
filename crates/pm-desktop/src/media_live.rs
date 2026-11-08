//! Android: the media session's feed and its buttons ([`crate::media`] decides; this runs it).
//!
//! * The zone the page controls (`OdeionAndroid.controlling`) is followed on the node's
//!   `/v1/events` WebSocket (a 3 s zone poll for an https node), and its state goes to the app's
//!   side (`MediaControl.kt`) whenever it changes: metadata, state, position, volume; the cover is
//!   fetched here (the node's own with the app's token; another site's over https only).
//! * The session's buttons (notification, lock screen, headset, Bluetooth) and the volume keys
//!   call the node as the page does (`POST /v1/zones/{id}/transport`, `/volume`); for the page's
//!   own player they come back to the page as `pmdesktop:media`.
//! * Android's media volume (the WebView player's slider, `OdeionAndroid.setMediaVolume`) is set
//!   and read through pm-out-local's `media_volume`; every change goes to the page as
//!   `window.pmDesktop.mediaVolume` + `pmdesktop:mediavolume`, and the phone's own zone tells the
//!   node at once (the slider on every controller follows).
//! * Audio focus for the phone's own zone (the WebView takes its own for the page's player; a
//!   second holder in the app would pause it): a call pauses it and it plays on afterwards; another
//!   app taking over pauses it; Android ducks shared playback itself, bit-perfect and Exclusive USB
//!   pause instead. "Becoming noisy" (headphones out) pauses both of the phone's players.

use crate::media::{self, Command, Event, FocusAction, PageMsg, Session};
use crate::settings::Settings;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Manager};

/// What the app's side does (`MediaControl.kt`, over JNI).
pub trait Native: Send + Sync {
    /// The session as [`Session`] JSON.
    fn update(&self, session: &str);
    /// The cover for `url` (empty: none).
    fn art(&self, url: &str, bytes: &[u8]);
    /// Hold Android's audio focus (the phone's own zone plays), or let it go; `ducks`: Android
    /// may lower it itself during a short prompt (shared playback).
    fn focus(&self, want: bool, ducks: bool);
}

pub struct Media {
    app: AppHandle,
    native: Arc<dyn Native>,
    offline_origin: Arc<Mutex<Option<String>>>,
    st: Mutex<State>,
}

#[derive(Default)]
struct State {
    settings: Option<Settings>,
    /// Counts connections: a feed for an older one ends.
    feed: u64,
    controlled: Option<String>,
    zones: HashMap<String, Value>,
    /// Each output's volume scale (`GET /v1/outputs`, the owner's).
    scales: HashMap<String, media::Scale>,
    page: media::PagePlayer,
    pushed: String,
    art: Option<String>,
    /// The audio focus as last asked: held, and whether Android may duck it.
    focus: (bool, bool),
    /// The phone's players paused by a call (or a prompt): they play on when it ends.
    paused_for_now: Vec<String>,
    /// Counts media volume changes: only the newest tells the node.
    volume_seq: u64,
}

static MEDIA: OnceLock<Arc<Media>> = OnceLock::new();

/// The running media feed, once [`init`] ran.
pub fn get() -> Option<Arc<Media>> {
    MEDIA.get().cloned()
}

/// A connection made before [`init`] ran (the activity's thread runs it a moment after start).
static PENDING: Mutex<Option<Settings>> = Mutex::new(None);

pub fn init(
    app: &AppHandle,
    native: Arc<dyn Native>,
    offline_origin: Arc<Mutex<Option<String>>>,
) -> Arc<Media> {
    let m = MEDIA
        .get_or_init(|| {
            Arc::new(Media {
                app: app.clone(),
                native,
                offline_origin,
                st: Mutex::new(State::default()),
            })
        })
        .clone();
    if let Some(s) = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take() {
        m.connected(s);
    }
    m
}

/// No node any more (forgotten): no feed, now or once it starts.
pub fn forget() {
    PENDING.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(m) = get() {
        m.forget();
    }
}

/// Connected to a node: the media feed follows it (now, or once it starts).
pub fn connected(s: Settings) {
    match get() {
        Some(m) => m.connected(s),
        None => *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(s),
    }
}

fn client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(8))
        .user_agent(concat!("pubky-music-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()
}

impl Media {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn eval(&self, js: String) {
        if let Some(w) = self.app.get_webview_window(crate::MAIN) {
            let _ = w.eval(js);
        }
    }

    /// Connected to a node (again): follow its zones.
    pub fn connected(self: &Arc<Self>, s: Settings) {
        let gen = {
            let mut st = self.state();
            st.settings = Some(s);
            st.feed += 1;
            st.zones.clear();
            st.feed
        };
        let me = self.clone();
        tauri::async_runtime::spawn(async move { me.feed(gen).await });
    }

    /// No node any more: no zone, no session.
    pub fn forget(&self) {
        {
            let mut st = self.state();
            st.settings = None;
            st.feed += 1;
            st.zones.clear();
            st.controlled = None;
        }
        self.push();
    }

    /// The phone's own zone: the zone that plays on this app's output.
    fn phone_zone(st: &State) -> Option<&Value> {
        let outs = crate::player::output_ids();
        st.zones.values().find(|z| {
            z.get("output")
                .and_then(Value::as_str)
                .is_some_and(|o| outs.iter().any(|x| x == o))
        })
    }

    /// The zone the session is for: the one the page controls; while that is the page's own
    /// player with nothing in it, the phone's own zone when something is queued there (what this
    /// phone plays, started from another controller).
    fn target(st: &State) -> String {
        let zone = st.controlled.as_deref().unwrap_or(media::BROWSER);
        if zone == media::BROWSER && st.page.title.is_empty() {
            if let Some(z) = Self::phone_zone(st) {
                let queued = z
                    .get("queue")
                    .and_then(Value::as_array)
                    .is_some_and(|q| !q.is_empty());
                if queued {
                    if let Some(id) = z.get("id").and_then(Value::as_str) {
                        return id.to_string();
                    }
                }
            }
        }
        zone.to_string()
    }

    fn session(st: &State) -> Session {
        let zone = Self::target(st);
        let zone = zone.as_str();
        if zone == media::BROWSER {
            return media::session_for_page(&st.page, "This phone");
        }
        let Some(z) = st.zones.get(zone) else {
            return Session::none();
        };
        let output = z.get("output").and_then(Value::as_str).unwrap_or("");
        let phone = crate::player::output_ids().iter().any(|o| o == output);
        let scale = if phone {
            match pm_out_local::media_volume::phone_kind() {
                Some(pm_model::VolumeKind::Db { min, max, step }) => {
                    Some(media::Scale::Db { min, max, step })
                }
                Some(pm_model::VolumeKind::Percent) => Some(media::Scale::Percent),
                _ => None,
            }
        } else {
            st.scales.get(output).copied()
        };
        media::session_for_zone(z, phone, scale)
    }

    /// The phone's own zone, when it plays now (its id).
    fn phone_zone_playing(st: &State) -> Option<String> {
        let z = Self::phone_zone(st)?;
        (z.get("transport").and_then(Value::as_str) == Some("playing"))
            .then(|| z.get("id").and_then(Value::as_str).map(str::to_string))
            .flatten()
    }

    /// The phone's own players playing now: its zone's id, and `browser` for the WebView player.
    fn phone_playing(st: &State) -> Vec<String> {
        let mut v: Vec<String> = Self::phone_zone_playing(st).into_iter().collect();
        if st.page.playing() {
            v.push(media::BROWSER.into());
        }
        v
    }

    /// Whether Android may lower the phone's zone itself (shared playback): bit-perfect and
    /// Exclusive USB can't be lowered.
    fn ducks() -> bool {
        !pm_out_local::usb_dac::current().is_some_and(|d| {
            d.confirmed
                && matches!(
                    d.mode,
                    pm_zones::DeviceMode::BitPerfect | pm_zones::DeviceMode::Exclusive
                )
        })
    }

    /// The session to the app's side when it changed; the cover when it is another; the audio
    /// focus while the phone's own players play (or wait for a call to end).
    fn push(&self) {
        let (json, art, focus) = {
            let mut st = self.state();
            let s = Self::session(&st);
            let json = serde_json::to_string(&s).unwrap_or_default();
            let changed = json != st.pushed;
            if changed {
                st.pushed = json.clone();
            }
            let art = (s.art != st.art).then(|| s.art.clone());
            if let Some(a) = &art {
                st.art = a.clone();
            }
            let want = Self::phone_zone_playing(&st).is_some() || !st.paused_for_now.is_empty();
            let now = (want, want && Self::ducks());
            let focus = (now != st.focus).then_some(now);
            if let Some(f) = focus {
                st.focus = f;
            }
            (changed.then_some(json), art, focus)
        };
        if let Some(j) = json {
            self.native.update(&j);
        }
        if let Some((want, ducks)) = focus {
            self.native.focus(want, ducks);
        }
        if let Some(art) = art {
            self.fetch_art(art);
        }
    }

    fn fetch_art(&self, art: Option<String>) {
        let native = self.native.clone();
        let Some(art) = art else {
            return;
        };
        let s = self.state().settings.clone();
        tauri::async_runtime::spawn(async move {
            let bytes = async {
                let s = s?;
                let (url, auth) = media::art_request(&art, s.base())?;
                let mut req = client()?.get(url);
                if auth {
                    req = req.bearer_auth(&s.token);
                }
                let resp = req.send().await.ok()?.error_for_status().ok()?;
                if resp.content_length().is_some_and(|n| n > ART_MAX) {
                    return None;
                }
                let b = resp.bytes().await.ok()?;
                (b.len() as u64 <= ART_MAX).then(|| b.to_vec())
            }
            .await
            .unwrap_or_default();
            native.art(&art, &bytes);
        });
    }

    /// What the app's side sent.
    pub async fn handle(self: Arc<Self>, ev: Event) {
        match ev {
            Event::Command(c) => self.command(c).await,
            Event::Page { origin, data } => self.page(&origin, &data).await,
            Event::Focus(change) => self.focus_changed(change).await,
            Event::Noisy => {
                let playing = Self::phone_playing(&self.state());
                self.state().paused_for_now.clear();
                for z in playing {
                    self.pause(&z).await;
                }
                self.push();
            }
            Event::MediaVolume => self.media_volume_changed().await,
            Event::Resumed => {
                let _ = tauri::async_runtime::spawn_blocking(pm_out_local::exclusive::auto_ask).await;
                self.state().pushed.clear();
                self.push();
                self.media_volume_changed().await;
            }
        }
    }

    async fn post(&self, url: Option<String>, body: Value) -> bool {
        let Some(s) = self.state().settings.clone() else {
            return false;
        };
        let (Some(url), Some(c)) = (url, client()) else {
            return false;
        };
        match c.post(url).bearer_auth(&s.token).json(&body).send().await {
            Ok(r) if r.status().is_success() => true,
            Ok(r) => {
                tracing::info!(status = %r.status(), "media controls: the node said no");
                false
            }
            Err(e) => {
                tracing::info!("media controls: the node did not answer: {e}");
                false
            }
        }
    }

    async fn transport(&self, zone: &str, body: Value) {
        let base = self.state().settings.as_ref().map(|s| s.base().to_string());
        let url = base.and_then(|b| media::zone_url(&b, zone, "transport"));
        self.post(url, body).await;
    }

    async fn pause(&self, zone: &str) {
        if zone == media::BROWSER {
            self.eval(media::page_action_script("pause", None));
        } else {
            self.transport(zone, serde_json::json!({"action": "pause"})).await;
        }
    }

    async fn play(&self, zone: &str) {
        if zone == media::BROWSER {
            self.eval(media::page_action_script("play", None));
        } else {
            self.transport(zone, serde_json::json!({"action": "play"})).await;
        }
    }

    async fn command(&self, c: Command) {
        let zone = Self::target(&self.state());
        if zone == media::BROWSER {
            if let Some((a, t)) = c.page_action() {
                self.eval(media::page_action_script(a, t));
            }
            return;
        }
        if let Some(body) = c.transport() {
            self.transport(&zone, body).await;
            return;
        }
        // A volume key or the system's panel on a zone with a volume of its own
        let (body, base) = {
            let mut st = self.state();
            let Some(z) = st.zones.get(&zone) else { return };
            let output = z.get("output").and_then(Value::as_str).unwrap_or("").to_string();
            let level = z.get("volume").cloned().unwrap_or(Value::Null);
            let scale = if crate::player::output_ids().contains(&output) {
                match pm_out_local::media_volume::phone_kind() {
                    Some(pm_model::VolumeKind::Db { min, max, step }) => {
                        Some(media::Scale::Db { min, max, step })
                    }
                    _ => None,
                }
            } else {
                st.scales
                    .get(&output)
                    .copied()
                    .or_else(|| media::Scale::of_level(&level))
            };
            let Some(scale) = scale else { return };
            let body = match c {
                Command::VolumeTo(n) => Some(scale.volume_at(n)),
                Command::VolumeBy(d) => scale.adjust(&level, d),
                _ => None,
            };
            let Some(body) = body else { return };
            // Shown at once; the node's read-back follows
            if let Some(z) = st.zones.get_mut(&zone) {
                let mut v = level.as_object().cloned().unwrap_or_default();
                v.remove("percent");
                v.remove("db");
                if let Some(o) = body.as_object() {
                    v.extend(o.clone());
                }
                z["volume"] = Value::Object(v);
            }
            (body, st.settings.as_ref().map(|s| s.base().to_string()))
        };
        self.push();
        let url = base.and_then(|b| media::zone_url(&b, &zone, "volume"));
        self.post(url, serde_json::json!({ "volume": body })).await;
    }

    async fn page(&self, origin: &str, data: &str) {
        let allowed = {
            let st = self.state();
            let mut a: Vec<String> = st
                .settings
                .as_ref()
                .and_then(Settings::origin)
                .into_iter()
                .collect();
            if let Some(o) = self.offline_origin.lock().ok().and_then(|o| o.clone()) {
                a.push(o);
            }
            a
        };
        if !media::page_allowed(origin, &allowed) {
            tracing::debug!(%origin, "media controls: a message from another page, ignored");
            return;
        }
        match media::parse_page(data) {
            Some(PageMsg::Controlling(z)) => {
                self.state().controlled = Some(z);
                self.push();
            }
            Some(PageMsg::SetMediaVolume(p)) => {
                let set = tauri::async_runtime::spawn_blocking(move || {
                    pm_out_local::media_volume::set(pm_model::Volume::Percent(p))
                })
                .await;
                if let Ok(Err(e)) = set {
                    tracing::info!("media volume: {e}");
                }
                self.media_volume_changed().await;
            }
            Some(PageMsg::Player(p)) => {
                self.state().page = p;
                self.push();
            }
            None => tracing::debug!("media controls: a page message that isn't one of ours"),
        }
    }

    async fn focus_changed(&self, change: i64) {
        // Shared playback is lowered by Android itself; bit-perfect and Exclusive USB can't be
        let action = media::focus_action(change, Self::ducks());
        tracing::info!(change, ?action, "audio focus");
        match action {
            FocusAction::PauseForNow | FocusAction::Pause => {
                let playing: Vec<String> = Self::phone_zone_playing(&self.state()).into_iter().collect();
                {
                    let mut st = self.state();
                    if action == FocusAction::PauseForNow {
                        for z in &playing {
                            if !st.paused_for_now.contains(z) {
                                st.paused_for_now.push(z.clone());
                            }
                        }
                    } else {
                        st.paused_for_now.clear();
                    }
                }
                for z in playing {
                    self.pause(&z).await;
                }
            }
            FocusAction::Resume => {
                let again = std::mem::take(&mut self.state().paused_for_now);
                for z in again {
                    self.play(&z).await;
                }
            }
            FocusAction::Nothing => {}
        }
        self.push();
    }

    /// Android's media volume changed (a key, the panel, the slider, another route): the page
    /// hears it, and the node's level for the phone's own zone follows at once.
    pub async fn media_volume_changed(&self) {
        let seq = {
            let mut st = self.state();
            st.volume_seq += 1;
            st.volume_seq
        };
        // Key presses come in bursts: the last one counts
        tokio::time::sleep(Duration::from_millis(250)).await;
        if self.state().volume_seq != seq {
            return;
        }
        let read = tauri::async_runtime::spawn_blocking(pm_out_local::media_volume::read)
            .await
            .ok()
            .flatten();
        let Some(v) = read else { return };
        let pm_model::VolumeLevel::Percent(p) = v.level else {
            return;
        };
        self.eval(media::media_volume_script(p));
        // The phone's own zone on Android's media volume: the node reads it back now (a set to
        // the level it already has changes nothing on the phone)
        let nudge = {
            let st = self.state();
            Self::phone_zone(&st).and_then(|z| {
                let shown = z
                    .get("volume")
                    .and_then(|v| v.get("percent"))
                    .and_then(Value::as_u64);
                let id = z.get("id")?.as_str()?.to_string();
                (shown != Some(u64::from(p))).then_some(id)
            })
        };
        let media = matches!(
            pm_out_local::media_volume::phone_kind(),
            Some(pm_model::VolumeKind::Percent)
        );
        if let (Some(id), true) = (nudge, media) {
            let base = self.state().settings.as_ref().map(|s| s.base().to_string());
            let url = base.and_then(|b| media::zone_url(&b, &id, "volume"));
            self.post(url, serde_json::json!({"volume": {"percent": p}}))
                .await;
        }
    }

    /// The node's page loaded: its own player starts afresh, and it hears Android's media volume.
    pub fn page_loaded(self: &Arc<Self>) {
        self.state().page = media::PagePlayer::default();
        self.push();
        let me = self.clone();
        tauri::async_runtime::spawn(async move { me.media_volume_changed().await });
    }

    fn current(&self, gen: u64) -> Option<Settings> {
        let st = self.state();
        (st.feed == gen).then(|| st.settings.clone()).flatten()
    }

    /// Follow the node's zones while connected to it (until the next connection).
    async fn feed(self: Arc<Self>, gen: u64) {
        let mut wait = Duration::from_secs(1);
        while let Some(s) = self.current(gen) {
            self.load_scales(&s).await;
            let r = if s.base().starts_with("http://") {
                self.events(&s, gen).await
            } else {
                self.poll(&s, gen).await
            };
            match r {
                Ok(()) => wait = Duration::from_secs(1),
                Err(e) => {
                    tracing::debug!("media controls: the node's events: {e}");
                    wait = (wait * 2).min(Duration::from_secs(30));
                }
            }
            tokio::time::sleep(wait).await;
        }
    }

    async fn load_scales(&self, s: &Settings) {
        let url = format!("{}/v1/outputs", s.base().trim_end_matches('/'));
        let Some(c) = client() else { return };
        let Ok(r) = c.get(url).bearer_auth(&s.token).send().await else {
            return;
        };
        // A listener sees no outputs: the scales then come from the levels themselves
        let Ok(r) = r.error_for_status() else {
            return;
        };
        let Ok(list) = r.json::<Vec<Value>>().await else {
            return;
        };
        let scales = list
            .iter()
            .filter_map(|o| {
                let id = o.get("id")?.as_str()?.to_string();
                let caps = o.get("live_caps").or_else(|| o.get("caps"))?;
                Some((id, media::Scale::from_caps(caps.get("hw_volume")?)?))
            })
            .collect();
        self.state().scales = scales;
    }

    /// One event-stream connection, until it ends or another connection begins.
    async fn events(&self, s: &Settings, gen: u64) -> Result<(), String> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        use tokio_tungstenite::tungstenite::Message;
        let u = url::Url::parse(s.base()).map_err(|e| e.to_string())?;
        let host = u.host_str().ok_or("no host")?.to_string();
        let port = u.port_or_known_default().unwrap_or(80);
        let tcp = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::TcpStream::connect(format!("{host}:{port}")),
        )
        .await
        .map_err(|_| "connect timed out".to_string())?
        .map_err(|e| e.to_string())?;
        let mut req = format!("ws://{host}:{port}/v1/events")
            .into_client_request()
            .map_err(|e| e.to_string())?;
        if !s.token.is_empty() {
            req.headers_mut().insert(
                "authorization",
                format!("Bearer {}", s.token)
                    .parse()
                    .map_err(|_| "token".to_string())?,
            );
        }
        if let Some(o) = s.origin() {
            if let Ok(v) = o.parse() {
                req.headers_mut().insert("origin", v);
            }
        }
        let (mut ws, _) =
            tokio::time::timeout(Duration::from_secs(8), tokio_tungstenite::client_async(req, tcp))
                .await
                .map_err(|_| "handshake timed out".to_string())?
                .map_err(|e| e.to_string())?;
        tracing::info!("media controls: following the node's zones");
        let mut ping = tokio::time::interval(Duration::from_secs(20));
        loop {
            let msg = tokio::select! {
                m = ws.next() => m,
                _ = ping.tick() => {
                    if self.current(gen).is_none() {
                        let _ = ws.close(None).await;
                        return Ok(());
                    }
                    let _ = ws.send(Message::Text(r#"{"type":"ping","id":1}"#.into())).await;
                    continue;
                }
            };
            let Some(msg) = msg else { return Ok(()) };
            let text = match msg.map_err(|e| e.to_string())? {
                Message::Text(t) => t.to_string(),
                Message::Close(_) => return Ok(()),
                _ => continue,
            };
            if self.current(gen).is_none() {
                return Ok(());
            }
            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            match v.get("type").and_then(Value::as_str) {
                Some("snapshot") => {
                    let zones = v
                        .get("zones")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let mut st = self.state();
                    st.zones = zones
                        .into_iter()
                        .filter_map(|z| Some((z.get("id")?.as_str()?.to_string(), z)))
                        .collect();
                }
                Some("zone") => {
                    if let Some(z) = v.get("state") {
                        if let Some(id) = z.get("id").and_then(Value::as_str) {
                            self.state().zones.insert(id.to_string(), z.clone());
                        }
                    }
                }
                Some("zone_removed") => {
                    if let Some(id) = v.get("zone").and_then(Value::as_str) {
                        self.state().zones.remove(id);
                    }
                }
                Some("outputs_changed") => {
                    self.load_scales(s).await;
                }
                // A gap in the numbers: a fresh snapshot
                _ => continue,
            }
            self.push();
        }
    }

    /// An https node: its zones every 3 s instead (the app speaks plain ws:// only).
    async fn poll(&self, s: &Settings, gen: u64) -> Result<(), String> {
        let url = format!("{}/v1/zones", s.base().trim_end_matches('/'));
        let c = client().ok_or("no client")?;
        while self.current(gen).is_some() {
            let zones: Vec<Value> = c
                .get(&url)
                .bearer_auth(&s.token)
                .send()
                .await
                .map_err(|e| e.to_string())?
                .error_for_status()
                .map_err(|e| e.to_string())?
                .json()
                .await
                .map_err(|e| e.to_string())?;
            self.state().zones = zones
                .into_iter()
                .filter_map(|z| Some((z.get("id")?.as_str()?.to_string(), z)))
                .collect();
            self.push();
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
        Ok(())
    }
}

/// The largest cover fetched.
const ART_MAX: u64 = 2 * 1024 * 1024;
