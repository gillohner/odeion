//! How a sign-in looks in the sources vault (`crates/pm-appdata/VAULT.md`) as an Odeion node writes
//! it, shared with the phone so a sign-in made there is one a node takes, and the other way round.
//! The node keeps its own copy of a sign-in in `node`: `settings` (what travels of its settings),
//! a `lineage` id a refresh keeps and a new sign-in changes, and `std`, the hash of the standard
//! members as it wrote them (a client that rewrites the tokens itself changes them, so the copy
//! then stops counting as a node's).

use pm_appdata::vault::{ServiceUser, SignIn};
use serde_json::{json, Map, Value};

/// What of a setting travels with the sign-in.
pub enum Part {
    /// The whole value (a session object, a cookie).
    Whole,
    /// These members of a JSON object; the rest stays on the node.
    Members(&'static [&'static str]),
}

/// One setting a sign-in lives in.
pub struct Carry {
    pub key: &'static str,
    pub part: Part,
}

pub const fn whole(key: &'static str) -> Carry {
    Carry {
        key,
        part: Part::Whole,
    }
}

pub const fn members(key: &'static str, m: &'static [&'static str]) -> Carry {
    Carry {
        key,
        part: Part::Members(m),
    }
}

pub fn lineage_after(service: &str, floor: Option<u64>, now: u64) -> String {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let id = blake3::hash(format!("{t}.{n}.{}.{service}", std::process::id()).as_bytes()).to_hex();
    let at = match floor {
        Some(f) if f >= now && f <= now.saturating_add(1) => f + 1,
        _ => now,
    };
    format!("{at}-{}", &id.as_str()[..16])
}

/// When a sign-in id was made (Unix seconds; 0 for an id made for a sign-in that was already
/// there, or one from before ids had a time).
pub fn lineage_at(lineage: &str) -> u64 {
    lineage
        .split_once('-')
        .and_then(|(t, _)| t.parse().ok())
        .unwrap_or(0)
}

/// A setting's value as a JSON object, when it is one.
pub fn object_of(v: &Value) -> Option<Map<String, Value>> {
    match v {
        Value::Object(o) => Some(o.clone()),
        Value::String(raw) => match serde_json::from_str::<Value>(raw).ok()? {
            Value::Object(o) => Some(o),
            _ => None,
        },
        _ => None,
    }
}

pub fn hash(m: &Map<String, Value>) -> String {
    blake3::hash(serde_json::to_string(m).unwrap_or_default().as_bytes())
        .to_hex()
        .to_string()
}

/// The account a copy is for, from the usual members of what travels (`user_id`, `id`,
/// `username`, `user`, Jamendo's `name`, Bandcamp's `fan.fan_id`), looking into a `session` too.
pub fn account(travels: &Map<String, Value>) -> Option<String> {
    fn find(o: &Map<String, Value>) -> Option<String> {
        for k in ["user_id", "id", "username", "user", "name"] {
            match o.get(k) {
                Some(Value::String(v)) if !v.is_empty() => return Some(v.clone()),
                Some(Value::Number(n)) => return Some(n.to_string()),
                _ => {}
            }
        }
        for k in ["session", "fan"] {
            if let Some(Value::Object(inner)) = o.get(k) {
                if let Some(a) = find(inner) {
                    return Some(a);
                }
            }
        }
        o.get("fan_id").map(|v| v.to_string())
    }
    travels.values().filter_map(object_of).find_map(|o| find(&o))
}

/// The OAuth session in what travels, if any (at the top of the main value, or under `session`).
pub fn session_of(carry: &[Carry], travels: &Map<String, Value>) -> Option<Map<String, Value>> {
    let obj = object_of(travels.get(carry.first()?.key)?)?;
    match obj.get("session") {
        Some(Value::Object(o)) if o.contains_key("access_token") => Some(o.clone()),
        _ => obj.contains_key("access_token").then_some(obj),
    }
}

/// The vault sign-in for what travels. It starts from the vault's current copy (`base`), so the
/// members other clients put in (`label`, `way`, `scopes`, `settings`, unknown ones) are kept, and
/// sets the node's copy (`node.settings`), the app it was made with, and the standard members
/// where it is an OAuth session.
pub fn sign_in_for(
    service: &str,
    carry: &[Carry],
    has_app: bool,
    travels: &Map<String, Value>,
    base: Option<&SignIn>,
    app: Option<String>,
    lineage: &str,
) -> SignIn {
    let mut s: SignIn = match base {
        Some(o) => o.clone(),
        None => serde_json::from_value(json!({ "service": service })).expect("a minimal sign-in"),
    };
    s.service = service.to_string();
    s.extra
        .insert("node".into(), json!({ "settings": travels, "lineage": lineage }));
    if has_app {
        s.client_id = app;
    }
    let sess = session_of(carry, travels);
    let mut tokens = Map::new();
    if let Some(sess) = &sess {
        if let Some(at) = sess.get("access_token").and_then(Value::as_str) {
            tokens.insert("access_token".into(), json!(at));
            tokens.insert("token_type".into(), json!("bearer"));
            if let Some(rt) = sess.get("refresh_token").and_then(Value::as_str) {
                tokens.insert("refresh_token".into(), json!(rt));
            }
        }
    }
    s.tokens = tokens;
    s.expires_at = sess
        .as_ref()
        .and_then(|x| x.get("expires_at"))
        .and_then(Value::as_i64)
        .and_then(|e| u64::try_from(e).ok());
    let id = account(travels);
    s.user = id.map(|id| {
        let pick = |keys: &[&str]| {
            keys.iter().find_map(|k| {
                sess.as_ref()
                    .and_then(|x| x.get(*k))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
        };
        ServiceUser {
            id,
            name: pick(&["display_name", "name", "username"]),
            country: pick(&["country", "country_code"]),
            extra: Map::new(),
        }
    });
    // What the standard members were when the node wrote them: a client that writes its own
    // sign-in and keeps `node` (as VAULT.md asks) changes them, so the copy stops counting as the
    // node's
    let std = std_hash(&s);
    if let Some(Value::Object(n)) = s.extra.get_mut("node") {
        n.insert("std".into(), json!(std));
    }
    s
}

/// The standard members a reader without `node` uses: the tokens, their expiry, the account.
pub fn std_hash(s: &SignIn) -> String {
    let std =
        json!({ "tokens": s.tokens, "expires_at": s.expires_at, "user": s.user.as_ref().map(|u| &u.id) });
    blake3::hash(std.to_string().as_bytes()).to_hex().to_string()
}

/// What a node wrote into a vault sign-in (`None`: written by a client that isn't a node, or
/// rewritten by one since: its standard members aren't the node's any more).
pub fn node_settings(s: &SignIn) -> Option<Map<String, Value>> {
    let node = s.extra.get("node")?;
    if node.get("std").and_then(Value::as_str) != Some(std_hash(s).as_str()) {
        return None;
    }
    match node.get("settings")? {
        Value::Object(o) => Some(o.clone()),
        _ => None,
    }
}

/// Which sign-in a vault copy is, when a node wrote it.
pub fn node_lineage(s: &SignIn) -> Option<String> {
    s.extra.get("node")?.get("lineage")?.as_str().map(str::to_string)
}

/// What of the node's settings travels for a bridge (`None`: not signed in here). Whole values are
/// kept as the stored string, members as an object.
/// A setting that can't be read is an error, never "not signed in".
pub fn carried(carry: &[Carry], settings: &dyn Settings) -> Result<Option<Map<String, Value>>, String> {
    let mut m = Map::new();
    for (i, c) in carry.iter().enumerate() {
        let raw = settings
            .get(c.key)
            .map_err(|e| format!("{}: {e}", c.key))?
            .filter(|v| !v.trim().is_empty());
        let part = match (&c.part, raw) {
            (Part::Whole, Some(raw)) => Some(Value::String(raw)),
            (Part::Members(ms), Some(raw)) => {
                let obj = object_of(&Value::String(raw)).unwrap_or_default();
                let sub: Map<String, Value> = ms
                    .iter()
                    .filter_map(|k| {
                        obj.get(*k)
                            .filter(|v| !v.is_null())
                            .map(|v| ((*k).to_string(), v.clone()))
                    })
                    .collect();
                (!sub.is_empty()).then_some(Value::Object(sub))
            }
            (_, None) => None,
        };
        match part {
            Some(v) => {
                m.insert(c.key.to_string(), v);
            }
            None if i == 0 => return Ok(None),
            None => {}
        }
    }
    Ok(Some(m))
}

/// Put what travels into the node's settings: whole values replace the setting, members replace
/// those members and leave the rest of the object. What the copy doesn't have is removed.
/// A write that fails is an error: the caller then records nothing, so the next pass doesn't
/// take a half-applied copy for the node's own.
pub fn apply(carry: &[Carry], travels: &Map<String, Value>, settings: &dyn Settings) -> Result<(), String> {
    for c in carry {
        match &c.part {
            Part::Whole => match travels.get(c.key) {
                Some(Value::String(v)) => {
                    settings.set(c.key, v)?;
                }
                Some(other) => {
                    settings.set(c.key, &other.to_string())?;
                }
                None => {
                    settings.delete(c.key)?;
                }
            },
            Part::Members(ms) => {
                let raw = settings.get(c.key)?;
                let mut obj = raw
                    .as_ref()
                    .and_then(|r| object_of(&Value::String(r.clone())))
                    .unwrap_or_default();
                let from = travels.get(c.key).and_then(object_of).unwrap_or_default();
                for k in *ms {
                    match from.get(*k) {
                        Some(v) => {
                            obj.insert((*k).to_string(), v.clone());
                        }
                        None => {
                            obj.remove(*k);
                        }
                    }
                }
                if obj.is_empty() {
                    settings.delete(c.key)?;
                } else {
                    settings.set(c.key, &Value::Object(obj).to_string())?;
                }
            }
        }
    }
    Ok(())
}

/// Where the settings a sign-in lives in are kept (the node: its library's settings; the phone:
/// its sign-ins file).
pub trait Settings {
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, value: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
}

/// One sign-in the phone and the node both carry: its id in the vault and the settings it lives
/// in (as the node's bridge for it has them), with an app id.
pub struct Form {
    pub service: &'static str,
    pub carry: &'static [Carry],
}

/// TIDAL full tracks: the session in `tidal.unofficial` (the node's bridge `tidal-full`).
pub const TIDAL_FULL: Form = Form {
    service: "tidal-full",
    carry: &[members("tidal.unofficial", &["session"])],
};

/// Qobuz: the session in `qobuz.unofficial` (the node's bridge `qobuz`).
pub const QOBUZ: Form = Form {
    service: "qobuz",
    carry: &[members("qobuz.unofficial", &["session"])],
};
