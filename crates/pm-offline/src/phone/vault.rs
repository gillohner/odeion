//! Phone mode as a client of the person's sources vault (`crates/pm-appdata/VAULT.md`, ADR 0017),
//! through `pm_pubky::vault::Vault`: the phone lists itself in the device list with its client and
//! signer proofs (`touch`), shares sign-ins with devices that joined, and keeps TIDAL full tracks
//! and Qobuz in step with the vault:
//!
//! - a sign-in a node (or another phone) wrote is taken when it is a node's copy (`node.settings`
//!   whose `std` matches: `vault_form::node_settings`), signed by one of the person's devices and
//!   not older than one seen (the vault client's own checks);
//! - a sign-in made or renewed here is written in the node's own form (`vault_form::sign_in_for`),
//!   so a node the person adds later takes it as its own;
//! - one signed out here is removed from the vault, one removed from the vault goes here too.
//!
//! TIDAL is renewed under the blob's lock (`Vault::refresh`), so the phone and the person's other
//! devices never renew the same sign-in at once. What the phone saw at the last sync is kept in
//! `phone/vault/synced.json` (hashes only), beside the vault key and the vault client's records.

use super::account::Account;
use super::Phone;
use pm_appdata::vault::SignIn;
use pm_pubky::vault::{RefreshFailed, Refreshed, Vault, VaultStoreError};
use pm_sources::vault_form::{self, Form, QOBUZ, TIDAL_FULL};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The sign-ins phone mode carries.
pub const FORMS: [&Form; 2] = [&TIDAL_FULL, &QOBUZ];
/// The phone's name in the device list (Settings › Accounts on a node shows it).
pub const DEVICE_NAME: &str = "Odeion on this phone";

fn local(e: impl std::fmt::Display) -> VaultStoreError {
    VaultStoreError::Local(std::io::Error::other(e.to_string()))
}

fn dir(phone: &Phone) -> PathBuf {
    phone.dir().join("vault")
}

/// What both sides held at the last sync, per service.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Synced {
    /// `vault_form::hash` of what travels.
    pub hash: String,
    /// The sign-in's id (a refresh keeps it; a new sign-in changes it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage: Option<String>,
}

fn record_path(phone: &Phone) -> PathBuf {
    dir(phone).join("synced.json")
}

fn load_record(path: &Path) -> BTreeMap<String, Synced> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_record(path: &Path, r: &BTreeMap<String, Synced>) {
    let tmp = path.with_extension("json.tmp");
    let ok = serde_json::to_vec(r)
        .ok()
        .and_then(|b| std::fs::write(&tmp, b).ok())
        .and_then(|_| std::fs::rename(&tmp, path).ok());
    if ok.is_none() {
        tracing::warn!("phone mode: the vault record could not be saved");
    }
}

/// The vault's copy of a service, as the sync rules see it.
pub struct Theirs<'a> {
    pub hash: String,
    /// A node's copy: its settings travel to the phone as they are.
    pub usable: bool,
    pub sign_in: &'a SignIn,
}

/// What to do for one service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Do {
    Nothing,
    /// The same on both sides.
    InStep,
    /// Write the phone's copy.
    Push,
    /// Take the vault's copy.
    Pull,
    /// Signed out here: remove it from the vault.
    Remove,
    /// Removed from the vault elsewhere: sign out here too.
    DropHere,
    /// Neither side has it any more: forget the record.
    Forget,
}

/// The sync rules for one service. `seen`: what both held at the last sync; `mine`: the hash of
/// what travels here; `theirs`: the vault's copy. A change on one side since the last sync goes to
/// the other; both changed, the vault's copy wins (another device's newer sign-in); a copy that
/// isn't a node's is never taken, only written over by a sign-in here.
pub fn decide(seen: Option<&Synced>, mine: Option<&str>, theirs: Option<&Theirs<'_>>) -> Do {
    let pull_or = |t: &Theirs<'_>, otherwise: Do| if t.usable { Do::Pull } else { otherwise };
    match (mine, theirs) {
        (None, None) => {
            if seen.is_some() {
                Do::Forget
            } else {
                Do::Nothing
            }
        }
        (Some(m), None) => match seen {
            Some(s) if s.hash == m => Do::DropHere,
            _ => Do::Push,
        },
        (None, Some(t)) => match seen {
            Some(s) if s.hash == t.hash => Do::Remove,
            _ => pull_or(t, Do::Nothing),
        },
        (Some(m), Some(t)) if m == t.hash => Do::InStep,
        (Some(m), Some(t)) => match seen {
            None => pull_or(t, Do::Push),
            Some(s) if s.hash == m => pull_or(t, Do::Nothing),
            Some(s) if s.hash == t.hash => Do::Push,
            Some(_) => pull_or(t, Do::Push),
        },
    }
}

/// The hash of a vault copy, comparable with the phone's own (`vault_form::hash` of what travels)
/// when it is a node's copy.
fn hash_of(s: &SignIn) -> String {
    match vault_form::node_settings(s) {
        Some(m) => vault_form::hash(&m),
        None => format!("std:{}", vault_form::std_hash(s)),
    }
}

/// The app the phone's copy of a service is made with: TIDAL's device client, or `pkce:` and the
/// browser client for Sign in with TIDAL (as a node names them); Qobuz's web-player app.
fn app_for(phone: &Phone, form: &Form, travels: &Map<String, Value>) -> Option<String> {
    if form.service == TIDAL_FULL.service {
        let sess = vault_form::session_of(form.carry, travels)?;
        return match sess.get("client").and_then(Value::as_str) {
            Some("pkce") => phone.tidal.browser.as_ref().map(|c| format!("pkce:{}", c.id)),
            _ => phone.tidal.device.as_ref().map(|c| c.id.clone()),
        };
    }
    pm_sources::qobuz::app_of(phone.store.as_ref()).map(|a| a.id)
}

/// A sign-in taken from the vault is switched on here (the vault carries the session, not the
/// switch), and Qobuz gets app values for it.
async fn after_pull(phone: &Phone, form: &Form) {
    if form.service == TIDAL_FULL.service {
        pm_sources::tidal::full::update_if(phone.store.as_ref(), |st| {
            let on = st.session.is_some() && !st.enabled;
            if on {
                st.enabled = true;
                st.health = None;
            }
            on
        });
        return;
    }
    let st = super::qobuz::state(phone);
    if let Some(sess) = st.session {
        if !st.enabled {
            super::qobuz::update(phone, |st| st.enabled = true);
        }
        if let Err(e) = pm_sources::qobuz::app_for_session(phone.store.as_ref(), &sess.token).await {
            tracing::warn!("phone mode: Qobuz from the vault has no app values yet: {e}");
        }
    }
}

/// What one sync did, for the log.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub pulled: Vec<&'static str>,
    pub pushed: Vec<&'static str>,
    pub removed: Vec<&'static str>,
    pub dropped: Vec<&'static str>,
    pub skipped: Vec<String>,
}

/// One sync with the vault: list this phone, share, then each service by [`decide`].
pub async fn pass(phone: &Phone, acct: &Account) -> Result<Report, VaultStoreError> {
    let d = dir(phone);
    std::fs::create_dir_all(&d).map_err(local)?;
    let secret = pm_pubky::vault::load_or_create_client_secret(&d.join("client.key")).map_err(local)?;
    let (revoked, marks) = (d.join("revoked.json"), d.join("marks.json"));
    let vault = Vault::new(&acct.session, &secret)
        .device("android")
        .remember_revocations(&revoked)
        .keep_marks(&marks);
    vault.touch(DEVICE_NAME).await?;
    vault.share_with_trusted().await?;
    let path = record_path(phone);
    let mut record = load_record(&path);
    let mut report = Report::default();
    for form in FORMS {
        let svc = form.service;
        let mine = vault_form::carried(form.carry, phone.store.as_ref()).map_err(local)?;
        let opened = match vault.read(svc).await {
            Ok(o) => o,
            // Not (yet) shared with this phone, not one of the person's devices', or older than one
            // seen: what the phone has stays as it is
            Err(e) => {
                report.skipped.push(format!("{svc}: {e}"));
                continue;
            }
        };
        let theirs = opened.as_ref().map(|o| Theirs {
            hash: hash_of(&o.sign_in),
            usable: vault_form::node_settings(&o.sign_in).is_some(),
            sign_in: &o.sign_in,
        });
        let mine_hash = mine.as_ref().map(vault_form::hash);
        match decide(record.get(svc), mine_hash.as_deref(), theirs.as_ref()) {
            Do::Nothing => {}
            Do::InStep => {
                let t = theirs.as_ref().expect("in step with a copy");
                record.insert(
                    svc.into(),
                    Synced {
                        hash: t.hash.clone(),
                        lineage: vault_form::node_lineage(t.sign_in),
                    },
                );
            }
            Do::Forget => {
                record.remove(svc);
            }
            Do::Pull => {
                let t = theirs.as_ref().expect("a copy to take");
                let travels = vault_form::node_settings(t.sign_in).expect("a node's copy");
                vault_form::apply(form.carry, &travels, phone.store.as_ref()).map_err(local)?;
                after_pull(phone, form).await;
                record.insert(
                    svc.into(),
                    Synced {
                        hash: vault_form::hash(&travels),
                        lineage: vault_form::node_lineage(t.sign_in),
                    },
                );
                report.pulled.push(svc);
            }
            Do::Push => {
                let travels = mine.as_ref().expect("a copy to write");
                let base = theirs.as_ref().map(|t| t.sign_in);
                // A refresh (the same account) keeps the sign-in's id; another account is a new one
                let same = base.is_some_and(|b| {
                    vault_form::node_settings(b).and_then(|m| vault_form::account(&m))
                        == vault_form::account(travels)
                });
                let lineage = base
                    .filter(|_| same)
                    .and_then(vault_form::node_lineage)
                    .or_else(|| record.get(svc).and_then(|s| s.lineage.clone()))
                    .unwrap_or_else(|| vault_form::lineage_after(svc, None, pm_sources::now_unix() as u64));
                let s = vault_form::sign_in_for(
                    svc,
                    form.carry,
                    true,
                    travels,
                    base,
                    app_for(phone, form, travels),
                    &lineage,
                );
                let expect = theirs.as_ref().map(|t| t.hash.clone());
                let written = vault
                    .write_shared_if(svc, &s, None, &|cur| cur.map(hash_of) == expect)
                    .await?;
                if written.is_some() {
                    record.insert(
                        svc.into(),
                        Synced {
                            hash: vault_form::hash(travels),
                            lineage: Some(lineage),
                        },
                    );
                    report.pushed.push(svc);
                } else {
                    report.skipped.push(format!("{svc}: changed meanwhile"));
                }
            }
            Do::Remove => {
                let expect = theirs.as_ref().map(|t| t.hash.clone());
                if vault.remove_if(svc, &|cur| cur.map(hash_of) == expect).await? {
                    record.remove(svc);
                    report.removed.push(svc);
                }
            }
            Do::DropHere => {
                vault_form::apply(form.carry, &Map::new(), phone.store.as_ref()).map_err(local)?;
                record.remove(svc);
                report.dropped.push(svc);
            }
        }
    }
    save_record(&path, &record);
    Ok(report)
}

/// TIDAL renewed under the vault blob's lock (VAULT.md, "Refreshing a token"): the session read
/// under the lock is renewed with the client that made it and written back; one another device
/// renewed meanwhile is used as it is. The result is the phone's session too. `Ok(false)` when the
/// vault has no node's copy of TIDAL in step with the phone's (the phone then renews on its own
/// and the next sync writes it).
pub async fn renew_tidal(phone: &std::sync::Arc<Phone>, acct: &Account) -> Result<bool, VaultStoreError> {
    let svc = TIDAL_FULL.service;
    let path = record_path(phone);
    let record = load_record(&path);
    let Some(mine) = vault_form::carried(TIDAL_FULL.carry, phone.store.as_ref()).map_err(local)? else {
        return Ok(false);
    };
    if record.get(svc).map(|s| s.hash.as_str()) != Some(vault_form::hash(&mine).as_str()) {
        return Ok(false);
    }
    let d = dir(phone);
    let secret = pm_pubky::vault::load_or_create_client_secret(&d.join("client.key")).map_err(local)?;
    let (revoked, marks) = (d.join("revoked.json"), d.join("marks.json"));
    let vault = Vault::new(&acct.session, &secret)
        .device("android")
        .remember_revocations(&revoked)
        .keep_marks(&marks);
    let p = phone.clone();
    let r = vault
        .refresh(svc, false, |si: SignIn| async move {
            let travels = vault_form::node_settings(&si).ok_or_else(|| {
                RefreshFailed("the vault's TIDAL sign-in isn't one this phone can renew".into())
            })?;
            let sess: pm_sources::tidal::full::Session = vault_form::session_of(TIDAL_FULL.carry, &travels)
                .and_then(|m| serde_json::from_value(Value::Object(m)).ok())
                .ok_or_else(|| RefreshFailed("the vault's TIDAL sign-in has no session".into()))?;
            let p2 = p.clone();
            let fresh = tokio::task::spawn_blocking(move || {
                pm_sources::tidal::full::renew(
                    super::stream::blocking_client(),
                    &super::tidal::endpoints(&p2),
                    &sess,
                )
            })
            .await
            .map_err(|_| RefreshFailed("the renewal stopped".into()))?
            .map_err(|e| {
                RefreshFailed(match e {
                    pm_sources::tidal::full::FullError::Mode(m)
                    | pm_sources::tidal::full::FullError::Track(m) => m,
                })
            })?;
            let mut travels = travels;
            let key = TIDAL_FULL.carry[0].key;
            let mut held = travels
                .get(key)
                .and_then(vault_form::object_of)
                .unwrap_or_default();
            held.insert(
                "session".into(),
                serde_json::to_value(&fresh).expect("a session serializes"),
            );
            travels.insert(key.into(), Value::Object(held));
            let lineage = vault_form::node_lineage(&si)
                .unwrap_or_else(|| vault_form::lineage_after(svc, None, pm_sources::now_unix() as u64));
            let app = si.client_id.clone();
            Ok(vault_form::sign_in_for(
                svc,
                TIDAL_FULL.carry,
                true,
                &travels,
                Some(&si),
                app,
                &lineage,
            ))
        })
        .await?;
    let si = r.sign_in();
    let Some(travels) = vault_form::node_settings(si) else {
        return Ok(false);
    };
    vault_form::apply(TIDAL_FULL.carry, &travels, phone.store.as_ref()).map_err(local)?;
    let mut record = load_record(&path);
    record.insert(
        svc.into(),
        Synced {
            hash: vault_form::hash(&travels),
            lineage: vault_form::node_lineage(si),
        },
    );
    save_record(&path, &record);
    if matches!(r, Refreshed::ByUs(_)) {
        tracing::info!("phone mode: TIDAL renewed under the vault's lock");
    }
    Ok(true)
}

/// Whether the phone's TIDAL session runs out within a minute.
pub fn tidal_due(phone: &Phone) -> bool {
    pm_sources::tidal::full::load(phone.store.as_ref())
        .session
        .is_some_and(|s| s.expires_at - pm_sources::now_unix() <= 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(h: &str) -> Synced {
        Synced {
            hash: h.into(),
            lineage: None,
        }
    }

    #[test]
    fn the_sync_rules() {
        let si: SignIn = serde_json::from_value(serde_json::json!({ "service": "tidal-full" })).unwrap();
        let t = |h: &str, usable| Theirs {
            hash: h.into(),
            usable,
            sign_in: &si,
        };
        // Nothing anywhere
        assert_eq!(decide(None, None, None), Do::Nothing);
        assert_eq!(decide(Some(&seen("a")), None, None), Do::Forget);
        // A sign-in made here goes out; one gone from the vault since the last sync goes here too
        assert_eq!(decide(None, Some("a"), None), Do::Push);
        assert_eq!(decide(Some(&seen("a")), Some("a"), None), Do::DropHere);
        assert_eq!(
            decide(Some(&seen("a")), Some("b"), None),
            Do::Push,
            "a new sign-in here after the removal"
        );
        // A node's copy is taken; another client's never is
        assert_eq!(decide(None, None, Some(&t("a", true))), Do::Pull);
        assert_eq!(decide(None, None, Some(&t("a", false))), Do::Nothing);
        // Signed out here since the last sync: out of the vault too
        assert_eq!(decide(Some(&seen("a")), None, Some(&t("a", true))), Do::Remove);
        assert_eq!(
            decide(Some(&seen("a")), None, Some(&t("b", true))),
            Do::Pull,
            "a newer sign-in elsewhere"
        );
        // Both the same
        assert_eq!(decide(None, Some("a"), Some(&t("a", true))), Do::InStep);
        // One side changed
        assert_eq!(decide(Some(&seen("a")), Some("a"), Some(&t("b", true))), Do::Pull);
        assert_eq!(decide(Some(&seen("a")), Some("b"), Some(&t("a", true))), Do::Push);
        assert_eq!(
            decide(Some(&seen("a")), Some("a"), Some(&t("b", false))),
            Do::Nothing
        );
        // Both changed: the vault's node copy wins; another client's is written over
        assert_eq!(decide(Some(&seen("a")), Some("b"), Some(&t("c", true))), Do::Pull);
        assert_eq!(
            decide(Some(&seen("a")), Some("b"), Some(&t("c", false))),
            Do::Push
        );
        // First contact with two different copies
        assert_eq!(decide(None, Some("b"), Some(&t("c", true))), Do::Pull);
        assert_eq!(decide(None, Some("b"), Some(&t("c", false))), Do::Push);
    }
}
