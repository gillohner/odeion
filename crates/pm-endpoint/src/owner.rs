//! One stream per output on this machine. The node's sessions (this crate's endpoint) and the
//! app's own player (`pm-app-player`) both take an output here before they open it. The node is
//! refused an output another of its sessions holds, as before; between the node and the app the
//! newer wins: taking an output stops whoever had it (its `stop` runs) and waits until it lets go,
//! so two streams never reach one DAC.

use pm_zones::OutputId;
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Who holds an output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holder {
    /// A session of the node this player is registered with (the phone's own zone).
    Node,
    /// The app's own player (the page's "This phone").
    App,
}

/// Stops a holder: it lets go of its [`Claim`] soon after (from another thread).
pub type Stop = Arc<dyn Fn() + Send + Sync>;

struct Held {
    token: u64,
    holder: Holder,
    stop: Stop,
}

#[derive(Default)]
struct Table {
    held: HashMap<OutputId, Held>,
    /// Last grant, retained after release: an automatic resume must see an intervening owner.
    granted: HashMap<OutputId, u64>,
    next: u64,
}

/// The outputs in use and who uses them. Cloning shares the table.
#[derive(Clone, Default)]
pub struct OutputOwner {
    table: Arc<(Mutex<Table>, Condvar)>,
}

/// An output taken; dropping it lets the output go.
pub struct Claim {
    owner: OutputOwner,
    id: OutputId,
    token: u64,
    previous: u64,
}

impl Claim {
    /// The atomic grant's previous/current generations, for a pause racing its own open.
    pub fn generations(&self) -> (u64, u64) {
        (self.previous, self.token)
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let (lock, freed) = &*self.owner.table;
        let mut t = lock.lock().unwrap_or_else(|e| e.into_inner());
        if t.held.get(&self.id).is_some_and(|h| h.token == self.token) {
            t.held.remove(&self.id);
        }
        freed.notify_all();
    }
}

impl std::fmt::Debug for Claim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Claim").field("id", &self.id).finish()
    }
}

impl OutputOwner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Who holds `id` now.
    pub fn holder(&self, id: &OutputId) -> Option<Holder> {
        let (lock, _) = &*self.table;
        let t = lock.lock().unwrap_or_else(|e| e.into_inner());
        t.held.get(id).map(|h| h.holder)
    }

    pub fn generation(&self, id: &OutputId) -> u64 {
        let (lock, _) = &*self.table;
        *lock
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .granted
            .get(id)
            .unwrap_or(&0)
    }

    /// Resume only if nobody took this output since the pause. The check and grant are atomic;
    /// this never stops another holder, even if it takes the output just as focus returns.
    pub fn take_unchanged(
        &self,
        id: &OutputId,
        holder: Holder,
        stop: Stop,
        generation: u64,
    ) -> Result<Claim, String> {
        self.take_inner(id, holder, stop, Duration::ZERO, Some(generation))
    }

    /// Take `id` for `holder`. A node session is refused an output another node session holds
    /// (`Err("<id> is in use")`); the other holder is stopped first, and this waits up to `wait`
    /// for it to let go. `stop` is how this holder is stopped in turn.
    pub fn take(&self, id: &OutputId, holder: Holder, stop: Stop, wait: Duration) -> Result<Claim, String> {
        self.take_inner(id, holder, stop, wait, None)
    }

    fn take_inner(
        &self,
        id: &OutputId,
        holder: Holder,
        stop: Stop,
        wait: Duration,
        unchanged: Option<u64>,
    ) -> Result<Claim, String> {
        let (lock, freed) = &*self.table;
        let mut t = lock.lock().unwrap_or_else(|e| e.into_inner());
        if unchanged.is_some_and(|g| t.held.contains_key(id) || t.granted.get(id).copied().unwrap_or(0) != g)
        {
            return Err(format!(
                "{id} was used since the pause; press Play to take it back"
            ));
        }
        let deadline = Instant::now() + wait;
        let mut stopped = None;
        while let Some(h) = t.held.get(id) {
            if h.holder == Holder::Node && holder == Holder::Node {
                return Err(format!("{id} is in use"));
            }
            // The other one is stopped once, outside the lock (its stop may take the lock to let
            // go); the app's own earlier sink is only waited for (it closes before the next opens)
            if h.holder != holder && stopped != Some(h.token) {
                stopped = Some(h.token);
                let stop = h.stop.clone();
                drop(t);
                stop();
                t = lock.lock().unwrap_or_else(|e| e.into_inner());
                continue;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(format!("{id} is still in use"));
            }
            t = freed.wait_timeout(t, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        t.next += 1;
        let token = t.next;
        let previous = t.granted.get(id).copied().unwrap_or(0);
        t.granted.insert(id.clone(), token);
        t.held.insert(id.clone(), Held { token, holder, stop });
        Ok(Claim {
            owner: self.clone(),
            id: id.clone(),
            token,
            previous,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn id() -> OutputId {
        OutputId::new("cpal", "Speaker or headphones")
    }

    fn never() -> Stop {
        Arc::new(|| {})
    }

    #[test]
    fn a_second_node_session_is_refused_and_a_free_output_is_taken() {
        let o = OutputOwner::new();
        let a = o.take(&id(), Holder::Node, never(), Duration::ZERO).unwrap();
        assert_eq!(o.holder(&id()), Some(Holder::Node));
        let e = o.take(&id(), Holder::Node, never(), Duration::ZERO).unwrap_err();
        assert!(e.contains("in use"), "{e}");
        drop(a);
        assert_eq!(o.holder(&id()), None);
        let _b = o.take(&id(), Holder::Node, never(), Duration::ZERO).unwrap();
    }

    #[test]
    fn automatic_resume_never_takes_over_or_forgets_a_released_owner() {
        let o = OutputOwner::new();
        let app = o.take(&id(), Holder::App, never(), Duration::ZERO).unwrap();
        let before = o.generation(&id());
        drop(app);
        let resumed = o.take_unchanged(&id(), Holder::App, never(), before).unwrap();
        let before = o.generation(&id());
        drop(resumed);
        let stops = Arc::new(AtomicUsize::new(0));
        let seen = stops.clone();
        let node = o
            .take(
                &id(),
                Holder::Node,
                Arc::new(move || {
                    seen.fetch_add(1, Ordering::SeqCst);
                }),
                Duration::ZERO,
            )
            .unwrap();
        assert!(o.take_unchanged(&id(), Holder::App, never(), before).is_err());
        assert_eq!(stops.load(Ordering::SeqCst), 0);
        drop(node);
        assert!(o.take_unchanged(&id(), Holder::App, never(), before).is_err());
        assert_eq!(stops.load(Ordering::SeqCst), 0);
    }

    /// The app takes the output from the node's session and the node takes it back: each time
    /// the one holding it is stopped once, lets go, and only then does the other hold it.
    #[test]
    fn the_newer_of_the_node_and_the_app_wins_after_the_other_let_go() {
        let o = OutputOwner::new();
        let slot: Arc<Mutex<Option<Claim>>> = Arc::default();
        let stops = Arc::new(AtomicUsize::new(0));
        let stopper = |slot: &Arc<Mutex<Option<Claim>>>, stops: &Arc<AtomicUsize>| -> Stop {
            let (slot, stops) = (slot.clone(), stops.clone());
            Arc::new(move || {
                stops.fetch_add(1, Ordering::SeqCst);
                let slot = slot.clone();
                // Lets go a moment later, from its own thread (as a session or a zone does)
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(30));
                    slot.lock().unwrap().take();
                });
            })
        };
        let node = o
            .take(&id(), Holder::Node, stopper(&slot, &stops), Duration::ZERO)
            .unwrap();
        *slot.lock().unwrap() = Some(node);
        let app = o
            .take(&id(), Holder::App, stopper(&slot, &stops), Duration::from_secs(5))
            .unwrap();
        assert_eq!(stops.load(Ordering::SeqCst), 1);
        assert!(slot.lock().unwrap().is_none(), "the node's claim went first");
        assert_eq!(o.holder(&id()), Some(Holder::App));
        *slot.lock().unwrap() = Some(app);
        let _node = o
            .take(
                &id(),
                Holder::Node,
                stopper(&slot, &stops),
                Duration::from_secs(5),
            )
            .unwrap();
        assert_eq!(stops.load(Ordering::SeqCst), 2);
        assert_eq!(o.holder(&id()), Some(Holder::Node));
    }

    #[test]
    fn a_holder_that_never_lets_go_is_an_error_not_a_second_stream() {
        let o = OutputOwner::new();
        let _app = o.take(&id(), Holder::App, never(), Duration::ZERO).unwrap();
        let e = o
            .take(&id(), Holder::Node, never(), Duration::from_millis(50))
            .unwrap_err();
        assert!(e.contains("still in use"), "{e}");
        assert_eq!(o.holder(&id()), Some(Holder::App));
    }

    /// A claim whose entry was taken over meanwhile, dropped late, leaves the new holder alone.
    #[test]
    fn an_old_claim_dropped_late_keeps_the_new_holder() {
        let o = OutputOwner::new();
        let old = o.take(&id(), Holder::App, never(), Duration::ZERO).unwrap();
        o.table.0.lock().unwrap().held.remove(&id());
        let _new = o.take(&id(), Holder::Node, never(), Duration::ZERO).unwrap();
        drop(old);
        assert_eq!(o.holder(&id()), Some(Holder::Node));
    }
}
