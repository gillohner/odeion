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
}

impl FileStore {
    /// The store in `dir` (made if missing); with no file yet, empty. A file that can't be read,
    /// or isn't the store's JSON, is an error and is left as it is: read as empty it would be a
    /// sign-out of everything, which the vault sync would then carry to the vault. The same for a
    /// streaming sign-in in it that isn't its service's JSON (each service reads one it can't
    /// parse as signed out).
    pub fn open(dir: &std::path::Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(FILE);
        let values: BTreeMap<String, String> = match std::fs::read(&path) {
            Ok(b) => serde_json::from_slice(&b).map_err(|e| unreadable(&path, e))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(unreadable(&path, e)),
        };
        for (k, v) in &values {
            sign_in_reads(k, v).map_err(|e| unreadable(&path, e))?;
        }
        Ok(Self {
            path,
            values: Mutex::new(values),
            changed: Mutex::new(None),
        })
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
                .map_err(|e| format!("TIDAL: {e}"))
        }
        pm_sources::qobuz::STATE_KEY => serde_json::from_str::<pm_sources::qobuz::ModeState>(value)
            .map(|_| ())
            .map_err(|e| format!("Qobuz: {e}")),
        _ => Ok(()),
    }
}

/// The sign-ins file can't be read (logged): phone mode doesn't start rather than start signed out.
fn unreadable(path: &std::path::Path, e: impl std::fmt::Display) -> std::io::Error {
    tracing::error!(
        "phone mode: the sign-ins in {} can't be read ({e}); left as they are",
        path.display()
    );
    std::io::Error::other(format!(
        "the sign-ins on this phone can't be read ({e}); they are left as they are"
    ))
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

    #[test]
    fn sign_ins_that_cant_be_read_are_an_error_and_stay_as_they_are() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        // None yet: a first start, empty
        assert!(SessionStore::get(&FileStore::open(dir.path()).unwrap(), "tidal.unofficial").is_none());
        // Not the store's JSON (cut short): an error, the file as it was
        let cut = br#"{"tidal.unofficial":"signed in""#;
        std::fs::write(&file, cut).unwrap();
        assert!(FileStore::open(dir.path()).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), cut);
        // Can't be read at all (a folder in its place): an error, left there
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert!(FileStore::open(dir.path()).is_err());
        assert!(file.is_dir());
    }

    #[test]
    fn a_sign_in_that_cant_be_read_is_never_a_sign_out() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        // The file reads, one service's sign-in in it doesn't (each would read it as signed out,
        // and the vault sync would remove the vault's copy): an error, the file as it was
        for (key, bad) in [("tidal.unofficial", "{"), ("qobuz.unofficial", "{\"session\":7}")] {
            let body = serde_json::to_vec(&BTreeMap::from([(key, bad)])).unwrap();
            std::fs::write(&file, &body).unwrap();
            assert!(FileStore::open(dir.path()).is_err(), "{key}");
            assert_eq!(std::fs::read(&file).unwrap(), body);
        }
        // Both as they are kept: read
        let good = BTreeMap::from([
            ("tidal.unofficial", r#"{"enabled":true,"session":null}"#),
            ("qobuz.unofficial", r#"{"enabled":false}"#),
            ("vault.lineage.qobuz", "q-1"),
        ]);
        std::fs::write(&file, serde_json::to_vec(&good).unwrap()).unwrap();
        let s = FileStore::open(dir.path()).unwrap();
        assert_eq!(
            SessionStore::get(&s, "vault.lineage.qobuz").as_deref(),
            Some("q-1")
        );
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
