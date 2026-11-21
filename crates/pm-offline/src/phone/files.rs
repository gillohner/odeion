//! Songs added on the phone, also kept on the person's homeserver (`/priv/pubky.music/files/`,
//! private) under the names a node gives them, so a node finds them there: a setting, off by
//! default, on by default only when the person's homeserver is the one installed with their Odeion
//! node (its `node.json` names that homeserver: `bundled_homeserver`).

use super::account::Account;
use super::{err, Phone};
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use pm_sources::SessionStore;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

/// The person's choice on this phone (`on` / `off`; none: the default).
const KEY: &str = "phone.files_on_homeserver";
/// A song bigger than this is not copied from the phone (it would be held in memory).
const MAX_COPY: u64 = 300 * 1024 * 1024;

/// z32 without the `pubky` prefix.
fn z32(k: &str) -> &str {
    k.strip_prefix("pubky").filter(|x| x.len() == 52).unwrap_or(k)
}

/// The person's choice, if they made one.
pub fn chosen(p: &Phone) -> Option<bool> {
    match p.store.get(KEY).as_deref() {
        Some("on") => Some(true),
        Some("off") => Some(false),
        _ => None,
    }
}

/// Whether songs added here go to the homeserver: the person's choice, else on only when their
/// homeserver is their node's own.
pub fn on(p: &Phone) -> bool {
    chosen(p).unwrap_or_else(|| *p.bundled_here.lock().unwrap_or_else(|e| e.into_inner()) == Some(true))
}

/// Whether the person's homeserver is the one installed with their node: their node record's
/// `bundled_homeserver` against the homeserver their Pubky names. `None`: no node record, or not
/// known now.
pub async fn bundled_here(acct: &Account) -> Option<bool> {
    let record = acct.hs.read_node_record(&acct.session).await.ok()??;
    let Some(bundled) = record.bundled_homeserver else {
        return Some(false);
    };
    let theirs = acct
        .hs
        .pubky()
        .get_homeserver_of(&acct.session.public_key())
        .await
        .ok()??;
    Some(z32(&theirs.z32()) == z32(&bundled))
}

/// What a sync copied.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub copied: usize,
    pub left: usize,
}

/// Copies the songs added here that the homeserver doesn't have yet (by file name, which carries
/// the content hash), one at a time, when the setting is on.
pub async fn sync(p: &Phone, acct: &Account) -> Report {
    let known = bundled_here(acct).await;
    if known.is_some() {
        *p.bundled_here.lock().unwrap_or_else(|e| e.into_inner()) = known;
    }
    let mut r = Report::default();
    if !on(p) {
        return r;
    }
    let there: std::collections::HashSet<String> = match acct.hs.list_private_files(&acct.session).await {
        Ok(list) => list.into_iter().map(|f| f.name().to_string()).collect(),
        Err(e) => {
            tracing::warn!("phone mode: the homeserver's files can't be listed now: {e}");
            return r;
        }
    };
    for s in p.songs.list() {
        if there.contains(&s.file) {
            continue;
        }
        if s.bytes > MAX_COPY {
            r.left += 1;
            continue;
        }
        let bytes = match tokio::fs::read(p.songs.path_of(&s)).await {
            Ok(b) => b,
            Err(_) => continue,
        };
        let path = format!("{}{}", pm_appdata::files::FILES_DIR, s.file);
        match acct.hs.upload_private(&acct.session, &path, bytes).await {
            Ok(()) => r.copied += 1,
            Err(e) => {
                tracing::warn!("phone mode: a song waits to be copied to the homeserver: {e}");
                r.left += 1;
                break;
            }
        }
    }
    r
}

/// `GET /v1/phone/storage`: whether added songs go to the homeserver, why, and how far.
pub(crate) async fn status(State(p): State<Arc<Phone>>) -> Json<Value> {
    Json(view(&p).await)
}

async fn view(p: &Phone) -> Value {
    let songs = p.songs.list();
    let there: Option<std::collections::HashSet<String>> = match p.account().await {
        Some(a) => {
            a.hs.list_private_files(&a.session)
                .await
                .ok()
                .map(|l| l.into_iter().map(|f| f.name().to_string()).collect())
        }
        None => None,
    };
    let uploaded = there
        .map(|t| songs.iter().filter(|s| t.contains(&s.file)).count())
        .unwrap_or(0);
    json!({
        "on": on(p),
        "chosen": chosen(p),
        "bundled": *p.bundled_here.lock().unwrap_or_else(|e| e.into_inner()),
        "uploaded": uploaded,
        "total": songs.len(),
    })
}

#[derive(Deserialize)]
pub(crate) struct Choice {
    /// `null`: back to the default.
    on: Option<bool>,
}

/// `PUT /v1/phone/storage {on}`.
pub(crate) async fn choose(State(p): State<Arc<Phone>>, Json(c): Json<Choice>) -> Response {
    let v = c.on.map(|on| if on { "on" } else { "off" });
    if let Err(e) = p.store.put(KEY, v) {
        return err(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    p.sync_soon();
    Json(view(&p).await).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_compare_without_the_pubky_prefix() {
        let k = "8pinxxgqs41n4aididenw5apqp1urfmzdztr8jt4abrkdn435ewo";
        assert_eq!(z32(&format!("pubky{k}")), k);
        assert_eq!(z32(k), k);
    }
}
