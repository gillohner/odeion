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
use super::store::FileStore;
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
/// How long a renewal waits for a vault pass running.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(20);
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

/// Sets what the phone holds for `form` (`travels`, `None`: only the id) and the id of that
/// sign-in, in one step and only when the phone still holds what it held at `from` (its
/// `vault_form::hash`; `None`: nothing): a sign-in or sign-out made on the phone while the vault
/// was being read wins. `Ok(false)`: it did, nothing changed.
fn apply_if_still(
    store: &FileStore,
    form: &Form,
    from: Option<&str>,
    travels: Option<&Map<String, Value>>,
    lineage: Option<&str>,
) -> Result<bool, VaultStoreError> {
    store
        .change_if(|d| {
            let now = vault_form::carried(form.carry, d)?.map(|m| vault_form::hash(&m));
            if now.as_deref() != from {
                return Ok(false);
            }
            if let Some(t) = travels {
                vault_form::apply(form.carry, t, d)?;
            }
            d.put(&lineage_key(form.service), lineage);
            Ok(true)
        })
        .map_err(local)
}

/// A sign-in was just made on the phone: it gets a new id (a renewal keeps it), dated now, so the
/// vault can tell it from a refresh of the one before, and from one made before a sign-out
/// everywhere.
pub fn signed_in(phone: &Phone, service: &str) {
    if let Err(e) = sign_in_here(phone, service, |_| true) {
        tracing::warn!("phone mode: a sign-in's id couldn't be saved: {e}");
    }
}

/// A sign-in made here: `write` puts it in the phone's sign-ins (and says whether it did), and in
/// the same step it gets its new id (dated after the last "Sign out everywhere", as on a node),
/// so a sync never sees the new sign-in under the old id. Then the vault hears of it. An error when
/// it couldn't be saved: then nothing changed.
pub fn sign_in_here(
    phone: &Phone,
    service: &str,
    write: impl FnOnce(&dyn SessionStore) -> bool,
) -> Result<bool, String> {
    // (A record that can't be read: no floor; nothing is synced until it reads anyway)
    let floor = load_record(&record_path(phone))
        .inspect_err(|e| tracing::warn!("phone mode: {e}"))
        .ok()
        .and_then(|r| r.marker);
    let id = vault_form::lineage_after(service, floor, pm_sources::now_unix() as u64);
    let kept = phone.store.change_if(|d| {
        if !write(d) {
            return Ok(false);
        }
        d.put(&lineage_key(service), Some(&id));
        Ok(true)
    })?;
    if kept {
        phone.sync_soon();
    }
    Ok(kept)
}

/// Whether what the phone holds for `service` differs from what the last sync recorded (a
/// sign-in, a sign-out or a renewal here; not a health note). A record that can't be read: yes
/// (the pass then says why it can't sync).
pub fn changed_here(phone: &Phone, form: &Form) -> bool {
    let mine = vault_form::carried(form.carry, phone.store.as_ref())
        .ok()
        .flatten()
        .map(|m| vault_form::hash(&m));
    let Ok(record) = load_record(&record_path(phone)) else {
        return true;
    };
    let seen = record.services.get(form.service).map(|s| s.mine.clone());
    mine != seen
}

/// What both sides held at the last sync, per service.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Synced {
    /// `vault_form::hash` of what travels, as the phone keeps it.
    pub mine: String,
    /// The hash of the vault's copy (`hash_of`).
    pub theirs: String,
    /// The id of the sign-in both held (a renewal keeps it; a new sign-in changes it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage: Option<String>,
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

/// The record of the last sync; with none yet (a first sync), empty. One that can't be read, or
/// isn't the record's JSON, is an error and is left as it is: taken for a first sync, the
/// first-contact rules could write an old sign-in over a newer one in the vault, or take back one
/// signed out here. Then nothing is synced until it reads again.
fn load_record(path: &Path) -> Result<Record, VaultStoreError> {
    let bad = |e: &dyn std::fmt::Display| {
        local(format!(
            "the record of the last vault sync ({}) can't be read ({e}): nothing is synced \
             with the vault until it can",
            path.display()
        ))
    };
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| bad(&e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Record::default()),
        Err(e) => Err(bad(&e)),
    }
}

/// Written whole (to disk before it replaces the old one), so a crash never leaves a record that
/// can't be read.
fn save_record(path: &Path, r: &Record) {
    let tmp = path.with_extension("json.tmp");
    let write = || -> std::io::Result<()> {
        use std::io::Write;
        let body = serde_json::to_vec(r).map_err(std::io::Error::other)?;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&body)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&tmp);
        tracing::warn!("phone mode: the vault record could not be saved: {e}");
    }
}

/// The vault's copy of a service, as the sync rules see it.
pub struct Theirs<'a> {
    pub hash: String,
    /// Its sign-in's id (`node.lineage`).
    pub lineage: Option<String>,
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

/// The sync rules for one service, the node's (`vault_cache::decide`) for one device. `seen`: what
/// both held at the last sync; `mine`: the hash of what travels here, with `here`, the id of its
/// sign-in; `theirs`: the vault's copy; `predates`: the phone's copy was made before the last
/// sign-out everywhere and isn't the one synced since.
///
/// A renewal (the same sign-in id) is not a new sign-in: a sign-out on either side stays one even
/// when the other side renewed meanwhile. A new sign-in goes out, also over a renewal there; a new
/// sign-in there is taken when the phone can use it. A copy the phone can't use is never taken,
/// and written over only by a sign-in made here.
pub fn decide(
    seen: Option<&Synced>,
    mine: Option<&str>,
    here: Option<&str>,
    theirs: Option<&Theirs<'_>>,
    predates: bool,
) -> Do {
    let pull_or = |t: &Theirs<'_>, otherwise: Do| if t.usable { Do::Pull } else { otherwise };
    if mine.is_some() && predates {
        // Never back into the vault: the vault's own copy, or out
        return match theirs {
            Some(t) if t.usable => Do::Pull,
            _ => Do::DropHere,
        };
    }
    // A sign-in made here (or there) since the last sync: another id than the one both held
    let new_here = |s: &Synced| here.is_some() && here != s.lineage.as_deref();
    let new_there = |s: &Synced, t: &Theirs<'_>| t.lineage != s.lineage;
    match (mine, theirs) {
        (None, None) => {
            if seen.is_some() {
                Do::Forget
            } else {
                Do::Nothing
            }
        }
        (Some(_), None) => match seen {
            // Gone from the vault: signed out elsewhere, even if renewed here since
            Some(s) if !new_here(s) => Do::DropHere,
            _ => Do::Push,
        },
        (None, Some(t)) => match seen {
            // Signed out here: out of the vault too, even if renewed there since
            Some(s) if !new_there(s, t) => Do::Remove,
            _ => pull_or(t, Do::Nothing),
        },
        (Some(m), Some(t)) => match seen {
            None if m == t.hash => Do::InStep,
            // First contact: a sign-in made here goes out; else the vault's, where usable
            None if here.is_some() && here != t.lineage.as_deref() => Do::Push,
            None => pull_or(t, Do::Nothing),
            Some(s) => match (m != s.mine, t.hash != s.theirs) {
                // The same credentials signed in anew here (a new id): a new sign-in, out it goes
                (false, _) if new_here(s) => Do::Push,
                (false, false) => Do::InStep,
                (true, false) if t.usable || new_here(s) => Do::Push,
                (true, false) => Do::Nothing,
                (false, true) => pull_or(t, Do::Nothing),
                (true, true) if new_here(s) => Do::Push,
                (true, true) => pull_or(t, Do::Nothing),
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
fn usable(phone: &Phone, form: &Form, s: &SignIn, marker: Option<u64>) -> bool {
    let Some(travels) = vault_form::node_settings(s) else {
        return false;
    };
    // Made before the last "Sign out everywhere" (a copy left behind or put back): not taken, as
    // a node doesn't take it
    let made = vault_form::node_lineage(s)
        .as_deref()
        .map(vault_form::lineage_at)
        .unwrap_or(0);
    if marker.is_some_and(|m| made <= m) {
        return false;
    }
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
    // One vault step at a time (a pass, a renewal): each reads and writes synced.json
    let _one = phone.vault_lock.lock().await;
    let d = dir(phone);
    std::fs::create_dir_all(&d).map_err(local)?;
    // First: without what the last sync saw, nothing is pushed, pulled or removed
    let path = record_path(phone);
    let mut record = load_record(&path)?;
    let secret = pm_pubky::vault::load_or_create_client_secret(&d.join("client.key")).map_err(local)?;
    let files = (d.join("revoked.json"), d.join("marks.json"));
    let vault = vault_of(acct, &secret, &files);
    vault.touch(DEVICE_NAME).await?;
    vault.share_with_trusted().await?;
    let marker = vault.marker(&vault.index().await?)?;
    let mut report = Report::default();
    for form in FORMS {
        let svc = form.service;
        // What the phone holds and its id, read together (a sign-in here writes both at once)
        let (mut mine, mut here) = (None, None);
        phone
            .store
            .change_if(|d| {
                mine = vault_form::carried(form.carry, d)?;
                here = d.get(&lineage_key(svc));
                Ok(false)
            })
            .map_err(local)?;
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
            lineage: vault_form::node_lineage(&o.sign_in),
            usable: usable(phone, form, &o.sign_in, marker),
            sign_in: &o.sign_in,
        });
        let mine_hash = mine.as_ref().map(vault_form::hash);
        let seen = record.services.get(svc);
        // Made before the last sign-out everywhere, and not the copy synced since it
        let synced_since =
            seen.is_some_and(|s| Some(&s.mine) == mine_hash.as_ref()) && record.marker == marker;
        let predates = marker.is_some_and(|m| here.as_deref().map(vault_form::lineage_at).unwrap_or(0) <= m)
            && !synced_since;
        match decide(
            seen,
            mine_hash.as_deref(),
            here.as_deref(),
            theirs.as_ref(),
            predates,
        ) {
            Do::Nothing => {}
            Do::InStep => {
                let t = theirs.as_ref().expect("in step with a copy");
                // The id of the sign-in both hold is the phone's too
                if t.lineage.is_some()
                    && here != t.lineage
                    && !apply_if_still(
                        &phone.store,
                        form,
                        mine_hash.as_deref(),
                        None,
                        t.lineage.as_deref(),
                    )?
                {
                    report.skipped.push(format!("{svc}: changed meanwhile"));
                    continue;
                }
                record.services.insert(
                    svc.into(),
                    Synced {
                        mine: mine_hash.clone().unwrap_or_default(),
                        theirs: t.hash.clone(),
                        lineage: t.lineage.clone(),
                    },
                );
            }
            Do::Forget => {
                record.services.remove(svc);
            }
            Do::Pull => {
                let t = theirs.as_ref().expect("a copy to take");
                let travels = vault_form::node_settings(t.sign_in).expect("a node's copy");
                let lineage = vault_form::node_lineage(t.sign_in);
                if !apply_if_still(
                    &phone.store,
                    form,
                    mine_hash.as_deref(),
                    Some(&travels),
                    lineage.as_deref(),
                )? {
                    report.skipped.push(format!("{svc}: changed meanwhile"));
                    continue;
                }
                after_pull(phone, form);
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
                        lineage: t.lineage.clone(),
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
                    // The id with the copy, unless the phone's sign-in changed during the write: its
                    // own new id stays, and the next pass (what was written is recorded) writes it
                    apply_if_still(&phone.store, form, mine_hash.as_deref(), None, Some(&lineage))?;
                    record.services.insert(
                        svc.into(),
                        Synced {
                            mine: vault_form::hash(travels),
                            theirs: hash_of(&s),
                            lineage: Some(lineage.clone()),
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
                if !apply_if_still(&phone.store, form, mine_hash.as_deref(), Some(&Map::new()), None)? {
                    report.skipped.push(format!("{svc}: changed meanwhile"));
                    continue;
                }
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

/// What a renewal under the vault's lock did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Renewal {
    /// Renewed (here, or by another device): the phone has a fresh session.
    Renewed,
    /// The vault has no copy of TIDAL in step with the phone's: the phone renews on its own.
    NotInVault,
    /// Another vault step holds the lock (a renewal, a pass): never a renewal beside it.
    Busy,
    /// TIDAL refused to renew the sign-in (its words): signing in again is the way back.
    Refused(String),
    /// TIDAL was asked, but what came of it didn't reach the phone (why): it may have renewed,
    /// so the phone never renews this one on its own.
    Unclear(String),
}

/// TIDAL renewed under the vault blob's lock (VAULT.md, "Refreshing a token"): the session read
/// under the lock is renewed with the client that made it and written back; one another device
/// renewed meanwhile is used as it is. The result is the phone's session too. `NotInVault` when the
/// vault has no copy of TIDAL in step with the phone's (the phone then renews on its own and the
/// next sync writes it).
pub async fn renew_tidal(phone: &std::sync::Arc<Phone>, acct: &Account) -> Result<Renewal, VaultStoreError> {
    // Behind a pass at most a while (a stalled pass ends by its own limit): then the phone keeps
    // its session for now
    let Ok(_one) = tokio::time::timeout(LOCK_WAIT, phone.vault_lock.lock()).await else {
        return Ok(Renewal::Busy);
    };
    let svc = TIDAL_FULL.service;
    let path = record_path(phone);
    let Some(mine) = vault_form::carried(TIDAL_FULL.carry, phone.store.as_ref()).map_err(local)? else {
        return Ok(Renewal::NotInVault);
    };
    // Can't be read: an error (a lasting one: the phone renews on its own, as without a vault)
    let record = load_record(&path)?;
    if record.services.get(svc).map(|s| s.mine.as_str()) != Some(vault_form::hash(&mine).as_str()) {
        return Ok(Renewal::NotInVault);
    }
    // "Sign out everywhere" as the last sync saw it
    let marker = record.marker;
    let d = dir(phone);
    let secret = pm_pubky::vault::load_or_create_client_secret(&d.join("client.key")).map_err(local)?;
    let files = (d.join("revoked.json"), d.join("marks.json"));
    let vault = vault_of(acct, &secret, &files);
    let p = phone.clone();
    // Whether TIDAL was asked, and its own refusal (not a failure on the way), kept apart from
    // the vault's errors
    let refused = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let refused2 = refused.clone();
    let asked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let asked2 = asked.clone();
    let r = vault
        .refresh(svc, false, |si: SignIn| async move {
            if !usable(&p, &TIDAL_FULL, &si, marker) {
                return Err(RefreshFailed(
                    "the vault's TIDAL sign-in was made with another app".into(),
                ));
            }
            let travels = vault_form::node_settings(&si).ok_or_else(|| {
                RefreshFailed("the vault's TIDAL sign-in isn't one this phone can renew".into())
            })?;
            let sess: pm_sources::tidal::full::Session = vault_form::session_of(TIDAL_FULL.carry, &travels)
                .and_then(|m| serde_json::from_value(Value::Object(m)).ok())
                .ok_or_else(|| RefreshFailed("the vault's TIDAL sign-in has no session".into()))?;
            let p2 = p.clone();
            asked2.store(true, std::sync::atomic::Ordering::SeqCst);
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
                    pm_sources::tidal::full::FullError::Mode(m) => {
                        *refused2.lock().unwrap_or_else(|e| e.into_inner()) = Some(m.clone());
                        m
                    }
                    pm_sources::tidal::full::FullError::Track(m) => m,
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
        .await;
    let r = match r {
        Ok(r) => r,
        // Renewed, but the vault didn't take it yet: the new tokens may be the only ones that
        // still work, so the phone keeps them; the record stays as it was, so the next pass
        // writes them to the vault
        Err(VaultStoreError::Unsaved { sign_in, reason, .. }) => {
            tracing::warn!("phone mode: renewed TIDAL kept here, the vault takes it later: {reason}");
            return Ok(match vault_form::node_settings(&sign_in) {
                Some(travels) => keep_renewed(phone, &mine, &travels, vault_form::node_lineage(&sign_in))
                    .unwrap_or(Renewal::Renewed),
                // (Never: the phone made this sign-in from such a copy)
                None => Renewal::Unclear("the renewed sign-in isn't one this phone can keep".into()),
            });
        }
        Err(e @ VaultStoreError::Refresh(_)) => {
            return match refused.lock().unwrap_or_else(|e| e.into_inner()).take() {
                Some(why) => Ok(Renewal::Refused(why)),
                // TIDAL was asked and didn't answer cleanly: it may have renewed
                None if asked.load(std::sync::atomic::Ordering::SeqCst) => {
                    Ok(Renewal::Unclear(e.to_string()))
                }
                // The vault's copy is not one this phone renews (another app, no session):
                // the phone renews its own
                None => Ok(Renewal::NotInVault),
            };
        }
        // After TIDAL was asked (its new tokens dropped: the sign-in changed hands meanwhile)
        Err(e) if asked.load(std::sync::atomic::Ordering::SeqCst) => {
            return Ok(Renewal::Unclear(e.to_string()))
        }
        Err(e) => return Err(e),
    };
    let si = r.sign_in();
    // One renewed by another device with another app is not taken here
    if !usable(phone, &TIDAL_FULL, si, marker) {
        return Ok(Renewal::NotInVault);
    }
    let Some(travels) = vault_form::node_settings(si) else {
        return Ok(Renewal::NotInVault);
    };
    // Taken only if the phone still holds the session it started from (a sign-out or a new
    // sign-in here meanwhile wins), with the id of the sign-in it takes. Not taken (changed
    // meanwhile, held, or unclear): the record stays as it was, and the next pass sees the
    // vault's copy as renewed there
    if let Some(other) = keep_renewed(phone, &mine, &travels, vault_form::node_lineage(si)) {
        return Ok(other);
    }
    let kept = vault_form::carried(TIDAL_FULL.carry, phone.store.as_ref())
        .ok()
        .flatten()
        .map(|m| vault_form::hash(&m))
        .unwrap_or_default();
    // (Read before the renewal under the same lock; should it not read now, it stays as it is)
    match load_record(&path) {
        Ok(mut record) => {
            record.services.insert(
                svc.into(),
                Synced {
                    mine: kept,
                    theirs: hash_of(si),
                    lineage: vault_form::node_lineage(si),
                },
            );
            save_record(&path, &record);
        }
        Err(e) => tracing::warn!("phone mode: {e}"),
    }
    if matches!(r, Refreshed::ByUs(_)) {
        tracing::info!("phone mode: TIDAL renewed under the vault's lock");
    }
    Ok(Renewal::Renewed)
}

/// The TIDAL session a renewal under the vault's lock gave (`travels`, the sign-in's `lineage`)
/// put in the phone's sign-ins, only over the one it started from (`mine`). `None`: saved.
/// Once TIDAL answered, nothing here lets the phone renew again on its own with the session
/// before (no error, no `NotInVault`): a sign-in or sign-out made here meanwhile wins
/// (`Renewed`: the phone uses what it holds now); a save that failed keeps the new session in
/// memory ([`super::tidal::hold_unsaved`], `Renewed`), or, without a session in it, is `Unclear`.
fn keep_renewed(
    phone: &Phone,
    mine: &Map<String, Value>,
    travels: &Map<String, Value>,
    lineage: Option<String>,
) -> Option<Renewal> {
    match apply_if_still(
        &phone.store,
        &TIDAL_FULL,
        Some(&vault_form::hash(mine)),
        Some(travels),
        lineage.as_deref(),
    ) {
        Ok(true) => None,
        Ok(false) => Some(Renewal::Renewed),
        Err(e) => {
            let fresh = vault_form::session_of(TIDAL_FULL.carry, travels).and_then(|m| {
                serde_json::from_value::<pm_sources::tidal::full::Session>(Value::Object(m)).ok()
            });
            let from = pm_sources::tidal::full::load(phone.store.as_ref())
                .session
                .map(|s| s.access_token);
            Some(match (fresh, from) {
                (Some(fresh), Some(from)) => {
                    tracing::warn!("phone mode: TIDAL renewed under the vault's lock, not saved here: {e}");
                    super::tidal::hold_unsaved(phone, from, fresh);
                    Renewal::Renewed
                }
                _ => Renewal::Unclear(format!("renewed, but not saved here: {e}")),
            })
        }
    }
}

/// Whether the phone's TIDAL sign-in is the one the last sync found in step with the vault (a
/// renewal then goes through the vault). A record that can't be read: no (as [`renew_tidal`]'s
/// lasting error: the phone renews on its own).
pub fn tidal_in_step(phone: &Phone) -> bool {
    let Ok(Some(mine)) = vault_form::carried(TIDAL_FULL.carry, phone.store.as_ref()) else {
        return false;
    };
    load_record(&record_path(phone)).is_ok_and(|r| {
        r.services.get(TIDAL_FULL.service).map(|s| s.mine.as_str()) == Some(vault_form::hash(&mine).as_str())
    })
}

/// Whether a failure before TIDAL was asked ([`renew_tidal`]'s error) leaves open that the
/// sign-in is being renewed elsewhere: the homeserver didn't answer (or failed in a way it may not
/// next time, or said "not now"), another step holds the blob, or it served a copy older than one
/// seen (a newer write exists). Then the phone never renews it on its own beside the vault. After
/// any other failure (the homeserver said no, the vault's copy can't be read or used here, a local
/// file) it does, as without a vault, so a lasting vault problem never stops TIDAL. (Once TIDAL
/// was asked, the answer is [`Renewal::Unclear`] instead.)
pub fn uncertain(e: &VaultStoreError) -> bool {
    match e {
        VaultStoreError::Busy(_)
        | VaultStoreError::TimedOut(_)
        | VaultStoreError::Refresh(_)
        | VaultStoreError::Stale { .. } => true,
        VaultStoreError::Homeserver(h) => {
            h.is_transient()
                || h.is_not_now()
                || matches!(
                    h,
                    pm_pubky::homeserver::HomeserverError::Transport(_)
                        | pm_pubky::homeserver::HomeserverError::Timeout
                )
        }
        _ => false,
    }
}

/// Whether the phone's TIDAL session runs out within a minute.
pub fn tidal_due(phone: &Phone) -> bool {
    pm_sources::tidal::full::load(phone.store.as_ref())
        .session
        .is_some_and(|s| s.expires_at - pm_sources::now_unix() <= 60)
}

/// Tests: TIDAL signed in here with a session that runs out in 30 s, made with the phone's device
/// client, and in step with the vault as the last sync saw it (a renewal goes through the vault).
#[cfg(test)]
pub(crate) fn tidal_due_in_step(p: &Phone) {
    pm_sources::tidal::full::update(p.store.as_ref(), |st| {
        st.enabled = true;
        st.session = Some(pm_sources::tidal::full::Session {
            access_token: "due".into(),
            refresh_token: Some("rt".into()),
            expires_at: pm_sources::now_unix() + 30,
            country: "CH".into(),
            ..Default::default()
        });
    });
    let mine = vault_form::carried(TIDAL_FULL.carry, p.store.as_ref())
        .unwrap()
        .unwrap();
    std::fs::create_dir_all(dir(p)).unwrap();
    let mut r = Record::default();
    r.services.insert(
        TIDAL_FULL.service.into(),
        Synced {
            mine: vault_form::hash(&mine),
            theirs: "x".into(),
            lineage: None,
        },
    );
    save_record(&record_path(p), &r);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone_here(dir: &std::path::Path) -> std::sync::Arc<Phone> {
        Phone::open(super::super::Config {
            dir: dir.to_path_buf(),
            cache: dir.join("streams"),
            version: "test".into(),
            pubky: None,
            tidal: Default::default(),
        })
        .unwrap()
    }

    #[test]
    fn a_sign_in_and_its_new_id_are_one_step() {
        let dir = tempfile::tempdir().unwrap();
        let p = phone_here(dir.path());
        // "Sign out everywhere" was this second: the new id is dated after it
        let now = pm_sources::now_unix() as u64;
        std::fs::create_dir_all(super::dir(&p)).unwrap();
        save_record(
            &record_path(&p),
            &Record {
                marker: Some(now),
                ..Default::default()
            },
        );
        let session = || {
            Some(pm_sources::tidal::full::Session {
                access_token: "new".into(),
                expires_at: 1,
                country: "CH".into(),
                ..Default::default()
            })
        };
        let kept = sign_in_here(&p, "tidal-full", |store| {
            pm_sources::tidal::full::update_if(store, |st| {
                st.session = session();
                true
            })
        });
        assert_eq!(kept, Ok(true));
        assert_eq!(
            pm_sources::tidal::full::load(p.store.as_ref())
                .session
                .unwrap()
                .access_token,
            "new"
        );
        let id = SessionStore::get(p.store.as_ref(), &lineage_key("tidal-full")).expect("its id");
        assert!(vault_form::lineage_at(&id) > now, "{id}");
        // One that didn't happen gets no id
        let before = SessionStore::get(p.store.as_ref(), &lineage_key("qobuz"));
        assert_eq!(sign_in_here(&p, "qobuz", |_| false), Ok(false));
        assert_eq!(SessionStore::get(p.store.as_ref(), &lineage_key("qobuz")), before);
        // A health note isn't a change for the vault, a sign-out is
        save_record(&record_path(&p), &{
            let mut r = Record::default();
            let mine = vault_form::carried(TIDAL_FULL.carry, p.store.as_ref())
                .unwrap()
                .unwrap();
            r.services.insert(
                "tidal-full".into(),
                Synced {
                    mine: vault_form::hash(&mine),
                    theirs: "x".into(),
                    lineage: Some(id),
                },
            );
            r
        });
        pm_sources::tidal::full::update(p.store.as_ref(), |st| st.health = None);
        assert!(!changed_here(&p, &TIDAL_FULL));
        pm_sources::tidal::full::update(p.store.as_ref(), |st| st.session = None);
        assert!(changed_here(&p, &TIDAL_FULL));
    }

    #[test]
    fn a_copy_from_before_sign_out_everywhere_is_not_taken() {
        let dir = tempfile::tempdir().unwrap();
        let phone = Phone::open(super::super::Config {
            dir: dir.path().to_path_buf(),
            cache: dir.path().join("streams"),
            version: "test".into(),
            pubky: None,
            tidal: super::super::TidalConf {
                device: Some(pm_sources::tidal::full::Client {
                    id: "device-client".into(),
                    secret: "s".into(),
                    from: "builtin".into(),
                }),
                ..Default::default()
            },
        })
        .unwrap();
        let mut travels = Map::new();
        travels.insert(
            "tidal.unofficial".into(),
            serde_json::json!({"session": {"access_token": "a", "expires_at": 1, "country": "CH"}}),
        );
        let copy = vault_form::sign_in_for(
            "tidal-full",
            TIDAL_FULL.carry,
            true,
            &travels,
            None,
            Some("device-client".into()),
            "1000-0123456789abcdef",
        );
        assert!(usable(&phone, &TIDAL_FULL, &copy, None));
        assert!(
            usable(&phone, &TIDAL_FULL, &copy, Some(999)),
            "made after the sign-out"
        );
        assert!(
            !usable(&phone, &TIDAL_FULL, &copy, Some(1000)),
            "made before it (or then)"
        );
        assert!(!usable(&phone, &TIDAL_FULL, &copy, Some(2000)));
    }

    #[test]
    fn what_the_vault_gives_waits_for_nothing_changed_here_meanwhile() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::open(dir.path()).unwrap();
        let session = |token: &str| {
            let mut t = Map::new();
            t.insert(
                "tidal.unofficial".into(),
                serde_json::json!({"session": {"access_token": token, "expires_at": 1, "country": "CH"}}),
            );
            t
        };
        let held = |store: &FileStore| {
            vault_form::carried(TIDAL_FULL.carry, store)
                .unwrap()
                .map(|m| vault_form::hash(&m))
        };
        vault_form::apply(TIDAL_FULL.carry, &session("a"), &store).unwrap();
        let from = held(&store);
        // Signed in anew on the phone while the vault was read: the vault's copy isn't taken
        vault_form::apply(TIDAL_FULL.carry, &session("here"), &store).unwrap();
        store.put(&lineage_key("tidal-full"), Some("2-new")).unwrap();
        let took = apply_if_still(
            &store,
            &TIDAL_FULL,
            from.as_deref(),
            Some(&session("vault")),
            Some("1-old"),
        )
        .unwrap();
        assert!(!took);
        assert!(SessionStore::get(&store, "tidal.unofficial")
            .unwrap()
            .contains("here"));
        assert_eq!(
            SessionStore::get(&store, &lineage_key("tidal-full")).as_deref(),
            Some("2-new")
        );
        // Signed out here meanwhile: nothing comes back
        vault_form::apply(TIDAL_FULL.carry, &Map::new(), &store).unwrap();
        assert!(!apply_if_still(
            &store,
            &TIDAL_FULL,
            from.as_deref(),
            Some(&session("vault")),
            None
        )
        .unwrap());
        assert_eq!(held(&store), None);
        // Nothing changed: taken, with its id, in one step
        assert!(apply_if_still(&store, &TIDAL_FULL, None, Some(&session("vault")), Some("1-old")).unwrap());
        assert!(SessionStore::get(&store, "tidal.unofficial")
            .unwrap()
            .contains("vault"));
        assert_eq!(
            SessionStore::get(&store, &lineage_key("tidal-full")).as_deref(),
            Some("1-old")
        );
    }

    fn seen(mine: &str, theirs: &str, lineage: &str) -> Synced {
        Synced {
            mine: mine.into(),
            theirs: theirs.into(),
            lineage: Some(lineage.into()),
        }
    }

    #[test]
    fn the_sync_rules() {
        let si: SignIn = serde_json::from_value(serde_json::json!({ "service": "tidal-full" })).unwrap();
        let t = |h: &str, lineage: &str, usable| Theirs {
            hash: h.into(),
            lineage: Some(lineage.into()),
            usable,
            sign_in: &si,
        };
        let s = seen("a", "x", "L1");
        // The same credentials signed in anew here (a new id): out, as a new sign-in
        assert_eq!(
            decide(Some(&s), Some("a"), Some("L2"), Some(&t("x", "L1", true)), false),
            Do::Push
        );
        // Nothing anywhere
        assert_eq!(decide(None, None, None, None, false), Do::Nothing);
        assert_eq!(decide(Some(&s), None, None, None, false), Do::Forget);
        // A sign-in made here goes out
        assert_eq!(decide(None, Some("a"), Some("L1"), None, false), Do::Push);
        // Gone from the vault: signed out elsewhere, also when renewed here since
        assert_eq!(decide(Some(&s), Some("a"), Some("L1"), None, false), Do::DropHere);
        assert_eq!(decide(Some(&s), Some("b"), Some("L1"), None, false), Do::DropHere);
        // ... but a new sign-in here after it goes out
        assert_eq!(decide(Some(&s), Some("b"), Some("L2"), None, false), Do::Push);
        // A usable copy is taken; one the phone can't use never is
        assert_eq!(
            decide(None, None, None, Some(&t("x", "L1", true)), false),
            Do::Pull
        );
        assert_eq!(
            decide(None, None, None, Some(&t("x", "L1", false)), false),
            Do::Nothing
        );
        // Signed out here: out of the vault too, also when renewed there since
        assert_eq!(
            decide(Some(&s), None, None, Some(&t("x", "L1", true)), false),
            Do::Remove
        );
        assert_eq!(
            decide(Some(&s), None, None, Some(&t("y", "L1", true)), false),
            Do::Remove
        );
        // ... but a new sign-in there is taken
        assert_eq!(
            decide(Some(&s), None, None, Some(&t("y", "L2", true)), false),
            Do::Pull
        );
        // Each side compared with what it was (the phone's copy may read differently)
        assert_eq!(
            decide(Some(&s), Some("a"), Some("L1"), Some(&t("x", "L1", true)), false),
            Do::InStep
        );
        assert_eq!(
            decide(Some(&s), Some("a"), Some("L1"), Some(&t("y", "L1", true)), false),
            Do::Pull
        );
        assert_eq!(
            decide(Some(&s), Some("b"), Some("L1"), Some(&t("x", "L1", true)), false),
            Do::Push
        );
        // A renewal here doesn't write over a copy the phone can't use; a new sign-in does
        assert_eq!(
            decide(Some(&s), Some("b"), Some("L1"), Some(&t("x", "L1", false)), false),
            Do::Nothing
        );
        assert_eq!(
            decide(Some(&s), Some("b"), Some("L2"), Some(&t("x", "L1", false)), false),
            Do::Push
        );
        // Both changed: a new sign-in here wins; a renewal on both sides takes the vault's
        assert_eq!(
            decide(Some(&s), Some("b"), Some("L2"), Some(&t("y", "L1", true)), false),
            Do::Push
        );
        assert_eq!(
            decide(Some(&s), Some("b"), Some("L1"), Some(&t("y", "L1", true)), false),
            Do::Pull
        );
        assert_eq!(
            decide(Some(&s), Some("b"), Some("L1"), Some(&t("y", "L1", false)), false),
            Do::Nothing
        );
        // First contact
        assert_eq!(
            decide(None, Some("x"), None, Some(&t("x", "L1", true)), false),
            Do::InStep
        );
        assert_eq!(
            decide(None, Some("b"), Some("L2"), Some(&t("c", "L1", true)), false),
            Do::Push
        );
        assert_eq!(
            decide(None, Some("b"), None, Some(&t("c", "L1", true)), false),
            Do::Pull
        );
        assert_eq!(
            decide(None, Some("b"), None, Some(&t("c", "L1", false)), false),
            Do::Nothing
        );
        // Made before a sign-out everywhere: never back into the vault
        assert_eq!(decide(None, Some("b"), Some("L0"), None, true), Do::DropHere);
        assert_eq!(
            decide(Some(&s), Some("b"), Some("L0"), Some(&t("x", "L1", true)), true),
            Do::Pull
        );
        assert_eq!(
            decide(Some(&s), Some("b"), Some("L0"), Some(&t("x", "L1", false)), true),
            Do::DropHere
        );
    }

    #[test]
    fn which_vault_failures_keep_the_phone_from_renewing_on_its_own() {
        use pm_pubky::homeserver::HomeserverError;
        let answered = |code: u16| {
            VaultStoreError::Homeserver(HomeserverError::Pubky(pubky::Error::Request(
                pubky::errors::RequestError::Server {
                    status: pubky::StatusCode::from_u16(code).unwrap(),
                    message: String::new(),
                },
            )))
        };
        // Another device may be renewing it: never beside it
        for e in [
            VaultStoreError::Busy("p".into()),
            VaultStoreError::TimedOut("p".into()),
            VaultStoreError::Homeserver(HomeserverError::Transport("reset".into())),
            VaultStoreError::Homeserver(HomeserverError::Timeout),
            answered(503),
            answered(429),
            answered(408),
            VaultStoreError::Stale {
                service: "tidal-full".into(),
                why: "older",
            },
        ] {
            assert!(uncertain(&e), "{e}");
        }
        // Lasting: renewed here, so TIDAL doesn't stop for good
        for e in [
            answered(401),
            answered(403),
            VaultStoreError::Revoked,
            VaultStoreError::NotOurs("tidal-full".into()),
            VaultStoreError::Local(std::io::Error::other("bad key")),
        ] {
            assert!(!uncertain(&e), "{e}");
        }
    }

    #[test]
    fn once_tidal_answered_nothing_leads_to_another_renewal() {
        let dir = tempfile::tempdir().unwrap();
        let p = phone_here(dir.path());
        tidal_due_in_step(&p);
        let mine = vault_form::carried(TIDAL_FULL.carry, p.store.as_ref())
            .unwrap()
            .unwrap();
        // What the renewal under the vault's lock gave
        let fresh = pm_sources::tidal::full::Session {
            access_token: "fresh".into(),
            refresh_token: Some("rt-2".into()),
            expires_at: pm_sources::now_unix() + 3600,
            country: "CH".into(),
            ..Default::default()
        };
        let key = TIDAL_FULL.carry[0].key;
        let mut travels = mine.clone();
        let mut held = travels.get(key).and_then(vault_form::object_of).unwrap();
        held.insert("session".into(), serde_json::to_value(&fresh).unwrap());
        travels.insert(key.into(), Value::Object(held));
        // The sign-ins file can't be replaced now (a full disk, say): no error (the phone would
        // renew again with the session before), the new session kept in memory
        let file = dir.path().join(super::super::store::FILE);
        let kept = std::fs::read(&file).unwrap();
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert_eq!(keep_renewed(&p, &mine, &travels, None), Some(Renewal::Renewed));
        assert!(
            p.tidal_unsaved.lock().unwrap().is_some(),
            "held until it can be saved"
        );
        // A sign-out here meanwhile: that wins, and nothing is renewed again either
        std::fs::remove_dir(&file).unwrap();
        std::fs::write(&file, kept).unwrap();
        super::super::tidal::sign_out(&p).unwrap();
        assert_eq!(keep_renewed(&p, &mine, &travels, None), Some(Renewal::Renewed));
        assert!(pm_sources::tidal::full::load(p.store.as_ref()).session.is_none());
    }

    #[test]
    fn a_record_that_cant_be_read_is_not_a_first_sync() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("synced.json");
        // None yet: a first sync
        assert_eq!(load_record(&path).unwrap(), Record::default());
        // Not the record's JSON, or not readable at all: an error
        std::fs::write(&path, br#"{"services":{"tidal-full":"#).unwrap();
        assert!(matches!(load_record(&path), Err(VaultStoreError::Local(_))));
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(load_record(&path), Err(VaultStoreError::Local(_))));
    }

    #[tokio::test]
    async fn with_a_record_that_cant_be_read_nothing_is_synced() {
        let dir = tempfile::tempdir().unwrap();
        let p = phone_here(dir.path());
        // Signed in here and synced before; then the record is cut short
        tidal_due_in_step(&p);
        let path = record_path(&p);
        let cut = br#"{"services":{"tidal-full":{"mine":"#;
        std::fs::write(&path, cut).unwrap();
        let held = SessionStore::get(p.store.as_ref(), "tidal.unofficial");
        let hs = super::super::stalled::homeserver().await;
        let acct = hs.account();
        tokio::time::pause();
        let r = tokio::time::timeout(std::time::Duration::from_secs(3600), pass(&p, &acct))
            .await
            .expect("the pass ends by itself");
        assert!(matches!(r, Err(VaultStoreError::Local(_))), "{r:?}");
        // Neither is it taken for "in step" by a renewal
        let r = renew_tidal(&p, &acct).await;
        assert!(matches!(r, Err(VaultStoreError::Local(_))), "{r:?}");
        assert_eq!(
            hs.taken.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "the vault wasn't asked: nothing pushed, pulled or removed"
        );
        assert_eq!(std::fs::read(&path).unwrap(), cut, "the record as it was");
        assert_eq!(SessionStore::get(p.store.as_ref(), "tidal.unofficial"), held);
    }

    #[test]
    fn sign_ins_that_cant_be_read_are_never_a_sign_out() {
        let dir = tempfile::tempdir().unwrap();
        // Signed in to TIDAL and synced with the vault before
        let p = phone_here(dir.path());
        tidal_due_in_step(&p);
        let record = std::fs::read(record_path(&p)).unwrap();
        drop(p);
        // The sign-ins file can't be read now: phone mode doesn't start (no pass would find TIDAL
        // signed out here and remove it from the vault), and the files stay as they are
        let file = dir.path().join(super::super::store::FILE);
        let cut = br#"{"tidal.unofficial":"#;
        std::fs::write(&file, cut).unwrap();
        assert!(Phone::open(super::super::Config {
            dir: dir.path().to_path_buf(),
            cache: dir.path().join("streams"),
            version: "test".into(),
            pubky: None,
            tidal: Default::default(),
        })
        .is_err());
        assert_eq!(std::fs::read(&file).unwrap(), cut);
        let again = dir.path().join("vault").join("synced.json");
        assert_eq!(std::fs::read(again).unwrap(), record);
    }

    #[tokio::test]
    async fn a_homeserver_that_never_answers_never_holds_the_vault() {
        let dir = tempfile::tempdir().unwrap();
        let p = phone_here(dir.path());
        tidal_due_in_step(&p);
        let hs = super::super::stalled::homeserver().await;
        let acct = hs.account();
        // From here only the homeserver is waited for: time moves on whenever nothing else can
        tokio::time::pause();
        let started = tokio::time::Instant::now();
        let r = tokio::time::timeout(std::time::Duration::from_secs(3600), renew_tidal(&p, &acct))
            .await
            .expect("the renewal ends by itself");
        assert!(matches!(r, Err(VaultStoreError::Busy(_))), "{r:?}");
        assert!(started.elapsed() <= std::time::Duration::from_secs(60));
        assert!(
            hs.taken.load(std::sync::atomic::Ordering::SeqCst) > 0,
            "it waited on the homeserver"
        );
        // The vault is free again: the next pass or renewal doesn't wait for this one
        assert!(p.vault_lock.try_lock().is_ok());
    }
}
