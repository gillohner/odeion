//! Pause points for this crate's own tests (cargo feature `test-hooks`, which only the crate's
//! dev-dependency on itself turns on): a test arms a point by name and the code stops there
//! until the test lets it go, so a race can be played out in a fixed order. Without the feature
//! every point is an empty inline function and nothing is checked.

#[cfg(feature = "test-hooks")]
mod imp {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    use tokio::sync::oneshot;

    type Armed = HashMap<String, (oneshot::Sender<()>, oneshot::Receiver<()>)>;

    fn armed() -> &'static Mutex<Armed> {
        static A: OnceLock<Mutex<Armed>> = OnceLock::new();
        A.get_or_init(Default::default)
    }

    /// Stop at `key` once: the returned receiver fires when the code got there, and the code
    /// goes on when the returned sender is used (or dropped).
    pub fn arm(key: &str) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (reached_tx, reached_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        armed()
            .lock()
            .expect("test hooks")
            .insert(key.to_string(), (reached_tx, release_rx));
        (reached_rx, release_tx)
    }

    pub async fn pause(key: impl FnOnce() -> String) {
        let entry = armed().lock().expect("test hooks").remove(&key());
        if let Some((reached, release)) = entry {
            let _ = reached.send(());
            let _ = release.await;
        }
    }
}

#[cfg(feature = "test-hooks")]
pub use imp::{arm, pause};

/// A pause point (see the module): nothing without the `test-hooks` feature.
#[cfg(not(feature = "test-hooks"))]
#[inline(always)]
pub async fn pause(_key: impl FnOnce() -> String) {}
