//! People on this node and its bundled homeserver (STEWARD-BUNDLED-HOMESERVER.md, step 3): the
//! owner invites someone and picks what they get.
//!
//! - **Homeserver only:** a normal Pubky homeserver account here, with a storage quota.
//! - **Homeserver + music:** the same, and they may also use this node's music service.
//! - **Music only:** someone whose Pubky account is elsewhere, added by their public key.
//!
//! Homeserver invites are signup tokens minted through the admin API, shown as a Pubky Ring link
//! (`pubkyauth://direct_signup?hs=…&st=…`) and QR. A used token names its user (`used_by`), which
//! is how an invite becomes a person. Per person, the owner sets the storage quota, turns the
//! homeserver account on or off (off blocks writes; reads stay) and music access on or off.
//!
//! The node keeps its own list in the library settings (`people`). Who may sign in to the music
//! service is decided by that list; the sign-in itself is the node's Pubky Ring sign-in.

use crate::bundled::{admin, endpoint};
use crate::state::NodeState;
use crate::{ApiError, ApiResult};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

pub(crate) const SETTING: &str = "people";

/// What an invite gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// A Pubky homeserver account here, nothing more.
    Homeserver,
    /// A homeserver account here and this node's music service.
    HomeserverMusic,
    /// The music service only (their account is on another homeserver).
    Music,
}

impl Access {
    pub fn on_homeserver(self) -> bool {
        matches!(self, Self::Homeserver | Self::HomeserverMusic)
    }
    pub fn music(self) -> bool {
        matches!(self, Self::HomeserverMusic | Self::Music)
    }
}

/// An invite the owner made; `key` is filled once someone used it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Invite {
    pub id: String,
    pub access: Access,
    /// Who it is for, as the owner wrote it.
    #[serde(default)]
    pub name: Option<String>,
    /// The homeserver signup token (homeserver invites).
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub storage_quota_mb: Option<u64>,
    pub created_at: i64,
}

/// Someone with access to this node or its homeserver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Person {
    /// Their public key (z32).
    pub key: String,
    pub access: Access,
    #[serde(default)]
    pub name: Option<String>,
    /// Music access on (the owner can take it away without touching their homeserver account).
    pub music_enabled: bool,
    pub added_at: i64,
    /// Their account on the bundled homeserver is on. The admin API has no way to read this, so
    /// the node keeps what it last set (`PATCH /v1/people/{key} {homeserver_enabled}`).
    #[serde(default = "on")]
    pub homeserver_enabled: bool,
    /// Their problem reports' name for this time they have music access, new each time they are
    /// given it: reports from an earlier time are never theirs again, and go at the next
    /// housekeeping (feedback.rs). `None` for someone listed before the node kept one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reports: Option<String>,
}

fn on() -> bool {
    true
}

/// A new name for someone's problem reports (random, 16 bytes in hex).
fn new_reports() -> String {
    rand::random::<[u8; 16]>()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Roster {
    #[serde(default)]
    pub people: Vec<Person>,
    #[serde(default)]
    pub invites: Vec<Invite>,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Whether `key` is listed with music access on (the library alone: the audio thread's check).
pub(crate) fn listed_for_music(lib: &pm_library::Library, key: &str) -> bool {
    lib.get_setting(SETTING)
        .ok()
        .flatten()
        .and_then(|j| serde_json::from_str::<Roster>(&j).ok())
        .is_some_and(|r| {
            r.people
                .iter()
                .any(|p| p.key == key && p.access.music() && p.music_enabled)
        })
}

pub fn load(s: &NodeState) -> Roster {
    s.library
        .lock()
        .expect("library poisoned")
        .get_setting(SETTING)
        .ok()
        .flatten()
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default()
}

/// Someone with music access: their key, and their reports' name for this membership
/// ([`Person::reports`]).
pub type MusicPerson = (String, Option<String>);

/// The people with music access, each with their reports' name for this membership
/// ([`Person::reports`]); a paused music switch still counts, People › Remove takes access away,
/// also from someone kept listed for their homeserver account. `None` when the roster can't be
/// read (a damaged setting): unlike [`load`], never an empty roster in its place, so what deletes
/// by it never takes everyone's.
pub fn music_reporters(s: &NodeState) -> Option<Vec<MusicPerson>> {
    match s.library.lock().expect("library poisoned").get_setting(SETTING) {
        Ok(None) => Some(Vec::new()),
        Ok(Some(j)) => serde_json::from_str::<Roster>(&j).ok().map(|r| {
            r.people
                .into_iter()
                .filter(|p| p.access.music())
                .map(|p| (p.key, p.reports))
                .collect()
        }),
        Err(_) => None,
    }
}

/// `key`'s reports' name for this membership ([`Person::reports`]), if they are listed with one.
pub fn reports_of(s: &NodeState, key: &str) -> Option<String> {
    let key = key.trim_start_matches("pubky");
    load(s)
        .people
        .into_iter()
        .find(|p| p.key == key)
        .and_then(|p| p.reports)
}

fn store(s: &NodeState, r: &Roster) -> Result<(), ApiError> {
    s.library
        .lock()
        .expect("library poisoned")
        .set_setting(SETTING, &serde_json::to_string(r).expect("serializes"))?;
    Ok(())
}

/// Change the roster: `f` gets the stored roster as it is now, and what it leaves is stored, while
/// every other change waits. Network calls go before, never inside, so a change made while one
/// was on its way is never overwritten with an older copy. Answers `f`'s value and the roster.
async fn update<T>(
    s: &NodeState,
    f: impl FnOnce(&mut Roster) -> Result<T, ApiError>,
) -> Result<(T, Roster), ApiError> {
    let _one = s.roster.lock().await;
    let mut r = load(s);
    let before = r.clone();
    let out = f(&mut r)?;
    if r != before {
        store(s, &r)?;
    }
    Ok((out, r))
}

/// Whether `key` (z32, with or without the `pubky` prefix) may use the music service: the owner
/// always; anyone else while listed with music access on.
pub fn may_use_music(s: &NodeState, key: &str) -> bool {
    let key = key.trim_start_matches("pubky");
    if s.sync_handle()
        .is_some_and(|h| h.z32().trim_start_matches("pubky") == key)
    {
        return true;
    }
    load(s)
        .people
        .iter()
        .any(|p| p.key == key && p.access.music() && p.music_enabled)
}

/// A raw z32 public key (52 characters of the z-base-32 alphabet).
pub(crate) fn valid_key(k: &str) -> Result<String, ApiError> {
    let k = k.trim().trim_start_matches("pubky:").trim_start_matches("pubky");
    if k.len() != 52
        || !k
            .bytes()
            .all(|b| b"ybndrfg8ejkmcpqxot1uwisza345h769".contains(&b))
    {
        return Err(ApiError::bad_request(
            "a Pubky public key is 52 z-base-32 characters",
        ));
    }
    Ok(k.to_string())
}

/// Used homeserver tokens become people: `token → used_by`.
async fn used_tokens(s: &NodeState) -> Result<Vec<(String, String)>, ApiError> {
    let v = admin(s, Method::GET, "/signup_tokens?state=used&limit=1000", None).await?;
    Ok(v["items"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|i| {
                    Some((
                        i["token"].as_str()?.to_string(),
                        i["used_by"].as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Turns used invites into people (once). The homeserver is asked first; the people it names are
/// then added to the roster as it is by then.
async fn reconcile(s: &NodeState) -> Result<Roster, ApiError> {
    let r = load(s);
    if r.invites.iter().all(|i| i.token.is_none()) || s.bundled_homeserver().is_none() {
        return Ok(r);
    }
    let used = used_tokens(s).await?;
    let ((), r) = update(s, |r| {
        add_used(r, &used);
        Ok(())
    })
    .await?;
    Ok(r)
}

/// Invites whose token was used become people.
fn add_used(r: &mut Roster, used: &[(String, String)]) {
    let Roster { people, invites } = r;
    invites.retain(|inv| {
        let Some(t) = &inv.token else { return true };
        let Some((_, key)) = used.iter().find(|(tok, _)| tok == t) else {
            return true;
        };
        if !people.iter().any(|p| &p.key == key) {
            people.push(Person {
                key: key.clone(),
                access: inv.access,
                name: inv.name.clone(),
                music_enabled: inv.access.music(),
                added_at: now(),
                homeserver_enabled: true,
                reports: inv.access.music().then(new_reports),
            });
        }
        false
    });
}

/// A person as the owner sees them, with their homeserver account's state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonView {
    #[serde(flatten)]
    pub person: Person,
    /// Their storage quota on the bundled homeserver (`unlimited`, a number of MB, or the default).
    pub storage_quota_mb: Option<Value>,
    /// Devices signed in to the music service with their own token.
    pub sessions: usize,
    /// Bytes their files take on the bundled homeserver, counted in its files folder when the node
    /// has it (`[homeserver] files_dir`); `None` otherwise.
    pub used_bytes: Option<u64>,
}

/// Sizes of the files under `dir`, not following links. `None` when `dir` isn't there.
fn folder_bytes(dir: &std::path::Path) -> Option<u64> {
    let mut total = 0u64;
    let mut todo = vec![std::fs::read_dir(dir).ok()?];
    while let Some(rd) = todo.pop() {
        for e in rd.flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                if let Ok(sub) = std::fs::read_dir(e.path()) {
                    todo.push(sub);
                }
            } else if m.is_file() {
                total += m.len();
            }
        }
    }
    Some(total)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteView {
    #[serde(flatten)]
    pub invite: Invite,
    /// The Pubky Ring link and its QR code (homeserver invites).
    pub url: Option<String>,
    pub qr_svg: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeopleView {
    /// The node's owner (its Pubky identity), when signed in.
    pub owner: Option<String>,
    pub people: Vec<PersonView>,
    pub invites: Vec<InviteView>,
    /// The bundled homeserver as a whole (users, disk used): `GET /v1/homeserver/bundled`.
    pub homeserver: crate::bundled::BundledStatus,
}

async fn homeserver_key(s: &NodeState) -> Result<String, ApiError> {
    let info = admin(s, Method::GET, "/info", None).await?;
    info["public_key"].as_str().map(str::to_string).ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "the bundled homeserver has no public key",
        )
    })
}

fn invite_link(hs: &str, token: &str) -> String {
    format!("pubkyauth://direct_signup?hs={hs}&st={token}")
}

fn invite_view(inv: Invite, hs: Option<&str>) -> InviteView {
    let url = match (hs, &inv.token) {
        (Some(hs), Some(t)) => Some(invite_link(hs, t)),
        _ => None,
    };
    InviteView {
        qr_svg: url.as_deref().and_then(crate::qr_svg),
        url,
        invite: inv,
    }
}

/// `GET /v1/people`.
pub(crate) async fn list(State(s): State<Arc<NodeState>>) -> ApiResult<PeopleView> {
    let r = reconcile(&s).await?;
    let homeserver = if s.bundled_homeserver().is_some() {
        match admin(&s, Method::GET, "/info", None).await {
            Ok(v) => crate::bundled::status_from_info(&v),
            Err(e) => crate::bundled::BundledStatus {
                configured: true,
                error: Some(e.message),
                ..Default::default()
            },
        }
    } else {
        Default::default()
    };
    let hs = homeserver.public_key.clone();
    let files = s.homeserver_files_dir().map(std::path::Path::to_path_buf);
    let mut people = Vec::new();
    for p in r.people {
        let used_bytes = match (&files, p.access.on_homeserver()) {
            (Some(root), true) => {
                let dir = root.join(&p.key);
                tokio::task::spawn_blocking(move || folder_bytes(&dir))
                    .await
                    .ok()
                    .flatten()
            }
            _ => None,
        };
        let quota = if p.access.on_homeserver() && homeserver.reachable {
            admin(&s, Method::GET, &format!("/users/{}/quota", p.key), None)
                .await
                .ok()
                .map(|q| q["effective"]["storage_quota_mb"].clone())
        } else {
            None
        };
        people.push(PersonView {
            sessions: crate::accounts::sessions_of(&s, &p.key),
            person: p,
            storage_quota_mb: quota,
            used_bytes,
        });
    }
    Ok(Json(PeopleView {
        owner: s.sync_handle().map(|h| h.z32().to_string()),
        people,
        invites: r
            .invites
            .into_iter()
            .map(|i| invite_view(i, hs.as_deref()))
            .collect(),
        homeserver,
    }))
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewInvite {
    pub access: Access,
    #[serde(default)]
    pub name: Option<String>,
    /// Homeserver invites: their storage in MB; absent = the homeserver's default.
    #[serde(default)]
    pub storage_quota_mb: Option<u64>,
    /// Music-only: their public key.
    #[serde(default)]
    pub key: Option<String>,
}

fn clean_name(n: Option<String>) -> Option<String> {
    n.map(|n| n.trim().chars().take(80).collect::<String>())
        .filter(|n| !n.is_empty())
}

/// `POST /v1/people/invites {access, name?, storage_quota_mb?, key?}`.
/// Homeserver invites answer with the Ring link and QR; a music-only one adds the person by key.
pub(crate) async fn invite(State(s): State<Arc<NodeState>>, Json(b): Json<NewInvite>) -> ApiResult<Value> {
    let name = clean_name(b.name);
    if !b.access.on_homeserver() {
        let key = valid_key(b.key.as_deref().unwrap_or(""))?;
        update(&s, |r| {
            if let Some(p) = r.people.iter_mut().find(|p| p.key == key) {
                p.music_enabled = true;
                if p.access == Access::Homeserver {
                    p.access = Access::HomeserverMusic;
                    p.reports = Some(new_reports());
                }
            } else {
                r.people.push(Person {
                    key: key.clone(),
                    access: Access::Music,
                    name,
                    music_enabled: true,
                    added_at: now(),
                    homeserver_enabled: true,
                    reports: Some(new_reports()),
                });
            }
            Ok(())
        })
        .await?;
        return Ok(Json(serde_json::json!({"person": key})));
    }
    endpoint(&s)?;
    let quota = match b.storage_quota_mb {
        Some(mb) => serde_json::json!({"storage_quota_mb": mb}),
        None => serde_json::json!({}),
    };
    let v = admin(&s, Method::POST, "/generate_signup_token", Some(&quota)).await?;
    let token = v.as_str().unwrap_or_default().to_string();
    if !crate::bundled::is_signup_token(&token) {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "the bundled homeserver did not hand out a signup token",
        ));
    }
    let inv = Invite {
        id: crate::tidal::random_urlsafe(9),
        access: b.access,
        name,
        token: Some(token),
        storage_quota_mb: b.storage_quota_mb,
        created_at: now(),
    };
    update(&s, |r| {
        r.invites.push(inv.clone());
        Ok(())
    })
    .await?;
    let hs = homeserver_key(&s).await?;
    Ok(Json(
        serde_json::to_value(invite_view(inv, Some(&hs))).expect("serializes"),
    ))
}

/// `DELETE /v1/people/invites/{id}`: forgets an unused invite (its token stays valid on the
/// homeserver until used; the admin API cannot revoke one).
pub(crate) async fn forget_invite(
    State(s): State<Arc<NodeState>>,
    Path(id): Path<String>,
) -> ApiResult<Value> {
    update(&s, |r| {
        let before = r.invites.len();
        r.invites.retain(|i| i.id != id);
        if r.invites.len() == before {
            return Err(ApiError::not_found("no such invite"));
        }
        Ok(())
    })
    .await?;
    Ok(Json(serde_json::json!({"forgotten": id})))
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PersonChange {
    #[serde(default)]
    pub name: Option<String>,
    /// Music access on or off.
    #[serde(default)]
    pub music: Option<bool>,
    /// Their homeserver account on or off (off blocks their writes; reads and deletes stay).
    #[serde(default)]
    pub homeserver_enabled: Option<bool>,
    /// MB, or `"unlimited"`, or `null` for the homeserver's default.
    #[serde(default, deserialize_with = "some_value")]
    pub storage_quota_mb: Option<Value>,
}

/// Keeps an explicit `null` (reset to the default) apart from an absent field.
fn some_value<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}

/// `PATCH /v1/people/{key} {name?, music?, homeserver_enabled?, storage_quota_mb?}`.
pub(crate) async fn change(
    State(s): State<Arc<NodeState>>,
    Path(key): Path<String>,
    Json(b): Json<PersonChange>,
) -> ApiResult<Person> {
    let key = valid_key(&key)?;
    let r = reconcile(&s).await?;
    let Some(found) = r.people.iter().find(|p| p.key == key) else {
        return Err(ApiError::not_found("no such person"));
    };
    let on_hs = found.access.on_homeserver();
    if let Some(q) = &b.storage_quota_mb {
        if !on_hs {
            return Err(ApiError::bad_request(
                "their account is not on this node's homeserver",
            ));
        }
        let ok = q.is_null() || q.as_u64().is_some() || q.as_str() == Some("unlimited");
        if !ok {
            return Err(ApiError::bad_request(
                "storage_quota_mb is a number of MB, \"unlimited\" or null",
            ));
        }
        admin(
            &s,
            Method::PATCH,
            &format!("/users/{key}/quota"),
            Some(&serde_json::json!({"storage_quota_mb": q})),
        )
        .await?;
    }
    if let Some(on) = b.homeserver_enabled {
        if !on_hs {
            return Err(ApiError::bad_request(
                "their account is not on this node's homeserver",
            ));
        }
        let path = format!("/users/{key}/{}", if on { "enable" } else { "disable" });
        admin(&s, Method::POST, &path, None).await?;
    }
    // The homeserver calls are done; the change goes to the roster as it is now.
    let rename = b.name.is_some().then(|| clean_name(b.name));
    let (out, _) = update(&s, |r| {
        let Some(p) = r.people.iter_mut().find(|p| p.key == key) else {
            return Err(ApiError::not_found("no such person"));
        };
        if let Some(on) = b.homeserver_enabled {
            p.homeserver_enabled = on;
        }
        if let Some(m) = b.music {
            p.music_enabled = m;
            if m && p.access == Access::Homeserver {
                p.access = Access::HomeserverMusic;
                p.reports = Some(new_reports());
            }
        }
        if let Some(name) = rename {
            p.name = name;
        }
        Ok(p.clone())
    })
    .await?;
    // Music access gone: their phones and laptops go too (devices.rs)
    if b.music == Some(false) {
        crate::devices::forget_all_of(&s, &out.key);
        // Their tokens stop working now: their usage insights end too (usage.rs)
        s.usage.revoke_listener(&out.key);
    }
    Ok(Json(out))
}

/// `DELETE /v1/people/{key}`: remove a person. Everything the node keeps for them goes: their own
/// streaming sign-ins, tokens (signed out everywhere), preferences and devices. Someone with an
/// account on this node's homeserver stays listed, without music access, so that account stays
/// in sight; anyone else leaves People.
pub(crate) async fn remove(State(s): State<Arc<NodeState>>, Path(key): Path<String>) -> ApiResult<Roster> {
    let key = valid_key(&key)?;
    reconcile(&s).await?;
    let ((), r) = update(&s, |r| {
        let Some(i) = r.people.iter().position(|p| p.key == key) else {
            return Err(ApiError::not_found("no such person"));
        };
        if r.people[i].access.on_homeserver() {
            let p = &mut r.people[i];
            p.music_enabled = false;
            if p.access == Access::HomeserverMusic {
                p.access = Access::Homeserver;
                p.reports = None;
            }
        } else {
            r.people.remove(i);
        }
        // Their problem reports go now, while no one can give them music again: done after,
        // a report sent in between under a new membership would go too. If the inbox can't be
        // written now, they go at the node's next housekeeping (feedback.rs)
        if let Err(e) = crate::feedback::forget_listener(&s.data_dir(), &key) {
            tracing::warn!("feedback: a removed listener's reports not removed now: {e}");
        }
        Ok(())
    })
    .await?;
    crate::accounts::forget_person(&s, &key)?;
    Ok(Json(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_kinds_and_keys() {
        assert!(Access::Homeserver.on_homeserver() && !Access::Homeserver.music());
        assert!(Access::HomeserverMusic.on_homeserver() && Access::HomeserverMusic.music());
        assert!(!Access::Music.on_homeserver() && Access::Music.music());
        let k = "tbepfp8fisyiqpskbmxsmpub47gxb7xdsh5x9ut9qrb97gcq9phy";
        assert_eq!(valid_key(&format!("pubky{k}")).unwrap(), k);
        assert!(valid_key("not-a-key").is_err());
        assert_eq!(
            invite_link("hs1", "AB12-CD34-EF56"),
            "pubkyauth://direct_signup?hs=hs1&st=AB12-CD34-EF56"
        );
        let c: PersonChange = serde_json::from_str(r#"{"storage_quota_mb": null}"#).unwrap();
        assert_eq!(
            c.storage_quota_mb,
            Some(Value::Null),
            "null resets to the default"
        );
        let c: PersonChange = serde_json::from_str(r#"{"music": false}"#).unwrap();
        assert!(c.storage_quota_mb.is_none());
        // Rosters saved before the node kept the homeserver switch read as "on"
        let p: Person =
            serde_json::from_str(r#"{"key": "k", "access": "music", "music_enabled": true, "added_at": 1}"#)
                .unwrap();
        assert!(p.homeserver_enabled);
    }

    #[cfg(unix)]
    #[test]
    fn folder_bytes_counts_files_not_links() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(folder_bytes(&d.path().join("nobody")), None);
        std::fs::create_dir_all(d.path().join("u/pub/a")).unwrap();
        std::fs::write(d.path().join("u/pub/a/one"), b"12345").unwrap();
        std::fs::write(d.path().join("u/two"), b"123").unwrap();
        std::os::unix::fs::symlink(d.path().join("u/pub"), d.path().join("u/loop")).unwrap();
        assert_eq!(folder_bytes(&d.path().join("u")), Some(8));
    }
}
