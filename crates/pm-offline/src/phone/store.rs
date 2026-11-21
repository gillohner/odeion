//! What phone mode keeps on the phone for its one person: the streaming sign-ins
//! (`tidal.unofficial`, `qobuz.unofficial`) and Qobuz's web-player app values (`qobuz.app`), as
//! one JSON file in the app's own storage (`phone/sign-ins.json`, readable only by the app).
//! Every change is written whole to a temporary file and renamed over the old one.

use pm_sources::qobuz::{App, AppCache};
use pm_sources::{SessionStore, SourceError};
use std::cell::RefCell;
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
    /// The store in `dir` (made if missing); a file that can't be read starts empty.
    pub fn open(dir: &std::path::Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(FILE);
        let values = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
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
        std::fs::rename(&tmp, &self.path)
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
            let d = Draft(RefCell::new(v.clone()));
            if !f(&d)? {
                return Ok(false);
            }
            let new = d.0.into_inner();
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

    /// Sets (or with `None` removes) a key.
    pub fn put(&self, key: &str, value: Option<&str>) -> std::io::Result<()> {
        {
            let mut v = self.values.lock().unwrap_or_else(|e| e.into_inner());
            match value {
                Some(x) => v.insert(key.to_string(), x.to_string()),
                None => v.remove(key),
            };
            self.save(&v)?;
        }
        self.tell(key);
        Ok(())
    }
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

    fn update(&self, key: &str, f: &mut dyn FnMut(Option<String>) -> Option<String>) {
        let wrote = {
            let mut v = self.values.lock().unwrap_or_else(|e| e.into_inner());
            match f(v.get(key).cloned()) {
                Some(new) => {
                    v.insert(key.to_string(), new);
                    if let Err(e) = self.save(&v) {
                        tracing::warn!("phone mode: can't save the sign-ins: {e}");
                    }
                    true
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
pub struct Draft(RefCell<BTreeMap<String, String>>);

impl Draft {
    pub fn get(&self, key: &str) -> Option<String> {
        self.0.borrow().get(key).cloned()
    }
    /// Sets (or with `None` removes) a key.
    pub fn put(&self, key: &str, value: Option<&str>) {
        let mut v = self.0.borrow_mut();
        match value {
            Some(x) => v.insert(key.to_string(), x.to_string()),
            None => v.remove(key),
        };
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
}
