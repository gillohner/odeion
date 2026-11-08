//! The system's media controls (Android's media notification, lock screen, quick-settings panel,
//! headset and Bluetooth buttons, the volume keys) for the zone the app's page controls.
//!
//! The app's side (`MediaControl.kt`) keeps one framework `MediaSession`; this module decides what
//! it shows and what its buttons do, without a JVM, so it is tested on every host:
//! * [`session_for_zone`]: a node zone (`/v1/events`) as the session's metadata, state and volume;
//! * [`session_for_page`]: the WebView player ("This phone"), mirrored from the page's own
//!   Media Session API;
//! * [`Scale`]: a zone's volume as the steps of the session's remote volume (one key press is one
//!   step, never less than one of the device's own steps);
//! * [`parse_event`] / [`parse_page`]: what the app's side and the page send, checked and bounded.
//!
//! One volume (odeion-control-consistency): a network zone's session volume is the zone's own (the
//! same `POST /zones/{id}/volume` the slider uses; the node's read-back is what it shows); the
//! phone's own zone and the WebView player use Android's media volume, which the keys move
//! themselves (a local session); in Exclusive USB the phone's zone is the DAC's own volume (dB).

// The runtime that uses all of it (`media_live`) is Android's; elsewhere only the tests build this
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use serde::Serialize;
use serde_json::Value;

/// The page's own player's zone id (app.js `BROWSER`).
pub const BROWSER: &str = "browser";

/// The longest text the session shows (a title, an artist).
const TEXT_MAX: usize = 512;
/// The largest message the page may send.
pub const PAGE_MAX: usize = 16 * 1024;
/// The longest zone id taken from the page.
const ZONE_MAX: usize = 512;

/// What the session shows and does now. `active: false`: nothing queued, the session goes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Session {
    pub active: bool,
    pub zone: String,
    pub zone_name: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub position_ms: u64,
    /// `playing`, `paused` or `stopped`.
    pub state: &'static str,
    pub speed: f32,
    pub can_prev: bool,
    pub can_next: bool,
    pub can_seek: bool,
    pub volume: SessionVolume,
    /// The cover's address (fetched by the app, never by Android).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub art: Option<String>,
}

impl Session {
    pub fn none() -> Self {
        Session {
            active: false,
            zone: String::new(),
            zone_name: String::new(),
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            duration_ms: 0,
            position_ms: 0,
            state: "stopped",
            speed: 1.0,
            can_prev: false,
            can_next: false,
            can_seek: false,
            volume: SessionVolume::Local,
            art: None,
        }
    }
}

/// The session's volume: Android's media volume (the keys move it themselves), or the zone's own
/// in `max` steps.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionVolume {
    Local,
    Remote {
        max: u32,
        current: u32,
    },
    /// A zone without a volume the node can set: the panel shows it, the keys do nothing.
    Fixed,
}

/// A zone's volume scale, as its output reports it (`caps.hw_volume`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scale {
    Percent,
    Db { min: f32, max: f32, step: f32 },
}

/// At most this many steps in the system's volume panel.
const MAX_STEPS: u32 = 100;

impl Scale {
    /// `caps.hw_volume` as JSON (`{"kind":"percent"}`, `{"kind":"db","min":…}`).
    pub fn from_caps(hw: &Value) -> Option<Self> {
        match hw.get("kind")?.as_str()? {
            "percent" => Some(Scale::Percent),
            "db" => {
                let min = hw.get("min")?.as_f64()? as f32;
                let max = hw.get("max")?.as_f64()? as f32;
                let step = hw.get("step").and_then(Value::as_f64).unwrap_or(1.0) as f32;
                // A mixer whose lowest step is mute says about -99999 dB: the pages' -60 instead
                let min = if min < -200.0 { -60.0 } else { min };
                (max > min).then_some(Scale::Db {
                    min,
                    max,
                    step: if step > 0.0 { step } else { 1.0 },
                })
            }
            _ => None,
        }
    }

    /// The scale a zone's level is on when its output's scale isn't known (a listener sees no
    /// outputs): percent, or the pages' -60…0 dB.
    pub fn of_level(v: &Value) -> Option<Self> {
        if v.get("percent").is_some_and(Value::is_number) {
            Some(Scale::Percent)
        } else if v.get("db").is_some_and(Value::is_number) {
            Some(Scale::Db {
                min: -60.0,
                max: 0.0,
                step: 1.0,
            })
        } else {
            None
        }
    }

    /// Steps in the system's panel: percent 100; dB one per device step, at most 100 (each then
    /// spans whole device steps).
    pub fn steps(&self) -> u32 {
        match *self {
            Scale::Percent => MAX_STEPS,
            Scale::Db { min, max, step } => {
                let n = ((max - min) / step).round();
                (n as u32).clamp(1, MAX_STEPS)
            }
        }
    }

    /// The step a zone's level (`{"percent":…}` / `{"db":…}`) shows at.
    pub fn step_of(&self, v: &Value) -> Option<u32> {
        let steps = self.steps();
        match *self {
            Scale::Percent => {
                let p = v.get("percent")?.as_f64()?;
                Some((p.round().clamp(0.0, 100.0)) as u32)
            }
            Scale::Db { min, max, .. } => {
                let db = v.get("db")?.as_f64()? as f32;
                let at = ((db.clamp(min, max) - min) / (max - min) * steps as f32).round();
                Some((at as u32).min(steps))
            }
        }
    }

    /// The zone volume command for step `n`: a percent, or a level on the device's own steps.
    pub fn volume_at(&self, n: u32) -> Value {
        let steps = self.steps();
        let n = n.min(steps);
        match *self {
            Scale::Percent => serde_json::json!({ "percent": n }),
            Scale::Db { min, max, step } => {
                let db = min + (max - min) * n as f32 / steps as f32;
                // On the device's own steps, from its minimum
                let db = (min + ((db - min) / step).round() * step).clamp(min, max);
                serde_json::json!({ "db": (db * 100.0).round() / 100.0 })
            }
        }
    }

    /// One key press from the level `v`: the next step up (`dir` > 0) or down.
    pub fn adjust(&self, v: &Value, dir: i32) -> Option<Value> {
        let now = self.step_of(v)?;
        let next = if dir > 0 {
            (now + 1).min(self.steps())
        } else if dir < 0 {
            now.saturating_sub(1)
        } else {
            return None;
        };
        (next != now).then(|| self.volume_at(next))
    }
}

fn text(v: Option<&Value>) -> String {
    let s = v.and_then(Value::as_str).unwrap_or("").trim();
    s.chars().take(TEXT_MAX).collect()
}

fn ms(v: Option<&Value>) -> Option<u64> {
    v.and_then(Value::as_f64)
        .filter(|x| x.is_finite() && *x >= 0.0)
        .map(|x| x as u64)
}

/// A node zone (as `/v1/events` sends it) as the session. `phone`: the zone plays on this app's
/// own output (its volume is Android's media volume, or the DAC's in Exclusive USB);
/// `scale`: its output's volume scale, when known.
pub fn session_for_zone(zone: &Value, phone: bool, scale: Option<Scale>) -> Session {
    let queue = zone.get("queue").and_then(Value::as_array);
    let len = queue.map_or(0, Vec::len);
    let index = zone
        .get("queue_index")
        .and_then(Value::as_u64)
        .map(|i| i as usize);
    let now = zone.get("now_playing").filter(|v| v.is_object());
    let item = now
        .and_then(|n| n.get("item"))
        .or_else(|| index.and_then(|i| queue?.get(i)))
        .filter(|v| v.is_object());
    let state = match zone.get("transport").and_then(Value::as_str) {
        Some("playing") => "playing",
        Some("paused") => "paused",
        _ => "stopped",
    };
    let mut title = text(now.and_then(|n| n.get("stream_title")));
    if title.is_empty() {
        title = text(item.and_then(|i| i.get("title")));
    }
    if title.is_empty() {
        title = text(item.and_then(|i| i.get("uri")));
    }
    let duration_ms = ms(now.and_then(|n| n.get("duration_ms")))
        .or_else(|| ms(item.and_then(|i| i.get("duration_ms"))))
        .unwrap_or(0);
    let repeat_all = zone.get("repeat").and_then(Value::as_str) == Some("all");
    let level = zone.get("volume").filter(|v| v.is_object());
    let volume = match (phone, scale.or_else(|| level.and_then(Scale::of_level))) {
        // The phone's own: Android's media volume, unless the DAC's own (dB, Exclusive USB)
        (true, Some(s @ Scale::Db { .. })) => SessionVolume::Remote {
            max: s.steps(),
            current: level.and_then(|v| s.step_of(v)).unwrap_or(0),
        },
        (true, _) => SessionVolume::Local,
        (false, Some(s)) => SessionVolume::Remote {
            max: s.steps(),
            current: level.and_then(|v| s.step_of(v)).unwrap_or(0),
        },
        (false, None) => SessionVolume::Fixed,
    };
    Session {
        active: len > 0 || item.is_some(),
        zone: text(zone.get("id")),
        zone_name: text(zone.get("name")),
        title,
        artist: text(item.and_then(|i| i.get("artist"))),
        album: text(item.and_then(|i| i.get("album"))),
        duration_ms,
        position_ms: ms(now.and_then(|n| n.get("position_ms"))).unwrap_or(0),
        state,
        speed: if state == "playing" { 1.0 } else { 0.0 },
        can_prev: item.is_some(),
        can_next: index.is_some_and(|i| i + 1 < len) || (repeat_all && len > 0),
        can_seek: duration_ms > 0,
        volume,
        art: item
            .and_then(|i| i.get("image"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= 4096)
            .map(str::to_string),
    }
}

/// What the page's own player last told (its Media Session metadata and `<audio>` state).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PagePlayer {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub art: Option<String>,
    /// `playing`, `paused`, or anything else (`none`).
    pub state: String,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub paused: bool,
    pub handlers: Vec<String>,
}

impl PagePlayer {
    pub fn playing(&self) -> bool {
        !self.paused && !self.title.is_empty()
    }
}

/// The WebView player as the session ("This phone").
pub fn session_for_page(p: &PagePlayer, zone_name: &str) -> Session {
    let has = |h: &str| p.handlers.iter().any(|x| x == h);
    let state = if p.title.is_empty() {
        "stopped"
    } else if p.paused {
        "paused"
    } else {
        "playing"
    };
    Session {
        active: !p.title.is_empty(),
        zone: BROWSER.into(),
        zone_name: zone_name.into(),
        title: p.title.clone(),
        artist: p.artist.clone(),
        album: p.album.clone(),
        duration_ms: p.duration_ms,
        position_ms: p.position_ms,
        state,
        speed: if state == "playing" { 1.0 } else { 0.0 },
        can_prev: has("previoustrack"),
        can_next: has("nexttrack"),
        can_seek: has("seekto") && p.duration_ms > 0,
        volume: SessionVolume::Local,
        art: p.art.clone(),
    }
}

/// A button of the session (the notification, the lock screen, a headset).
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Stop,
    Seek(u64),
    /// The system's volume panel: to step `n`.
    VolumeTo(u32),
    /// A volume key: one step up or down.
    VolumeBy(i32),
}

impl Command {
    /// The node's `POST /v1/zones/{id}/transport` body; `None` for a volume.
    pub fn transport(&self) -> Option<Value> {
        let a = match self {
            Command::Play => "play",
            Command::Pause => "pause",
            Command::Toggle => "toggle",
            Command::Next => "next",
            Command::Previous => "previous",
            Command::Stop => "stop",
            Command::Seek(ms) => return Some(serde_json::json!({"action": "seek", "position_ms": ms})),
            Command::VolumeTo(_) | Command::VolumeBy(_) => return None,
        };
        Some(serde_json::json!({ "action": a }))
    }

    /// The page's Media Session action for it (the WebView player), with the seek time in seconds.
    pub fn page_action(&self) -> Option<(&'static str, Option<f64>)> {
        Some(match self {
            Command::Play => ("play", None),
            Command::Pause => ("pause", None),
            Command::Next => ("nexttrack", None),
            Command::Previous => ("previoustrack", None),
            Command::Stop => ("pause", None),
            Command::Seek(ms) => ("seekto", Some(*ms as f64 / 1000.0)),
            Command::Toggle | Command::VolumeTo(_) | Command::VolumeBy(_) => return None,
        })
    }
}

/// Android's audio focus changes (`AudioManager.AUDIOFOCUS_*`).
pub const FOCUS_GAIN: i64 = 1;
pub const FOCUS_LOSS: i64 = -1;
pub const FOCUS_LOSS_TRANSIENT: i64 = -2;
pub const FOCUS_LOSS_TRANSIENT_CAN_DUCK: i64 = -3;

/// What the app's side sends (one JSON object per `MediaControl.next`).
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Command(Command),
    /// A message from a page, with the origin Android says it came from (main frame only).
    Page {
        origin: String,
        data: String,
    },
    Focus(i64),
    /// Headphones unplugged, a Bluetooth sink gone: Android's "becoming noisy".
    Noisy,
    /// Android's media volume or its route changed (`VOLUME_CHANGED_ACTION`, a device callback).
    MediaVolume,
    /// The app is on screen again.
    Resumed,
}

pub fn parse_event(s: &str) -> Option<Event> {
    if s.is_empty() || s.len() > PAGE_MAX + 4096 {
        return None;
    }
    let v: Value = serde_json::from_str(s).ok()?;
    Some(match v.get("k")?.as_str()? {
        "cmd" => Event::Command(match v.get("a")?.as_str()? {
            "play" => Command::Play,
            "pause" => Command::Pause,
            "toggle" => Command::Toggle,
            "next" => Command::Next,
            "previous" => Command::Previous,
            "stop" => Command::Stop,
            "seek" => Command::Seek(ms(v.get("ms"))?),
            "volume_to" => Command::VolumeTo(u32::try_from(v.get("v")?.as_u64()?).ok()?),
            "volume_by" => Command::VolumeBy(v.get("d")?.as_i64()?.signum() as i32),
            _ => return None,
        }),
        "page" => {
            let data = v.get("data")?.as_str()?;
            if data.len() > PAGE_MAX {
                return None;
            }
            Event::Page {
                origin: v.get("origin")?.as_str()?.to_string(),
                data: data.to_string(),
            }
        }
        "focus" => Event::Focus(v.get("change")?.as_i64()?),
        "noisy" => Event::Noisy,
        "media_volume" => Event::MediaVolume,
        "resumed" => Event::Resumed,
        _ => return None,
    })
}

/// What a page may say (`window.OdeionAndroid`, set up by the app's init script).
#[derive(Debug, Clone, PartialEq)]
pub enum PageMsg {
    /// `OdeionAndroid.controlling(zoneId)`: the zone the page shows and controls.
    Controlling(String),
    /// `OdeionAndroid.setMediaVolume(percent)`: the WebView player's slider.
    SetMediaVolume(u8),
    /// The page's Media Session API, mirrored (the WebView player).
    Player(PagePlayer),
}

/// An `http(s)` address of at most 2 kB, or nothing.
fn web_url(v: Option<&Value>) -> Option<String> {
    let s = v?.as_str()?;
    (s.len() <= 2048 && (s.starts_with("https://") || s.starts_with("http://") || s.starts_with('/')))
        .then(|| s.to_string())
}

/// A page's message, when it is one of the three and within bounds.
pub fn parse_page(data: &str) -> Option<PageMsg> {
    if data.len() > PAGE_MAX {
        return None;
    }
    let v: Value = serde_json::from_str(data).ok()?;
    match v.get("t")?.as_str()? {
        "controlling" => {
            let z = v.get("zone")?.as_str()?;
            (z.len() <= ZONE_MAX && !z.chars().any(char::is_control)).then(|| PageMsg::Controlling(z.into()))
        }
        "volume" => {
            let p = v.get("percent")?.as_f64()?;
            p.is_finite()
                .then(|| PageMsg::SetMediaVolume(p.round().clamp(0.0, 100.0) as u8))
        }
        "session" => {
            let meta = v.get("meta").filter(|m| m.is_object());
            let art = meta
                .and_then(|m| m.get("artwork"))
                .and_then(Value::as_array)
                .and_then(|a| a.iter().find_map(|x| web_url(Some(x))));
            let secs = |k: &str| {
                v.get(k)
                    .and_then(Value::as_f64)
                    .filter(|x| x.is_finite() && *x >= 0.0)
                    .map_or(0, |x| (x * 1000.0) as u64)
            };
            let handlers = v
                .get("handlers")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .filter(|h| h.len() <= 32)
                        .take(16)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            Some(PageMsg::Player(PagePlayer {
                title: text(meta.and_then(|m| m.get("title"))),
                artist: text(meta.and_then(|m| m.get("artist"))),
                album: text(meta.and_then(|m| m.get("album"))),
                art,
                state: text(v.get("state")).chars().take(16).collect(),
                position_ms: secs("pos"),
                duration_ms: secs("dur"),
                paused: v.get("paused").and_then(Value::as_bool).unwrap_or(true),
                handlers,
            }))
        }
        _ => None,
    }
}

/// Whether a page message may be taken: from the node's page or the offline page (exactly their
/// origins), never another site's (Android hands over the main frame's origin only).
pub fn page_allowed(origin: &str, allowed: &[String]) -> bool {
    !origin.is_empty()
        && allowed
            .iter()
            .any(|a| !a.is_empty() && a == origin.trim_end_matches('/'))
}

/// The cover's address as the app fetches it: the node's own (relative, or at its origin) with the
/// app's token; another site's only over https, without it. `None`: not fetched.
pub fn art_request(art: &str, base: &str) -> Option<(String, bool)> {
    let base = base.trim_end_matches('/');
    if art.starts_with('/') && !art.starts_with("//") {
        return Some((format!("{base}{art}"), true));
    }
    let u = url::Url::parse(art).ok()?;
    let node = url::Url::parse(base).ok()?;
    if u.origin() == node.origin() {
        return Some((art.to_string(), true));
    }
    (u.scheme() == "https").then(|| (art.to_string(), false))
}

/// What one audio focus change does to the phone's own players. `ducks`: Android lowers this
/// player itself (shared playback); bit-perfect and Exclusive USB can't be lowered, so they pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusAction {
    /// Pause, and play again when the focus comes back (a call, a navigation prompt).
    PauseForNow,
    /// Pause for good (another app plays now).
    Pause,
    /// Play again, if it was paused for now.
    Resume,
    Nothing,
}

pub fn focus_action(change: i64, ducks: bool) -> FocusAction {
    match change {
        FOCUS_GAIN => FocusAction::Resume,
        FOCUS_LOSS => FocusAction::Pause,
        FOCUS_LOSS_TRANSIENT => FocusAction::PauseForNow,
        FOCUS_LOSS_TRANSIENT_CAN_DUCK if ducks => FocusAction::Nothing,
        FOCUS_LOSS_TRANSIENT_CAN_DUCK => FocusAction::PauseForNow,
        _ => FocusAction::Nothing,
    }
}

/// The page's script for the app's side of the bridge, run before every page (the app's init
/// script, on Android only): `window.OdeionAndroid` with exactly `controlling(zoneId)` and
/// `setMediaVolume(percent)`, over the app's message port (`OdeionAndroidPort`, which Android puts
/// in the main frame of the node's page); and the page's own Media Session API mirrored to the
/// app (metadata, state, which actions it handles, the `<audio>` element's play, pause, seek and
/// duration), as Chrome does for a site. The app's buttons come back as `pmdesktop:media` and call
/// the page's own action handlers. Nothing else crosses.
pub const BRIDGE_JS: &str = r#"(function(){if(window.OdeionAndroid)return;var post=function(m){try{var p=window.OdeionAndroidPort;if(p)p.postMessage(JSON.stringify(m));}catch(e){}};window.OdeionAndroid=Object.freeze({controlling:function(z){post({t:'controlling',zone:String(z==null?'':z)});},setMediaVolume:function(p){post({t:'volume',percent:Number(p)});}});var ms=navigator.mediaSession;if(!ms||typeof MediaSession!=='function')return;var meta=null,st='none',hs={},timer=null;var audio=function(){return document.querySelector('audio#browser')||document.querySelector('audio');};var send=function(){timer=null;var a=audio();post({t:'session',meta:meta,state:st,pos:a?a.currentTime:0,dur:a&&isFinite(a.duration)?a.duration:0,paused:a?a.paused:true,handlers:Object.keys(hs)});};var soon=function(){if(!timer)timer=setTimeout(send,50);};var P=MediaSession.prototype,md=Object.getOwnPropertyDescriptor(P,'metadata'),sd=Object.getOwnPropertyDescriptor(P,'playbackState'),sa=P.setActionHandler;try{if(md&&md.set)Object.defineProperty(ms,'metadata',{configurable:true,get:function(){return md.get.call(ms);},set:function(m){md.set.call(ms,m);meta=m?{title:m.title||'',artist:m.artist||'',album:m.album||'',artwork:(m.artwork||[]).map(function(x){return x&&x.src;})}:null;soon();}});if(sd&&sd.set)Object.defineProperty(ms,'playbackState',{configurable:true,get:function(){return sd.get.call(ms);},set:function(s){sd.set.call(ms,s);st=String(s);soon();}});if(sa)ms.setActionHandler=function(a,f){sa.call(ms,a,f);if(f)hs[a]=f;else delete hs[a];soon();};}catch(e){}['play','pause','seeked','durationchange','ended','emptied'].forEach(function(n){document.addEventListener(n,function(e){if(e.target&&e.target.tagName==='AUDIO')soon();},true);});window.addEventListener('pmdesktop:media',function(e){var d=e.detail||{},h=hs[d.action];if(h)try{h(d.seekTime!=null?{action:d.action,seekTime:d.seekTime}:{action:d.action});}catch(_){}});})();"#;

/// `{base}/v1/zones/{id}/{what}`, the id as one path segment.
pub fn zone_url(base: &str, id: &str, what: &str) -> Option<String> {
    let mut u = url::Url::parse(base.trim_end_matches('/')).ok()?;
    u.path_segments_mut()
        .ok()?
        .pop_if_empty()
        .extend(["v1", "zones", id, what]);
    Some(u.to_string())
}

/// The page's script that says Android's media volume now.
pub fn media_volume_script(p: u8) -> String {
    format!("(function(p){{if(!window.pmDesktop)return;window.pmDesktop.mediaVolume=p;window.dispatchEvent(new Event('pmdesktop:mediavolume'));}})({p});")
}

/// The page's script for one of its own player's Media Session actions.
pub fn page_action_script(action: &str, seek: Option<f64>) -> String {
    let detail = match seek {
        Some(t) if t.is_finite() => serde_json::json!({ "action": action, "seekTime": t }),
        _ => serde_json::json!({ "action": action }),
    };
    format!("window.dispatchEvent(new CustomEvent('pmdesktop:media',{{detail:{detail}}}));")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn zone(transport: &str, volume: Value) -> Value {
        json!({
            "id": "linn-1", "name": "Living room", "output": "openhome:linn", "transport": transport,
            "queue": [{"uri": "a", "title": "One"}, {"uri": "b", "title": "Two"}],
            "queue_index": 0, "repeat": "off",
            "now_playing": {"item": {"uri": "a", "title": "One", "artist": "Artist", "album": "Album",
                "image": "/v1/library/albums/7/cover"}, "index": 0, "position_ms": 61000,
                "duration_ms": 200000},
            "volume": volume
        })
    }

    #[test]
    fn a_network_zone_is_the_session_with_its_own_volume() {
        let s = session_for_zone(
            &zone("playing", json!({"percent": 12, "muted": false})),
            false,
            Some(Scale::Percent),
        );
        assert!(s.active && s.state == "playing");
        assert_eq!(
            (s.title.as_str(), s.artist.as_str(), s.album.as_str()),
            ("One", "Artist", "Album")
        );
        assert_eq!((s.position_ms, s.duration_ms, s.speed), (61000, 200000, 1.0));
        assert!(s.can_prev && s.can_next && s.can_seek);
        assert_eq!(
            s.volume,
            SessionVolume::Remote {
                max: 100,
                current: 12
            }
        );
        assert_eq!(s.art.as_deref(), Some("/v1/library/albums/7/cover"));
        // Paused: the position stands still
        let p = session_for_zone(&zone("paused", json!({"percent": 12})), false, None);
        assert_eq!((p.state, p.speed), ("paused", 0.0));
        // The last item, no repeat: no next; repeat all: next wraps
        let mut last = zone("playing", json!({}));
        last["queue_index"] = json!(1);
        assert!(!session_for_zone(&last, false, None).can_next);
        last["repeat"] = json!("all");
        assert!(session_for_zone(&last, false, None).can_next);
        // A zone the node can't set the volume of: the panel's volume is fixed
        assert_eq!(
            session_for_zone(&zone("playing", Value::Null), false, None).volume,
            SessionVolume::Fixed
        );
        // Nothing queued: the session goes
        let empty = json!({"id": "z", "name": "Z", "transport": "stopped", "queue": [], "now_playing": null});
        assert!(!session_for_zone(&empty, false, None).active);
    }

    #[test]
    fn the_phones_own_zone_keeps_androids_media_volume_unless_the_dac_has_its_own() {
        let pct = session_for_zone(
            &zone("playing", json!({"percent": 40})),
            true,
            Some(Scale::Percent),
        );
        assert_eq!(
            pct.volume,
            SessionVolume::Local,
            "the keys move STREAM_MUSIC themselves"
        );
        let unit = Scale::Db {
            min: -127.0,
            max: 0.0,
            step: 0.5,
        };
        let dac = session_for_zone(&zone("playing", json!({"db": -30.0})), true, Some(unit));
        assert_eq!(
            dac.volume,
            SessionVolume::Remote {
                max: 100,
                current: 76
            },
            "Exclusive USB: the DAC's own volume, in steps"
        );
    }

    #[test]
    fn one_key_press_is_one_step_never_less_than_the_devices_own() {
        // Percent: one percent
        let p = Scale::Percent;
        assert_eq!(p.adjust(&json!({"percent": 4}), 1), Some(json!({"percent": 5})));
        assert_eq!(p.adjust(&json!({"percent": 0}), -1), None, "nothing below 0");
        assert_eq!(p.adjust(&json!({"percent": 100}), 1), None);
        assert_eq!(p.volume_at(37), json!({"percent": 37}));
        // dB, one panel step per device step (-60…0 in 1 dB: 60 steps)
        let d = Scale::Db {
            min: -60.0,
            max: 0.0,
            step: 1.0,
        };
        assert_eq!(d.steps(), 60);
        assert_eq!(d.adjust(&json!({"db": -20.0}), 1), Some(json!({"db": -19.0})));
        assert_eq!(d.adjust(&json!({"db": -20.0}), -1), Some(json!({"db": -21.0})));
        // A fine unit (-127…0 in 0.5 dB): 100 steps, each on the unit's own steps and larger
        let fine = Scale::Db {
            min: -127.0,
            max: 0.0,
            step: 0.5,
        };
        assert_eq!(fine.steps(), 100);
        let mut at = json!({"db": -127.0});
        let mut seen = vec![-127.0f64];
        while let Some(next) = fine.adjust(&at, 1) {
            let db = next["db"].as_f64().unwrap();
            assert!(db > *seen.last().unwrap(), "every press moves it: {seen:?}");
            assert_eq!((db * 2.0).fract(), 0.0, "on the unit's steps: {db}");
            seen.push(db);
            at = next;
        }
        assert_eq!(*seen.last().unwrap(), 0.0);
        assert_eq!(seen.len(), 101);
        // Down from the top comes back the same way
        assert_eq!(fine.adjust(&json!({"db": 0.0}), -1), Some(json!({"db": -1.5})));
        // The panel's position for a level, and back
        for n in 0..=100 {
            assert_eq!(fine.step_of(&fine.volume_at(n)), Some(n), "{n}");
        }
        // Scales from the outputs' caps
        assert_eq!(
            Scale::from_caps(&json!({"kind": "percent"})),
            Some(Scale::Percent)
        );
        assert_eq!(
            Scale::from_caps(&json!({"kind": "db", "min": -99999.0, "max": 0.0, "step": 1.0})),
            Some(Scale::Db {
                min: -60.0,
                max: 0.0,
                step: 1.0
            })
        );
        assert_eq!(Scale::from_caps(&json!({"kind": "incremental"})), None);
    }

    #[test]
    fn the_app_side_and_the_page_are_read_strictly() {
        assert_eq!(
            parse_event(r#"{"k":"cmd","a":"next"}"#),
            Some(Event::Command(Command::Next))
        );
        assert_eq!(
            parse_event(r#"{"k":"cmd","a":"seek","ms":5000}"#),
            Some(Event::Command(Command::Seek(5000)))
        );
        assert_eq!(
            parse_event(r#"{"k":"cmd","a":"volume_by","d":-3}"#),
            Some(Event::Command(Command::VolumeBy(-1)))
        );
        assert_eq!(parse_event(r#"{"k":"cmd","a":"format"}"#), None);
        assert_eq!(parse_event("not json"), None);
        assert_eq!(
            parse_event(r#"{"k":"focus","change":-2}"#),
            Some(Event::Focus(-2))
        );
        // The page: the three messages, within bounds
        assert_eq!(
            parse_page(r#"{"t":"controlling","zone":"linn-1"}"#),
            Some(PageMsg::Controlling("linn-1".into()))
        );
        assert_eq!(
            parse_page(r#"{"t":"volume","percent":250}"#),
            Some(PageMsg::SetMediaVolume(100))
        );
        assert_eq!(parse_page(r#"{"t":"controlling","zone":"a\nb"}"#), None);
        assert_eq!(parse_page(r#"{"t":"exec","js":"x"}"#), None);
        assert_eq!(parse_page(&"x".repeat(PAGE_MAX + 1)), None);
        let Some(PageMsg::Player(p)) = parse_page(
            r#"{"t":"session","meta":{"title":"Song","artist":"A","album":"B","artwork":["javascript:x","/v1/library/albums/3/cover?token=t"]},"state":"playing","pos":12.5,"dur":180,"paused":false,"handlers":["play","pause","nexttrack"]}"#,
        ) else {
            panic!("a session")
        };
        assert_eq!(p.art.as_deref(), Some("/v1/library/albums/3/cover?token=t"));
        assert_eq!((p.position_ms, p.duration_ms), (12500, 180000));
        let s = session_for_page(&p, "This phone");
        assert!(s.active && s.state == "playing" && s.can_next && !s.can_prev && !s.can_seek);
        assert_eq!(s.volume, SessionVolume::Local);
        // Only the node's page (or the offline page), by its exact origin
        let allowed = ["http://192.168.1.10:7788".to_string()];
        assert!(page_allowed("http://192.168.1.10:7788", &allowed));
        assert!(page_allowed("http://192.168.1.10:7788/", &allowed));
        assert!(!page_allowed("http://192.168.1.10:7789", &allowed));
        assert!(!page_allowed("https://www.youtube.com", &allowed));
        assert!(!page_allowed("", &[String::new()]));
    }

    #[test]
    fn covers_come_from_the_node_with_the_token_or_elsewhere_over_https_without() {
        let base = "http://192.168.1.10:7788";
        assert_eq!(
            art_request("/v1/library/albums/7/cover", base),
            Some(("http://192.168.1.10:7788/v1/library/albums/7/cover".into(), true))
        );
        assert_eq!(
            art_request("http://192.168.1.10:7788/x.jpg", base),
            Some(("http://192.168.1.10:7788/x.jpg".into(), true))
        );
        assert_eq!(
            art_request("https://resources.tidal.com/images/a/640x640.jpg", base),
            Some(("https://resources.tidal.com/images/a/640x640.jpg".into(), false))
        );
        assert_eq!(art_request("http://other.example/x.jpg", base), None);
        assert_eq!(art_request("//other.example/x.jpg", base), None);
        assert_eq!(art_request("file:///etc/passwd", base), None);
    }

    #[test]
    fn audio_focus_pauses_the_phones_players_as_android_expects() {
        assert_eq!(focus_action(FOCUS_LOSS, true), FocusAction::Pause);
        assert_eq!(focus_action(FOCUS_LOSS_TRANSIENT, true), FocusAction::PauseForNow);
        // Shared playback: Android ducks it itself; bit-perfect or Exclusive USB: it can't
        assert_eq!(
            focus_action(FOCUS_LOSS_TRANSIENT_CAN_DUCK, true),
            FocusAction::Nothing
        );
        assert_eq!(
            focus_action(FOCUS_LOSS_TRANSIENT_CAN_DUCK, false),
            FocusAction::PauseForNow
        );
        assert_eq!(focus_action(FOCUS_GAIN, false), FocusAction::Resume);
    }

    #[test]
    fn the_bridge_script_offers_exactly_the_two_methods() {
        assert!(BRIDGE_JS.contains("controlling:function(z)"));
        assert!(BRIDGE_JS.contains("setMediaVolume:function(p)"));
        assert!(BRIDGE_JS.contains("Object.freeze"));
        assert!(!BRIDGE_JS.contains("addJavascriptInterface"));
        assert!(!BRIDGE_JS.contains("eval("));
    }

    #[test]
    fn zone_addresses_keep_the_id_as_one_segment() {
        assert_eq!(
            zone_url(
                "http://192.168.1.10:7788/",
                "net:10.0.0.5:7790/cpal:Speaker",
                "volume"
            )
            .as_deref(),
            Some("http://192.168.1.10:7788/v1/zones/net:10.0.0.5:7790%2Fcpal:Speaker/volume")
        );
        assert_eq!(
            page_action_script("seekto", Some(12.5)),
            r#"window.dispatchEvent(new CustomEvent('pmdesktop:media',{detail:{"action":"seekto","seekTime":12.5}}));"#
        );
        let v = media_volume_script(42);
        assert!(v.contains("mediaVolume=p") && v.ends_with("(42);"), "{v}");
    }
}
