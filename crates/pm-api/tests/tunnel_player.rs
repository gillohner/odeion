//! The app's player away from home: the app reaches the node through the iroh tunnel and serves
//! its player over that same connection; it registers as `tunnel:<its key>`, and the node reaches
//! it through a loopback relay into that connection. The phone is then a zone like at home: its
//! output is named by the app's key and takes audio through the tunnel.

use pm_api::remote::RemoteConfig;
use pm_api::{NodeSettings, NodeState};
use pm_audio::PipelineConfig;
use pm_library::Library;
use pm_model::{AudioBlock, AudioFormat};
use pm_remote::{ClientTunnel, Relays, SecretKey};
use pm_zones::outputs::NullOutput;
use pm_zones::{OutputId, OutputRegistry};
use std::sync::Arc;

const OWNER: &str = "owner-token";

fn node(dir: &std::path::Path) -> Arc<NodeState> {
    let db = dir.join("lib.sqlite");
    NodeState::new(
        NodeSettings {
            name: "t".into(),
            version: "0".into(),
            db_path: db.clone(),
            scan_folders: vec![],
            art_dir: dir.join("art"),
            mirror_dir: dir.join("homeserver"),
            pipeline: PipelineConfig::default(),
            token: Some(OWNER.into()),
            subsonic_user: "pubky".into(),
            pubky: Default::default(),
            public_url: "http://127.0.0.1:7788".into(),
            upnp: false,
            cast: false,
            slimproto_port: 0,
            session_secret_path: None,
            tidal: Default::default(),
            spotify: Default::default(),
            acoustid_key: String::new(),
            jamendo: Default::default(),
        },
        Library::open(&db).unwrap(),
        OutputRegistry::new(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_apps_player_away_from_home_is_a_zone_through_the_tunnel() {
    let dir = tempfile::tempdir().unwrap();
    let s = node(dir.path());
    let api = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_addr = api.local_addr().unwrap();
    tokio::spawn(pm_api::serve_listener(s.clone(), api));
    s.configure_remote(RemoteConfig {
        key_path: dir.path().join("iroh.key"),
        relays: Relays::Off,
        target: api_addr,
    })
    .await;
    assert!(s.ensure_remote().await.is_some(), "the tunnel runs");
    let at = s.remote_addr().await.unwrap();

    // The app: its tunnel (with a key it keeps) and its player on loopback, served over the tunnel
    let app = ClientTunnel::start_with_key(at, &Relays::Off, Some(SecretKey::generate()))
        .await
        .unwrap();
    app.connect().await.unwrap();
    let key = pm_stream::Key::random();
    let outputs = OutputRegistry::new();
    outputs.register(Arc::new(NullOutput::new()));
    let endpoint = pm_endpoint::Endpoint::new("Pixel 8", outputs, key.clone());
    endpoint.expect_peer(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    let serving = endpoint
        .spawn(std::net::TcpListener::bind("127.0.0.1:0").unwrap())
        .unwrap();
    app.serve_player(serving.addr()).await;

    // Registered through the tunnel's HTTP side, as the app does away from home
    let name = format!("tunnel:{}", app.id());
    let r = reqwest::Client::new()
        .post(format!("http://{}/v1/outputs/endpoints", app.local_addr()))
        .bearer_auth(OWNER)
        .json(&serde_json::json!({ "address": name, "key": key.expose() }))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.text().await.unwrap_or_default());
    let outs: Vec<serde_json::Value> = r.json().await.unwrap();
    let id = format!("net:{name}/null");
    assert_eq!(outs[0]["id"], id.as_str(), "named by the app's key: {outs:?}");

    // Audio reaches the player through the tunnel
    let out = s
        .outputs
        .get(&OutputId(id.clone()))
        .expect("the phone's output on the node");
    let fmt = AudioFormat::pcm(44_100, 16, 2);
    let mut sink = out.open(fmt).await.expect("opened through the tunnel");
    let block = AudioBlock::pcm_i32(fmt, vec![0; 2 * 4_410]);
    tokio::task::spawn_blocking(move || {
        for _ in 0..5 {
            sink.write(&block).unwrap();
        }
        assert!(sink.frames_written() >= 5 * 4_410);
    })
    .await
    .unwrap();

    // A re-registration that fails (here a wrong player key; on a phone, a link that drops
    // during Hello) leaves the working zone as it was: its relay stays
    let r = reqwest::Client::new()
        .post(format!("http://{}/v1/outputs/endpoints", app.local_addr()))
        .bearer_auth(OWNER)
        .json(&serde_json::json!({ "address": name, "key": pm_stream::Key::random().expose() }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST);
    let mut sink = out.open(fmt).await.expect("still reached through the tunnel");
    let block = AudioBlock::pcm_i32(fmt, vec![0; 2 * 4_410]);
    tokio::task::spawn_blocking(move || sink.write(&block).unwrap())
        .await
        .unwrap();

    // Another app's key that never connected: refused, nothing registered
    let other = format!("tunnel:{}", SecretKey::generate().public());
    let r = reqwest::Client::new()
        .post(format!("http://{}/v1/outputs/endpoints", app.local_addr()))
        .bearer_auth(OWNER)
        .json(&serde_json::json!({ "address": other, "key": key.expose() }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST);
}
