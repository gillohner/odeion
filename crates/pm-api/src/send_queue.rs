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
//!
//! The cap (`max`) holds what is still waiting after a send tried it ([`settle`]): a backlog that
//! reached it and a new listen are all sent first, and only the oldest that failed go. Between
//! sends the list may grow to [`ceiling`] (bounded all the same). A listen the library refused to
//! store is sent from memory; when that send fails it is stored then, and while the library
//! still refuses it waits in memory for the same account ([`hold`]), never in another's list.

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

/// How long the list may grow between sends for a cap of `max`: what a send hasn't tried yet is
/// kept until it has, up to this.
pub(crate) fn ceiling(max: usize) -> usize {
    max.saturating_mul(2)
}

/// Add `new` at the end of the list under `key`; past [`ceiling`]`(max)` entries the oldest go
/// (the cap itself is applied once a send tried them, [`settle`]). On an error nothing was
/// stored: the caller keeps `new` (it gets it back) and sends it from memory.
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
    let skip = all.len().saturating_sub(ceiling(max));
    write(lib, key, &all[skip..]).map_err(|e| (e, kept))
}

/// Remove `sent` from the list under `key` (each entry once, the first equal one), keeping the
/// rest in order: what failed, and what was added since the list was read. A list that can't
/// be read is left as it is. (Senders use [`settle`].)
#[cfg(test)]
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

type Held = Mutex<HashMap<(String, &'static str, String), Vec<Value>>>;

fn held() -> &'static Held {
    static H: OnceLock<Held> = OnceLock::new();
    H.get_or_init(Default::default)
}

/// Keep `items` in memory for `owner` (the account they were played for) on node `node`, while
/// the library refuses to store them: the next send for that account takes them ([`batch`]). At
/// most `max`, the oldest going.
pub(crate) fn hold(node: &str, service: &'static str, owner: &str, items: Vec<Value>, max: usize) {
    if items.is_empty() {
        return;
    }
    let mut h = held().lock().expect("send queue held");
    let mine = h
        .entry((node.to_string(), service, owner.to_string()))
        .or_default();
    mine.extend(items);
    let skip = mine.len().saturating_sub(max);
    mine.drain(..skip);
}

fn take_held(node: &str, service: &'static str, owner: &str) -> Vec<Value> {
    held()
        .lock()
        .expect("send queue held")
        .remove(&(node.to_string(), service, owner.to_string()))
        .unwrap_or_default()
}

/// What a sender sends, by where each listen is: the list on disk (`durable`), and what only
/// memory has (`memory`: the library refused to store it).
#[derive(Debug, Default)]
pub(crate) struct Batch {
    pub(crate) durable: Vec<Value>,
    pub(crate) memory: Vec<Value>,
}

impl Batch {
    /// Everything to send, the list on disk first.
    pub(crate) fn all(&self) -> Vec<Value> {
        self.durable.iter().chain(&self.memory).cloned().collect()
    }
}

/// What a sender for `owner` sends, read under the library's lock with the account (the caller
/// holds it): the list on disk, and what only memory has (`unstored`, and what waited in memory
/// for this account). What only memory had is stored first when the library takes it now, so
/// it is on disk once, not sent twice.
pub(crate) fn batch(
    lib: &pm_library::Library,
    key: &str,
    max: usize,
    (node, service, owner): (&str, &'static str, &str),
    unstored: Vec<Value>,
) -> Batch {
    let mut memory = take_held(node, service, owner);
    memory.extend(unstored);
    if !memory.is_empty() {
        memory = match enqueue(lib, key, memory, max) {
            Ok(()) => Vec::new(),
            Err((_, back)) => back,
        };
    }
    let durable = match read(lib, key) {
        Ok(all) => all,
        Err(e) => {
            tracing::warn!("{key}: {e}; sending only the listens in memory");
            Vec::new()
        }
    };
    Batch { durable, memory }
}

/// After a send for `owner` tried `batch`: what was `sent` leaves the list; what only memory had
/// and wasn't sent is stored now (when `owns`, the account still being the one connected), else
/// kept in memory for `owner` ([`hold`]); and, while `owns`, the list is held to `max`, the oldest
/// (tried and failed) going first. A list that can't be read is left as it is.
pub(crate) fn settle(
    lib: &pm_library::Library,
    key: &str,
    max: usize,
    (node, service, owner): (&str, &'static str, &str),
    owns: bool,
    batch: Batch,
    sent: &[Value],
) {
    let mut sent = sent.to_vec();
    let mut failed = Vec::new();
    for m in batch.memory {
        match sent.iter().position(|v| *v == m) {
            Some(i) => {
                sent.remove(i);
            }
            None => failed.push(m),
        }
    }
    let mut left = match read(lib, key) {
        Ok(left) => left,
        Err(e) => {
            tracing::warn!("{key}: {e}; the ones sent stay on the list");
            hold(node, service, owner, failed, max);
            return;
        }
    };
    for s in &sent {
        if let Some(i) = left.iter().position(|v| v == s) {
            left.remove(i);
        }
    }
    let added = if owns {
        std::mem::take(&mut failed)
    } else {
        Vec::new()
    };
    hold(node, service, owner, failed, max);
    left.extend(added.iter().cloned());
    if owns {
        let skip = left.len().saturating_sub(max);
        left.drain(..skip);
    }
    if let Err(e) = write(lib, key, &left) {
        tracing::warn!("{key}: {e}; the ones sent stay on the list, the failed new ones wait in memory");
        hold(node, service, owner, added, max);
    }
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

    /// Past the cap nothing goes before a send tried it: a full list and a new listen are all
    /// sent; only what failed is held to the cap after, the oldest going first. Between sends
    /// the list is bounded by the ceiling.
    #[test]
    fn the_cap_holds_what_failed_after_a_send_tried_it() {
        let lib = pm_library::Library::open_in_memory().unwrap();
        // Its own node: what waits in memory is process-wide, and tests run at once
        const WHO: (&str, &str, &str) = ("node-cap", "test", "owner");
        enqueue(&lib, "k", (1..=3).map(l).collect(), 3).unwrap();
        enqueue(&lib, "k", vec![l(4)], 3).unwrap();
        assert_eq!(
            read(&lib, "k").unwrap(),
            [l(1), l(2), l(3), l(4)],
            "all kept for the send"
        );
        let b = batch(&lib, "k", 3, WHO, Vec::new());
        assert_eq!(b.all(), [l(1), l(2), l(3), l(4)]);
        // Everything failed: the oldest goes
        settle(&lib, "k", 3, WHO, true, b, &[]);
        assert_eq!(read(&lib, "k").unwrap(), [l(2), l(3), l(4)]);
        // A healthy send of a full list and a new one: nothing lost
        enqueue(&lib, "k", vec![l(5)], 3).unwrap();
        let b = batch(&lib, "k", 3, WHO, Vec::new());
        let sent = b.all();
        settle(&lib, "k", 3, WHO, true, b, &sent);
        assert_eq!(read(&lib, "k").unwrap(), Vec::<Value>::new());
        // The ceiling bounds the list between sends
        enqueue(&lib, "k", (1..=9).map(l).collect(), 3).unwrap();
        assert_eq!(read(&lib, "k").unwrap().len(), ceiling(3));
    }

    /// A listen only memory had (the library refused it) that the send didn't get through is
    /// stored once the library takes writes again; while it doesn't, it waits in memory for the
    /// same account, comes back with that account's next send, and never goes to another's list.
    #[test]
    fn a_failed_listen_from_memory_is_kept() {
        let lib = pm_library::Library::open_in_memory().unwrap();
        const WHO: (&str, &str, &str) = ("node-memory", "test", "owner");
        // Storage works again by the time the send failed: stored, once
        let b = Batch {
            durable: vec![l(1)],
            memory: vec![l(2), l(3)],
        };
        enqueue(&lib, "k", vec![l(1)], 10).unwrap();
        settle(&lib, "k", 10, WHO, true, b, &[l(1), l(3)]);
        assert_eq!(read(&lib, "k").unwrap(), [l(2)]);
        // Another account connected meanwhile: memory for its own account, not this list
        let other = ("node-memory", "test", "other-owner");
        let b = Batch {
            durable: Vec::new(),
            memory: vec![l(4)],
        };
        settle(&lib, "k", 10, other, false, b, &[]);
        assert_eq!(read(&lib, "k").unwrap(), [l(2)]);
        assert_eq!(batch(&lib, "k", 10, WHO, Vec::new()).memory, Vec::<Value>::new());
        // ...and that account's next send has it, stored first (the library takes it now)
        let b = batch(&lib, "k", 10, other, Vec::new());
        assert_eq!(
            (b.durable.clone(), b.memory.clone()),
            (vec![l(2), l(4)], Vec::new())
        );
        // The list can't be read: what failed waits in memory, the list is left alone
        lib.set_setting("k", "not a list").unwrap();
        let b = Batch {
            durable: Vec::new(),
            memory: vec![l(5)],
        };
        settle(&lib, "k", 10, WHO, true, b, &[]);
        assert_eq!(lib.get_setting("k").unwrap().as_deref(), Some("not a list"));
        lib.delete_setting("k").unwrap();
        let b = batch(&lib, "k", 10, WHO, Vec::new());
        assert_eq!(
            (b.durable, b.memory),
            (vec![l(5)], Vec::new()),
            "stored once it could be"
        );
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
        assert_eq!(
            batch(&lib, "k", 10, ("node-unreadable", "test", "owner"), vec![l(2)]).all(),
            [l(2)]
        );
    }
}
