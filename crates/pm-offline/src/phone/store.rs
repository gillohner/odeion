//! What phone mode keeps on the phone for its one person: the streaming sign-ins
//! (`tidal.unofficial`, `qobuz.unofficial`) and Qobuz's web-player app values (`qobuz.app`), as
//! one JSON file in the app's own storage (`phone/sign-ins.json`, readable only by the app).
//! Every change is written whole to a temporary file and renamed over the old one, and only once
//! it is there does the phone use it: a change that couldn't be saved didn't happen.

use pm_sources::qobuz::{App, AppCache};
use pm_sources::{SessionStore, SourceError};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

pub const FILE: &str = "sign-ins.json";
/// Where the Qobuz app values are kept (as on the node).
const APP_KEY: &str = pm_sources::qobuz::APP_KEY;

/// Who hears of a changed key.
type Hook = Box<dyn Fn(&str) + Send + Sync>;

pub struct FileStore {
    path: PathBuf,
    values: Mutex<BTreeMap<String, String>>,
    /// Told after every change (phase 3: the vault hears of new sign-ins).
    changed: Mutex<Option<Hook>>,
    /// The sign-ins set aside when this store was opened (they couldn't be read), and the copy
    /// of the file kept with them (its name in the store's folder).
    set_aside: Vec<String>,
    kept_as: Option<String>,
}

/// The streaming sign-ins the file holds, each read by its service.
const SIGN_INS: [&str; 2] = [pm_sources::tidal::full::STATE_KEY, pm_sources::qobuz::STATE_KEY];

/// Where the store notes that the sign-in under `key` was set aside at start (the value: the copy
/// of the file kept with it): until the person signs in to it again (or the vault's copy is taken),
/// the vault sync writes nothing of it and never removes it from the vault (`vault::services`).
pub(crate) fn set_aside_key(key: &str) -> String {
    format!("vault.set_aside.{key}")
}

impl FileStore {
    /// The store in `dir` (made if missing); with no file yet, empty.
    ///
    /// A file that isn't the store's JSON is never read as empty, and a sign-in in it that isn't
    /// its service's JSON is never read as signed out: either would be a sign-out the vault sync
    /// carries to the vault. The file is kept whole under a dated name instead
    /// (`sign-ins.json.<date>.unreadable`, beside it, never deleted), and the store starts
    /// without what couldn't be read (a value that doesn't read: only that one), each such
    /// sign-in noted as set aside (`vault.set_aside.<key>`): signed out on this phone, and held out of
    /// the vault sync until the person signs in to it again. A file that can't be read at all (a
    /// read error), or a copy that can't be kept, is an error, and the file is left as it is.
    pub fn open(dir: &std::path::Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(FILE);
        let bytes = match std::fs::read(&path) {
            Ok(b) => Some(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(unreadable(&path, e)),
        };
        let mut values = BTreeMap::new();
        // The sign-ins that don't read, and whether anything else was left out
        let (mut aside, mut left_out) = (Vec::new(), false);
        match bytes
            .as_deref()
            .map(serde_json::from_slice::<BTreeMap<String, serde_json::Value>>)
        {
            None => {}
            Some(Ok(all)) => {
                for (k, v) in all {
                    let read = match &v {
                        serde_json::Value::String(x) => sign_in_reads(&k, x).map(|()| x.clone()),
                        _ => Err("not a string".to_string()),
                    };
                    match read {
                        Ok(x) => {
                            values.insert(k, x);
                        }
                        Err(e) => {
                            tracing::error!("phone mode: {k} in {} can't be read ({e})", path.display());
                            if SIGN_INS.contains(&k.as_str()) {
                                aside.push(k);
                            } else {
                                left_out = true;
                            }
                        }
                    }
                }
            }
            Some(Err(e)) => {
                tracing::error!(
                    "phone mode: the sign-ins in {} can't be read ({})",
                    path.display(),
                    where_in(&e)
                );
                aside = SIGN_INS.map(String::from).to_vec();
            }
        }
        let mut store = Self {
            path,
            values: Mutex::new(BTreeMap::new()),
            changed: Mutex::new(None),
            set_aside: Vec::new(),
            kept_as: None,
        };
        if let (Some(bytes), true) = (&bytes, !aside.is_empty() || left_out) {
            // The file kept whole first (on disk before anything replaces it), then the one
            // without what couldn't be read put in its place in one step: a crash in between
            // leaves the file as it was, set aside again at the next start
            let kept = keep_aside(&store.path, bytes).map_err(|e| unreadable(&store.path, e))?;
            for k in &aside {
                values.insert(set_aside_key(k), kept.clone());
            }
            if let Err(e) = store.save(&values) {
                // (Only the copy just made goes: the file is still there as it was)
                let _ = std::fs::remove_file(store.path.with_file_name(&kept));
                return Err(unreadable(&store.path, e));
            }
            sync_dir(&store.path);
            tracing::warn!(
                "phone mode: what couldn't be read in the sign-ins ({}) was set aside, the file kept \
                 as {kept}: signed out here until signed in again",
                if aside.is_empty() {
                    "no sign-in".to_string()
                } else {
                    aside.join(", ")
                }
            );
            store.set_aside = aside;
            store.kept_as = Some(kept);
        }
        *store.values.get_mut().unwrap_or_else(|e| e.into_inner()) = values;
        Ok(store)
    }

    /// The sign-ins set aside when the store was opened ([`FileStore::open`]: their keys), and the
    /// name the file was kept under.
    pub fn set_aside(&self) -> (&[String], Option<&str>) {
        (&self.set_aside, self.kept_as.as_deref())
    }

    /// Who hears of a changed key.
    pub fn on_change(&self, f: impl Fn(&str) + Send + Sync + 'static) {
        *self.changed.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
    }

    fn save(&self, values: &BTreeMap<String, String>) -> std::io::Result<()> {
        let tmp = self.path.with_extension("json.tmp");
        let body = serde_json::to_vec(values).map_err(std::io::Error::other)?;
        write_private(&tmp, &body)?;
        std::fs::rename(&tmp, &self.path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    fn tell(&self, key: &str) {
        if let Some(f) = self.changed.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            f(key);
        }
    }

    /// Runs `f` on a draft of the values, under the store's lock: no other change lands between
    /// what it reads and what it writes. What it changed is saved (one write) when it returns
    /// `Ok(true)`; with `Ok(false)` or an error nothing changes.
    pub fn change_if(&self, f: impl FnOnce(&Draft) -> Result<bool, String>) -> Result<bool, String> {
        let changed: Vec<String> = {
            let mut v = self.values.lock().unwrap_or_else(|e| e.into_inner());
            let d = Draft(Mutex::new(v.clone()));
            if !f(&d)? {
                return Ok(false);
            }
            let new = d.0.into_inner().unwrap_or_else(|e| e.into_inner());
            let changed = v
                .keys()
                .chain(new.keys())
                .filter(|k| v.get(*k) != new.get(*k))
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            self.save(&new).map_err(|e| e.to_string())?;
            *v = new;
            changed
        };
        for k in changed {
            self.tell(&k);
        }
        Ok(true)
    }

    /// Sets (or with `None` removes) a key: saved first, then used (an error, and nothing
    /// changed, when it can't be saved).
    pub fn put(&self, key: &str, value: Option<&str>) -> std::io::Result<()> {
        {
            let mut v = self.values.lock().unwrap_or_else(|e| e.into_inner());
            let mut new = v.clone();
            match value {
                Some(x) => new.insert(key.to_string(), x.to_string()),
                None => new.remove(key),
            };
            self.save(&new)?;
            *v = new;
        }
        self.tell(key);
        Ok(())
    }
}

/// Whether `value` under `key` is what its service reads: a streaming sign-in (`tidal.unofficial`,
/// `qobuz.unofficial`) that isn't its state's JSON would read as signed out. Any other key: yes.
pub(crate) fn sign_in_reads(key: &str, value: &str) -> Result<(), String> {
    match key {
        pm_sources::tidal::full::STATE_KEY => {
            serde_json::from_str::<pm_sources::tidal::full::ModeState>(value)
                .map(|_| ())
                .map_err(|e| format!("TIDAL: {}", where_in(&e)))
        }
        pm_sources::qobuz::STATE_KEY => serde_json::from_str::<pm_sources::qobuz::ModeState>(value)
            .map(|_| ())
            .map_err(|e| format!("Qobuz: {}", where_in(&e))),
        _ => Ok(()),
    }
}

/// Where a sign-in doesn't read, never what it read (the parser quotes values: a token, say).
fn where_in(e: &serde_json::Error) -> String {
    let what = match e.classify() {
        serde_json::error::Category::Io => "not read",
        serde_json::error::Category::Syntax => "not JSON",
        serde_json::error::Category::Data => "not its form",
        serde_json::error::Category::Eof => "cut short",
    };
    format!("{what} (line {}, column {})", e.line(), e.column())
}

/// The sign-ins file can't be read and couldn't be set aside (logged): phone mode doesn't start
/// rather than start signed out.
fn unreadable(path: &std::path::Path, e: impl std::fmt::Display) -> std::io::Error {
    tracing::error!(
        "phone mode: the sign-ins in {} can't be read, nor kept aside ({e}); left as they are",
        path.display()
    );
    std::io::Error::other(format!(
        "the sign-ins on this phone can't be read, nor kept aside ({e}); they are left as they are"
    ))
}

/// `bytes` (what `path` holds) kept under a dated name beside it (`<name>.<UTC date>.unreadable`,
/// never one already there), readable by this app's user only and on disk before this returns.
/// Its name.
fn keep_aside(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<String> {
    use std::io::Write;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| std::io::Error::other("no file name"))?;
    let at = utc_stamp(pm_sources::now_unix());
    for n in 0..100 {
        let kept = match n {
            0 => format!("{name}.{at}.unreadable"),
            n => format!("{name}.{at}-{n}.unreadable"),
        };
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let mut f = match o.open(path.with_file_name(&kept)) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let written = f.write_all(bytes).and_then(|()| f.sync_all());
        if let Err(e) = written {
            drop(f);
            let _ = std::fs::remove_file(path.with_file_name(&kept));
            return Err(e);
        }
        sync_dir(path);
        return Ok(kept);
    }
    Err(std::io::Error::other("too many copies kept already"))
}

/// The folder of `path` flushed (its new and renamed names on disk); best effort.
fn sync_dir(path: &std::path::Path) {
    #[cfg(unix)]
    if let Some(d) = path.parent() {
        if let Err(e) = std::fs::File::open(d).and_then(|f| f.sync_all()) {
            tracing::debug!("phone mode: {} not flushed: {e}", d.display());
        }
    }
}

/// `secs` (Unix) as `2026-11-22T09-40-00Z` (a file name's form of the UTC time).
fn utc_stamp(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days to a civil date (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}-{:02}-{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// What the page is told when the sign-ins couldn't be saved (nothing changed).
pub(crate) fn not_saved(e: impl std::fmt::Display) -> SourceError {
    SourceError::new(
        pm_sources::StatusCode::INTERNAL_SERVER_ERROR,
        format!("This phone couldn't save its sign-ins ({e}), so nothing changed: try again"),
    )
}

/// Writes `body` to `path`, readable and writable by this app's user only.
fn write_private(path: &std::path::Path, body: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(path)?;
    f.write_all(body)?;
    f.sync_all()
}

impl SessionStore for FileStore {
    fn get(&self, key: &str) -> Option<String> {
        self.values
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned()
    }

    /// Saved first, then used: a change that can't be saved is left out (and logged; the phone's
    /// own answers go through [`FileStore::change_if`], which says so).
    fn update(&self, key: &str, f: &mut dyn FnMut(Option<String>) -> Option<String>) {
        let wrote = {
            let mut v = self.values.lock().unwrap_or_else(|e| e.into_inner());
            match f(v.get(key).cloned()) {
                Some(value) => {
                    let mut new = v.clone();
                    new.insert(key.to_string(), value);
                    match self.save(&new) {
                        Ok(()) => {
                            *v = new;
                            true
                        }
                        Err(e) => {
                            tracing::warn!("phone mode: can't save the sign-ins, nothing changed: {e}");
                            false
                        }
                    }
                }
                None => false,
            }
        };
        if wrote {
            self.tell(key);
        }
    }
}

impl AppCache for FileStore {
    fn cached(&self) -> Option<App> {
        self.get(APP_KEY)
            .and_then(|v| serde_json::from_str(&v).ok())
            .filter(|a: &App| !a.id.is_empty() && !a.secret.is_empty())
    }

    fn keep(&self, app: &App) -> Result<(), SourceError> {
        let v = serde_json::to_string(app).expect("app serializes");
        self.put(APP_KEY, Some(&v))
            .map_err(|e| SourceError::new(pm_sources::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
    }
}

/// The vault form reads and writes the phone's sign-ins like the node's settings.
impl pm_sources::vault_form::Settings for FileStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(SessionStore::get(self, key))
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.put(key, Some(value)).map_err(|e| e.to_string())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        self.put(key, None).map_err(|e| e.to_string())
    }
}

/// The values as [`FileStore::change_if`] sees them, read and written like the store.
pub struct Draft(Mutex<BTreeMap<String, String>>);

impl Draft {
    pub fn get(&self, key: &str) -> Option<String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).get(key).cloned()
    }
    /// Sets (or with `None` removes) a key.
    pub fn put(&self, key: &str, value: Option<&str>) {
        let mut v = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match value {
            Some(x) => v.insert(key.to_string(), x.to_string()),
            None => v.remove(key),
        };
    }
}

/// The phone's sign-in code writes a draft as it writes the store.
impl SessionStore for Draft {
    fn get(&self, key: &str) -> Option<String> {
        Draft::get(self, key)
    }
    fn update(&self, key: &str, f: &mut dyn FnMut(Option<String>) -> Option<String>) {
        let now = Draft::get(self, key);
        if let Some(new) = f(now) {
            self.put(key, Some(&new));
        }
    }
}

impl pm_sources::vault_form::Settings for Draft {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(Draft::get(self, key))
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.put(key, Some(value));
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        self.put(key, None);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_ins_survive_a_restart_and_the_file_is_the_apps_alone() {
        let dir = tempfile::tempdir().unwrap();
        let s = FileStore::open(dir.path()).unwrap();
        s.update("tidal.unofficial", &mut |now| {
            assert!(now.is_none());
            Some("{\"enabled\":true}".into())
        });
        s.update("tidal.unofficial", &mut |_| None);
        let again = FileStore::open(dir.path()).unwrap();
        assert_eq!(
            again.get("tidal.unofficial").as_deref(),
            Some("{\"enabled\":true}")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "only the app reads the sign-ins: {mode:o}");
        }
        assert!(again.cached().is_none());
        again
            .keep(&App {
                id: "1".into(),
                secret: "s".into(),
                from: "web".into(),
                bundle_url: None,
            })
            .unwrap();
        assert_eq!(FileStore::open(dir.path()).unwrap().cached().unwrap().id, "1");
    }

    /// The copies kept beside the sign-ins file: name, contents.
    fn kept(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
        let mut v: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().into_string().unwrap())
            .filter(|n| n.ends_with(".unreadable"))
            .map(|n| (n.clone(), std::fs::read(dir.join(&n)).unwrap()))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn sign_ins_that_cant_be_read_are_kept_aside_whole() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        // None yet: a first start, empty, nothing set aside
        let s = FileStore::open(dir.path()).unwrap();
        assert!(SessionStore::get(&s, "tidal.unofficial").is_none());
        assert!(s.set_aside().0.is_empty());
        // Not the store's JSON (cut short): kept whole under a dated name, both sign-ins set
        // aside (signed out here, held out of the vault sync)
        let cut = br#"{"tidal.unofficial":"signed in""#;
        std::fs::write(&file, cut).unwrap();
        let s = FileStore::open(dir.path()).unwrap();
        let (keys, name) = s.set_aside();
        assert_eq!(keys, ["tidal.unofficial", "qobuz.unofficial"]);
        let name = name.unwrap().to_string();
        assert!(
            name.starts_with("sign-ins.json.20") && name.ends_with(".unreadable"),
            "{name}"
        );
        assert_eq!(kept(dir.path()), vec![(name.clone(), cut.to_vec())]);
        for k in ["tidal.unofficial", "qobuz.unofficial"] {
            assert!(SessionStore::get(&s, k).is_none());
            assert_eq!(
                SessionStore::get(&s, &set_aside_key(k)).as_deref(),
                Some(name.as_str())
            );
        }
        // The next start reads the file (nothing more set aside; the notes stay until a sign-in)
        drop(s);
        let s = FileStore::open(dir.path()).unwrap();
        assert!(s.set_aside().0.is_empty());
        assert!(SessionStore::get(&s, &set_aside_key("tidal.unofficial")).is_some());
        assert_eq!(kept(dir.path()).len(), 1);
        // Can't be read at all (a folder in its place, a read error): an error, left there
        drop(s);
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert!(FileStore::open(dir.path()).is_err());
        assert!(file.is_dir());
    }

    #[test]
    fn a_sign_in_that_cant_be_read_is_set_aside_alone() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        // The file reads, one service's sign-in in it doesn't (read as signed out, the vault sync
        // would remove the vault's copy): only that one is set aside, the file kept whole
        for (key, bad, other) in [
            ("tidal.unofficial", "{", "qobuz.unofficial"),
            ("qobuz.unofficial", "{\"session\":7}", "tidal.unofficial"),
        ] {
            let body = serde_json::to_vec(&BTreeMap::from([
                (key, bad),
                (other, r#"{"enabled":true}"#),
                ("vault.lineage.qobuz", "q-1"),
            ]))
            .unwrap();
            std::fs::write(&file, &body).unwrap();
            let s = FileStore::open(dir.path()).unwrap();
            assert_eq!(s.set_aside().0, [key], "{key}");
            assert!(SessionStore::get(&s, key).is_none());
            assert!(SessionStore::get(&s, &set_aside_key(key)).is_some());
            assert!(SessionStore::get(&s, &set_aside_key(other)).is_none());
            assert_eq!(
                SessionStore::get(&s, other).as_deref(),
                Some(r#"{"enabled":true}"#)
            );
            assert_eq!(
                SessionStore::get(&s, "vault.lineage.qobuz").as_deref(),
                Some("q-1")
            );
            let name = s.set_aside().1.unwrap().to_string();
            assert_eq!(std::fs::read(dir.path().join(name)).unwrap(), body);
        }
        // A value that isn't a string: that one alone (a sign-in set aside, anything else left
        // out), the rest read
        let body =
            br#"{"tidal.unofficial":{"enabled":true},"qobuz.app":7,"qobuz.unofficial":"{\"enabled\":true}"}"#;
        std::fs::write(&file, body).unwrap();
        let s = FileStore::open(dir.path()).unwrap();
        assert_eq!(s.set_aside().0, ["tidal.unofficial"]);
        assert!(SessionStore::get(&s, "qobuz.app").is_none());
        assert_eq!(
            SessionStore::get(&s, "qobuz.unofficial").as_deref(),
            Some(r#"{"enabled":true}"#)
        );
        let name = s.set_aside().1.unwrap().to_string();
        assert_eq!(std::fs::read(dir.path().join(name)).unwrap(), body);
        // Both as they are kept: read, nothing set aside
        let good = BTreeMap::from([
            ("tidal.unofficial", r#"{"enabled":true,"session":null}"#),
            ("qobuz.unofficial", r#"{"enabled":false}"#),
            ("vault.lineage.qobuz", "q-1"),
        ]);
        std::fs::write(&file, serde_json::to_vec(&good).unwrap()).unwrap();
        let s = FileStore::open(dir.path()).unwrap();
        assert!(s.set_aside().0.is_empty());
        assert_eq!(
            SessionStore::get(&s, "vault.lineage.qobuz").as_deref(),
            Some("q-1")
        );
    }

    #[test]
    fn why_a_sign_in_doesnt_read_never_quotes_it() {
        let e = sign_in_reads("tidal.unofficial", r#"{"session":"tok-123"}"#).unwrap_err();
        assert!(!e.contains("tok-123"), "{e}");
        assert!(e.starts_with("TIDAL: not its form"), "{e}");
    }

    #[test]
    fn the_copys_date_is_utc() {
        assert_eq!(utc_stamp(0), "1970-01-01T00-00-00Z");
        assert_eq!(utc_stamp(1_795_340_000), "2026-11-22T09-33-20Z");
        assert_eq!(utc_stamp(951_782_400), "2000-02-29T00-00-00Z");
    }

    #[test]
    fn a_change_that_cant_be_saved_doesnt_happen() {
        let dir = tempfile::tempdir().unwrap();
        let s = FileStore::open(dir.path()).unwrap();
        s.put("tidal.unofficial", Some("signed in")).unwrap();
        let told = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let t2 = told.clone();
        s.on_change(move |_| {
            t2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        // The file can't be replaced now (a folder in its place)
        let file = dir.path().join(FILE);
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        // A sign-out: an error, and still signed in
        assert!(s.put("tidal.unofficial", None).is_err());
        assert_eq!(s.get("tidal.unofficial").as_deref(), Some("signed in"));
        // A renewal through the shared code: left out
        s.update("tidal.unofficial", &mut |_| Some("renewed".into()));
        assert_eq!(s.get("tidal.unofficial").as_deref(), Some("signed in"));
        // A step of the phone's own: an error, nothing changed
        assert!(s
            .change_if(|d| {
                d.put("tidal.unofficial", None);
                Ok(true)
            })
            .is_err());
        assert_eq!(s.get("tidal.unofficial").as_deref(), Some("signed in"));
        assert_eq!(
            told.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "nobody told of a change"
        );
    }
}
