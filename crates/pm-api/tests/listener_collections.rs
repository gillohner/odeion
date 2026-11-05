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
    pm_api::accounts::forget_person(&s, ANN).unwrap();
    let key = format!("person.{ANN}.collections");
    assert!(
        s.library.lock().unwrap().get_setting(&key).unwrap().is_none(),
        "People › Remove leaves no copy of hers"
    );
}
