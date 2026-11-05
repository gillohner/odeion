//! A phone's USB DAC reaches the node: the app's player reports it as its output's `device`, and
//! the node's outputs for that endpoint learn it when the app registers again (a new `Welcome`)
//! and when a zone opens the output, also the output a zone already holds. Nothing is renamed.

use pm_endpoint::Endpoint;
use pm_model::{AudioFormat, OutputCaps};
use pm_out_net::{KeyCell, NetOutput};
use pm_stream::Key;
use pm_zones::outputs::NullOutput;
use pm_zones::{AudioSink, DeviceMode, Output, OutputDevice, OutputId, OutputRegistry, SinkError};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// The phone's output as the endpoint offers it: plays like the null output, and reports the DAC
/// the app last saw.
struct Phone {
    inner: NullOutput,
    dac: Mutex<Option<OutputDevice>>,
    /// What it takes now, when not its caps (bit-perfect to the DAC)
    live: Mutex<Option<OutputCaps>>,
    /// Its sinks find the device gone (the DAC unplugged)
    gone: Arc<std::sync::atomic::AtomicBool>,
}

impl Phone {
    fn new() -> Self {
        Self {
            inner: NullOutput::new(),
            dac: Mutex::new(None),
            live: Mutex::new(None),
            gone: Arc::default(),
        }
    }
}

/// The phone's sink: the null output's, until the DAC goes.
struct PhoneSink(Box<dyn AudioSink>, Arc<std::sync::atomic::AtomicBool>);

impl AudioSink for PhoneSink {
    fn format(&self) -> AudioFormat {
        self.0.format()
    }
    fn write(&mut self, block: &pm_model::AudioBlock) -> Result<(), SinkError> {
        if self.1.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(SinkError::Gone(
                OutputId::new("cpal", "Speaker or headphones"),
                "FiiO KA17 was unplugged".into(),
            ));
        }
        self.0.write(block)
    }
    fn buffer_level(&self) -> pm_zones::BufferLevel {
        self.0.buffer_level()
    }
    fn flush(&mut self) -> Result<(), SinkError> {
        self.0.flush()
    }
    fn drain(&mut self) -> Result<(), SinkError> {
        self.0.drain()
    }
    fn frames_written(&self) -> u64 {
        self.0.frames_written()
    }
}

#[async_trait::async_trait]
impl Output for Phone {
    fn id(&self) -> OutputId {
        OutputId::new("cpal", "Speaker or headphones")
    }
    fn name(&self) -> String {
        "Speaker or headphones (Android audio, shared mode)".into()
    }
    fn device(&self) -> Option<OutputDevice> {
        self.dac.lock().unwrap().clone()
    }
    fn caps(&self) -> &OutputCaps {
        self.inner.caps()
    }
    fn live_caps(&self) -> OutputCaps {
        self.live
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| self.caps().clone())
    }
    async fn configure_device(&self, settings: pm_zones::DeviceSettings) -> Result<(), SinkError> {
        if let Some(d) = self.dac.lock().unwrap().as_mut() {
            d.settings = Some(settings);
        }
        Ok(())
    }
    async fn open(&self, fmt: AudioFormat) -> Result<Box<dyn AudioSink>, SinkError> {
        let s = self.inner.open(fmt).await?;
        Ok(Box::new(PhoneSink(s, self.gone.clone())))
    }
}

fn ka17() -> OutputDevice {
    OutputDevice {
        name: "FiiO KA17".into(),
        summary: "FiiO KA17 · PCM up to 768 kHz / 32 bit".into(),
        rates: vec![44_100, 48_000, 96_000, 192_000, 768_000],
        bits: vec![16, 24, 32],
        max_rate: 768_000,
        max_bits: 32,
        channels: 2,
        mode: DeviceMode::Shared,
        modes: vec![DeviceMode::Shared],
        ..Default::default()
    }
}

/// The phone's player on loopback, and the node's key cell for it.
fn phone() -> (Arc<Phone>, pm_endpoint::Serving, KeyCell) {
    let phone = Arc::new(Phone::new());
    let registry = OutputRegistry::new();
    registry.register(phone.clone());
    let key = Key::random();
    let serving = Endpoint::new("Pixel 8", registry, key.clone())
        .spawn(TcpListener::bind("127.0.0.1:0").unwrap())
        .unwrap();
    (phone, serving, KeyCell::new(key))
}

#[test]
fn registering_again_tells_the_output_a_zone_already_holds() {
    let (phone, serving, cell) = phone();
    // The node's output for the phone, as a zone holds it from the first registration
    let held: Arc<dyn Output> = Arc::new(NetOutput::connect(serving.addr(), &cell).unwrap().remove(0));
    let name = held.name();
    assert_eq!(held.device(), None);
    assert!(held.info().device.is_none());

    // The KA17 goes in; the app registers again (the node connects with the same key cell)
    *phone.dac.lock().unwrap() = Some(ka17());
    let fresh = NetOutput::connect(serving.addr(), &cell).unwrap().remove(0);
    assert_eq!(fresh.device(), Some(ka17()));
    assert_eq!(
        held.device(),
        Some(ka17()),
        "the output the zone holds learns it too"
    );
    let info = serde_json::to_value(held.info()).unwrap();
    assert_eq!(info["device"]["name"], "FiiO KA17");
    assert_eq!(info["device"]["mode"], "shared");
    assert_eq!(info["device"]["confirmed"], false);
    assert_eq!(held.name(), name, "the name stays: the zone was named from it");

    // And out again
    *phone.dac.lock().unwrap() = None;
    NetOutput::connect(serving.addr(), &cell).unwrap();
    assert_eq!(held.device(), None);
    assert!(serde_json::to_value(held.info()).unwrap().get("device").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opening_the_output_also_tells_it() {
    let (phone, serving, cell) = phone();
    let held = NetOutput::connect(serving.addr(), &cell).unwrap().remove(0);
    // Plugged in, and the app could not register again (say the node was busy)
    *phone.dac.lock().unwrap() = Some(ka17());
    assert_eq!(held.device(), None);
    let fmt = AudioFormat::pcm(48_000, 24, 2);
    let sink = held.open(fmt).await.unwrap();
    drop(sink);
    assert_eq!(held.device(), Some(ka17()), "the session's Welcome carried it");
}

#[test]
fn another_nodes_key_cell_learns_nothing_from_this_one() {
    let (phone, serving, cell) = phone();
    *phone.dac.lock().unwrap() = Some(ka17());
    let a = NetOutput::connect(serving.addr(), &cell).unwrap().remove(0);
    assert_eq!(a.device(), Some(ka17()));
    // A cell made apart (another endpoint address) keeps its own view
    let other = KeyCell::new(Key::random());
    *phone.dac.lock().unwrap() = None;
    assert!(
        NetOutput::connect(serving.addr(), &other).is_err(),
        "a wrong key is refused"
    );
    assert_eq!(a.device(), Some(ka17()), "a refused connection changes nothing");
}

/// A hand-made endpoint (no pm-endpoint): each connection gets its own answer, so the tests can
/// send what a newer app sends, or answer out of order.
mod fake {
    use std::io::Write;
    use std::net::TcpStream;

    /// The phone output's info as a Welcome carries it, with `device` as raw JSON.
    pub fn output(device: serde_json::Value) -> serde_json::Value {
        let phone = super::Phone::new();
        let mut v = serde_json::to_value(pm_zones::Output::info(&phone)).unwrap();
        v["device"] = device;
        v
    }

    /// Read the node's Hello.
    pub fn hello(s: &mut TcpStream) {
        match pm_stream::read_frame(s).unwrap() {
            pm_stream::Incoming::Control(pm_stream::Frame::Hello { .. }) => {}
            other => panic!("not a Hello: {other:?}"),
        }
    }

    /// A control frame from raw JSON, as the wire carries it.
    pub fn send(s: &mut TcpStream, frame: serde_json::Value) {
        let body = serde_json::to_vec(&frame).unwrap();
        let mut b = vec![1u8];
        b.extend_from_slice(&(body.len() as u32).to_be_bytes());
        b.extend_from_slice(&body);
        s.write_all(&b).unwrap();
    }

    pub fn welcome(s: &mut TcpStream, output: serde_json::Value) {
        send(
            s,
            serde_json::json!({"Welcome": {"name": "Pixel 8", "version": pm_stream::PROTOCOL_VERSION, "outputs": [output]}}),
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_from_a_newer_app_keeps_the_phone_zone_working() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let newer = fake::output(
        serde_json::json!({"name": "FiiO KA17", "kind": "usb_c_hub", "mode": "hyper_perfect",
        "modes": ["hyper_perfect"], "confirmed": true, "dsd": "dsd_over_usb4", "rates": [44100]}),
    );
    let server = std::thread::spawn(move || {
        // The registration, then a zone opening the output
        let (mut s, _) = l.accept().unwrap();
        fake::hello(&mut s);
        fake::welcome(&mut s, newer.clone());
        let (mut s, _) = l.accept().unwrap();
        fake::hello(&mut s);
        fake::welcome(&mut s, newer);
        match pm_stream::read_frame(&mut s).unwrap() {
            pm_stream::Incoming::Control(pm_stream::Frame::Open { .. }) => {}
            other => panic!("not an Open: {other:?}"),
        }
        fake::send(&mut s, serde_json::json!({"Opened": {"buffer_frames": 4800}}));
        s
    });
    let out = NetOutput::connect(addr, &KeyCell::new(Key::random()))
        .expect("an unknown mode, kind or DSD way must not refuse the endpoint")
        .remove(0);
    let d = out.device().unwrap();
    assert_eq!(
        (d.name.as_str(), d.mode, d.kind),
        ("FiiO KA17", DeviceMode::Unknown, pm_zones::DeviceKind::Unknown)
    );
    let sink = out
        .open(AudioFormat::pcm(48_000, 24, 2))
        .await
        .expect("the output opens");
    assert_eq!(sink.buffer_level().capacity_frames, 4800);
    drop(server.join().unwrap());
    drop(sink);
}

#[test]
fn an_older_welcome_never_overwrites_a_newer_one() {
    // Connection A says Hello while the KA17 is in; then it is unplugged and the app registers
    // again (B). The endpoint answers B first, A a moment later, as a slow network may.
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let (a_said_hello, a_hello) = std::sync::mpsc::channel();
    let attached = fake::output(serde_json::to_value(ka17()).unwrap());
    let unplugged = fake::output(serde_json::Value::Null);
    let server = std::thread::spawn(move || {
        let (mut a, _) = l.accept().unwrap();
        fake::hello(&mut a);
        a_said_hello.send(()).unwrap();
        // B, if it can get in while A waits for its answer
        l.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(800);
        let mut b = None;
        while b.is_none() && std::time::Instant::now() < deadline {
            match l.accept() {
                Ok((s, _)) => b = Some(s),
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        }
        if let Some(mut b) = b {
            b.set_nonblocking(false).unwrap();
            fake::hello(&mut b);
            fake::welcome(&mut b, unplugged.clone());
            std::thread::sleep(std::time::Duration::from_millis(100));
            fake::welcome(&mut a, attached);
            return;
        }
        // B waited for A: A's answer, then B's
        fake::welcome(&mut a, attached);
        l.set_nonblocking(false).unwrap();
        let (mut b, _) = l.accept().unwrap();
        fake::hello(&mut b);
        fake::welcome(&mut b, unplugged);
    });
    let cell = KeyCell::new(Key::random());
    let c2 = cell.clone();
    let first = std::thread::spawn(move || NetOutput::connect(addr, &c2).unwrap().remove(0));
    a_hello.recv().unwrap();
    let second = NetOutput::connect(addr, &cell).unwrap().remove(0);
    let first = first.join().unwrap();
    server.join().unwrap();
    assert_eq!(second.device(), None, "the KA17 was unplugged last");
    assert_eq!(first.device(), None, "for every output of the endpoint");
}

#[test]
fn the_node_hears_when_a_device_comes_or_goes() {
    let (phone, serving, cell) = phone();
    let heard = Arc::new(Mutex::new(Vec::<OutputId>::new()));
    let h = heard.clone();
    cell.on_device_change(move |id| h.lock().unwrap().push(id.clone()));
    // A new output without a device: nothing to tell
    NetOutput::connect(serving.addr(), &cell).unwrap();
    assert!(heard.lock().unwrap().is_empty());
    *phone.dac.lock().unwrap() = Some(ka17());
    NetOutput::connect(serving.addr(), &cell).unwrap();
    NetOutput::connect(serving.addr(), &cell).unwrap(); // the same again: no change
    *phone.dac.lock().unwrap() = None;
    NetOutput::connect(serving.addr(), &cell).unwrap();
    let id = OutputId(format!("net:{}/cpal:Speaker or headphones", serving.addr()));
    assert_eq!(*heard.lock().unwrap(), [id.clone(), id]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_silent_session_holds_up_nothing_else() {
    // The endpoint takes connection A and never answers it (a phone frozen mid-handshake); B
    // (the app registering again) and C (a zone opening the output) must not wait for A
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let (a_in, a_said_hello) = std::sync::mpsc::channel();
    let output = fake::output(serde_json::Value::Null);
    let server = std::thread::spawn(move || {
        let (mut a, _) = l.accept().unwrap();
        fake::hello(&mut a);
        a_in.send(()).unwrap();
        let (mut b, _) = l.accept().unwrap();
        fake::hello(&mut b);
        fake::welcome(&mut b, output.clone());
        let (mut c, _) = l.accept().unwrap();
        fake::hello(&mut c);
        fake::welcome(&mut c, output);
        match pm_stream::read_frame(&mut c).unwrap() {
            pm_stream::Incoming::Control(pm_stream::Frame::Open { .. }) => {}
            other => panic!("not an Open: {other:?}"),
        }
        fake::send(&mut c, serde_json::json!({"Opened": {"buffer_frames": 4800}}));
        (a, c) // A stays open and silent until the test ends
    });
    let cell = KeyCell::new(Key::random());
    let c2 = cell.clone();
    let started = std::time::Instant::now();
    let silent = tokio::task::spawn_blocking(move || NetOutput::connect(addr, &c2).map(|_| ()));
    a_said_hello.recv().unwrap();
    let c3 = cell.clone();
    let b = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::task::spawn_blocking(move || NetOutput::connect(addr, &c3)),
    )
    .await
    .expect("registering again waits for no other session")
    .unwrap()
    .expect("and works");
    let out = b.into_iter().next().unwrap();
    let sink = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        out.open(AudioFormat::pcm(48_000, 24, 2)),
    )
    .await
    .expect("opening waits for no other session")
    .expect("and works");
    drop(sink);
    // A itself gives up within the handshake bound
    let a = tokio::time::timeout(
        pm_out_net::HANDSHAKE_TIMEOUT + std::time::Duration::from_secs(3),
        silent,
    )
    .await
    .expect("a silent endpoint is given up on")
    .unwrap();
    assert!(a.is_err(), "no Welcome, no outputs");
    assert!(started.elapsed() < pm_out_net::HANDSHAKE_TIMEOUT + std::time::Duration::from_secs(3));
    drop(server.join().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn what_the_phone_takes_now_reaches_the_zone_and_settings_reach_the_phone() {
    let (phone, serving, cell) = phone();
    let held = NetOutput::connect(serving.addr(), &cell).unwrap().remove(0);
    let base = held.caps().clone();
    // Bit-perfect to the KA17: the phone takes the DAC's rates now
    let bit_perfect = OutputCaps {
        pcm_rates: vec![44_100, 48_000, 88_200, 96_000, 176_400, 192_000],
        pcm_bits: vec![16, 24, 32],
        max_pcm_bits: 32,
        quality_ceiling: pm_model::QualityCeiling::BitPerfect,
        ..base.clone()
    };
    *phone.live.lock().unwrap() = Some(bit_perfect.clone());
    *phone.dac.lock().unwrap() = Some(ka17());
    NetOutput::connect(serving.addr(), &cell).unwrap();
    assert_eq!(
        held.live_caps(),
        bit_perfect,
        "the output a zone holds plans with them"
    );
    assert_eq!(held.caps(), &base, "caps stay what they were when found");
    assert_eq!(held.info().caps, bit_perfect, "GET /v1/outputs shows them");
    for fmt in [AudioFormat::pcm(96_000, 24, 2), AudioFormat::pcm(192_000, 24, 2)] {
        assert!(held.live_caps().accepts(&fmt), "{fmt} goes to the DAC as it is");
        drop(held.open(fmt).await.expect("opens"));
    }
    // Settings from the node reach the phone, and the node sees them at once
    let s = pm_zones::DeviceSettings {
        mode: pm_zones::PreferredMode::Shared,
        dac_volume: false,
    };
    held.configure_device(s).await.unwrap();
    assert_eq!(held.device().unwrap().settings, Some(s));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dac_unplugged_on_the_phone_reaches_the_zone_as_gone() {
    let (phone, serving, cell) = phone();
    let out = NetOutput::connect(serving.addr(), &cell).unwrap().remove(0);
    let fmt = AudioFormat::pcm(48_000, 24, 2);
    let mut sink = out.open(fmt).await.unwrap();
    let block = pm_model::AudioBlock::pcm_i32(fmt, vec![0; 480 * 2]);
    sink.write(&block).unwrap();
    phone.gone.store(true, std::sync::atomic::Ordering::SeqCst);
    let mut err = None;
    for _ in 0..500 {
        if let Err(e) = sink.write(&block) {
            err = Some(e);
            break;
        }
    }
    let e = err.expect("the write fails once the endpoint's sink found the DAC gone");
    assert!(matches!(e, SinkError::Gone(..)), "{e}");
    assert_eq!(e.gone_reason().as_deref(), Some("FiiO KA17 was unplugged"));
}
