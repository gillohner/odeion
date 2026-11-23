//! Outputs on the network: renderers, endpoints, discovery and their limits.
//!
//! Part of pm-api's unit tests (`mod tests` in lib.rs, which holds the helpers these use). A new
//! test of this area goes at the end of this file.

use super::*;

/// A UPnP renderer's volume follows its events (TASK-volume-readback): the node subscribes,
/// a renew the renderer refuses (412) makes a fresh SUBSCRIBE, a NOTIFY to the callback
/// moves the zone's level and mute, and the poll steps aside only once events arrive. Late,
/// stale and foreign events are dropped, and a zone made again under the same name doesn't
/// get this renderer's level.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_renderers_volume_events_move_the_zone() {
    // The same value the read-back test uses (tests share the process)
    crate::state::VOLUME_POLL_MS_FOR_TESTS.store(200, std::sync::atomic::Ordering::Relaxed);
    let log = Arc::new(std::sync::Mutex::new(Vec::<(String, String)>::new()));
    let renewals = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let reads = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (l2, r2, rd) = (log.clone(), renewals.clone(), reads.clone());
    let mock = Router::new()
        .route(
            "/rc/event",
            axum::routing::any(move |method: axum::http::Method, h: axum::http::HeaderMap| {
                let (log, renewals) = (l2.clone(), r2.clone());
                async move {
                    let cb = h
                        .get("CALLBACK")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let sid = h
                        .get("SID")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    log.lock().unwrap().push((
                        method.to_string(),
                        if sid.is_empty() {
                            cb
                        } else {
                            format!("renew {sid}")
                        },
                    ));
                    if !sid.is_empty() {
                        // The first renewal is refused: the node must subscribe afresh
                        if renewals.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                            return (
                                StatusCode::PRECONDITION_FAILED,
                                [("SID", "uuid:gone"), ("TIMEOUT", "Second-1")],
                            );
                        }
                    }
                    (StatusCode::OK, [("SID", "uuid:sub-1"), ("TIMEOUT", "Second-1")])
                }
            }),
        )
        // The poll's GetVolume/GetMute, counted
        .route(
            "/rc/control",
            post(move || {
                let rd = rd.clone();
                async move {
                    rd.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let base = format!("http://127.0.0.1:{port}");
    let r = pm_out_upnp::Renderer {
        udn: "uuid:fake-linn".into(),
        friendly_name: "Fake Linn".into(),
        manufacturer: Some("Linn Products Ltd.".into()),
        model: Some("Klimax DSM".into()),
        av_transport: format!("{base}/av/control"),
        rendering_control: Some(format!("{base}/rc/control")),
        connection_manager: None,
        zone_group_topology: None,
        location: format!("{base}/desc.xml"),
        events: pm_out_upnp::RendererEvents {
            rendering_control: Some(format!("{base}/rc/event")),
            openhome_volume: None,
        },
    };
    let pull = s.pull_output("fake-linn");
    let out = Arc::new(pm_out_upnp::UpnpOutput::new(r, pull, s.pull_url("fake-linn")));
    let oid = pm_zones::Output::id(out.as_ref());
    s.renderers.lock().unwrap().insert(oid.clone(), out.clone());
    s.outputs.register(out.clone());
    let zone = s.create_zone("Fake Linn", out);
    let app = router(s.clone());
    // It subscribes, with a callback on the node
    let token = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some((_, cb)) = log
                .lock()
                .unwrap()
                .iter()
                .find(|(m, c)| m == "SUBSCRIBE" && c.starts_with('<'))
            {
                return cb
                    .trim_matches(['<', '>'])
                    .rsplit('/')
                    .next()
                    .unwrap()
                    .to_string();
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the node never subscribed");
    let reads_now = || reads.load(std::sync::atomic::Ordering::SeqCst);
    let ms = |n| tokio::time::sleep(std::time::Duration::from_millis(n));
    // A page is open: with no event arrived yet the poll keeps reading, subscription or not
    let watching = s.events.watch();
    let before = reads_now();
    ms(700).await;
    assert!(reads_now() > before, "no event yet, so the poll reads");
    // The renewal is refused (412): the node subscribes afresh
    let again = tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            let l = log.lock().unwrap().clone();
            let renewed = l.iter().position(|(_, c)| c.starts_with("renew"));
            if let Some(i) = renewed {
                if l[i + 1..]
                    .iter()
                    .any(|(m, c)| m == "SUBSCRIBE" && c.starts_with('<'))
                {
                    return l;
                }
            }
            drop(l);
            ms(100).await;
        }
    })
    .await;
    assert!(
        again.is_ok(),
        "no fresh SUBSCRIBE after a refused renewal: {:?}",
        log.lock().unwrap()
    );
    // A NOTIFY from the LAN moves the zone's volume and mute
    let notify_as = |token: &str, sid: &str, seq: u32, body: &str| {
        let mut req = Request::builder()
            .method("NOTIFY")
            .uri(format!("/v1/upnp/events/{token}"))
            .header("NT", "upnp:event")
            .header("SID", sid)
            .header("SEQ", seq.to_string())
            .body(Body::from(body.to_string()))
            .unwrap();
        req.extensions_mut().insert(axum::extract::ConnectInfo(
            "192.168.1.30:49152".parse::<std::net::SocketAddr>().unwrap(),
        ));
        req
    };
    let notify = |token: &str, seq: u32, body: &str| notify_as(token, "uuid:sub-1", seq, body);
    let event = |inner: &str| {
        format!(
            r#"<e:propertyset xmlns:e="urn:schemas-upnp-org:event-1-0"><e:property><LastChange>&lt;Event&gt;&lt;InstanceID val="0"&gt;{inner}&lt;/InstanceID&gt;&lt;/Event&gt;</LastChange></e:property></e:propertyset>"#
        )
    };
    let level = |v: u32| event(&format!(r#"&lt;Volume channel="Master" val="{v}"/&gt;"#));
    let body = event(r#"&lt;Volume channel="Master" val="37"/&gt;&lt;Mute channel="Master" val="1"/&gt;"#);
    let resp = app.clone().oneshot(notify(&token, 0, &body)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    zone.wait_acked().await;
    assert_eq!(
        zone.state().volume,
        Some(pm_model::ZoneVolume {
            level: pm_model::VolumeLevel::Percent(37),
            muted: true
        })
    );
    // Events arrive now: the poll leaves the zone alone (one read may have been under way)
    ms(300).await;
    let before = reads_now();
    ms(800).await;
    assert_eq!(reads_now(), before, "events work, so the poll doesn't read");
    // Only the mute changes next: the level stays
    let body = event(r#"&lt;Mute channel="Master" val="0"/&gt;"#);
    app.clone().oneshot(notify(&token, 1, &body)).await.unwrap();
    zone.wait_acked().await;
    assert_eq!(
        zone.state().volume.map(|v| (v.level, v.muted)),
        Some((pm_model::VolumeLevel::Percent(37), false))
    );
    // A quick turn whose events arrive out of order: the later one (SEQ) wins
    app.clone().oneshot(notify(&token, 3, &level(40))).await.unwrap();
    app.clone().oneshot(notify(&token, 2, &level(30))).await.unwrap();
    zone.wait_acked().await;
    assert_eq!(
        zone.state().volume.map(|v| v.level),
        Some(pm_model::VolumeLevel::Percent(40))
    );
    // Back and forth at once (40 → 41 → 40): ends at 40, the last word
    app.clone().oneshot(notify(&token, 4, &level(41))).await.unwrap();
    app.clone().oneshot(notify(&token, 5, &level(40))).await.unwrap();
    zone.wait_acked().await;
    assert_eq!(
        zone.state().volume.map(|v| v.level),
        Some(pm_model::VolumeLevel::Percent(40))
    );
    // SEQ 0 is a subscription's first event: the count starts again, the level is taken
    app.clone().oneshot(notify(&token, 0, &level(33))).await.unwrap();
    zone.wait_acked().await;
    assert_eq!(
        zone.state().volume.map(|v| v.level),
        Some(pm_model::VolumeLevel::Percent(33))
    );
    // An event with nothing the node reads (left channel only) changes nothing
    let lf = event(r#"&lt;Volume channel="LF" val="5"/&gt;"#);
    assert_eq!(
        app.clone()
            .oneshot(notify(&token, 1, &lf))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    zone.wait_acked().await;
    assert_eq!(
        zone.state().volume.map(|v| v.level),
        Some(pm_model::VolumeLevel::Percent(33))
    );
    app.clone().oneshot(notify(&token, 2, &level(40))).await.unwrap();
    zone.wait_acked().await;
    // An event of a subscription the node gave up: 412, nothing moves
    let resp = app
        .clone()
        .oneshot(notify_as(&token, "uuid:old", 6, &level(12)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PRECONDITION_FAILED);
    // A token the node doesn't hold: 412, as GENA says; a request from outside the LAN: refused
    let resp = app.clone().oneshot(notify("nope", 7, &body)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::PRECONDITION_FAILED);
    let mut outside = notify(&token, 7, &body);
    outside.extensions_mut().insert(axum::extract::ConnectInfo(
        "8.8.8.8:5000".parse::<std::net::SocketAddr>().unwrap(),
    ));
    assert_eq!(
        app.clone().oneshot(outside).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        zone.state().volume.map(|v| v.level),
        Some(pm_model::VolumeLevel::Percent(40))
    );
    // The zone goes and another output gets a zone of the same name: the renderer's events
    // don't reach it, and the node unsubscribes
    let zid = zone.id().clone();
    s.remove_zone(&zid);
    let other: Arc<dyn pm_zones::Output> = Arc::new(NullOutput::with_volume(pm_model::VolumeKind::Percent));
    s.outputs.register(other.clone());
    let twin = s.create_zone("Fake Linn", other);
    assert_eq!(twin.id(), &zid, "the same name gives the same id");
    twin.wait_acked().await;
    // The new zone reads its own output's level once it starts: wait for that, so the
    // comparison below sees only what the renderer's event could change
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while twin.state().volume.is_none() {
            ms(20).await;
        }
    })
    .await
    .expect("the new zone never read its output's level");
    let twin_was = twin.state().volume;
    let resp = app.clone().oneshot(notify(&token, 8, &level(9))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::PRECONDITION_FAILED);
    twin.wait_acked().await;
    assert_eq!(
        twin.state().volume,
        twin_was,
        "the renderer's level went to another zone"
    );
    let gone = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !log.lock().unwrap().iter().any(|(m, _)| m == "UNSUBSCRIBE") {
            ms(100).await;
        }
    })
    .await;
    assert!(gone.is_ok(), "no UNSUBSCRIBE: {:?}", log.lock().unwrap());
    drop(watching);
}

/// A renderer on another subnet is added by its description URL, becomes
/// An endpoint added by address (the desktop app's player, a Pi on
/// another subnet) gets a zone per output, is remembered for the next
/// start, and leaves nothing behind when forgotten.
#[tokio::test]
async fn an_endpoint_can_be_added_by_address() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let key = pm_stream::Key::random();
    let ep = pm_endpoint::Endpoint::new("Gil's laptop", pm_endpoint::local_outputs(), key.clone());
    std::thread::spawn(move || ep.serve(listener));
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (st, outs) = call(
        &app,
        "POST",
        "/v1/outputs/endpoints",
        Some(serde_json::json!({"address": address, "key": key.expose()})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{outs}");
    let outs = outs.as_array().unwrap().clone();
    assert!(!outs.is_empty());
    assert!(outs.iter().all(|o| o["protocol"] == "net"), "{outs:?}");
    assert!(outs[0]["name"].as_str().unwrap().starts_with("Gil's laptop · "));
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    assert!(
        zones
            .as_array()
            .unwrap()
            .iter()
            .any(|z| z["output"] == outs[0]["id"]),
        "{zones}"
    );
    let (_, rep) = call(&app, "GET", "/v1/outputs/discovery", None).await;
    assert_eq!(rep["saved_endpoints"][0], address);
    // Adding again (the app restarted) changes nothing.
    let (st, again) = call(
        &app,
        "POST",
        "/v1/outputs/endpoints",
        Some(serde_json::json!({"address": address, "key": key.expose()})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(again.as_array().unwrap().len(), outs.len());
    let (_, zones2) = call(&app, "GET", "/v1/zones", None).await;
    assert_eq!(zones2.as_array().unwrap().len(), zones.as_array().unwrap().len());
    let (st, e) = call(
        &app,
        "POST",
        "/v1/outputs/endpoints",
        Some(serde_json::json!({"address": "127.0.0.1:1", "key": key.expose()})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{e}");
    let enc = address.replace(':', "%3A");
    let (st, left) = call(&app, "DELETE", &format!("/v1/outputs/endpoints/{enc}"), None).await;
    assert_eq!(st, StatusCode::OK, "{left}");
    assert!(left.as_array().unwrap().is_empty());
    let (_, zones3) = call(&app, "GET", "/v1/zones", None).await;
    assert!(zones3
        .as_array()
        .unwrap()
        .iter()
        .all(|z| z["output"] != outs[0]["id"]));
    let (_, outs3) = call(&app, "GET", "/v1/outputs", None).await;
    assert!(outs3.as_array().unwrap().iter().all(|o| o["protocol"] != "net"));
}

/// The player's key: the node stores it per address in a
/// private file, replaces it when the app registers again with a new one, uses it after a
/// restart, and never shows it; a missing or wrong key is refused with a reason.
#[tokio::test]
async fn an_endpoint_key_is_kept_per_address_and_never_shown() {
    fn endpoint(key: &pm_stream::Key, l: std::net::TcpListener) -> pm_endpoint::Serving {
        let outs = pm_zones::OutputRegistry::new();
        outs.register(Arc::new(pm_zones::outputs::NullOutput::new()));
        pm_endpoint::Endpoint::new("Laptop", outs, key.clone())
            .spawn(l)
            .unwrap()
    }
    let first = pm_stream::Key::random();
    let serving = endpoint(&first, std::net::TcpListener::bind("127.0.0.1:0").unwrap());
    let address = serving.addr().to_string();
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let register = |key: serde_json::Value| {
        let (app, address) = (app.clone(), address.clone());
        async move {
            call(
                &app,
                "POST",
                "/v1/outputs/endpoints",
                Some(serde_json::json!({"address": address, "key": key})),
            )
            .await
        }
    };
    let stored = || std::fs::read_to_string(dir.path().join("endpoint-keys.json")).unwrap_or_default();
    let (st, e) = call(
        &app,
        "POST",
        "/v1/outputs/endpoints",
        Some(serde_json::json!({"address": address})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(e.to_string().contains("update the Odeion app"), "{e}");
    let (st, e) = register(serde_json::json!(pm_stream::Key::random().expose())).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(e.to_string().contains("wrong key"), "{e}");
    assert!(stored().is_empty(), "a key that did not work is not kept");

    let (st, outs) = register(serde_json::json!(first.expose())).await;
    assert_eq!(st, StatusCode::OK, "{outs}");
    let out_id = outs[0]["id"].as_str().unwrap().to_string();
    assert!(stored().contains(first.expose()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("endpoint-keys.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    for path in ["/v1/outputs", "/v1/zones", "/v1/outputs/discovery?quick=1"] {
        let (_, body) = call(&app, "GET", path, None).await;
        assert!(!body.to_string().contains(first.expose()), "{path} shows the key");
    }
    assert!(!outs.to_string().contains(first.expose()));

    // The app starts again: the same address, a new key. The zone it had plays with it.
    drop(serving);
    let second = pm_stream::Key::random();
    let _serving = endpoint(&second, std::net::TcpListener::bind(&address).unwrap());
    let (st, again) = register(serde_json::json!(second.expose())).await;
    assert_eq!(st, StatusCode::OK, "{again}");
    assert!(stored().contains(second.expose()) && !stored().contains(first.expose()));
    let out = s.outputs.get(&OutputId(out_id.clone())).unwrap();
    let fmt = pm_model::AudioFormat::pcm(48_000, 16, 2);
    drop(out.open(fmt).await.unwrap());

    // The node restarts: the stored key reaches the player
    let s2 = state(dir.path(), None).await;
    assert!(s2.outputs.get(&OutputId(out_id.clone())).is_none());
    let st2 = s2.clone();
    let found = tokio::task::spawn_blocking(move || st2.restore_endpoint_addresses())
        .await
        .unwrap();
    assert_eq!(found, vec![OutputId(out_id.clone())]);

    // Forgotten: the key goes too
    let enc = address.replace(':', "%3A");
    let (st, _) = call(&app, "DELETE", &format!("/v1/outputs/endpoints/{enc}"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(!stored().contains(second.expose()));
}

/// The app registers its player again while its zone plays (it reconnected: Back onto its connect
/// page, back on screen, a USB DAC): the same output, so the zone plays on, the same song and
/// queue, and its stream to the player is never cut (TASK-android-back-reregister-1111).
#[tokio::test]
async fn the_same_player_registering_again_leaves_its_zone_playing() {
    use pm_zones::outputs::NullOutput;
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let mock = Router::new().route(
        "/song.flac",
        get(move || {
            let f = flac.clone();
            async move { f }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    // The phone's player: a real-time null output, so its zone plays for as long as the song lasts
    let outs = pm_zones::OutputRegistry::new();
    outs.register(Arc::new(NullOutput::realtime()));
    let key = pm_stream::Key::random();
    let serving = pm_endpoint::Endpoint::new("Pixel 8", outs, key.clone())
        .spawn(std::net::TcpListener::bind("127.0.0.1:0").unwrap())
        .unwrap();
    let address = serving.addr().to_string();
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let register = || {
        let (app, address, key) = (app.clone(), address.clone(), key.clone());
        async move {
            call(
                &app,
                "POST",
                "/v1/outputs/endpoints",
                Some(serde_json::json!({"address": address, "key": key.expose()})),
            )
            .await
        }
    };
    let (st, outs) = register().await;
    assert_eq!(st, StatusCode::OK, "{outs}");
    let out_id = outs[0]["id"].as_str().unwrap().to_string();
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones
        .as_array()
        .unwrap()
        .iter()
        .find(|z| z["output"] == out_id.as_str())
        .unwrap_or_else(|| panic!("no zone for {out_id}: {zones}"))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let zone = || {
        let (app, zid) = (app.clone(), zid.clone());
        async move { call(&app, "GET", &format!("/v1/zones/{zid}"), None).await.1 }
    };
    let queue = || {
        let (app, zid) = (app.clone(), zid.clone());
        async move { call(&app, "GET", &format!("/v1/zones/{zid}/queue"), None).await.1 }
    };
    let (st, _) = call(&app, "POST", &format!("/v1/zones/{zid}/queue"), Some(serde_json::json!({"items": [
        {"uri": format!("{base}/song.flac"), "title": "A", "artist": "Band"},
        {"uri": format!("{base}/song.flac"), "title": "B", "artist": "Band"},
        {"uri": format!("{base}/song.flac"), "title": "C", "artist": "Band"}], "replace": true, "play": true}))).await;
    assert_eq!(st, StatusCode::OK);
    let mut z = serde_json::Value::Null;
    for _ in 0..100 {
        z = zone().await;
        if z["transport"] == "playing" && z["now_playing"]["position_ms"].as_u64().unwrap_or(0) >= 1000 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(z["transport"], "playing", "{z}");
    let q0 = queue().await;
    let pos0 = z["now_playing"]["position_ms"].as_u64().unwrap();
    // The app registers again, twice (a reconnect, then the page's own load), with the same key
    for _ in 0..2 {
        let (st, again) = register().await;
        assert_eq!(st, StatusCode::OK, "{again}");
        assert_eq!(again[0]["id"], out_id.as_str(), "the same output: {again}");
    }
    // Still the one zone on the player's output, playing the same song, on from where it was
    let (_, zones2) = call(&app, "GET", "/v1/zones", None).await;
    assert_eq!(
        zones2
            .as_array()
            .unwrap()
            .iter()
            .filter(|z| z["output"] == out_id.as_str())
            .count(),
        1,
        "{zones2}"
    );
    let mut last = pos0;
    for _ in 0..15 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let z = zone().await;
        assert_eq!(
            z["transport"], "playing",
            "the zone stopped after the player registered again: {z}"
        );
        assert_eq!(z["now_playing"]["item"]["title"], "A", "{z}");
        assert_eq!(z["queue_index"], 0, "{z}");
        let pos = z["now_playing"]["position_ms"].as_u64().unwrap_or(0);
        assert!(pos >= last, "the song started again ({last} -> {pos} ms): {z}");
        last = pos;
    }
    assert!(last > pos0, "the position moved on ({pos0} -> {last} ms)");
    assert_eq!(queue().await, q0, "the queue is as it was");
    drop(serving);
}

/// an output and a zone, is remembered for the next start, and can be
/// forgotten again.
#[tokio::test]
async fn a_renderer_can_be_added_by_address() {
    let desc = r#"<?xml version="1.0"?>
<root xmlns="urn:schemas-upnp-org:device-1-0">
  <device>
    <deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType>
    <friendlyName>Living Room DSM</friendlyName>
    <manufacturer>Linn Products Ltd.</manufacturer>
    <UDN>uuid:4c494e4e-0026-0f21-cc3f-01234567890a</UDN>
    <serviceList>
      <service>
        <serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType>
        <serviceId>urn:upnp-org:serviceId:AVTransport</serviceId>
        <controlURL>/AVTransport/control</controlURL>
      </service>
    </serviceList>
  </device>
</root>"#;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = Router::new().route(
        "/desc.xml",
        get(move || async move { ([(header::CONTENT_TYPE, "text/xml")], desc) }),
    );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let address = format!("http://127.0.0.1:{port}/desc.xml");
    let (st, outs) = call(
        &app,
        "POST",
        "/v1/outputs/upnp",
        Some(serde_json::json!({"address": address})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{outs}");
    assert!(
        outs.as_array()
            .unwrap()
            .iter()
            .any(|o| o["name"] == "Living Room DSM"),
        "{outs}"
    );
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    assert!(zones
        .as_array()
        .unwrap()
        .iter()
        .any(|z| z["name"] == "Living Room DSM"));
    let (_, rep) = call(&app, "GET", "/v1/outputs/discovery", None).await;
    assert_eq!(rep["saved_addresses"][0], address);
    // Adding again is idempotent; a dead address is a 400 with the reason.
    let (st, _) = call(
        &app,
        "POST",
        "/v1/outputs/upnp",
        Some(serde_json::json!({"address": address})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, e) = call(
        &app,
        "POST",
        "/v1/outputs/upnp",
        Some(serde_json::json!({"address": "127.0.0.2:1"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{e}");
    assert!(e["error"].as_str().unwrap().contains("did not answer"), "{e}");
    let enc = address.replace('/', "%2F").replace(':', "%3A");
    let (st, left) = call(&app, "DELETE", &format!("/v1/outputs/upnp/{enc}"), None).await;
    assert_eq!(st, StatusCode::OK, "{left}");
    assert_eq!(left.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn the_discovery_report_explains_an_empty_network() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s);
    let (st, v) = call(&app, "GET", "/v1/outputs/discovery", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["upnp_enabled"], false);
    assert_eq!(v["upnp"].as_array().unwrap().len(), 0);
    assert_eq!(v["public_url"], "http://127.0.0.1:7788");
    let notes: Vec<String> = serde_json::from_value(v["notes"].clone()).unwrap();
    assert!(notes.iter().any(|n| n.contains("both off")), "{notes:?}");
    assert!(
        notes
            .iter()
            .any(|n| n.contains("public_url is http://127.0.0.1:7788")),
        "{notes:?}"
    );
}

/// The network check says when an address the node picked for itself looks wrong for
/// speakers (Docker's bridge, a VPN); never about one the owner set.
#[tokio::test]
async fn the_network_check_doubts_a_guessed_bridge_or_vpn_address() {
    let dir = tempfile::tempdir().unwrap();
    let notes = |s: Arc<NodeState>| async move {
        let (_, r) = call(&router(s), "GET", "/v1/outputs/discovery?quick=1", None).await;
        r["notes"].to_string()
    };
    let docker = notes(state_at(dir.path(), "http://172.17.0.2:7788", true).await).await;
    assert!(docker.contains("Docker's bridge network"), "{docker}");
    let vpn = notes(state_at(dir.path(), "http://100.101.2.3:7788", true).await).await;
    assert!(vpn.contains("a VPN?"), "{vpn}");
    let home = notes(state_at(dir.path(), "http://192.168.1.10:7788", true).await).await;
    assert!(!home.contains("picked"), "{home}");
    let set = notes(state_at(dir.path(), "http://172.17.0.2:7788", false).await).await;
    assert!(!set.contains("picked"), "{set}");
}

/// A phone's USB DAC: the node tells owners (`outputs_changed`) only once the phone's output is
/// in the list, so a controller that fetches `GET /v1/outputs` on the event finds it with its
/// device; plugging out tells them again; listeners never hear it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_output_device_is_announced_after_its_output_is_listed() {
    struct Phone {
        inner: NullOutput,
        dac: std::sync::Mutex<Option<pm_zones::OutputDevice>>,
    }
    #[async_trait::async_trait]
    impl pm_zones::Output for Phone {
        fn id(&self) -> OutputId {
            OutputId::new("cpal", "Speaker or headphones")
        }
        fn name(&self) -> String {
            "Speaker or headphones (Android audio, shared mode)".into()
        }
        fn device(&self) -> Option<pm_zones::OutputDevice> {
            self.dac.lock().unwrap().clone()
        }
        async fn configure_device(
            &self,
            settings: pm_zones::DeviceSettings,
        ) -> Result<(), pm_zones::SinkError> {
            if let Some(d) = self.dac.lock().unwrap().as_mut() {
                d.settings = Some(settings);
            }
            Ok(())
        }
        fn caps(&self) -> &pm_model::OutputCaps {
            self.inner.caps()
        }
        async fn open(
            &self,
            fmt: pm_model::AudioFormat,
        ) -> Result<Box<dyn pm_zones::AudioSink>, pm_zones::SinkError> {
            self.inner.open(fmt).await
        }
    }
    let ka17 = pm_zones::OutputDevice {
        name: "FiiO KA17".into(),
        ..Default::default()
    };
    let phone = Arc::new(Phone {
        inner: NullOutput::new(),
        dac: std::sync::Mutex::new(Some(ka17)),
    });
    let outs = pm_zones::OutputRegistry::new();
    outs.register(phone.clone());
    let key = pm_stream::Key::random();
    let serving = pm_endpoint::Endpoint::new("Pixel 8", outs, key.clone())
        .spawn(std::net::TcpListener::bind("127.0.0.1:0").unwrap())
        .unwrap();
    let address = serving.addr().to_string();
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    // What a controller would see when it fetches the outputs the moment the event arrives
    let mut rx = s.events.subscribe();
    let s2 = s.clone();
    // Each observation is also signalled, so the test changes the phone only once the one
    // before was observed (the observer reads the registry as it is then)
    let (observed_tx, observed) = std::sync::mpsc::channel::<()>();
    let observed = std::sync::Arc::new(std::sync::Mutex::new(observed));
    let wait_observed = || {
        let o = observed.clone();
        async move {
            tokio::task::spawn_blocking(move || {
                o.lock()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .expect("an outputs_changed event")
            })
            .await
            .unwrap()
        }
    };
    let seen = std::thread::spawn(move || {
        let mut seen = Vec::new();
        while let Ok(ev) = rx.blocking_recv() {
            match &ev.event {
                ApiEvent::OutputsChanged { output } => {
                    let listed = s2.outputs.get(output).map(|o| o.device().map(|d| d.name));
                    seen.push((ev.event.clone(), listed));
                    let _ = observed_tx.send(());
                }
                // The test's own end mark
                ApiEvent::LibraryChanged => break,
                _ => {}
            }
        }
        seen
    });
    let register = || {
        let (app, address, key) = (app.clone(), address.clone(), key.clone());
        async move {
            call(
                &app,
                "POST",
                "/v1/outputs/endpoints",
                Some(serde_json::json!({"address": address, "key": key.expose()})),
            )
            .await
        }
    };
    let (st, body) = register().await;
    assert_eq!(st, StatusCode::OK, "{body}");
    wait_observed().await;
    // The owner's choices reach the phone, and the answer shows them
    let id = body[0]["id"].as_str().unwrap().to_string();
    let enc = id
        .replace('%', "%25")
        .replace('/', "%2F")
        .replace(':', "%3A")
        .replace(' ', "%20");
    // An earlier UI's body is taken and means the one behaviour (nothing to choose, no
    // agreement to full level); "Allow Exclusive USB" (`mode: exclusive`) reaches the phone
    let (st, out) = call(
        &app,
        "PUT",
        &format!("/v1/outputs/{enc}/device"),
        Some(serde_json::json!({"mode": "shared", "dac_volume": false, "full_level_ok": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{out}");
    assert_eq!(
        out["device"]["settings"],
        serde_json::json!({"mode": "auto", "dac_volume": true, "full_level_ok": false})
    );
    wait_observed().await;
    let (st, out) = call(
        &app,
        "PUT",
        &format!("/v1/outputs/{enc}/device"),
        Some(serde_json::json!({"mode": "exclusive"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{out}");
    assert_eq!(out["device"]["settings"]["mode"], "exclusive");
    let (st, _) = call(
        &app,
        "PUT",
        &format!("/v1/outputs/{enc}/device"),
        Some(serde_json::json!({"mode": "loudest"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "a mode this node doesn't know");
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/outputs/null/device",
        Some(serde_json::json!({"mode": "shared"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "an output without a device");
    wait_observed().await;
    *phone.dac.lock().unwrap() = None;
    let (st, _) = register().await;
    assert_eq!(st, StatusCode::OK);
    s.events.send(ApiEvent::LibraryChanged);
    let seen = tokio::task::spawn_blocking(move || seen.join().unwrap())
        .await
        .unwrap();
    assert_eq!(
        seen.len(),
        4,
        "plugged in at the first registration, the settings twice, then out"
    );
    assert_eq!(
        seen[0].1,
        Some(Some("FiiO KA17".to_string())),
        "listed, with its device, when announced"
    );
    for i in [1, 2] {
        assert_eq!(
            seen[i].1,
            Some(Some("FiiO KA17".to_string())),
            "its settings changed"
        );
    }
    assert_eq!(seen[3].1, Some(None), "listed, without a device, when unplugged");
    let listener = accounts::Caller {
        role: accounts::Role::Listener,
        key: Some("o4dksfbqk85ogzdb5osziw6befigbuxmuxkuxq8434q89uj56uyy".into()),
    };
    let owner = accounts::Caller {
        role: accounts::Role::Owner,
        key: None,
    };
    for (ev, _) in &seen {
        assert!(
            events::for_caller(&s, &listener, ev).is_none(),
            "listeners don't hear it"
        );
        assert!(events::for_caller(&s, &owner, ev).is_some(), "owners do");
    }
    drop(serving);
}

/// Outputs found on the network stop being added once the node has `MAX_FOUND_OUTPUTS`, and
/// are added again when there is room.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn outputs_found_on_the_network_are_capped() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let max = crate::state::MAX_FOUND_OUTPUTS;
    let had = s.outputs.len();
    let found: Vec<Arc<dyn Output>> = (0..max + 10)
        .map(|i| {
            Arc::new(pm_zones::outputs::PullOutput::controlled(&format!("found-{i}"))) as Arc<dyn Output>
        })
        .collect();
    s.set_discover(Arc::new(move || found.clone()));
    let st = s.clone();
    let new = tokio::task::spawn_blocking(move || st.rescan_outputs())
        .await
        .unwrap();
    assert_eq!(new.len(), max - had, "added up to the cap");
    assert_eq!(s.outputs.len(), max);
    s.outputs.remove(&new[0]);
    let st = s.clone();
    let again = tokio::task::spawn_blocking(move || st.rescan_outputs())
        .await
        .unwrap();
    assert_eq!(again.len(), 1, "one place freed, one added");
    assert_eq!(s.outputs.len(), max);
}

/// A device the node turns away for want of room leaves no pull stream behind (rescans with new
/// devices can't grow the node past the cap); once there is room it is taken, stream and all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_turned_away_leaves_no_pull_stream() {
    // A stand-in MPD server: the greeting, then OK to anything
    let mpd = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = mpd.local_addr().unwrap();
    std::thread::spawn(move || {
        for c in mpd.incoming().flatten() {
            std::thread::spawn(move || {
                use std::io::{BufRead, Write};
                let mut w = c.try_clone().unwrap();
                let _ = w.write_all(b"OK MPD 0.23.5\n");
                for line in std::io::BufReader::new(c).lines() {
                    if line.is_err() || w.write_all(b"OK\n").is_err() {
                        break;
                    }
                }
            });
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let max = crate::state::MAX_FOUND_OUTPUTS;
    let fill: Vec<OutputId> = (s.outputs.len()..max)
        .map(|i| {
            let o = Arc::new(pm_zones::outputs::PullOutput::controlled(&format!("filler-{i}")));
            let id = o.id();
            s.outputs.register(o);
            id
        })
        .collect();
    s.set_mpd(&[addr.to_string()]);
    let pulls_before = s.pulls.lock().unwrap().len();
    let st = s.clone();
    let new = tokio::task::spawn_blocking(move || st.discover_mpd())
        .await
        .unwrap();
    assert!(new.is_empty(), "no room: {new:?}");
    assert_eq!(s.pulls.lock().unwrap().len(), pulls_before, "nothing left behind");
    s.outputs.remove(&fill[0]);
    let st = s.clone();
    let new = tokio::task::spawn_blocking(move || st.discover_mpd())
        .await
        .unwrap();
    assert_eq!(new.len(), 1, "room again: taken");
    assert_eq!(s.pulls.lock().unwrap().len(), pulls_before + 1, "with its stream");
}

/// The node's own zones, offered as UPnP renderers, are never taken as outputs: not by their
/// UDN, not by their description's address (loopback or a LAN address of this host), and not
/// when typed as an address. A rescan once made `<zone>-2` zones playing into the node itself.
#[tokio::test]
async fn our_own_renderers_get_no_zone() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    if s.zones.list().is_empty() {
        s.create_zone("Kitchen", Arc::new(NullOutput::new()));
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let _adv = s.start_renderer(port, false);
    let app = router(s.clone());
    tokio::spawn({
        let app = app.clone();
        async move { axum::serve(listener, app).await.unwrap() }
    });
    let own = s.renderer.get().unwrap().clone();
    let (zone, _) = own.zones().into_iter().next().expect("a zone offered");
    let zones = s.zones.list().len();
    let r = |udn: String, location: String| pm_out_upnp::Renderer {
        udn,
        friendly_name: "Kitchen".into(),
        manufacturer: None,
        model: None,
        av_transport: "http://192.0.2.9/avt".into(),
        rendering_control: None,
        connection_manager: None,
        zone_group_topology: None,
        location,
        events: Default::default(),
    };
    // Our UDN, wherever it says it is
    let by_udn = r(
        format!("uuid:{}", own.udn(&zone)),
        "http://192.0.2.9:1400/desc.xml".into(),
    );
    assert!(s.is_own_renderer(&by_udn));
    assert!(s.register_renderer(by_udn).is_none());
    // Our description (this host, our port, a zone's path), whatever its UDN
    let by_place = r(
        "uuid:somebody-else".into(),
        format!("http://127.0.0.1:{port}/upnp/zone/{zone}/desc.xml"),
    );
    assert!(s.is_own_renderer(&by_place));
    assert!(s.register_renderer(by_place).is_none());
    // …also at this host's LAN address, as the node announces it
    if let Some(ip) = pm_out_upnp::local_ipv4_addrs().first() {
        assert!(s.is_own_location(&format!("http://{ip}:{port}/upnp/zone/{zone}/desc.xml")));
    }
    // Someone else's: another port, or another path
    assert!(!s.is_own_location(&format!(
        "http://127.0.0.1:{}/upnp/zone/{zone}/desc.xml",
        port + 1
    )));
    assert!(!s.is_own_location(&format!("http://127.0.0.1:{port}/desc.xml")));
    assert_eq!(s.zones.list().len(), zones, "no zone for our own renderer");
    // Typed as an address: refused, saying why, and not remembered
    let address = format!("http://127.0.0.1:{port}/upnp/zone/{zone}/desc.xml");
    let (st, e) = call(
        &app,
        "POST",
        "/v1/outputs/upnp",
        Some(serde_json::json!({"address": address})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{e}");
    assert!(e["error"].as_str().unwrap().contains("own zone"), "{e}");
    assert!(s.upnp_addresses().is_empty());
    assert_eq!(s.zones.list().len(), zones);
}

/// An AirPlay receiver and a BluOS player that were off at the start get their zones from the
/// mDNS watch, without a rescan, each with the queue it had saved; found again on every pass, or
/// by two registrations at once, each is registered once (no `-2` zone).
#[tokio::test]
async fn speakers_switched_on_after_the_start_get_their_zones_and_queues() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let air = || pm_out_airplay::Device {
        id: "90568261365F".into(),
        name: "ecofriday".into(),
        model: Some("PULSE FLEX 2i".into()),
        addr: "127.0.0.1:9".parse().unwrap(),
        encryption: vec!["0".into(), "4".into()],
        codecs: vec!["0".into(), "1".into()],
    };
    let blu = || pm_out_bluos::Player {
        id: "90568261365f".into(),
        name: "ecofriday".into(),
        host: "127.0.0.1".into(),
        port: 9,
        model: Some("PULSE FLEX 2i".into()),
    };
    let zones_of = |s: &Arc<NodeState>, outs: &[OutputId]| -> Vec<String> {
        let mut z: Vec<String> = s
            .zones
            .list()
            .iter()
            .filter(|z| outs.contains(&z.output))
            .map(|z| z.id.to_string())
            .collect();
        z.sort();
        z
    };
    let titles = |z: &serde_json::Value| -> Vec<String> {
        z["queue"]
            .as_array()
            .map(|q| {
                q.iter()
                    .map(|i| i["title"].as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default()
    };
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("short.wav");
    write_wav(&wav, 441);

    // Before the restart: both speakers had zones, each with its queue (saved)
    let (outs, zids) = {
        let s = state(dir.path(), None).await;
        let app = router(s.clone());
        let mut outs = s.register_airplay(vec![air()]);
        outs.extend(s.register_bluos(vec![blu()]));
        assert_eq!(outs.len(), 2, "{outs:?}");
        let zids = zones_of(&s, &outs);
        assert_eq!(zids.len(), 2, "{zids:?}");
        for zid in &zids {
            let items: Vec<_> = (0..3)
                .map(|i| serde_json::json!({"uri": wav.to_string_lossy(), "title": format!("{zid} {i}")}))
                .collect();
            let (st, _) = call(
                &app,
                "POST",
                &format!("/v1/zones/{zid}/queue"),
                Some(serde_json::json!({"items": items})),
            )
            .await;
            assert_eq!(st, StatusCode::OK);
            let key = format!("queue.{zid}");
            let want = format!("{zid} 2");
            let saved = tokio::time::timeout(std::time::Duration::from_secs(30), async {
                while !s
                    .library
                    .lock()
                    .unwrap()
                    .get_setting(&key)
                    .unwrap()
                    .is_some_and(|j| j.contains(&want))
                {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await;
            assert!(saved.is_ok(), "{zid}'s queue was not saved");
        }
        (outs, zids)
    };

    // The start: both speakers are off. They answer from the watch's third pass on
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    assert!(zones_of(&s, &outs).is_empty(), "no zone while they are off");
    let passes = Arc::new(AtomicUsize::new(0));
    let n = passes.clone();
    let browse: crate::state::MdnsBrowse = Arc::new(move || {
        if n.fetch_add(1, Ordering::SeqCst) < 2 {
            (Vec::new(), Vec::new())
        } else {
            (vec![air()], vec![blu()])
        }
    });
    s.spawn_mdns_passes(
        std::time::Duration::from_millis(50),
        std::time::Duration::from_millis(300),
        browse,
    );
    let mut waited = 0;
    while passes.load(Ordering::SeqCst) < 2 && waited < 100 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        waited += 1;
    }
    assert!(zones_of(&s, &outs).is_empty(), "still off: no zone");
    let mut waited = 0;
    while zones_of(&s, &outs).len() < 2 && waited < 100 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        waited += 1;
    }
    assert_eq!(
        zones_of(&s, &outs),
        zids,
        "both came back, under their own ids, without a rescan"
    );
    for zid in &zids {
        let (_, z) = call(&app, "GET", &format!("/v1/zones/{zid}"), None).await;
        let want: Vec<String> = (0..3).map(|i| format!("{zid} {i}")).collect();
        assert_eq!(titles(&z), want, "{zid}'s saved queue is back");
    }
    // Found again on later passes: nothing more
    let seen = passes.load(Ordering::SeqCst);
    let mut waited = 0;
    while passes.load(Ordering::SeqCst) < seen + 2 && waited < 1500 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        waited += 1;
    }
    assert!(
        passes.load(Ordering::SeqCst) >= seen + 2,
        "the watch keeps looking"
    );
    assert_eq!(zones_of(&s, &outs), zids, "one zone each, no -2");

    // Two registrations of the same speakers at once (a rescan during a pass): one zone each
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let both = |s: &Arc<NodeState>| {
        let s = s.clone();
        tokio::task::spawn_blocking(move || {
            let mut new = s.register_airplay(vec![air()]);
            new.extend(s.register_bluos(vec![blu()]));
            new
        })
    };
    let (a, b) = tokio::join!(both(&s), both(&s));
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(a.len() + b.len(), 2, "{a:?} {b:?}");
    assert_eq!(zones_of(&s, &outs).len(), 2);
    assert!(
        s.zones.list().iter().all(|z| !z.id.to_string().ends_with("-2")),
        "no -2 zone"
    );
}

/// A renderer that had a zone comes back at a start whose SSDP search missed it, described at
/// its last URL (once: one with its zone isn't described again); one that answers only later
/// gets its zone from the next pass, without a rescan; another device now at a known URL gets
/// its own zone and the old one is forgotten.
#[tokio::test]
async fn a_known_renderer_missed_by_the_search_gets_its_zone() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let linn = "uuid:4c494e4e-0026-0f21-cc3f-0123456789ab";
    let named = |s: &Arc<NodeState>, n: &str| s.zones.list().iter().filter(|z| z.name == n).count();
    let restore = |s: &Arc<NodeState>| {
        let s = s.clone();
        tokio::task::spawn_blocking(move || s.restore_known())
    };
    // Known, answering: back at once, and described once
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let hits = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://127.0.0.1:{}/desc.xml",
        listener.local_addr().unwrap().port()
    );
    fake_renderer_at(
        listener,
        linn,
        "Klimax UPnP AV",
        hits.clone(),
        Arc::new(AtomicBool::new(true)),
    )
    .await;
    s.remember_upnp(linn, &url);
    assert_eq!(restore(&s).await.unwrap().len(), 1);
    assert_eq!(named(&s, "Klimax UPnP AV"), 1);
    let fetched = hits.load(Ordering::SeqCst);
    assert!(restore(&s).await.unwrap().is_empty());
    assert_eq!(
        hits.load(Ordering::SeqCst),
        fetched,
        "a renderer with its zone isn't described again"
    );
    assert_eq!(s.upnp_known().get(linn).map(|k| k.location.clone()), Some(url));

    // Known, answering only after the start: the next pass gives it its zone
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let ready = Arc::new(AtomicBool::new(false));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    s.remember_upnp(
        linn,
        &format!(
            "http://127.0.0.1:{}/desc.xml",
            listener.local_addr().unwrap().port()
        ),
    );
    fake_renderer_at(
        listener,
        linn,
        "Klimax UPnP AV",
        Arc::new(AtomicUsize::new(0)),
        ready.clone(),
    )
    .await;
    s.spawn_upnp_passes(
        std::time::Duration::from_millis(50),
        std::time::Duration::from_millis(150),
    );
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert_eq!(named(&s, "Klimax UPnP AV"), 0, "nothing answers yet");
    assert!(s.upnp_known().contains_key(linn), "one not answering stays known");
    ready.store(true, Ordering::SeqCst);
    let mut waited = 0;
    while named(&s, "Klimax UPnP AV") == 0 && waited < 50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        waited += 1;
    }
    assert_eq!(
        named(&s, "Klimax UPnP AV"),
        1,
        "the late renderer got its zone without a rescan"
    );

    // Another device now at a known URL: its own zone, and the old one forgotten
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://127.0.0.1:{}/desc.xml",
        listener.local_addr().unwrap().port()
    );
    let tv = "uuid:2555a126-0000-4000-8000-000000000001";
    fake_renderer_at(
        listener,
        tv,
        "The Frame",
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicBool::new(true)),
    )
    .await;
    s.remember_upnp(linn, &url);
    assert_eq!(restore(&s).await.unwrap().len(), 1);
    assert_eq!(named(&s, "The Frame"), 1);
    assert!(
        !s.upnp_known().contains_key(linn),
        "the old renderer at that URL is forgotten"
    );
    assert_eq!(
        s.upnp_known().get(tv).map(|k| k.location.clone()),
        Some(url.clone())
    );
    // A restore still describing a renderer at its old address while another pass registers it
    // at its new one: the old address isn't written back over the new (the old description is held
    // until the registration is done, so the restore is surely past its own check)
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let (arrived, release) = (
        Arc::new(tokio::sync::Notify::new()),
        Arc::new(tokio::sync::Notify::new()),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let old_url = format!(
        "http://127.0.0.1:{}/desc.xml",
        listener.local_addr().unwrap().port()
    );
    fake_renderer_gated(listener, linn, "Klimax UPnP AV", arrived.clone(), release.clone()).await;
    s.remember_upnp(linn, &old_url);
    let restoring = restore(&s);
    tokio::time::timeout(std::time::Duration::from_secs(5), arrived.notified())
        .await
        .expect("the restore asks the old address");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let new_url = format!(
        "http://127.0.0.1:{}/desc.xml",
        listener.local_addr().unwrap().port()
    );
    fake_renderer_at(
        listener,
        linn,
        "Klimax UPnP AV",
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicBool::new(true)),
    )
    .await;
    // (blocking HTTP, off the runtime's threads)
    let registered = tokio::task::spawn_blocking({
        let s = s.clone();
        let url = new_url.clone();
        move || {
            let found = pm_out_upnp::describe(&url).unwrap().unwrap();
            s.register_renderer(found).is_some()
        }
    })
    .await
    .unwrap();
    assert!(registered, "registered at its new address meanwhile");
    release.notify_one();
    assert!(
        restoring.await.unwrap().is_empty(),
        "the late answer registers nothing"
    );
    assert_eq!(
        s.upnp_known().get(linn).map(|k| k.location.clone()),
        Some(new_url),
        "the new address stands"
    );
    assert_eq!(named(&s, "Klimax UPnP AV"), 1);

    // A known address now answered by another device that already has its zone: the old one is
    // forgotten there all the same
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let shared = format!(
        "http://127.0.0.1:{}/desc.xml",
        listener.local_addr().unwrap().port()
    );
    fake_renderer_at(
        listener,
        tv,
        "The Frame",
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicBool::new(true)),
    )
    .await;
    let has_zone = tokio::task::spawn_blocking({
        let s = s.clone();
        let url = shared.clone();
        move || {
            s.register_renderer(pm_out_upnp::describe(&url).unwrap().unwrap())
                .is_some()
        }
    })
    .await
    .unwrap();
    assert!(has_zone);
    s.remember_upnp(linn, &shared);
    assert!(restore(&s).await.unwrap().is_empty());
    assert!(
        !s.upnp_known().contains_key(linn),
        "the old renderer at that address is forgotten"
    );
    assert_eq!(named(&s, "The Frame"), 1);

    // A restore's save keeps an address another pass saved meanwhile, and adds one not known
    s.remember_upnp(linn, "http://192.0.2.5/desc.xml");
    s.remember_upnp_unless_moved(linn, "http://192.0.2.4/desc.xml");
    assert_eq!(
        s.upnp_known().get(linn).map(|k| k.location.clone()).as_deref(),
        Some("http://192.0.2.5/desc.xml")
    );
    s.remember_upnp_unless_moved("uuid:fresh", "http://192.0.2.6/desc.xml");
    assert_eq!(
        s.upnp_known()
            .get("uuid:fresh")
            .map(|k| k.location.clone())
            .as_deref(),
        Some("http://192.0.2.6/desc.xml")
    );
    // Forgetting is for the address it was at: one found elsewhere meanwhile stays
    s.remember_upnp(linn, "http://192.0.2.7/desc.xml");
    s.forget_upnp_at(linn, &url);
    assert_eq!(
        s.upnp_known().get(linn).map(|k| k.location.clone()).as_deref(),
        Some("http://192.0.2.7/desc.xml")
    );
    // Long unseen ones go before a pass describes them (dead ones don't hold every pass)
    // (as stored by an earlier run: nothing has pruned it since; it answers, but isn't asked)
    let gone_hits = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gone_url = format!(
        "http://127.0.0.1:{}/desc.xml",
        listener.local_addr().unwrap().port()
    );
    fake_renderer_at(
        listener,
        "uuid:gone",
        "Gone",
        gone_hits.clone(),
        Arc::new(AtomicBool::new(true)),
    )
    .await;
    let mut known = s.upnp_known();
    let month_ago = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - 31 * 86_400;
    known.insert(
        "uuid:gone".into(),
        crate::state::KnownRenderer {
            location: gone_url,
            seen: month_ago,
        },
    );
    s.library
        .lock()
        .unwrap()
        .set_setting("outputs.upnp_known", &serde_json::to_string(&known).unwrap())
        .unwrap();
    assert!(s.upnp_known().contains_key("uuid:gone"));
    let _ = restore(&s).await.unwrap();
    assert!(!s.upnp_known().contains_key("uuid:gone"));
    assert_eq!(
        gone_hits.load(Ordering::SeqCst),
        0,
        "a long-unseen renderer isn't asked before it is dropped"
    );
    assert_eq!(named(&s, "Gone"), 0);
}

/// A listen-along group plays on several speakers through the host's settings: its volume
/// limit is the strictest among them, so a joiner's limited speaker never goes past its
/// own limit in the group, and the software volume stays under it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_group_zone_takes_the_strictest_limit_of_its_speakers() {
    use pm_model::{Volume, VolumeKind};
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let host = Arc::new(NotedVolume {
        inner: NullOutput::with_volume(VolumeKind::Db {
            min: -60.0,
            max: 0.0,
            step: 1.0,
        }),
        asked: Default::default(),
    });
    let joiner = Arc::new(NotedVolume {
        inner: NullOutput::with_volume(VolumeKind::Percent),
        asked: Default::default(),
    });
    let (host_out, joiner_out): (Arc<dyn Output>, Arc<dyn Output>) = (host.clone(), joiner.clone());
    s.outputs.register(host_out.clone());
    s.outputs.register(joiner_out.clone());
    let (hid, jid) = (host_out.id(), joiner_out.id());
    // The joiner's speaker is limited to -30 dB; the host's has a software volume and no limit
    s.sources
        .set_dsp(
            &jid,
            Some(pm_audio::PipelineConfig {
                max_volume_db: Some(-30.0),
                ..pm_audio::PipelineConfig::default()
            }),
        )
        .unwrap();
    s.sources
        .set_dsp(
            &hid,
            Some(pm_audio::PipelineConfig {
                volume_db: Some(-6.0),
                ..pm_audio::PipelineConfig::default()
            }),
        )
        .unwrap();
    let group = zone_output(&s, "Den + Kitchen", &[hid.0.clone(), jid.0.clone()])
        .await
        .unwrap();
    s.sources
        .carry_dsp(&hid, &group.id(), &[hid.clone(), jid.clone()]);
    let carried = s.sources.dsp_for(&group.id());
    assert_eq!(
        (carried.max_volume_db, carried.volume_db),
        (Some(-30.0), Some(-30.0)),
        "the strictest limit, and the host's software volume held under it"
    );
    // Full volume on the group reaches each speaker held to -30 dB
    let zone = s.create_zone("Den + Kitchen", group);
    zone.set_volume(Volume::Percent(100)).await.unwrap();
    zone.wait_acked().await;
    let asked = |o: &NotedVolume| o.asked.lock().unwrap().clone();
    assert_eq!(
        asked(&joiner),
        [Volume::Percent(50)],
        "the joiner's own limit holds"
    );
    assert_eq!(asked(&host), [Volume::Percent(50)]);
    // A software volume under the carried limit stays as it is
    s.sources
        .set_dsp(
            &hid,
            Some(pm_audio::PipelineConfig {
                volume_db: Some(-40.0),
                ..pm_audio::PipelineConfig::default()
            }),
        )
        .unwrap();
    s.sources
        .carry_dsp(&hid, &zone.output().id(), &[hid.clone(), jid.clone()]);
    assert_eq!(s.sources.dsp_for(&zone.output().id()).volume_db, Some(-40.0));
    // No limit anywhere: none carried
    s.sources.set_dsp(&jid, None).unwrap();
    s.sources
        .carry_dsp(&hid, &zone.output().id(), &[hid.clone(), jid.clone()]);
    assert_eq!(s.sources.dsp_for(&zone.output().id()).max_volume_db, None);
}

/// A zone of several speakers the owner makes takes the strictest volume limit among them, as
/// a listen-along group does: a speaker limited to -30 dB is never sent more in the group, a
/// limit lowered while the group exists holds at the next request, and the group's limit goes
/// with its zone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_zone_of_several_speakers_takes_the_strictest_limit_among_them() {
    use pm_model::{Volume, VolumeKind};
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let amp = Arc::new(NotedVolume {
        inner: NullOutput::with_volume(VolumeKind::Db {
            min: -60.0,
            max: 0.0,
            step: 1.0,
        }),
        asked: Default::default(),
    });
    let speaker = Arc::new(NotedVolume {
        inner: NullOutput::with_volume(VolumeKind::Percent),
        asked: Default::default(),
    });
    let (amp_out, speaker_out): (Arc<dyn Output>, Arc<dyn Output>) = (amp.clone(), speaker.clone());
    s.outputs.register(amp_out.clone());
    s.outputs.register(speaker_out.clone());
    let (aid, sid) = (amp_out.id(), speaker_out.id());
    let limit = |db: f32| {
        Some(pm_audio::PipelineConfig {
            max_volume_db: Some(db),
            ..pm_audio::PipelineConfig::default()
        })
    };
    s.sources.set_dsp(&sid, limit(-30.0)).unwrap();
    let (st, z) = call(
        &app,
        "POST",
        "/v1/zones",
        Some(serde_json::json!({"name": "Pair", "outputs": [aid.0, sid.0]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{z}");
    let zone = zone_of(&s, z["id"].as_str().unwrap()).unwrap();
    let group = zone.output().id();
    assert_eq!(s.sources.volume_limit(&group), Some(-30.0));
    let asked = |o: &NotedVolume| o.asked.lock().unwrap().clone();
    // Full volume on the pair reaches each speaker held to -30 dB
    zone.set_volume(Volume::Percent(100)).await.unwrap();
    zone.wait_acked().await;
    assert_eq!(
        asked(&speaker),
        [Volume::Percent(50)],
        "the speaker's own limit holds"
    );
    assert_eq!(asked(&amp), [Volume::Percent(50)]);
    // Lowered while the pair exists: the next request is held to the new limit
    s.sources.set_dsp(&sid, limit(-40.0)).unwrap();
    zone.set_volume(Volume::Percent(100)).await.unwrap();
    zone.wait_acked().await;
    assert_eq!(asked(&speaker).last(), Some(&Volume::Percent(33)));
    assert_eq!(asked(&amp).last(), Some(&Volume::Percent(33)));
    // A second zone on the same speakers shares the entry: deleting the first leaves the
    // second capped; the entry goes with the last zone
    let (st, z2) = call(
        &app,
        "POST",
        "/v1/zones",
        Some(serde_json::json!({"name": "Pair too", "outputs": [aid.0, sid.0]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{z2}");
    let zone2 = zone_of(&s, z2["id"].as_str().unwrap()).unwrap();
    assert_eq!(
        zone2.output().id(),
        group,
        "the same speakers, the same group output"
    );
    let (st, _) = call(&app, "DELETE", &format!("/v1/zones/{}", zone.id()), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(
        s.sources.volume_limit(&group),
        Some(-40.0),
        "the other zone still caps"
    );
    zone2.set_volume(Volume::Percent(100)).await.unwrap();
    zone2.wait_acked().await;
    assert_eq!(asked(&speaker).last(), Some(&Volume::Percent(33)));
    // The last zone goes: so does the group's limit
    let (st, _) = call(&app, "DELETE", &format!("/v1/zones/{}", zone2.id()), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(s.sources.volume_limit(&group), None);
}
