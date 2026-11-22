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
/// The longest a renewal waits for the vault's copy when its record doesn't say whether the vault
/// has the phone's sign-in.
const READ_FOR: std::time::Duration = pm_pubky::homeserver::OBJECT_TIMEOUT;
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
/// was being read wins. `Ok(false)`: it did, nothing changed. A copy that would leave a sign-in
/// this version can't read (another version's session, say) is an error, and nothing changed:
/// kept, it would read as signed out, and the next start would refuse the file.
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
                for c in form.carry {
                    if let Some(v) = d.get(c.key) {
                        super::store::sign_in_reads(c.key, &v)
                            .map_err(|e| format!("not a sign-in this version reads ({e})"))?;
                    }
                }
                if !t.is_empty() {
                    not_set_aside(d, form);
                }
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
        if let Some(form) = FORMS.iter().find(|f| f.service == service) {
            not_set_aside(d, form);
        }
        Ok(true)
    })?;
    if kept {
        phone.sync_soon();
    }
    Ok(kept)
}

/// Whether what the phone holds for `service`, and the id of its sign-in, differ from what the
/// last sync recorded (a sign-in, a sign-out or a renewal here; not a health note). A sign-out of
/// a sign-in the record doesn't have yet (a pass cut short after taking it) is one too. A record
/// that can't be read: yes (the pass then says why it can't sync).
pub fn changed_here(phone: &Phone, form: &Form) -> bool {
    let mine = vault_form::carried(form.carry, phone.store.as_ref())
        .ok()
        .flatten()
        .map(|m| vault_form::hash(&m));
    let here = SessionStore::get(phone.store.as_ref(), &lineage_key(form.service));
    let Ok(record) = load_record(&record_path(phone)) else {
        return true;
    };
    let seen = record
        .services
        .get(form.service)
        .map(|s| (Some(s.mine.clone()), s.lineage.clone()));
    (mine, here) != seen.unwrap_or_default()
}

/// Whether `form`'s sign-in was set aside here and not signed in again since.
fn set_aside(store: &dyn SessionStore, form: &Form) -> bool {
    form.carry
        .iter()
        .any(|c| SessionStore::get(store, &super::store::set_aside_key(c.key)).is_some())
}

/// `apply_if_still`'s draft, `d`, with a sign-in in it again: no longer set aside.
fn not_set_aside(d: &super::store::Draft, form: &Form) {
    for c in form.carry {
        d.put(&super::store::set_aside_key(c.key), None);
    }
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
    /// The device list's "signed out everywhere" marker at the last sync that went through every
    /// service.
    #[serde(default)]
    pub marker: Option<u64>,
    /// Services whose take from the vault, write to it or renewal under its lock was under way
    /// and isn't in `services` yet (a pass or a renewal cut short, or this record not saved after
    /// it): the vault may hold the phone's sign-in though `services` doesn't say so, so a renewal
    /// asks the vault first ([`renew_tidal`]), and the next pass reads the vault's copy against
    /// what the record would say had the step been recorded ([`UnderWay`]). Written before the
    /// step, gone once it is recorded.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub under_way: BTreeMap<String, UnderWay>,
}

/// A step of one service under way ([`Record::under_way`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnderWay {
    /// What `services` holds for it once the step is recorded (a renewal's: only the id).
    pub then: Synced,
    /// A write to the vault: then the hash of the copy it replaces (`None`: none), which is still
    /// there when the write didn't land.
    #[serde(default)]
    pub write: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
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
/// can't be read. An error when it couldn't be: the record on disk is the one before.
fn save_record(path: &Path, r: &Record) -> Result<(), VaultStoreError> {
    let tmp = path.with_extension("json.tmp");
    let write = || -> std::io::Result<()> {
        use std::io::Write;
        let body = serde_json::to_vec(r).map_err(std::io::Error::other)?;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&body)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    };
    write().map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        local(format!(
            "the record of the vault sync ({}) can't be saved: {e}",
            path.display()
        ))
    })
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
            // The very sign-in the phone held (a sign-out keeps its id) though the record doesn't
            // say so (a pass cut short after it took or wrote it): signed out here all the same
            _ if here.is_some() && here == t.lineage.as_deref() => Do::Remove,
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

/// The vault steps a pass takes for each service: the vault client's own, or a test's stand-in.
pub(crate) trait Side {
    /// The service's copy (`None`: there is none).
    async fn read(&self, service: &str) -> Result<Option<SignIn>, VaultStoreError>;
    /// `s` written in the vault while its copy is still the one hashed `expect` (`None`: none) and
    /// "Sign out everywhere" is still `marker`; whether it was written.
    async fn write_if(
        &self,
        service: &str,
        s: &SignIn,
        marker: Option<u64>,
        expect: Option<String>,
    ) -> Result<bool, VaultStoreError>;
    /// The copy removed while it is still the one hashed `expect`; whether it was.
    async fn remove_if(&self, service: &str, expect: Option<String>) -> Result<bool, VaultStoreError>;
}

impl Side for Vault<'_> {
    async fn read(&self, service: &str) -> Result<Option<SignIn>, VaultStoreError> {
        Ok(Vault::read(self, service).await?.map(|o| o.sign_in))
    }
    async fn write_if(
        &self,
        service: &str,
        s: &SignIn,
        marker: Option<u64>,
        expect: Option<String>,
    ) -> Result<bool, VaultStoreError> {
        let written = self
            .write_shared_if(service, s, Some(marker), &|cur| cur.map(hash_of) == expect)
            .await?;
        Ok(written.is_some())
    }
    async fn remove_if(&self, service: &str, expect: Option<String>) -> Result<bool, VaultStoreError> {
        Vault::remove_if(self, service, &|cur| cur.map(hash_of) == expect).await
    }
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
    let report = services(phone, &vault, &path, &mut record, marker).await?;
    qobuz_app(phone).await;
    Ok(report)
}

/// `record` with `service`'s entry set to `entry` (`None`: none) and its step no longer under
/// way, saved at once (only when that changes it): before the next step, so a pass cut short
/// later (an error, its time limit, the app ending) never leaves a change the phone or the vault
/// made unrecorded. An error, which ends the pass, when it can't be saved.
fn record_service(
    path: &Path,
    record: &mut Record,
    service: &str,
    entry: Option<Synced>,
) -> Result<(), VaultStoreError> {
    let before = record.clone();
    match entry {
        Some(e) => {
            record.services.insert(service.into(), e);
        }
        None => {
            record.services.remove(service);
        }
    }
    record.under_way.remove(service);
    if *record != before {
        save_record(path, record)?;
    }
    Ok(())
}

/// `service`'s step marked under way in `record`, saved before it is taken (see
/// [`Record::under_way`]); an error, and the step isn't taken, when it can't be saved. Whether
/// none was noted before.
fn mark_under_way(
    path: &Path,
    record: &mut Record,
    service: &str,
    step: UnderWay,
) -> Result<bool, VaultStoreError> {
    let before = record.under_way.insert(service.into(), step.clone());
    if before.as_ref() != Some(&step) {
        if let Err(e) = save_record(path, record) {
            match before.clone() {
                Some(b) => record.under_way.insert(service.into(), b),
                None => record.under_way.remove(service),
            };
            return Err(e);
        }
    }
    Ok(before.is_none())
}

/// Each service by [`decide`], against the vault's `side`, with `record` (saved at `path`) as the
/// last sync saw it: each service's outcome is recorded before the next one is looked at, and
/// `marker` ("Sign out everywhere" now) only once every service was.
pub(crate) async fn services(
    phone: &Phone,
    side: &impl Side,
    path: &Path,
    record: &mut Record,
    marker: Option<u64>,
) -> Result<Report, VaultStoreError> {
    // "Sign out everywhere" as the last whole sync saw it (`record.marker` moves only at the end,
    // and only when every service's copy was read)
    let before = record.marker;
    let mut whole = true;
    let mut report = Report::default();
    for form in FORMS {
        let svc = form.service;
        // A TIDAL renewal held since its save failed, which the vault doesn't have: what the
        // file holds is not what the phone uses (its refresh token may be spent), so it isn't
        // synced until it is saved
        if svc == TIDAL_FULL.service {
            super::tidal::try_save_unsaved(phone);
        }
        if svc == TIDAL_FULL.service && super::tidal::held_outside_vault(phone) {
            report
                .skipped
                .push(format!("{svc}: a renewal waits to be saved here"));
            continue;
        }
        // What the phone holds and its id, read together (a sign-in here writes both at once), and
        // whether it was set aside at start (it couldn't be read)
        let (mut mine, mut here, mut aside) = (None, None, false);
        phone
            .store
            .change_if(|d| {
                mine = vault_form::carried(form.carry, d)?;
                here = d.get(&lineage_key(svc));
                aside = set_aside(d, form);
                Ok(false)
            })
            .map_err(local)?;
        let copy = match side.read(svc).await {
            Ok(o) => o,
            // Not (yet) shared with this phone, not one of the person's devices', or older than one
            // seen: what the phone has stays as it is
            Err(e) => {
                report.skipped.push(format!("{svc}: {e}"));
                whole = false;
                continue;
            }
        };
        let theirs = copy.as_ref().map(|si| Theirs {
            hash: hash_of(si),
            lineage: vault_form::node_lineage(si),
            usable: usable(phone, form, si, marker),
            sign_in: si,
        });
        let mine_hash = mine.as_ref().map(vault_form::hash);
        // A take or a write of this sign-in that a pass didn't get to record (cut short, or the
        // record not saved): what the record would say had it been, while the phone still holds
        // that sign-in (its id) and a write's copy is no longer the one it replaced. So the vault's
        // copy is read against it: one changed since (a new sign-in there) is taken, one gone
        // (signed out there) is signed out here, and neither is written over
        let seen = match record.under_way.get(svc) {
            Some(u)
                if u.then.lineage.is_some()
                    && here == u.then.lineage
                    && record.services.get(svc).map(|s| &s.lineage) != Some(&u.then.lineage)
                    && !(u.write && theirs.as_ref().map(|t| &t.hash) == u.before.as_ref()) =>
            {
                Some(u.then.clone())
            }
            _ => record.services.get(svc).cloned(),
        };
        // Made before the last sign-out everywhere, and not the copy synced since it
        let synced_since =
            seen.as_ref().is_some_and(|s| Some(&s.mine) == mine_hash.as_ref()) && before == marker;
        let predates = marker.is_some_and(|m| here.as_deref().map(vault_form::lineage_at).unwrap_or(0) <= m)
            && !synced_since;
        let what = if aside {
            // Set aside at start: signed out here only because it couldn't be read, so never a
            // sign-out for the vault, and nothing of it is written there; the vault's copy is
            // taken, as on a first sync, which is a sign-in again
            match decide(None, None, None, theirs.as_ref(), false) {
                Do::Pull if mine.is_none() => Do::Pull,
                _ => {
                    report
                        .skipped
                        .push(format!("{svc}: set aside here until signed in again"));
                    continue;
                }
            }
        } else {
            decide(
                seen.as_ref(),
                mine_hash.as_deref(),
                here.as_deref(),
                theirs.as_ref(),
                predates,
            )
        };
        match what {
            Do::Nothing => {
                record_service(path, record, svc, seen)?;
                if mine.is_none() && theirs.is_none() {
                    let_id_go(phone, form, here.as_deref());
                }
            }
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
                let entry = Synced {
                    mine: mine_hash.clone().unwrap_or_default(),
                    theirs: t.hash.clone(),
                    lineage: t.lineage.clone(),
                };
                record_service(path, record, svc, Some(entry))?;
            }
            Do::Forget => {
                record_service(path, record, svc, None)?;
                let_id_go(phone, form, here.as_deref());
            }
            Do::Pull => {
                let t = theirs.as_ref().expect("a copy to take");
                let travels = vault_form::node_settings(t.sign_in).expect("a node's copy");
                let lineage = vault_form::node_lineage(t.sign_in);
                // From the take on, the phone holds the vault's tokens: noted first, with what the
                // record will say (as the phone will keep it)
                let mut kept = String::new();
                phone
                    .store
                    .change_if(|d| {
                        vault_form::apply(form.carry, &travels, d)?;
                        kept = vault_form::carried(form.carry, d)?
                            .map(|m| vault_form::hash(&m))
                            .unwrap_or_default();
                        Ok(false)
                    })
                    .map_err(local)?;
                let step = UnderWay {
                    then: Synced {
                        mine: kept,
                        theirs: t.hash.clone(),
                        lineage: t.lineage.clone(),
                    },
                    ..Default::default()
                };
                mark_under_way(path, record, svc, step)?;
                match apply_if_still(
                    &phone.store,
                    form,
                    mine_hash.as_deref(),
                    Some(&travels),
                    lineage.as_deref(),
                ) {
                    Ok(true) => {}
                    Ok(false) => {
                        record_service(path, record, svc, seen)?;
                        report.skipped.push(format!("{svc}: changed meanwhile"));
                        continue;
                    }
                    // Not taken (a copy this version can't read, or not saved): nothing recorded,
                    // the next pass tries again
                    Err(e) => {
                        tracing::warn!("phone mode: vault: {svc}: the vault's copy is not taken: {e}");
                        record_service(path, record, svc, seen)?;
                        report.skipped.push(format!("{svc}: {e}"));
                        continue;
                    }
                }
                after_pull(phone, form);
                // As the phone keeps it now (written back, it may read differently from the copy)
                let kept = vault_form::carried(form.carry, phone.store.as_ref())
                    .map_err(local)?
                    .map(|m| vault_form::hash(&m))
                    .unwrap_or_default();
                let entry = Synced {
                    mine: kept,
                    theirs: t.hash.clone(),
                    lineage: t.lineage.clone(),
                };
                record_service(path, record, svc, Some(entry))?;
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
                let entry = Synced {
                    mine: vault_form::hash(travels),
                    theirs: hash_of(&s),
                    lineage: Some(lineage.clone()),
                };
                // Once written, the vault holds the phone's tokens: noted first (a write that
                // fails or is cut off may still have landed, so the note stays then)
                let step = UnderWay {
                    then: entry.clone(),
                    write: true,
                    before: expect.clone(),
                };
                mark_under_way(path, record, svc, step)?;
                if side.write_if(svc, &s, marker, expect).await? {
                    // The id with the copy, unless the phone's sign-in changed during the write: its
                    // own new id stays, and the next pass (what was written is recorded) writes it
                    apply_if_still(&phone.store, form, mine_hash.as_deref(), None, Some(&lineage))?;
                    record_service(path, record, svc, Some(entry))?;
                    report.pushed.push(svc);
                } else {
                    record_service(path, record, svc, seen)?;
                    report.skipped.push(format!("{svc}: changed meanwhile"));
                }
            }
            Do::Remove => {
                let expect = theirs.as_ref().map(|t| t.hash.clone());
                if side.remove_if(svc, expect).await? {
                    record_service(path, record, svc, None)?;
                    let_id_go(phone, form, here.as_deref());
                    report.removed.push(svc);
                }
            }
            Do::DropHere => {
                if !apply_if_still(&phone.store, form, mine_hash.as_deref(), Some(&Map::new()), None)? {
                    report.skipped.push(format!("{svc}: changed meanwhile"));
                    continue;
                }
                record_service(path, record, svc, None)?;
                report.dropped.push(svc);
            }
        }
    }
    if whole && record.marker != marker {
        record.marker = marker;
        save_record(path, record)?;
    }
    Ok(report)
}

/// A signed-out sign-in's id let go (`here`: the phone's; unless signed in again meanwhile), once
/// neither side has it: kept, it would only remove that copy again (best effort).
fn let_id_go(phone: &Phone, form: &Form, here: Option<&str>) {
    if here.is_none() {
        return;
    }
    if let Err(e) = apply_if_still(&phone.store, form, None, None, None) {
        tracing::warn!("phone mode: vault: {}: {e}", form.service);
    }
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
    let Some(_turn) = vault_turn(phone).await else {
        return Ok(Renewal::Busy);
    };
    renew_tidal_in_turn(phone, acct).await
}

/// The phone's turn at the vault (`Phone::vault_lock`: one vault step at a time, a pass or a TIDAL
/// renewal, through the vault or on the phone), waited for behind a pass at most [`LOCK_WAIT`] (a
/// stalled pass ends by its own limit); `None`: not now, the phone keeps its session for now.
pub(crate) async fn vault_turn(phone: &Phone) -> Option<tokio::sync::MutexGuard<'_, ()>> {
    tokio::time::timeout(LOCK_WAIT, phone.vault_lock.lock())
        .await
        .ok()
}

/// [`renew_tidal`] in the phone's turn at the vault ([`vault_turn`], held by the caller).
pub(crate) async fn renew_tidal_in_turn(
    phone: &std::sync::Arc<Phone>,
    acct: &Account,
) -> Result<Renewal, VaultStoreError> {
    let svc = TIDAL_FULL.service;
    let path = record_path(phone);
    let Some(mine) = vault_form::carried(TIDAL_FULL.carry, phone.store.as_ref()).map_err(local)? else {
        return Ok(Renewal::NotInVault);
    };
    // Can't be read: an error (a lasting one: the phone renews on its own, as without a vault)
    let mut record = load_record(&path)?;
    let in_record =
        record.services.get(svc).map(|s| s.mine.as_str()) == Some(vault_form::hash(&mine).as_str());
    if !in_record && !record.under_way.contains_key(svc) {
        return Ok(Renewal::NotInVault);
    }
    // "Sign out everywhere" as the last sync saw it
    let marker = record.marker;
    let d = dir(phone);
    let secret = pm_pubky::vault::load_or_create_client_secret(&d.join("client.key")).map_err(local)?;
    let files = (d.join("revoked.json"), d.join("marks.json"));
    let vault = vault_of(acct, &secret, &files);
    if !in_record {
        // A take, a write or a renewal of TIDAL was under way when a pass or a renewal was cut
        // short (or its record not saved): the vault may hold the phone's tokens though the
        // record doesn't say so. The vault is asked first; only a copy without the phone's refresh
        // token is not in step (a homeserver that doesn't answer: never renewed beside it)
        if !holds_phones_token(&vault, &mine).await? {
            return Ok(Renewal::NotInVault);
        }
    }
    // Under way until its outcome is recorded: a renewal cut short, or one whose record can't be
    // saved, leaves the next renewal asking the vault first (best effort: not noted, a renewal
    // here is as before). A step a pass left under way stays as it is
    let marked = !record.under_way.contains_key(svc) && {
        let step = UnderWay {
            then: Synced {
                lineage: record.services.get(svc).and_then(|s| s.lineage.clone()),
                ..Default::default()
            },
            ..Default::default()
        };
        mark_under_way(&path, &mut record, svc, step)
            .inspect_err(|e| tracing::warn!("phone mode: {e}"))
            .unwrap_or(false)
    };
    // Nothing written to the vault after all: not under way any more
    let unmark = || {
        if marked {
            let r = load_record(&path).and_then(|mut r| {
                r.under_way.remove(svc);
                save_record(&path, &r)
            });
            if let Err(e) = r {
                tracing::warn!("phone mode: {e}");
            }
        }
    };
    let p = phone.clone();
    // Whether TIDAL was asked, and its own refusal (not a failure on the way), kept apart from
    // the vault's errors
    let refused = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let refused2 = refused.clone();
    let asked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let asked2 = asked.clone();
    // What the phone made of TIDAL's answer (what travels, in the sign-in it wrote): its own
    // renewal, whatever the vault's copy says after the write
    let made = std::sync::Arc::new(std::sync::Mutex::new(None::<SignIn>));
    let made2 = made.clone();
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
            let fresh = tokio::task::spawn_blocking(move || super::tidal::renew_blocking(&p2, &sess))
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
            let out =
                vault_form::sign_in_for(svc, TIDAL_FULL.carry, true, &travels, Some(&si), app, &lineage);
            *made2.lock().unwrap_or_else(|e| e.into_inner()) = Some(out.clone());
            Ok(out)
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
                // (The vault doesn't have it: held, it is renewed on the phone, never the
                // vault's copy, whose refresh token it replaced)
                Some(travels) => {
                    keep_renewed(phone, &mine, &travels, vault_form::node_lineage(&sign_in), false)
                        .unwrap_or(Renewal::Renewed)
                }
                // (Never: the phone made this sign-in from such a copy)
                None => Renewal::Unclear("the renewed sign-in isn't one this phone can keep".into()),
            });
        }
        Err(e @ VaultStoreError::Refresh(_)) => {
            return match refused.lock().unwrap_or_else(|e| e.into_inner()).take() {
                Some(why) => {
                    unmark();
                    Ok(Renewal::Refused(why))
                }
                // TIDAL was asked and didn't answer cleanly: it may have renewed
                None if asked.load(std::sync::atomic::Ordering::SeqCst) => {
                    Ok(Renewal::Unclear(e.to_string()))
                }
                // The vault's copy is not one this phone renews (another app, no session):
                // the phone renews its own
                None => {
                    unmark();
                    Ok(Renewal::NotInVault)
                }
            };
        }
        // After TIDAL was asked (its new tokens dropped: the sign-in changed hands meanwhile)
        Err(e) if asked.load(std::sync::atomic::Ordering::SeqCst) => {
            return Ok(Renewal::Unclear(e.to_string()))
        }
        Err(e) => {
            unmark();
            return Err(e);
        }
    };
    let made = made.lock().unwrap_or_else(|e| e.into_inner()).take();
    let asked = asked.load(std::sync::atomic::Ordering::SeqCst);
    let (travels, lineage) = match after_refresh(phone, &r, made.as_ref(), asked, marker) {
        Ok(t) => t,
        Err(other) => {
            if other == Renewal::NotInVault {
                unmark();
            }
            return Ok(other);
        }
    };
    let si = r.sign_in();
    // Taken only if the phone still holds the session it started from (a sign-out or a new
    // sign-in here meanwhile wins), with the id of the sign-in it takes. Not taken (changed
    // meanwhile, held, or unclear): the record stays as it was, and the next pass sees the
    // vault's copy as renewed there
    if let Some(other) = keep_renewed(phone, &mine, &travels, lineage, true) {
        return Ok(other);
    }
    // (Signed out just after it was taken: the record stays, the next pass sees it)
    let Ok(Some(kept)) = vault_form::carried(TIDAL_FULL.carry, phone.store.as_ref()) else {
        return Ok(Renewal::Renewed);
    };
    let kept = vault_form::hash(&kept);
    // (Read before the renewal under the same lock; should it not read now, it stays as it is)
    // (Not saved: logged, not an error, as TIDAL answered; the renewal stays under way, so the next
    // one asks the vault first)
    let saved = load_record(&path).and_then(|mut record| {
        let entry = Synced {
            mine: kept,
            theirs: hash_of(si),
            lineage: vault_form::node_lineage(si),
        };
        record_service(&path, &mut record, svc, Some(entry))
    });
    if let Err(e) = saved {
        tracing::warn!("phone mode: {e}");
    }
    if matches!(r, Refreshed::ByUs(_)) {
        tracing::info!("phone mode: TIDAL renewed under the vault's lock");
    }
    Ok(Renewal::Renewed)
}

/// What travels of the TIDAL sign-in a refresh under the vault's lock gave, and its id (`r`; `made`: the one
/// the phone made of TIDAL's answer, when it renewed; `asked`: TIDAL was asked), or what the phone
/// does instead. Its own renewal (`ByUs`) is the session it made, whatever the vault's copy shows
/// after the write; once TIDAL was asked, nothing sends the phone to renew on its own (TIDAL may
/// have spent the refresh token it holds): a copy it can't take is `Unclear`. Before that, a copy
/// another device renewed or wrote that the phone can't take (another app, not a node's) is
/// `NotInVault`: the phone renews its own.
fn after_refresh(
    phone: &Phone,
    r: &Refreshed,
    made: Option<&SignIn>,
    asked: bool,
    marker: Option<u64>,
) -> Result<(Map<String, Value>, Option<String>), Renewal> {
    if let Refreshed::ByUs(si) = r {
        // (The vault's copy as written: the phone's own, unless another client's members came
        // in; its own renewal is what it made either way)
        let own = made.or(Some(si));
        return own
            .and_then(|s| Some((vault_form::node_settings(s)?, vault_form::node_lineage(s))))
            .or_else(|| Some((vault_form::node_settings(si)?, vault_form::node_lineage(si))))
            .ok_or_else(|| Renewal::Unclear("the renewed sign-in isn't one this phone can keep".into()));
    }
    let si = r.sign_in();
    match vault_form::node_settings(si).filter(|_| usable(phone, &TIDAL_FULL, si, marker)) {
        Some(t) => Ok((t, vault_form::node_lineage(si))),
        // TIDAL was asked, and another device's copy stood instead of the phone's renewal
        None if asked => Err(Renewal::Unclear(
            "renewed, but the vault kept another device's sign-in this phone can't use".into(),
        )),
        // One renewed by another device with another app is not taken here
        None => Err(Renewal::NotInVault),
    }
}

/// The TIDAL session a renewal under the vault's lock gave (`travels`, the sign-in's `lineage`)
/// put in the phone's sign-ins, only over the one it started from (`mine`). `None`: saved.
/// Once TIDAL answered, nothing here lets the phone renew again on its own with the session
/// before (no error, no `NotInVault`): a sign-in or sign-out made here meanwhile wins
/// (`Renewed`: the phone uses what it holds now); a save that failed keeps the new session in
/// memory ([`super::tidal::hold_unsaved`], `Renewed`; `in_vault`: the vault holds it too), or,
/// without a session in it, is `Unclear`.
fn keep_renewed(
    phone: &Phone,
    mine: &Map<String, Value>,
    travels: &Map<String, Value>,
    lineage: Option<String>,
    in_vault: bool,
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
                    super::tidal::hold_unsaved(phone, from, fresh, in_vault);
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
    // (Or a step of it under way: the vault may hold the phone's tokens)
    load_record(&record_path(phone)).is_ok_and(|r| {
        r.services.get(TIDAL_FULL.service).map(|s| s.mine.as_str()) == Some(vault_form::hash(&mine).as_str())
            || r.under_way.contains_key(TIDAL_FULL.service)
    })
}

/// Whether the vault's copy of TIDAL (read within [`READ_FOR`]: past it `Busy`, so the phone
/// doesn't renew beside it) carries the refresh token of the phone's session (`mine`).
async fn holds_phones_token(side: &impl Side, mine: &Map<String, Value>) -> Result<bool, VaultStoreError> {
    let svc = TIDAL_FULL.service;
    match tokio::time::timeout(READ_FOR, side.read(svc)).await {
        Err(_) => Err(VaultStoreError::Busy(svc.into())),
        Ok(r) => Ok(r?.is_some_and(|si| same_refresh_token(&si, mine))),
    }
}

/// Whether the vault's copy `si` carries the refresh token of the phone's TIDAL session (`mine`,
/// what travels): renewed by either, it would be spent for the other.
fn same_refresh_token(si: &SignIn, mine: &Map<String, Value>) -> bool {
    let token = |m: &Map<String, Value>| {
        vault_form::session_of(TIDAL_FULL.carry, m)
            .and_then(|s| s.get("refresh_token").and_then(Value::as_str).map(str::to_string))
    };
    let theirs = vault_form::node_settings(si).and_then(|m| token(&m));
    theirs.is_some() && theirs == token(mine)
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
    save_record(&record_path(p), &r).unwrap();
}

/// Tests: the vault as a map of copies, for [`services`]; a service in `stalled` never answers a
/// write or a removal (a pass cut off there by its time limit).
#[cfg(test)]
#[derive(Default)]
pub(crate) struct StandIn {
    pub copies: std::sync::Mutex<BTreeMap<String, SignIn>>,
    pub stalled: std::sync::Mutex<std::collections::BTreeSet<String>>,
    /// Services whose write lands, and then never answers.
    pub lands_then_stalls: std::sync::Mutex<std::collections::BTreeSet<String>>,
    /// Services whose copy can't be read (not shared with the phone, say).
    pub unreadable: std::sync::Mutex<std::collections::BTreeSet<String>>,
}

#[cfg(test)]
impl StandIn {
    async fn stall(&self, service: &str) {
        if self.stalled.lock().unwrap().contains(service) {
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(test)]
impl Side for StandIn {
    async fn read(&self, service: &str) -> Result<Option<SignIn>, VaultStoreError> {
        if self.unreadable.lock().unwrap().contains(service) {
            return Err(VaultStoreError::NotOurs(service.into()));
        }
        Ok(self.copies.lock().unwrap().get(service).cloned())
    }
    async fn write_if(
        &self,
        service: &str,
        s: &SignIn,
        _marker: Option<u64>,
        expect: Option<String>,
    ) -> Result<bool, VaultStoreError> {
        self.stall(service).await;
        {
            let mut c = self.copies.lock().unwrap();
            if c.get(service).map(hash_of) != expect {
                return Ok(false);
            }
            c.insert(service.into(), s.clone());
        }
        if self.lands_then_stalls.lock().unwrap().contains(service) {
            std::future::pending::<()>().await;
        }
        Ok(true)
    }
    async fn remove_if(&self, service: &str, expect: Option<String>) -> Result<bool, VaultStoreError> {
        self.stall(service).await;
        let mut c = self.copies.lock().unwrap();
        if c.get(service).map(hash_of) != expect {
            return Ok(false);
        }
        c.remove(service);
        Ok(true)
    }
}

/// Tests: a node's copy of `form` holding `session` (its `session` member), made with `app`,
/// with the sign-in id `lineage`.
#[cfg(test)]
pub(crate) fn node_copy(form: &Form, session: Value, app: Option<&str>, lineage: &str) -> SignIn {
    let mut travels = Map::new();
    travels.insert(
        form.carry[0].key.into(),
        serde_json::json!({ "session": session }),
    );
    vault_form::sign_in_for(
        form.service,
        form.carry,
        true,
        &travels,
        None,
        app.map(str::to_string),
        lineage,
    )
}

/// Tests: [`services`] on the phone's own record, as [`pass`] runs it.
#[cfg(test)]
pub(crate) async fn services_here(phone: &Phone, side: &StandIn) -> Result<Report, VaultStoreError> {
    let path = record_path(phone);
    std::fs::create_dir_all(dir(phone)).map_err(local)?;
    let mut record = load_record(&path)?;
    services(phone, side, &path, &mut record, None).await
}

/// Tests: TIDAL's copy `copy` taken here (with its id), the take still under way in the record (as
/// a pass leaves it when the record couldn't be saved after the take, or the app ended there).
#[cfg(test)]
pub(crate) fn taken_unrecorded(phone: &Phone, copy: &SignIn) {
    let travels = vault_form::node_settings(copy).unwrap();
    let lineage = vault_form::node_lineage(copy);
    assert!(apply_if_still(
        &phone.store,
        &TIDAL_FULL,
        None,
        Some(&travels),
        lineage.as_deref()
    )
    .unwrap());
    after_pull(phone, &TIDAL_FULL);
    std::fs::create_dir_all(dir(phone)).unwrap();
    let mut r = Record::default();
    let then = Synced {
        mine: vault_form::hash(&travels),
        theirs: hash_of(copy),
        lineage,
    };
    r.under_way.insert(
        TIDAL_FULL.service.into(),
        UnderWay {
            then,
            ..Default::default()
        },
    );
    save_record(&record_path(phone), &r).unwrap();
}

/// Tests: a Qobuz copy the phone signed out of since the last sync: a pass removes it from the
/// vault (after TIDAL), and with `stall`, that removal never answers.
#[cfg(test)]
pub(crate) fn qobuz_removal_stalls(p: &Phone, vault: &StandIn) {
    qobuz_to_remove(p, vault);
    vault.stalled.lock().unwrap().insert(QOBUZ.service.into());
}

/// Tests: [`qobuz_removal_stalls`], answering.
#[cfg(test)]
pub(crate) fn qobuz_to_remove(p: &Phone, vault: &StandIn) {
    let q = node_copy(
        &QOBUZ,
        serde_json::json!({"token": "q"}),
        None,
        "2000-0123456789abcdef",
    );
    std::fs::create_dir_all(dir(p)).unwrap();
    let mut r = Record::default();
    r.services.insert(
        QOBUZ.service.into(),
        Synced {
            mine: "gone".into(),
            theirs: hash_of(&q),
            lineage: vault_form::node_lineage(&q),
        },
    );
    save_record(&record_path(p), &r).unwrap();
    vault.copies.lock().unwrap().insert(QOBUZ.service.into(), q);
}

/// Tests: the phone's record of the last sync.
#[cfg(test)]
pub(crate) fn record_here(phone: &Phone) -> Record {
    load_record(&record_path(phone)).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TIDAL signed in on the phone (`token`), with its own new id.
    fn signed_in_here(p: &Phone, token: &str) {
        let token = token.to_string();
        assert_eq!(
            sign_in_here(p, TIDAL_FULL.service, |store| {
                pm_sources::tidal::full::update_if(store, |st| {
                    st.enabled = true;
                    st.session = Some(pm_sources::tidal::full::Session {
                        access_token: token.clone(),
                        refresh_token: Some(format!("rt-{token}")),
                        expires_at: pm_sources::now_unix() + 3600,
                        country: "CH".into(),
                        ..Default::default()
                    });
                    true
                })
            }),
            Ok(true)
        );
    }

    #[tokio::test]
    async fn a_write_cut_off_after_it_landed_never_goes_over_a_newer_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let p = phone_with_device_client(dir.path());
        std::fs::create_dir_all(dir.path().join("vault")).unwrap();
        signed_in_here(&p, "phone");
        // The write lands, and the pass is cut off before it is recorded
        let vault = StandIn::default();
        vault
            .lands_then_stalls
            .lock()
            .unwrap()
            .insert(TIDAL_FULL.service.into());
        tokio::time::pause();
        let cut = tokio::time::timeout(std::time::Duration::from_secs(120), services_here(&p, &vault)).await;
        assert!(cut.is_err());
        assert!(
            vault.copies.lock().unwrap().contains_key(TIDAL_FULL.service),
            "landed"
        );
        assert!(
            !record_here(&p).services.contains_key(TIDAL_FULL.service),
            "not recorded"
        );
        vault.lands_then_stalls.lock().unwrap().clear();
        // Then the person signs in anew on their node: that one is taken here, never written over
        vault.copies.lock().unwrap().insert(
            TIDAL_FULL.service.into(),
            tidal_copy("node", "3000-0123456789abcdef"),
        );
        let r = services_here(&p, &vault).await.unwrap();
        assert!(r.pushed.is_empty(), "{r:?}");
        assert_eq!(r.pulled, vec![TIDAL_FULL.service], "{r:?}");
        assert_eq!(tidal_here(&p).as_deref(), Some("node"));
        assert!(record_here(&p).under_way.is_empty());
    }

    #[tokio::test]
    async fn a_write_cut_off_before_it_landed_is_written_again() {
        let dir = tempfile::tempdir().unwrap();
        let p = phone_with_device_client(dir.path());
        std::fs::create_dir_all(dir.path().join("vault")).unwrap();
        signed_in_here(&p, "phone");
        let vault = StandIn::default();
        vault.stalled.lock().unwrap().insert(TIDAL_FULL.service.into());
        tokio::time::pause();
        let cut = tokio::time::timeout(std::time::Duration::from_secs(120), services_here(&p, &vault)).await;
        assert!(cut.is_err());
        assert!(vault.copies.lock().unwrap().is_empty(), "never landed");
        vault.stalled.lock().unwrap().clear();
        let r = services_here(&p, &vault).await.unwrap();
        assert_eq!(r.pushed, vec![TIDAL_FULL.service], "{r:?}");
    }

    #[tokio::test]
    async fn a_take_the_record_missed_follows_a_sign_out_there() {
        let dir = tempfile::tempdir().unwrap();
        let p = phone_with_device_client(dir.path());
        // Taken, the record not saved after it; then the person signs out of TIDAL on the node
        taken_unrecorded(&p, &tidal_copy("a", "1000-0123456789abcdef"));
        let vault = StandIn::default();
        let r = services_here(&p, &vault).await.unwrap();
        assert!(r.pushed.is_empty(), "never written back: {r:?}");
        assert_eq!(r.dropped, vec![TIDAL_FULL.service], "{r:?}");
        assert!(vault.copies.lock().unwrap().is_empty());
        assert_eq!(tidal_here(&p), None);
    }

    #[tokio::test]
    async fn a_sign_out_on_both_sides_lets_the_id_go() {
        let dir = tempfile::tempdir().unwrap();
        let p = phone_with_device_client(dir.path());
        let vault = StandIn::default();
        vault.copies.lock().unwrap().insert(
            TIDAL_FULL.service.into(),
            tidal_copy("a", "1000-0123456789abcdef"),
        );
        services_here(&p, &vault).await.unwrap();
        // Signed out here, and the vault's copy gone too before the next pass
        super::super::tidal::sign_out(&p).unwrap();
        vault.copies.lock().unwrap().clear();
        let r = services_here(&p, &vault).await.unwrap();
        assert_eq!(r, Report::default());
        assert!(!record_here(&p).services.contains_key(TIDAL_FULL.service));
        assert_eq!(
            SessionStore::get(p.store.as_ref(), &lineage_key(TIDAL_FULL.service)),
            None
        );
        assert!(!changed_here(&p, &TIDAL_FULL));
    }

    #[tokio::test]
    async fn sign_out_everywhere_is_recorded_only_once_every_copy_was_read() {
        let dir = tempfile::tempdir().unwrap();
        let p = phone_with_device_client(dir.path());
        std::fs::create_dir_all(dir.path().join("vault")).unwrap();
        let vault = StandIn::default();
        vault.unreadable.lock().unwrap().insert(QOBUZ.service.into());
        let path = record_path(&p);
        let mut record = load_record(&path).unwrap();
        services(&p, &vault, &path, &mut record, Some(5)).await.unwrap();
        assert_eq!(record_here(&p).marker, None);
        vault.unreadable.lock().unwrap().clear();
        services(&p, &vault, &path, &mut record, Some(5)).await.unwrap();
        assert_eq!(record_here(&p).marker, Some(5));
    }

    #[tokio::test]
    async fn the_vault_holds_the_phones_tidal_only_with_its_refresh_token() {
        let mine = |rt: &str| {
            let mut m = Map::new();
            m.insert(
                "tidal.unofficial".into(),
                serde_json::json!({"session": {"access_token": "x", "refresh_token": rt, "expires_at": 1, "country": "CH"}}),
            );
            m
        };
        let vault = StandIn::default();
        assert!(!holds_phones_token(&vault, &mine("rt")).await.unwrap(), "no copy");
        vault.copies.lock().unwrap().insert(
            TIDAL_FULL.service.into(),
            tidal_copy("a", "1000-0123456789abcdef"),
        );
        assert!(holds_phones_token(&vault, &mine("rt")).await.unwrap());
        assert!(!holds_phones_token(&vault, &mine("other")).await.unwrap());
        vault.unreadable.lock().unwrap().insert(TIDAL_FULL.service.into());
        assert!(holds_phones_token(&vault, &mine("rt")).await.is_err());
    }

    fn phone_with_device_client(dir: &std::path::Path) -> std::sync::Arc<Phone> {
        Phone::open(super::super::Config {
            dir: dir.to_path_buf(),
            cache: dir.join("streams"),
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
        .unwrap()
    }

    fn tidal_copy(token: &str, lineage: &str) -> SignIn {
        node_copy(
            &TIDAL_FULL,
            serde_json::json!({"access_token": token, "refresh_token": "rt", "expires_at": pm_sources::now_unix() + 3600, "country": "CH"}),
            Some("device-client"),
            lineage,
        )
    }

    fn tidal_here(p: &Phone) -> Option<String> {
        pm_sources::tidal::full::load(p.store.as_ref())
            .session
            .map(|s| s.access_token)
    }

    #[tokio::test]
    async fn a_pass_cut_short_after_taking_tidal_has_recorded_it() {
        let dir = tempfile::tempdir().unwrap();
        let p = phone_with_device_client(dir.path());
        let vault = StandIn::default();
        vault.copies.lock().unwrap().insert(
            TIDAL_FULL.service.into(),
            tidal_copy("a", "1000-0123456789abcdef"),
        );
        // The next step, Qobuz's removal, never answers: the pass ends by its time limit
        qobuz_removal_stalls(&p, &vault);
        tokio::time::pause();
        let cut = tokio::time::timeout(std::time::Duration::from_secs(120), services_here(&p, &vault)).await;
        assert!(cut.is_err(), "cut short at Qobuz");
        assert_eq!(tidal_here(&p).as_deref(), Some("a"), "TIDAL was taken");
        // Recorded before the next step: a renewal goes through the vault, and a sign-out here is
        // one for the vault
        let r = record_here(&p);
        let t = r.services.get(TIDAL_FULL.service).expect("TIDAL's take recorded");
        assert_eq!(t.lineage.as_deref(), Some("1000-0123456789abcdef"));
        assert!(!r.under_way.contains_key(TIDAL_FULL.service));
        assert!(tidal_in_step(&p));
        assert!(
            r.services.contains_key(QOBUZ.service),
            "Qobuz's removal not done: as it was"
        );
        super::super::tidal::sign_out(&p).unwrap();
        assert!(
            changed_here(&p, &TIDAL_FULL),
            "the sign-out goes to the vault at once"
        );
        vault.stalled.lock().unwrap().clear();
        let next = services_here(&p, &vault).await.unwrap();
        assert_eq!(next.removed, vec![TIDAL_FULL.service, QOBUZ.service], "{next:?}");
        assert!(vault.copies.lock().unwrap().is_empty());
        assert_eq!(tidal_here(&p), None, "the sign-out stays");
    }

    #[tokio::test]
    async fn a_sign_out_of_a_take_the_record_missed_is_one_for_the_vault() {
        let dir = tempfile::tempdir().unwrap();
        let p = phone_with_device_client(dir.path());
        let vault = StandIn::default();
        let copy = tidal_copy("a", "1000-0123456789abcdef");
        vault
            .copies
            .lock()
            .unwrap()
            .insert(TIDAL_FULL.service.into(), copy.clone());
        // Taken from the vault, and the app ended before the record was saved (it has no TIDAL)
        let travels = vault_form::node_settings(&copy).unwrap();
        assert!(apply_if_still(
            &p.store,
            &TIDAL_FULL,
            None,
            Some(&travels),
            Some("1000-0123456789abcdef")
        )
        .unwrap());
        std::fs::create_dir_all(dir.path().join("vault")).unwrap();
        save_record(&record_path(&p), &Record::default()).unwrap();
        // Signed out here: never taken again, removed from the vault
        super::super::tidal::sign_out(&p).unwrap();
        assert!(changed_here(&p, &TIDAL_FULL));
        let r = services_here(&p, &vault).await.unwrap();
        assert_eq!(r.removed, vec![TIDAL_FULL.service], "{r:?}");
        assert!(r.pulled.is_empty(), "{r:?}");
        assert_eq!(tidal_here(&p), None);
        assert!(vault.copies.lock().unwrap().is_empty());
        // Its id let go with it: nothing more to sync
        assert_eq!(
            SessionStore::get(p.store.as_ref(), &lineage_key(TIDAL_FULL.service)),
            None
        );
        assert!(!changed_here(&p, &TIDAL_FULL));
        // A new sign-in there is still taken
        vault.copies.lock().unwrap().insert(
            TIDAL_FULL.service.into(),
            tidal_copy("b", "3000-0123456789abcdef"),
        );
        let r = services_here(&p, &vault).await.unwrap();
        assert_eq!(r.pulled, vec![TIDAL_FULL.service], "{r:?}");
        assert_eq!(tidal_here(&p).as_deref(), Some("b"));
    }

    #[tokio::test]
    async fn a_record_that_cant_be_saved_stops_the_pass() {
        let dir = tempfile::tempdir().unwrap();
        let p = phone_with_device_client(dir.path());
        let vault = StandIn::default();
        vault.copies.lock().unwrap().insert(
            TIDAL_FULL.service.into(),
            tidal_copy("a", "1000-0123456789abcdef"),
        );
        qobuz_to_remove(&p, &vault);
        let before = std::fs::read(record_path(&p)).unwrap();
        // The record can't be replaced now
        std::fs::create_dir(record_path(&p).with_extension("json.tmp")).unwrap();
        let r = services_here(&p, &vault).await;
        assert!(matches!(r, Err(VaultStoreError::Local(_))), "{r:?}");
        // Nothing taken or removed that the record can't say
        assert_eq!(tidal_here(&p), None);
        assert!(
            vault.copies.lock().unwrap().contains_key(QOBUZ.service),
            "Qobuz not removed"
        );
        assert_eq!(std::fs::read(record_path(&p)).unwrap(), before);
    }

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
        )
        .unwrap();
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
        })
        .unwrap();
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
        // Signed out of the very sign-in the vault has (its id kept), which the record missed
        // (a pass cut short after taking or writing it): out of the vault too
        assert_eq!(
            decide(None, None, Some("L2"), Some(&t("y", "L2", true)), false),
            Do::Remove
        );
        assert_eq!(
            decide(Some(&s), None, Some("L2"), Some(&t("y", "L2", true)), false),
            Do::Remove
        );
        // ... and one signed out of before a new sign-in there isn't
        assert_eq!(
            decide(None, None, Some("L1"), Some(&t("y", "L2", true)), false),
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
    fn a_vault_copy_this_version_cant_read_is_not_taken() {
        let dir = tempfile::tempdir().unwrap();
        let p = phone_here(dir.path());
        tidal_due_in_step(&p);
        let before = std::fs::read(dir.path().join(super::super::store::FILE)).unwrap();
        let mine = vault_form::carried(TIDAL_FULL.carry, p.store.as_ref())
            .unwrap()
            .unwrap();
        // Another version's session (no expiry): kept, TIDAL would read as signed out here and
        // the next start would refuse the file
        let key = TIDAL_FULL.carry[0].key;
        let mut travels = mine.clone();
        let mut held = travels.get(key).and_then(vault_form::object_of).unwrap();
        held.insert("session".into(), serde_json::json!({ "access_token": "other" }));
        travels.insert(key.into(), Value::Object(held));
        let r = apply_if_still(
            &p.store,
            &TIDAL_FULL,
            Some(&vault_form::hash(&mine)),
            Some(&travels),
            None,
        );
        assert!(r.is_err(), "{r:?}");
        assert_eq!(
            std::fs::read(dir.path().join(super::super::store::FILE)).unwrap(),
            before
        );
        assert_eq!(
            pm_sources::tidal::full::load(p.store.as_ref())
                .session
                .unwrap()
                .access_token,
            "due"
        );
    }

    /// The vault's copy rewritten by another device while the phone renewed TIDAL under its lock
    /// (a re-wrap for a new device: a new revision, the same tokens), then the phone's renewal
    /// written over it: the copy stays the phone's (a node's copy, with the new session), and the
    /// phone's own renewal is never one it renews again on its own.
    #[test]
    fn a_renewal_written_over_a_rewritten_copy_is_never_renewed_again() {
        let dir = tempfile::tempdir().unwrap();
        let p = phone_with_device_client(dir.path());
        let lineage = "1000-0123456789abcdef";
        let mut cur = tidal_copy("a0", lineage);
        cur.revision = 3;
        let mut theirs = cur.clone();
        theirs.revision = 4;
        // What the phone makes of TIDAL's answer (as renew_tidal does)
        let fresh_session = serde_json::json!({"access_token": "a1", "refresh_token": "rt-1", "expires_at": pm_sources::now_unix() + 3600, "country": "CH"});
        let mut travels = vault_form::node_settings(&cur).unwrap();
        let key = TIDAL_FULL.carry[0].key;
        let mut held = travels.get(key).and_then(vault_form::object_of).unwrap();
        held.insert("session".into(), fresh_session);
        travels.insert(key.into(), Value::Object(held));
        let made = vault_form::sign_in_for(
            TIDAL_FULL.service,
            TIDAL_FULL.carry,
            true,
            &travels,
            Some(&cur),
            cur.client_id.clone(),
            lineage,
        );
        // The copy the vault writes: still a node's copy, with the new session in it
        let written = pm_pubky::vault::merge_refreshed(&cur, &theirs, &made);
        let kept = vault_form::node_settings(&written).expect("a node's copy");
        let token = |m: &Map<String, Value>| {
            vault_form::session_of(TIDAL_FULL.carry, m).and_then(|s| s.get("access_token").cloned())
        };
        assert_eq!(token(&kept), Some(Value::from("a1")));
        assert!(usable(&p, &TIDAL_FULL, &written, None));
        // Whatever the vault's copy shows after the write (here: the old session's member beside
        // the new tokens), the phone's own renewal is what it made, never NotInVault
        let mut odd = theirs.clone();
        odd.tokens = made.tokens.clone();
        odd.expires_at = made.expires_at;
        let got = after_refresh(&p, &Refreshed::ByUs(odd.clone()), Some(&made), true, None).unwrap();
        assert_eq!(token(&got.0), Some(Value::from("a1")));
        assert_eq!(got.1.as_deref(), Some(lineage));
        // Another device's copy stood instead (TIDAL was asked): never renewed here on its own
        let other = node_copy(
            &TIDAL_FULL,
            serde_json::json!({"access_token": "b", "refresh_token": "rt-b", "expires_at": pm_sources::now_unix() + 3600}),
            Some("another-app"),
            "2000-0123456789abcdef",
        );
        assert!(matches!(
            after_refresh(&p, &Refreshed::ByAnother(other.clone()), Some(&made), true, None),
            Err(Renewal::Unclear(_))
        ));
        // Before TIDAL was asked: the phone renews its own, as before
        assert_eq!(
            after_refresh(&p, &Refreshed::ByAnother(other), None, false, None).unwrap_err(),
            Renewal::NotInVault
        );
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
        assert_eq!(
            keep_renewed(&p, &mine, &travels, None, true),
            Some(Renewal::Renewed)
        );
        assert!(
            p.tidal_unsaved.lock().unwrap().is_some(),
            "held until it can be saved"
        );
        // A sign-out here meanwhile: that wins, and nothing is renewed again either
        std::fs::remove_dir(&file).unwrap();
        std::fs::write(&file, kept).unwrap();
        super::super::tidal::sign_out(&p).unwrap();
        assert_eq!(
            keep_renewed(&p, &mine, &travels, None, true),
            Some(Renewal::Renewed)
        );
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

    #[tokio::test]
    async fn sign_ins_that_cant_be_read_are_set_aside_and_never_a_sign_out() {
        let dir = tempfile::tempdir().unwrap();
        // Signed in to TIDAL and synced with the vault before (the vault's copy is made with an
        // app this phone doesn't renew with: never taken here)
        let p = phone_with_device_client(dir.path());
        tidal_due_in_step(&p);
        let vault = StandIn::default();
        let other = node_copy(
            &TIDAL_FULL,
            serde_json::json!({"access_token": "t", "refresh_token": "rt", "expires_at": 1, "country": "CH"}),
            Some("another-app"),
            "1000-0123456789abcdef",
        );
        let mut r = record_here(&p);
        let e = r.services.get_mut(TIDAL_FULL.service).unwrap();
        e.theirs = hash_of(&other);
        e.lineage = vault_form::node_lineage(&other);
        save_record(&record_path(&p), &r).unwrap();
        vault
            .copies
            .lock()
            .unwrap()
            .insert(TIDAL_FULL.service.into(), other);
        // A Qobuz copy in the vault the phone can take
        vault.copies.lock().unwrap().insert(
            QOBUZ.service.into(),
            node_copy(
                &QOBUZ,
                serde_json::json!({"token": "q"}),
                None,
                "2000-0123456789abcdef",
            ),
        );
        drop(p);
        // The sign-ins file can't be read now: phone mode starts, both set aside, the file kept
        let file = dir.path().join(super::super::store::FILE);
        let cut = br#"{"tidal.unofficial":"#;
        std::fs::write(&file, cut).unwrap();
        let p = phone_with_device_client(dir.path());
        let told = p.set_aside_notice().expect("the person is told");
        assert!(told.starts_with("Your streaming sign-ins on this phone couldn't be read. The file was kept as sign-ins.json.20"), "{told}");
        assert!(!told.contains("  "), "{told}");
        // Until they saw it (Continue on the connect page)
        assert!(p.set_aside_notice().is_some());
        p.set_aside_seen().unwrap();
        assert_eq!(p.set_aside_notice(), None);
        let name = p.store.set_aside().1.unwrap().to_string();
        assert_eq!(std::fs::read(dir.path().join(&name)).unwrap(), cut);
        assert_eq!(tidal_here(&p), None, "signed out on this phone");
        // The sync never removes TIDAL from the vault for it, nor writes it; the vault's Qobuz
        // copy is taken (a sign-in again)
        let r = services_here(&p, &vault).await.unwrap();
        assert!(r.removed.is_empty(), "{r:?}");
        assert!(vault.copies.lock().unwrap().contains_key(TIDAL_FULL.service));
        assert_eq!(r.pulled, vec![QOBUZ.service], "{r:?}");
        assert!(!set_aside(p.store.as_ref(), &QOBUZ));
        assert!(set_aside(p.store.as_ref(), &TIDAL_FULL));
        // Still held at the next start (until a sign-in); the person isn't told again
        drop(p);
        let p = phone_with_device_client(dir.path());
        assert_eq!(p.set_aside_notice(), None, "seen");
        let r = services_here(&p, &vault).await.unwrap();
        assert!(r.removed.is_empty() && r.pushed.is_empty(), "{r:?}");
        // Signed in again here: synced as a sign-in made here
        assert_eq!(
            sign_in_here(&p, TIDAL_FULL.service, |store| {
                pm_sources::tidal::full::update_if(store, |st| {
                    st.session = Some(pm_sources::tidal::full::Session {
                        access_token: "new".into(),
                        expires_at: pm_sources::now_unix() + 3600,
                        country: "CH".into(),
                        ..Default::default()
                    });
                    true
                })
            }),
            Ok(true)
        );
        assert!(!set_aside(p.store.as_ref(), &TIDAL_FULL));
        let r = services_here(&p, &vault).await.unwrap();
        assert_eq!(r.pushed, vec![TIDAL_FULL.service], "{r:?}");
    }

    #[tokio::test]
    async fn a_sign_in_set_aside_alone_keeps_its_id_and_is_never_removed() {
        let dir = tempfile::tempdir().unwrap();
        let p = phone_with_device_client(dir.path());
        // TIDAL taken from the vault and recorded
        let vault = StandIn::default();
        let copy = tidal_copy("a", "1000-0123456789abcdef");
        vault
            .copies
            .lock()
            .unwrap()
            .insert(TIDAL_FULL.service.into(), copy);
        assert_eq!(
            services_here(&p, &vault).await.unwrap().pulled,
            vec![TIDAL_FULL.service]
        );
        drop(p);
        // Its entry in the file is cut now (its id, beside it, reads): set aside alone
        let file = dir.path().join(super::super::store::FILE);
        let mut v: BTreeMap<String, String> = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        v.insert("tidal.unofficial".into(), "{".into());
        std::fs::write(&file, serde_json::to_vec(&v).unwrap()).unwrap();
        let p = phone_with_device_client(dir.path());
        assert!(p
            .set_aside_notice()
            .unwrap()
            .starts_with("Your TIDAL sign-in on this phone"));
        assert_eq!(
            SessionStore::get(p.store.as_ref(), &lineage_key(TIDAL_FULL.service)).as_deref(),
            Some("1000-0123456789abcdef")
        );
        assert!(changed_here(&p, &TIDAL_FULL));
        // Never read as a sign-out of the vault's very sign-in: nothing removed, and the vault's
        // copy (one this phone can use) is taken back
        let r = services_here(&p, &vault).await.unwrap();
        assert!(r.removed.is_empty(), "{r:?}");
        assert_eq!(r.pulled, vec![TIDAL_FULL.service], "{r:?}");
        assert!(vault.copies.lock().unwrap().contains_key(TIDAL_FULL.service));
        assert_eq!(tidal_here(&p).as_deref(), Some("a"));
        assert!(!set_aside(p.store.as_ref(), &TIDAL_FULL));
        assert_eq!(p.set_aside_notice(), None, "signed in again");
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
