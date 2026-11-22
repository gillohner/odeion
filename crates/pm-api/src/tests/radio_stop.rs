//! Library radio adds songs when a zone's queue runs out, and only then.

use super::*;

/// With radio on, a Stop is a stop: the zone stays stopped and its queue is as it was. (Radio
/// went on after every stop: a Stop added similar songs and started them, and the stop before a
/// queue is replaced, cleared or moved to another zone could add them to the new queue.)
#[tokio::test]
async fn radio_does_not_start_again_after_a_stop() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("music")).unwrap();
    // Three seconds: still playing when the Stop comes (the zone below plays in real time)
    write_wav(&dir.path().join("music/one.wav"), 3 * 44_100);
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
    let long = tracks["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["file"]["path"].as_str().is_some_and(|p| p.ends_with("one.wav")))
        .unwrap_or_else(|| panic!("no song from one.wav: {tracks}"))["id"]
        .as_i64()
        .unwrap();
    let zone = s.create_zone("Paced", Arc::new(NullOutput::realtime()));
    let zid = zone.id().to_string();
    let (st, _) = call(
        &app,
        "PUT",
        &format!("/v1/zones/{zid}/radio"),
        Some(serde_json::json!({"on": true})),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let mut events = s.events.subscribe();
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/queue"),
        Some(serde_json::json!({"track_ids": [long], "replace": true, "play": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    for _ in 0..500 {
        if zone.state().now_playing.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(zone.state().now_playing.is_some(), "the song did not start");
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/zones/{zid}/transport"),
        Some(serde_json::json!({"action": "stop"})),
    )
    .await;
    assert!(st.is_success(), "{st}");
    // The node tells its controllers of the stop after it has acted on it itself
    let told = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(crate::events::Sequenced {
                    event: crate::types::ApiEvent::ZoneEvent { zone, event },
                    ..
                }) if zone.to_string() == zid => {
                    if let pm_zones::ZoneEvent::Stopped { queue_ended } = event {
                        return queue_ended;
                    }
                }
                Ok(_) => {}
                Err(e) => panic!("events: {e}"),
            }
        }
    })
    .await
    .expect("no stopped event");
    assert!(!told, "a Stop is not the end of the queue");
    assert_eq!(zone.queue().len(), 1, "radio added songs after a Stop");
    let st = zone.wait_stopped().await;
    assert_eq!(st.transport, pm_zones::Transport::Stopped);
    assert_eq!(st.queue.len(), 1);
    assert_eq!(st.error, None);
}
