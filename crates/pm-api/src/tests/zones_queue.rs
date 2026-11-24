//! Zones, queues and playback: volume, shuffle, radio, timers, transfers.
//!
//! Part of pm-api's unit tests (`mod tests` in lib.rs, which holds the helpers these use). A new
//! test of this area goes at the end of this file.

use super::*;

#[tokio::test]
async fn scan_then_queue_and_play_a_library_track() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("music")).unwrap();
    write_wav(&dir.path().join("music/one.wav"), 4410);
    let s = state(dir.path(), None).await;
    let app = router(s.clone());

    let (st, info) = call(&app, "GET", "/v1/info", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(info["api"], "v1");
    assert_eq!(info["zones"], 1);

    let (st, _) = call(&app, "POST", "/v1/library/scan", Some(serde_json::json!({}))).await;
    assert_eq!(st, StatusCode::OK);
    for _ in 0..500 {
        if !s.is_scanning() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(!s.is_scanning());
    let (st, tracks) = call(&app, "GET", "/v1/library/tracks?limit=10", None).await;
    assert_eq!(st, StatusCode::OK);
    let id = tracks["tracks"][0]["id"].as_i64().unwrap();
    let (st, found) = call(&app, "GET", "/v1/library/search?q=one", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(found["tracks"][0]["id"], id);

    let (st, zones) = call(&app, "GET", "/v1/zones", None).await;
    assert_eq!(st, StatusCode::OK);
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let (st, q) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"track_ids": [id], "replace": true, "play": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{q}");
    assert_eq!(q["ids"].as_array().unwrap().len(), 1);
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    let st = zone.wait_stopped().await;
    assert_eq!(st.error, None);
    let (st, q) = call(&app, "GET", &format!("/v1/zones/{zid}/queue"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(q[0]["uri"], format!("track:{id}"));
    assert_eq!(q[0]["title"], "One", "an untagged file is titled from its name");

    // The play was recorded by the zone event hook; a client can add its own.
    let mut recorded = false;
    for _ in 0..200 {
        let (_, h) = call(&app, "GET", "/v1/library/history", None).await;
        if !h["plays"].as_array().unwrap().is_empty() {
            assert_eq!(h["plays"][0]["track"]["id"], id);
            assert_eq!(h["plays"][0]["zone"], zid);
            recorded = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(recorded, "node zone play not recorded");
    let (st, h) = call(
        &app,
        "POST",
        "/v1/library/history",
        Some(serde_json::json!({"track_id": id, "zone": "local"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{h}");
    assert_eq!(h["plays"][0]["zone"], "local");
    let (st, _) = call(
        &app,
        "POST",
        "/v1/library/history",
        Some(serde_json::json!({"track_id": 999_999})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, home) = call(&app, "GET", "/v1/library/home", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(home["history"].as_array().unwrap().len(), 2);
    assert_eq!(home["most_played"][0]["plays"], 2);
    assert_eq!(
        home["recently_added"].as_array().unwrap().len(),
        home["stats"]["albums"]
    );

    // Without a contact, the Pubky view of an object is a clear 412; the
    // contact can be set through the API and discovery comes alive.
    let (st, e) = call(&app, "GET", &format!("/v1/discover/uri?track_id={id}"), None).await;
    assert_eq!(st, StatusCode::PRECONDITION_FAILED, "{e}");
    let (st, v) = call(&app, "GET", "/v1/node/settings", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["discovery"], false);
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/node/settings",
        Some(serde_json::json!({"contact": "github.com/x"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, v) = call(
        &app,
        "PUT",
        "/v1/node/settings",
        Some(serde_json::json!({"contact": " me@example.org "})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["contact"], "me@example.org");
    assert_eq!(v["discovery"], true);
    assert_eq!(
        s.library
            .lock()
            .unwrap()
            .get_setting("node.contact")
            .unwrap()
            .as_deref(),
        Some("me@example.org")
    );

    // Unknown track and unknown zone are 404s.
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"track_ids": [999]})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = call(&app, "GET", "/v1/zones/nope", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    // Transport on an empty-after-clear zone is accepted and reports state.
    let (st, _) = call(&app, "DELETE", &format!("/v1/zones/{zid}/queue"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, z) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/transport"),
        Some(serde_json::json!({"action": "play"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(z["transport"], "stopped");
}

/// A zone's queue is saved on every change and comes back after a restart.
#[tokio::test]
async fn queues_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("music")).unwrap();
    write_wav(&dir.path().join("music/one.wav"), 4410);
    let zid;
    {
        let s = state(dir.path(), None).await;
        let app = router(s.clone());
        call(&app, "POST", "/v1/library/scan", Some(serde_json::json!({}))).await;
        for _ in 0..500 {
            if !s.is_scanning() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let (_, tracks) = call(&app, "GET", "/v1/library/tracks?limit=10", None).await;
        let id = tracks["tracks"][0]["id"].as_i64().unwrap();
        let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
        zid = zones[0]["id"].as_str().unwrap().to_string();
        let (st, _) = call(
            &app,
            "POST",
            &format!("/v1/zones/{zid}/queue"),
            Some(serde_json::json!({"track_ids": [id], "items": [{"uri": "https://radio.example/live", "title": "Live"}], "play": false})),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let (st, _) = call(
            &app,
            "PUT",
            &format!("/v1/zones/{zid}/repeat"),
            Some(serde_json::json!({"repeat": "all"})),
        )
        .await;
        assert_eq!(st, StatusCode::NO_CONTENT);
        // The save happens on the state watch; give it a moment.
        let key = format!("queue.{zid}");
        let mut saved = None;
        for _ in 0..200 {
            saved = s.library.lock().unwrap().get_setting(&key).unwrap();
            if saved
                .as_deref()
                .is_some_and(|j| j.contains("radio.example") && j.contains("\"all\""))
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(saved.is_some(), "queue was not saved");
    }
    // "Restart": a new node state over the same database and outputs.
    let s2 = state(dir.path(), None).await;
    let app2 = router(s2.clone());
    let (st, q) = call(&app2, "GET", &format!("/v1/zones/{zid}/queue"), None).await;
    assert_eq!(st, StatusCode::OK, "{q}");
    let q = q.as_array().unwrap();
    assert_eq!(q.len(), 2, "{q:?}");
    assert_eq!(q[0]["title"], "One");
    assert_eq!(q[1]["uri"], "https://radio.example/live");
    let (_, z) = call(&app2, "GET", &format!("/v1/zones/{zid}"), None).await;
    assert_eq!(z["repeat"], "all");
    assert_eq!(z["transport"], "stopped", "restored queues do not start playing");
}

/// A zone says its output's level (`volume`): null for an output without one, the level
/// read back after every change (the dB cap applied), and mute (TASK-volume-one-source).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zones_report_their_volume_and_read_it_back_after_a_change() {
    use pm_zones::outputs::NullOutput;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let db_out: Arc<dyn pm_zones::Output> = Arc::new(NullOutput::with_volume(pm_model::VolumeKind::Db {
        min: -60.0,
        max: 0.0,
        step: 1.0,
    }));
    // The poll runs fast here (read when a zone's poll starts), and only while a controller
    // watches: one does for this test
    crate::state::VOLUME_POLL_MS_FOR_TESTS.store(200, std::sync::atomic::Ordering::Relaxed);
    let watching = s.events.watch();
    let pc_out_raw = Arc::new(NullOutput::with_volume(pm_model::VolumeKind::Percent));
    let pc_out: Arc<dyn pm_zones::Output> = pc_out_raw.clone();
    s.outputs.register(db_out.clone());
    s.outputs.register(pc_out.clone());
    let db_zone = s.create_zone("Null dB", db_out);
    let pc_zone = s.create_zone("Null percent", pc_out);
    let app = router(s.clone());
    // The poll's first read has happened once the zones settle
    db_zone.refresh_volume().await;
    pc_zone.refresh_volume().await;
    db_zone.wait_acked().await;
    pc_zone.wait_acked().await;
    let (st, zones) = call(&app, "GET", "/v1/zones", None).await;
    assert_eq!(st, StatusCode::OK, "{zones}");
    let by_out = |id: &str| {
        zones
            .as_array()
            .unwrap()
            .iter()
            .find(|z| z["output"] == id)
            .cloned()
            .unwrap()
    };
    assert!(
        by_out("null")["volume"].is_null(),
        "no hardware volume: {}",
        by_out("null")
    );
    assert_eq!(
        by_out("null:db")["volume"],
        serde_json::json!({"db": 0.0, "muted": false})
    );
    assert_eq!(
        by_out("null:percent")["volume"],
        serde_json::json!({"percent": 100, "muted": false})
    );
    // A change is read back
    let dz = db_zone.id().0.clone();
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{dz}/volume"),
        Some(serde_json::json!({"volume": {"db": -20.0}})),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    db_zone.wait_acked().await;
    let (_, z) = call(&app, "GET", &format!("/v1/zones/{dz}"), None).await;
    assert_eq!(
        z["volume"],
        serde_json::json!({"db": -20.0, "muted": false}),
        "{z}"
    );
    // The zone's volume limit caps it
    let (st, _) = call(
        &app,
        "PUT",
        &format!("/v1/zones/{dz}/dsp"),
        Some(serde_json::json!({"max_volume_db": -30.0})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{dz}/volume"),
        Some(serde_json::json!({"volume": {"db": -10.0}})),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    db_zone.wait_acked().await;
    let (_, z) = call(&app, "GET", &format!("/v1/zones/{dz}"), None).await;
    assert_eq!(z["volume"]["db"], -30.0, "capped: {z}");
    // Asked again, the level stays at the cap: controllers still hear the read-back (a page
    // showing the -10 it asked for learns the device kept -30)
    let mut w = db_zone.watch();
    w.borrow_and_update();
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{dz}/volume"),
        Some(serde_json::json!({"volume": {"db": -10.0}})),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let heard = tokio::time::timeout(std::time::Duration::from_secs(2), w.changed()).await;
    assert!(
        matches!(heard, Ok(Ok(()))),
        "no zone update after an unchanged read-back"
    );
    assert_eq!(
        w.borrow().volume.map(|v| v.level),
        Some(pm_model::VolumeLevel::Db(-30.0))
    );
    // The device's own level changes behind the node's back (its remote): the poll shows it
    pc_out_raw.pretend_level(pm_model::ZoneVolume {
        level: pm_model::VolumeLevel::Percent(33),
        muted: false,
    });
    let seen = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while pc_zone.state().volume.map(|v| v.level) != Some(pm_model::VolumeLevel::Percent(33)) {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        seen.is_ok(),
        "the poll never showed the device's own change: {:?}",
        pc_zone.state().volume
    );
    // Nobody watching: the poll stops reading (the device changes again, unseen)
    drop(watching);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    pc_out_raw.pretend_level(pm_model::ZoneVolume {
        level: pm_model::VolumeLevel::Percent(21),
        muted: false,
    });
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    assert_eq!(
        pc_zone.state().volume.map(|v| v.level),
        Some(pm_model::VolumeLevel::Percent(33)),
        "no controller connected: no reads"
    );
    let _watching = s.events.watch();
    pc_out_raw.pretend_level(pm_model::ZoneVolume {
        level: pm_model::VolumeLevel::Percent(33),
        muted: false,
    });
    // Percent and mute, on a percent output
    let pz = pc_zone.id().0.clone();
    for (body, want) in [
        (
            serde_json::json!({"volume": {"percent": 40}}),
            serde_json::json!({"percent": 40, "muted": false}),
        ),
        (
            serde_json::json!({"volume": {"mute": true}}),
            serde_json::json!({"percent": 40, "muted": true}),
        ),
        (
            serde_json::json!({"volume": "up"}),
            serde_json::json!({"percent": 43, "muted": true}),
        ),
    ] {
        let (st, _) = call(&app, "POST", &format!("/v1/zones/{pz}/volume"), Some(body)).await;
        assert_eq!(st, StatusCode::NO_CONTENT);
        pc_zone.wait_acked().await;
        let (_, z) = call(&app, "GET", &format!("/v1/zones/{pz}"), None).await;
        assert_eq!(z["volume"], want, "{z}");
    }
    // The limit caps a percent request too (-30 dB is 50 % on the outputs' scale)
    let (st, _) = call(
        &app,
        "PUT",
        &format!("/v1/zones/{pz}/dsp"),
        Some(serde_json::json!({"max_volume_db": -30.0})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{pz}/volume"),
        Some(serde_json::json!({"volume": {"percent": 100}})),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    pc_zone.wait_acked().await;
    let (_, z) = call(&app, "GET", &format!("/v1/zones/{pz}"), None).await;
    assert_eq!(z["volume"]["percent"], 50, "capped: {z}");
}

#[tokio::test]
async fn listeners_get_the_library_and_their_zones_not_the_owners_settings() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), Some("s3cret")).await;
    let app = router(s.clone());
    let key = "tbepfp8fisyiqpskbmxsmpub47gxb7xdsh5x9ut9qrb97gcq9phy";
    s.library.lock().unwrap().set_setting("people", &serde_json::json!({"people": [{"key": key, "access": "music", "music_enabled": true, "added_at": 1}]}).to_string()).unwrap();
    let tok = accounts::mint(&s, key).unwrap();
    let send = |method: &'static str, uri: String, token: String, body: Option<serde_json::Value>| {
        let app = app.clone();
        async move {
            let req = Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(
                    body.map(|b| Body::from(b.to_string()))
                        .unwrap_or_else(Body::empty),
                )
                .unwrap();
            let r = app.oneshot(req).await.unwrap();
            let st = r.status();
            let b = r.into_body().collect().await.unwrap().to_bytes();
            (
                st,
                serde_json::from_slice::<serde_json::Value>(&b).unwrap_or_default(),
            )
        }
    };
    let (_, me) = send("GET", "/v1/me".into(), tok.clone(), None).await;
    assert_eq!(
        (me["role"].as_str(), me["key"].as_str()),
        (Some("listener"), Some(key))
    );
    let (st, _) = send("GET", "/v1/library/tracks".into(), tok.clone(), None).await;
    assert_eq!(st, StatusCode::OK, "the shared library");
    for (m, p) in [
        ("GET", "/v1/people"),
        ("PUT", "/v1/node/settings"),
        ("POST", "/v1/library/folders"),
        ("POST", "/v1/zones"),
        // (a listener's own collections are theirs: tests/listener_collections.rs)
        ("POST", "/v1/playlists/from-queue"),
        ("GET", "/v1/sources/plex/server"),
    ] {
        let (st, v) = send(m, p.into(), tok.clone(), Some(serde_json::json!({}))).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{m} {p}: {v}");
        assert!(v.to_string().contains("Not for listeners"), "{v}");
    }
    // Playing on a zone: yes, until the owner limits listeners to another zone
    let (_, zones) = send("GET", "/v1/zones".into(), "s3cret".into(), None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let (st, v) = send(
        "POST",
        format!("/v1/zones/{zid}/transport"),
        tok.clone(),
        Some(serde_json::json!({"action": "stop"})),
    )
    .await;
    assert_ne!(st, StatusCode::FORBIDDEN, "{v}");
    let (st, _) = send(
        "PUT",
        "/v1/people/zones".into(),
        "s3cret".into(),
        Some(serde_json::json!({"zones": ["elsewhere"]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = send(
        "POST",
        format!("/v1/zones/{zid}/transport"),
        tok.clone(),
        Some(serde_json::json!({"action": "stop"})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "a zone the owner kept back");
    // A list the node can't read keeps listeners off every zone (never opens all of them)
    // until the owner sets it again; the owner is told, and a list that isn't one is refused
    s.library
        .lock()
        .unwrap()
        .set_setting("accounts.listener_zones", "not a list")
        .unwrap();
    let (st, v) = send(
        "POST",
        format!("/v1/zones/{zid}/transport"),
        tok.clone(),
        Some(serde_json::json!({"action": "stop"})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    assert!(v.to_string().contains("zone list"), "{v}");
    let (_, me) = send("GET", "/v1/me".into(), tok.clone(), None).await;
    assert_eq!(me["zones"], serde_json::json!([]), "{me}");
    let (st, v) = send("GET", "/v1/people/zones".into(), "s3cret".into(), None).await;
    assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    let (st, _) = send(
        "PUT",
        "/v1/people/zones".into(),
        "s3cret".into(),
        Some(serde_json::json!({"zones": "x"})),
    )
    .await;
    assert!(st.is_client_error(), "{st}");
    let (st, _) = send(
        "PUT",
        "/v1/people/zones".into(),
        "s3cret".into(),
        Some(serde_json::json!({"zones": [zid.clone()]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, v) = send(
        "POST",
        format!("/v1/zones/{zid}/transport"),
        tok.clone(),
        Some(serde_json::json!({"action": "stop"})),
    )
    .await;
    assert_ne!(st, StatusCode::FORBIDDEN, "{v}");
    // No zone at all: said as such, kept, and every zone is closed (the people page shows
    // "no zone", the listener's own page too); every zone again when the owner says so
    let (st, v) = send(
        "PUT",
        "/v1/people/zones".into(),
        "s3cret".into(),
        Some(serde_json::json!({"zones": [], "none": true})),
    )
    .await;
    assert_eq!(
        (st, &v),
        (StatusCode::OK, &serde_json::json!({"zones": [], "none": true}))
    );
    let (_, v) = send("GET", "/v1/people/zones".into(), "s3cret".into(), None).await;
    assert_eq!(v, serde_json::json!({"zones": [], "none": true}));
    let (st, v) = send(
        "POST",
        format!("/v1/zones/{zid}/transport"),
        tok.clone(),
        Some(serde_json::json!({"action": "stop"})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    let (_, me) = send("GET", "/v1/me".into(), tok.clone(), None).await;
    assert_eq!(me["zones"], serde_json::json!([]), "{me}");
    let (_, seen) = send("GET", "/v1/zones".into(), tok.clone(), None).await;
    assert_eq!(seen, serde_json::json!([]), "no zone is listed for them");
    // The library is theirs to browse and play in their own browser all the same
    let (st, _) = send("GET", "/v1/library/albums".into(), tok.clone(), None).await;
    assert_eq!(st, StatusCode::OK);
    // (stored as an older version reads it too: a list, of one zone no node has)
    let stored = s
        .library
        .lock()
        .unwrap()
        .get_setting("accounts.listener_zones")
        .unwrap();
    assert_eq!(stored.as_deref(), Some(r#"[""]"#));
    // Both at once is refused, and nothing changes
    let (st, _) = send(
        "PUT",
        "/v1/people/zones".into(),
        "s3cret".into(),
        Some(serde_json::json!({"zones": [zid.clone()], "none": true})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    // An empty id without `none` is refused too: dropped, it would leave every zone open
    for zones in [serde_json::json!([""]), serde_json::json!([" ", zid.clone()])] {
        let (st, v) = send(
            "PUT",
            "/v1/people/zones".into(),
            "s3cret".into(),
            Some(serde_json::json!({"zones": zones})),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    }
    let (_, v) = send("GET", "/v1/people/zones".into(), "s3cret".into(), None).await;
    assert_eq!(
        v,
        serde_json::json!({"zones": [], "none": true}),
        "nothing changed"
    );
    // (a page that also talks to nodes before this sends the empty id next to `none`)
    let (st, v) = send(
        "PUT",
        "/v1/people/zones".into(),
        "s3cret".into(),
        Some(serde_json::json!({"zones": [""], "none": true})),
    )
    .await;
    assert_eq!(
        (st, &v),
        (StatusCode::OK, &serde_json::json!({"zones": [], "none": true}))
    );
    // A zone with no name in the path is no zone of theirs either
    let (st, _) = send("GET", "/v1/zones//queue".into(), tok.clone(), None).await;
    assert_ne!(st, StatusCode::OK);
    let (st, v) = send(
        "PUT",
        "/v1/people/zones".into(),
        "s3cret".into(),
        Some(serde_json::json!({"zones": []})),
    )
    .await;
    assert_eq!(
        (st, &v),
        (StatusCode::OK, &serde_json::json!({"zones": [], "none": false}))
    );
    let (_, me) = send("GET", "/v1/me".into(), tok.clone(), None).await;
    assert_eq!(me["zones"], serde_json::Value::Null, "every zone: {me}");
    let (st, v) = send(
        "POST",
        format!("/v1/zones/{zid}/transport"),
        tok.clone(),
        Some(serde_json::json!({"action": "stop"})),
    )
    .await;
    assert_ne!(st, StatusCode::FORBIDDEN, "{v}");
    // Music access taken away: the token stops at once; a made-up token never works
    s.library.lock().unwrap().set_setting("people", &serde_json::json!({"people": [{"key": key, "access": "music", "music_enabled": false, "added_at": 1}]}).to_string()).unwrap();
    let (st, _) = send("GET", "/v1/library/tracks".into(), tok.clone(), None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _) = send("GET", "/v1/library/tracks".into(), "pml_madeup".into(), None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    // The owner's token is the owner
    let (_, me) = send("GET", "/v1/me".into(), "s3cret".into(), None).await;
    assert_eq!(me["role"], "owner");
    // Log out with the API token ends nothing (apps keep it; the page forgets it)
    let (st, v) = send("DELETE", "/v1/me/token".into(), "s3cret".into(), None).await;
    assert_eq!((st, v["ended"].as_bool()), (StatusCode::OK, Some(false)), "{v}");
    let (st, _) = send("GET", "/v1/me".into(), "s3cret".into(), None).await;
    assert_eq!(st, StatusCode::OK);
    // An owner's browser session (the Ring log in, Log out and the identity's own sessions
    // are in tests/bundled_homeserver.rs). While the node has no identity loaded, none is the
    // owner: an earlier owner's session can't come back when the node's own session isn't
    // restored at start (Kimi review of #209); the API token still works
    const OLD: &str = "o3ftkuxjd9k6zgnc4hhbhbg5jusubr4zqyb1dtzi8hhm3hf5wbmy";
    const NEW: &str = "gujx6qd8ksydh1makdphd3bxu351d9b8waqka8hfg6q7hnqkxexo";
    // No identity loaded, so no session is made for anyone
    assert_eq!(
        accounts::mint_owner(&s, OLD).unwrap_err().status,
        StatusCode::CONFLICT
    );
    // An earlier owner's session, kept from before: not the owner now. The answer is a 503 of
    // its own, so the page keeps the session instead of logging out for good
    let sessions = |keys: &[&str]| {
        s.library
            .lock()
            .unwrap()
            .set_setting(
                "accounts.owner_sessions",
                &serde_json::to_string(
                    &keys
                        .iter()
                        .map(|k| serde_json::json!({"hash": blake3::hash(format!("pmo_{k}").as_bytes()).to_hex().to_string(), "key": k, "created_at": 1}))
                        .collect::<Vec<_>>(),
                )
                .unwrap(),
            )
            .unwrap()
    };
    sessions(&[OLD, NEW]);
    let (st, v) = send("GET", "/v1/me".into(), format!("pmo_{OLD}"), None).await;
    assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE, "{v}");
    assert_eq!(v["error"], accounts::IDENTITY_NOT_LOADED);
    let (st, _) = send("GET", "/v1/me".into(), "s3cret".into(), None).await;
    assert_eq!(st, StatusCode::OK, "the API token logs in meanwhile");
    // Log out still works then, so the browser gets to Log in and the API token (Hermes
    // review of #209 at 6bf5f7d)
    sessions(&[OLD, NEW, "logsout"]);
    let (st, v) = send("DELETE", "/v1/me/token".into(), "pmo_logsout".into(), None).await;
    assert_eq!((st, v["ended"].as_bool()), (StatusCode::OK, Some(true)), "{v}");
    let (st, v) = send("DELETE", "/v1/me/token".into(), "pmo_logsout".into(), None).await;
    assert_eq!(
        (st, v["ended"].as_bool()),
        (StatusCode::OK, Some(false)),
        "already ended: {v}"
    );
    let (st, _) = send("POST", "/v1/node/token".into(), format!("pmo_{OLD}"), None).await;
    assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE, "only Log out goes through");
    // The node takes on another identity: the old owner's sessions are gone for good, the new
    // owner's stay
    accounts::keep_owner_sessions_of(&s, &format!("pubky{NEW}")).unwrap();
    let kept: Vec<String> = serde_json::from_str::<Vec<serde_json::Value>>(
        &s.library
            .lock()
            .unwrap()
            .get_setting("accounts.owner_sessions")
            .unwrap()
            .unwrap(),
    )
    .unwrap()
    .iter()
    .map(|t| t["key"].as_str().unwrap().to_string())
    .collect();
    assert_eq!(kept, [NEW]);
    // A wrong token is still the 401 the page logs in again on
    let (st, v) = send("GET", "/v1/me".into(), "pml_madeup".into(), None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert_eq!(
        v["error"], "missing or wrong bearer token",
        "the page logs in again on exactly this"
    );
}

#[tokio::test]
async fn a_queue_can_be_saved_as_a_playlist() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("untagged.wav");
    write_wav(&wav, 441);
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let archive = "https://archive.org/download/gd1977-05-08/gd77-05-08d1t01.flac";
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"items": [
            {"uri": archive, "title": "Scarlet Begonias"},
            {"uri": wav.to_string_lossy(), "title": "Untagged"},
        ], "replace": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, r) = call(
        &app,
        "POST",
        "/v1/playlists/from-queue",
        Some(serde_json::json!({"zone": zid})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert!(r["playlist"]["summary"]["name"]
        .as_str()
        .unwrap()
        .starts_with("Queue · "));
    assert_eq!(r["playlist"]["playlist"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(r["playlist"]["playlist"]["items"][0]["uri"], archive);
    assert_eq!(r["skipped"], serde_json::json!(["Untagged"]));
    // Explicit URIs (the browser's queue) and a name of one's own.
    let (st, r) = call(
        &app,
        "POST",
        "/v1/playlists/from-queue",
        Some(serde_json::json!({"uris": [archive, archive], "name": "Twice"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["playlist"]["summary"]["name"], "Twice");
    assert_eq!(r["playlist"]["playlist"]["items"].as_array().unwrap().len(), 2);
    // Nothing canonical: 422, no playlist.
    let (st, _) = call(
        &app,
        "POST",
        "/v1/playlists/from-queue",
        Some(serde_json::json!({"uris": [wav.to_string_lossy()]})),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    let (_, pls) = call(&app, "GET", "/v1/playlists", None).await;
    assert_eq!(pls.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn a_volume_limit_caps_the_dsp_volume_and_requests() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let (st, _) = call(
        &app,
        "PUT",
        &format!("/v1/zones/{zid}/dsp"),
        Some(serde_json::json!({"max_volume_db": 3.0})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, cfg) = call(
        &app,
        "PUT",
        &format!("/v1/zones/{zid}/dsp"),
        Some(serde_json::json!({"max_volume_db": -20.0, "volume_db": -6.0})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{cfg}");
    assert_eq!(cfg["volume_db"], -20.0, "software volume is capped at the limit");
    let (_, cfg) = call(&app, "GET", &format!("/v1/zones/{zid}/dsp"), None).await;
    assert_eq!(cfg["max_volume_db"], -20.0);
    // Requests are capped before they reach the output (pm_zones::cap_volume, applied in
    // the zone for every controller)
    use pm_model::Volume;
    use pm_zones::cap_volume;
    let linear = |db: f32| ((db.clamp(-60.0, 0.0) + 60.0) / 60.0 * 100.0).floor() as u8;
    assert_eq!(
        cap_volume(Volume::Db(-3.0), Some(-20.0), linear),
        Volume::Db(-20.0)
    );
    assert_eq!(
        cap_volume(Volume::Db(-30.0), Some(-20.0), linear),
        Volume::Db(-30.0)
    );
    assert_eq!(cap_volume(Volume::Db(-3.0), None, linear), Volume::Db(-3.0));
    assert_eq!(cap_volume(Volume::Up, Some(-20.0), linear), Volume::Up);
    // Percent is capped where the output itself puts the limit: -30 dB is 50 % on a linear
    // output (UPnP), 3 % on one that sets an amplitude (BluOS, MPD, Cast)
    assert_eq!(
        cap_volume(Volume::Percent(100), Some(-30.0), linear),
        Volume::Percent(50)
    );
    assert_eq!(
        cap_volume(Volume::Percent(100), Some(-30.0), pm_zones::amplitude_percent),
        Volume::Percent(3)
    );
    assert_eq!(
        cap_volume(Volume::Percent(2), Some(-30.0), pm_zones::amplitude_percent),
        Volume::Percent(2)
    );
    // Rounded down, never past the limit: -46 dB is 0.5 % as an amplitude (1 % would be -40 dB)
    assert_eq!(
        cap_volume(Volume::Percent(100), Some(-46.0), pm_zones::amplitude_percent),
        Volume::Percent(0)
    );
    assert_eq!(
        cap_volume(Volume::Percent(100), Some(-46.0), linear),
        Volume::Percent(23)
    );
    // An output's own mapping: the default (linear) on an output that doesn't override it
    let plain = pm_zones::outputs::NullOutput::with_volume(pm_model::VolumeKind::Percent);
    assert_eq!(
        cap_volume(Volume::Percent(100), Some(-46.0), |db| plain.percent_for_db(db)),
        Volume::Percent(23),
        "rounded down (23.3 %)"
    );
    // The null output has no volume control at all: the request is refused as before.
    let (st, body) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/volume"),
        Some(serde_json::json!({"volume": {"db": -3.0}})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn the_jukebox_zone_is_a_node_setting() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let (_, v) = call(&app, "GET", "/v1/node/settings", None).await;
    assert!(v.get("jukebox_zone").is_none());
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/node/settings",
        Some(serde_json::json!({"jukebox_zone": "nope"})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, v) = call(
        &app,
        "PUT",
        "/v1/node/settings",
        Some(serde_json::json!({"jukebox_zone": zid})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["jukebox_zone"], zid);
    let (_, v) = call(
        &app,
        "PUT",
        "/v1/node/settings",
        Some(serde_json::json!({"jukebox_zone": ""})),
    )
    .await;
    assert!(v.get("jukebox_zone").is_none());
}

#[tokio::test]
async fn a_sleep_timer_pauses_the_zone() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("long.wav");
    write_wav(&wav, 441);
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    call(
        &app,
        "PUT",
        &format!("/v1/zones/{zid}/repeat"),
        Some(serde_json::json!({"repeat": "one"})),
    )
    .await;
    call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"items": [{"uri": wav.to_string_lossy()}], "replace": true, "play": true})),
    )
    .await;
    let (_, v) = call(&app, "GET", &format!("/v1/zones/{zid}/sleep"), None).await;
    assert!(v.get("ends_at_ms").is_none());
    let (st, _) = call(
        &app,
        "PUT",
        &format!("/v1/zones/{zid}/sleep"),
        Some(serde_json::json!({"seconds": 0})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, v) = call(
        &app,
        "PUT",
        &format!("/v1/zones/{zid}/sleep"),
        Some(serde_json::json!({"seconds": 1})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(v["remaining_ms"].as_u64().unwrap() <= 1000);
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    let mut paused = false;
    for _ in 0..400 {
        if zone.state().transport == pm_zones::Transport::Paused {
            paused = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(paused, "the zone did not pause: {:?}", zone.state().transport);
    let (_, v) = call(&app, "GET", &format!("/v1/zones/{zid}/sleep"), None).await;
    assert!(v.get("ends_at_ms").is_none(), "timer cleared after firing");
    // Cancel: set and clear, nothing pauses later.
    let (st, _) = call(
        &app,
        "PUT",
        &format!("/v1/zones/{zid}/sleep"),
        Some(serde_json::json!({"seconds": 3600})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(s.sleep_ends_at(zone.id()).is_some());
    let (_, v) = call(
        &app,
        "PUT",
        &format!("/v1/zones/{zid}/sleep"),
        Some(serde_json::json!({"seconds": null})),
    )
    .await;
    assert!(v.get("ends_at_ms").is_none());
    assert!(s.sleep_ends_at(zone.id()).is_none());
}

/// A queue takes at most `MAX_QUEUE_ITEMS`: a request for more is a 400 before any song is
/// looked up (the ids here are no songs: without the limit the answer is a 404), and a
/// sleep timer of any length is set without its end time running over.
#[tokio::test]
async fn a_queue_and_a_sleep_timer_have_limits() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let many: Vec<u64> = (0..=pm_zones::MAX_QUEUE_ITEMS as u64)
        .map(|i| 9_000_000 + i)
        .collect();
    let (st, e) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"track_ids": many, "replace": true})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{e}");
    assert!(
        e["error"].as_str().unwrap().contains("at most 100000 items"),
        "{e}"
    );
    let (st, e) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"track_ids": [9_000_000]})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND, "{e}");
    // What is in the queue counts unless it is replaced
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    zone.enqueue(
        (0..5)
            .map(|i| pm_zones::QueueItem::new(format!("https://radio.example/{i}")))
            .collect(),
        pm_zones::Insert::End,
    );
    let fill: Vec<u64> = (0..pm_zones::MAX_QUEUE_ITEMS as u64 - 4)
        .map(|i| 9_000_000 + i)
        .collect();
    let (st, e) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"track_ids": fill})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{e}");
    assert!(e["error"].as_str().unwrap().contains("this one has 5"), "{e}");

    let (st, v) = call(
        &app,
        "PUT",
        &format!("/v1/zones/{zid}/sleep"),
        Some(serde_json::json!({"seconds": u64::MAX})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["ends_at_ms"], u64::MAX);
    let (st, v) = call(
        &app,
        "PUT",
        &format!("/v1/zones/{zid}/sleep"),
        Some(serde_json::json!({"seconds": null})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(v.get("ends_at_ms").is_none(), "{v}");
}

/// A list that would pass what a queue holds is refused, 400, before the zone is touched, on
/// every route that puts it in place of the queue: a move from This browser (`load`, whose
/// browser empties its own queue after a move) and a collection played into the zone. Without
/// the check the zone stopped, emptied, kept 100,000 of them and answered success.
#[tokio::test]
async fn a_list_past_the_queue_limit_leaves_the_zone_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    let had = zone.enqueue(
        (0..5)
            .map(|i| pm_zones::QueueItem::new(format!("https://radio.example/{i}")))
            .collect(),
        pm_zones::Insert::End,
    );
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), zone.wait_acked()).await;
    let ids = |z: &Zone| z.queue().iter().map(|it| it.id).collect::<Vec<_>>();
    assert_eq!(ids(&zone), had);
    let over = pm_zones::MAX_QUEUE_ITEMS + 1;

    // A move of 100,001 items (short ones: the request stays under the body limit, 2 MiB)
    let items: Vec<serde_json::Value> = (0..over)
        .map(|i| serde_json::json!({"uri": format!("x{i}")}))
        .collect();
    assert!(serde_json::to_vec(&items).unwrap().len() < 2 << 20);
    let (st, e) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/load"),
        Some(serde_json::json!({"items": items, "index": 0, "transport": "playing"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{e}");
    assert!(
        e["error"].as_str().unwrap().contains("at most 100000 items"),
        "{e}"
    );
    assert_eq!(ids(&zone), had, "the zone's queue is as it was");

    // A collection of 100,001 library songs (one album), played in place of the queue
    let release = "c1a9c0e9-d987-4042-ae91-78d6a3267d69";
    {
        let lib = s.library.lock().unwrap();
        lib.begin_batch().unwrap();
        for i in 0..over {
            let t = pm_model::Track {
                id: None,
                title: format!("Part {i}"),
                artists: vec![pm_model::Artist {
                    id: None,
                    name: "Reader".into(),
                    mbid: None,
                }],
                album: Some(pm_model::Album {
                    id: None,
                    title: "Long".into(),
                    album_artist: Some("Reader".into()),
                    release_mbid: Some(pm_model::Mbid::parse(release).unwrap()),
                    release_group_mbid: None,
                    year: None,
                    upc: None,
                    total_discs: None,
                }),
                track_no: Some(i as u32 + 1),
                disc_no: None,
                duration_ms: Some(1_000),
                file: pm_model::AudioFile {
                    path: dir.path().join(format!("long/{i}.flac")),
                    size_bytes: 1,
                    mtime_unix: 0,
                    container: "flac".into(),
                    format: pm_model::AudioFormat::pcm(44_100, 16, 2),
                    duration_ms: Some(1_000),
                },
                recording_mbid: None,
                isrc: None,
                acoustid: None,
                canonical: None,
                replaygain_track_gain_db: None,
                replaygain_album_gain_db: None,
                genres: vec![],
            };
            lib.upsert_track(&t).unwrap();
        }
        lib.commit_batch().unwrap();
    }
    let (st, pl) = call(
        &app,
        "POST",
        "/v1/playlists",
        Some(serde_json::json!({"name": "Long",
            "uris": [format!("https://musicbrainz.org/release/{release}")]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{pl}");
    let id = pl["summary"]["id"].as_str().unwrap().to_string();
    for replace in [serde_json::json!(true), serde_json::Value::Null] {
        let (st, e) = call(
            &app,
            "POST",
            &format!("/v1/playlists/{id}/play"),
            Some(serde_json::json!({"zone": zid, "replace": replace})),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{e}");
        assert!(
            e["error"].as_str().unwrap().contains("at most 100000 items"),
            "{e}"
        );
        assert_eq!(ids(&zone), had, "the zone's queue is as it was");
    }
    // After the queue too: the five there count
    let (st, e) = call(
        &app,
        "POST",
        &format!("/v1/playlists/{id}/play"),
        Some(serde_json::json!({"zone": zid, "replace": false})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{e}");
    assert!(e["error"].as_str().unwrap().contains("this one has 5"), "{e}");
    assert_eq!(ids(&zone), had);
}

#[tokio::test]
async fn playback_transfers_between_zones() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("long.wav");
    write_wav(&wav, 44_100 * 30);
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let out = Arc::new(pm_zones::outputs::WavFileOutput::new(dir.path().join("sink.wav")));
    s.outputs.register(out.clone());
    let target = s.create_zone("Target", out);
    let tid = target.id().to_string();
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones
        .as_array()
        .unwrap()
        .iter()
        .map(|z| z["id"].as_str().unwrap().to_string())
        .find(|z| *z != tid)
        .unwrap();
    let items: Vec<_> = (0..3)
        .map(|i| serde_json::json!({"uri": wav.to_string_lossy(), "title": format!("t{i}")}))
        .collect();
    call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"items": items, "replace": true})),
    )
    .await;
    call(
        &app,
        "PUT",
        &format!("/v1/zones/{zid}/repeat"),
        Some(serde_json::json!({"repeat": "all"})),
    )
    .await;
    call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/transport"),
        Some(serde_json::json!({"action": "play", "index": 1})),
    )
    .await;
    let source = s.zones.get(&ZoneId::new(&zid)).unwrap();
    for _ in 0..500 {
        if source.state().now_playing.as_ref().map(|n| n.index) == Some(1) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    // The null output does not pace, so pause right away to hold a position.
    source.pause();
    for _ in 0..500 {
        if source.state().transport == pm_zones::Transport::Paused {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/transfer"),
        Some(serde_json::json!({"to": zid})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, z) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/transfer"),
        Some(serde_json::json!({"to": tid})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{z}");
    assert_eq!(z["id"], tid);
    assert_eq!(z["queue"].as_array().unwrap().len(), 3);
    assert_eq!(z["queue"][1]["title"], "t1");
    assert_eq!(z["repeat"], "all");
    assert_eq!(z["now_playing"]["index"], 1);
    assert_eq!(z["transport"], "paused");
    let src = source.state();
    assert!(src.queue.is_empty());
    assert_eq!(src.transport, pm_zones::Transport::Stopped);
    // Nothing left to move.
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/transfer"),
        Some(serde_json::json!({"to": tid})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn shuffle_reorders_only_the_upcoming_items() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("short.wav");
    write_wav(&wav, 441);
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let items: Vec<_> = (0..8)
        .map(|_| serde_json::json!({"uri": wav.to_string_lossy()}))
        .collect();
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"items": items, "replace": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    // Loop the third item so the "current" index holds still on the null output.
    call(
        &app,
        "PUT",
        &format!("/v1/zones/{zid}/repeat"),
        Some(serde_json::json!({"repeat": "one"})),
    )
    .await;
    call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/transport"),
        Some(serde_json::json!({"action": "play", "index": 2})),
    )
    .await;
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    for _ in 0..500 {
        if zone.state().queue_index == Some(2) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(zone.state().queue_index, Some(2));
    // The playing item and the two before it must stay put.
    let before: Vec<u64> = zone.queue().iter().map(|i| i.id).collect();
    let mut changed = false;
    for _ in 0..5 {
        let (st, _) = call(&app, "POST", &format!("/v1/zones/{zid}/shuffle"), None).await;
        assert_eq!(st, StatusCode::NO_CONTENT);
        let after: Vec<u64> = zone.queue().iter().map(|i| i.id).collect();
        assert_eq!(&after[..3], &before[..3]);
        let mut rest = after[3..].to_vec();
        rest.sort_unstable();
        assert_eq!(rest, before[3..]);
        if after[3..] != before[3..] {
            changed = true;
            break;
        }
    }
    assert!(changed, "five shuffles of five items never moved anything");
    let (_, z) = call(&app, "GET", &format!("/v1/zones/{zid}"), None).await;
    assert_eq!(z["queue_index"], 2);
    let (st, _) = call(&app, "POST", "/v1/zones/nope/shuffle", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn library_radio_continues_after_the_last_track() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("music")).unwrap();
    write_wav(&dir.path().join("music/one.wav"), 4410);
    write_wav(&dir.path().join("music/two.wav"), 4410);
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    call(&app, "POST", "/v1/library/scan", Some(serde_json::json!({}))).await;
    for _ in 0..500 {
        if !s.is_scanning() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let (_, tracks) = call(&app, "GET", "/v1/library/tracks?limit=10", None).await;
    let first = tracks["tracks"][0]["id"].as_i64().unwrap();
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let (st, _) = call(
        &app,
        "PUT",
        &format!("/v1/zones/{zid}/radio"),
        Some(serde_json::json!({"on": true})),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (_, z) = call(&app, "GET", &format!("/v1/zones/{zid}"), None).await;
    assert_eq!(z["radio"], true);
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"track_ids": [first], "replace": true, "play": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    let mut grew = false;
    for _ in 0..500 {
        if zone.queue().len() >= 2 {
            grew = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(grew, "radio did not add tracks");
    assert!(zone.queue().len() <= 2, "only the one other track exists");
    zone.set_radio(false);
    assert_eq!(zone.wait_stopped().await.error, None);
}

#[tokio::test]
async fn zones_can_be_created_and_removed() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let mut events = s.events.subscribe();
    let (st, z) = call(
        &app,
        "POST",
        "/v1/zones",
        Some(serde_json::json!({"name": "Kitchen", "output": "null"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{z}");
    assert_eq!(z["id"], "kitchen");
    let (st, _) = call(
        &app,
        "POST",
        "/v1/zones",
        Some(serde_json::json!({"name": "X", "output": "nope"})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = call(&app, "DELETE", "/v1/zones/kitchen", None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let mut removed = false;
    while let Ok(ev) = events.try_recv() {
        if matches!(ev.event, ApiEvent::ZoneRemoved { ref zone } if zone.0 == "kitchen") {
            removed = true;
        }
    }
    assert!(removed);
    assert_eq!(s.zones.list().len(), 1);
}

/// Announcements: a chime and a clip play instead of the music, then the zone goes back to the
/// same track at the same position, playing or paused as it was, with the queue as it was.
#[tokio::test]
async fn an_announcement_interrupts_and_the_zone_comes_back() {
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
        get({
            let f = flac.clone();
            move || {
                let f = f.clone();
                async move { f }
            }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    // A zone on a real-time null output, so positions mean something
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("lib.sqlite");
    let lib = Library::open(&db).unwrap();
    let outputs = OutputRegistry::new();
    outputs.register(Arc::new(NullOutput::realtime()));
    let s = NodeState::new(
        NodeSettings {
            name: "test".into(),
            version: "0".into(),
            db_path: db,
            scan_folders: vec![dir.path().join("music")],
            art_dir: dir.path().join("art"),
            mirror_dir: dir.path().join("homeserver"),
            pipeline: PipelineConfig::default(),
            token: None,
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
        lib,
        outputs,
    );
    *s.deezer_base.write().unwrap() = "http://127.0.0.1:9".into();
    s.zones_for_all_outputs();
    let app = router(s.clone());
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let zone = |app: &Router| {
        let app = app.clone();
        let zid = zid.clone();
        async move { call(&app, "GET", &format!("/v1/zones/{zid}"), None).await.1 }
    };
    // Two songs, the first playing for a while
    let (st, _) = call(&app, "POST", &format!("/v1/zones/{zid}/queue"), Some(serde_json::json!({"items": [
        {"uri": format!("{base}/song.flac"), "title": "A", "artist": "Band"},
        {"uri": format!("{base}/song.flac"), "title": "B", "artist": "Band"}], "replace": true, "play": true}))).await;
    assert_eq!(st, StatusCode::OK);
    let mut z = serde_json::Value::Null;
    for _ in 0..60 {
        z = zone(&app).await;
        if z["transport"] == "playing" && z["now_playing"]["position_ms"].as_u64().unwrap_or(0) >= 1200 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let saved = z["now_playing"]["position_ms"].as_u64().unwrap();
    assert!(saved >= 1200, "{z}");
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/announce"),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "needs a uri or a chime");
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/announce"),
        Some(serde_json::json!({"uri": "ftp://x"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    // The announcement: a chime, then the clip
    let (st, a) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/announce"),
        Some(serde_json::json!({"uri": format!("{base}/song.flac"), "chime": true, "volume_percent": 60})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{a}");
    assert_eq!(
        (
            a["resumes"].clone(),
            a["items"].clone(),
            a["volume_applied"].clone()
        ),
        (
            serde_json::json!(true),
            serde_json::json!(2),
            serde_json::json!(false)
        ),
        "no hardware volume on a null output: {a}"
    );
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/announce"),
        Some(serde_json::json!({"chime": true})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "one at a time");
    let mut seen_chime = false;
    let mut seen_clip = false;
    for _ in 0..80 {
        z = zone(&app).await;
        let t = z["now_playing"]["item"]["title"].as_str().unwrap_or("");
        seen_chime |= t == "Chime";
        seen_clip |= t == "Announcement";
        if seen_clip && t != "Announcement" && t != "Chime" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        seen_chime && seen_clip,
        "chime {seen_chime}, clip {seen_clip}: {z}"
    );
    // Back on A, near where it was, playing; the queue as before
    let mut back = serde_json::Value::Null;
    for _ in 0..40 {
        back = zone(&app).await;
        if back["now_playing"]["item"]["title"] == "A"
            && back["transport"] == "playing"
            && back["now_playing"]["position_ms"].as_u64().unwrap_or(0) >= saved
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(back["now_playing"]["item"]["title"], "A", "{back}");
    assert_eq!(back["transport"], "playing");
    let pos = back["now_playing"]["position_ms"].as_u64().unwrap();
    assert!(
        pos >= saved && pos < saved + 2500,
        "position {pos} vs saved {saved}"
    );
    assert_eq!(
        back["queue"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["title"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["A", "B"]
    );
    // Paused before: paused again after
    call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/transport"),
        Some(serde_json::json!({"action": "pause"})),
    )
    .await;
    let (st, a) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/announce"),
        Some(serde_json::json!({"uri": format!("{base}/song.flac"), "title": "Doorbell"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{a}");
    let mut seen = false;
    for _ in 0..80 {
        z = zone(&app).await;
        let t = z["now_playing"]["item"]["title"].as_str().unwrap_or("");
        seen |= t == "Doorbell";
        if seen && t != "Doorbell" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(seen, "{z}");
    for _ in 0..30 {
        back = zone(&app).await;
        if back["transport"] == "paused" && back["now_playing"]["item"]["title"] == "A" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(
        (
            back["transport"].clone(),
            back["now_playing"]["item"]["title"].clone()
        ),
        (serde_json::json!("paused"), serde_json::json!("A")),
        "{back}"
    );
    assert_eq!(back["queue"].as_array().unwrap().len(), 2);
    // The chime file is a WAV in the data dir
    assert!(s.data_dir().join("announce/chime.wav").is_file());
}

#[tokio::test]
async fn load_fills_a_zone_at_a_position() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("long.wav");
    write_wav(&wav, 44_100 * 30);
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let items: Vec<_> = (0..3)
        .map(|i| serde_json::json!({"uri": wav.to_string_lossy(), "title": format!("t{i}")}))
        .collect();
    // Nothing to move
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/load"),
        Some(serde_json::json!({"items": []})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/load"),
        Some(serde_json::json!({"items": [{"uri": " "}]})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    // A paused browser queue: the second song at 5 s, repeat all
    let (st, z) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/load"),
        Some(serde_json::json!({"items": items, "index": 1, "position_ms": 5000, "transport": "paused", "repeat": "all"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{z}");
    assert_eq!(z["queue"].as_array().unwrap().len(), 3);
    assert_eq!(z["queue"][1]["title"], "t1");
    assert_eq!(z["repeat"], "all");
    assert_eq!(z["now_playing"]["index"], 1);
    assert_eq!(z["transport"], "paused");
    assert!(z["now_playing"]["position_ms"].as_u64().unwrap() >= 4_000, "{z}");
    // Stopped: loaded, nothing plays
    let one = vec![serde_json::json!({"uri": wav.to_string_lossy(), "title": "only"})];
    let (st, z) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/load"),
        Some(serde_json::json!({"items": one})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{z}");
    assert_eq!(z["queue"].as_array().unwrap().len(), 1);
    assert_eq!(z["transport"], "stopped");
    assert_eq!(z["repeat"], "off");
}

/// Moving a queue into a listen-along group (or any zone, `load_zone`) keeps who queued each
/// item: the host's songs play with the host's account, a joiner's with theirs.
#[tokio::test]
async fn a_moved_queue_keeps_who_queued_each_item() {
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let (base, seen) = tidal_mock_recording_tokens(flac).await;
    let dir = tempfile::tempdir().unwrap();
    let (s, anna, _) = two_people_node(dir.path(), &base).await;
    let zid = s.zones.list()[0].id.clone();
    let zone = s.zones.get(&zid).unwrap();
    let mut theirs = QueueItem::new("tidal:track:2");
    theirs.by = Some(anna.into());
    theirs.limit = Some(pm_zones::FetchLimit::PUBLIC);
    let queue = vec![QueueItem::new("tidal:track:1"), theirs];
    load_zone(
        &zone,
        queue,
        Some(0),
        0,
        pm_zones::Transport::Playing,
        pm_zones::Repeat::Off,
        LoadShuffle::Off,
        false,
    )
    .await;
    assert_eq!(zone.queue()[1].by.as_deref(), Some(anna));
    let st = zone.wait_stopped().await;
    assert!(st.error.is_none(), "{:?}", st.error);
    assert_eq!(*seen.lock().unwrap(), ["Bearer owner-token", "Bearer anna-token"]);
}

/// A listener whose music access is switched off (People) plays nothing with their sign-ins,
/// items they queued before included (L2); the owner taking a queue over from a browser takes
/// it whole, with no listener's fetch limit left on it (L4).
#[tokio::test]
async fn a_blocked_listeners_items_play_with_no_account_and_the_owner_takes_queues_whole() {
    let flac = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pm-audio/tests/fixtures/60_mono_audio.flac"
    ))
    .unwrap();
    let (base, seen) = tidal_mock_recording_tokens(flac).await;
    let dir = tempfile::tempdir().unwrap();
    let (s, anna, _) = two_people_node(dir.path(), &base).await;
    let app = router(s.clone());
    let anna_tok = accounts::mint(&s, anna).unwrap();
    let zid = s.zones.list()[0].id.clone();
    let zone = s.zones.get(&zid).unwrap();
    let item =
        serde_json::json!({"items": [{"uri": "tidal:track:20115564"}], "replace": true, "play": false});
    let (st, _, _) = send_as(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(&anna_tok),
        &[],
        Some(item),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let off = serde_json::json!({"music": false});
    let (st, v, _) = send_as(
        &app,
        "PATCH",
        &format!("/v1/people/{anna}"),
        Some("s3cret"),
        &[],
        Some(off),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    zone.play();
    let st = zone.wait_stopped().await;
    assert!(st.error.is_some(), "Anna's item no longer plays with her account");
    assert!(seen.lock().unwrap().is_empty(), "{:?}", seen.lock().unwrap());

    // The owner moves a browser queue holding a listener's item: theirs now, whole
    let moved = serde_json::json!({"items": [{"uri": "tidal:track:20115564", "by": anna, "limit": {"lan": false}}], "index": 0, "transport": "playing"});
    let (st, v, _) = send_as(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/load"),
        Some("s3cret"),
        &[],
        Some(moved),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let q = zone.queue();
    assert_eq!((q[0].by.as_deref(), q[0].limit), (None, None));
    let st = zone.wait_stopped().await;
    assert!(st.error.is_none(), "{:?}", st.error);
    assert_eq!(*seen.lock().unwrap(), ["Bearer owner-token"]);
}

/// Shuffle is a mode: on shuffles what comes after the current item and keeps the order;
/// off puts every item back, the current one in its place and items added meanwhile in theirs.
/// The state says so (on `/v1/events` too), the older POST turns it on, a transfer carries it.
#[tokio::test]
async fn shuffle_is_a_mode_that_restores_the_order() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("short.wav");
    write_wav(&wav, 441);
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let items: Vec<_> = (0..8)
        .map(|i| serde_json::json!({"uri": wav.to_string_lossy(), "title": format!("t{i}")}))
        .collect();
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"items": items, "replace": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    // Loop the third item so the current index holds still on the null output
    call(
        &app,
        "PUT",
        &format!("/v1/zones/{zid}/repeat"),
        Some(serde_json::json!({"repeat": "one"})),
    )
    .await;
    call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/transport"),
        Some(serde_json::json!({"action": "play", "index": 2})),
    )
    .await;
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    for _ in 0..500 {
        if zone.state().queue_index == Some(2) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(zone.state().queue_index, Some(2));
    let titles = |z: &serde_json::Value| -> Vec<String> {
        z["queue"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["title"].as_str().unwrap_or_default().to_string())
            .collect()
    };
    let original: Vec<String> = (0..8).map(|i| format!("t{i}")).collect();
    let mut events = s.events.subscribe();
    let put = |on: bool| {
        let app = app.clone();
        let zid = zid.clone();
        async move {
            call(
                &app,
                "PUT",
                &format!("/v1/zones/{zid}/shuffle"),
                Some(serde_json::json!({"shuffle": on})),
            )
            .await
        }
    };
    // On: the playing item and the ones before it stay, the rest is shuffled (tried again
    // when a shuffle happened to leave the order as it was)
    let mut on = serde_json::Value::Null;
    for _ in 0..5 {
        let (st, z) = put(true).await;
        assert_eq!(st, StatusCode::OK, "{z}");
        assert_eq!(
            (z["shuffle"].as_bool(), z["queue_index"].as_u64()),
            (Some(true), Some(2)),
            "{z}"
        );
        on = z;
        if titles(&on)[3..] != original[3..] {
            break;
        }
        put(false).await;
    }
    let t = titles(&on);
    assert_eq!(&t[..3], &original[..3], "played and current stay");
    let (mut rest, mut want) = (t[3..].to_vec(), original[3..].to_vec());
    rest.sort();
    want.sort();
    assert_eq!(rest, want, "the same items");
    assert_ne!(&t[3..], &original[3..], "five shuffles never moved anything");
    // The zone state on /v1/events says so
    let mut seen = false;
    while let Ok(Ok(ev)) = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv()).await {
        if let ApiEvent::Zone { state } = ev.event {
            if state.id == ZoneId::new(&zid) && state.shuffle {
                assert_eq!(serde_json::to_value(&state).unwrap()["shuffle"], true);
                seen = true;
                break;
            }
        }
    }
    assert!(seen, "no zone state with shuffle on arrived");
    // Added while on: "Play next" is next, "Add to queue" goes to the end
    let titled = |t: &str| {
        let mut it = QueueItem::new(wav.to_string_lossy());
        it.title = Some(t.into());
        it
    };
    zone.enqueue(vec![titled("next")], Insert::Next);
    zone.enqueue(vec![titled("end")], Insert::End);
    settle(zone.as_ref()).await;
    let (_, z) = call(&app, "GET", &format!("/v1/zones/{zid}"), None).await;
    assert_eq!(titles(&z)[3], "next");
    assert_eq!(titles(&z).last().map(String::as_str), Some("end"));
    // Off: the original order, the current item in its place, the added items in theirs
    let (st, z) = put(false).await;
    assert_eq!(st, StatusCode::OK, "{z}");
    assert_eq!(
        (z["shuffle"].as_bool(), z["queue_index"].as_u64()),
        (Some(false), Some(2)),
        "{z}"
    );
    let mut want = original.clone();
    want.insert(3, "next".into());
    want.push("end".into());
    assert_eq!(titles(&z), want);
    // The older POST turns it on
    let (st, _) = call(&app, "POST", &format!("/v1/zones/{zid}/shuffle"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (_, z) = call(&app, "GET", &format!("/v1/zones/{zid}"), None).await;
    assert_eq!(z["shuffle"], true);
    // A transfer carries the mode
    let (st, k) = call(
        &app,
        "POST",
        "/v1/zones",
        Some(serde_json::json!({"name": "Kitchen", "output": "null"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{k}");
    let (st, moved) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/transfer"),
        Some(serde_json::json!({"to": k["id"]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{moved}");
    assert_eq!(moved["shuffle"], true, "{moved}");
    assert_eq!(moved["queue"].as_array().unwrap().len(), 10);
    // With its original order: off on the new zone puts the items back
    let (st, z) = call(
        &app,
        "PUT",
        &format!("/v1/zones/{}/shuffle", k["id"].as_str().unwrap()),
        Some(serde_json::json!({"shuffle": false})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{z}");
    assert_eq!(titles(&z), want, "the order the source had");
    assert_eq!(z["queue_index"], 2);
}

/// A queue loaded with shuffle on plays the chosen item first and the rest shuffled after it;
/// off puts the loaded order back. Loaded without shuffle, it is in order.
#[tokio::test]
async fn a_queue_loaded_with_shuffle_starts_from_the_chosen_item() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("short.wav");
    write_wav(&wav, 441);
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    let items: Vec<_> = (0..6)
        .map(|i| serde_json::json!({"uri": wav.to_string_lossy(), "title": format!("t{i}")}))
        .collect();
    let titles = |z: &serde_json::Value| -> Vec<String> {
        z["queue"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["title"].as_str().unwrap_or_default().to_string())
            .collect()
    };
    let original: Vec<String> = (0..6).map(|i| format!("t{i}")).collect();
    let (st, z) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/load"),
        Some(serde_json::json!({"items": items, "index": 3, "transport": "stopped", "shuffle": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{z}");
    assert_eq!(
        (z["shuffle"].as_bool(), z["queue_index"].as_u64()),
        (Some(true), Some(0)),
        "{z}"
    );
    let t = titles(&z);
    assert_eq!(t[0], "t3", "the chosen item first");
    let mut rest = t[1..].to_vec();
    rest.sort();
    assert_eq!(rest, ["t0", "t1", "t2", "t4", "t5"]);
    let (st, z) = call(
        &app,
        "PUT",
        &format!("/v1/zones/{zid}/shuffle"),
        Some(serde_json::json!({"shuffle": false})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{z}");
    assert_eq!(titles(&z), original);
    assert_eq!(z["queue_index"], 3);
    // In order, shuffle off, after a shuffled queue
    let (st, z) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/load"),
        Some(serde_json::json!({"items": items, "index": 1, "transport": "stopped"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{z}");
    assert_eq!(z["shuffle"], false);
    assert_eq!(titles(&z), original);
}

/// A shuffled queue comes back after a restart shuffled, with its original order kept, so
/// shuffle off still puts it back.
#[tokio::test]
async fn a_shuffled_queue_is_restored_with_its_order() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("short.wav");
    write_wav(&wav, 441);
    let zid;
    let original: Vec<String> = (0..5).map(|i| format!("t{i}")).collect();
    {
        let s = state(dir.path(), None).await;
        let app = router(s.clone());
        let (_, zones) = call(&app, "GET", "/v1/zones", None).await;
        zid = zones[0]["id"].as_str().unwrap().to_string();
        let items: Vec<_> = original
            .iter()
            .map(|t| serde_json::json!({"uri": wav.to_string_lossy(), "title": t}))
            .collect();
        let (st, _) = call(
            &app,
            "POST",
            &format!("/v1/zones/{zid}/queue"),
            Some(serde_json::json!({"items": items, "replace": true})),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        // The queue is saved just before the zone's state goes out on the bus, so each zone
        // event is the moment to look (no fixed poll a loaded runner can outlast)
        let mut events = s.events.subscribe();
        let (st, z) = call(
            &app,
            "PUT",
            &format!("/v1/zones/{zid}/shuffle"),
            Some(serde_json::json!({"shuffle": true})),
        )
        .await;
        assert_eq!((st, z["shuffle"].as_bool()), (StatusCode::OK, Some(true)), "{z}");
        let key = format!("queue.{zid}");
        let saved_shuffled = || {
            s.library
                .lock()
                .unwrap()
                .get_setting(&key)
                .unwrap()
                .is_some_and(|j| j.contains("\"shuffle\":true"))
        };
        let saved = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            while !saved_shuffled() {
                // Lagged: look again
                let _ = events.recv().await;
            }
        })
        .await;
        assert!(saved.is_ok(), "the mode was not saved");
    }
    let s2 = state(dir.path(), None).await;
    let app2 = router(s2.clone());
    let (_, z) = call(&app2, "GET", &format!("/v1/zones/{zid}"), None).await;
    assert_eq!(z["shuffle"], true, "{z}");
    let (st, z) = call(
        &app2,
        "PUT",
        &format!("/v1/zones/{zid}/shuffle"),
        Some(serde_json::json!({"shuffle": false})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{z}");
    let titles: Vec<String> = z["queue"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["title"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(titles, original);
}

/// A zone's volume limit holds whichever way the volume is set: the page's route, an
/// announcement's volume (and the level put back after it), a UPnP controller's SetVolume
/// (whose GetVolume then shows the level taken), a Subsonic client's jukebox gain, an up
/// step; the output never hears a level above it. Without a limit requests pass as asked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_volume_limit_holds_for_every_way_of_setting_the_volume() {
    use pm_model::{Volume, VolumeKind, VolumeLevel};
    use pm_renderer::{Act, Host};
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), Some("s3cret")).await;
    let pc = Arc::new(NotedVolume {
        inner: NullOutput::with_volume(VolumeKind::Percent),
        asked: Default::default(),
    });
    let db = Arc::new(NotedVolume {
        inner: NullOutput::with_volume(VolumeKind::Db {
            min: -60.0,
            max: 0.0,
            step: 1.0,
        }),
        asked: Default::default(),
    });
    let (pc_out, db_out): (Arc<dyn Output>, Arc<dyn Output>) = (pc.clone(), db.clone());
    s.outputs.register(pc_out.clone());
    s.outputs.register(db_out.clone());
    let pc_zone = s.create_zone("Linn", pc_out);
    let db_zone = s.create_zone("Amp", db_out);
    let app = router(s.clone());
    let (pz, dz) = (pc_zone.id().0.clone(), db_zone.id().0.clone());
    // As the owner (the node has a token here, so the Subsonic client can sign in with it)
    let owner = |method: &'static str, uri: String, body: Option<serde_json::Value>| {
        let app = app.clone();
        async move {
            let req = Request::builder()
                .method(method)
                .uri(&uri)
                .header("content-type", "application/json")
                .header("authorization", "Bearer s3cret")
                .body(match body {
                    Some(b) => Body::from(b.to_string()),
                    None => Body::empty(),
                })
                .unwrap();
            let resp = app.oneshot(req).await.unwrap();
            let status = resp.status();
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            (status, json)
        }
    };
    for (z, max) in [(&pz, -30.0), (&dz, -20.0)] {
        let (st, c) = owner(
            "PUT",
            format!("/v1/zones/{z}/dsp"),
            Some(serde_json::json!({"max_volume_db": max})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{c}");
    }
    let asked = |o: &NotedVolume| o.asked.lock().unwrap().clone();
    let clear = |o: &NotedVolume| o.asked.lock().unwrap().clear();
    // The page's route: 100 % is 50 % on a linear output at -30 dB
    let (st, _) = owner(
        "POST",
        format!("/v1/zones/{pz}/volume"),
        Some(serde_json::json!({"volume": {"percent": 100}})),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(asked(&pc), [Volume::Percent(50)]);
    clear(&pc);
    // An announcement at full volume: the clip's level and the one put back afterwards (the
    // 100 % asked above) both reach the output held to the limit
    let (st, a) = owner(
        "POST",
        format!("/v1/zones/{pz}/announce"),
        Some(serde_json::json!({"chime": true, "volume_percent": 100})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{a}");
    assert_eq!(a["volume_applied"], true, "{a}");
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while asked(&pc).len() < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the level was not put back after the announcement");
    assert_eq!(asked(&pc), [Volume::Percent(50), Volume::Percent(50)]);
    clear(&pc);
    // A UPnP controller's SetVolume 100: the output hears 50, and GetVolume answers 50
    let r = crate::renderer::ZoneRenderers::new(&s);
    r.act(&pz, Act::Volume(100)).await.unwrap();
    assert_eq!(asked(&pc), [Volume::Percent(50)]);
    assert_eq!(r.view(&pz).unwrap().volume, 50, "GetVolume shows the level taken");
    clear(&pc);
    // ...also on a zone whose level is in dB: the request is held to the output's percent
    // for -20 dB (66 % of its range), and GetVolume answers in that percent
    r.act(&dz, Act::Volume(100)).await.unwrap();
    assert_eq!(asked(&db), [Volume::Percent(66)]);
    assert_eq!(
        r.view(&dz).unwrap().volume,
        66,
        "GetVolume in the output's percent"
    );
    clear(&db);
    // A Subsonic client's jukebox gain 1.0 (0 dB) on a zone limited to -20 dB
    let (st, b) = call(
        &app,
        "GET",
        &format!(
            "/rest/jukeboxControl?action=setGain&gain=1.0&zone={dz}&u={}&p=s3cret&v=1.16.1&c=test&f=json",
            s.subsonic_user
        ),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(asked(&db), [Volume::Db(-20.0)], "{b}");
    clear(&db);
    // A step up the device takes past the limit is set back to it
    db_zone.set_volume(Volume::Db(-20.5)).await.unwrap();
    db_zone.set_volume(Volume::Up).await.unwrap();
    db_zone.wait_acked().await;
    assert_eq!(asked(&db), [Volume::Db(-20.5), Volume::Up, Volume::Db(-20.0)]);
    assert_eq!(
        db_zone.state().volume.map(|v| v.level),
        Some(VolumeLevel::Db(-20.0))
    );
    // No limit: as asked
    let (st, _) = owner("DELETE", format!("/v1/zones/{pz}/dsp"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    r.act(&pz, Act::Volume(100)).await.unwrap();
    assert_eq!(asked(&pc), [Volume::Percent(100)]);
    assert_eq!(r.view(&pz).unwrap().volume, 100);
}

/// `GET /v1/zones/{id}/volume` reads the device now and publishes it (the app asks this after
/// the phone's volume keys; it never sets the level to report it): what the device says, also
/// a change made on the device itself; `null` for an output without a volume.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_zones_level_is_read_from_the_device_on_request() {
    use pm_zones::outputs::NullOutput;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let raw = Arc::new(NullOutput::with_volume(pm_model::VolumeKind::Percent));
    let out: Arc<dyn pm_zones::Output> = raw.clone();
    s.outputs.register(out.clone());
    let zone = s.create_zone("Phone-like", out);
    let plain: Arc<dyn pm_zones::Output> = Arc::new(NullOutput::new());
    s.outputs.register(plain.clone());
    let none = s.create_zone("No volume", plain);
    let app = router(s.clone());
    zone.wait_acked().await;
    // The device changes its level itself (a volume key on the phone)
    raw.pretend_level(pm_model::ZoneVolume {
        level: pm_model::VolumeLevel::Percent(7),
        muted: false,
    });
    let zid = zone.id().0.clone();
    let (st, v) = call(&app, "GET", &format!("/v1/zones/{zid}/volume"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v, serde_json::json!({"percent": 7, "muted": false}));
    zone.wait_acked().await;
    let (_, z) = call(&app, "GET", &format!("/v1/zones/{zid}"), None).await;
    assert_eq!(
        z["volume"],
        serde_json::json!({"percent": 7, "muted": false}),
        "published: {z}"
    );
    let nid = none.id().0.clone();
    let (st, v) = call(&app, "GET", &format!("/v1/zones/{nid}/volume"), None).await;
    assert_eq!((st, v), (StatusCode::OK, serde_json::Value::Null));
    let (st, _) = call(&app, "GET", "/v1/zones/no-such-zone/volume", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

/// A queue replaced while the zone's shuffle is on plays shuffled: shuffle stays on and means
/// it, the order sent is kept as the original one, and shuffle off gives it back.
#[tokio::test]
async fn a_queue_replaced_with_shuffle_on_plays_shuffled() {
    let (dir, s, app, zid) = looping_queue(8, 0).await;
    let wav = dir.path().join("t.wav");
    call(
        &app,
        "PUT",
        &format!("/v1/zones/{zid}/shuffle"),
        Some(serde_json::json!({"shuffle": true})),
    )
    .await;
    let sent: Vec<String> = (0..10).map(|i| format!("n{i}")).collect();
    let items: Vec<_> = sent
        .iter()
        .map(|t| serde_json::json!({"uri": wav.to_string_lossy(), "title": t}))
        .collect();
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    // A shuffle leaves ten items as they were once in 10! tries: replace again then
    let mut played = Vec::new();
    for _ in 0..3 {
        let (st, _) = call(
            &app,
            "POST",
            &format!("/v1/zones/{zid}/queue"),
            Some(serde_json::json!({"items": items, "replace": true})),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let (_, z) = call(&app, "GET", &format!("/v1/zones/{zid}"), None).await;
        assert_eq!(z["shuffle"], true, "{z}");
        played = queue_titles(&z);
        if played != sent {
            break;
        }
    }
    assert_ne!(played, sent, "plays shuffled");
    let order = zone.shuffle_order().expect("shuffled");
    let original: Vec<String> = order.iter().map(|i| played[*i].clone()).collect();
    assert_eq!(original, sent, "the order sent is the original order");
    let (_, z) = call(
        &app,
        "PUT",
        &format!("/v1/zones/{zid}/shuffle"),
        Some(serde_json::json!({"shuffle": false})),
    )
    .await;
    assert_eq!(queue_titles(&z), sent, "shuffle off: the order sent");
    // Shuffle off: a replace plays in the order sent
    let (_, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"items": items, "replace": true})),
    )
    .await;
    let (_, z) = call(&app, "GET", &format!("/v1/zones/{zid}"), None).await;
    assert_eq!(
        (queue_titles(&z), z["shuffle"].as_bool()),
        (sent.clone(), Some(false))
    );
}

/// Where the playing item is after its queue changes order: `now_playing.index` follows it
/// (shuffle off puts it back in its original place; an earlier item moved after it moves it
/// up one), always the same as `queue_index`.
#[tokio::test]
async fn now_playing_index_follows_the_item_when_the_queue_reorders() {
    let (_dir, s, app, zid) = looping_queue(8, 0).await;
    let zone = s.zones.get(&ZoneId::new(&zid)).unwrap();
    let same_place = |z: &serde_json::Value| {
        let qi = z["queue_index"].as_u64().unwrap() as usize;
        assert_eq!(z["now_playing"]["index"].as_u64(), Some(qi as u64), "{z}");
        assert_eq!(z["now_playing"]["item"]["id"], z["queue"][qi]["id"], "{z}");
        z["now_playing"]["item"]["title"].as_str().unwrap().to_string()
    };
    // Shuffle on (the playing item goes first), play another one, shuffle off
    call(
        &app,
        "PUT",
        &format!("/v1/zones/{zid}/shuffle"),
        Some(serde_json::json!({"shuffle": true})),
    )
    .await;
    let (_, z) = call(&app, "GET", &format!("/v1/zones/{zid}"), None).await;
    let k = 5;
    let title = z["queue"][k]["title"].as_str().unwrap().to_string();
    call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/transport"),
        Some(serde_json::json!({"action": "play", "index": k})),
    )
    .await;
    for _ in 0..500 {
        if zone.state().now_playing.is_some_and(|n| n.index == k) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    call(
        &app,
        "PUT",
        &format!("/v1/zones/{zid}/shuffle"),
        Some(serde_json::json!({"shuffle": false})),
    )
    .await;
    let z = serde_json::to_value(zone.wait_acked().await).unwrap();
    assert_eq!(same_place(&z), title);
    let at: usize = title[1..].parse().unwrap();
    assert_eq!(
        z["queue_index"].as_u64(),
        Some(at as u64),
        "back in its original place"
    );
    // An earlier item moved after the playing one: it moves up one
    let first = z["queue"][0]["id"].as_u64().unwrap();
    zone.move_item(first, at);
    let st = zone.wait_acked().await;
    let z = serde_json::to_value(&st).unwrap();
    assert_eq!(same_place(&z), title);
    assert_eq!(z["queue_index"].as_u64(), Some(at as u64 - 1));
}

/// A write to a zone marks the zone the handler acts on, however the request spells its
/// name (the router's decoded parameter, not the raw path).
#[tokio::test]
async fn a_zone_named_another_way_is_still_that_zone() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), Some("s3cret")).await;
    let app = router(s.clone());
    let key = "tbepfp8fisyiqpskbmxsmpub47gxb7xdsh5x9ut9qrb97gcq9phy";
    let people = serde_json::json!({"people": [
        {"key": key, "access": "music", "music_enabled": true, "added_at": 1}]});
    s.library
        .lock()
        .unwrap()
        .set_setting("people", &people.to_string())
        .unwrap();
    let tok = accounts::mint(&s, key).unwrap();
    let (_, zones) = call_auth(&app, "GET", "/v1/zones", "s3cret").await;
    let zid = zones[0]["id"].as_str().unwrap().to_string();
    assert!(listens::plays_are_the_owners(&s, &zid));
    // The same zone, its name written another way in the address
    let spelled = format!("%{:02x}{}", zid.as_bytes()[0], &zid[1..]);
    let (st, v) = as_who(
        &app,
        "POST",
        format!("/v1/zones/{spelled}/queue"),
        &tok,
        Some(serde_json::json!({"items": [{"uri": "https://example.com/a.mp3"}]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(
        !listens::plays_are_the_owners(&s, &zid),
        "the zone is in the listener's hands"
    );
    let hands = s.zones_in_listeners_hands.lock().unwrap().clone();
    assert_eq!(hands.len(), 1, "and under no other name: {hands:?}");
}
