//! The person's Odeion data on their homeserver, from the phone, written with pm-pubky's own v1
//! writers (the node's): Bookmarks (social v1 bookmarks of canonical music URLs), listens (music v1
//! `listen` posts, private) and the shared preferences (`/priv/pubky.music/prefs.json`, newest
//! `updated_at` wins; the phone uses its streaming quality).
//!
//! Phone mode keeps working without the homeserver: what the page changes is kept on the phone
//! (`phone/<pubky>/data.json`: the Bookmarks as last known, and the writes still to make) and the
//! writes go out at the next sync, in order.

use super::account::Account;
use super::Phone;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use pm_pubky::v1;
use pm_sources::SourceError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// A write the homeserver hasn't had yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Pending {
    Bookmark {
        url: String,
        on: bool,
        at: i64,
    },
    Listen {
        url: String,
        service: Option<String>,
        duration_ms: Option<u64>,
        at: i64,
        /// The post's id: its write time, set at the first try and kept for the retries (as a
        /// node, which dates a listen's id by when it writes it, not by the play).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// A source's streaming quality changed here.
    Quality {
        source: String,
    },
}

/// What the phone keeps of the person's data.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Local {
    /// Bookmarked canonical URLs and when (microseconds).
    #[serde(default)]
    pub bookmarks: BTreeMap<String, i64>,
    #[serde(default)]
    pub pending: Vec<Pending>,
    /// The `updated_at` of the newest prefs.json the phone took or wrote.
    #[serde(default)]
    pub prefs_at: u64,
    /// The Bookmarks as last read, by file: only new files are read again.
    #[serde(default)]
    pub known: std::collections::HashMap<String, (String, i64)>,
}

fn path(phone: &Phone) -> PathBuf {
    phone.dir().join("data.json")
}

/// What the phone keeps; with no file yet, nothing. One that can't be read, or isn't its JSON, is
/// an error and is left as it is (read as empty, the next change would save over the writes still
/// waiting): nothing changes and nothing is written to the homeserver until it reads again.
pub(crate) fn load(phone: &Phone) -> std::io::Result<Local> {
    let p = path(phone);
    let bad = |e: &dyn std::fmt::Display| {
        tracing::error!("phone mode: {} can't be read ({e}); left as it is", p.display());
        std::io::Error::other(format!("what this phone keeps of your data can't be read ({e})"))
    };
    match std::fs::read(&p) {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| bad(&e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Local::default()),
        Err(e) => Err(bad(&e)),
    }
}

/// Written whole, on the disk before it replaces the old one (a power cut never leaves it cut
/// short).
fn save(phone: &Phone, l: &Local) -> std::io::Result<()> {
    use std::io::Write;
    let p = path(phone);
    let tmp = p.with_extension("json.tmp");
    let b = serde_json::to_vec(l).map_err(std::io::Error::other)?;
    let write = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&b)?;
        f.sync_all()?;
        std::fs::rename(&tmp, &p)
    };
    write().inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// One change to what the phone keeps, made under the phone's data lock. An error when it couldn't
/// be read or saved: then nothing changed (the file is what every reader reads).
#[must_use = "a change that wasn't saved didn't happen"]
pub(crate) fn change(phone: &Phone, f: impl FnOnce(&mut Local)) -> std::io::Result<()> {
    let _g = phone.data_lock.lock().unwrap_or_else(|e| e.into_inner());
    let mut l = load(phone)?;
    f(&mut l);
    save(phone, &l).inspect_err(|e| tracing::warn!("phone mode: the data cache could not be saved: {e}"))
}

/// What the page is told when the phone couldn't keep what it asked for.
pub(crate) fn not_kept(e: std::io::Error) -> Response {
    super::err(
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        &format!("This phone couldn't keep that ({e}), so it isn't saved: try again"),
    )
}

/// The canonical URL of what the page names (a queue URI, a service link, a MusicBrainz link).
fn canonical(uri: &str) -> Result<pm_model::MusicUri, SourceError> {
    // A queue URI (`tidal:track:1`) is its service page
    let link = super::item::song_page(uri.trim()).unwrap_or_else(|| uri.trim().to_string());
    pm_model::MusicUri::parse(&link).map_err(|e| SourceError::bad_request(format!("{uri}: {e}")))
}

#[derive(Deserialize)]
pub(crate) struct Item {
    uri: Option<String>,
    track_id: Option<i64>,
}

fn uri_of(p: &Phone, it: &Item) -> Result<String, SourceError> {
    if let Some(id) = it.track_id {
        // A song added to the phone: its canonical URL, from its tags as on a node
        let s = p
            .songs
            .get(id)
            .ok_or_else(|| SourceError::not_found(format!("song {id}")))?;
        return s.canonical.ok_or_else(|| {
            SourceError::new(
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "“{}” has no MusicBrainz id or ISRC in its tags, so it can't be saved outside this phone",
                    s.title
                ),
            )
        });
    }
    it.uri
        .clone()
        .filter(|u| !u.trim().is_empty())
        .ok_or_else(|| SourceError::bad_request("give the song's uri"))
}

/// `GET /v1/favourites`: the Bookmarks (the node's shape; on a phone they are links, no library).
pub(crate) async fn bookmarks(State(p): State<Arc<Phone>>) -> Response {
    let l = match load(&p) {
        Ok(l) => l,
        Err(e) => return super::err(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let mut urls: Vec<(&String, &i64)> = l.bookmarks.iter().collect();
    urls.sort_by(|a, b| b.1.cmp(a.1));
    let urls: Vec<&String> = urls.into_iter().map(|(u, _)| u).collect();
    Json(json!({ "tracks": [], "albums": [], "artists": [], "posts": [], "other": urls, "uris": urls }))
        .into_response()
}

async fn bookmark(p: &Arc<Phone>, it: &Item, on: bool) -> Response {
    let uri = match uri_of(p, it).and_then(|u| canonical(&u)) {
        Ok(u) => u,
        Err(e) => return super::source_err(e),
    };
    let url = uri.to_url();
    let at = pm_pubky::ids::now_micros();
    let kept = change(p, |l| {
        if on {
            l.bookmarks.insert(url.clone(), at);
        } else {
            l.bookmarks.remove(&url);
        }
        l.pending.push(Pending::Bookmark {
            url: url.clone(),
            on,
            at,
        });
    });
    if let Err(e) = kept {
        return not_kept(e);
    }
    p.data_soon();
    Json(json!({ "uri": url, "favourite": on, "pending": true })).into_response()
}

/// `PUT /v1/favourites {uri}`.
pub(crate) async fn add(State(p): State<Arc<Phone>>, Json(it): Json<Item>) -> Response {
    bookmark(&p, &it, true).await
}

/// `DELETE /v1/favourites?uri=`.
pub(crate) async fn remove(State(p): State<Arc<Phone>>, Query(it): Query<Item>) -> Response {
    bookmark(&p, &it, false).await
}

#[derive(Deserialize)]
pub(crate) struct Played {
    uri: Option<String>,
    track_id: Option<i64>,
    duration_ms: Option<u64>,
}

/// `POST /v1/library/history {uri, duration_ms?}`: a song started on the phone, a listen (private,
/// as the node keeps them by default).
pub(crate) async fn played(State(p): State<Arc<Phone>>, Json(b): Json<Played>) -> Response {
    let it = Item {
        uri: b.uri,
        track_id: b.track_id,
    };
    let uri = match uri_of(&p, &it).and_then(|u| canonical(&u)) {
        Ok(u) => u,
        // A song of the phone's own with nothing to point at: no listen to keep
        Err(e) if it.track_id.is_some() && e.status == axum::http::StatusCode::UNPROCESSABLE_ENTITY => {
            return Json(json!({ "plays": [] })).into_response()
        }
        Err(e) => return super::source_err(e),
    };
    let service = match &uri {
        pm_model::MusicUri::Service(s) => Some(s.service.label().to_string()),
        _ => None,
    };
    let kept = change(&p, |l| {
        l.pending.push(Pending::Listen {
            url: uri.to_url(),
            service,
            duration_ms: b.duration_ms,
            at: pm_pubky::ids::now_micros(),
            id: None,
        })
    });
    if let Err(e) = kept {
        return not_kept(e);
    }
    p.data_soon();
    Json(json!({ "plays": [] })).into_response()
}

/// The person's z32 as the specs name an owner.
fn owner(acct: &Account) -> Result<v1::PubkyId, String> {
    let z32 = acct
        .pubky
        .strip_prefix("pubky")
        .filter(|k| k.len() == 52)
        .unwrap_or(&acct.pubky);
    v1::PubkyId::try_from(z32).map_err(|e| format!("{e:?}"))
}

/// Where the person keeps their listens (`listens.history`, set on a node and kept in its
/// settings copy, `/priv/pubky.music/settings.json`): the node's rule, private by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum History {
    Off,
    Private,
    Public,
}

/// The setting as the settings copy has it.
pub fn history_of(settings: Option<&Value>) -> History {
    let v = settings
        .and_then(|s| s.get("settings"))
        .and_then(|s| s.get("listens.history"))
        .and_then(Value::as_str);
    match v {
        Some("off") => History::Off,
        Some("public") => History::Public,
        _ => History::Private,
    }
}

async fn read_history(acct: &Account) -> Result<History, String> {
    let s = acct
        .hs
        .read_node_settings(&acct.session)
        .await
        .map_err(|e| format!("the listen history setting: {e}"))?;
    Ok(history_of(s.as_ref()))
}

/// Makes one pending write on the homeserver (`history`: where listens go).
async fn write(acct: &Account, p: &Phone, op: &Pending, history: History) -> Result<(), String> {
    match op {
        Pending::Bookmark { url, on: true, at } => {
            let (bm, path) = v1::bookmark_of(url, *at)?;
            acct.hs
                .write_object(&acct.session, &path, &bm)
                .await
                .map_err(|e| e.to_string())
        }
        Pending::Bookmark { url, on: false, .. } => {
            let path = v1::bookmark_path(url)?;
            match acct.hs.delete(&acct.session, &path).await {
                // Not there (never written, or removed elsewhere): done, as on a node
                Err(e) if e.is_not_found() => Ok(()),
                r => r.map_err(|e| e.to_string()),
            }
        }
        Pending::Listen {
            url,
            service,
            duration_ms,
            at,
            id,
        } => {
            let root = match history {
                // The person keeps no listen history
                History::Off => return Ok(()),
                History::Private => v1::Root::Priv,
                History::Public => v1::Root::Pub,
            };
            let uri = pm_model::MusicUri::parse(url).map_err(|e| e.to_string())?;
            let source = match service {
                Some(name) => pm_pubky::ActivitySource::Service { name: name.clone() },
                None => pm_pubky::ActivitySource::Unknown,
            };
            let mut a = pm_pubky::Activity::new(&uri, source, *duration_ms);
            a.played_at = *at;
            let post = v1::to_v1(&pm_pubky::MusicContent::Activity(a)).map_err(|e| format!("{e:?}"))?;
            let id = id.clone().unwrap_or_else(|| pm_pubky::ids::timestamp_id(*at));
            let path = post.version_path(&id, None, root, &owner(acct)?)?;
            acct.hs
                .write_object(&acct.session, &path, &post.to_json())
                .await
                .map_err(|e| e.to_string())
        }
        Pending::Quality { source } => write_prefs(acct, p, source).await,
    }
}

/// The streaming quality the phone uses for a source.
fn quality_of(p: &Phone, source: &str) -> Option<String> {
    let q = match source {
        "tidal" => pm_sources::tidal::full::load(p.store.as_ref()).quality,
        "qobuz" => super::qobuz::state(p).quality,
        _ => return None,
    };
    (!q.is_empty()).then_some(q)
}

/// prefs.json with the phone's quality for `source` in it (every other member as it is), newer
/// than what is there.
async fn write_prefs(acct: &Account, p: &Phone, source: &str) -> Result<(), String> {
    let mut prefs = acct
        .hs
        .read_prefs(&acct.session)
        .await
        .map_err(|e| e.to_string())?
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({ "version": 1 }));
    let now = (pm_pubky::ids::now_micros() / 1000) as u64;
    let at = (prefs["updated_at"].as_u64().unwrap_or(0) + 1).max(now);
    if !prefs["quality"].is_object() {
        prefs["quality"] = json!({});
    }
    // What another device set since (newer than what the phone took) applies here too: this
    // write moves prefs_at past it
    let took = load(p).map_err(|e| e.to_string())?.prefs_at;
    if prefs["updated_at"].as_u64().unwrap_or(0) > took {
        apply_quality(p, &prefs);
    }
    if let Some(q) = quality_of(p, source) {
        prefs["quality"][source] = Value::String(q);
    }
    prefs["updated_at"] = json!(at);
    acct.hs
        .write_prefs(&acct.session, &prefs)
        .await
        .map_err(|e| e.to_string())?;
    // Not kept: the next read takes this prefs.json as another device's (the same values)
    let _ = change(p, |l| l.prefs_at = at);
    Ok(())
}

/// Takes the person's prefs.json when it is newer than what the phone last took or wrote: the
/// streaming quality set on another device applies here too.
async fn take_prefs(acct: &Account, p: &Phone) -> Result<bool, String> {
    let Some(prefs) = acct
        .hs
        .read_prefs(&acct.session)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(false);
    };
    let at = prefs["updated_at"].as_u64().unwrap_or(0);
    if at <= load(p).map_err(|e| e.to_string())?.prefs_at {
        return Ok(false);
    }
    apply_quality(p, &prefs);
    // Not kept: taken again next time (the same values)
    let _ = change(p, |l| l.prefs_at = at);
    Ok(true)
}

/// The qualities in `prefs` applied here, except a source's changed here and not written yet.
fn apply_quality(p: &Phone, prefs: &Value) {
    // (Not known: as if waiting, nothing applied)
    let waiting = |source: &str| {
        load(p).map_or(true, |l| {
            l.pending
                .iter()
                .any(|op| matches!(op, Pending::Quality { source: s } if s == source))
        })
    };
    if let Some(q) = prefs["quality"]["tidal"]
        .as_str()
        .filter(|q| ["max", "high"].contains(q) && !waiting("tidal"))
    {
        pm_sources::tidal::full::update(p.store.as_ref(), |st| st.quality = q.into());
    }
    if let Some(q) = prefs["quality"]["qobuz"]
        .as_str()
        .filter(|q| ["max", "cd", "mp3"].contains(q) && !waiting("qobuz"))
    {
        super::qobuz::update(p, |st| st.quality = q.into());
    }
}

/// What the page is told when a quality set here couldn't be queued for the person's other
/// devices (it is set on the phone).
pub(crate) fn not_shared(e: std::io::Error) -> Response {
    super::err(
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        &format!(
            "The quality is set on this phone, but this phone couldn't keep it for your other devices ({e}): set it again to share it"
        ),
    )
}

/// A source's quality changed on the phone: prefs.json gets it at the next sync. An error when the
/// phone couldn't keep that ([`not_shared`] says it to the page).
pub(crate) fn prefs_changed(p: &Phone, source: &str) -> std::io::Result<()> {
    let op = Pending::Quality {
        source: source.to_string(),
    };
    change(p, |l| {
        if !l.pending.contains(&op) {
            l.pending.push(op.clone());
        }
    })?;
    p.data_soon();
    Ok(())
}

/// What a data sync did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub written: usize,
    pub left: usize,
    pub prefs_taken: bool,
}

/// The pending writes, in order (one that fails stops the rest until the next sync), then the
/// Bookmarks as the homeserver has them (with what is still pending on top), then prefs.json.
pub async fn sync(p: &Phone, acct: &Account) -> Report {
    let mut r = write_pending(p, acct).await;
    // Can't be read: nothing more (write_pending said why)
    let Ok(known) = load(p).map(|l| l.known) else {
        return r;
    };
    match acct.hs.read_v1_bookmarks_except(&acct.session, &known).await {
        Ok(list) => {
            // Not kept: read again next time
            let _ = change(p, |l| {
                // By file name, as the reader looks them up
                l.known = list
                    .iter()
                    .map(|(path, uri, at)| {
                        let file = path.rsplit('/').next().unwrap_or(path);
                        (file.to_string(), (uri.clone(), *at))
                    })
                    .collect();
                let mut b: BTreeMap<String, i64> = list.into_iter().map(|(_, uri, at)| (uri, at)).collect();
                for op in &l.pending {
                    if let Pending::Bookmark { url, on, at } = op {
                        if *on {
                            b.insert(url.clone(), *at);
                        } else {
                            b.remove(url);
                        }
                    }
                }
                l.bookmarks = b;
            });
        }
        Err(e) => tracing::warn!("phone mode: Bookmarks not read: {e}"),
    }
    match take_prefs(acct, p).await {
        Ok(t) => r.prefs_taken = t,
        Err(e) => tracing::warn!("phone mode: prefs.json not read: {e}"),
    }
    r.left = load(p).map_or(0, |l| l.pending.len());
    r
}

/// The waiting writes, in order (one that fails stops the rest until the next sync). When what
/// the phone keeps can't be read, none (logged).
pub async fn write_pending(p: &Phone, acct: &Account) -> Report {
    let mut r = Report::default();
    // Read once a pass, when a listen waits
    let mut history = None;
    loop {
        let next = match load(p) {
            Ok(l) => l.pending.first().cloned(),
            Err(e) => {
                tracing::warn!("phone mode: nothing is written to the homeserver: {e}");
                break;
            }
        };
        let Some(mut op) = next else { break };
        // A listen gets its id now, its write time, kept should this try fail
        if let Pending::Listen { id: None, .. } = &op {
            let new = pm_pubky::ids::timestamp_id(pm_pubky::ids::now_micros());
            // Never written without its id kept (a retry must land on the same post)
            if change(p, |l| {
                if let Some(Pending::Listen { id, .. }) = l.pending.first_mut() {
                    id.get_or_insert(new);
                }
            })
            .is_err()
            {
                break;
            }
            match load(p).ok().and_then(|l| l.pending.first().cloned()) {
                Some(o) => op = o,
                None => break,
            }
        }
        if matches!(op, Pending::Listen { .. }) && history.is_none() {
            match read_history(acct).await {
                Ok(h) => history = Some(h),
                Err(e) => {
                    tracing::warn!("phone mode: a write waits for the homeserver: {e}");
                    break;
                }
            }
        }
        match write(acct, p, &op, history.unwrap_or(History::Private)).await {
            Ok(()) => {
                // Not kept: written again next time (the same path, the same content)
                if change(p, |l| {
                    if l.pending.first() == Some(&op) {
                        l.pending.remove(0);
                    }
                })
                .is_err()
                {
                    break;
                }
                r.written += 1;
            }
            Err(e) => {
                tracing::warn!("phone mode: a write waits for the homeserver: {e}");
                break;
            }
        }
    }
    r.left = load(p).map_or(0, |l| l.pending.len());
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_change_on_the_page_wakes_only_the_writes() {
        let dir = tempfile::tempdir().unwrap();
        let p = Phone::open(super::super::Config {
            dir: dir.path().to_path_buf(),
            cache: dir.path().join("streams"),
            version: "test".into(),
            pubky: None,
            tidal: Default::default(),
        })
        .unwrap();
        let r = add(
            State(p.clone()),
            Json(Item {
                uri: Some("tidal:track:1".into()),
                track_id: None,
            }),
        )
        .await;
        assert_eq!(r.status(), axum::http::StatusCode::OK);
        let wait = std::time::Duration::from_millis(50);
        assert!(
            tokio::time::timeout(wait, p.data_now.notified()).await.is_ok(),
            "the write goes soon"
        );
        assert!(
            tokio::time::timeout(wait, p.sync_now.notified()).await.is_err(),
            "no vault pass for it"
        );
    }

    #[tokio::test]
    async fn what_the_phone_couldnt_keep_is_never_acknowledged() {
        let dir = tempfile::tempdir().unwrap();
        let p = Phone::open(super::super::Config {
            dir: dir.path().to_path_buf(),
            cache: dir.path().join("streams"),
            version: "test".into(),
            pubky: None,
            tidal: Default::default(),
        })
        .unwrap();
        // data.json can't be written now (a folder where it is written first)
        std::fs::create_dir(path(&p).with_extension("json.tmp")).unwrap();
        let item = || Item {
            uri: Some("tidal:track:1".into()),
            track_id: None,
        };
        let r = add(State(p.clone()), Json(item())).await;
        assert_eq!(r.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        let r = remove(State(p.clone()), Query(item())).await;
        assert_eq!(r.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        let r = played(
            State(p.clone()),
            Json(Played {
                uri: Some("tidal:track:1".into()),
                track_id: None,
                duration_ms: Some(1000),
            }),
        )
        .await;
        assert_eq!(r.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        assert!(prefs_changed(&p, "tidal").is_err());
        // Nothing waits to be written, nothing woke the writes, and the page's list is as it was
        assert!(load(&p).unwrap().pending.is_empty());
        let wait = std::time::Duration::from_millis(50);
        assert!(tokio::time::timeout(wait, p.data_now.notified()).await.is_err());
        assert_eq!(body(bookmarks(State(p.clone())).await).await["uris"], json!([]));
    }

    async fn body(r: Response) -> Value {
        serde_json::from_slice(&axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn what_the_phone_keeps_that_cant_be_read_stays_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let p = Phone::open(super::super::Config {
            dir: dir.path().to_path_buf(),
            cache: dir.path().join("streams"),
            version: "test".into(),
            pubky: None,
            tidal: Default::default(),
        })
        .unwrap();
        // A Bookmark removed here waits to be written, then data.json is cut short (a power cut)
        change(&p, |l| {
            l.pending.push(Pending::Bookmark {
                url: "https://tidal.com/browse/track/1".into(),
                on: false,
                at: 5,
            })
        })
        .unwrap();
        let whole = std::fs::read(path(&p)).unwrap();
        let cut = &whole[..whole.len() / 2];
        std::fs::write(path(&p), cut).unwrap();
        assert!(load(&p).is_err());
        // A new Bookmark isn't kept over it (that would lose the one waiting): an error, the file
        // as it was; the Bookmarks are an error too, not an empty list
        let r = add(
            State(p.clone()),
            Json(Item {
                uri: Some("tidal:track:2".into()),
                track_id: None,
            }),
        )
        .await;
        assert_eq!(r.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(std::fs::read(path(&p)).unwrap(), cut);
        let r = bookmarks(State(p.clone())).await;
        assert_eq!(r.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        // The sync neither writes nor reads the person's data meanwhile (nothing replaces it)
        let s = super::super::stalled::homeserver().await;
        let acct = s.account();
        let r = tokio::time::timeout(std::time::Duration::from_secs(10), sync(&p, &acct))
            .await
            .expect("the sync ends without the homeserver");
        assert_eq!(r, Report::default());
        assert_eq!(s.taken.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(std::fs::read(path(&p)).unwrap(), cut);
    }

    #[test]
    fn listens_go_where_the_person_keeps_them() {
        let with = |v: &str| json!({"version": 1, "settings": {"listens.history": v}});
        assert_eq!(history_of(Some(&with("off"))), History::Off);
        assert_eq!(history_of(Some(&with("public"))), History::Public);
        assert_eq!(history_of(Some(&with("private"))), History::Private);
        // No setting, or no settings copy: private, as on a node
        assert_eq!(history_of(Some(&json!({"settings": {}}))), History::Private);
        assert_eq!(history_of(None), History::Private);
    }

    #[test]
    fn songs_are_kept_by_their_canonical_link() {
        assert_eq!(
            canonical("tidal:track:12").unwrap().to_url(),
            canonical("https://tidal.com/browse/track/12").unwrap().to_url()
        );
        assert!(canonical("qobuz:track:5").is_ok());
        assert!(canonical("not a link").is_err());
    }

    #[test]
    fn pending_writes_keep_their_shape_on_disk() {
        let l = Local {
            bookmarks: [("https://tidal.com/browse/track/1".to_string(), 5)].into(),
            pending: vec![
                Pending::Bookmark {
                    url: "https://tidal.com/browse/track/1".into(),
                    on: true,
                    at: 5,
                },
                Pending::Listen {
                    url: "https://tidal.com/browse/track/1".into(),
                    service: Some("tidal".into()),
                    duration_ms: Some(1000),
                    at: 6,
                    id: None,
                },
                Pending::Quality {
                    source: "tidal".into(),
                },
            ],
            prefs_at: 3,
            known: Default::default(),
        };
        let back: Local = serde_json::from_slice(&serde_json::to_vec(&l).unwrap()).unwrap();
        assert_eq!(back.pending, l.pending);
        assert_eq!(back.bookmarks, l.bookmarks);
    }
}
