//! A listener's own collections (TASK-listener-personal-library-1104, ADR 0016 step 5): social
//! v1 collections on their own homeserver, through the session they gave the node, never the
//! owner's. The listener's homeserver is a fake here (`pm_api::personal::PersonalHomeserver`).

mod personal_fake;

use axum::http::StatusCode;
use personal_fake::*;
use pm_api::router;
use std::sync::atomic::Ordering;

const DRAFTS: &str = "/priv/social/v1/posts/";
const PUBLISHED: &str = "/pub/social/v1/posts/";

/// The node's own collections, as its tables hold them (what the owner's sync pushes).
fn owners_collections(s: &pm_api::NodeState) -> Vec<String> {
    s.library
        .lock()
        .unwrap()
        .list_music_posts(None)
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect()
}

fn names(v: &serde_json::Value) -> Vec<String> {
    let mut n: Vec<String> = v
        .as_array()
        .unwrap_or_else(|| panic!("a list: {v}"))
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect();
    n.sort();
    n
}

/// The newest private version of `id` on `key`'s homeserver: its path and the collection in it.
fn newest(fake: &Fake, key: &str, id: &str) -> (String, serde_json::Value) {
    let path = fake
        .paths(key, &format!("{DRAFTS}{id}/"))
        .into_iter()
        .max()
        .unwrap_or_else(|| panic!("no version of {id} for {key}"));
    let post = fake.json(key, &path);
    assert_eq!(post["kind"], "collection", "{post}");
    let content: serde_json::Value = serde_json::from_str(post["content"].as_str().unwrap()).unwrap();
    (path, content)
}

fn item_uris(content: &serde_json::Value) -> Vec<String> {
    content["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["uri"].as_str().unwrap().to_string())
        .collect()
}

async fn create(
    app: &axum::Router,
    token: &str,
    name: &str,
    uris: &[&str],
) -> (StatusCode, serde_json::Value) {
    call(
        app,
        token,
        "POST",
        "/v1/collections",
        Some(serde_json::json!({"name": name, "uris": uris})),
    )
    .await
}

#[tokio::test]
async fn each_person_keeps_their_own_collections() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, bo) = node(dir.path());
    fake.connected
        .lock()
        .unwrap()
        .extend([ANN.to_string(), BO.to_string()]);
    let app = router(s.clone());

    let (st, owners) = create(&app, OWNER, "The owner's", &[SONG_A]).await;
    assert_eq!(st, StatusCode::OK, "{owners}");
    let owners_id = owners["summary"]["id"].as_str().unwrap().to_string();
    let (st, anns) = create(&app, &ann, "Ann's", &[SONG_B]).await;
    assert_eq!(st, StatusCode::OK, "{anns}");
    assert_eq!(anns["summary"]["synced"], true, "{anns}");
    let anns_id = anns["summary"]["id"].as_str().unwrap().to_string();
    let (st, v) = call(
        &app,
        &bo,
        "POST",
        "/v1/playlists",
        Some(serde_json::json!({"name": "Bo's", "uris": [SONG_C]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "the older name answers the same: {v}");

    // Each lists their own, and only that
    let (_, v) = call(&app, OWNER, "GET", "/v1/collections", None).await;
    assert_eq!(names(&v), vec!["The owner's"], "{v}");
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(names(&v), vec!["Ann's"], "{v}");
    let (_, v) = call(&app, &bo, "GET", "/v1/playlists", None).await;
    assert_eq!(names(&v), vec!["Bo's"], "{v}");

    // Ann's is on her homeserver, a private draft, as the owner's sync writes the owner's
    let (_, content) = newest(&fake, ANN, &anns_id);
    assert_eq!(content["name"], "Ann's");
    assert_eq!(item_uris(&content), vec![SONG_B]);
    assert!(
        fake.paths(ANN, PUBLISHED).is_empty(),
        "private until she publishes it"
    );
    assert!(
        fake.paths(BO, DRAFTS).iter().all(|p| !p.contains(&anns_id)),
        "nothing of hers on Bo's"
    );
    // The node's tables hold the owner's only: nothing for the owner's sync to push of theirs
    assert_eq!(owners_collections(&s), vec![owners_id.clone()]);

    // Nobody opens or changes someone else's
    for (token, id) in [(&ann, &owners_id), (&bo, &anns_id)] {
        let (st, v) = call(&app, token, "GET", &format!("/v1/collections/{id}"), None).await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{v}");
        let (st, v) = call(
            &app,
            token,
            "PUT",
            &format!("/v1/collections/{id}"),
            Some(serde_json::json!({"name": "mine now"})),
        )
        .await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{v}");
        let (st, _) = call(&app, token, "DELETE", &format!("/v1/collections/{id}"), None).await;
        assert_eq!(st, StatusCode::NOT_FOUND);
    }
    let (st, v) = call(&app, OWNER, "GET", &format!("/v1/collections/{anns_id}"), None).await;
    assert_eq!(
        st,
        StatusCode::NOT_FOUND,
        "the owner's are the node's, never hers: {v}"
    );
    let (_, v) = call(&app, &ann, "GET", &format!("/v1/collections/{anns_id}"), None).await;
    assert_eq!(v["playlist"]["name"], "Ann's", "{v}");
    assert_eq!(v["entries"][0]["uri"], SONG_B, "{v}");
}

#[tokio::test]
async fn a_collection_is_changed_published_and_deleted_on_her_homeserver() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, v) = create(&app, &ann, "Road trip", &[SONG_A]).await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();
    let at = |what: &str| format!("/v1/collections/{id}{what}");

    // Add at the front, then a new order, a new name, a song out: each a new version
    let (st, v) = call(
        &app,
        &ann,
        "POST",
        &at("/items"),
        Some(serde_json::json!({"uris": [SONG_B], "at": 0})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(item_uris(&newest(&fake, ANN, &id).1), vec![SONG_B, SONG_A]);
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        &at(""),
        Some(serde_json::json!({"name": "Road trip '26", "uris": [SONG_A, SONG_B, SONG_C]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (path, content) = newest(&fake, ANN, &id);
    assert_eq!(content["name"], "Road trip '26");
    assert_eq!(item_uris(&content), vec![SONG_A, SONG_B, SONG_C]);
    let (st, v) = call(&app, &ann, "DELETE", &at("/items/1"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(item_uris(&newest(&fake, ANN, &id).1), vec![SONG_A, SONG_C]);
    assert!(
        newest(&fake, ANN, &id).0 > path,
        "an edit is a new version above the last"
    );

    // Published: the same version copied under /pub/; private again: every published copy goes
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        &at("/visibility"),
        Some(serde_json::json!({"public": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["summary"]["visibility"], "public", "{v}");
    let published = fake.paths(ANN, &format!("{PUBLISHED}{id}/"));
    assert_eq!(published.len(), 1, "{published:?}");
    let (draft, _) = newest(&fake, ANN, &id);
    assert_eq!(
        published[0].rsplit('/').next(),
        draft.rsplit('/').next(),
        "the published copy is the newest draft's version"
    );
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(v[0]["visibility"], "public", "{v}");
    let (st, _) = call(
        &app,
        &ann,
        "PUT",
        &at("/visibility"),
        Some(serde_json::json!({"public": false})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(fake.paths(ANN, PUBLISHED).is_empty());
    assert!(
        !fake.paths(ANN, &format!("{DRAFTS}{id}/")).is_empty(),
        "the drafts stay"
    );

    // Deleted: every version, drafts and published copies
    call(
        &app,
        &ann,
        "PUT",
        &at("/visibility"),
        Some(serde_json::json!({"public": true})),
    )
    .await;
    let (st, _) = call(&app, &ann, "DELETE", &at(""), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert!(fake.paths(ANN, DRAFTS).is_empty() && fake.paths(ANN, PUBLISHED).is_empty());
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(v, serde_json::json!([]));
    assert!(owners_collections(&s).is_empty(), "never the owner's");
}

#[tokio::test]
async fn a_listener_without_their_homeserver_is_asked_to_connect() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    let app = router(s.clone());
    let (st, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v, serde_json::json!([]), "theirs aren't known here yet");
    let (st, v) = create(&app, &ann, "x", &[SONG_A]).await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["needs_connect"], true, "{v}");
    assert!(owners_collections(&s).is_empty(), "never the owner's instead");
    assert!(fake.calls.lock().unwrap().is_empty(), "{:?}", fake.calls);
}

#[tokio::test]
async fn a_refused_grant_is_lost_and_never_falls_back_to_the_owner() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, v) = create(&app, &ann, "Mine", &[SONG_A]).await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();
    // Revoked in Ring
    fake.refused.lock().unwrap().insert(ANN.to_string());
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        &format!("/v1/collections/{id}"),
        Some(serde_json::json!({"name": "Renamed"})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["lost"], true, "{v}");
    let (_, v) = call(&app, &ann, "GET", "/v1/me/homeserver", None).await;
    assert_eq!(v["lost"], true, "{v}");
    // Reads answer from the node's copy, as it was
    let (st, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(names(&v), vec!["Mine"], "{v}");
    let (st, v) = create(&app, &ann, "Another", &[SONG_B]).await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert!(owners_collections(&s).is_empty(), "never the owner's instead");
}

#[tokio::test]
async fn an_unreachable_homeserver_keeps_the_change_until_it_answers() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, v) = create(&app, &ann, "Before", &[SONG_A]).await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();

    fake.down.store(true, Ordering::SeqCst);
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        &format!("/v1/collections/{id}"),
        Some(serde_json::json!({"name": "After"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["summary"]["synced"], false, "not on her homeserver yet: {v}");
    let (st, v) = create(&app, &ann, "New while away", &[SONG_B]).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let new_id = v["summary"]["id"].as_str().unwrap().to_string();
    let (_, v) = call(&app, &ann, "GET", "/v1/me/homeserver", None).await;
    assert_eq!(v["pending"], 2, "{v}");
    // Her reads still answer, from the copy
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(names(&v), vec!["After", "New while away"], "{v}");
    assert_eq!(newest(&fake, ANN, &id).1["name"], "Before");

    // Back: the next read writes what waited
    fake.down.store(false, Ordering::SeqCst);
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(names(&v), vec!["After", "New while away"], "{v}");
    assert!(v.as_array().unwrap().iter().all(|c| c["synced"] == true), "{v}");
    assert_eq!(newest(&fake, ANN, &id).1["name"], "After");
    assert_eq!(newest(&fake, ANN, &new_id).1["name"], "New while away");
    let (_, v) = call(&app, &ann, "GET", "/v1/me/homeserver", None).await;
    assert_eq!(v["pending"], 0, "{v}");
    assert!(owners_collections(&s).is_empty());
}

#[tokio::test]
async fn changes_made_at_once_all_land() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, v) = create(&app, &ann, "Many hands", &[]).await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();
    let songs: Vec<String> = (0..6)
        .map(|i| format!("https://musicbrainz.org/recording/0c9b1d6e-4e5f-4f3a-9c11-5b7d0a6e2f4{i}"))
        .collect();
    // Six songs added at once (two devices, say), and a read meanwhile
    let mut all = Vec::new();
    for u in &songs {
        let (app, ann, u, id) = (app.clone(), ann.clone(), u.clone(), id.clone());
        all.push(tokio::spawn(async move {
            call(
                &app,
                &ann,
                "POST",
                &format!("/v1/collections/{id}/items"),
                Some(serde_json::json!({"uris": [u]})),
            )
            .await
        }));
    }
    let reading = {
        let (app, ann) = (app.clone(), ann.clone());
        tokio::spawn(async move { call(&app, &ann, "GET", "/v1/collections", None).await })
    };
    for h in all {
        let (st, v) = h.await.unwrap();
        assert_eq!(st, StatusCode::OK, "{v}");
    }
    assert_eq!(reading.await.unwrap().0, StatusCode::OK);
    let mut landed = item_uris(&newest(&fake, ANN, &id).1);
    landed.sort();
    assert_eq!(landed, songs, "every one, none lost to another");
}

#[tokio::test]
async fn her_other_devices_changes_come_in_and_one_deleted_there_goes() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, v) = create(&app, &ann, "From the node", &[SONG_A]).await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();

    // Another app of hers writes a newer version
    let (_, mut content) = newest(&fake, ANN, &id);
    content["name"] = "From her phone".into();
    let (draft, _) = newest(&fake, ANN, &id);
    let mut post = fake.json(ANN, &draft);
    post["content"] = content.to_string().into();
    let later = pm_pubky::v1::id_of(pm_pubky::ids::now_micros() + 5_000_000);
    let path = format!("{DRAFTS}{id}/{later}.json");
    fake.files
        .lock()
        .unwrap()
        .get_mut(ANN)
        .unwrap()
        .insert(path.clone(), serde_json::to_vec(&post).unwrap());
    // And a private note of hers (not a collection): never taken, and read only once
    let note_id = pm_pubky::v1::id_of(pm_pubky::ids::now_micros() - 9_000_000);
    let note = format!("{DRAFTS}{note_id}/{note_id}.json");
    fake.files.lock().unwrap().get_mut(ANN).unwrap().insert(
        note.clone(),
        serde_json::to_vec(&serde_json::json!({"kind": "short", "content": "a private thought"})).unwrap(),
    );

    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(names(&v), vec!["From her phone"], "{v}");
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(names(&v), vec!["From her phone"], "{v}");
    let reads = |p: &str| {
        fake.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.ends_with(&format!("read {p}")))
            .count()
    };
    assert_eq!(reads(&path), 1, "the new version is read once");
    assert_eq!(reads(&note), 1, "her note is looked at once, and left alone");

    // An edit here goes above her phone's version
    let (st, _) = call(
        &app,
        &ann,
        "PUT",
        &format!("/v1/collections/{id}"),
        Some(serde_json::json!({"description": "both"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (newest_path, content) = newest(&fake, ANN, &id);
    assert!(newest_path > path, "{newest_path} above {path}");
    assert_eq!(content["name"], "From her phone");

    // Deleted there: gone here too
    fake.files
        .lock()
        .unwrap()
        .get_mut(ANN)
        .unwrap()
        .retain(|p, _| !p.contains(&id));
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(v, serde_json::json!([]), "{v}");
    assert!(
        fake.paths(ANN, &note).len() == 1,
        "her note is still there, untouched"
    );
}

#[tokio::test]
async fn a_listener_reaches_only_their_own_collection_routes() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, owners) = create(&app, OWNER, "The owner's", &[SONG_A]).await;
    let oid = owners["summary"]["id"].as_str().unwrap().to_string();
    let (_, v) = create(&app, &ann, "Ann's", &[SONG_B]).await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();
    for c in [&oid, &id] {
        for (method, path, body) in [
            ("GET", format!("/v1/collections/{c}/play-from"), None),
            (
                "PUT",
                format!("/v1/collections/{c}/order"),
                Some(serde_json::json!({"order": null})),
            ),
            (
                "PUT",
                format!("/v1/collections/{c}/items/0/source"),
                Some(serde_json::json!({"service": "auto"})),
            ),
            ("DELETE", format!("/v1/playlists/{c}/local/0"), None),
            (
                "POST",
                "/v1/playlists/from-queue".to_string(),
                Some(serde_json::json!({"uris": [SONG_C]})),
            ),
            (
                "POST",
                "/v1/playlists/import".to_string(),
                Some(serde_json::json!({"url": "https://example.com/x"})),
            ),
        ] {
            let (st, v) = call(&app, &ann, method, &path, body).await;
            assert_eq!(st, StatusCode::FORBIDDEN, "{method} {path}: {v}");
        }
    }
    // Playing: only on a zone the owner opened to listeners, and only hers
    let (st, _) = call(
        &app,
        OWNER,
        "PUT",
        "/v1/people/zones",
        Some(serde_json::json!({"zones": ["kitchen"]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, v) = call(
        &app,
        &ann,
        "POST",
        &format!("/v1/collections/{id}/play"),
        Some(serde_json::json!({"zone": "office"})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
    let (st, v) = call(
        &app,
        &ann,
        "POST",
        &format!("/v1/collections/{oid}/play"),
        Some(serde_json::json!({"zone": "kitchen"})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND, "{v}");
    assert_eq!(owners_collections(&s), vec![oid], "the owner's untouched");
}

#[tokio::test]
async fn a_song_not_identified_yet_is_no_listener_collection_item() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    // A library song without a MusicBrainz id or ISRC yet
    let track: pm_model::Track = serde_json::from_value(serde_json::json!({
        "id": null, "title": "Untagged", "artists": [{"id": null, "name": "X", "mbid": null}],
        "album": {"id": null, "title": "No Id", "album_artist": "X", "release_mbid": null,
                  "release_group_mbid": null, "year": null, "upc": null, "total_discs": null},
        "track_no": 1, "disc_no": null, "duration_ms": 1000,
        "file": {"path": dir.path().join("A.flac"), "size_bytes": 1, "mtime_unix": 0, "container": "flac",
                 "format": serde_json::to_value(pm_model::AudioFormat::pcm(44_100, 16, 2)).unwrap(), "duration_ms": 1000},
        "recording_mbid": null, "isrc": null, "acoustid": null, "canonical": null,
        "replaygain_track_gain_db": null, "replaygain_album_gain_db": null}))
    .unwrap();
    let track = s.library.lock().unwrap().upsert_track(&track).unwrap();
    let (st, v) = call(
        &app,
        &ann,
        "POST",
        "/v1/collections",
        Some(serde_json::json!({"name": "x", "track_ids": [track.0]})),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(fake.paths(ANN, DRAFTS).is_empty(), "nothing written");
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(v, serde_json::json!([]));
    assert!(
        s.library
            .lock()
            .unwrap()
            .settings_with_prefix("playlist.local.")
            .unwrap()
            .is_empty(),
        "nor in the owner's node-only songs"
    );
}

#[tokio::test]
async fn a_removed_listener_leaves_no_collections_and_disconnecting_keeps_hers() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, v) = create(&app, &ann, "Kept", &[SONG_A]).await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();
    let (st, _) = call(&app, &ann, "DELETE", "/v1/me/homeserver", None).await;
    assert_eq!(st, StatusCode::OK);
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(v, serde_json::json!([]), "not connected: none known here");
    assert_eq!(
        newest(&fake, ANN, &id).1["name"],
        "Kept",
        "her homeserver keeps it"
    );
    // Connected again: hers again
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(names(&v), vec!["Kept"], "{v}");
    pm_api::accounts::forget_person(&s, ANN).await.unwrap();
    let key = format!("person.{ANN}.collections");
    assert!(
        s.library.lock().unwrap().get_setting(&key).unwrap().is_none(),
        "People › Remove leaves no copy of hers"
    );
}

#[tokio::test]
async fn a_change_their_homeserver_wont_take_is_read_again_from_there() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, v) = create(&app, &ann, "Kept name", &[SONG_A]).await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();
    fake.down.store(true, Ordering::SeqCst);
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        &format!("/v1/collections/{id}"),
        Some(serde_json::json!({"name": "Never taken"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    // Back, but it won't take that version: the node shows what it has, not the change as synced
    fake.down.store(false, Ordering::SeqCst);
    fake.rejects.lock().unwrap().insert(ANN.to_string());
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(names(&v), vec!["Kept name"], "{v}");
    assert_eq!(v[0]["synced"], true, "{v}");
    assert_eq!(newest(&fake, ANN, &id).1["name"], "Kept name");
}

#[tokio::test]
async fn a_change_half_written_is_kept_and_never_made_twice() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    // Its draft lands, its published copy is refused: kept, waiting, not a 409 to send it again
    fake.refuse_pub.store(true, Ordering::SeqCst);
    let (st, v) = call(
        &app,
        &ann,
        "POST",
        "/v1/collections",
        Some(serde_json::json!({"name": "Shared", "uris": [SONG_A], "public": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["summary"]["synced"], false, "{v}");
    let (_, me) = call(&app, &ann, "GET", "/v1/me/homeserver", None).await;
    assert_eq!(me["lost"], true, "{me}");
    let ids: std::collections::BTreeSet<String> = fake
        .paths(ANN, DRAFTS)
        .iter()
        .map(|p| {
            p.trim_start_matches(DRAFTS)
                .split('/')
                .next()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(ids.len(), 1, "one collection, never two: {ids:?}");
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(names(&v), vec!["Shared"], "{v}");
}

#[tokio::test]
async fn a_delete_waiting_removes_every_version_even_another_devices() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, v) = call(
        &app,
        &ann,
        "POST",
        "/v1/collections",
        Some(serde_json::json!({"name": "Going", "uris": [SONG_A], "public": true})),
    )
    .await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();
    fake.down.store(true, Ordering::SeqCst);
    let (st, _) = call(&app, &ann, "DELETE", &format!("/v1/collections/{id}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    // Meanwhile another app of hers writes a newer version, drafted and published
    let (draft, _) = newest(&fake, ANN, &id);
    let body = fake.json(ANN, &draft);
    let later = pm_pubky::v1::id_of(pm_pubky::ids::now_micros() + 5_000_000);
    for root in [DRAFTS, PUBLISHED] {
        fake.files.lock().unwrap().get_mut(ANN).unwrap().insert(
            format!("{root}{id}/{later}.json"),
            serde_json::to_vec(&body).unwrap(),
        );
    }
    fake.down.store(false, Ordering::SeqCst);
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(v, serde_json::json!([]), "{v}");
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(v, serde_json::json!([]), "{v}");
    assert!(
        fake.paths(ANN, DRAFTS).is_empty() && fake.paths(ANN, PUBLISHED).is_empty(),
        "every version went, the other app's too"
    );
}

/// Another app of hers writes a newer version of `id` (drafted, and published when `published`),
/// its content changed by `edit`, without the node reading it. Its draft's path.
fn another_app_writes(
    fake: &Fake,
    id: &str,
    published: bool,
    edit: impl FnOnce(&mut serde_json::Value),
) -> String {
    let (draft, mut content) = newest(fake, ANN, id);
    edit(&mut content);
    let mut post = fake.json(ANN, &draft);
    post["content"] = content.to_string().into();
    let later = pm_pubky::v1::id_of(pm_pubky::ids::now_micros() + 5_000_000);
    let roots: &[&str] = if published {
        &[DRAFTS, PUBLISHED]
    } else {
        &[DRAFTS]
    };
    for root in roots {
        fake.files.lock().unwrap().get_mut(ANN).unwrap().insert(
            format!("{root}{id}/{later}.json"),
            serde_json::to_vec(&post).unwrap(),
        );
    }
    format!("{DRAFTS}{id}/{later}.json")
}

#[tokio::test]
async fn a_change_made_at_once_starts_from_her_other_apps_newer_version() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, v) = create(&app, &ann, "Mix", &[SONG_A]).await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();
    // Her phone adds a song; the node hasn't read it when she renames here
    let phone = another_app_writes(&fake, &id, false, |c| {
        let mut it = c["items"][0].clone();
        it["uri"] = SONG_B.into();
        c["items"].as_array_mut().unwrap().push(it);
    });
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        &format!("/v1/collections/{id}"),
        Some(serde_json::json!({"name": "Mix 2"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (path, content) = newest(&fake, ANN, &id);
    assert!(path > phone, "{path} above her phone's {phone}");
    assert_eq!(content["name"], "Mix 2");
    assert_eq!(
        item_uris(&content),
        vec![SONG_A, SONG_B],
        "her phone's song stays: {content}"
    );
}

#[tokio::test]
async fn a_delete_made_at_once_removes_her_other_apps_newer_version() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, v) = call(
        &app,
        &ann,
        "POST",
        "/v1/collections",
        Some(serde_json::json!({"name": "Going", "uris": [SONG_A], "public": true})),
    )
    .await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();
    another_app_writes(&fake, &id, true, |c| c["name"] = "Still here?".into());
    let (st, _) = call(&app, &ann, "DELETE", &format!("/v1/collections/{id}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert!(
        fake.paths(ANN, DRAFTS).is_empty() && fake.paths(ANN, PUBLISHED).is_empty(),
        "every version went, the other app's newer one too"
    );
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(v, serde_json::json!([]), "nothing comes back: {v}");
}

#[tokio::test]
async fn private_again_at_once_unpublishes_her_other_apps_newer_copy() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, v) = call(
        &app,
        &ann,
        "POST",
        "/v1/collections",
        Some(serde_json::json!({"name": "Shared", "uris": [SONG_A], "public": true})),
    )
    .await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();
    another_app_writes(&fake, &id, true, |_| {});
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        &format!("/v1/collections/{id}/visibility"),
        Some(serde_json::json!({"public": false})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        (v["summary"]["visibility"].clone(), v["summary"]["synced"].clone()),
        (serde_json::json!("private"), serde_json::json!(true)),
        "{v}"
    );
    assert!(
        fake.paths(ANN, PUBLISHED).is_empty(),
        "no public copy is left, the other app's newer one neither"
    );
}

#[tokio::test]
async fn her_homeserver_cant_make_the_node_keep_more_than_a_thousand() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, v) = create(&app, &ann, "One", &[SONG_A]).await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();
    // 999 more on her homeserver, already kept by the node (same versions, nothing to read)
    let key = format!("person.{ANN}.collections");
    let lib_shelf = || -> serde_json::Value {
        serde_json::from_str(&s.library.lock().unwrap().get_setting(&key).unwrap().unwrap()).unwrap()
    };
    let mut shelf = lib_shelf();
    let one = shelf["items"][0].clone();
    let (draft, _) = newest(&fake, ANN, &id);
    let body = fake.files.lock().unwrap()[ANN][&draft].clone();
    let base = pm_pubky::ids::now_micros() - 60_000_000_000;
    for i in 0..999 {
        let cid = pm_pubky::v1::id_of(base + i * 1_000);
        let path = format!("{DRAFTS}{cid}/{cid}.json");
        fake.files
            .lock()
            .unwrap()
            .get_mut(ANN)
            .unwrap()
            .insert(path.clone(), body.clone());
        let mut k = one.clone();
        k["id"] = cid.clone().into();
        k["head"] = cid.into();
        k["versions"] = serde_json::json!([path]);
        shelf["items"].as_array_mut().unwrap().push(k);
    }
    s.library
        .lock()
        .unwrap()
        .set_setting(&key, &shelf.to_string())
        .unwrap();
    // One more from her homeserver: not taken; one more here: refused
    let late = pm_pubky::v1::id_of(pm_pubky::ids::now_micros() + 5_000_000);
    fake.files
        .lock()
        .unwrap()
        .get_mut(ANN)
        .unwrap()
        .insert(format!("{DRAFTS}{late}/{late}.json"), body.clone());
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(
        v.as_array().map(Vec::len),
        Some(1_000),
        "the node keeps a thousand"
    );
    assert!(
        !fake
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.ends_with(&format!("read {DRAFTS}{late}/{late}.json"))),
        "the one over isn't even downloaded"
    );
    let (st, v) = create(&app, &ann, "Too many", &[SONG_A]).await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert_eq!(lib_shelf()["items"].as_array().unwrap().len(), 1_000);
}

#[tokio::test]
async fn a_change_waits_when_her_homeserver_cant_be_read_first() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, v) = create(&app, &ann, "Mix", &[SONG_A]).await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();
    let phone = another_app_writes(&fake, &id, false, |c| c["name"] = "From her phone".into());
    // Her folder can't be listed whole: the node can't see her phone's version, so it writes nothing
    fake.list_fails.store(true, Ordering::SeqCst);
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        &format!("/v1/collections/{id}"),
        Some(serde_json::json!({"name": "Mix 2"})),
    )
    .await;
    assert_eq!(
        (st, v["summary"]["synced"].clone()),
        (StatusCode::OK, serde_json::json!(false)),
        "{v}"
    );
    assert_eq!(
        newest(&fake, ANN, &id).0,
        phone,
        "nothing written over a version it couldn't see"
    );
    // Listed again: the change goes, above her phone's version
    fake.list_fails.store(false, Ordering::SeqCst);
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(names(&v), vec!["Mix 2"], "{v}");
    assert!(newest(&fake, ANN, &id).0 > phone);

    // A delete the same way: it waits, then every version goes, her phone's too
    let (_, v) = call(
        &app,
        &ann,
        "POST",
        "/v1/collections",
        Some(serde_json::json!({"name": "Going", "uris": [SONG_A], "public": true})),
    )
    .await;
    let gone = v["summary"]["id"].as_str().unwrap().to_string();
    another_app_writes(&fake, &gone, true, |_| {});
    fake.list_fails.store(true, Ordering::SeqCst);
    let (st, _) = call(&app, &ann, "DELETE", &format!("/v1/collections/{gone}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert!(
        !fake.paths(ANN, &format!("{DRAFTS}{gone}/")).is_empty(),
        "nothing removed blind"
    );
    fake.list_fails.store(false, Ordering::SeqCst);
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(names(&v), vec!["Mix 2"], "{v}");
    assert!(
        fake.paths(ANN, &format!("{DRAFTS}{gone}/")).is_empty()
            && fake.paths(ANN, &format!("{PUBLISHED}{gone}/")).is_empty(),
        "every version went"
    );
}

#[tokio::test]
async fn her_other_apps_version_the_node_cant_hold_is_followed_never_written_over() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, v) = call(
        &app,
        &ann,
        "POST",
        "/v1/collections",
        Some(serde_json::json!({"name": "Shared", "uris": [SONG_A], "public": true})),
    )
    .await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();
    // Another app writes a version the node can't hold (a name far over its limit); the node
    // looks at it once and leaves it aside
    let other = another_app_writes(&fake, &id, false, |c| c["name"] = "x".repeat(300).into());
    call(&app, &ann, "GET", "/v1/collections", None).await;
    // Then that app publishes it: the node still follows that version
    let edit = other.trim_start_matches(&format!("{DRAFTS}{id}/")).to_string();
    let body = fake.files.lock().unwrap()[ANN][&other].clone();
    fake.files
        .lock()
        .unwrap()
        .get_mut(ANN)
        .unwrap()
        .insert(format!("{PUBLISHED}{id}/{edit}"), body);
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        &format!("/v1/collections/{id}/visibility"),
        Some(serde_json::json!({"public": false})),
    )
    .await;
    // Changing it here would write over that version unread: refused, nothing written
    assert_eq!(st, StatusCode::PAYLOAD_TOO_LARGE, "{v}");
    assert_eq!(
        newest(&fake, ANN, &id).0,
        other,
        "the other app's version stays the newest"
    );
    // A delete reaches every version, the one it couldn't hold too
    let (st, _) = call(&app, &ann, "DELETE", &format!("/v1/collections/{id}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert!(
        fake.paths(ANN, DRAFTS).is_empty() && fake.paths(ANN, PUBLISHED).is_empty(),
        "every version went, the one it couldn't hold and its public copy too"
    );
}

#[tokio::test]
async fn a_version_that_can_never_be_read_doesnt_hold_up_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, v) = create(&app, &ann, "Here", &[SONG_A]).await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();
    let (draft, content) = newest(&fake, ANN, &id);
    let post = fake.json(ANN, &draft);
    // Two new ones from another app: the newer can never be read (too big), the older can
    let put = |name: &str, at: i64| -> String {
        let cid = pm_pubky::v1::id_of(at);
        let mut c = content.clone();
        c["name"] = name.into();
        let mut p = post.clone();
        p["content"] = c.to_string().into();
        let path = format!("{DRAFTS}{cid}/{cid}.json");
        fake.files
            .lock()
            .unwrap()
            .get_mut(ANN)
            .unwrap()
            .insert(path.clone(), serde_json::to_vec(&p).unwrap());
        path
    };
    let now = pm_pubky::ids::now_micros();
    let big = put("Too big", now + 9_000_000);
    put("Older", now + 5_000_000);
    fake.unreadable.lock().unwrap().insert(big.clone());
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(
        names(&v),
        vec!["Here", "Older"],
        "the one after it still came in: {v}"
    );
    call(&app, &ann, "GET", "/v1/collections", None).await;
    let tries = fake
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c.ends_with(&format!("read {big}")))
        .count();
    assert_eq!(tries, 1, "and it isn't tried again on every read");
}

#[tokio::test]
async fn a_collection_rejected_by_post_validation_is_never_written_or_kept() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    // About 1.1 MB of links exceeds the Post validator first, before the node's 1 MiB read
    // limit. Assert that actual refusal, including its reason, and that nothing is published.
    let uris: Vec<String> = (0..1_100)
        .map(|i| format!("https://example.com/{i}/{}", "a".repeat(990)))
        .collect();
    let (st, v) = call(
        &app,
        &ann,
        "POST",
        "/v1/collections",
        Some(serde_json::json!({"name": "Huge", "uris": uris})),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    let why = v["error"].as_str().unwrap_or_default();
    assert!(
        why.contains("Post content exceeds maximum length") && why.contains("50000"),
        "{v}"
    );
    assert!(fake.paths(ANN, DRAFTS).is_empty(), "nothing written");
    assert!(fake.paths(ANN, PUBLISHED).is_empty(), "nothing published");
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(v, serde_json::json!([]), "nor kept: {v}");
}

#[tokio::test]
async fn her_collection_plays_on_a_zone_open_to_her_and_nobody_elses_does() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, bo, track) = node_with_zone(dir.path()).await;
    for k in [ANN, BO] {
        fake.connected.lock().unwrap().insert(k.to_string());
    }
    let app = router(s.clone());
    let (st, v) = call(
        &app,
        &ann,
        "POST",
        "/v1/collections",
        Some(serde_json::json!({"name": "Hers", "uris": [SONG_A, RELEASE_A, "https://example.com/notes"]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let id = v["summary"]["id"].as_str().unwrap().to_string();
    let (st, v) = call(
        &app,
        OWNER,
        "POST",
        "/v1/collections",
        Some(serde_json::json!({"name": "The owner's", "uris": [SONG_A]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let oid = v["summary"]["id"].as_str().unwrap().to_string();
    let play = async |who: &str, id: &str| {
        call(
            &app,
            who,
            "POST",
            &format!("/v1/collections/{id}/play"),
            Some(serde_json::json!({"zone": "null-output", "replace": true})),
        )
        .await
    };
    // Hers: the song and her album's song queue, in order; the web page doesn't play
    let (st, v) = play(&ann, &id).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["queued"], 2, "{v}");
    assert_eq!(
        v["missing"],
        serde_json::json!(["https://example.com/notes"]),
        "{v}"
    );
    let (_, q) = call(&app, OWNER, "GET", "/v1/zones/null-output/queue", None).await;
    let queued: Vec<String> = q
        .as_array()
        .unwrap_or_else(|| panic!("a queue: {q}"))
        .iter()
        .map(|i| i["uri"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(queued, vec![format!("track:{track}"), format!("track:{track}")]);
    // With that zone there: Bo can't play hers, and she can't play the owner's
    let (st, v) = play(&bo, &id).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "{v}");
    let (st, v) = play(&ann, &oid).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "{v}");
    let (_, q) = call(&app, OWNER, "GET", "/v1/zones/null-output/queue", None).await;
    assert_eq!(
        q.as_array().map(Vec::len),
        Some(2),
        "nothing else was queued: {q}"
    );
}

#[tokio::test]
async fn changes_go_one_collection_at_a_time_when_all_her_posts_cant_be_listed() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, v) = create(&app, &ann, "Mix", &[SONG_A]).await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();
    let phone = another_app_writes(&fake, &id, false, |c| {
        let mut it = c["items"][0].clone();
        it["uri"] = SONG_B.into();
        c["items"].as_array_mut().unwrap().push(it);
    });
    // All of her posts are more than the node lists; this collection's own folder still lists
    fake.whole_list_fails.store(true, Ordering::SeqCst);
    let rename = |name: &str| Some(serde_json::json!({"name": name}));
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        &format!("/v1/collections/{id}"),
        rename("Mix 2"),
    )
    .await;
    assert_eq!(
        (st, v["summary"]["synced"].clone()),
        (StatusCode::OK, serde_json::json!(true)),
        "{v}"
    );
    let (path, content) = newest(&fake, ANN, &id);
    assert!(path > phone, "{path} above her phone's {phone}");
    assert_eq!(content["name"], "Mix 2");
    assert_eq!(
        item_uris(&content),
        vec![SONG_A, SONG_B],
        "caught up from its own folder first: {content}"
    );
    let (_, me) = call(&app, &ann, "GET", "/v1/me/homeserver", None).await;
    assert_eq!(
        me["stale"], true,
        "the page can say the node couldn't read everything: {me}"
    );
    // A change that waited (her homeserver away) still goes at the next read
    fake.down.store(true, Ordering::SeqCst);
    let (_, v) = call(
        &app,
        &ann,
        "PUT",
        &format!("/v1/collections/{id}"),
        rename("Mix 3"),
    )
    .await;
    assert_eq!(v["summary"]["synced"], false, "{v}");
    fake.down.store(false, Ordering::SeqCst);
    call(&app, &ann, "GET", "/v1/collections", None).await;
    let (_, me) = call(&app, &ann, "GET", "/v1/me/homeserver", None).await;
    assert_eq!(me["pending"], 0, "{me}");
    assert_eq!(newest(&fake, ANN, &id).1["name"], "Mix 3");
    // And a delete goes, every version
    let (st, _) = call(&app, &ann, "DELETE", &format!("/v1/collections/{id}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert!(fake.paths(ANN, &format!("{DRAFTS}{id}/")).is_empty());
}

#[tokio::test]
async fn a_version_the_node_cant_keep_is_never_written_over() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, v) = create(&app, &ann, "Small", &[SONG_A]).await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();
    // Another collection of hers, already on the node, brings its copy close to the 32 MiB cap
    let key = format!("person.{ANN}.collections");
    let shelf = || -> serde_json::Value {
        serde_json::from_str(&s.library.lock().unwrap().get_setting(&key).unwrap().unwrap()).unwrap()
    };
    let bid = pm_pubky::v1::id_of(pm_pubky::ids::now_micros() - 60_000_000_000);
    let set_ballast = |bytes: usize| {
        let mut sh = shelf();
        let items = sh["items"].as_array_mut().unwrap();
        items.retain(|k| k["id"] != bid.as_str());
        let mut ballast = items[0].clone();
        let path = format!("{DRAFTS}{bid}/{bid}.json");
        let (draft, _) = newest(&fake, ANN, &id);
        let body = fake.files.lock().unwrap()[ANN][&draft].clone();
        fake.files
            .lock()
            .unwrap()
            .get_mut(ANN)
            .unwrap()
            .insert(path.clone(), body);
        ballast["id"] = bid.clone().into();
        ballast["head"] = bid.clone().into();
        ballast["versions"] = serde_json::json!([path]);
        ballast["playlist"]["description"] = "x".repeat(bytes).into();
        items.push(ballast);
        s.library
            .lock()
            .unwrap()
            .set_setting(&key, &sh.to_string())
            .unwrap();
    };
    const CAP: usize = 32 * 1024 * 1024;
    set_ballast(CAP - 15_000);
    assert!(
        serde_json::to_vec(&shelf()).unwrap().len() < CAP,
        "the existing shelf still fits"
    );
    // A valid Post below the content and per-object limits, but too big for the shelf's
    // remaining room. An invalid oversized Post would test Unreadable instead of Held::Size.
    let phone = another_app_writes(&fake, &id, false, |c| {
        let one = c["items"][0].clone();
        for i in 0..25 {
            let mut it = one.clone();
            it["uri"] = format!("https://example.com/{i}/{}", "a".repeat(990)).into();
            c["items"].as_array_mut().unwrap().push(it);
        }
    });
    // And a new one of hers, as big: not kept either
    let yid = pm_pubky::v1::id_of(pm_pubky::ids::now_micros() + 7_000_000);
    let big = fake.files.lock().unwrap()[ANN][&phone].clone();
    let post: pubky_social_specs::PubkySocialPost = serde_json::from_slice(&big).unwrap();
    pm_pubky::v1::validate_social(
        &post,
        &id,
        pm_pubky::v1::Root::Priv,
        &pm_pubky::v1::PubkyId::try_from(ANN).unwrap(),
    )
    .expect("the phone fixture must pass the same Post validation as a real collection");
    let (_, edit_id) = pm_pubky::v1::parse_version(&phone).unwrap();
    let content = pm_pubky::v1::from_social(&post, &id, &edit_id).unwrap();
    pm_pubky::MusicPost::new(content)
        .validate(Some(&id))
        .expect("the phone fixture must also pass the node's collection checks");
    assert!(big.len() < pm_pubky::homeserver::OBJECT_MAX);
    fake.files
        .lock()
        .unwrap()
        .get_mut(ANN)
        .unwrap()
        .insert(format!("{DRAFTS}{yid}/{yid}.json"), big);
    let files_before = fake.files.lock().unwrap()[ANN].clone();
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        &format!("/v1/collections/{id}"),
        Some(serde_json::json!({"name": "Renamed"})),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::PAYLOAD_TOO_LARGE,
        "refused, not written over her phone's: {v}"
    );
    assert!(
        v["error"]
            .as_str()
            .unwrap_or_default()
            .contains("MiB of collections it keeps"),
        "{v}"
    );
    assert_eq!(
        fake.files.lock().unwrap()[ANN],
        files_before,
        "no version was overwritten or deleted"
    );
    let (_, list) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(
        list.as_array().map(Vec::len),
        Some(2),
        "the new one isn't kept: {list}"
    );
    assert_eq!(
        newest(&fake, ANN, &id).0,
        phone,
        "her phone's version stays the newest"
    );
    call(&app, &ann, "GET", "/v1/collections", None).await;
    let reads = fake
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c.ends_with(&format!("read {phone}")))
        .count();
    assert_eq!(reads, 1, "not downloaded again on every read");
    // Room again (the other collection shortened): both are read and kept, and changes go above
    set_ballast(1_000);
    let (_, list) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(list.as_array().map(Vec::len), Some(3), "the new one too: {list}");
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        &format!("/v1/collections/{id}"),
        Some(serde_json::json!({"name": "Renamed"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (path, content) = newest(&fake, ANN, &id);
    assert!(path > phone, "{path} above her phone's {phone}");
    assert_eq!(item_uris(&content).len(), 26, "her phone's songs are kept");
    // Even with the copy over the cap, a delete goes, every version
    set_ballast(CAP + 100_000);
    let (st, v) = call(&app, &ann, "DELETE", &format!("/v1/collections/{id}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT, "{v}");
    assert!(fake.paths(ANN, &format!("{DRAFTS}{id}/")).is_empty());
}

#[tokio::test]
async fn a_newer_version_the_node_cant_read_is_never_written_over() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let rename = |name: &str| Some(serde_json::json!({"name": name}));
    let (_, v) = create(&app, &ann, "Long one", &[SONG_A]).await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();
    // Another device of hers made it longer than the node can read (over its 1 MiB cap)
    let phone = another_app_writes(&fake, &id, false, |c| c["name"] = "Longer".into());
    fake.unreadable.lock().unwrap().insert(phone.clone());
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        &format!("/v1/collections/{id}"),
        rename("Here"),
    )
    .await;
    assert_eq!(st, StatusCode::PAYLOAD_TOO_LARGE, "{v}");
    assert!(
        v["error"].as_str().unwrap_or_default().contains("can't read"),
        "{v}"
    );
    assert_eq!(
        newest(&fake, ANN, &id).0,
        phone,
        "nothing written over her other device's"
    );
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        &format!("/v1/collections/{id}"),
        rename("Here"),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::PAYLOAD_TOO_LARGE,
        "and on the next try too (it is set aside): {v}"
    );
    assert_eq!(newest(&fake, ANN, &id).0, phone);

    // The same when it is caught up from its own folder (all her posts can't be listed)
    let (_, v) = create(&app, &ann, "Another", &[SONG_A]).await;
    let other = v["summary"]["id"].as_str().unwrap().to_string();
    let laptop = another_app_writes(&fake, &other, false, |c| c["name"] = "Longer too".into());
    fake.unreadable.lock().unwrap().insert(laptop.clone());
    fake.whole_list_fails.store(true, Ordering::SeqCst);
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        &format!("/v1/collections/{other}"),
        rename("Here"),
    )
    .await;
    assert_eq!(st, StatusCode::PAYLOAD_TOO_LARGE, "{v}");
    assert_eq!(newest(&fake, ANN, &other).0, laptop);

    // A delete still goes, every version
    let (st, _) = call(&app, &ann, "DELETE", &format!("/v1/collections/{other}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert!(fake.paths(ANN, &format!("{DRAFTS}{other}/")).is_empty());
}

#[tokio::test]
async fn a_new_one_her_homeserver_never_takes_goes_while_all_her_posts_cant_be_listed() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    // Made while her homeserver was away: it waits
    fake.down.store(true, Ordering::SeqCst);
    let (st, v) = create(&app, &ann, "Never taken", &[SONG_A]).await;
    assert_eq!(
        (st, v["summary"]["synced"].clone()),
        (StatusCode::OK, serde_json::json!(false)),
        "{v}"
    );
    // Back, but all her posts can't be listed and it won't take that one: it goes, not shown as kept
    fake.down.store(false, Ordering::SeqCst);
    fake.whole_list_fails.store(true, Ordering::SeqCst);
    fake.rejects.lock().unwrap().insert(ANN.to_string());
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(v, serde_json::json!([]), "{v}");
    let (_, me) = call(&app, &ann, "GET", "/v1/me/homeserver", None).await;
    assert_eq!(me["pending"], 0, "{me}");
}

#[tokio::test]
async fn a_version_known_unreadable_holds_back_changes_made_while_away_and_waiting_ones() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, v) = create(&app, &ann, "Mix", &[SONG_A]).await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();
    let phone = another_app_writes(&fake, &id, false, |c| c["name"] = "Longer".into());
    fake.unreadable.lock().unwrap().insert(phone.clone());
    call(&app, &ann, "GET", "/v1/collections", None).await;
    // Her homeserver away: the node still knows that version, so a change isn't kept to go later
    fake.down.store(true, Ordering::SeqCst);
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        &format!("/v1/collections/{id}"),
        Some(serde_json::json!({"name": "While away"})),
    )
    .await;
    assert_eq!(st, StatusCode::PAYLOAD_TOO_LARGE, "{v}");
    fake.down.store(false, Ordering::SeqCst);
    // A change already waiting (made before the node knew) doesn't go over it either
    let key = format!("person.{ANN}.collections");
    let mut shelf: serde_json::Value =
        serde_json::from_str(&s.library.lock().unwrap().get_setting(&key).unwrap().unwrap()).unwrap();
    let item = shelf["items"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|k| k["id"] == id.as_str())
        .unwrap();
    item["pending"] = true.into();
    item["playlist"]["name"] = "Waiting".into();
    s.library
        .lock()
        .unwrap()
        .set_setting(&key, &shelf.to_string())
        .unwrap();
    call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(
        newest(&fake, ANN, &id).0,
        phone,
        "her other device's stays the newest"
    );
    let (_, me) = call(&app, &ann, "GET", "/v1/me/homeserver", None).await;
    assert_eq!(me["pending"], 1, "it keeps waiting: {me}");
    // The same when what waits goes one collection at a time
    fake.whole_list_fails.store(true, Ordering::SeqCst);
    call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(newest(&fake, ANN, &id).0, phone);
    // A delete still goes
    let (st, _) = call(&app, &ann, "DELETE", &format!("/v1/collections/{id}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert!(fake.paths(ANN, &format!("{DRAFTS}{id}/")).is_empty());
}

#[tokio::test]
async fn a_waiting_change_never_taken_shows_what_her_homeserver_has() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, v) = create(&app, &ann, "Mix", &[SONG_A]).await;
    let id = v["summary"]["id"].as_str().unwrap().to_string();
    fake.down.store(true, Ordering::SeqCst);
    call(
        &app,
        &ann,
        "PUT",
        &format!("/v1/collections/{id}"),
        Some(serde_json::json!({"name": "Never taken"})),
    )
    .await;
    // Back, all her posts can't be listed, and it won't take that change: her homeserver's shows
    fake.down.store(false, Ordering::SeqCst);
    fake.whole_list_fails.store(true, Ordering::SeqCst);
    fake.rejects.lock().unwrap().insert(ANN.to_string());
    let (_, v) = call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(names(&v), vec!["Mix"], "{v}");
}

#[tokio::test]
async fn removing_a_listener_revokes_access_while_their_collection_write_is_held() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let hold = fake.gate.write().await;
    let (a, t) = (app.clone(), ann.clone());
    let writing = tokio::spawn(async move { create(&a, &t, "Held", &[SONG_B]).await });
    fake.write_started.notified().await;
    let a = app.clone();
    let removal =
        tokio::spawn(async move { call(&a, OWNER, "DELETE", &format!("/v1/people/{ANN}"), None).await });
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while pm_api::people::may_use_music(&s, ANN) || pm_api::accounts::sessions_of(&s, ANN) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("revocation must not wait for her held collection write");
    assert_eq!(
        call(&app, &ann, "GET", "/v1/me", None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert!(!writing.is_finished() && !removal.is_finished());
    drop(hold);
    writing.await.unwrap();
    let (st, v) = removal.await.unwrap();
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(!fake.connected.lock().unwrap().contains(ANN));
    assert!(s
        .library
        .lock()
        .unwrap()
        .settings_with_prefix(&format!("person.{ANN}."))
        .unwrap()
        .is_empty());
}

fn stored_shelf(s: &pm_api::NodeState) -> serde_json::Value {
    serde_json::from_str(
        &s.library
            .lock()
            .unwrap()
            .get_setting(&format!("person.{ANN}.collections"))
            .unwrap()
            .unwrap(),
    )
    .unwrap()
}

fn store_shelf(s: &pm_api::NodeState, shelf: &serde_json::Value) {
    s.library
        .lock()
        .unwrap()
        .set_setting(&format!("person.{ANN}.collections"), &shelf.to_string())
        .unwrap();
}

fn remote_version(fake: &Fake, id: &str, after: i64, post: &serde_json::Value, public: bool) -> String {
    let edit = pm_pubky::v1::id_of(pm_pubky::ids::now_micros() + after);
    let path = format!("{DRAFTS}{id}/{edit}.json");
    let bytes = serde_json::to_vec(post).unwrap();
    let mut files = fake.files.lock().unwrap();
    let mine = files.entry(ANN.to_string()).or_default();
    mine.insert(path.clone(), bytes.clone());
    if public {
        mine.insert(format!("{PUBLISHED}{id}/{edit}.json"), bytes);
    }
    path
}

fn no_public_writes(fake: &Fake) {
    let calls = fake.calls.lock().unwrap().clone();
    assert!(
        calls.iter().all(|c| !c.contains(" write /pub/")),
        "cancelled sharing must never be published, even briefly: {calls:?}"
    );
}

#[tokio::test]
async fn pending_edits_classify_unseen_heads_before_full_or_targeted_replay() {
    for fallback in [false, true] {
        for bad_kind in ["malformed", "unsupported", "oversized"] {
            let dir = tempfile::tempdir().unwrap();
            let (s, fake, ann, _) = node(dir.path());
            fake.connected.lock().unwrap().insert(ANN.to_string());
            let app = router(s.clone());
            let (_, made) = create(&app, &ann, "Before", &[SONG_A]).await;
            let id = made["summary"]["id"].as_str().unwrap().to_string();
            let uri = format!("/v1/collections/{id}");
            let good = fake.json(ANN, &newest(&fake, ANN, &id).0);
            fake.down.store(true, Ordering::SeqCst);
            let (st, queued) = call(
                &app,
                &ann,
                "PUT",
                &uri,
                Some(serde_json::json!({"name":"Queued"})),
            )
            .await;
            assert_eq!(st, StatusCode::OK, "{queued}");
            assert_eq!(queued["summary"]["synced"], false);
            // This head did not exist when the pending intent was created.
            let mut bad = good.clone();
            match bad_kind {
                "malformed" => bad["content"] = "{".into(),
                "unsupported" => bad["kind"] = "future_post_kind".into(),
                "oversized" => bad["future_data"] = "x".repeat(pm_pubky::homeserver::OBJECT_MAX).into(),
                _ => unreachable!(),
            }
            let path = remote_version(&fake, &id, 60_000_000, &bad, false);
            let before = fake.files.lock().unwrap()[ANN].clone();
            fake.down.store(false, Ordering::SeqCst);
            fake.whole_list_fails.store(fallback, Ordering::SeqCst);
            fake.calls.lock().unwrap().clear();
            for _ in 0..2 {
                let (st, _) = call(&app, &ann, "GET", "/v1/collections", None).await;
                assert_eq!(st, StatusCode::OK);
                let shelf = stored_shelf(&s); // Every request reloads persisted state.
                assert!(shelf["other"].as_array().unwrap().iter().any(|p| p == &path));
                assert_eq!(shelf["items"][0]["pending"], true);
                assert_eq!(fake.files.lock().unwrap()[ANN], before);
            }
            assert!(!fake.calls.lock().unwrap().iter().any(|c| c.contains(" write ")));
            assert_eq!(
                fake.calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|c| c.ends_with(&format!("read {path}")))
                    .count(),
                1
            );
            // A supported later version releases the hold; the queued local intent still wins.
            let supported = remote_version(&fake, &id, 120_000_000, &good, false);
            let (st, _) = call(&app, &ann, "GET", "/v1/collections", None).await;
            assert_eq!(st, StatusCode::OK);
            let (head, content) = newest(&fake, ANN, &id);
            assert!(head > supported);
            assert_eq!(content["name"], "Queued");
            assert_eq!(stored_shelf(&s)["items"][0]["pending"], false);
        }
    }
}

#[tokio::test]
async fn private_or_delete_as_the_first_reconnected_request_never_flushes_a_queued_share() {
    for delete in [false, true] {
        for failure in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let (s, fake, ann, _) = node(dir.path());
            fake.connected.lock().unwrap().insert(ANN.to_string());
            let app = router(s.clone());
            let (_, made) = create(&app, &ann, "Private", &[SONG_A]).await;
            let id = made["summary"]["id"].as_str().unwrap();
            let uri = format!("/v1/collections/{id}");
            fake.down.store(true, Ordering::SeqCst);
            call(
                &app,
                &ann,
                "PUT",
                &format!("{uri}/visibility"),
                Some(serde_json::json!({"public":true})),
            )
            .await;
            assert_eq!(stored_shelf(&s)["items"][0]["public"], true);
            fake.down.store(false, Ordering::SeqCst);
            fake.calls.lock().unwrap().clear();
            if failure {
                if delete {
                    fake.remove_fails.store(true, Ordering::SeqCst);
                } else {
                    fake.rejects.lock().unwrap().insert(ANN.to_string());
                }
            }
            let (st, v) = if delete {
                call(&app, &ann, "DELETE", &uri, None).await
            } else {
                call(
                    &app,
                    &ann,
                    "PUT",
                    &format!("{uri}/visibility"),
                    Some(serde_json::json!({"public":false})),
                )
                .await
            };
            assert_eq!(
                st,
                if delete {
                    StatusCode::NO_CONTENT
                } else if failure {
                    StatusCode::UNPROCESSABLE_ENTITY
                } else {
                    StatusCode::OK
                },
                "{v}"
            );
            no_public_writes(&fake);
            assert!(fake.paths(ANN, PUBLISHED).is_empty());
            fake.remove_fails.store(false, Ordering::SeqCst);
            fake.rejects.lock().unwrap().clear();
            call(&app, &ann, "GET", "/v1/collections", None).await;
            no_public_writes(&fake);
            if delete {
                assert!(fake.paths(ANN, &format!("{DRAFTS}{id}/")).is_empty());
            }
        }
    }
}

#[tokio::test]
async fn cancelling_a_privacy_request_after_io_starts_keeps_the_latest_intent() {
    for delete in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (s, fake, ann, _) = node(dir.path());
        fake.connected.lock().unwrap().insert(ANN.to_string());
        let app = router(s.clone());
        let (_, made) = create(&app, &ann, "Private", &[SONG_A]).await;
        let id = made["summary"]["id"].as_str().unwrap();
        let uri = format!("/v1/collections/{id}");
        // Consume the notification from creation before waiting for this request's write.
        fake.write_started.notified().await;
        fake.down.store(true, Ordering::SeqCst);
        call(
            &app,
            &ann,
            "PUT",
            &format!("{uri}/visibility"),
            Some(serde_json::json!({"public":true})),
        )
        .await;
        fake.down.store(false, Ordering::SeqCst);
        fake.calls.lock().unwrap().clear();
        let write_hold = fake.gate.write().await;
        let remove_hold = fake.removal_gate.write().await;
        let task = tokio::spawn({
            let app = app.clone();
            let token = ann.clone();
            let uri = uri.clone();
            async move {
                if delete {
                    call(&app, &token, "DELETE", &uri, None).await
                } else {
                    call(
                        &app,
                        &token,
                        "PUT",
                        &format!("{uri}/visibility"),
                        Some(serde_json::json!({"public":false})),
                    )
                    .await
                }
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            if delete {
                fake.remove_started.notified().await;
            } else {
                fake.write_started.notified().await;
            }
        })
        .await
        .expect("the new intent reaches its gated operation");
        let shelf = stored_shelf(&s);
        assert_eq!(shelf["items"][0]["pending"], true);
        assert!(shelf["items"][0]["public"] == false || shelf["items"][0]["deleted"] == true);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        drop(write_hold);
        drop(remove_hold);
        call(&app, &ann, "GET", "/v1/collections", None).await;
        no_public_writes(&fake);
        assert!(fake.paths(ANN, PUBLISHED).is_empty());
    }
}

#[tokio::test]
async fn original_posts_with_unrepresentable_data_are_held_without_loss() {
    for data in [
        "note",
        "item_extra",
        "content_extra",
        "post_extra",
        "item_uri",
        "layout",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (s, fake, ann, _) = node(dir.path());
        fake.connected.lock().unwrap().insert(ANN.to_string());
        let app = router(s.clone());
        let (_, made) = create(&app, &ann, "Original", &[SONG_A, SONG_B]).await;
        let id = made["summary"]["id"].as_str().unwrap();
        let uri = format!("/v1/collections/{id}");
        let mut post = fake.json(ANN, &newest(&fake, ANN, id).0);
        let mut content: serde_json::Value = serde_json::from_str(post["content"].as_str().unwrap()).unwrap();
        match data {
            "note" => content["items"][0]["note"] = "Play this at the wedding".into(),
            "item_extra" => content["items"][0]["other_app"] = serde_json::json!({"colour":"blue"}),
            "content_extra" => content["other_app"] = serde_json::json!({"sequence":[3,1,2]}),
            "post_extra" => post["other_app"] = serde_json::json!({"saved":true}),
            "item_uri" => content["items"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({"uri":format!("pubky://{ANN}/pub/example.app/things/one")})),
            "layout" => content["layout"] = "future_layout".into(),
            _ => unreachable!(),
        }
        post["content"] = content.to_string().into();
        let typed: pubky_social_specs::PubkySocialPost = serde_json::from_value(post.clone()).unwrap();
        pm_pubky::v1::validate_social(
            &typed,
            id,
            pm_pubky::v1::Root::Priv,
            &pm_pubky::v1::PubkyId::try_from(ANN).unwrap(),
        )
        .unwrap_or_else(|e| panic!("valid remote {data} fixture: {e}"));
        remote_version(&fake, id, 60_000_000, &post, true);
        let before = fake.files.lock().unwrap()[ANN].clone();
        for (method, path, body) in [
            ("PUT", uri.clone(), serde_json::json!({"name":"Renamed"})),
            (
                "POST",
                format!("{uri}/items"),
                serde_json::json!({"uris":[SONG_C]}),
            ),
            ("PUT", uri.clone(), serde_json::json!({"uris":[SONG_B,SONG_A]})),
        ] {
            let (st, v) = call(&app, &ann, method, &path, Some(body)).await;
            assert_eq!(st, StatusCode::PAYLOAD_TOO_LARGE, "{data}: {v}");
            assert_eq!(
                fake.files.lock().unwrap()[ANN],
                before,
                "{data} must remain authoritative and byte-identical"
            );
        }
        // Privacy still has an escape: no private post is re-encoded, and its opaque data stays.
        let (st, v) = call(
            &app,
            &ann,
            "PUT",
            &format!("{uri}/visibility"),
            Some(serde_json::json!({"public":false})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{data}: {v}");
        assert!(fake.paths(ANN, PUBLISHED).is_empty());
        let private_before: std::collections::BTreeMap<_, _> = before
            .into_iter()
            .filter(|(p, _)| p.starts_with(DRAFTS))
            .collect();
        assert_eq!(fake.files.lock().unwrap()[ANN], private_before);
    }
}

/// Seed many independent offline intents without spending the regression on HTTP setup.
/// Each is a valid small collection; none has a remote version yet.
fn seed_pending(s: &pm_api::NodeState, count: usize) {
    let mut shelf = stored_shelf(s);
    let template = shelf["items"][0].clone();
    let base = pm_pubky::ids::now_micros() - 10_000_000;
    for n in 0..count {
        let mut k = template.clone();
        k["id"] = pm_pubky::v1::id_of(base - n as i64).into();
        k["head"] = "".into();
        k["versions"] = serde_json::json!([]);
        k["writing"] = serde_json::Value::Null;
        k["pending"] = true.into();
        k["playlist"]["name"] = format!("Waiting {n}").into();
        shelf["items"].as_array_mut().unwrap().push(k);
    }
    store_shelf(s, &shelf);
}

fn pending_count(s: &pm_api::NodeState) -> usize {
    stored_shelf(s)["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|k| k["pending"] == true)
        .count()
}

#[tokio::test]
async fn replay_has_one_shared_budget_and_checkpoints_progress_in_both_listing_modes() {
    for fallback in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (s, fake, ann, _) = node(dir.path());
        fake.connected.lock().unwrap().insert(ANN.to_string());
        let app = router(s.clone());
        create(&app, &ann, "Template", &[SONG_A]).await;
        seed_pending(&s, 45);
        fake.whole_list_fails.store(fallback, Ordering::SeqCst);
        fake.calls.lock().unwrap().clear();
        call(&app, &ann, "GET", "/v1/collections", None).await;
        let remaining = pending_count(&s);
        assert!(
            remaining > 0 && remaining < 45,
            "one request must make bounded durable progress: {remaining}"
        );
        assert!(
            fake.calls.lock().unwrap().len() <= 32,
            "list/read/write all share the call budget"
        );
        // Subsequent requests reload the persisted pending work and finish only what remains.
        for _ in 0..12 {
            if pending_count(&s) == 0 {
                break;
            }
            fake.calls.lock().unwrap().clear();
            call(&app, &ann, "GET", "/v1/collections", None).await;
            assert!(fake.calls.lock().unwrap().len() <= 32);
        }
        assert_eq!(pending_count(&s), 0);
        assert_eq!(
            fake.paths(ANN, DRAFTS).len(),
            46,
            "every pending collection written exactly once"
        );
    }
}

fn add_history(fake: &Fake, id: &str, count: usize, public: bool) {
    let body = fake.json(ANN, &newest(fake, ANN, id).0);
    let head = remote_version(fake, id, 60_000_000, &body, public);
    let (_, edit) = pm_pubky::v1::parse_version(&head).unwrap();
    let end = pm_pubky::v1::micros_of(&edit).unwrap();
    let mut files = fake.files.lock().unwrap();
    let mine = files.get_mut(ANN).unwrap();
    for i in 1..=count {
        let version = pm_pubky::v1::id_of(end - i as i64);
        let root = if public { PUBLISHED } else { DRAFTS };
        mine.insert(
            format!("{root}{id}/{version}.json"),
            serde_json::to_vec(&body).unwrap(),
        );
    }
}

#[tokio::test]
async fn delete_and_unshare_large_histories_finish_in_bounded_durable_batches() {
    for delete in [false, true] {
        for fallback in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let (s, fake, ann, _) = node(dir.path());
            fake.connected.lock().unwrap().insert(ANN.to_string());
            let app = router(s.clone());
            let (_, made) = create(&app, &ann, "History", &[SONG_A]).await;
            let id = made["summary"]["id"].as_str().unwrap();
            let uri = format!("/v1/collections/{id}");
            add_history(&fake, id, 70, !delete);
            let folder = if delete { DRAFTS } else { PUBLISHED };
            let before = fake.paths(ANN, folder).len();
            fake.whole_list_fails.store(fallback, Ordering::SeqCst);
            fake.calls.lock().unwrap().clear();
            let (st, v) = if delete {
                call(&app, &ann, "DELETE", &uri, None).await
            } else {
                call(
                    &app,
                    &ann,
                    "PUT",
                    &format!("{uri}/visibility"),
                    Some(serde_json::json!({"public":false})),
                )
                .await
            };
            assert_eq!(
                st,
                if delete {
                    StatusCode::NO_CONTENT
                } else {
                    StatusCode::OK
                },
                "{v}"
            );
            let remaining = fake.paths(ANN, folder).len();
            assert!(
                remaining > 0 && remaining < before,
                "a batch must stop with durable remaining work"
            );
            assert_eq!(pending_count(&s), 1);
            assert!(fake.calls.lock().unwrap().len() <= 32);
            assert!(
                stored_shelf(&s)["items"][0]["versions"].as_array().unwrap().len() <= 2,
                "the whole history is never stored"
            );
            for _ in 0..12 {
                if pending_count(&s) == 0 {
                    break;
                }
                fake.calls.lock().unwrap().clear();
                call(&app, &ann, "GET", "/v1/collections", None).await;
                assert!(fake.calls.lock().unwrap().len() <= 32);
            }
            assert_eq!(pending_count(&s), 0);
            assert!(fake.paths(ANN, folder).is_empty());
            if !delete {
                assert!(!fake.paths(ANN, DRAFTS).is_empty(), "private content stays");
            }
        }
    }
}

#[tokio::test]
async fn a_held_replay_deadline_releases_cleanup_without_waiting_for_its_backlog() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    create(&app, &ann, "Template", &[SONG_A]).await;
    fake.write_started.notified().await;
    seed_pending(&s, 45);
    let hold = fake.gate.write().await;
    let reading = tokio::spawn({
        let app = app.clone();
        let token = ann.clone();
        async move { call(&app, &token, "GET", "/v1/collections", None).await }
    });
    fake.write_started.notified().await;
    assert_eq!(pending_count(&s), 45, "intent survives before the held write");
    let removal = tokio::spawn({
        let app = app.clone();
        async move { call(&app, OWNER, "DELETE", &format!("/v1/people/{ANN}"), None).await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while pm_api::people::may_use_music(&s, ANN) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("revocation remains prompt while I/O is held");
    assert_eq!(
        call(&app, &ann, "GET", "/v1/me", None).await.0,
        StatusCode::UNAUTHORIZED
    );
    // Keep the gate closed. The shared eight-second deadline, not the homeserver/test,
    // releases the operation lock and allows removal's cleanup to finish.
    let (st, v) = tokio::time::timeout(std::time::Duration::from_secs(12), removal)
        .await
        .expect("cleanup must not await the entire pending backlog")
        .unwrap();
    assert_eq!(st, StatusCode::OK, "{v}");
    reading.await.unwrap();
    assert!(!fake.connected.lock().unwrap().contains(ANN));
    assert!(s
        .library
        .lock()
        .unwrap()
        .settings_with_prefix(&format!("person.{ANN}."))
        .unwrap()
        .is_empty());
    drop(hold);
}

#[tokio::test]
async fn shelf_budget_includes_history_markers_and_reserved_publication_paths() {
    const CAP: usize = 32 * 1024 * 1024;
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    let (_, made) = create(&app, &ann, "Small", &[SONG_A]).await;
    let id = made["summary"]["id"].as_str().unwrap();
    let uri = format!("/v1/collections/{id}");
    let original = newest(&fake, ANN, id).0;
    let body = fake.json(ANN, &original);
    let bid = pm_pubky::v1::id_of(pm_pubky::ids::now_micros() - 60_000_000);
    let ballast_path = format!("{DRAFTS}{bid}/{bid}.json");
    fake.files
        .lock()
        .unwrap()
        .get_mut(ANN)
        .unwrap()
        .insert(ballast_path.clone(), serde_json::to_vec(&body).unwrap());
    let mut shelf = stored_shelf(&s);
    let mut ballast = shelf["items"][0].clone();
    ballast["id"] = bid.clone().into();
    ballast["head"] = bid.clone().into();
    ballast["versions"] = serde_json::json!([ballast_path]);
    ballast["playlist"]["description"] = "".into();
    shelf["items"].as_array_mut().unwrap().push(ballast);
    let set_free = |free: usize| {
        let mut shelf = stored_shelf(&s);
        let ballast = shelf["items"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|k| k["id"] == bid.as_str())
            .unwrap();
        ballast["playlist"]["description"] = "".into();
        let overhead = serde_json::to_vec(&shelf).unwrap().len();
        let ballast = shelf["items"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|k| k["id"] == bid.as_str())
            .unwrap();
        ballast["playlist"]["description"] = "x".repeat(CAP - free - overhead).into();
        assert_eq!(serde_json::to_vec(&shelf).unwrap().len(), CAP - free);
        store_shelf(&s, &shelf);
    };
    store_shelf(&s, &shelf);
    // Move the real head ahead, then add many older paths without changing that cached head.
    let head = remote_version(&fake, id, 60_000_000, &body, false);
    call(&app, &ann, "GET", "/v1/collections", None).await;
    let (_, edit) = pm_pubky::v1::parse_version(&head).unwrap();
    let end = pm_pubky::v1::micros_of(&edit).unwrap();
    for n in 1..=80 {
        let version = pm_pubky::v1::id_of(end - n);
        fake.files.lock().unwrap().get_mut(ANN).unwrap().insert(
            format!("{DRAFTS}{id}/{version}.json"),
            serde_json::to_vec(&body).unwrap(),
        );
    }
    set_free(8);
    for fallback in [false, true] {
        fake.whole_list_fails.store(fallback, Ordering::SeqCst);
        fake.calls.lock().unwrap().clear();
        let files = fake.files.lock().unwrap()[ANN].clone();
        let (st, v) = call(
            &app,
            &ann,
            "PUT",
            &format!("{uri}/visibility"),
            Some(serde_json::json!({"public":true})),
        )
        .await;
        assert_eq!(
            st,
            StatusCode::PAYLOAD_TOO_LARGE,
            "reserve progress and both paths before writing: {v}"
        );
        assert_eq!(fake.files.lock().unwrap()[ANN], files);
        assert!(serde_json::to_vec(&stored_shelf(&s)).unwrap().len() <= CAP);
        assert!(!fake.calls.lock().unwrap().iter().any(|c| c.contains(" write ")));
    }
    // An unfamiliar head's marker is also budgeted; lack of marker space cannot permit replay.
    fake.whole_list_fails.store(false, Ordering::SeqCst);
    let mut bad = body.clone();
    bad["content"] = "{".into();
    let invalid = remote_version(&fake, id, 120_000_000, &bad, false);
    let files = fake.files.lock().unwrap()[ANN].clone();
    let (st, _) = call(&app, &ann, "PUT", &uri, Some(serde_json::json!({"name":"Other"}))).await;
    assert_eq!(st, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(serde_json::to_vec(&stored_shelf(&s)).unwrap().len() <= CAP);
    assert_eq!(fake.files.lock().unwrap()[ANN], files);
    assert_eq!(
        fake.paths(ANN, &format!("{DRAFTS}{id}/"))
            .into_iter()
            .max()
            .unwrap(),
        invalid
    );
    // Make room and provide a supported head: the same share can now complete within the cap.
    set_free(4_000);
    remote_version(&fake, id, 180_000_000, &body, false);
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        &format!("{uri}/visibility"),
        Some(serde_json::json!({"public":true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    for _ in 0..4 {
        assert!(serde_json::to_vec(&stored_shelf(&s)).unwrap().len() <= CAP);
        if pending_count(&s) == 0 {
            break;
        }
        call(&app, &ann, "GET", "/v1/collections", None).await;
    }
    assert!(!fake.paths(ANN, PUBLISHED).is_empty());
    assert_eq!(pending_count(&s), 0);
    assert!(stored_shelf(&s)["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|k| k["versions"].as_array().unwrap().len() <= 2));
    // A full shelf cannot prevent an explicit privacy withdrawal. It can defer its new
    // private version, but must retain the intent and remove the existing public copies.
    set_free(0);
    let (st, v) = call(
        &app,
        &ann,
        "PUT",
        &format!("{uri}/visibility"),
        Some(serde_json::json!({"public":false})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(fake.paths(ANN, PUBLISHED).is_empty());
    assert!(serde_json::to_vec(&stored_shelf(&s)).unwrap().len() <= CAP);
}

#[tokio::test]
async fn a_budget_boundary_between_draft_and_publication_keeps_the_completed_draft() {
    let dir = tempfile::tempdir().unwrap();
    let (s, fake, ann, _) = node(dir.path());
    fake.connected.lock().unwrap().insert(ANN.to_string());
    let app = router(s.clone());
    create(&app, &ann, "Template", &[SONG_A]).await;
    seed_pending(&s, 30);
    let mut shelf = stored_shelf(&s);
    // The full listing's two calls plus 29 private writes leave one call for this last
    // collection's draft. Publication must wait for the next request.
    let last = shelf["items"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .filter(|k| k["pending"] == true)
        .max_by_key(|k| k["id"].as_str().unwrap().to_string())
        .unwrap();
    last["public"] = true.into();
    let id = last["id"].as_str().unwrap().to_string();
    store_shelf(&s, &shelf);
    fake.calls.lock().unwrap().clear();
    call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(fake.calls.lock().unwrap().len(), 32);
    let shelf = stored_shelf(&s);
    let kept = shelf["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["id"] == id.as_str())
        .unwrap();
    assert_eq!(kept["pending"], true);
    assert_eq!(
        kept["writing"]["drafted"], true,
        "completed mutation checkpointed before yielding"
    );
    assert_eq!(kept["writing"]["published"], false);
    let draft = newest(&fake, ANN, &id).0;
    assert!(fake.paths(ANN, &format!("{PUBLISHED}{id}/")).is_empty());
    fake.calls.lock().unwrap().clear();
    call(&app, &ann, "GET", "/v1/collections", None).await;
    assert_eq!(pending_count(&s), 0);
    assert_eq!(fake.paths(ANN, &format!("{DRAFTS}{id}/")), vec![draft]);
    let calls = fake.calls.lock().unwrap();
    assert!(
        !calls.iter().any(|c| c.contains(&format!("write {DRAFTS}{id}/"))),
        "resume publication without duplicating the draft: {calls:?}"
    );
    assert_eq!(fake.paths(ANN, &format!("{PUBLISHED}{id}/")).len(), 1);
}
