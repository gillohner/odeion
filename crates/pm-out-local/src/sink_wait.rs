//! The waits of the phone's shared (cpal) sink, apart from cpal so they are tested on every host:
//! waiting for room in its queue and for it to play out never outlasts a USB DAC that went away
//! (its stream stops taking audio, and the wait would never end), and a stream fault ends the
//! stream for the sink only when it really did (a disconnect, not an underrun).

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

/// Wait until `room()` (or `stopped()`); a `gone()` error ends the wait at once.
pub fn wait_for_room(
    room: impl Fn() -> bool,
    stopped: impl Fn() -> bool,
    gone: impl Fn() -> Option<SinkError>,
) -> Result<(), SinkError> {
    while !room() {
        if stopped() {
            return Ok(());
        }
        if let Some(e) = gone() {
            return Err(e);
        }
        std::thread::sleep(STEP);
    }
    Ok(())
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
            let _ = tx.send(wait_for_room(|| false, || false, g));
        });
        std::thread::sleep(Duration::from_millis(50));
        gone.store(true, Ordering::SeqCst);
        let r = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("the wait ends once the DAC is gone");
        assert!(matches!(r, Err(SinkError::Gone(..))), "{r:?}");
        // And as before: room ends it, a stop ends it
        assert!(wait_for_room(|| true, || false, || None).is_ok());
        assert!(wait_for_room(|| false, || true, || None).is_ok());
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
}
