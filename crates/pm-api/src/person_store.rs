//! The node's [`pm_sources::SessionStore`]: one person's settings (`Who`), read and written under
//! the library lock, so the shared TIDAL and Qobuz code keeps sign-ins exactly where the node
//! always has (`tidal.unofficial`, `qobuz.unofficial`, per person).

use crate::accounts::Who;

pub(crate) struct PersonStore<'a> {
    pub library: &'a pm_library::LibraryLock,
    pub who: &'a Who,
}

impl<'a> PersonStore<'a> {
    pub(crate) fn new(library: &'a pm_library::LibraryLock, who: &'a Who) -> Self {
        Self { library, who }
    }
}

impl pm_sources::SessionStore for PersonStore<'_> {
    fn get(&self, key: &str) -> Option<String> {
        let lib = self.library.lock().unwrap_or_else(|e| e.into_inner());
        self.who.get(&lib, key)
    }

    fn update(&self, key: &str, f: &mut dyn FnMut(Option<String>) -> Option<String>) {
        let lib = self.library.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = f(self.who.get(&lib, key)) {
            // A person no longer on the node (or nobody) gets nothing written, as before
            let _ = self.who.put(&lib, key, Some(&v));
        }
    }
}
