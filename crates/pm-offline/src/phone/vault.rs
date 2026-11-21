//! Phone mode as a client of the person's sources vault (`crates/pm-appdata/VAULT.md`, ADR 0017),
//! through `pm_pubky::vault::Vault`: the phone lists itself in the device list with its client and
//! signer proofs (`touch`), shares sign-ins with devices that joined, and keeps TIDAL full tracks
//! and Qobuz in step with the vault, by the node's rules:
//!
//! - a sign-in another device wrote is taken when it is a node's copy (`node.settings` whose `std`
//!   matches: `vault_form::node_settings`) made with an app this phone renews with, signed by one
//!   of the person's devices and not older than one seen (the vault client's own checks);
//! - a sign-in made or renewed here is written in the node's own form (`vault_form::sign_in_for`),
//!   with the id it got when it was made here, so a node the person adds later takes it as its
//!   own;
//! - one signed out here is removed from the vault, one removed from the vault goes here too;
//! - after "Sign out everywhere" (the device list's marker), a sign-in made here before it never
//!   goes back in.
//!
//! TIDAL is renewed under the blob's lock (`Vault::refresh`), so the phone and the person's other
//! devices never renew the same sign-in at once. What the phone saw at the last sync is kept in
//! `phone/<pubky>/vault/synced.json` (hashes only), beside the vault key and the vault client's
//! records.

use super::account::Account;
use super::Phone;
use pm_appdata::vault::SignIn;
use pm_pubky::vault::{RefreshFailed, Refreshed, Vault, VaultStoreError};
use pm_sources::vault_form::{self, Form, QOBUZ, TIDAL_FULL};
use pm_sources::SessionStore;
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

/// Where the id of the sign-in the phone holds for a service is kept (with its sign-ins).
fn lineage_key(service: &str) -> String {
    format!("vault.lineage.{service}")
}

/// The id of the sign-in the phone holds for `service` (made here, or taken with its copy).
fn lineage_here(phone: &Phone, service: &str) -> Option<String> {
    phone.store.get(&lineage_key(service))
}

fn set_lineage(phone: &Phone, service: &str, lineage: Option<&str>) {
    let _ = phone.store.put(&lineage_key(service), lineage);
}

/// A sign-in was just made on the phone: it gets a new id (a renewal keeps it), dated now, so the
/// vault can tell it from a refresh of the one before, and from one made before a sign-out
/// everywhere.
pub fn signed_in(phone: &Phone, service: &str) {
    let id = vault_form::lineage_after(service, None, pm_sources::now_unix() as u64);
    set_lineage(phone, service, Some(&id));
    phone.sync_soon();
}

/// What both sides held at the last sync, per service.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Synced {
    /// `vault_form::hash` of what travels, as the phone keeps it.
    pub mine: String,
    /// The hash of the vault's copy (`hash_of`).
    pub theirs: String,
}

/// The phone's record of the last sync.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    #[serde(default)]
    pub services: BTreeMap<String, Synced>,
    /// The device list's "signed out everywhere" marker at the last sync.
    #[serde(default)]
    pub marker: Option<u64>,
}

fn record_path(phone: &Phone) -> PathBuf {
    dir(phone).join("synced.json")
}

fn load_record(path: &Path) -> Record {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_record(path: &Path, r: &Record) {
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
    /// A node's copy made with an app this phone renews with: its settings travel to the phone.
    pub usable: bool,
    pub sign_in: &'a SignIn,
}

/// What to do for one service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Do {
    Nothing,
    /// The same on both sides as at the last sync.
    InStep,
    /// Write the phone's copy.
    Push,
    /// Take the vault's copy.
    Pull,
    /// Signed out here: remove it from the vault.
    Remove,
    /// Removed from the vault elsewhere (or signed out everywhere since it was made): sign out
    /// here too.
    DropHere,
    /// Neither side has it any more: forget the record.
    Forget,
}

/// The sync rules for one service. `seen`: what both held at the last sync; `mine`: the hash of
/// what travels here; `theirs`: the vault's copy; `predates`: the phone's copy was made before the
/// last sign-out everywhere and isn't the one synced since. A change on one side since the last
/// sync goes to the other; both changed, the vault's usable copy wins (another device's newer
/// sign-in); a copy the phone can't use is never taken, only written over by a sign-in here.
pub fn decide(seen: Option<&Synced>, mine: Option<&str>, theirs: Option<&Theirs<'_>>, predates: bool) -> Do {
    let pull_or = |t: &Theirs<'_>, otherwise: Do| if t.usable { Do::Pull } else { otherwise };
    if mine.is_some() && predates {
        // Never back into the vault: the vault's own copy, or out
        return match theirs {
            Some(t) if t.usable => Do::Pull,
            _ => Do::DropHere,
        };
    }
    match (mine, theirs) {
        (None, None) => {
            if seen.is_some() {
                Do::Forget
            } else {
                Do::Nothing
            }
        }
        (Some(m), None) => match seen {
            Some(s) if s.mine == m => Do::DropHere,
            _ => Do::Push,
        },
        (None, Some(t)) => match seen {
            Some(s) if s.theirs == t.hash => Do::Remove,
            _ => pull_or(t, Do::Nothing),
        },
        (Some(m), Some(t)) => match seen {
            None => pull_or(t, Do::Push),
            Some(s) => match (m != s.mine, t.hash != s.theirs) {
                (false, false) => Do::InStep,
                (true, false) => Do::Push,
                (false, true) => pull_or(t, Do::Nothing),
                (true, true) => pull_or(t, Do::Push),
            },
        },
    }
}

/// The hash of a vault copy: of what travels when it is a node's copy (as the phone hashes its
/// own), else of its standard members.
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

/// Whether the phone can use a vault copy: a node's copy (the node takes only those), made with the
/// app the phone renews this kind of sign-in with (Qobuz: any app; the phone finds its own).
fn usable(phone: &Phone, form: &Form, s: &SignIn) -> bool {
    let Some(travels) = vault_form::node_settings(s) else {
        return false;
    };
    if form.service != TIDAL_FULL.service {
        return true;
    }
    app_for(phone, form, &travels).is_some_and(|app| s.client_id.as_deref() == Some(app.as_str()))
}

/// A sign-in taken from the vault is switched on here (the vault carries the session, not the
/// switch).
fn after_pull(phone: &Phone, form: &Form) {
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
    if st.session.is_some() && !st.enabled {
        super::qobuz::update(phone, |st| st.enabled = true);
    }
}

/// Qobuz needs app values for a session it took (found once; tried again at every sync until it
/// works).
async fn qobuz_app(phone: &Phone) {
    let st = super::qobuz::state(phone);
    let Some(sess) = st.session else { return };
    if pm_sources::qobuz::app_of(phone.store.as_ref()).is_some() {
        return;
    }
    if let Err(e) = pm_sources::qobuz::app_for_session(phone.store.as_ref(), &sess.token).await {
        tracing::warn!("phone mode: Qobuz has no app values yet: {e}");
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

fn vault_of<'a>(
    acct: &'a Account,
    secret: &'a pm_appdata::vault::ClientSecret,
    d: &'a (PathBuf, PathBuf),
) -> Vault<'a> {
    Vault::new(&acct.session, secret)
        .device("android")
        .remember_revocations(&d.0)
        .keep_marks(&d.1)
}

/// One sync with the vault: list this phone, share, then each service by [`decide`].
pub async fn pass(phone: &Phone, acct: &Account) -> Result<Report, VaultStoreError> {
    let d = dir(phone);
    std::fs::create_dir_all(&d).map_err(local)?;
    let secret = pm_pubky::vault::load_or_create_client_secret(&d.join("client.key")).map_err(local)?;
    let files = (d.join("revoked.json"), d.join("marks.json"));
    let vault = vault_of(acct, &secret, &files);
    vault.touch(DEVICE_NAME).await?;
    vault.share_with_trusted().await?;
    let marker = vault.marker(&vault.index().await?)?;
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
            usable: usable(phone, form, &o.sign_in),
            sign_in: &o.sign_in,
        });
        let mine_hash = mine.as_ref().map(vault_form::hash);
        let here = lineage_here(phone, svc);
        let seen = record.services.get(svc);
        // Made before the last sign-out everywhere, and not the copy synced since it
        let synced_since =
            seen.is_some_and(|s| Some(&s.mine) == mine_hash.as_ref()) && record.marker == marker;
        let predates = marker.is_some_and(|m| here.as_deref().map(vault_form::lineage_at).unwrap_or(0) <= m)
            && !synced_since;
        match decide(seen, mine_hash.as_deref(), theirs.as_ref(), predates) {
            Do::Nothing => {}
            Do::InStep => {
                let t = theirs.as_ref().expect("in step with a copy");
                record.services.insert(
                    svc.into(),
                    Synced {
                        mine: mine_hash.clone().unwrap_or_default(),
                        theirs: t.hash.clone(),
                    },
                );
            }
            Do::Forget => {
                record.services.remove(svc);
            }
            Do::Pull => {
                let t = theirs.as_ref().expect("a copy to take");
                let travels = vault_form::node_settings(t.sign_in).expect("a node's copy");
                vault_form::apply(form.carry, &travels, phone.store.as_ref()).map_err(local)?;
                after_pull(phone, form);
                set_lineage(phone, svc, vault_form::node_lineage(t.sign_in).as_deref());
                // As the phone keeps it now (written back, it may read differently from the copy)
                let kept = vault_form::carried(form.carry, phone.store.as_ref())
                    .map_err(local)?
                    .map(|m| vault_form::hash(&m))
                    .unwrap_or_default();
                record.services.insert(
                    svc.into(),
                    Synced {
                        mine: kept,
                        theirs: t.hash.clone(),
                    },
                );
                report.pulled.push(svc);
            }
            Do::Push => {
                let travels = mine.as_ref().expect("a copy to write");
                let base = theirs.as_ref().map(|t| t.sign_in);
                // The id the sign-in got here; one from before ids: the vault copy's when it is the
                // same account (a refresh), else a new one
                let lineage = here.clone().unwrap_or_else(|| {
                    let same = base.is_some_and(|b| {
                        vault_form::node_settings(b).and_then(|m| vault_form::account(&m))
                            == vault_form::account(travels)
                    });
                    base.filter(|_| same)
                        .and_then(vault_form::node_lineage)
                        .unwrap_or_else(|| {
                            vault_form::lineage_after(svc, None, pm_sources::now_unix() as u64)
                        })
                });
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
                    .write_shared_if(svc, &s, Some(marker), &|cur| cur.map(hash_of) == expect)
                    .await?;
                if written.is_some() {
                    set_lineage(phone, svc, Some(&lineage));
                    record.services.insert(
                        svc.into(),
                        Synced {
                            mine: vault_form::hash(travels),
                            theirs: hash_of(&s),
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
                    record.services.remove(svc);
                    report.removed.push(svc);
                }
            }
            Do::DropHere => {
                vault_form::apply(form.carry, &Map::new(), phone.store.as_ref()).map_err(local)?;
                set_lineage(phone, svc, None);
                record.services.remove(svc);
                report.dropped.push(svc);
            }
        }
    }
    record.marker = marker;
    save_record(&path, &record);
    qobuz_app(phone).await;
    Ok(report)
}

/// TIDAL renewed under the vault blob's lock (VAULT.md, "Refreshing a token"): the session read
/// under the lock is renewed with the client that made it and written back; one another device
/// renewed meanwhile is used as it is. The result is the phone's session too. `Ok(false)` when the
/// vault has no copy of TIDAL in step with the phone's (the phone then renews on its own and the
/// next sync writes it).
pub async fn renew_tidal(phone: &std::sync::Arc<Phone>, acct: &Account) -> Result<bool, VaultStoreError> {
    let svc = TIDAL_FULL.service;
    let path = record_path(phone);
    let Some(mine) = vault_form::carried(TIDAL_FULL.carry, phone.store.as_ref()).map_err(local)? else {
        return Ok(false);
    };
    if load_record(&path).services.get(svc).map(|s| s.mine.as_str()) != Some(vault_form::hash(&mine).as_str())
    {
        return Ok(false);
    }
    let d = dir(phone);
    let secret = pm_pubky::vault::load_or_create_client_secret(&d.join("client.key")).map_err(local)?;
    let files = (d.join("revoked.json"), d.join("marks.json"));
    let vault = vault_of(acct, &secret, &files);
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
    let kept = vault_form::carried(TIDAL_FULL.carry, phone.store.as_ref())
        .map_err(local)?
        .map(|m| vault_form::hash(&m))
        .unwrap_or_default();
    let mut record = load_record(&path);
    record.services.insert(
        svc.into(),
        Synced {
            mine: kept,
            theirs: hash_of(si),
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

    fn seen(mine: &str, theirs: &str) -> Synced {
        Synced {
            mine: mine.into(),
            theirs: theirs.into(),
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
        assert_eq!(decide(None, None, None, false), Do::Nothing);
        assert_eq!(decide(Some(&seen("a", "a")), None, None, false), Do::Forget);
        // A sign-in made here goes out; one gone from the vault since the last sync goes here too
        assert_eq!(decide(None, Some("a"), None, false), Do::Push);
        assert_eq!(
            decide(Some(&seen("a", "a")), Some("a"), None, false),
            Do::DropHere
        );
        assert_eq!(
            decide(Some(&seen("a", "a")), Some("b"), None, false),
            Do::Push,
            "a new sign-in here after the removal"
        );
        // A usable copy is taken; one the phone can't use never is
        assert_eq!(decide(None, None, Some(&t("a", true)), false), Do::Pull);
        assert_eq!(decide(None, None, Some(&t("a", false)), false), Do::Nothing);
        // Signed out here since the last sync: out of the vault too
        assert_eq!(
            decide(Some(&seen("a", "x")), None, Some(&t("x", true)), false),
            Do::Remove
        );
        assert_eq!(
            decide(Some(&seen("a", "x")), None, Some(&t("y", true)), false),
            Do::Pull,
            "a newer sign-in elsewhere"
        );
        // Each side compared with what it was: the phone's copy may read differently from the vault's
        assert_eq!(
            decide(Some(&seen("a", "x")), Some("a"), Some(&t("x", true)), false),
            Do::InStep
        );
        assert_eq!(
            decide(Some(&seen("a", "x")), Some("a"), Some(&t("y", true)), false),
            Do::Pull
        );
        assert_eq!(
            decide(Some(&seen("a", "x")), Some("b"), Some(&t("x", true)), false),
            Do::Push
        );
        assert_eq!(
            decide(Some(&seen("a", "x")), Some("a"), Some(&t("y", false)), false),
            Do::Nothing
        );
        // Both changed: the vault's usable copy wins; one the phone can't use is written over
        assert_eq!(
            decide(Some(&seen("a", "x")), Some("b"), Some(&t("y", true)), false),
            Do::Pull
        );
        assert_eq!(
            decide(Some(&seen("a", "x")), Some("b"), Some(&t("y", false)), false),
            Do::Push
        );
        // First contact with two copies
        assert_eq!(decide(None, Some("b"), Some(&t("c", true)), false), Do::Pull);
        assert_eq!(decide(None, Some("b"), Some(&t("c", false)), false), Do::Push);
        // Made before a sign-out everywhere: never back into the vault
        assert_eq!(decide(None, Some("b"), None, true), Do::DropHere);
        assert_eq!(
            decide(Some(&seen("a", "x")), Some("b"), Some(&t("x", true)), true),
            Do::Pull
        );
        assert_eq!(
            decide(Some(&seen("a", "x")), Some("b"), Some(&t("x", false)), true),
            Do::DropHere
        );
    }
}
