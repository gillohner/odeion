//! App passwords: one for each app the owner connects (Home Assistant, a Subsonic app, a script),
//! instead of the node's API token. An app password plays, controls zones and browses as the
//! owner, and nothing else ([`crate::accounts::Role::App`], `access.rs`). Removing one in
//! Settings › People & access stops that app alone.
//!
//! The node keeps each password itself, not only a hash of it: a Subsonic app proves its password
//! as md5(password + salt), which needs the password. They stay on this node: never in a backup
//! (`backup::NODE_OWN`), and a restore keeps the node's own.

use crate::state::NodeState;
use crate::{ApiError, ApiResult};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Where they are kept: this node's own (`backup::NODE_OWN`).
pub(crate) const KEY: &str = "accounts.app_passwords";
/// The start of every app password.
pub const PREFIX: &str = "pma_";
/// App passwords kept at once.
const CAP: usize = 20;
/// The longest name kept, in characters.
const NAME_MAX: usize = 60;

/// An app password as the node keeps it.
#[derive(Clone, Serialize, Deserialize)]
struct Stored {
    id: String,
    name: String,
    password: String,
    created_at: i64,
    /// The day it was last used (unix seconds at that UTC day's start), written once a day at most.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_used: Option<i64>,
}

/// An app password as Settings lists it: never the password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listed {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<i64>,
}

/// A new app password: the password is shown this once.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Made {
    #[serde(flatten)]
    pub listed: Listed,
    pub password: String,
}

impl From<&Stored> for Listed {
    fn from(a: &Stored) -> Self {
        Listed {
            id: a.id.clone(),
            name: a.name.clone(),
            created_at: a.created_at,
            last_used: a.last_used,
        }
    }
}

/// The kept list; an empty one when there is none, an error when it can't be read (so nothing
/// writes over a list it could not read).
fn load(lib: &pm_library::Library) -> Result<Vec<Stored>, ApiError> {
    let Some(json) = lib.get_setting(KEY)? else {
        return Ok(Vec::new());
    };
    serde_json::from_str(&json).map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot read the app passwords: {e}"),
        )
    })
}

fn store(lib: &pm_library::Library, all: &[Stored]) -> Result<(), ApiError> {
    lib.set_setting(KEY, &serde_json::to_string(all).expect("serializes"))?;
    // Answers to people and apps hide the node's passwords (`accounts::KnownSet`): read them again
    crate::accounts::sign_ins_changed();
    Ok(())
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The app passwords, newest first.
pub fn list(s: &NodeState) -> Result<Vec<Listed>, ApiError> {
    let lib = s.library.lock().expect("library poisoned");
    // (kept oldest first: the later of two made in the same second comes first)
    let mut out: Vec<Listed> = load(&lib)?.iter().rev().map(Listed::from).collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.created_at));
    Ok(out)
}

/// A new app password named `name`.
pub fn make(s: &NodeState, name: &str) -> Result<Made, ApiError> {
    let name: String = name.trim().chars().filter(|c| !c.is_control()).collect();
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request("name the app this password is for"));
    }
    let name: String = name
        .chars()
        .take(NAME_MAX)
        .collect::<String>()
        .trim_end()
        .to_string();
    let lib = s.library.lock().expect("library poisoned");
    let mut all = load(&lib)?;
    if all.len() >= CAP {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("a node keeps {CAP} app passwords at most: remove one you no longer use first"),
        ));
    }
    let a = Stored {
        id: crate::tidal::random_urlsafe(8),
        name,
        password: format!("{PREFIX}{}", crate::tidal::random_urlsafe(32)),
        created_at: now(),
        last_used: None,
    };
    all.push(a.clone());
    store(&lib, &all)?;
    Ok(Made {
        listed: Listed::from(&a),
        password: a.password,
    })
}

/// Removes the app password `id`; whether there was one. The app stops working at once: its
/// next request, and its live updates within seconds.
pub fn remove(s: &NodeState, id: &str) -> Result<bool, ApiError> {
    let lib = s.library.lock().expect("library poisoned");
    let mut all = load(&lib)?;
    let before = all.len();
    all.retain(|a| a.id != id);
    if all.len() == before {
        return Ok(false);
    }
    store(&lib, &all)?;
    Ok(true)
}

/// The id of the app password `token` is, if it is one. Compared through hashes, which compare
/// in constant time.
pub fn of(s: &NodeState, token: &str) -> Option<String> {
    if !token.starts_with(PREFIX) {
        return None;
    }
    let lib = s.library.lock().expect("library poisoned");
    let all = load(&lib).ok()?;
    let asked = blake3::hash(token.as_bytes());
    let mut found = None;
    // Every one is compared, the first match kept: how long this takes says nothing of which
    for a in &all {
        if blake3::hash(a.password.as_bytes()) == asked && found.is_none() {
            found = Some(a.id.clone());
        }
    }
    found
}

/// The app password whose md5(password + `salt`) is `token` (Subsonic's token sign-in), if one is.
pub fn of_subsonic_token(s: &NodeState, salt: &str, token: &str) -> Option<String> {
    use md5::{Digest, Md5};
    let lib = s.library.lock().expect("library poisoned");
    let all = load(&lib).ok()?;
    let asked = token.to_ascii_lowercase();
    let mut found = None;
    for a in &all {
        let mut h = Md5::new();
        h.update(a.password.as_bytes());
        h.update(salt.as_bytes());
        if same(hex::encode(h.finalize()).as_bytes(), asked.as_bytes()) && found.is_none() {
            found = Some(a.id.clone());
        }
    }
    found
}

/// Whether `a` and `b` are the same, in a time that depends only on their lengths.
pub(crate) fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// Writes that the app password `id` was used on `today` (`accounts::caller_seen` asks once a
/// day); whether the list says so now. Never an error: a request is not refused because the day
/// could not be written.
pub(crate) fn used_today(s: &NodeState, id: &str, today: i64) -> bool {
    let lib = s.library.lock().expect("library poisoned");
    let Ok(mut all) = load(&lib) else {
        return false;
    };
    let Some(a) = all.iter_mut().find(|a| a.id == id) else {
        return false;
    };
    if a.last_used == Some(today) {
        return true;
    }
    a.last_used = Some(today);
    // The set of passwords is the same: no `sign_ins_changed`
    lib.set_setting(KEY, &serde_json::to_string(&all).expect("serializes"))
        .is_ok()
}

/// [`used_today`] for a request that `accounts::caller_seen` doesn't see (Subsonic): once a day.
pub(crate) fn note_used_now(s: &NodeState, id: &str) {
    let today = now().div_euclid(86_400) * 86_400;
    let seen = format!("app {id}");
    if s.sign_ins_seen.lock().expect("sign-ins seen").get(&seen) == Some(&today) {
        return;
    }
    if used_today(s, id, today) {
        s.sign_ins_seen.lock().expect("sign-ins seen").insert(seen, today);
    }
}

/// The kept passwords, for hiding them in answers to people and apps (`accounts::KnownSet`).
pub(crate) fn passwords(lib: &pm_library::Library) -> Vec<String> {
    load(lib)
        .unwrap_or_default()
        .into_iter()
        .map(|a| a.password)
        .collect()
}

/// `GET /v1/node/app-passwords`.
pub(crate) async fn get(State(s): State<Arc<NodeState>>) -> ApiResult<Vec<Listed>> {
    Ok(Json(list(&s)?))
}

#[derive(Debug, Deserialize)]
pub(crate) struct New {
    name: String,
}

/// `POST /v1/node/app-passwords {name}`: a new app password, shown this once.
pub(crate) async fn post(State(s): State<Arc<NodeState>>, Json(b): Json<New>) -> ApiResult<Made> {
    Ok(Json(make(&s, &b.name)?))
}

/// `DELETE /v1/node/app-passwords/{id}`.
pub(crate) async fn delete(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    if !remove(&s, &id)? {
        return Err(ApiError::not_found("no such app password"));
    }
    Ok(Json(serde_json::json!({"removed": true})))
}
