//! `/v1/events?zone_queue=changed`: zone messages carry the queue only when it changed since
//! this socket last sent it; without the parameter every zone message carries it, as before.

use futures_util::StreamExt;
use pm_api::types::ApiEvent;
use pm_api::{NodeSettings, NodeState};
use pm_audio::PipelineConfig;
use pm_library::Library;
use pm_zones::outputs::NullOutput;
use pm_zones::{OutputRegistry, QueueItem, ZoneId, ZoneState};
use std::sync::Arc;

const OWNER: &str = "owner-token";

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The next text message.
async fn next(ws: &mut Socket) -> serde_json::Value {
    loop {
        let m = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .expect("a message")
            .unwrap()
            .unwrap();
        if let tokio_tungstenite::tungstenite::Message::Text(t) = m {
            return serde_json::from_str(&t).unwrap();
        }
    }
}

/// The next zone message this test sent (named `name`); the engine's own states are skipped.
async fn zone_named(ws: &mut Socket, name: &str) -> serde_json::Value {
    loop {
        let ev = next(ws).await;
        if ev["type"] == "zone" && ev["state"]["name"] == name {
            return ev["state"].clone();
        }
    }
}

fn items(n: usize) -> Vec<QueueItem> {
    (0..n)
        .map(|i| QueueItem {
            id: i as u64 + 1,
            uri: format!("https://radio.example/{i}"),
            title: Some(format!("Song {i}")),
            artist: None,
            album: None,
            duration_ms: None,
            image: None,
            limit: None,
            by: None,
        })
        .collect()
}

#[tokio::test]
async fn the_queue_goes_out_only_when_it_changed_for_sockets_that_ask() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("lib.sqlite");
    let s = NodeState::new(
        NodeSettings {
            name: "t".into(),
            version: "0".into(),
            db_path: db.clone(),
            scan_folders: vec![],
            art_dir: dir.path().join("art"),
            mirror_dir: dir.path().join("homeserver"),
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
    );
    let zone = s.create_zone("Kitchen", Arc::new(NullOutput::new()));
    let base: ZoneState = zone.state();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(pm_api::serve_listener(s.clone(), l));
    let connect = move |q: &'static str| async move {
        let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/v1/events?token={OWNER}{q}"))
            .await
            .unwrap();
        ws
    };
    let mut lean = connect("&zone_queue=changed").await;
    let mut plain = connect("").await;
    for ws in [&mut lean, &mut plain] {
        let snap = next(ws).await;
        assert_eq!(snap["type"], "snapshot");
        assert!(
            snap["zones"][0]["queue"].is_array(),
            "a snapshot carries the queue: {snap}"
        );
    }
    let send = |name: &str, version: u64, n: usize| {
        let mut st = base.clone();
        st.name = name.into();
        st.queue = items(n);
        st.queue_version = base.queue_version + version;
        s.events.send(ApiEvent::Zone { state: st });
    };

    // A new queue goes out whole
    send("a", 1, 3);
    assert_eq!(
        zone_named(&mut lean, "a").await["queue"].as_array().map(Vec::len),
        Some(3)
    );
    // The same queue (the position moved): left out, its version still there
    send("b", 1, 3);
    let b = zone_named(&mut lean, "b").await;
    assert!(b.get("queue").is_none(), "{b}");
    assert_eq!(b["queue_version"], base.queue_version + 1);
    // A changed queue goes out whole again
    send("c", 2, 4);
    assert_eq!(
        zone_named(&mut lean, "c").await["queue"].as_array().map(Vec::len),
        Some(4)
    );
    // A zone removed and made again starts over: its first state carries the queue
    s.events.send(ApiEvent::ZoneRemoved {
        zone: ZoneId(base.id.0.clone()),
    });
    send("d", 2, 4);
    assert_eq!(
        zone_named(&mut lean, "d").await["queue"].as_array().map(Vec::len),
        Some(4)
    );
    // A resync's snapshot carries every queue, and what follows is measured against it
    use futures_util::SinkExt;
    lean.send(tokio_tungstenite::tungstenite::Message::Text(
        r#"{"type":"resync"}"#.into(),
    ))
    .await
    .unwrap();
    loop {
        if next(&mut lean).await["type"] == "snapshot" {
            break;
        }
    }
    send("e", 2, 4);
    assert_eq!(
        zone_named(&mut lean, "e").await["queue"].as_array().map(Vec::len),
        Some(4)
    );

    // Without the parameter every zone message carries the queue, as before
    for (name, n) in [("a", 3), ("b", 3), ("c", 4), ("d", 4), ("e", 4)] {
        let st = zone_named(&mut plain, name).await;
        assert_eq!(st["queue"].as_array().map(Vec::len), Some(n), "{name}: {st}");
    }
}
