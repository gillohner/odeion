//! Node-side `Output` for the desktop app's built-in player (`pm-endpoint`) over the `pm-stream`
//! protocol.
//!
//! [`NetOutput::connect`] asks an endpoint for its outputs (one `NetOutput`
//! per remote output, caps copied verbatim); `open` starts a dedicated TCP
//! session whose `write` sends a block without waiting for the endpoint's
//! `Ack` (its own thread collects them), as long as at most [`IN_FLIGHT`] of
//! audio is not acknowledged yet; past that it waits, so the remote DAC paces
//! the zone like a local one, a network round trip later. A group writes its
//! members one after another: waiting for each member's round trip on every
//! block would add them up, and starve the DACs on Wi-Fi. Endpoints are
//! never discovered: the app registers its player's address and key with
//! the node, and every connection shows that key.
//!
//! Each `Ack` reads the endpoint's clock: what its sink has written and still holds. An output
//! on a `hw:` DAC (ALSA) holds what the hardware has not played yet, so the node keeps such
//! outputs in time with each other in a group ([`OutputCaps::groupable`], ADR 0009); one behind a
//! system mixer (cpal: a phone, a Mac, a Windows PC) has latency nobody reports.

use async_trait::async_trait;
use pm_model::{AudioBlock, AudioFormat, OutputCaps, SampleKind, Volume};
use pm_stream::{expect_control, write_audio, write_frame, Frame, Key, StreamError};
use pm_zones::sink::check_block;
use pm_zones::{AudioSink, BufferLevel, Output, OutputDevice, OutputId, OutputInfo, SinkError};
use std::collections::HashMap;
use std::io::{BufReader, BufWriter};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, Instant};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Audio sent and not acknowledged yet, at most (at least one block is always sent).
pub const IN_FLIGHT: Duration = Duration::from_millis(100);
/// [`IN_FLIGHT`] for an endpoint reached through the node's tunnel (the app's player away from
/// home, [`KeyCell::through_tunnel`]): a mobile link's round trip is often 100-300 ms, and at most
/// one window of audio travels per round trip.
pub const IN_FLIGHT_TUNNEL: Duration = Duration::from_millis(1500);
/// The endpoint's Welcome comes within this, or the session fails; also its answer to a setting.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// The endpoint's answer to `Open` (its device may take a moment to start) comes within this.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(15);

/// One control frame from the endpoint within `within`; a silence reads as "did not answer".
fn answer_within(r: &mut BufReader<TcpStream>, within: Duration) -> Result<Frame, SinkError> {
    r.get_ref().set_read_timeout(Some(within))?;
    let f = expect_control(r).map_err(|e| match e {
        StreamError::Io(io)
            if matches!(
                io.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            SinkError::Device(format!(
                "the endpoint did not answer within {} s",
                within.as_secs()
            ))
        }
        other => io_err(other),
    });
    r.get_ref().set_read_timeout(None)?;
    f
}
/// The endpoint's answer to `GetVolume` (its level now) comes within this, or it is unknown.
const VOLUME_READ_TIMEOUT: Duration = Duration::from_secs(2);
/// An endpoint that answers nothing for this long is gone.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);

fn io_err(e: StreamError) -> SinkError {
    SinkError::Device(e.to_string())
}

/// The key of one endpoint, shared by all its outputs: when the app registers again with a new
/// key (it makes one each start), the zones that already play there use it too.
///
/// It also keeps the [`OutputDevice`] each of the endpoint's outputs reported in its last
/// `Welcome`: a phone's output describes the USB DAC plugged into it, and the app registers again
/// when that changes. Every [`NetOutput`] of the endpoint, also those zones already hold, reads it
/// from here. (Names stay as they were: a zone is named once, from its output's name.)
///
/// Sessions never wait for each other: each takes a number before it says Hello, and its Welcome
/// is applied only when no session that started later has been applied already, so an older
/// snapshot never overwrites a newer one (a DAC unplugged meanwhile), and a silent endpoint holds
/// up only its own session, for at most [`HANDSHAKE_TIMEOUT`].
#[derive(Clone, Default)]
pub struct KeyCell {
    key: Arc<RwLock<Key>>,
    devices: Arc<RwLock<Devices>>,
    /// The number the next session takes.
    next: Arc<std::sync::atomic::AtomicU64>,
    on_change: Arc<RwLock<Option<DeviceChanged>>>,
    /// Reached through the node's tunnel: the endpoint's name in its outputs' ids (its loopback
    /// relay's address changes with every start of the node; its key does not).
    tunnel: Arc<RwLock<Option<String>>>,
}

/// What the outputs last said, and the number of the session that said it.
#[derive(Default)]
struct Devices {
    applied: u64,
    by_output: HashMap<OutputId, Reported>,
}

/// One output's device and what it takes now, as its last applied `Welcome` said.
struct Reported {
    device: Option<OutputDevice>,
    caps: OutputCaps,
}

type DeviceChanged = Arc<dyn Fn(&OutputId) + Send + Sync>;

impl KeyCell {
    pub fn new(key: Key) -> Self {
        Self {
            key: Arc::new(RwLock::new(key)),
            ..Self::default()
        }
    }

    pub fn set(&self, key: Key) {
        *self.key.write().expect("endpoint key") = key;
    }

    /// The endpoint is reached through the node's tunnel, as `name` (`tunnel:<the app's key>`):
    /// its outputs' ids carry that name instead of the relay's address, and audio has
    /// [`IN_FLIGHT_TUNNEL`] on its way.
    pub fn through_tunnel(&self, name: &str) {
        *self.tunnel.write().expect("endpoint route") = Some(name.to_string());
    }

    /// The node's id for one of this endpoint's outputs, reached at `addr`.
    fn id_for(&self, addr: SocketAddr, remote: &OutputId) -> OutputId {
        match self.tunnel.read().expect("endpoint route").as_deref() {
            Some(name) => OutputId::new("net", &format!("{name}/{remote}")),
            None => net_id(addr, remote),
        }
    }

    fn in_flight(&self) -> Duration {
        if self.tunnel.read().expect("endpoint route").is_some() {
            IN_FLIGHT_TUNNEL
        } else {
            IN_FLIGHT
        }
    }

    pub fn get(&self) -> Key {
        self.key.read().expect("endpoint key").clone()
    }

    /// Call `f` with the node's id for an output (`net:<address>/<output>`) whenever it reports
    /// another device than before (the node tells its controllers). Replaces an earlier callback.
    pub fn on_device_change(&self, f: impl Fn(&OutputId) + Send + Sync + 'static) {
        *self.on_change.write().expect("endpoint callback") = Some(Arc::new(f));
    }

    /// Remember the devices the `Welcome` of session `seq` reports for the endpoint's outputs,
    /// unless a session that started later was applied already.
    fn report(&self, seq: u64, addr: SocketAddr, outputs: &[OutputInfo]) {
        let mut changed = Vec::new();
        {
            let mut d = self.devices.write().expect("endpoint devices");
            if seq < d.applied {
                tracing::debug!(seq, applied = d.applied, "an older Welcome, not applied");
                return;
            }
            d.applied = seq;
            let known = &mut d.by_output;
            for o in outputs {
                let before = known
                    .insert(
                        o.id.clone(),
                        Reported {
                            device: o.device.clone(),
                            caps: o.caps.clone(),
                        },
                    )
                    .map(|r| r.device);
                if before.as_ref() != Some(&o.device) {
                    tracing::info!(output = %o.id, device = ?o.device.as_ref().map(|d| &d.name), "endpoint output device");
                    // A new output without a device is no change anyone shows
                    if before.flatten() != o.device {
                        changed.push(self.id_for(addr, &o.id));
                    }
                }
            }
        }
        let f = self.on_change.read().expect("endpoint callback").clone();
        if let Some(f) = f {
            for id in &changed {
                f(id);
            }
        }
    }

    fn device(&self, id: &OutputId) -> Option<Option<OutputDevice>> {
        self.devices
            .read()
            .expect("endpoint devices")
            .by_output
            .get(id)
            .map(|r| r.device.clone())
    }

    fn caps(&self, id: &OutputId) -> Option<OutputCaps> {
        self.devices
            .read()
            .expect("endpoint devices")
            .by_output
            .get(id)
            .map(|r| r.caps.clone())
    }

    /// A session with the endpoint (blocking, at most [`CONNECT_TIMEOUT`] plus
    /// [`HANDSHAKE_TIMEOUT`] until its Welcome), the Welcome's devices applied in session order.
    fn session(&self, addr: SocketAddr) -> Result<(Conn, Frame), SinkError> {
        // Numbered before Hello: a session that starts later sees a newer endpoint
        let seq = self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let (c, welcome) = Conn::open(addr, self)?;
        if let Frame::Welcome { outputs, .. } = &welcome {
            self.report(seq, addr, outputs);
        }
        Ok((c, welcome))
    }
}

/// The node's id for an endpoint's output.
fn net_id(addr: SocketAddr, remote: &OutputId) -> OutputId {
    OutputId::new("net", &format!("{addr}/{remote}"))
}

struct Conn {
    r: BufReader<TcpStream>,
    w: BufWriter<TcpStream>,
}

impl Conn {
    fn open(addr: SocketAddr, key: &KeyCell) -> Result<(Self, Frame), SinkError> {
        let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
        stream.set_nodelay(true)?;
        let mut c = Conn {
            r: BufReader::new(stream.try_clone()?),
            w: BufWriter::new(stream),
        };
        write_frame(
            &mut c.w,
            &Frame::Hello {
                client: format!("pubky-music-node/{}", env!("CARGO_PKG_VERSION")),
                version: pm_stream::PROTOCOL_VERSION,
                key: key.get(),
            },
        )
        .map_err(io_err)?;
        // An endpoint that took the connection and says nothing: give up, never wait for good
        let welcome = answer_within(&mut c.r, HANDSHAKE_TIMEOUT)?;
        Ok((c, welcome))
    }
}

/// One output of one endpoint.
pub struct NetOutput {
    addr: SocketAddr,
    key: KeyCell,
    endpoint_name: String,
    remote: OutputInfo,
}

impl NetOutput {
    /// Ask the endpoint at `addr` for its outputs, showing it `key`.
    pub fn connect(addr: impl ToSocketAddrs, key: &KeyCell) -> Result<Vec<NetOutput>, SinkError> {
        let addr = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| SinkError::Device("endpoint address does not resolve".into()))?;
        let (_c, welcome) = key.session(addr)?;
        let Frame::Welcome {
            name,
            version,
            outputs,
        } = welcome
        else {
            return Err(SinkError::Device("endpoint did not send Welcome".into()));
        };
        if version != pm_stream::PROTOCOL_VERSION {
            let update = if version < pm_stream::PROTOCOL_VERSION {
                "update the Odeion app"
            } else {
                "update the node"
            };
            return Err(SinkError::Device(format!(
                "endpoint speaks protocol {version}, this node {}; {update}",
                pm_stream::PROTOCOL_VERSION
            )));
        }
        Ok(outputs
            .into_iter()
            .map(|mut remote| {
                // A hw: DAC reports its clock in every Ack; the node keeps it in time
                remote.caps.groupable = remote.id.0.starts_with("alsa:hw:");
                NetOutput {
                    addr,
                    key: key.clone(),
                    endpoint_name: name.clone(),
                    remote,
                }
            })
            .collect())
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn remote_id(&self) -> &OutputId {
        &self.remote.id
    }
}

#[async_trait]
impl Output for NetOutput {
    fn id(&self) -> OutputId {
        self.key.id_for(self.addr, &self.remote.id)
    }

    fn name(&self) -> String {
        format!("{} · {}", self.endpoint_name, self.remote.name)
    }

    /// What the endpoint last said is behind this output (a phone's USB DAC).
    fn device(&self) -> Option<OutputDevice> {
        self.key
            .device(&self.remote.id)
            .unwrap_or_else(|| self.remote.device.clone())
    }

    fn caps(&self) -> &OutputCaps {
        &self.remote.caps
    }

    /// What the endpoint last said the output takes (a phone's output takes its USB DAC's rates in
    /// bit-perfect mode).
    fn live_caps(&self) -> OutputCaps {
        self.key
            .caps(&self.remote.id)
            .unwrap_or_else(|| self.remote.caps.clone())
    }

    /// For one source: its own format when the device lists it (a phone's DAC in bit-perfect
    /// mode takes 96/24 as it is), else the closest listed one.
    fn live_caps_for(&self, src: AudioFormat) -> OutputCaps {
        let caps = self.live_caps();
        match self.device() {
            Some(d) if !d.formats.is_empty() => pm_zones::caps_for_source(&d.formats, src, &caps),
            _ => caps,
        }
    }

    /// Pass the listener's choices to the endpoint, then read its outputs again, so the node (and
    /// every controller) sees the device as it is now.
    async fn configure_device(&self, settings: pm_zones::DeviceSettings) -> Result<(), SinkError> {
        let (addr, key, remote) = (self.addr, self.key.clone(), self.remote.id.0.clone());
        blocking(move || {
            {
                let (mut c, _) = key.session(addr)?;
                write_frame(
                    &mut c.w,
                    &Frame::Configure {
                        output: remote,
                        settings,
                    },
                )
                .map_err(io_err)?;
                match answer_within(&mut c.r, HANDSHAKE_TIMEOUT) {
                    Ok(Frame::Done) => {}
                    Ok(other) => return Err(SinkError::Device(format!("unexpected {other:?}"))),
                    Err(e) => {
                        return Err(SinkError::Device(format!(
                            "the app did not take the setting ({e}); update the Odeion app"
                        )))
                    }
                }
            }
            key.session(addr).map(|_| ())
        })
        .await
    }

    async fn open(&self, fmt: AudioFormat) -> Result<Box<dyn AudioSink>, SinkError> {
        // What the endpoint takes now, or a format its device lists exactly
        let listed = self.device().is_some_and(|d| {
            d.formats
                .iter()
                .any(|f| (f.rate_hz, f.bits, f.channels) == (fmt.rate_hz, fmt.bits, fmt.channels))
        });
        if !listed && !self.live_caps().accepts(&fmt) {
            return Err(SinkError::UnsupportedFormat(self.id(), fmt));
        }
        let (addr, key, remote, id) = (self.addr, self.key.clone(), self.remote.id.0.clone(), self.id());
        blocking(move || open_session(addr, &key, remote, id, fmt))
            .await
            .map(|s| Box::new(s) as Box<dyn AudioSink>)
    }

    async fn set_volume(&self, v: Volume) -> Result<(), SinkError> {
        // What the endpoint takes now (a DAC plugged in since it registered)
        if self.live_caps().hw_volume.is_none() {
            return Err(SinkError::NoVolume(self.id()));
        }
        let (addr, key, remote) = (self.addr, self.key.clone(), self.remote.id.0.clone());
        blocking(move || {
            let (mut c, _) = key.session(addr)?;
            write_frame(
                &mut c.w,
                &Frame::SetVolume {
                    output: remote,
                    volume: v,
                },
            )
            .map_err(io_err)?;
            match answer_within(&mut c.r, HANDSHAKE_TIMEOUT)? {
                Frame::Done => Ok(()),
                other => Err(SinkError::Device(format!("unexpected {other:?}"))),
            }
        })
        .await
    }

    /// The level the endpoint's output reports now (a phone's media volume, its USB DAC's own),
    /// so a change made on the phone (its volume keys) shows on every controller. An app from
    /// before `GetVolume` ends the session instead of answering: the level is unknown then.
    async fn volume(&self) -> Result<Option<pm_model::ZoneVolume>, SinkError> {
        if self.live_caps().hw_volume.is_none() {
            return Ok(None);
        }
        let (addr, key, remote) = (self.addr, self.key.clone(), self.remote.id.0.clone());
        blocking(move || {
            let (mut c, _) = key.session(addr)?;
            write_frame(&mut c.w, &Frame::GetVolume { output: remote }).map_err(io_err)?;
            // Shorter than the node's poll gives a read: reads never pile up on a slow phone
            match answer_within(&mut c.r, VOLUME_READ_TIMEOUT) {
                Ok(Frame::Level { volume }) => Ok(volume),
                Ok(other) => Err(SinkError::Device(format!("unexpected {other:?}"))),
                // An older app: it can't say
                Err(_) => Ok(None),
            }
        })
        .await
    }
}

/// Run blocking socket work off the async workers when there is a runtime (an API handler); in
/// place otherwise (a zone's own thread).
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, SinkError> + Send + 'static,
) -> Result<T, SinkError> {
    match tokio::runtime::Handle::try_current() {
        Ok(h) => h
            .spawn_blocking(f)
            .await
            .unwrap_or_else(|e| Err(SinkError::Device(format!("the endpoint's task failed: {e}")))),
        Err(_) => f(),
    }
}

/// Open `remote` on the endpoint in `fmt`: the session, `Open`, and the thread that reads the
/// endpoint's answers. Blocking.
fn open_session(
    addr: SocketAddr,
    key: &KeyCell,
    remote: String,
    id: OutputId,
    fmt: AudioFormat,
) -> Result<NetSink, SinkError> {
    // Each session starts with the endpoint's outputs as they are now (a DAC plugged in since)
    let (mut c, _welcome) = key.session(addr)?;
    let remote_id = remote.clone();
    let remote_for_sink = remote.clone();
    write_frame(
        &mut c.w,
        &Frame::Open {
            output: remote,
            format: fmt,
        },
    )
    .map_err(io_err)?;
    let capacity = match answer_within(&mut c.r, OPEN_TIMEOUT)? {
        Frame::Opened { buffer_frames } => buffer_frames,
        other => return Err(SinkError::Device(format!("unexpected {other:?} after Open"))),
    };
    // An output with a device (a phone's USB DAC) decided at this open whether it plays
    // bit-perfect, and the endpoint says so in its next Welcome: read it now, apart from the audio
    if matches!(key.device(&OutputId(remote_id)), Some(Some(_))) {
        let k = key.clone();
        let _ = std::thread::Builder::new()
            .name("pm-net-device".into())
            .spawn(move || {
                if let Err(e) = k.session(addr) {
                    tracing::debug!("the endpoint's device after an open: {e}");
                }
            });
    }
    // The endpoint's answers, read on their own thread so a write need not wait for them
    let Conn { mut r, w } = c;
    let stream = w.get_ref().try_clone()?;
    let shared = Arc::new(Answers {
        state: Mutex::new(AckState {
            level: BufferLevel {
                queued_frames: 0,
                capacity_frames: capacity,
            },
            at: None,
            written: 0,
            acks: 0,
            reopen: false,
            replies: std::collections::VecDeque::new(),
            closed: None,
        }),
        changed: Condvar::new(),
    });
    let reader = shared.clone();
    std::thread::Builder::new()
        .name("pm-net-acks".into())
        .spawn(move || loop {
            // Read to the end even after the sink went: closing with answers unread would
            // reset the connection and lose the audio the endpoint has not read yet
            let f = expect_control(&mut r);
            let mut st = reader.state.lock().expect("answers");
            match f {
                Ok(Frame::Ack {
                    level,
                    frames_written,
                    reopen,
                }) => {
                    // Stamped as it arrives: the level counts down from then
                    st.level = level;
                    st.reopen = reopen;
                    st.at = Some(Instant::now());
                    st.written = frames_written;
                    st.acks += 1;
                }
                Ok(other) => st.replies.push_back(other),
                Err(e) => {
                    st.closed = Some(e.to_string());
                    reader.changed.notify_all();
                    break;
                }
            }
            reader.changed.notify_all();
        })
        .map_err(|e| SinkError::Device(format!("cannot read the endpoint's answers: {e}")))?;
    let remote = OutputId(remote_for_sink);
    let seen = endpoint_choice(key, &remote);
    Ok(NetSink {
        id,
        w,
        stream,
        answers: shared,
        parts: 0,
        sent: std::collections::VecDeque::new(),
        fmt,
        key: key.clone(),
        remote,
        seen,
    })
}

/// What the endpoint has answered, kept by the sink's answer thread.
struct AckState {
    /// The endpoint's level at its last `Ack`, and when that came.
    level: BufferLevel,
    at: Option<Instant>,
    /// What the endpoint's sink has written, at its last `Ack`.
    written: u64,
    /// `Ack`s so far: one per audio part.
    acks: u64,
    /// The endpoint's sink asked, at its last `Ack`, to be opened afresh at the next song.
    reopen: bool,
    /// Replies to control frames, in order.
    replies: std::collections::VecDeque<Frame>,
    /// The connection ended (why).
    closed: Option<String>,
}

struct Answers {
    state: Mutex<AckState>,
    changed: Condvar,
}

struct NetSink {
    id: OutputId,
    w: BufWriter<TcpStream>,
    /// The session's socket, to end it when the sink goes.
    stream: TcpStream,
    answers: Arc<Answers>,
    /// Audio parts sent so far.
    parts: u64,
    /// Writes not fully acknowledged, oldest first.
    sent: std::collections::VecDeque<Sent>,
    fmt: AudioFormat,
    /// The endpoint's output, and its device settings and caps when this opened: when they change
    /// (a phone's DAC switched between bit-perfect and shared), the next song opens afresh.
    key: KeyCell,
    remote: OutputId,
    seen: (Option<pm_zones::DeviceSettings>, Option<OutputCaps>),
}

/// An endpoint output's device settings and caps, as last reported.
fn endpoint_choice(
    key: &KeyCell,
    remote: &OutputId,
) -> (Option<pm_zones::DeviceSettings>, Option<OutputCaps>) {
    (
        key.device(remote).flatten().and_then(|d| d.settings),
        key.caps(remote),
    )
}

/// One write on its way: its audio parts end at `end` (counting from the session's first), and
/// it has `frames`, `per_part` in each of its parts but the last.
struct Sent {
    end: u64,
    parts: u64,
    frames: u64,
    per_part: u64,
}

impl Sent {
    /// Its frames the endpoint has not acknowledged after `acks` answers.
    fn unacked(&self, acks: u64) -> u64 {
        let done = acks.saturating_sub(self.end - self.parts).min(self.parts);
        if done == self.parts {
            0
        } else {
            self.frames - done * self.per_part
        }
    }
}

impl NetSink {
    /// Audio sent that the endpoint has not acknowledged yet: it is written and held all the
    /// same, a network away. Leaving it out would put the position ahead by that much (a zone
    /// counts from `frames_written` when a track starts, a group from the leader's level).
    fn unacked(&self, st: &AckState) -> u64 {
        self.sent.iter().map(|s| s.unacked(st.acks)).sum()
    }

    fn window(&self) -> u64 {
        let per_s = match self.fmt.kind {
            SampleKind::Pcm => self.fmt.rate_hz as u64,
            SampleKind::Dsd => self.fmt.rate_hz as u64 / 8,
        };
        per_s * self.key.in_flight().as_millis() as u64 / 1000
    }

    /// A write that failed because the endpoint closed: what it said first (its device went away,
    /// say), once its answers are read; else the write's own error.
    fn after_failed_write(&self, e: SinkError) -> SinkError {
        let mut st = self.answers.state.lock().expect("answers");
        let deadline = Instant::now() + Duration::from_millis(500);
        while st.closed.is_none() {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            st = self
                .answers
                .changed
                .wait_timeout(st, deadline - now)
                .expect("answers")
                .0;
        }
        let said = st
            .closed
            .as_ref()
            .map(|why| SinkError::Device(format!("{}: {why}", self.id)));
        match said.and_then(|s| s.gone_reason()) {
            Some(r) => SinkError::Gone(self.id.clone(), r),
            None => e,
        }
    }

    /// Wait on the answers until `done` says so; an ended connection or [`ANSWER_TIMEOUT`]
    /// without a word is an error.
    fn wait<T>(&self, mut done: impl FnMut(&mut AckState) -> Option<T>) -> Result<T, SinkError> {
        let mut st = self.answers.state.lock().expect("answers");
        let deadline = Instant::now() + ANSWER_TIMEOUT;
        loop {
            if let Some(v) = done(&mut st) {
                return Ok(v);
            }
            if let Some(why) = &st.closed {
                let e = SinkError::Device(format!("{}: {why}", self.id));
                // The endpoint's device went away (a USB DAC unplugged): the zone pauses
                return Err(match e.gone_reason() {
                    Some(r) => SinkError::Gone(self.id.clone(), r),
                    None => e,
                });
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(SinkError::Device(format!(
                    "{}: the endpoint stopped answering",
                    self.id
                )));
            }
            st = self
                .answers
                .changed
                .wait_timeout(st, deadline - now)
                .expect("answers")
                .0;
        }
    }

    /// Send a control frame and wait for its reply (the `Ack`s of audio sent before it come
    /// first).
    fn control(&mut self, send: Frame, expect: fn(&Frame) -> bool) -> Result<(), SinkError> {
        write_frame(&mut self.w, &send).map_err(io_err)?;
        let reply = self.wait(|st| st.replies.pop_front())?;
        if expect(&reply) {
            Ok(())
        } else {
            Err(SinkError::Device(format!("{}: unexpected {reply:?}", self.id)))
        }
    }

    /// After a flush or drain the endpoint holds nothing.
    fn emptied(&mut self) {
        let mut st = self.answers.state.lock().expect("answers");
        st.level.queued_frames = 0;
        st.at = None;
        self.sent.clear();
    }
}

impl AudioSink for NetSink {
    fn format(&self) -> AudioFormat {
        self.fmt
    }

    fn write(&mut self, block: &AudioBlock) -> Result<(), SinkError> {
        check_block(self.fmt, block)?;
        // A block larger than a frame goes out in parts, each acknowledged
        let parts = match write_audio(&mut self.w, block) {
            Ok(n) => n as u64,
            Err(e) => return Err(self.after_failed_write(io_err(e))),
        };
        self.parts += parts;
        self.sent.push_back(Sent {
            end: self.parts,
            parts,
            frames: block.frames() as u64,
            per_part: pm_stream::frames_per_part(self.fmt),
        });
        // Wait only while too much is unacknowledged (one write is always on its way)
        let window = self.window();
        let mut sent = std::mem::take(&mut self.sent);
        let r = self.wait(|st| {
            while sent.front().is_some_and(|s| s.end <= st.acks) {
                sent.pop_front();
            }
            let in_flight: u64 = sent.iter().map(|s| s.unacked(st.acks)).sum();
            (sent.len() <= 1 || in_flight <= window).then_some(())
        });
        self.sent = sent;
        r
    }

    /// What the endpoint holds now: its level at the last `Ack`, less what its device has
    /// played since, plus what is on its way. A group compares members at one instant, and a
    /// member whose `Ack` came later (a slower network) would otherwise seem ahead by the
    /// difference.
    fn buffer_level(&self) -> BufferLevel {
        let st = self.answers.state.lock().expect("answers");
        let played = st.at.map_or(0, |at| {
            let per_s = match self.fmt.kind {
                SampleKind::Pcm => self.fmt.rate_hz as u128,
                SampleKind::Dsd => self.fmt.rate_hz as u128 / 8,
            };
            (at.elapsed().as_micros() * per_s / 1_000_000) as u64
        });
        BufferLevel {
            queued_frames: st.level.queued_frames.saturating_sub(played) + self.unacked(&st),
            ..st.level
        }
    }

    fn flush(&mut self) -> Result<(), SinkError> {
        self.control(Frame::Flush, |f| matches!(f, Frame::Flushed))?;
        self.emptied();
        Ok(())
    }

    fn drain(&mut self) -> Result<(), SinkError> {
        self.control(Frame::Drain, |f| matches!(f, Frame::Drained))?;
        self.emptied();
        Ok(())
    }

    fn frames_written(&self) -> u64 {
        let st = self.answers.state.lock().expect("answers");
        st.written + self.unacked(&st)
    }

    /// The endpoint's device settings or caps changed since this opened (a phone's DAC switched
    /// between bit-perfect and shared, or was plugged in or out): the next song opens afresh.
    fn reopen_at_track(&self) -> bool {
        endpoint_choice(&self.key, &self.remote) != self.seen
            || self.answers.state.lock().expect("answers").reopen
    }
}

impl Drop for NetSink {
    fn drop(&mut self) {
        let _ = write_frame(&mut self.w, &Frame::Close);
        // The endpoint plays what it has, then closes; the answer thread reads to the end
        let _ = self.stream.shutdown(std::net::Shutdown::Write);
    }
}

#[cfg(test)]
mod tests {
    use super::Sent;

    /// A block sent in three parts (parts 5 to 7) counts down part by part as they are answered.
    #[test]
    fn a_write_in_parts_is_acknowledged_part_by_part() {
        let s = Sent {
            end: 7,
            parts: 3,
            frames: 250,
            per_part: 100,
        };
        assert_eq!(
            [3, 4, 5, 6, 7, 8].map(|acks| s.unacked(acks)),
            [250, 250, 150, 50, 0, 0]
        );
    }
}
