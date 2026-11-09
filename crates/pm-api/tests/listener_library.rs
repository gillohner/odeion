//! A listener's own bookmarks (TASK-listener-personal-library-1104, ADR 0016 step 5): on their
//! own homeserver, through the session they gave the node, never the owner's. The listener's
//! homeserver is a fake here (`pm_api::personal::PersonalHomeserver`); how the node's own
//! reaches it (`Grants`: what its errors mean, the grant file) has unit tests in personal.rs.

mod personal_fake;

use axum::http::StatusCode;
use personal_fake::*;
use pm_api::personal::PersonalHomeserver;
use pm_api::router;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn each_person_bookmarks_on_their_own_homeserver() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, bo) = node(dir.path());
    fake.connected
        .lock()
        .unwrap()
        .extend([ANN.to_string(), BO.to_string()]);
    let app = router(s.clone());

    let (st, v) = call(
        &app,
        OWNER,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_A})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    // The owner's rows and what the owner's sync still has to push: nothing of a listener's joins them
    let owners = owners_rows(&s);
    assert!(owners.contains(&SONG_A.to_string()), "{owners:?}");
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_B})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["bookmarked"], true, "{v}");
    let (st, v) = call(
        &app,
        &bo,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_C})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");

    // Each reads their own, and only that
    let (_, v) = call(&app, OWNER, "GET", "/v1/bookmarks", None).await;
    assert_eq!(uris(&v), vec![SONG_A], "the owner's: {v}");
    let (_, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert_eq!(uris(&v), vec![SONG_B], "Ann's: {v}");
    assert_eq!(v["homeserver"]["connected"], true, "{v}");
    let (_, v) = call(&app, &bo, "GET", "/v1/bookmarks", None).await;
    assert_eq!(uris(&v), vec![SONG_C], "Bo's: {v}");
    // By the older name too (the page and the offline app's sync use it)
    let (st, v) = call(&app, &bo, "GET", "/v1/favourites", None).await;
    assert_eq!((st, uris(&v)), (StatusCode::OK, vec![SONG_C.to_string()]), "{v}");

    // On their homeservers, through their own grant; the owner's tables hold the owner's only
    assert_eq!(fake.of(ANN), vec![SONG_B]);
    assert_eq!(fake.of(BO), vec![SONG_C]);
    assert_eq!(
        owners_rows(&s),
        owners,
        "a listener's bookmark never lands in the owner's"
    );
    assert!(
        !fake.calls.lock().unwrap().iter().any(|c| c.contains(SONG_A)),
        "the owner's bookmark never goes to a listener's homeserver"
    );

    // Removing: only the remover's own
    let q = format!("/v1/bookmarks?uri={}", urlencode(SONG_B));
    let (st, v) = call(&app, &ann, "DELETE", &q, None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["bookmarked"], false);
    assert!(fake.of(ANN).is_empty());
    let q = format!("/v1/bookmarks?uri={}", urlencode(SONG_A));
    let (st, _) = call(&app, &bo, "DELETE", &q, None).await;
    assert_eq!(
        st,
        StatusCode::OK,
        "Bo removing a bookmark he doesn't have is a no-op"
    );
    assert_eq!(owners_rows(&s), owners, "and never the owner's");
}

#[tokio::test]
async fn a_listener_without_their_homeserver_is_asked_to_connect() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    let app = router(s.clone());
    call(
        &app,
        OWNER,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_A})),
    )
    .await;
    let owners = owners_rows(&s);

    let (st, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(uris(&v).is_empty(), "never the owner's: {v}");
    assert_eq!(v["homeserver"]["connected"], false, "{v}");
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_B})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["needs_connect"], true, "{v}");
    assert_eq!(owners_rows(&s), owners);
    assert!(fake.of(ANN).is_empty());
    let (st, v) = call(&app, &ann, "GET", "/v1/me/homeserver", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["connected"], false, "{v}");
    // The owner's homeserver is the node's own: this is for listeners
    let (st, _) = call(&app, OWNER, "POST", "/v1/me/homeserver/connect", None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_refused_grant_is_lost_and_never_falls_back_to_the_owner() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_B})),
    )
    .await;
    assert_eq!(fake.of(ANN), vec![SONG_B]);

    // Ann revoked the node in Ring
    fake.refused.lock().unwrap().insert(ANN.to_string());
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_C})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["needs_connect"], true, "{v}");
    assert_eq!(v["lost"], true, "{v}");
    // What the node had of hers still reads, marked
    let (st, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(uris(&v), vec![SONG_B], "{v}");
    assert_eq!(v["homeserver"]["lost"], true, "{v}");
    let (_, v) = call(&app, &ann, "GET", "/v1/me/homeserver", None).await;
    assert_eq!(v["lost"], true, "{v}");
    assert!(owners_rows(&s).is_empty(), "nothing went to the owner's");
}

#[tokio::test]
async fn an_unreachable_homeserver_keeps_the_change_until_it_answers() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    fake.down.store(true, Ordering::SeqCst);
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_B})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["pending"], true, "{v}");
    let (_, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert_eq!(uris(&v), vec![SONG_B], "kept on the node meanwhile: {v}");
    assert_eq!(v["homeserver"]["pending"], 1, "{v}");
    assert!(fake.of(ANN).is_empty());
    // Back: the next read writes it
    fake.down.store(false, Ordering::SeqCst);
    let (_, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert_eq!(uris(&v), vec![SONG_B], "{v}");
    assert_eq!(v["homeserver"]["pending"], 0, "{v}");
    assert_eq!(fake.of(ANN), vec![SONG_B]);
    // A removal made while it is away too
    fake.down.store(true, Ordering::SeqCst);
    let q = format!("/v1/bookmarks?uri={}", urlencode(SONG_B));
    let (st, v) = call(&app, &ann, "DELETE", &q, None).await;
    assert_eq!(
        (st, v["pending"].clone()),
        (StatusCode::OK, serde_json::json!(true)),
        "{v}"
    );
    let (_, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert!(uris(&v).is_empty(), "{v}");
    fake.down.store(false, Ordering::SeqCst);
    call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert!(fake.of(ANN).is_empty());
    assert!(owners_rows(&s).is_empty());
}

#[tokio::test]
async fn disconnecting_and_removing_a_person() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_B})),
    )
    .await;
    let (st, v) = call(&app, &ann, "DELETE", "/v1/me/homeserver", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (_, v) = call(&app, &ann, "GET", "/v1/me/homeserver", None).await;
    assert_eq!(v["connected"], false, "{v}");
    assert_eq!(
        fake.of(ANN),
        vec![SONG_B],
        "disconnecting deletes nothing on her homeserver"
    );
    // People › Remove: her grant goes too
    fake.connected.lock().unwrap().insert(ANN.to_string());
    pm_api::accounts::forget_person(&s, ANN).unwrap();
    // (forget_person is sync; the grant goes with it, the fake hears it in the background)
    for _ in 0..50 {
        if !fake.connected(ANN) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(!fake.connected(ANN), "removed people keep no grant");
}

#[tokio::test]
async fn a_listener_reaches_only_their_own_bookmark_routes() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    // (Their own collections are theirs too: tests/listener_collections.rs)
    for (m, p) in [
        ("GET", "/v1/smart"),
        ("POST", "/v1/playlists/from-queue"),
        ("GET", "/v1/tags"),
        ("PUT", "/v1/collections"),
        ("GET", "/v1/me/listens"),
    ] {
        let (st, v) = call(
            &app,
            &ann,
            m,
            p,
            Some(serde_json::json!({"name": "x", "uri": SONG_B})),
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{m} {p}: {v}");
    }
    assert!(
        fake.calls.lock().unwrap().is_empty(),
        "{:?}",
        fake.calls.lock().unwrap()
    );
}

#[tokio::test]
async fn a_direct_change_wins_over_one_still_waiting() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    // Hearted while her homeserver is away (it waits), un-hearted once it is back
    fake.down.store(true, Ordering::SeqCst);
    call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_B})),
    )
    .await;
    fake.down.store(false, Ordering::SeqCst);
    let q = format!("/v1/bookmarks?uri={}", urlencode(SONG_B));
    let (st, v) = call(&app, &ann, "DELETE", &q, None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    // The next read must not bring the waiting heart back
    let (_, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert!(uris(&v).is_empty(), "{v}");
    assert_eq!(v["homeserver"]["pending"], 0, "{v}");
    assert!(fake.of(ANN).is_empty(), "the un-heart stands");
}

#[tokio::test]
async fn changes_made_at_once_all_wait_and_all_land() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    fake.down.store(true, Ordering::SeqCst);
    let songs: Vec<String> = (0..6)
        .map(|i| format!("https://musicbrainz.org/recording/0c9b1d6e-4e5f-4f3a-9c11-5b7d0a6e2f4{i}"))
        .collect();
    // Six hearts at once (two devices, say), and a read meanwhile
    let mut all = Vec::new();
    for u in &songs {
        let (app, ann, u) = (app.clone(), ann.clone(), u.clone());
        all.push(tokio::spawn(async move {
            call(
                &app,
                &ann,
                "PUT",
                "/v1/bookmarks",
                Some(serde_json::json!({"uri": u})),
            )
            .await
        }));
    }
    let reading = {
        let (app, ann) = (app.clone(), ann.clone());
        tokio::spawn(async move { call(&app, &ann, "GET", "/v1/bookmarks", None).await })
    };
    for h in all {
        let (st, v) = h.await.unwrap();
        assert_eq!(
            (st, v["pending"].clone()),
            (StatusCode::OK, serde_json::json!(true)),
            "{v}"
        );
    }
    reading.await.unwrap();
    let (_, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert_eq!(v["homeserver"]["pending"], 6, "none of them lost: {v}");
    fake.down.store(false, Ordering::SeqCst);
    call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    let mut want = songs.clone();
    want.sort();
    assert_eq!(fake.of(ANN), want, "every one reached her homeserver");
}

#[tokio::test]
async fn an_offline_change_keeps_its_time() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    // A heart a device made offline at 1000 s keeps that time
    call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_B, "at": 1000})),
    )
    .await;
    assert_eq!(fake.at(ANN, SONG_B), Some(1_000_000_000));
    // Hearted again now; a clear the device made offline at 2000 s, sent later, is older: it stays
    call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_C})),
    )
    .await;
    let q = format!("/v1/bookmarks?uri={}&at=2000", urlencode(SONG_C));
    let (st, v) = call(&app, &ann, "DELETE", &q, None).await;
    assert_eq!(
        (st, v["bookmarked"].clone()),
        (StatusCode::OK, serde_json::json!(true)),
        "{v}"
    );
    assert!(fake.of(ANN).contains(&SONG_C.to_string()));
}

#[tokio::test]
async fn a_disconnected_listener_reads_nothing_and_a_removed_one_leaves_no_copy() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_B})),
    )
    .await;
    call(&app, &ann, "DELETE", "/v1/me/homeserver", None).await;
    let (_, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert!(
        uris(&v).is_empty(),
        "not connected: none (as docs/api.md says): {v}"
    );
    let copy = format!("person.{ANN}.bookmarks");
    assert!(
        s.library.lock().unwrap().get_setting(&copy).unwrap().is_some(),
        "the copy waits for a reconnect"
    );
    pm_api::accounts::forget_person(&s, ANN).unwrap();
    assert!(
        s.library.lock().unwrap().get_setting(&copy).unwrap().is_none(),
        "removed: no copy left"
    );
}

#[tokio::test]
async fn a_listener_whose_music_access_is_off_reaches_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let roster = serde_json::json!({"people": [
        {"key": ANN, "access": "music", "music_enabled": false, "added_at": 0}]});
    s.library
        .lock()
        .unwrap()
        .set_setting("people", &roster.to_string())
        .unwrap();
    let app = router(s.clone());
    let (st, _) = call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_B})),
    )
    .await;
    assert!(
        st == StatusCode::UNAUTHORIZED || st == StatusCode::FORBIDDEN,
        "{st}"
    );
    assert!(fake.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_library_item_not_identified_yet_is_no_listener_bookmark() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    // A song in an album MusicBrainz doesn't know yet
    let track: pm_model::Track = serde_json::from_value(serde_json::json!({
        "id": null, "title": "A", "artists": [{"id": null, "name": "X", "mbid": null}],
        "album": {"id": null, "title": "No Id", "album_artist": "X", "release_mbid": null,
                  "release_group_mbid": null, "year": null, "upc": null, "total_discs": null},
        "track_no": 1, "disc_no": null, "duration_ms": 1000,
        "file": {"path": dir.path().join("A.flac"), "size_bytes": 1, "mtime_unix": 0, "container": "flac",
                 "format": serde_json::to_value(pm_model::AudioFormat::pcm(44_100, 16, 2)).unwrap(), "duration_ms": 1000},
        "recording_mbid": null, "isrc": null, "acoustid": null, "canonical": null,
        "replaygain_track_gain_db": null, "replaygain_album_gain_db": null}))
    .unwrap();
    let (tid, album) = {
        let lib = s.library.lock().unwrap();
        let tid = lib.upsert_track(&track).unwrap();
        (
            tid.0,
            lib.get_track(tid).unwrap().unwrap().album.unwrap().id.unwrap(),
        )
    };
    for body in [
        serde_json::json!({"album_id": album}),
        serde_json::json!({"track_id": tid}),
    ] {
        let (st, v) = call(&app, &ann, "PUT", "/v1/bookmarks", Some(body.clone())).await;
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{body}: {v}");
    }
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"album_id": 999_999})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND, "{v}");
    assert!(
        fake.calls.lock().unwrap().is_empty(),
        "nothing reaches her homeserver"
    );
    assert!(owners_rows(&s).is_empty(), "and nothing lands in the owner's");
    assert!(
        s.library
            .lock()
            .unwrap()
            .get_setting("bookmarks.local.albums")
            .unwrap()
            .is_none(),
        "nor in the owner's node-only bookmarks"
    );
}

#[tokio::test]
async fn an_older_heart_never_brings_back_a_newer_clear() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let heart = |uri: &str, at: Option<i64>| {
        let mut b = serde_json::json!({"uri": uri});
        if let Some(at) = at {
            b["at"] = at.into();
        }
        b
    };
    // Her phone clears a heart now; her laptop's heart from an hour ago (offline) comes in after
    call(&app, &ann, "PUT", "/v1/bookmarks", Some(heart(SONG_B, None))).await;
    let q = format!("/v1/bookmarks?uri={}", urlencode(SONG_B));
    let (st, _) = call(&app, &ann, "DELETE", &q, None).await;
    assert_eq!(st, StatusCode::OK);
    let hour_ago = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        - 3600;
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(heart(SONG_B, Some(hour_ago))),
    )
    .await;
    assert_eq!(
        (st, v["bookmarked"].clone()),
        (StatusCode::OK, serde_json::json!(false)),
        "the newer clear stands: {v}"
    );
    assert!(fake.of(ANN).is_empty(), "nothing written back to her homeserver");
    let (_, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert!(uris(&v).is_empty(), "{v}");
    // The same while her homeserver is away: the older heart doesn't replace the waiting clear
    call(&app, &ann, "PUT", "/v1/bookmarks", Some(heart(SONG_C, None))).await;
    fake.down.store(true, Ordering::SeqCst);
    let q = format!("/v1/bookmarks?uri={}", urlencode(SONG_C));
    let (_, v) = call(&app, &ann, "DELETE", &q, None).await;
    assert_eq!(v["pending"], true, "{v}");
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(heart(SONG_C, Some(hour_ago))),
    )
    .await;
    assert_eq!(
        (st, v["bookmarked"].clone()),
        (StatusCode::OK, serde_json::json!(false)),
        "{v}"
    );
    let (_, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert!(uris(&v).is_empty(), "{v}");
    assert_eq!(v["homeserver"]["pending"], 1, "only the clear waits: {v}");
    fake.down.store(false, Ordering::SeqCst);
    call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert!(
        fake.of(ANN).is_empty(),
        "the clear landed, the old heart never did"
    );
    // A heart made after the clear is hers again
    let (_, v) = call(&app, &ann, "PUT", "/v1/bookmarks", Some(heart(SONG_C, None))).await;
    assert_eq!(v["bookmarked"], true, "{v}");
    assert_eq!(fake.of(ANN), vec![SONG_C]);
    assert!(owners_rows(&s).is_empty());
}

#[tokio::test]
async fn a_change_her_homeserver_never_takes_is_refused_for_good() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    fake.rejects.lock().unwrap().insert(SONG_B.to_string());
    let app = router(s.clone());
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_B})),
    )
    .await;
    // A 4xx: a device's offline queue drops it and goes on (a 5xx it would send again forever)
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    let (_, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert!(uris(&v).is_empty(), "{v}");
    assert_eq!(v["homeserver"]["pending"], 0, "nothing waits: {v}");
    assert!(owners_rows(&s).is_empty());
}

#[tokio::test]
async fn a_waiting_change_never_taken_is_dropped_and_a_refusal_loses_the_grant() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let heart = |uri: &str| Some(serde_json::json!({"uri": uri}));
    // Two hearts wait while her homeserver is away
    fake.down.store(true, Ordering::SeqCst);
    call(&app, &ann, "PUT", "/v1/bookmarks", heart(SONG_B)).await;
    call(&app, &ann, "PUT", "/v1/bookmarks", heart(SONG_C)).await;
    // Back, but it will never take the first: that one goes, the next lands
    fake.down.store(false, Ordering::SeqCst);
    fake.rejects.lock().unwrap().insert(SONG_B.to_string());
    let (_, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert_eq!(v["homeserver"]["pending"], 0, "{v}");
    assert_eq!(fake.of(ANN), vec![SONG_C], "the one after it was written");
    // Another waits; then she revokes the node in Ring: the grant is lost, the change still waits
    fake.down.store(true, Ordering::SeqCst);
    call(&app, &ann, "PUT", "/v1/bookmarks", heart(SONG_A)).await;
    fake.down.store(false, Ordering::SeqCst);
    fake.refused.lock().unwrap().insert(ANN.to_string());
    let (_, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert_eq!(v["homeserver"]["lost"], true, "{v}");
    assert_eq!(
        v["homeserver"]["pending"], 1,
        "kept until she approves again: {v}"
    );
    assert!(owners_rows(&s).is_empty());
}

/// A regression check for People › Remove while a change waits on her homeserver. (The narrower
/// gap between the roster check and the write is closed by construction, one library hold for
/// both in `personal::save`; a test can't land a removal inside it.)
#[tokio::test]
async fn a_listener_removed_while_her_change_is_on_its_way_leaves_no_copy() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (app2, ann2) = (app.clone(), ann.clone());
    let heart = tokio::spawn(async move {
        call(
            &app2,
            &ann2,
            "PUT",
            "/v1/bookmarks",
            Some(serde_json::json!({"uri": SONG_B})),
        )
        .await
    });
    // Her homeserver takes a moment; meanwhile People › Remove: off the roster, then her data goes
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let roster = serde_json::json!({"people": [
        {"key": BO, "access": "music", "music_enabled": true, "added_at": 0}]});
    s.library
        .lock()
        .unwrap()
        .set_setting("people", &roster.to_string())
        .unwrap();
    pm_api::accounts::forget_person(&s, ANN).unwrap();
    heart.await.unwrap();
    assert!(
        s.library
            .lock()
            .unwrap()
            .get_setting(&format!("person.{ANN}.bookmarks"))
            .unwrap()
            .is_none(),
        "her copy doesn't come back once she is removed"
    );
}

#[tokio::test]
async fn a_late_repeat_never_moves_a_bookmark_back_in_time() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let heart = |uri: &str, at: i64| Some(serde_json::json!({"uri": uri, "at": at}));
    let clear = |uri: &str, at: i64| format!("/v1/bookmarks?uri={}&at={at}", urlencode(uri));
    // A heart at 200 s, a late repeat of it from 100 s, then a clear from 150 s: the heart stands
    call(&app, &ann, "PUT", "/v1/bookmarks", heart(SONG_B, 200)).await;
    let (_, v) = call(&app, &ann, "PUT", "/v1/bookmarks", heart(SONG_B, 100)).await;
    assert_eq!(v["bookmarked"], true, "{v}");
    let (_, v) = call(&app, &ann, "DELETE", &clear(SONG_B, 150), None).await;
    assert_eq!(v["bookmarked"], true, "the newest is the heart at 200 s: {v}");
    assert_eq!(fake.at(ANN, SONG_B), Some(200_000_000), "its time not moved back");
    // The other way round, while her homeserver is away: a clear at 200 s, a repeat from 100 s,
    // a heart from 150 s: it stays cleared, and only the clear waits
    call(&app, &ann, "PUT", "/v1/bookmarks", heart(SONG_C, 50)).await;
    fake.down.store(true, Ordering::SeqCst);
    call(&app, &ann, "DELETE", &clear(SONG_C, 200), None).await;
    let (_, v) = call(&app, &ann, "DELETE", &clear(SONG_C, 100), None).await;
    assert_eq!(v["bookmarked"], false, "{v}");
    let (_, v) = call(&app, &ann, "PUT", "/v1/bookmarks", heart(SONG_C, 150)).await;
    assert_eq!(v["bookmarked"], false, "the newest is the clear at 200 s: {v}");
    let (_, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert_eq!(v["homeserver"]["pending"], 1, "{v}");
    fake.down.store(false, Ordering::SeqCst);
    let (_, v) = call(&app, &ann, "GET", "/v1/bookmarks", None).await;
    assert_eq!(uris(&v), vec![SONG_B], "{v}");
    assert_eq!(fake.of(ANN), vec![SONG_B]);
}

#[tokio::test]
async fn a_disconnect_that_couldnt_forget_the_grant_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    fake.disconnect_fails.store(true, Ordering::SeqCst);
    let (st, v) = call(&app, &ann, "DELETE", "/v1/me/homeserver", None).await;
    assert_eq!(
        st,
        StatusCode::INTERNAL_SERVER_ERROR,
        "never answered as disconnected: {v}"
    );
    let (_, v) = call(&app, &ann, "GET", "/v1/me/homeserver", None).await;
    assert_eq!(v["connected"], true, "{v}");
}

#[tokio::test]
async fn a_removal_whose_token_write_fails_still_forgets_her_grant_and_copy() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    call(
        &app,
        &ann,
        "PUT",
        "/v1/bookmarks",
        Some(serde_json::json!({"uri": SONG_B})),
    )
    .await;
    let copy = format!("person.{ANN}.bookmarks");
    assert!(s.library.lock().unwrap().get_setting(&copy).unwrap().is_some());
    // Her preferences, kept on the node: they go too
    let prefs = format!("me.prefs.{ANN}");
    s.library.lock().unwrap().set_setting(&prefs, "{}").unwrap();
    // People › Remove takes her off the roster first
    let roster = serde_json::json!({"people": [
        {"key": BO, "access": "music", "music_enabled": true, "added_at": 0}]});
    s.library
        .lock()
        .unwrap()
        .set_setting("people", &roster.to_string())
        .unwrap();
    // The library refuses the token list from here on
    let db = rusqlite::Connection::open(dir.path().join("lib.sqlite")).unwrap();
    db.execute_batch(
        "CREATE TRIGGER no_tokens_u BEFORE UPDATE ON settings WHEN NEW.key = 'accounts.tokens'
           BEGIN SELECT RAISE(ABORT, 'test: no token writes'); END;
         CREATE TRIGGER no_tokens_i BEFORE INSERT ON settings WHEN NEW.key = 'accounts.tokens'
           BEGIN SELECT RAISE(ABORT, 'test: no token writes'); END;",
    )
    .unwrap();
    assert!(
        pm_api::accounts::forget_person(&s, ANN).is_err(),
        "the token write failed"
    );
    assert!(
        s.library.lock().unwrap().get_setting(&copy).unwrap().is_none(),
        "her copy went all the same"
    );
    for _ in 0..100 {
        if !fake.connected.lock().unwrap().contains(ANN) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(!fake.connected.lock().unwrap().contains(ANN), "and her grant");
    assert!(
        s.library.lock().unwrap().get_setting(&prefs).unwrap().is_none(),
        "and the rest of what the node keeps for her, after the failed write too"
    );
    // Her sessions are still listed (the write failed): removing her again signs them out
    assert!(pm_api::accounts::sessions_of(&s, ANN) > 0);
    db.execute_batch("DROP TRIGGER no_tokens_u; DROP TRIGGER no_tokens_i;")
        .unwrap();
    let (st, v) = call(&app, OWNER, "DELETE", &format!("/v1/people/{ANN}"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(pm_api::accounts::sessions_of(&s, ANN), 0, "her sessions are gone");
}

#[cfg(unix)]
#[tokio::test]
async fn a_removal_that_cant_delete_her_grant_says_so_and_can_be_done_again() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, _ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    // Her grant, as the node keeps it (next to the library), on a folder it can't change
    let grants = dir.path().join("grants");
    std::fs::create_dir_all(&grants).unwrap();
    let file = grants.join(format!("{ANN}.session"));
    std::fs::write(&file, "fixture-secret").unwrap();
    let prefs = format!("me.prefs.{ANN}");
    s.library.lock().unwrap().set_setting(&prefs, "{}").unwrap();
    std::fs::set_permissions(&grants, std::fs::Permissions::from_mode(0o500)).unwrap();
    let (st, v) = call(&app, OWNER, "DELETE", &format!("/v1/people/{ANN}"), None).await;
    let stuck = file.exists();
    std::fs::set_permissions(&grants, std::fs::Permissions::from_mode(0o700)).unwrap();
    let listed = |s: &pm_api::NodeState| {
        s.library
            .lock()
            .unwrap()
            .get_setting("people")
            .unwrap()
            .unwrap_or_default()
            .contains(ANN)
    };
    // (As root the folder can't refuse: nothing to show there)
    if stuck {
        assert_eq!(
            st,
            StatusCode::INTERNAL_SERVER_ERROR,
            "never a success with her grant left: {v}"
        );
        assert!(!listed(&s), "she is off the roster all the same");
        assert!(
            s.library.lock().unwrap().get_setting(&prefs).unwrap().is_none(),
            "the rest of what the node keeps for her went before the error was answered"
        );
        // Removing her again finishes it
        let (st, v) = call(&app, OWNER, "DELETE", &format!("/v1/people/{ANN}"), None).await;
        assert_eq!(st, StatusCode::OK, "{v}");
    } else {
        assert_eq!(st, StatusCode::OK, "{v}");
    }
    assert!(!file.exists(), "her grant is gone");
    assert!(!listed(&s));
    // Nothing left: a third removal is a plain "no such person"
    let (st, _) = call(&app, OWNER, "DELETE", &format!("/v1/people/{ANN}"), None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}
