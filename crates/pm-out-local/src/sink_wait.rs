//! The waits of the shared (cpal) sink, apart from cpal so they are tested on every host:
//! waiting for room in its queue and for it to play out never outlasts a USB DAC that went away
//! (its stream stops taking audio, and the wait would never end), nor a stream that stopped
//! taking audio by itself, and a stream fault ends the stream for the sink only when it really
//! did (a disconnect, not an underrun).

use pm_zones::SinkError;
use std::time::{Duration, Instant};

/// What went wrong with a stream, as far as the sink cares (cpal's `ErrorKind`, mapped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamFault {
    /// An underrun or overrun: a glitch, and the stream goes on.
    Xrun,
    /// Android moved the stream to another device and it goes on there.
    Rerouted,
    /// The device is gone.
    DeviceGone,
    /// The stream must be rebuilt.
    Invalidated,
    /// A refused real-time priority: it goes on.
    RealtimeDenied,
    /// Anything else the stream's error callback reports (a busy device, no memory, unknown): the
    /// stream stopped taking audio.
    Other,
}

/// Whether a fault ends the stream for the sink: a disconnect or an invalidated stream always;
/// a reroute or any other fault when a USB DAC was attached at open (Android moved it, likely
/// to the phone's speaker; or the stream stopped and a wait for room would never end); an
/// underrun or a refused real-time priority never.
pub fn ends_stream(fault: StreamFault, dac_at_open: bool) -> bool {
    match fault {
        StreamFault::DeviceGone | StreamFault::Invalidated => true,
        StreamFault::Rerouted | StreamFault::Other => dac_at_open,
        StreamFault::Xrun | StreamFault::RealtimeDenied => false,
    }
}

const STEP: Duration = Duration::from_millis(5);

/// How long a write waits for room while the stream takes no audio at all before the output counts
/// as stopped. A write waits while about a second is queued, so a stream that plays frees room well
/// within it; one whose device went away without saying so (its callback simply stops) never
/// does, and the zone would hold its commands (Stop, Pause, Next) for good.
pub const STALL: Duration = Duration::from_secs(10);

/// Wait until `room()` (or `stopped()`); a `gone()` error ends the wait at once, and so does a
/// stream that took nothing (`taken()`, the frames it consumed, did not move) for `stall`: then
/// the error is `stalled()`.
pub fn wait_for_room(
    room: impl Fn() -> bool,
    stopped: impl Fn() -> bool,
    gone: impl Fn() -> Option<SinkError>,
    taken: impl Fn() -> u64,
    stall: Duration,
    stalled: impl FnOnce() -> SinkError,
) -> Result<(), SinkError> {
    let mut last = (taken(), Instant::now());
    let mut polled = Instant::now();
    while !room() {
        if stopped() {
            return Ok(());
        }
        if let Some(e) = gone() {
            return Err(e);
        }
        let now = taken();
        // A look that comes far later than its step: the machine slept (Windows counts that time
        // in `Instant`), and the stream had no chance to take anything meanwhile
        if polled.elapsed() >= ASLEEP {
            last.1 = Instant::now();
        }
        if now != last.0 {
            last = (now, Instant::now());
        } else if last.1.elapsed() >= stall {
            return Err(stalled());
        }
        polled = Instant::now();
        std::thread::sleep(STEP);
    }
    Ok(())
}

/// A gap between two looks of [`wait_for_room`] this long is time asleep, not a stall.
const ASLEEP: Duration = Duration::from_secs(1);

/// A stream that ended by itself (its error callback said the device went away), with or without a
/// USB DAC: the sink's writes end with Gone, so the zone pauses instead of waiting on it.
pub fn ended_by_itself(dead: bool, id: &pm_zones::OutputId) -> Option<SinkError> {
    dead.then(|| SinkError::Gone(id.clone(), "the output's device went away".into()))
}

/// Wait until `empty()`, at most `within`; a `gone()` error ends the wait at once.
pub fn wait_drained(
    empty: impl Fn() -> bool,
    gone: impl Fn() -> Option<SinkError>,
    within: Duration,
) -> Result<(), SinkError> {
    // Asked before and after, also with nothing queued: an unplug at a song boundary must not be
    // lost because the queue happened to be empty
    if let Some(e) = gone() {
        return Err(e);
    }
    let deadline = Instant::now() + within;
    while !empty() && Instant::now() < deadline {
        if let Some(e) = gone() {
            return Err(e);
        }
        std::thread::sleep(STEP);
    }
    match gone() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_zones::OutputId;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    fn gone_after(flag: &Arc<AtomicBool>) -> impl Fn() -> Option<SinkError> {
        let f = flag.clone();
        move || {
            f.load(Ordering::SeqCst)
                .then(|| SinkError::Gone(OutputId::new("cpal", "phone"), "FiiO KA17 was unplugged".into()))
        }
    }

    /// A stream that stopped taking audio (its DAC unplugged) never holds a write for good: the
    /// wait ends with Gone as soon as the DAC is gone.
    #[test]
    fn a_full_queue_on_a_dac_that_goes_ends_with_gone() {
        let gone = Arc::new(AtomicBool::new(false));
        let g = gone_after(&gone);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(wait_for_room(
                || false,
                || false,
                g,
                || 0,
                STALL,
                || unreachable!(),
            ));
        });
        std::thread::sleep(Duration::from_millis(50));
        gone.store(true, Ordering::SeqCst);
        let r = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("the wait ends once the DAC is gone");
        assert!(matches!(r, Err(SinkError::Gone(..))), "{r:?}");
        // And as before: room ends it, a stop ends it
        assert!(wait_for_room(|| true, || false, || None, || 0, STALL, || unreachable!()).is_ok());
        assert!(wait_for_room(|| false, || true, || None, || 0, STALL, || unreachable!()).is_ok());
    }

    #[test]
    fn playing_out_on_a_dac_that_goes_ends_with_gone() {
        let gone = Arc::new(AtomicBool::new(false));
        let g = gone_after(&gone);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(wait_drained(|| false, g, Duration::from_secs(30)));
        });
        std::thread::sleep(Duration::from_millis(50));
        gone.store(true, Ordering::SeqCst);
        let r = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("the drain ends once the DAC is gone");
        assert!(matches!(r, Err(SinkError::Gone(..))), "{r:?}");
        assert!(wait_drained(|| true, || None, Duration::from_secs(1)).is_ok());
        // Codex H3 (round 3): nothing queued, the DAC gone: still Gone
        let gone = Arc::new(AtomicBool::new(true));
        let r = wait_drained(|| true, gone_after(&gone), Duration::from_secs(1));
        assert!(matches!(r, Err(SinkError::Gone(..))), "{r:?}");
        // An empty that never comes: the deadline, not forever
        assert!(wait_drained(|| false, || None, Duration::from_millis(20)).is_ok());
    }

    /// An underrun is a glitch, not an unplug: the zone must not pause for it.
    #[test]
    fn only_a_real_disconnect_ends_the_stream() {
        for dac in [false, true] {
            assert!(!ends_stream(StreamFault::Xrun, dac), "an underrun never ends it");
            assert!(!ends_stream(StreamFault::RealtimeDenied, dac));
            // Any other fault with a DAC: the stream stopped, so the sink stops (Claude L2)
            assert_eq!(ends_stream(StreamFault::Other, dac), dac);
            assert!(ends_stream(StreamFault::DeviceGone, dac));
            assert!(ends_stream(StreamFault::Invalidated, dac));
        }
        assert!(
            ends_stream(StreamFault::Rerouted, true),
            "moved off the DAC (to the speaker)"
        );
        assert!(
            !ends_stream(StreamFault::Rerouted, false),
            "the phone's own reroutes go on"
        );
    }

    fn stalled() -> SinkError {
        SinkError::Gone(OutputId::new("cpal", "desk"), "it took no audio".into())
    }

    /// A stream that stops taking audio without saying so never holds a write for good.
    #[test]
    fn a_stream_that_takes_nothing_ends_the_wait() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(wait_for_room(
                || false,
                || false,
                || None,
                || 7,
                Duration::from_millis(50),
                stalled,
            ));
        });
        let r = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the wait ends when the stream takes nothing");
        assert!(matches!(r, Err(SinkError::Gone(..))), "{r:?}");
    }

    /// A stream that plays, however slowly, is waited for: only a stretch with nothing taken counts.
    #[test]
    fn a_stream_that_still_takes_audio_is_waited_for() {
        let taken = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let room = Arc::new(AtomicBool::new(false));
        let (t, r2) = (taken.clone(), room.clone());
        // Takes a little every 50 ms for 1.5 s (longer than the 1 s stall), then there is room
        std::thread::spawn(move || {
            for _ in 0..30 {
                std::thread::sleep(Duration::from_millis(50));
                t.fetch_add(1, Ordering::SeqCst);
            }
            r2.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        let r = wait_for_room(
            || room.load(Ordering::SeqCst),
            || false,
            || None,
            || taken.load(Ordering::SeqCst),
            Duration::from_secs(1),
            stalled,
        );
        assert!(r.is_ok(), "{r:?}");
        assert!(started.elapsed() >= Duration::from_millis(1200));
    }

    /// A machine that sleeps mid-song: the time asleep is not a stall (the stream had no chance to
    /// play), so a stream that takes audio again on waking plays on.
    #[test]
    fn time_asleep_is_not_a_stall() {
        let looks = std::sync::atomic::AtomicU32::new(0);
        let started = Instant::now();
        let r = wait_for_room(
            // Room half a second after the sleep below
            || started.elapsed() >= Duration::from_millis(2000),
            || false,
            || None,
            || {
                // The third look comes 1.5 s late, as after a sleep
                if looks.fetch_add(1, Ordering::SeqCst) == 2 {
                    std::thread::sleep(Duration::from_millis(1500));
                }
                0
            },
            Duration::from_millis(1200),
            stalled,
        );
        assert!(r.is_ok(), "{r:?}");
    }

    #[test]
    fn a_stream_that_ended_by_itself_is_gone_without_a_dac_too() {
        let id = OutputId::new("cpal", "desk");
        assert!(ended_by_itself(false, &id).is_none());
        assert!(matches!(ended_by_itself(true, &id), Some(SinkError::Gone(..))));
    }
}
