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
    async fn open(&self, fmt: AudioFormat) -> Result<Box<dyn AudioSink>, SinkError> {
        self.inner.open(fmt).await
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
    let phone = Arc::new(Phone {
        inner: NullOutput::new(),
        dac: Mutex::new(None),
    });
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
