//! The exclusive USB audio driver: the DAC's clock and playback setting, isochronous packets sized
//! by the DAC's feedback, and its feature unit's volume. It speaks to the device through a
//! [`Transport`] (the kernel's usbfs on the file descriptor Android hands over: [`crate::usbfs`]),
//! so the whole state machine runs and is tested on any machine with a fake one.
//!
//! Opening a DAC: claim its AudioControl and AudioStreaming interfaces, the streaming interface
//! to its zero-bandwidth setting, then (UAC2) set the clock source's rate and read it back, then
//! the playback setting; UAC1 sets the rate on the endpoint once the setting is chosen. Playing:
//! each transfer carries a few milliseconds of packets, one per service interval, as many frames
//! each as the schedule gives; an asynchronous DAC's feedback moves the schedule. Closing: the
//! zero-bandwidth setting again, the interfaces released.

use crate::quirks::Quirks;
use crate::requests::{self, ControlRequest};
use crate::schedule::{decode_feedback, packets_per_second, Schedule, Speed};
use crate::stream::Setting;
use crate::volume::VolumeRange;
use crate::{Sync, Uac};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Why a transfer or request failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsbError {
    /// The device went away (unplugged).
    Gone,
    Other(String),
}

/// An isochronous transfer to submit: its packets' lengths, and (OUT) their bytes one after
/// another, or (IN) room for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Urb {
    pub tag: u64,
    pub endpoint: u8,
    pub data: Vec<u8>,
    pub packets: Vec<u32>,
}

/// A finished transfer: its status (0, or a negative errno), its buffer and each packet's actual
/// length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reaped {
    pub tag: u64,
    pub endpoint: u8,
    pub status: i32,
    pub data: Vec<u8>,
    pub actual: Vec<u32>,
}

/// `-ENODEV`, `-ESHUTDOWN`: a transfer on a device that went.
pub const STATUS_GONE: [i32; 2] = [-19, -108];

/// The device, as the driver needs it. Calls may come from two threads (the audio thread plays,
/// another sets the volume): implementations synchronise inside.
pub trait Transport: Send + std::marker::Sync {
    /// The bus speed, when the transport knows it.
    fn speed(&self) -> Option<Speed>;
    fn claim(&self, interface: u8) -> Result<(), UsbError>;
    fn release(&self, interface: u8);
    fn set_alt(&self, interface: u8, alt: u8) -> Result<(), UsbError>;
    /// A control transfer: what it read (device to host), or nothing.
    fn control(&self, req: &ControlRequest) -> Result<Vec<u8>, UsbError>;
    fn submit(&self, urb: Urb) -> Result<(), UsbError>;
    /// The next finished transfer, waiting up to `timeout`.
    fn reap(&self, timeout: Duration) -> Result<Option<Reaped>, UsbError>;
    /// Cancel a submitted transfer: it is still reaped (with an error status).
    fn discard(&self, tag: u64);
}

/// Why the driver stopped or did not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverError {
    /// The DAC went away.
    Gone,
    /// The clock read back another rate than the one set (`None`: it would not say).
    Rate {
        asked: u32,
        got: Option<u32>,
    },
    Usb(String),
}

impl std::fmt::Display for DriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DriverError::Gone => f.write_str("the DAC went away"),
            DriverError::Rate { asked, got: Some(g) } => {
                write!(f, "the DAC's clock runs at {g} Hz, not {asked} Hz")
            }
            DriverError::Rate { asked, got: None } => {
                write!(f, "the DAC did not confirm {asked} Hz")
            }
            DriverError::Usb(e) => write!(f, "USB: {e}"),
        }
    }
}

impl From<UsbError> for DriverError {
    fn from(e: UsbError) -> Self {
        match e {
            UsbError::Gone => DriverError::Gone,
            UsbError::Other(s) => DriverError::Usb(s),
        }
    }
}

/// What to play, on what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub uac: Uac,
    /// The AudioControl interface.
    pub control_interface: u8,
    pub setting: Setting,
    pub rate: u32,
    pub speed: Speed,
    pub quirks: Quirks,
}

/// Milliseconds of audio in one transfer.
pub const URB_MS: u32 = 4;
/// Transfers in flight at most: [`URB_MS`] times this is the buffer (128 ms).
pub const MAX_URBS: usize = 32;
/// A device that finishes nothing for this long has stopped.
const STALL: Duration = Duration::from_millis(1000);

/// The driver for one DAC, playing one format at a time.
pub struct Driver {
    t: Arc<dyn Transport>,
    plan: Plan,
    schedule: Schedule,
    pending: Vec<u8>,
    /// OUT transfers in flight: tag and frames.
    in_flight: VecDeque<(u64, u64)>,
    /// The feedback transfer in flight.
    feedback: Option<u64>,
    next_tag: u64,
    readback: Option<u32>,
    /// A transfer finished without error since the stream started.
    running: bool,
    submitted: u64,
    played: u64,
    claimed: Vec<u8>,
    open: bool,
}

impl Driver {
    /// Take the DAC over and set it up for `plan`: interfaces claimed, the rate set and read
    /// back, the playback setting chosen, feedback started.
    pub fn open(t: Arc<dyn Transport>, plan: Plan) -> Result<Self, DriverError> {
        let mut d = Driver {
            schedule: schedule_for(&plan),
            t,
            plan,
            pending: Vec::new(),
            in_flight: VecDeque::new(),
            feedback: None,
            next_tag: 1,
            readback: None,
            running: false,
            submitted: 0,
            played: 0,
            claimed: Vec::new(),
            open: true,
        };
        for i in [d.plan.control_interface, d.plan.setting.interface] {
            if !d.claimed.contains(&i) {
                if let Err(e) = d.t.claim(i) {
                    d.stop();
                    return Err(e.into());
                }
                d.claimed.push(i);
            }
        }
        if let Err(e) = d.start() {
            d.stop();
            return Err(e);
        }
        Ok(d)
    }

    /// The zero-bandwidth setting, the rate, the playback setting, feedback.
    fn start(&mut self) -> Result<(), DriverError> {
        let s = self.plan.setting.clone();
        self.t.set_alt(s.interface, 0)?;
        let rate = self.plan.rate;
        self.readback = None;
        if self.plan.uac == Uac::V2 {
            if let Some(clock) = s.clock {
                let ac = self.plan.control_interface;
                self.t.control(&requests::uac2_set_rate(clock, ac, rate))?;
                self.readback = self.read_rate(&requests::uac2_get_rate(clock, ac))?;
                if self.readback != Some(rate) && !self.plan.quirks.late_rate_readback {
                    return Err(DriverError::Rate {
                        asked: rate,
                        got: self.readback,
                    });
                }
            }
            self.t.set_alt(s.interface, s.alt)?;
        } else {
            // UAC1: the rate belongs to the endpoint of the chosen setting
            self.t.set_alt(s.interface, s.alt)?;
            let ep = s.endpoint.address;
            self.t.control(&requests::uac1_set_rate(ep, rate))?;
            // Some UAC1 DACs don't answer GET_CUR: then nothing is confirmed
            self.readback = self.read_rate(&requests::uac1_get_rate(ep))?;
        }
        self.schedule = schedule_for(&self.plan);
        self.running = false;
        self.submit_feedback()?;
        Ok(())
    }

    /// A rate read back; a refused request is no answer (not an error), a gone device is.
    fn read_rate(&self, req: &ControlRequest) -> Result<Option<u32>, DriverError> {
        match self.t.control(req) {
            Ok(b) => Ok(requests::parse_rate(&b)),
            Err(UsbError::Gone) => Err(DriverError::Gone),
            Err(UsbError::Other(_)) => Ok(None),
        }
    }

    /// The rate the clock read back after it was set.
    pub fn readback(&self) -> Option<u32> {
        self.readback
    }

    /// The format plays as asked: the clock read back exactly its rate, and the stream runs.
    pub fn confirmed(&self) -> bool {
        self.readback == Some(self.plan.rate) && self.running
    }

    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    /// Frames handed in and not played yet.
    pub fn queued_frames(&self) -> u64 {
        let fb = self.plan.setting.frame_bytes().max(1) as u64;
        self.submitted.saturating_sub(self.played) + self.pending.len() as u64 / fb
    }

    /// Frames the buffer holds at most.
    pub fn capacity_frames(&self) -> u64 {
        u64::from(self.plan.rate) * u64::from(URB_MS) * MAX_URBS as u64 / 1000
    }

    /// Frames the DAC has taken.
    pub fn played_frames(&self) -> u64 {
        self.played
    }

    fn tag(&mut self) -> u64 {
        let t = self.next_tag;
        self.next_tag += 1;
        t
    }

    fn submit_feedback(&mut self) -> Result<(), DriverError> {
        let ep = &self.plan.setting.endpoint;
        let (Sync::Async, Some(address)) = (ep.sync, ep.feedback) else {
            return Ok(());
        };
        let len = match (self.plan.speed, self.plan.quirks.four_byte_feedback_fs) {
            (Speed::Full, false) => 3,
            _ => 4,
        };
        let tag = self.tag();
        self.t.submit(Urb {
            tag,
            endpoint: address,
            data: vec![0; len],
            packets: vec![len as u32],
        })?;
        self.feedback = Some(tag);
        Ok(())
    }

    /// Packets per transfer.
    fn urb_packets(&self) -> usize {
        let pps = packets_per_second(self.plan.speed, self.plan.setting.endpoint.interval);
        ((pps * URB_MS / 1000).max(1) as usize).min(128)
    }

    /// Submit one transfer of the queued bytes, when there are enough for it (`partial`: whatever
    /// is left). Returns whether it did.
    fn submit_one(&mut self, partial: bool) -> Result<bool, DriverError> {
        let fb = self.plan.setting.frame_bytes();
        if fb == 0 || self.pending.len() < fb {
            return Ok(false);
        }
        let max_frames = (usize::from(self.plan.setting.endpoint.max_packet) / fb).max(1) as u32;
        let mut sched = self.schedule.clone();
        let mut packets = Vec::with_capacity(self.urb_packets());
        let mut bytes = 0usize;
        let mut have = self.pending.len() / fb * fb;
        for _ in 0..self.urb_packets() {
            let frames = sched.next_packet().min(max_frames) as usize;
            let want = frames * fb;
            if want > have {
                if !partial {
                    return Ok(false);
                }
                if have > 0 {
                    packets.push(have as u32);
                    bytes += have;
                }
                break;
            }
            packets.push(want as u32);
            bytes += want;
            have -= want;
        }
        if packets.is_empty() {
            return Ok(false);
        }
        self.schedule = sched;
        let data: Vec<u8> = self.pending.drain(..bytes).collect();
        let frames = (bytes / fb) as u64;
        let tag = self.tag();
        self.t.submit(Urb {
            tag,
            endpoint: self.plan.setting.endpoint.address,
            data,
            packets,
        })?;
        self.in_flight.push_back((tag, frames));
        self.submitted += frames;
        Ok(true)
    }

    /// Handle one finished transfer, waiting up to `timeout`. Returns whether one came.
    fn reap_one(&mut self, timeout: Duration) -> Result<bool, DriverError> {
        let Some(r) = self.t.reap(timeout)? else {
            return Ok(false);
        };
        if STATUS_GONE.contains(&r.status) {
            return Err(DriverError::Gone);
        }
        if Some(r.tag) == self.feedback {
            self.feedback = None;
            if r.status == 0 {
                let n = r.actual.first().copied().unwrap_or(0) as usize;
                let got = r.data.get(..n.min(r.data.len())).unwrap_or(&[]);
                if let Some(v) = decode_feedback(got, self.plan.speed, self.plan.quirks.four_byte_feedback_fs)
                {
                    self.schedule.follow(v);
                }
            }
            if self.open {
                self.submit_feedback()?;
            }
        } else if let Some(pos) = self.in_flight.iter().position(|(t, _)| *t == r.tag) {
            let (_, frames) = self.in_flight.remove(pos).expect("found");
            self.played += frames;
            if r.status == 0 {
                self.running = true;
            }
        }
        Ok(true)
    }

    /// Wait until `done`, reaping; an error when the DAC finishes no audio for [`STALL`].
    fn wait(&mut self, done: impl Fn(&Self) -> bool) -> Result<(), DriverError> {
        let mut since = Instant::now();
        let mut played = self.played;
        while !done(self) {
            self.reap_one(Duration::from_millis(50))?;
            if self.played != played {
                played = self.played;
                since = Instant::now();
            } else if since.elapsed() > STALL {
                return Err(DriverError::Usb("the DAC stopped taking audio".into()));
            }
        }
        Ok(())
    }

    /// Queue `bytes` (whole frames on the wire: [`crate::stream::pack`]), blocking until they fit.
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), DriverError> {
        self.pending.extend_from_slice(bytes);
        loop {
            while self.in_flight.len() < MAX_URBS {
                if !self.submit_one(false)? {
                    break;
                }
            }
            if self.in_flight.len() < MAX_URBS {
                break;
            }
            // Full: wait for the DAC to take some
            self.wait(|d| d.in_flight.len() < MAX_URBS)?;
        }
        // What already finished, without waiting (feedback comes back once per interval: bounded)
        for _ in 0..self.in_flight.len() + 2 {
            if !self.reap_one(Duration::ZERO)? {
                break;
            }
        }
        Ok(())
    }

    /// Play out what is queued, then return.
    pub fn drain(&mut self) -> Result<(), DriverError> {
        while self.submit_one(true)? {}
        self.pending.clear();
        self.wait(|d| d.in_flight.is_empty())
    }

    /// Discard what is queued and in flight.
    pub fn flush(&mut self) -> Result<(), DriverError> {
        self.pending.clear();
        for (tag, _) in self.in_flight.clone() {
            self.t.discard(tag);
        }
        let since = Instant::now();
        while !self.in_flight.is_empty() && since.elapsed() < STALL {
            self.reap_one(Duration::from_millis(50))?;
        }
        self.in_flight.clear();
        self.played = self.submitted;
        Ok(())
    }

    /// Play another rate or setting on the same DAC: the stream stops, the clock is set and read
    /// back again, the new setting starts.
    pub fn retune(&mut self, rate: u32, setting: Setting) -> Result<(), DriverError> {
        if !self.open {
            return Err(DriverError::Usb("the DAC was let go".into()));
        }
        self.flush()?;
        self.stop_feedback();
        if !self.claimed.contains(&setting.interface) {
            self.t.claim(setting.interface)?;
            self.claimed.push(setting.interface);
        }
        if setting.interface != self.plan.setting.interface {
            self.t.set_alt(self.plan.setting.interface, 0)?;
        }
        self.plan.rate = rate;
        self.plan.setting = setting;
        self.start()
    }

    fn stop_feedback(&mut self) {
        if let Some(tag) = self.feedback.take() {
            self.t.discard(tag);
            let since = Instant::now();
            // Its completion, so a later reap doesn't take it for something else
            while since.elapsed() < Duration::from_millis(200) {
                match self.t.reap(Duration::from_millis(20)) {
                    Ok(Some(r)) if r.tag == tag => break,
                    Ok(Some(r)) => {
                        if let Some(pos) = self.in_flight.iter().position(|(t, _)| *t == r.tag) {
                            self.in_flight.remove(pos);
                        }
                    }
                    Ok(None) => {}
                    Err(_) => break,
                }
            }
        }
    }

    /// Between songs: the stream stops and the zero-bandwidth setting is set, the DAC stays
    /// claimed (Android's driver doesn't take it back); [`Driver::retune`] starts it again.
    pub fn idle(&mut self) -> Result<(), DriverError> {
        self.flush()?;
        self.stop_feedback();
        self.running = false;
        self.t.set_alt(self.plan.setting.interface, 0)?;
        Ok(())
    }

    /// Stop for good, the interfaces still claimed (their owner gives them back, all together).
    pub fn stop(&mut self) {
        if !self.open {
            return;
        }
        let _ = self.flush();
        self.open = false;
        self.stop_feedback();
        let _ = self.t.set_alt(self.plan.setting.interface, 0);
    }

    /// Let the DAC go: [`Driver::stop`], then the interfaces released (back to Android's driver),
    /// the AudioControl interface last, so the driver that probes it finds the rest free.
    pub fn close(&mut self) {
        self.stop();
        for i in self.claimed.drain(..).rev() {
            self.t.release(i);
        }
    }
}

/// Dropped: the stream stops; the interfaces stay claimed for their owner to give back
/// ([`Driver::close`] does both).
impl Drop for Driver {
    fn drop(&mut self) {
        self.stop();
    }
}

fn schedule_for(plan: &Plan) -> Schedule {
    Schedule::new(plan.rate, plan.speed, plan.setting.endpoint.interval)
}

/// The feature unit's volume range on the master channel (`None`: it has none, or won't say).
pub fn volume_range(t: &dyn Transport, unit: u8, ac: u8) -> Result<Option<VolumeRange>, UsbError> {
    match t.control(&requests::uac2_get_volume_range(unit, ac, 0)) {
        Ok(b) => Ok(requests::parse_volume_range(&b)),
        Err(UsbError::Gone) => Err(UsbError::Gone),
        Err(UsbError::Other(_)) => Ok(None),
    }
}

/// The feature unit's master volume now (`None`: it won't say).
pub fn get_volume(t: &dyn Transport, unit: u8, ac: u8) -> Result<Option<i16>, UsbError> {
    match t.control(&requests::uac2_get_volume(unit, ac, 0)) {
        Ok(b) if b.len() >= 2 => Ok(Some(i16::from_le_bytes([b[0], b[1]]))),
        Ok(_) | Err(UsbError::Other(_)) => Ok(None),
        Err(UsbError::Gone) => Err(UsbError::Gone),
    }
}

/// Set the feature unit's master volume (1/256 dB) and unmute it.
pub fn set_volume(t: &dyn Transport, unit: u8, ac: u8, cur: i16) -> Result<(), UsbError> {
    t.control(&requests::uac2_set_volume(unit, ac, 0, cur))?;
    t.control(&requests::uac2_set_mute(unit, ac, 0, false))
        .map(|_| ())
}

/// Mute or unmute the feature unit.
pub fn set_mute(t: &dyn Transport, unit: u8, ac: u8, mute: bool) -> Result<(), UsbError> {
    t.control(&requests::uac2_set_mute(unit, ac, 0, mute)).map(|_| ())
}

/// The clock's rates (`RANGE`), as a list of the common ones.
pub fn clock_rates(t: &dyn Transport, clock: u8, ac: u8) -> Result<Vec<u32>, UsbError> {
    match t.control(&requests::uac2_get_rate_ranges(clock, ac)) {
        Ok(b) => Ok(crate::rates_in(&crate::parse_rate_ranges(&b))),
        Err(UsbError::Gone) => Err(UsbError::Gone),
        Err(UsbError::Other(_)) => Ok(Vec::new()),
    }
}

/// A transport that remembers what was asked of it and answers like a DAC: for tests here and in
/// the crates that use the driver.
pub mod fake {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    pub struct State {
        /// Calls in order: "claim 0", "alt 1 0", "set rate 96000", "get rate", "volume -2560"...
        pub log: Vec<String>,
        /// The clock's rate (set by SET_CUR).
        pub clock: u32,
        /// Read back this instead of the clock, when set.
        pub readback: Option<u32>,
        /// The feedback the DAC sends (16.16 or 10.14 bytes as given), when any.
        pub feedback: Option<Vec<u8>>,
        /// OUT transfers submitted, finished or not.
        pub out: Vec<Urb>,
        /// Submitted and not yet reaped.
        pub queue: VecDeque<Urb>,
        /// Hold OUT transfers (the DAC takes nothing).
        pub stall: bool,
        /// Unplugged: everything fails with Gone.
        pub gone: bool,
        pub volume: Option<i16>,
        pub muted: Option<bool>,
        /// The volume `RANGE` reply.
        pub volume_range: Option<Vec<u8>>,
        pub rate_ranges: Option<Vec<u8>>,
        pub speed: Option<Speed>,
        pub discarded: Vec<u64>,
        /// OUT transfers finish with this status (e.g. -19, ENODEV: unplugged mid-transfer).
        pub out_status: i32,
    }

    #[derive(Default, Clone)]
    pub struct FakeDac(pub Arc<Mutex<State>>);

    impl FakeDac {
        pub fn high_speed() -> Self {
            let f = FakeDac::default();
            f.state().speed = Some(Speed::High);
            f
        }

        pub fn state(&self) -> std::sync::MutexGuard<'_, State> {
            self.0.lock().unwrap_or_else(|e| e.into_inner())
        }

        fn check(&self) -> Result<std::sync::MutexGuard<'_, State>, UsbError> {
            let s = self.state();
            if s.gone {
                Err(UsbError::Gone)
            } else {
                Ok(s)
            }
        }
    }

    impl Transport for FakeDac {
        fn speed(&self) -> Option<Speed> {
            self.state().speed
        }

        fn claim(&self, interface: u8) -> Result<(), UsbError> {
            self.check()?.log.push(format!("claim {interface}"));
            Ok(())
        }

        fn release(&self, interface: u8) {
            self.state().log.push(format!("release {interface}"));
        }

        fn set_alt(&self, interface: u8, alt: u8) -> Result<(), UsbError> {
            let mut s = self.check()?;
            s.log.push(format!("alt {interface} {alt}"));
            Ok(())
        }

        fn control(&self, req: &ControlRequest) -> Result<Vec<u8>, UsbError> {
            let mut s = self.check()?;
            let target = req.value >> 8;
            match (req.request_type, req.request, target) {
                // UAC2 clock (or UAC1 endpoint) rate
                (0x21 | 0x22, 0x01, 0x01)
                    if req.data.len() >= 3 && (req.request_type == 0x22 || req.length == 4) =>
                {
                    let mut b = [0u8; 4];
                    b[..req.data.len().min(4)].copy_from_slice(&req.data[..req.data.len().min(4)]);
                    s.clock = u32::from_le_bytes(b);
                    let line = format!("set rate {}", s.clock);
                    s.log.push(line);
                    Ok(Vec::new())
                }
                (0xa1 | 0xa2, 0x01 | 0x81, 0x01) => {
                    s.log.push("get rate".into());
                    let r = s.readback.unwrap_or(s.clock);
                    Ok(r.to_le_bytes()[..usize::from(req.length.min(4))].to_vec())
                }
                (0xa1, 0x02, 0x01) => {
                    s.log.push("get rate ranges".into());
                    s.rate_ranges.clone().ok_or(UsbError::Other("stall".into()))
                }
                // Feature unit
                (0x21, 0x01, 0x02) => {
                    let v = i16::from_le_bytes([req.data[0], req.data[1]]);
                    s.volume = Some(v);
                    s.log.push(format!("volume {v}"));
                    Ok(Vec::new())
                }
                (0x21, 0x01, 0x01) => {
                    let m = req.data.first() == Some(&1);
                    s.muted = Some(m);
                    s.log.push(format!("mute {m}"));
                    Ok(Vec::new())
                }
                (0xa1, 0x01, 0x02) => {
                    s.log.push("get volume".into());
                    s.volume
                        .map(|v| v.to_le_bytes().to_vec())
                        .ok_or(UsbError::Other("stall".into()))
                }
                (0xa1, 0x02, 0x02) => {
                    s.log.push("get volume range".into());
                    s.volume_range.clone().ok_or(UsbError::Other("stall".into()))
                }
                _ => Err(UsbError::Other(format!("unexpected request {req:?}"))),
            }
        }

        fn submit(&self, urb: Urb) -> Result<(), UsbError> {
            let mut s = self.check()?;
            if urb.endpoint & 0x80 == 0 {
                s.out.push(urb.clone());
            }
            s.queue.push_back(urb);
            Ok(())
        }

        fn reap(&self, timeout: Duration) -> Result<Option<Reaped>, UsbError> {
            let mut s = self.check()?;
            // OUT transfers finish in order; while stalled only discarded ones and feedback do
            let pos = s
                .queue
                .iter()
                .position(|u| u.endpoint & 0x80 != 0 || !s.stall || s.discarded.contains(&u.tag));
            let Some(pos) = pos else {
                drop(s);
                std::thread::sleep(timeout.min(Duration::from_millis(5)));
                return Ok(None);
            };
            let u = s.queue.remove(pos).expect("found");
            let discarded = s.discarded.contains(&u.tag);
            let status = if discarded { -2 } else { s.out_status };
            if u.endpoint & 0x80 != 0 {
                let fb = s.feedback.clone().unwrap_or_default();
                let n = fb.len().min(u.data.len());
                let mut data = u.data.clone();
                data[..n].copy_from_slice(&fb[..n]);
                return Ok(Some(Reaped {
                    tag: u.tag,
                    endpoint: u.endpoint,
                    status: if s.feedback.is_some() && !discarded { 0 } else { -2 },
                    data,
                    actual: vec![n as u32],
                }));
            }
            Ok(Some(Reaped {
                tag: u.tag,
                endpoint: u.endpoint,
                status,
                actual: u.packets.clone(),
                data: Vec::new(),
            }))
        }

        fn discard(&self, tag: u64) {
            self.state().discarded.push(tag);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeDac;
    use super::*;
    use crate::{Endpoint, Uac};

    fn setting(alt: u8, subslot: u8, bits: u8) -> Setting {
        Setting {
            interface: 1,
            alt,
            channels: 2,
            subslot_bytes: subslot,
            bits,
            endpoint: Endpoint {
                address: 0x01,
                sync: Sync::Async,
                max_packet: 0x308,
                interval: 1,
                feedback: Some(0x81),
            },
            clock: Some(0x29),
        }
    }

    fn plan(rate: u32) -> Plan {
        Plan {
            uac: Uac::V2,
            control_interface: 0,
            setting: setting(1, 4, 32),
            rate,
            speed: Speed::High,
            quirks: Quirks::default(),
        }
    }

    fn open(f: &FakeDac, rate: u32) -> Result<Driver, DriverError> {
        Driver::open(Arc::new(f.clone()), plan(rate))
    }

    #[test]
    fn open_sets_the_clock_reads_it_back_then_the_setting() {
        let f = FakeDac::high_speed();
        let d = open(&f, 96_000).unwrap();
        assert_eq!(
            f.state().log,
            [
                "claim 0",
                "claim 1",
                "alt 1 0",
                "set rate 96000",
                "get rate",
                "alt 1 1"
            ]
        );
        assert_eq!(d.readback(), Some(96_000));
        assert!(!d.confirmed(), "not before the stream runs");
        // The feedback transfer is waiting
        assert_eq!(f.state().queue.len(), 1);
        assert_eq!(f.state().queue[0].endpoint, 0x81);
    }

    #[test]
    fn a_clock_that_reads_back_another_rate_is_refused_and_let_go() {
        let f = FakeDac::high_speed();
        f.state().readback = Some(48_000);
        let e = open(&f, 96_000).err().unwrap();
        assert_eq!(
            e,
            DriverError::Rate {
                asked: 96_000,
                got: Some(48_000)
            }
        );
        assert_eq!(e.to_string(), "the DAC's clock runs at 48000 Hz, not 96000 Hz");
        let log = f.state().log.clone();
        assert!(!log.contains(&"alt 1 1".to_string()), "{log:?}");
        // Stopped; the interfaces are their owner's to give back
        assert_eq!(log.last().map(String::as_str), Some("alt 1 0"), "{log:?}");
        assert!(!log.iter().any(|l| l.starts_with("release")), "{log:?}");
    }

    #[test]
    fn running_packets_follow_the_feedback() {
        let f = FakeDac::high_speed();
        // 12.1 frames a microframe at 96 kHz: the DAC runs a little fast
        let fb = ((12u64 << 16) + (1u64 << 16) / 10) as u32;
        f.state().feedback = Some(fb.to_le_bytes().to_vec());
        let mut d = open(&f, 96_000).unwrap();
        // One transfer's worth (32 microframes of 12 frames, 8 bytes each): the feedback comes back
        d.write(&vec![0u8; 32 * 12 * 8]).unwrap();
        assert!(d.confirmed(), "the clock read back and transfers finished");
        // Then 100 ms more
        d.write(&vec![0u8; 9_600 * 8]).unwrap();
        let s = f.state();
        let first = &s.out[0];
        assert_eq!(first.packets.len(), 32, "4 ms of microframes");
        assert!(
            first.packets.iter().all(|p| *p == 12 * 8),
            "nominal before feedback: {:?}",
            first.packets
        );
        // After the feedback: one more frame about every ten packets
        let later: u32 = s.out[2].packets.iter().sum::<u32>() / 8;
        assert!(later == 32 * 12 + 3 || later == 32 * 12 + 4, "{later}");
        // Every packet's bytes are whole frames within the endpoint's size
        for u in &s.out {
            assert_eq!(u.data.len() as u32, u.packets.iter().sum::<u32>());
            assert!(u.packets.iter().all(|p| p % 8 == 0 && *p <= 0x308));
        }
    }

    #[test]
    fn full_speed_sends_a_packet_a_millisecond() {
        let f = FakeDac::default();
        f.state().feedback = Some({
            let v = (44.1f64 * 16384.0).round() as u32;
            v.to_le_bytes()[..3].to_vec()
        });
        let mut p = plan(44_100);
        p.speed = Speed::Full;
        p.setting = setting(2, 2, 16);
        let mut d = Driver::open(Arc::new(f.clone()), p).unwrap();
        d.write(&vec![0u8; 44_100 * 4]).unwrap();
        d.drain().unwrap();
        let s = f.state();
        assert_eq!(s.out[0].packets.len(), 4);
        let frames: u32 = s.out.iter().flat_map(|u| u.packets.iter()).sum::<u32>() / 4;
        assert_eq!(frames, 44_100, "every frame, once");
        assert!(s
            .out
            .iter()
            .flat_map(|u| u.packets.iter())
            .all(|p| *p == 44 * 4 || *p == 45 * 4 || *p < 44 * 4));
    }

    #[test]
    fn a_full_buffer_blocks_until_the_dac_takes_some() {
        let f = FakeDac::high_speed();
        let mut d = open(&f, 96_000).unwrap();
        f.state().stall = true;
        // More than the buffer holds: the write waits, then gives up on a DAC that takes nothing
        let start = Instant::now();
        let e = d.write(&vec![0u8; 96_000 * 8]).unwrap_err();
        assert_eq!(e, DriverError::Usb("the DAC stopped taking audio".into()));
        assert!(start.elapsed() >= STALL);
        assert_eq!(f.state().out.len(), MAX_URBS);
        assert!(d.queued_frames() > 0 && d.queued_frames() >= d.capacity_frames());
    }

    #[test]
    fn a_rate_change_stops_sets_and_reads_back_again() {
        let f = FakeDac::high_speed();
        let mut d = open(&f, 96_000).unwrap();
        d.write(&vec![0u8; 9_600 * 8]).unwrap();
        f.state().log.clear();
        d.retune(44_100, setting(2, 2, 16)).unwrap();
        let log = f.state().log.clone();
        assert_eq!(log, ["alt 1 0", "set rate 44100", "get rate", "alt 1 2"]);
        assert_eq!(d.readback(), Some(44_100));
        assert!(!d.confirmed(), "not running yet at the new rate");
        d.write(&vec![0u8; 4_410 * 4]).unwrap();
        assert!(d.confirmed());
        let last = f.state().out.last().cloned().unwrap();
        assert!(
            last.packets.iter().all(|p| *p == 5 * 4 || *p == 6 * 4),
            "{:?}",
            last.packets
        );
    }

    #[test]
    fn unplugging_is_gone_everywhere() {
        let f = FakeDac::high_speed();
        let mut d = open(&f, 96_000).unwrap();
        d.write(&vec![0u8; 9_600 * 8]).unwrap();
        f.state().gone = true;
        assert_eq!(d.write(&vec![0u8; 96_000 * 8]).unwrap_err(), DriverError::Gone);
        assert_eq!(d.drain().unwrap_err(), DriverError::Gone);
        // Opening a gone DAC is Gone too
        assert_eq!(open(&f, 96_000).err(), Some(DriverError::Gone));
        // A transfer that finishes with ENODEV is Gone
        let g = FakeDac::high_speed();
        let mut d = open(&g, 96_000).unwrap();
        g.state().out_status = -19;
        assert_eq!(d.write(&vec![0u8; 9_600 * 8]).unwrap_err(), DriverError::Gone);
    }

    #[test]
    fn between_songs_the_dac_stays_claimed() {
        let f = FakeDac::high_speed();
        let mut d = open(&f, 96_000).unwrap();
        d.write(&vec![0u8; 9_600 * 8]).unwrap();
        f.state().log.clear();
        d.idle().unwrap();
        assert_eq!(f.state().log, ["alt 1 0"]);
        assert!(!d.confirmed());
        d.retune(192_000, setting(1, 4, 32)).unwrap();
        d.write(&vec![0u8; 19_200 * 8]).unwrap();
        assert!(d.confirmed());
        assert!(!f.state().log.iter().any(|l| l.starts_with("release")));
    }

    #[test]
    fn close_lets_the_dac_go() {
        let f = FakeDac::high_speed();
        let mut d = open(&f, 192_000).unwrap();
        d.write(&vec![0u8; 19_200 * 8]).unwrap();
        f.state().log.clear();
        d.close();
        let log = f.state().log.clone();
        assert_eq!(log, ["alt 1 0", "release 1", "release 0"]);
        // Dropped after: nothing more
        drop(d);
        assert_eq!(f.state().log.len(), 3);
    }

    #[test]
    fn uac1_sets_the_rate_on_the_endpoint_after_the_setting() {
        let f = FakeDac::default();
        let mut p = plan(48_000);
        p.uac = Uac::V1;
        p.speed = Speed::Full;
        p.setting.clock = None;
        let d = Driver::open(Arc::new(f.clone()), p).unwrap();
        assert_eq!(
            f.state().log,
            [
                "claim 0",
                "claim 1",
                "alt 1 0",
                "alt 1 1",
                "set rate 48000",
                "get rate"
            ]
        );
        assert_eq!(d.readback(), Some(48_000));
    }

    #[test]
    fn the_feature_unit() {
        let f = FakeDac::high_speed();
        let mut range = 1u16.to_le_bytes().to_vec();
        range.extend((-127i16 * 256).to_le_bytes());
        range.extend(0i16.to_le_bytes());
        range.extend(128u16.to_le_bytes());
        f.state().volume_range = Some(range);
        let r = volume_range(&f, 0x0a, 0).unwrap().unwrap();
        assert_eq!(
            r,
            VolumeRange {
                min: -127 * 256,
                max: 0,
                res: 128
            }
        );
        set_volume(&f, 0x0a, 0, -20 * 256).unwrap();
        {
            let st = f.state();
            assert_eq!((st.volume, st.muted), (Some(-20 * 256), Some(false)));
        }
        set_mute(&f, 0x0a, 0, true).unwrap();
        assert_eq!(f.state().muted, Some(true));
        // No range: no hardware volume (not an error)
        let g = FakeDac::high_speed();
        assert_eq!(volume_range(&g, 0x0a, 0).unwrap(), None);
    }
}
