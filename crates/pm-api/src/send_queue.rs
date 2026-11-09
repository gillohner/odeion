//! The listens waiting for a scrobble service (ListenBrainz, Last.fm): a list in one setting that
//! stays on disk until the service took each entry.
//!
//! A new listen is added to the list first ([`enqueue`]), under the library's lock. One sender at a
//! time per node and service ([`sender`], an async lock of its own, never the library's) reads
//! the list, sends it, and removes only what the service acknowledged ([`remove_sent`]): a send cut
//! short (the node stopping, the app killed) leaves every listen on disk for the next one, a
//! listen added meanwhile stays, and the count of waiting listens is right throughout. Two plays
//! ending together never send the same listens: the second sender waits, then finds only what
//! the first left.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;

type Senders = Mutex<HashMap<(String, &'static str), Arc<tokio::sync::Mutex<()>>>>;

fn senders() -> &'static Senders {
    static S: OnceLock<Senders> = OnceLock::new();
    S.get_or_init(Default::default)
}

/// The right to send `service`'s waiting listens for node `node`, held across the sends.
pub(crate) async fn sender(node: &str, service: &'static str) -> tokio::sync::OwnedMutexGuard<()> {
    let m = senders()
        .lock()
        .expect("send queue senders")
        .entry((node.to_string(), service))
        .or_default()
        .clone();
    m.lock_owned().await
}

/// The waiting list under `key`. An error when it can't be read or isn't a list: never taken
/// for an empty one (which would be written over).
pub(crate) fn read(lib: &pm_library::Library, key: &str) -> Result<Vec<Value>, String> {
    match lib.get_setting(key).map_err(|e| e.to_string())? {
        None => Ok(Vec::new()),
        Some(j) => serde_json::from_str(&j).map_err(|e| format!("the waiting listens aren't readable: {e}")),
    }
}

fn write(lib: &pm_library::Library, key: &str, list: &[Value]) -> Result<(), String> {
    if list.is_empty() {
        lib.delete_setting(key).map(|_| ()).map_err(|e| e.to_string())
    } else {
        lib.set_setting(key, &Value::from(list.to_vec()).to_string())
            .map_err(|e| e.to_string())
    }
}

/// Add `new` at the end of the list under `key`; past `max` entries the oldest go. On an error
/// nothing was stored: the caller keeps `new` (it gets it back) and sends it from memory.
pub(crate) fn enqueue(
    lib: &pm_library::Library,
    key: &str,
    new: Vec<Value>,
    max: usize,
) -> Result<(), (String, Vec<Value>)> {
    if new.is_empty() {
        return Ok(());
    }
    let mut all = match read(lib, key) {
        Ok(all) => all,
        Err(e) => return Err((e, new)),
    };
    let kept = new.clone();
    all.extend(new);
    let skip = all.len().saturating_sub(max);
    write(lib, key, &all[skip..]).map_err(|e| (e, kept))
}

/// Remove `sent` from the list under `key` (each entry once, the first equal one), keeping the
/// rest in order: what failed, and what was added since the list was read. A list that can't
/// be read is left as it is.
pub(crate) fn remove_sent(lib: &pm_library::Library, key: &str, sent: &[Value]) -> Result<(), String> {
    if sent.is_empty() {
        return Ok(());
    }
    let mut left = read(lib, key)?;
    for s in sent {
        if let Some(i) = left.iter().position(|v| v == s) {
            left.remove(i);
        }
    }
    write(lib, key, &left)
}

/// What a sender sends: the list on disk, and what couldn't be stored there (`unstored`), read
/// together with the account under the library's lock by the caller.
pub(crate) fn batch(lib: &pm_library::Library, key: &str, unstored: Vec<Value>) -> Vec<Value> {
    let mut all = match read(lib, key) {
        Ok(all) => all,
        Err(e) => {
            tracing::warn!("{key}: {e}; sending only the new listens");
            Vec::new()
        }
    };
    all.extend(unstored);
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(n: u64) -> Value {
        serde_json::json!({"listened_at": n})
    }

    #[test]
    fn the_list_keeps_what_wasnt_sent_and_what_came_meanwhile() {
        let lib = pm_library::Library::open_in_memory().unwrap();
        enqueue(&lib, "k", (1..=4).map(l).collect(), 10).unwrap();
        let taken = read(&lib, "k").unwrap();
        // Added while a send ran
        enqueue(&lib, "k", vec![l(5)], 10).unwrap();
        // The send took 1, 2 and 4; 3 failed
        remove_sent(&lib, "k", &[l(1), l(2), l(4)]).unwrap();
        assert_eq!(read(&lib, "k").unwrap(), [l(3), l(5)]);
        assert_eq!(taken.len(), 4);
        // Everything sent: the setting goes
        remove_sent(&lib, "k", &[l(3), l(5)]).unwrap();
        assert_eq!(lib.get_setting("k").unwrap(), None);
    }

    #[test]
    fn past_the_cap_the_oldest_go() {
        let lib = pm_library::Library::open_in_memory().unwrap();
        enqueue(&lib, "k", (1..=3).map(l).collect(), 3).unwrap();
        enqueue(&lib, "k", vec![l(4)], 3).unwrap();
        assert_eq!(read(&lib, "k").unwrap(), [l(2), l(3), l(4)]);
    }

    /// A list that isn't readable is never taken for an empty one: an add is refused (the new
    /// listens come back to be sent from memory), and nothing is written over it.
    #[test]
    fn an_unreadable_list_is_left_alone() {
        let lib = pm_library::Library::open_in_memory().unwrap();
        lib.set_setting("k", "not a list").unwrap();
        let (_, back) = enqueue(&lib, "k", vec![l(1)], 10).unwrap_err();
        assert_eq!(back, [l(1)]);
        assert!(remove_sent(&lib, "k", &[l(1)]).is_err());
        assert_eq!(lib.get_setting("k").unwrap().as_deref(), Some("not a list"));
        assert_eq!(batch(&lib, "k", vec![l(2)]), [l(2)]);
    }
}
