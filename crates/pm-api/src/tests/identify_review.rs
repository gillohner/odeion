//! Identification: how a run ends, album-level review, cautious album matching and reuse of
//! earlier answers. Router and DB seams with mock providers; nothing reaches the network and no
//! file's tags are written (Write IDs stays off).

use super::*;

/// The Identify card's headline: what state identification is in, said once, beside the old
/// fields (which keep their meaning).
#[tokio::test]
async fn the_status_says_how_identification_stands() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let phase = |app: Router| async move {
        let (st, b) = call(&app, "GET", "/v1/library/identify", None).await;
        assert_eq!(st, StatusCode::OK, "{b}");
        // The old fields stay as they were
        assert!(b["progress"]["running"].is_boolean(), "{b}");
        assert!(b["counts"]["suggested"].is_u64(), "{b}");
        (b["phase"].as_str().unwrap_or_default().to_string(), b)
    };
    // No songs at all
    assert_eq!(phase(app.clone()).await.0, "empty");
    // Songs nobody has looked at
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    let mut ids = Vec::new();
    for i in 1..=2 {
        let t = untagged_track(music.join(format!("{i:02} s.wav")), &format!("S {i}"), "Band");
        ids.push(s.library.lock().unwrap().upsert_track(&t).unwrap().0);
    }
    assert_eq!(phase(app.clone()).await.0, "new");
    // A run going on
    s.identify.lock().unwrap().running = true;
    assert_eq!(phase(app.clone()).await.0, "running");
    s.identify.lock().unwrap().running = false;
    // A run that ends with nothing found and nothing to review (no MusicBrainz contact, no
    // AcoustID key): finished, ready
    let (st, _) = call(&app, "POST", "/v1/library/identify", Some(serde_json::json!({}))).await;
    assert_eq!(st, StatusCode::OK);
    let finished = async {
        loop {
            let (p, b) = phase(app.clone()).await;
            if p != "running" {
                return (p, b);
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    };
    let (p, b) = tokio::time::timeout(std::time::Duration::from_secs(30), finished)
        .await
        .unwrap();
    assert_eq!(p, "ready", "{b}");
    assert_eq!(b["progress"]["done"], 2, "{b}");
    assert!(b["progress"]["finished_at"].is_i64(), "{b}");
    // Finished with matches waiting for the owner: said as such
    s.library
        .lock()
        .unwrap()
        .add_suggestions(
            ids[0],
            &[pm_library::identify::Candidate {
                recording: pm_model::Mbid::parse("b1a9c0e9-d987-4042-ae91-78d6a3267d69").unwrap(),
                release: None,
                title: Some("S 1".into()),
                artist: Some("Band".into()),
                release_title: None,
                score: 0.8,
                source: "search".into(),
            }],
        )
        .unwrap();
    let (p, b) = phase(app.clone()).await;
    assert_eq!(p, "needs_review", "{b}");
    assert_eq!(b["counts"]["suggested"], 1, "{b}");
    // A run that stopped on an error
    s.identify.lock().unwrap().stopped = Some("the library stayed busy".into());
    assert_eq!(phase(app.clone()).await.0, "failed");

    // The node restarts: the last run is remembered, so the card still knows it finished
    let s2 = state(dir.path(), None).await;
    let app2 = router(s2.clone());
    let (p, b) = phase(app2.clone()).await;
    assert_eq!(p, "needs_review", "the finished run is remembered: {b}");
    assert_eq!(b["progress"]["done"], 2, "{b}");
    assert!(b["progress"]["finished_at"].is_i64(), "{b}");
    // A run cut off by a restart (it never finished): interrupted, not running and not finished
    let mut last = s2.identify.lock().unwrap().clone();
    last.running = true;
    last.finished_at = None;
    last.stopped = None;
    s2.library
        .lock()
        .unwrap()
        .set_setting("identify.last_run", &serde_json::to_string(&last).unwrap())
        .unwrap();
    let s3 = state(dir.path(), None).await;
    let (p, b) = phase(router(s3)).await;
    assert_eq!(p, "interrupted", "{b}");
    assert_eq!(b["progress"]["running"], false, "{b}");
}

const REL: &str = "1a5b6a49-4c4f-4d18-9e3d-1c4d1b0a7a3e";
const ELSE: &str = "3c7d8c6b-6e6b-4f3a-9a5f-3e6f3d2c9c5a";
fn rec(i: u32) -> String {
    format!("00000000-0000-4000-8000-{i:012}")
}

/// An untagged song in `folder` (no file on disk is needed: nothing here reads it).
fn song(s: &NodeState, dir: &std::path::Path, folder: &str, no: u32, title: &str) -> i64 {
    let mut t = untagged_track(
        dir.join("music")
            .join(folder)
            .join(format!("{no:02} {title}.flac")),
        title,
        "Band",
    );
    t.track_no = Some(no);
    t.disc_no = None;
    t.album = Some(pm_model::Album {
        id: None,
        title: folder.rsplit('/').next().unwrap().into(),
        album_artist: Some("Band".into()),
        release_mbid: None,
        release_group_mbid: None,
        year: None,
        upc: None,
        total_discs: None,
    });
    s.library.lock().unwrap().upsert_track(&t).unwrap().0
}

fn option(
    recording: &str,
    release: Option<&str>,
    title: &str,
    score: f32,
    source: &str,
) -> pm_library::identify::Candidate {
    pm_library::identify::Candidate {
        recording: pm_model::Mbid::parse(recording).unwrap(),
        release: release.map(|r| pm_model::Mbid::parse(r).unwrap()),
        title: Some(title.into()),
        artist: Some("Band".into()),
        release_title: Some(if release == Some(REL) { "Album" } else { "Best Of" }.into()),
        score,
        source: source.into(),
    }
}

fn suggest(s: &NodeState, track: i64, c: &[pm_library::identify::Candidate]) {
    let lib = s.library.lock().unwrap();
    lib.add_suggestions(track, c).unwrap();
    lib.set_identify_state(track, "suggested", None).unwrap();
}

async fn send_as(
    app: &Router,
    token: &str,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
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
    let r = app.clone().oneshot(req).await.unwrap();
    let st = r.status();
    let b = r.into_body().collect().await.unwrap().to_bytes();
    (st, serde_json::from_slice(&b).unwrap_or_default())
}

/// Album review: a folder's open suggestions as one album, each song with its own evidence; only
/// songs with one strong option are offered ticked, a song whose options disagree waits for the
/// owner's pick; the owner's explicit choices are confirmed together, or none when one went
/// stale; owner only.
#[tokio::test]
async fn album_review_groups_songs_and_confirms_only_explicit_choices() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), Some("owner-token")).await;
    let app = router(s.clone());
    let owner = |m: &'static str, u: String, b: Option<serde_json::Value>| {
        let app = app.clone();
        async move { send_as(&app, "owner-token", m, &u, b).await }
    };
    let t: Vec<i64> = (1..=4)
        .map(|i| song(&s, dir.path(), "Band/Album", i, &format!("Song {i}")))
        .collect();
    let lone = song(&s, dir.path(), "Band/Single", 1, "Lone");
    suggest(&s, t[0], &[option(&rec(1), Some(REL), "Song 1", 0.95, "album")]);
    suggest(
        &s,
        t[1],
        &[option(&rec(2), Some(REL), "Song 2", 0.93, "fingerprint")],
    );
    // Song 3: the folder and the fingerprint name different recordings, a real conflict
    suggest(
        &s,
        t[2],
        &[
            option(&rec(3), Some(REL), "Song 3", 0.95, "album"),
            option(ELSE, None, "Something Else", 0.96, "fingerprint"),
        ],
    );
    // Song 4: one weak search hit on another release
    suggest(&s, t[3], &[option(&rec(4), Some(ELSE), "Song 4", 0.6, "search")]);
    suggest(&s, lone, &[option(&rec(9), None, "Lone", 0.9, "search")]);

    let (st, b) = owner("GET", "/v1/library/suggestions/albums".into(), None).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["songs"], 5, "{b}");
    let albums = b["albums"].as_array().unwrap();
    assert_eq!(albums.len(), 2, "{b}");
    let album = albums.iter().find(|a| a["album"] == "Album").unwrap();
    assert_eq!(album["artist"], "Band");
    let songs = album["songs"].as_array().unwrap();
    let ids: Vec<i64> = songs.iter().map(|x| x["track"]["id"].as_i64().unwrap()).collect();
    assert_eq!(ids, t, "in album order");
    let sid = |k: usize, rec: &str| -> i64 {
        songs[k]["options"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["recording_mbid"] == rec)
            .unwrap()["id"]
            .as_i64()
            .unwrap()
    };
    assert_eq!(songs[0]["status"], "single");
    assert_eq!(songs[0]["pick"], sid(0, &rec(1)));
    assert_eq!(songs[1]["pick"], sid(1, &rec(2)));
    assert_eq!(songs[2]["status"], "conflict");
    assert!(
        songs[2]["pick"].is_null(),
        "a conflict is never picked for the owner"
    );
    assert_eq!(
        songs[2]["options"].as_array().unwrap().len(),
        2,
        "both kept, with their evidence"
    );
    assert_eq!(songs[2]["options"][0]["source"], "fingerprint");
    assert_eq!(songs[3]["status"], "single");
    assert!(
        songs[3]["pick"].is_null(),
        "weak and on another release: not ticked"
    );
    assert_eq!(
        (album["ready"].as_u64(), album["conflicts"].as_u64()),
        (Some(2), Some(1))
    );
    assert_eq!(album["release"]["mbid"], REL);
    assert_eq!(album["release"]["songs"], 3);
    // The flat list stays as it was
    let (_, flat) = owner("GET", "/v1/library/suggestions".into(), None).await;
    assert_eq!(flat.as_array().unwrap().len(), 6);

    // Confirm what the owner ticked
    let choice = |track: i64, sug: i64, rec: &str| serde_json::json!({"track_id": track, "suggestion_id": sug, "recording_mbid": rec});
    let (st, b) = owner(
        "POST",
        "/v1/library/suggestions/confirm".into(),
        Some(serde_json::json!({"choices": [choice(t[0], sid(0, &rec(1)), &rec(1)), choice(t[1], sid(1, &rec(2)), &rec(2))]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["confirmed"], serde_json::json!([t[0], t[1]]));
    let identity = |id: i64| {
        s.library
            .lock()
            .unwrap()
            .get_track(pm_model::TrackId(id))
            .unwrap()
            .unwrap()
            .canonical
    };
    assert_eq!(identity(t[0]).unwrap().1, pm_model::Confidence::ConfirmedByUser);
    // The owner picks song 3's folder option, but song 4 got an identity meanwhile: 409, nothing stored
    let pick3 = choice(t[2], sid(2, &rec(3)), &rec(3));
    let pick4 = choice(t[3], sid(3, &rec(4)), &rec(4));
    s.library
        .lock()
        .unwrap()
        .set_identity(
            t[3],
            &pm_model::MusicUri::Recording {
                mbid: pm_model::Mbid::parse(&rec(4)).unwrap(),
            },
            pm_model::Confidence::Fingerprint { score: 0.97 },
            None,
        )
        .unwrap();
    let (st, b) = owner(
        "POST",
        "/v1/library/suggestions/confirm".into(),
        Some(serde_json::json!({"choices": [pick3.clone(), pick4]})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["stale"][0]["track_id"], t[3]);
    assert_eq!(b["stale"][0]["reason"], "identified");
    assert!(identity(t[2]).is_none(), "nothing stored");
    // Nothing chosen, or too many at once: refused before anything is looked at
    let (st, _) = owner(
        "POST",
        "/v1/library/suggestions/confirm".into(),
        Some(serde_json::json!({"choices": []})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let many: Vec<serde_json::Value> = (0..501).map(|i| choice(i, i, &rec(1))).collect();
    let (st, _) = owner(
        "POST",
        "/v1/library/suggestions/confirm".into(),
        Some(serde_json::json!({"choices": many})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    // The owner only
    let key = "tbepfp8fisyiqpskbmxsmpub47gxb7xdsh5x9ut9qrb97gcq9phy";
    s.library
        .lock()
        .unwrap()
        .set_setting("people", &serde_json::json!({"people": [{"key": key, "access": "music", "music_enabled": true, "added_at": 1}]}).to_string())
        .unwrap();
    let tok = accounts::mint(&s, key).unwrap();
    let (st, _) = send_as(&app, &tok, "GET", "/v1/library/suggestions/albums", None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _) = send_as(
        &app,
        &tok,
        "POST",
        "/v1/library/suggestions/confirm",
        Some(serde_json::json!({"choices": [pick3.clone()]})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    assert!(identity(t[2]).is_none());
    // No token, or a body that isn't a list of choices: refused, nothing stored
    let (st, _) = call(
        &app,
        "POST",
        "/v1/library/suggestions/confirm",
        Some(serde_json::json!({"choices": [pick3.clone()]})),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _) = call(&app, "GET", "/v1/library/suggestions/albums", None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    for bad in [
        serde_json::json!({"choices": "all"}),
        serde_json::json!({"choices": [{"track_id": t[2]}]}),
        serde_json::json!({"release": REL}),
    ] {
        let (st, b) = owner(
            "POST",
            "/v1/library/suggestions/confirm".into(),
            Some(bad.clone()),
        )
        .await;
        assert!(st.is_client_error(), "{bad}: {st} {b}");
    }
    assert!(identity(t[2]).is_none());
    // The owner's pick of the conflict, alone
    let (st, b) = owner(
        "POST",
        "/v1/library/suggestions/confirm".into(),
        Some(serde_json::json!({"choices": [pick3]})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(
        identity(t[2]).unwrap().0.to_url(),
        format!("https://musicbrainz.org/recording/{}", rec(3))
    );
    let (_, b) = owner("GET", "/v1/library/suggestions/albums".into(), None).await;
    assert_eq!(
        b["songs"], 1,
        "song 4 has an identity now, so only the lone song is left: {b}"
    );
    // A single accept for a song identified meanwhile: refused, its identity kept
    let (st, b) = owner(
        "POST",
        format!("/v1/library/suggestions/{}/accept", sid(3, &rec(4))),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{b}");
    assert_eq!(
        identity(t[3]).unwrap().1,
        pm_model::Confidence::Fingerprint { score: 0.97 }
    );
}

/// A mock MusicBrainz (and AcoustID) that counts what it is asked.
#[derive(Default)]
struct Asked {
    searches: std::sync::atomic::AtomicUsize,
    lookups: std::sync::atomic::AtomicUsize,
    recordings: std::sync::atomic::AtomicUsize,
    acoustid: std::sync::atomic::AtomicUsize,
}

impl Asked {
    fn n(&self) -> (usize, usize, usize, usize) {
        use std::sync::atomic::Ordering::SeqCst;
        (
            self.searches.load(SeqCst),
            self.lookups.load(SeqCst),
            self.recordings.load(SeqCst),
            self.acoustid.load(SeqCst),
        )
    }
}

/// `releases`: album title → its editions (release JSON with `id`, `title`, `media`), searched by
/// `release:"<title>"`.
async fn mock_mb(releases: Vec<(String, Vec<serde_json::Value>)>, asked: Arc<Asked>) -> u16 {
    use std::sync::atomic::Ordering::SeqCst;
    let by_id: std::collections::HashMap<String, serde_json::Value> = releases
        .iter()
        .flat_map(|(_, rs)| {
            rs.iter()
                .map(|r| (r["id"].as_str().unwrap().to_string(), r.clone()))
        })
        .collect();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (a1, a2, a3, a4) = (asked.clone(), asked.clone(), asked.clone(), asked);
    let mock = Router::new()
        .route(
            "/ws/2/release",
            get(move |Query(q): Query<std::collections::HashMap<String, String>>| {
                a1.searches.fetch_add(1, SeqCst);
                let query = q.get("query").cloned().unwrap_or_default();
                let found: Vec<serde_json::Value> = releases
                    .iter()
                    .filter(|(title, _)| query.starts_with(&format!("release:\"{title}\"")))
                    .flat_map(|(_, rs)| rs.iter())
                    .map(|r| {
                        let count: usize = r["media"].as_array().unwrap().iter().map(|m| m["tracks"].as_array().unwrap().len()).sum();
                        serde_json::json!({"id": r["id"], "score": 100, "title": r["search_title"].as_str().unwrap_or(r["title"].as_str().unwrap()),
                            "track-count": count, "artist-credit": [{"name": "Band"}], "release-group": r["release-group"]})
                    })
                    .collect();
                async move { Json(serde_json::json!({"releases": found})) }
            }),
        )
        .route(
            "/ws/2/release/{id}",
            get(move |Path(id): Path<String>| {
                a2.lookups.fetch_add(1, SeqCst);
                let r = by_id.get(&id).cloned();
                async move {
                    match r {
                        Some(r) => Json(r).into_response(),
                        None => StatusCode::NOT_FOUND.into_response(),
                    }
                }
            }),
        )
        .route(
            "/ws/2/recording",
            get(move || {
                a3.recordings.fetch_add(1, SeqCst);
                async { Json(serde_json::json!({"recordings": []})) }
            }),
        )
        .route(
            "/v2/lookup",
            post(move || {
                a4.acoustid.fetch_add(1, SeqCst);
                async { Json(serde_json::json!({"status": "ok", "results": []})) }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    port
}

/// An edition of `title` with `n` tracks "Song i" (200 + i s); `other` puts other recordings
/// (900 + i) at those places, `titles` other names.
fn edition(id: &str, title: &str, n: u32, other: &[u32], titles: &[(u32, &str)]) -> serde_json::Value {
    serde_json::json!({"id": id, "title": title, "search_title": title, "media": [{"position": 1, "tracks": (1..=n).map(|i| serde_json::json!({
        "position": i,
        "title": titles.iter().find(|(k, _)| *k == i).map(|(_, t)| t.to_string()).unwrap_or(format!("Song {i}")),
        "length": (200 + i) * 1000,
        "recording": {"id": rec(if other.contains(&i) { 900 + i } else { i })}
    })).collect::<Vec<_>>()}]})
}

/// Album folder songs ripped a second longer than MusicBrainz says.
fn album_folder(s: &NodeState, dir: &std::path::Path, folder: &str, n: u32) -> Vec<i64> {
    (1..=n)
        .map(|i| {
            let id = song(s, dir, folder, i, &format!("Song {i}"));
            let lib = s.library.lock().unwrap();
            let mut t = lib.get_track(pm_model::TrackId(id)).unwrap().unwrap();
            t.duration_ms = Some((201 + i as u64) * 1000);
            t.file.duration_ms = t.duration_ms;
            lib.upsert_track(&t).unwrap();
            id
        })
        .collect()
}

/// The album lane with several editions: two that put the same recordings everywhere identify
/// the songs (the edition stays open); two that differ on a song leave that song as a choice of
/// both recordings, never one picked silently; a live track at a plain file's place doesn't fit
/// it.
#[tokio::test]
async fn equal_editions_and_versions_in_a_real_run() {
    const A1: &str = "a1a1a1a1-0000-4000-8000-000000000001";
    const A2: &str = "a2a2a2a2-0000-4000-8000-000000000002";
    const B1: &str = "b1b1b1b1-0000-4000-8000-000000000001";
    const B2: &str = "b2b2b2b2-0000-4000-8000-000000000002";
    const L1: &str = "c1c1c1c1-0000-4000-8000-000000000001";
    let asked = Arc::new(Asked::default());
    let port = mock_mb(
        vec![
            (
                "Twin".into(),
                vec![edition(A1, "Twin", 4, &[], &[]), edition(A2, "Twin", 4, &[], &[])],
            ),
            (
                "Split".into(),
                vec![
                    edition(B1, "Split", 4, &[], &[]),
                    edition(B2, "Split", 4, &[4], &[]),
                ],
            ),
            (
                "Stage".into(),
                vec![edition(L1, "Stage", 5, &[], &[(2, "Song 2 (Live)")])],
            ),
        ],
        asked.clone(),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("tests@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    let twin = album_folder(&s, dir.path(), "Band/Twin", 4);
    let split = album_folder(&s, dir.path(), "Band/Split", 4);
    let stage = album_folder(&s, dir.path(), "Band/Stage", 5);
    identify::run_with(&s, identify::RunOptions::quick(false))
        .await
        .unwrap();
    let lib = || s.library.lock().unwrap();
    let identity = |id: i64| lib().get_track(pm_model::TrackId(id)).unwrap().unwrap().canonical;
    let release_of = |id: i64| -> Option<String> {
        lib()
            .identified_not_tagged()
            .unwrap()
            .into_iter()
            .find(|(t, _)| t.id == Some(pm_model::TrackId(id)))
            .and_then(|(_, r)| r.map(|r| r.to_string()))
    };
    for (k, id) in twin.iter().enumerate() {
        let (uri, conf) = identity(*id).expect("same recordings on both editions");
        assert_eq!(
            uri.to_url(),
            format!("https://musicbrainz.org/recording/{}", rec(k as u32 + 1))
        );
        assert!(matches!(conf, pm_model::Confidence::AlbumFit { .. }));
        assert_eq!(release_of(*id), None, "which edition is left open");
    }
    for (k, id) in split.iter().take(3).enumerate() {
        assert_eq!(
            identity(*id).unwrap().0.to_url(),
            format!("https://musicbrainz.org/recording/{}", rec(k as u32 + 1))
        );
    }
    assert!(identity(split[3]).is_none(), "the editions disagree on song 4");
    let mut opts: Vec<(String, String, Option<String>)> = lib()
        .suggestions_for(split[3])
        .unwrap()
        .into_iter()
        .map(|o| (o.source, o.recording_mbid, o.release_mbid))
        .collect();
    opts.sort();
    assert_eq!(
        opts,
        vec![
            ("album".into(), rec(4), Some(B1.into())),
            ("album".into(), rec(904), Some(B2.into()))
        ]
    );
    // The live album's track 2 doesn't fit the plain file; the rest do (4 of 5)
    assert!(identity(stage[1]).is_none());
    assert!(stage
        .iter()
        .enumerate()
        .filter(|(k, _)| *k != 1)
        .all(|(_, id)| identity(*id).is_some()));
    // One search per album; both editions looked up only where both are equally near in size
    let (searches, lookups, _, acoustid) = asked.n();
    assert_eq!((searches, lookups, acoustid), (3, 5, 0));
    let (_, b) = call(&router(s.clone()), "GET", "/v1/library/identify", None).await;
    assert_eq!(b["phase"], "needs_review", "{b}");
}

/// Runs again reuse what earlier ones learnt: an unchanged song is not read, decoded or asked
/// about again (MusicBrainz and AcoustID answers, file keys and fingerprints are kept); a plain
/// run re-checks a "not found" song whose tags changed, and only that one; a retry still looks at
/// every "not found" song (cheaply); a recording the owner rejected stays out through all of it.
#[tokio::test]
async fn later_runs_reuse_answers_and_recheck_only_what_changed() {
    use std::sync::atomic::Ordering::SeqCst;
    const T1: &str = "a1a1a1a1-0000-4000-8000-000000000001";
    const S1: &str = "b1b1b1b1-0000-4000-8000-000000000001";
    const S2: &str = "b2b2b2b2-0000-4000-8000-000000000002";
    let asked = Arc::new(Asked::default());
    let port = mock_mb(
        vec![
            ("Twin".into(), vec![edition(T1, "Twin", 3, &[], &[])]),
            (
                "Split".into(),
                vec![
                    edition(S1, "Split", 2, &[], &[]),
                    edition(S2, "Split", 2, &[2], &[]),
                ],
            ),
        ],
        asked.clone(),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("tests@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    *s.acoustid.write().unwrap() = identify::AcoustIdSettings {
        api_key: "test-key".into(),
        base_url: format!("http://127.0.0.1:{port}"),
        configured: String::new(),
    };
    album_folder(&s, dir.path(), "Band/Twin", 3);
    let split = album_folder(&s, dir.path(), "Band/Split", 2);
    // Three loose songs with real audio: fingerprinted, AcoustID knows nothing, nor MusicBrainz
    let loose: Vec<i64> = (1..=3)
        .map(|i| {
            let path = dir.path().join("music/Loose").join(format!("{i:02} take.wav"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            tone_wav(&path, 2 + i);
            let mut t = untagged_track(path, &format!("Take {i}"), &format!("Someone {i}"));
            t.track_no = None;
            s.library.lock().unwrap().upsert_track(&t).unwrap().0
        })
        .collect();
    let opts = identify::RunOptions::quick(false);
    identify::run_with(&s, opts.clone()).await.unwrap();
    let first = asked.n();
    let w1 = s.identify.lock().unwrap().work.clone();
    assert_eq!(first.0, 2, "one search per album");
    assert_eq!(first.3, 3, "each loose song looked up on AcoustID");
    assert_eq!(w1.decoded, 3, "{w1:?}");
    assert!(w1.mb_asked >= 2 + 3 + 3, "{w1:?}");
    // The owner says "Not this" to both of Split's song 2 options
    let options = s.library.lock().unwrap().suggestions_for(split[1]).unwrap();
    assert_eq!(options.len(), 2, "the editions disagree on song 2");
    for o in options {
        assert!(s.library.lock().unwrap().reject_suggestion(o.id).unwrap());
    }

    // A retry: every "not found" song looked at again, from what is kept
    let retry = identify::RunOptions::quick(true);
    identify::run_with(&s, retry.clone()).await.unwrap();
    let p = s.identify.lock().unwrap().clone();
    assert_eq!(p.done, 4, "the three loose songs and the rejected one: {p:?}");
    assert_eq!(asked.n(), first, "nothing asked again");
    assert_eq!(
        (
            p.work.decoded,
            p.work.file_reads,
            p.work.mb_asked,
            p.work.acoustid_asked
        ),
        (0, 0, 0, 0),
        "{:?}",
        p.work
    );
    assert_eq!(p.work.lookups_reused, 3, "{:?}", p.work);
    assert!(p.work.mb_reused >= 4, "{:?}", p.work);
    assert!(
        s.library
            .lock()
            .unwrap()
            .suggestions_for(split[1])
            .unwrap()
            .is_empty(),
        "rejected stay out"
    );
    assert!(s
        .library
        .lock()
        .unwrap()
        .get_track(pm_model::TrackId(split[1]))
        .unwrap()
        .unwrap()
        .canonical
        .is_none());

    // A plain run with nothing new: nothing to do
    identify::run_with(&s, opts.clone()).await.unwrap();
    assert_eq!(s.identify.lock().unwrap().total, 0);
    // The owner fixes one loose song's tags: a plain run re-checks it alone, by its new tags
    {
        let lib = s.library.lock().unwrap();
        let mut t = lib.get_track(pm_model::TrackId(loose[0])).unwrap().unwrap();
        t.title = "Take One".into();
        lib.upsert_track(&t).unwrap();
    }
    identify::run_with(&s, opts).await.unwrap();
    let p = s.identify.lock().unwrap().clone();
    assert_eq!((p.total, p.done), (1, 1), "{p:?}");
    assert_eq!(p.work.decoded, 0, "the same audio: {:?}", p.work);
    assert_eq!(asked.acoustid.load(SeqCst), 3, "its kept AcoustID answer");
    assert!(asked.recordings.load(SeqCst) > first.2, "its new title searched");
    // The status of these runs: the last one finished, nothing to review
    let (_, b) = call(&router(s.clone()), "GET", "/v1/library/identify", None).await;
    assert_eq!(b["phase"], "ready", "{b}");
}

/// An edition with release-group types (`None`: MusicBrainz says none) and `discs` media of
/// `per_disc` tracks; recordings `base + disc * 10 + i`.
fn typed(
    id: &str,
    title: &str,
    kind: Option<(&str, &[&str])>,
    discs: u32,
    per_disc: u32,
    other: &[u32],
) -> serde_json::Value {
    let mut r = serde_json::json!({"id": id, "title": title, "search_title": title, "media": (1..=discs).map(|d| serde_json::json!({
        "position": d, "tracks": (1..=per_disc).map(|i| serde_json::json!({
            "position": i, "title": format!("Song {}", (d - 1) * per_disc + i), "length": (200 + (d - 1) * per_disc + i) * 1000,
            "recording": {"id": rec(if other.contains(&((d - 1) * per_disc + i)) { 900 } else { 0 } + (d - 1) * per_disc + i)}
        })).collect::<Vec<_>>()})).collect::<Vec<_>>()});
    if let Some((primary, secondary)) = kind {
        r["release-group"] = serde_json::json!({"primary-type": primary, "secondary-types": secondary});
    }
    r
}

/// The owner's refinement: the recording is the identity; which release (album, single, EP,
/// compilation, an edition) is said only when MusicBrainz's own release-group types or the
/// folder single one out, never picked from the order of an answer, and never decides between
/// two recordings.
#[tokio::test]
async fn release_types_choose_the_release_never_the_recording() {
    const HA: &str = "aa000000-0000-4000-8000-000000000001";
    const HC: &str = "aa000000-0000-4000-8000-000000000002";
    const MA: &str = "bb000000-0000-4000-8000-000000000001";
    const MC: &str = "bb000000-0000-4000-8000-000000000002";
    const UA: &str = "cc000000-0000-4000-8000-000000000001";
    const UX: &str = "cc000000-0000-4000-8000-000000000002";
    const EP: &str = "dd000000-0000-4000-8000-000000000001";
    const SG: &str = "dd000000-0000-4000-8000-000000000002";
    const DD: &str = "ee000000-0000-4000-8000-000000000001";
    const STD: &str = "ff000000-0000-4000-8000-000000000001";
    const DLX: &str = "ff000000-0000-4000-8000-000000000002";
    let album: Option<(&str, &[&str])> = Some(("Album", &[]));
    let comp: Option<(&str, &[&str])> = Some(("Album", &["Compilation"]));
    let asked = Arc::new(Asked::default());
    let port = mock_mb(
        vec![
            // The same recordings on an album and a compilation of the same name
            (
                "Hits".into(),
                vec![
                    typed(HC, "Hits", comp, 1, 3, &[]),
                    typed(HA, "Hits", album, 1, 3, &[]),
                ],
            ),
            // ... with another recording at place 3 on the compilation: a choice, types or not
            (
                "Mixed".into(),
                vec![
                    typed(MA, "Mixed", album, 1, 3, &[]),
                    typed(MC, "Mixed", comp, 1, 3, &[3]),
                ],
            ),
            // The same recordings, one release without a type: which one stays open
            (
                "Plain".into(),
                vec![
                    typed(UA, "Plain", album, 1, 3, &[]),
                    typed(UX, "Plain", None, 1, 3, &[]),
                ],
            ),
            // An EP and a single of one name: the folder of four is the EP; the single isn't asked for
            (
                "Four".into(),
                vec![
                    typed(SG, "Four", Some(("Single", &[])), 1, 1, &[]),
                    typed(EP, "Four", Some(("EP", &[])), 1, 4, &[]),
                ],
            ),
            // Two discs
            ("Double".into(), vec![typed(DD, "Double", album, 2, 3, &[])]),
            // A deluxe edition with a bonus track, and the standard one
            (
                "Bonus".into(),
                vec![
                    typed(STD, "Bonus", album, 1, 3, &[]),
                    typed(DLX, "Bonus", album, 1, 4, &[]),
                ],
            ),
        ],
        asked.clone(),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("tests@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    let hits = album_folder(&s, dir.path(), "Band/Hits", 3);
    let mixed = album_folder(&s, dir.path(), "Band/Mixed", 3);
    let plain = album_folder(&s, dir.path(), "Band/Plain", 3);
    let four = album_folder(&s, dir.path(), "Band/Four", 4);
    let bonus = album_folder(&s, dir.path(), "Band/Bonus", 4);
    // Two discs in one folder: songs 1-3 on disc 1, 4-6 on disc 2 (numbered 1-3 there)
    let double: Vec<i64> = album_folder(&s, dir.path(), "Band/Double", 6)
        .into_iter()
        .enumerate()
        .map(|(k, id)| {
            let lib = s.library.lock().unwrap();
            let mut t = lib.get_track(pm_model::TrackId(id)).unwrap().unwrap();
            t.disc_no = Some(if k < 3 { 1 } else { 2 });
            t.track_no = Some(k as u32 % 3 + 1);
            lib.upsert_track(&t).unwrap();
            id
        })
        .collect();
    identify::run_with(&s, identify::RunOptions::quick(false))
        .await
        .unwrap();
    let lib = || s.library.lock().unwrap();
    let identity = |id: i64| {
        lib()
            .get_track(pm_model::TrackId(id))
            .unwrap()
            .unwrap()
            .canonical
            .map(|(u, _)| u.to_url())
    };
    let rec_url = |i: u32| Some(format!("https://musicbrainz.org/recording/{}", rec(i)));
    let release_of = |id: i64| -> Option<String> {
        lib()
            .identified_not_tagged()
            .unwrap()
            .into_iter()
            .find(|(t, _)| t.id == Some(pm_model::TrackId(id)))
            .and_then(|(_, r)| r.map(|r| r.to_string()))
    };
    for (k, id) in hits.iter().enumerate() {
        assert_eq!(identity(*id), rec_url(k as u32 + 1));
        assert_eq!(release_of(*id).as_deref(), Some(HA), "the album, by its type");
    }
    assert_eq!(identity(mixed[0]), rec_url(1));
    assert_eq!(identity(mixed[2]), None, "two recordings: the owner chooses");
    let mut opts: Vec<String> = lib()
        .suggestions_for(mixed[2])
        .unwrap()
        .into_iter()
        .map(|o| o.recording_mbid)
        .collect();
    opts.sort();
    assert_eq!(opts, vec![rec(3), rec(903)]);
    for (k, id) in plain.iter().enumerate() {
        assert_eq!(identity(*id), rec_url(k as u32 + 1));
        assert_eq!(
            release_of(*id),
            None,
            "a release without a type isn't taken for anything"
        );
    }
    for (k, id) in four.iter().enumerate() {
        assert_eq!(identity(*id), rec_url(k as u32 + 1));
        assert_eq!(release_of(*id).as_deref(), Some(EP));
    }
    for (k, id) in double.iter().enumerate() {
        assert_eq!(identity(*id), rec_url(k as u32 + 1), "disc {}", k / 3 + 1);
        assert_eq!(release_of(*id).as_deref(), Some(DD));
    }
    for (k, id) in bonus.iter().enumerate() {
        assert_eq!(identity(*id), rec_url(k as u32 + 1));
        assert_eq!(
            release_of(*id).as_deref(),
            Some(DLX),
            "the edition with the bonus track"
        );
    }
    // One search per folder; every bounded candidate looked up (fix round 1, item 2: Four's
    // single and Bonus's standard edition too; neither changes what is stored)
    let (searches, lookups, _, _) = asked.n();
    assert_eq!((searches, lookups), (6, 2 + 2 + 2 + 2 + 1 + 2));
}

/// The fingerprint's side of the refinement: a song AcoustID knows on several releases gets its
/// recording; the release only when one fits the folder (a single in a folder of its own), never
/// the first of several that fit.
#[tokio::test]
async fn a_fingerprinted_song_on_a_single_and_an_album_gets_its_recording() {
    const SINGLE: &str = "51000000-0000-4000-8000-000000000001";
    const SINGLE2: &str = "51000000-0000-4000-8000-000000000002";
    const ALBUM: &str = "a1000000-0000-4000-8000-000000000001";
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    *s.acoustid.write().unwrap() = identify::AcoustIdSettings {
        api_key: "test-key".into(),
        base_url: "http://127.0.0.1:9".into(),
        configured: String::new(),
    };
    let rel = |id: &str, total: u32, pos: u32| serde_json::json!({"id": id, "track_count": total, "mediums": [{"position": 1, "track_count": total, "tracks": [{"position": pos}]}]});
    let mut songs = Vec::new();
    for (i, releases) in [
        // On a one-track single and on a twelve-track album
        serde_json::json!([{"id": "g1", "type": "Single", "releases": [rel(SINGLE, 1, 1)]}, {"id": "g2", "type": "Album", "releases": [rel(ALBUM, 12, 4)]}]),
        // On two editions of a one-track single
        serde_json::json!([{"id": "g3", "type": "Single", "releases": [rel(SINGLE, 1, 1), rel(SINGLE2, 1, 1)]}]),
    ]
    .into_iter()
    .enumerate()
    {
        let path = dir.path().join(format!("music/Single {i}/01 song.wav"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        tone_wav(&path, 2 + i as u32);
        let mut t = untagged_track(path.clone(), "Song", "Band");
        t.track_no = Some(1);
        t.duration_ms = Some(200_000);
        let id = s.library.lock().unwrap().upsert_track(&t).unwrap().0;
        let key = Library::file_key(&path).unwrap();
        let lib = s.library.lock().unwrap();
        lib.save_fingerprint(&key, 200, "AQfake").unwrap();
        let answer = serde_json::json!({"status": "ok", "results": [{"id": format!("acoustid-{i}"), "score": 0.97,
            "recordings": [{"id": rec(70 + i as u32), "title": "Song", "duration": 200, "releasegroups": releases}]}]});
        lib.save_lookup(&key, &answer.to_string()).unwrap();
        songs.push(id);
    }
    identify::run_with(&s, identify::RunOptions::quick(false))
        .await
        .unwrap();
    let lib = || s.library.lock().unwrap();
    let release_of = |id: i64| -> Option<String> {
        lib()
            .identified_not_tagged()
            .unwrap()
            .into_iter()
            .find(|(t, _)| t.id == Some(pm_model::TrackId(id)))
            .and_then(|(_, r)| r.map(|r| r.to_string()))
    };
    for (i, id) in songs.iter().enumerate() {
        let (uri, conf) = lib()
            .get_track(pm_model::TrackId(*id))
            .unwrap()
            .unwrap()
            .canonical
            .expect("identified");
        assert_eq!(
            uri.to_url(),
            format!("https://musicbrainz.org/recording/{}", rec(70 + i as u32))
        );
        assert!(
            matches!(conf, pm_model::Confidence::FingerprintRelease { .. }),
            "{conf:?}"
        );
    }
    assert_eq!(
        release_of(songs[0]).as_deref(),
        Some(SINGLE),
        "the only release that fits a folder of one"
    );
    assert_eq!(
        release_of(songs[1]),
        None,
        "two singles fit: which one stays open"
    );
}

/// The folder names one recording and an earlier run's fingerprint, sure of itself, names another
/// of the same name (a live take, another master): a conflict between two ways of matching, left
/// for the owner with both, never settled by the title.
#[tokio::test]
async fn a_fingerprint_naming_another_recording_of_the_same_name_is_a_conflict() {
    const REL1: &str = "f1000000-0000-4000-8000-000000000001";
    const LIVE: &str = "f2000000-0000-4000-8000-000000000002";
    let asked = Arc::new(Asked::default());
    let port = mock_mb(
        vec![("Album".into(), vec![edition(REL1, "Album", 3, &[], &[])])],
        asked.clone(),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("tests@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    // Real files: the second opinion is read by content
    let ids: Vec<i64> = (1..=3)
        .map(|i| {
            let path = dir.path().join(format!("music/Band/Album/{i:02} Song {i}.wav"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            tone_wav(&path, 1 + i);
            let mut t = untagged_track(path, &format!("Song {i}"), "Band");
            t.track_no = Some(i);
            t.disc_no = None;
            t.duration_ms = Some((201 + i as u64) * 1000);
            t.album = Some(pm_model::Album {
                id: None,
                title: "Album".into(),
                album_artist: Some("Band".into()),
                release_mbid: None,
                release_group_mbid: None,
                year: None,
                upc: None,
                total_discs: None,
            });
            s.library.lock().unwrap().upsert_track(&t).unwrap().0
        })
        .collect();
    let cached = |id: i64, recording: &str, title: &str| {
        let lib = s.library.lock().unwrap();
        let t = lib.get_track(pm_model::TrackId(id)).unwrap().unwrap();
        let key = Library::file_key(&t.file.path).unwrap();
        lib.save_fingerprint(&key, 3, "AQfake").unwrap();
        let answer = serde_json::json!({"status": "ok", "results": [{"id": "9ff43b6a-4f16-427c-93c2-92307ca505e0",
            "score": 0.95, "recordings": [{"id": recording, "title": title}]}]});
        lib.save_lookup(&key, &answer.to_string()).unwrap();
    };
    cached(ids[0], &rec(1), "Song 1");
    cached(ids[1], LIVE, "Song 2 (Live)");
    identify::run_with(&s, identify::RunOptions::quick(false))
        .await
        .unwrap();
    let identity = |id: i64| {
        s.library
            .lock()
            .unwrap()
            .get_track(pm_model::TrackId(id))
            .unwrap()
            .unwrap()
            .canonical
    };
    let (_, conf) = identity(ids[0]).expect("the fingerprint agrees");
    assert!(matches!(conf, pm_model::Confidence::AlbumFit { score } if score > 0.95));
    assert!(identity(ids[2]).is_some(), "no second opinion: the folder");
    assert!(identity(ids[1]).is_none(), "the two ways disagree");
    let mut opts: Vec<(String, String)> = s
        .library
        .lock()
        .unwrap()
        .suggestions_for(ids[1])
        .unwrap()
        .into_iter()
        .map(|o| (o.source, o.recording_mbid))
        .collect();
    opts.sort();
    assert_eq!(
        opts,
        vec![("album".into(), rec(2)), ("fingerprint".into(), LIVE.into())]
    );
}

/// A 4 s mono 11 025 Hz tone, its pitch from `seed` (each file its own audio, ~88 KB).
fn small_wav(path: &std::path::Path, seed: u32) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 11_025,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    let f = 150.0 + seed as f64 * 0.5;
    for i in 0..(4 * 11_025) {
        let t = i as f64 / 11_025.0;
        let g = if ((t * 4.0) as u32).is_multiple_of(2) {
            f
        } else {
            f * 1.5
        };
        w.write_sample(((t * g * std::f64::consts::TAU).sin() * 9_000.0) as i16)
            .unwrap();
    }
    w.finalize().unwrap();
}

/// A stable small hash for the mock's choices.
fn h(s: &str) -> u64 {
    s.bytes().fold(1469598103934665603u64, |a, b| {
        (a ^ b as u64).wrapping_mul(1099511628211)
    })
}

/// The replay benchmark (run on demand: `cargo test -p pm-api --lib identify_replay_19k -- --ignored
/// --nocapture`; `PM_REPLAY_REPORT=<file>` also writes the report). A library shaped like the
/// owner's (19 452 songs: 9 721 identified by their tags, 8 280 in 690 album folders, 1 300 loose
/// songs with real audio, 60 whose audio is too short to fingerprint, 91 whose files are gone)
/// against mock MusicBrainz and AcoustID that
/// count every request. Three runs: cold; a retry with nothing changed; a plain run after 50 loose
/// songs' tags changed. Times are this machine's, unpaced (the mocks answer at once), so they are
/// no promise for a real node; the paced estimate counts MusicBrainz's 1 request a second and
/// AcoustID's 3.
#[tokio::test]
#[ignore = "benchmark: run on demand"]
async fn identify_replay_19k() {
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    const TAGGED: u32 = 9_721;
    const ALBUMS: u32 = 635;
    const TWINS: u32 = 40;
    const SPLITS: u32 = 15;
    const LOOSE: u32 = 1_300;
    const SHORT: u32 = 60;
    const GONE: u32 = 91;
    let release_id = |folder: u32, edition: u32| format!("{folder:08x}-0000-4000-8000-{edition:012x}");
    let recording_id = |folder: u32, track: u32, alt: bool| {
        format!(
            "{folder:08x}-0000-4000-{}-{track:012x}",
            if alt { "a000" } else { "9000" }
        )
    };
    #[derive(Default)]
    struct Count {
        searches: AtomicUsize,
        lookups: AtomicUsize,
        recordings: AtomicUsize,
        acoustid: AtomicUsize,
    }
    let count = Arc::new(Count::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (c1, c2, c3, c4) = (count.clone(), count.clone(), count.clone(), count.clone());
    // Folders 0..ALBUMS: one release; then TWINS: two editions, same recordings; then SPLITS: two
    // editions whose tracks 11 and 12 are other recordings
    let editions = move |folder: u32| if folder < ALBUMS { 1 } else { 2 };
    let mock = Router::new()
        .route(
            "/ws/2/release",
            get(move |Query(q): Query<std::collections::HashMap<String, String>>| {
                c1.searches.fetch_add(1, SeqCst);
                let query = q.get("query").cloned().unwrap_or_default();
                let folder: u32 = query
                    .strip_prefix("release:\"Album ")
                    .and_then(|r| r.split('"').next())
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(u32::MAX);
                let found: Vec<serde_json::Value> = (1..=if folder == u32::MAX { 0 } else { editions(folder) })
                    .map(|e| serde_json::json!({"id": release_id(folder, e), "score": 100, "title": format!("Album {folder:04}"),
                        "track-count": 12, "artist-credit": [{"name": format!("Artist {:03}", folder % 300)}],
                        "release-group": {"primary-type": "Album"}}))
                    .collect();
                async move { Json(serde_json::json!({"releases": found})) }
            }),
        )
        .route(
            "/ws/2/release/{id}",
            get(move |Path(id): Path<String>| {
                c2.lookups.fetch_add(1, SeqCst);
                let folder = u32::from_str_radix(&id[..8], 16).unwrap();
                let edition = u32::from_str_radix(&id[24..], 16).unwrap();
                let split = folder >= ALBUMS + TWINS && edition == 2;
                let tracks: Vec<serde_json::Value> = (1..=12)
                    .map(|i| serde_json::json!({"position": i, "title": format!("Track {i:02}"), "length": (180 + i) * 1000,
                        "recording": {"id": recording_id(folder, i, split && i > 10)}}))
                    .collect();
                async move { Json(serde_json::json!({"id": id, "title": format!("Album {folder:04}"), "release-group": {"primary-type": "Album"}, "media": [{"position": 1, "tracks": tracks}]})) }
            }),
        )
        .route(
            "/ws/2/recording",
            get(move |Query(q): Query<std::collections::HashMap<String, String>>| {
                c3.recordings.fetch_add(1, SeqCst);
                let query = q.get("query").cloned().unwrap_or_default();
                let title = query.strip_prefix("recording:\"").and_then(|r| r.split('"').next()).unwrap_or_default().to_string();
                // A third of the searches find two recordings of that name
                let hits: Vec<serde_json::Value> = if h(&title).is_multiple_of(3) {
                    (0..2u64)
                        .map(|k| serde_json::json!({"id": format!("{:08x}-0000-4000-b000-{k:012x}", h(&title) as u32), "score": 100, "title": title, "length": 2000,
                            "artist-credit": [{"name": "Someone"}], "releases": [{"id": format!("{:08x}-0000-4000-c000-{k:012x}", h(&title) as u32), "title": "Somewhere", "status": "Official"}]}))
                        .collect()
                } else {
                    Vec::new()
                };
                async move { Json(serde_json::json!({"recordings": hits})) }
            }),
        )
        .route(
            "/v2/lookup",
            post(move |body: String| {
                c4.acoustid.fetch_add(1, SeqCst);
                // Half the audio is known, as one recording
                let known = h(&body).is_multiple_of(2);
                async move {
                    Json(if known {
                        serde_json::json!({"status": "ok", "results": [{"id": "acoustid-x", "score": 0.95, "recordings": [{"id": format!("{:08x}-0000-4000-d000-000000000001", h(&body) as u32)}]}]})
                    } else {
                        serde_json::json!({"status": "ok", "results": []})
                    })
                }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("tests@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    *s.acoustid.write().unwrap() = identify::AcoustIdSettings {
        api_key: "test-key".into(),
        base_url: format!("http://127.0.0.1:{port}"),
        configured: String::new(),
    };
    let built = std::time::Instant::now();
    let music = dir.path().join("music");
    let mut loose_ids = Vec::new();
    let mut short_ids = Vec::new();
    {
        let lib = s.library.lock().unwrap();
        let mut n = 0u32;
        let mut add = |t: pm_model::Track| {
            if n.is_multiple_of(500) {
                lib.commit_batch().unwrap();
                lib.begin_batch().unwrap();
            }
            n += 1;
            lib.upsert_track(&t).unwrap().0
        };
        for i in 0..TAGGED {
            let mut t = untagged_track(
                music.join(format!("Tagged/{:03}/{i:05}.flac", i / 12)),
                &format!("Tagged {i}"),
                "Tagged Artist",
            );
            t.canonical = Some((
                pm_model::MusicUri::Recording {
                    mbid: pm_model::Mbid::parse(&format!("{i:08x}-0000-4000-e000-000000000000")).unwrap(),
                },
                pm_model::Confidence::TaggedMbid,
            ));
            add(t);
        }
        for folder in 0..ALBUMS + TWINS + SPLITS {
            for i in 1..=12u32 {
                let mut t = untagged_track(
                    music.join(format!(
                        "Artist {:03}/Album {folder:04}/{i:02}.flac",
                        folder % 300
                    )),
                    &format!("Track {i:02}"),
                    &format!("Artist {:03}", folder % 300),
                );
                t.track_no = Some(i);
                t.disc_no = None;
                t.duration_ms = Some((181 + i as u64) * 1000);
                t.album = Some(pm_model::Album {
                    id: None,
                    title: format!("Album {folder:04}"),
                    album_artist: Some(format!("Artist {:03}", folder % 300)),
                    release_mbid: None,
                    release_group_mbid: None,
                    year: None,
                    upc: None,
                    total_discs: None,
                });
                add(t);
            }
        }
        for i in 0..LOOSE + SHORT + GONE {
            let path = music.join(format!("Loose/{:02}/{i:05} loose.wav", i / 100));
            if i < LOOSE + SHORT {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            }
            if i < LOOSE {
                small_wav(&path, i);
            } else if i < LOOSE + SHORT {
                // Too short to fingerprint (0.4 s), each its own
                let spec = hound::WavSpec {
                    channels: 1,
                    sample_rate: 11_025,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                };
                let mut w = hound::WavWriter::create(&path, spec).unwrap();
                for k in 0..4_410 {
                    w.write_sample(((k as f64 * (i as f64 + 1.0) * 0.01).sin() * 8_000.0) as i16)
                        .unwrap();
                }
                w.finalize().unwrap();
            }
            let mut t = untagged_track(path, &format!("Loose {i:05}"), &format!("Someone {}", i % 40));
            t.track_no = None;
            t.duration_ms = Some(4000);
            t.file.duration_ms = Some(4000);
            let id = add(t);
            if i < LOOSE {
                loose_ids.push(id);
            } else if i < LOOSE + SHORT {
                short_ids.push(id);
            }
        }
        lib.commit_batch().unwrap();
    }
    let built = built.elapsed();
    let total = s.library.lock().unwrap().identity_counts().unwrap().total;
    assert_eq!(total, 19_452);
    let asked = || {
        (
            count.searches.load(SeqCst) + count.lookups.load(SeqCst) + count.recordings.load(SeqCst),
            count.acoustid.load(SeqCst),
        )
    };
    fn entry(
        name: &str,
        p: &identify::IdentifyProgress,
        took: std::time::Duration,
        (mb, ac): (usize, usize),
    ) -> serde_json::Value {
        let e = serde_json::json!({
            "run": name, "songs_checked": p.done, "identified": p.identified, "suggested": p.suggested, "unknown": p.unknown, "errors": p.errors,
            "musicbrainz_requests": mb, "acoustid_requests": ac,
            "musicbrainz_reused": p.work.mb_reused, "acoustid_reused": p.work.lookups_reused,
            "files_read_for_key": p.work.file_reads, "file_keys_reused": p.work.file_keys_reused, "decoded": p.work.decoded,
            "measured_unpaced_secs": (took.as_secs_f64() * 100.0).round() / 100.0,
            "paced_estimate_secs": mb as f64 * 1.0 + ac as f64 * 0.34,
        });
        println!("{e}");
        e
    }
    let delta = |a: (usize, usize), b: (usize, usize)| (b.0 - a.0, b.1 - a.1);
    let mut report = Vec::new();
    let run = |retry: bool| {
        let s = s.clone();
        async move {
            let t = std::time::Instant::now();
            identify::run_with(&s, identify::RunOptions::quick(retry))
                .await
                .unwrap();
            let took = t.elapsed();
            (s.identify.lock().unwrap().clone(), took)
        }
    };
    // 1. Cold
    let (p1, t1) = run(false).await;
    let after1 = asked();
    report.push(entry("cold", &p1, t1, delta((0, 0), after1)));
    assert_eq!(
        p1.done as u32,
        (ALBUMS + TWINS + SPLITS) * 12 + LOOSE + SHORT + GONE
    );
    assert_eq!(
        p1.work.decoded as u32,
        LOOSE + SHORT,
        "every file with audio decoded once"
    );
    // 2. A retry, nothing changed: every "not found" song again, nothing asked, read or decoded
    let (p2, t2) = run(true).await;
    let after2 = asked();
    report.push(entry("retry_unchanged", &p2, t2, delta(after1, after2)));
    assert_eq!(p2.done, p1.unknown + p1.errors);
    assert_eq!(after1, after2, "nothing asked again");
    assert_eq!(
        (
            p2.work.mb_asked,
            p2.work.acoustid_asked,
            p2.work.decoded,
            p2.work.file_reads
        ),
        (0, 0, 0, 0),
        "{:?}",
        p2.work
    );
    // The too-short songs a retry looks at (those a search didn't give suggestions to) are not
    // decoded again
    let short_retried = {
        let lib = s.library.lock().unwrap();
        short_ids
            .iter()
            .filter(|id| lib.suggestions_for(**id).unwrap().is_empty())
            .count()
    };
    assert!(short_retried > 0);
    assert_eq!(
        p2.work.failures_reused, short_retried,
        "too-short audio is not decoded again: {:?}",
        p2.work
    );
    // 3. 50 "not found" loose songs get new tags: a plain run checks those alone
    let changed: Vec<i64> = {
        let lib = s.library.lock().unwrap();
        let open: std::collections::HashSet<i64> = lib.identify_inputs().unwrap().into_keys().collect();
        loose_ids
            .iter()
            .copied()
            .filter(|id| open.contains(id))
            .take(50)
            .collect()
    };
    for id in &changed {
        let lib = s.library.lock().unwrap();
        let mut t = lib.get_track(pm_model::TrackId(*id)).unwrap().unwrap();
        t.title = format!("{} (fixed)", t.title);
        lib.upsert_track(&t).unwrap();
    }
    let (p3, t3) = run(false).await;
    report.push(entry("plain_after_50_tag_fixes", &p3, t3, delta(after2, asked())));
    assert_eq!(p3.done, changed.len());
    assert_eq!(
        (p3.work.decoded, p3.work.file_reads, p3.work.acoustid_asked),
        (0, 0, 0),
        "{:?}",
        p3.work
    );
    let out = serde_json::json!({
        "library": total, "built_secs": built.as_secs_f64(), "runs": report, "last_error_cold": p1.last_error,
        "scope": "synthetic replay against in-process mocks on the build host; times unpaced; not a measurement of a real node",
    });
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
    if let Ok(path) = std::env::var("PM_REPLAY_REPORT") {
        std::fs::write(path, serde_json::to_string_pretty(&out).unwrap()).unwrap();
    }
}

/// A file whose audio can't be fingerprinted (too short) is decoded once: a retry with the file
/// unchanged says so again without decoding it; a changed file is tried again.
#[tokio::test]
async fn a_file_that_cannot_be_fingerprinted_is_not_decoded_again() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    *s.acoustid.write().unwrap() = identify::AcoustIdSettings {
        api_key: "test-key".into(),
        base_url: "http://127.0.0.1:9".into(),
        configured: String::new(),
    };
    let path = dir.path().join("music/Short/01 blip.wav");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    write_wav(&path, 2_000);
    let id = s
        .library
        .lock()
        .unwrap()
        .upsert_track(&untagged_track(path.clone(), "Blip", "Band"))
        .unwrap()
        .0;
    identify::run_with(&s, identify::RunOptions::quick(false))
        .await
        .unwrap();
    let p = s.identify.lock().unwrap().clone();
    assert_eq!((p.errors, p.work.decoded), (1, 1), "{p:?}");
    let first = p.last_error.clone().unwrap();
    assert!(first.contains("too short"), "{first}");
    identify::run_with(&s, identify::RunOptions::quick(true))
        .await
        .unwrap();
    let p = s.identify.lock().unwrap().clone();
    assert_eq!((p.done, p.errors, p.work.decoded), (1, 1, 0), "{p:?}");
    assert!(p.last_error.unwrap().contains("too short"));
    // The file changes: tried again
    write_wav(&path, 3_000);
    identify::run_with(&s, identify::RunOptions::quick(true))
        .await
        .unwrap();
    assert_eq!(s.identify.lock().unwrap().work.decoded, 1);
    assert!(s
        .library
        .lock()
        .unwrap()
        .get_track(pm_model::TrackId(id))
        .unwrap()
        .unwrap()
        .canonical
        .is_none());
}

/// "Which song is this?" searches again now; a recording the owner rejected for the song doesn't
/// come back, and when nothing else is found the song isn't marked as waiting for review (it would
/// be left out of every retry).
#[tokio::test]
async fn which_song_never_brings_back_a_rejected_recording() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let one = rec(5);
    let mock = Router::new().route(
        "/ws/2/recording",
        get(move || {
            let one = one.clone();
            async move {
                Json(serde_json::json!({"recordings": [{"id": one, "score": 100, "title": "Song", "length": 200_000,
                    "artist-credit": [{"name": "Band"}], "releases": [{"id": REL, "title": "Album", "status": "Official"}]}]}))
            }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("tests@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    let mut t = untagged_track(dir.path().join("music/x/01 Song.flac"), "Song", "Band");
    t.duration_ms = Some(200_000);
    let id = s.library.lock().unwrap().upsert_track(&t).unwrap().0;
    let app = router(s.clone());
    let (st, b) = call(&app, "GET", &format!("/v1/library/tracks/{id}/candidates"), None).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["suggestions"].as_array().unwrap().len(), 1, "{b}");
    let sid = b["suggestions"][0]["id"].as_i64().unwrap();
    let (st, _) = call(&app, "DELETE", &format!("/v1/library/suggestions/{sid}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (_, b) = call(&app, "GET", &format!("/v1/library/tracks/{id}/candidates"), None).await;
    assert!(b["suggestions"].as_array().unwrap().is_empty(), "{b}");
    let todo: Vec<i64> = s
        .library
        .lock()
        .unwrap()
        .tracks_to_identify(true)
        .unwrap()
        .into_iter()
        .filter_map(|t| t.id.map(|i| i.0))
        .collect();
    assert!(todo.contains(&id), "still a song a retry looks at");
}

// ---- Fix round 1 (independent reviews r1) ----

/// Fix round 1, item 1: the owner confirms another recording, or rejects the one found, while
/// the run's write for that song waits on a busy library (a scan writing). The run's answer is
/// stored only if it still holds when it is written: the owner's recording keeps its state and
/// no release of the run's; a rejected recording is not stored; only real transitions count.
#[tokio::test]
async fn owner_choices_made_while_a_run_waits_to_write_stand() {
    use std::sync::atomic::Ordering::SeqCst;
    const RX: &str = "a1000000-0000-4000-8000-0000000000aa";
    const RY: &str = "a2000000-0000-4000-8000-0000000000bb";
    const RZ: &str = "a3000000-0000-4000-8000-0000000000cc";
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let node: Arc<std::sync::OnceLock<Arc<NodeState>>> = Default::default();
    node.set(s.clone()).ok().unwrap();
    // The owner's action when a folder's release is looked up: (release, suggestion, accept?)
    let actions: Arc<std::sync::Mutex<std::collections::HashMap<String, (i64, bool)>>> = Default::default();
    let lookups = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let release = |id: &str, title: &str, r: u32| {
        serde_json::json!({"id": id, "title": title, "media": [{"position": 1, "tracks": [
            {"position": 1, "title": "Song 1", "length": 201_000, "recording": {"id": rec(r)}}]}]})
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mock = {
        let (node, actions, lookups) = (node.clone(), actions.clone(), lookups.clone());
        let db = dir.path().join("lib.sqlite");
        let rels = [
            (RX, release(RX, "X", 1)),
            (RY, release(RY, "Y", 2)),
            (RZ, release(RZ, "Z", 3)),
        ];
        let rels2 = rels.clone();
        Router::new()
            .route(
                "/ws/2/release",
                get(move |Query(q): Query<std::collections::HashMap<String, String>>| {
                    let query = q.get("query").cloned().unwrap_or_default();
                    let found: Vec<serde_json::Value> = rels
                        .iter()
                        .filter(|(_, r)| query.starts_with(&format!("release:\"{}\"", r["title"].as_str().unwrap())))
                        .map(|(id, r)| serde_json::json!({"id": id, "score": 100, "title": r["title"], "track-count": 1, "artist-credit": [{"name": "Band"}]}))
                        .collect();
                    async move { Json(serde_json::json!({"releases": found})) }
                }),
            )
            .route(
                "/ws/2/release/{id}",
                get(move |Path(id): Path<String>| {
                    lookups.fetch_add(1, SeqCst);
                    let r = rels2.iter().find(|(x, _)| *x == id).unwrap().1.clone();
                    let action = actions.lock().unwrap().get(&id).copied();
                    let (node, db) = (node.clone(), db.clone());
                    async move {
                        if let Some((sid, accept)) = action {
                            // A scan's connection takes the write lock; the owner's choice lands
                            // in it, after the run's write for this song has been turned away
                            let (locked_tx, locked_rx) = std::sync::mpsc::channel();
                            let s = node.get().unwrap().clone();
                            std::thread::spawn(move || {
                                let other = Library::open(&db).unwrap();
                                other.begin_batch().unwrap();
                                locked_tx.send(()).unwrap();
                                // Two waits of the run end in a refusal: keeping the answer, then the song's write
                                let mut refused = 0;
                                let mut held_since: Option<std::time::Instant> = None;
                                let give_up = std::time::Instant::now() + 8 * pm_library::db::BUSY_TIMEOUT;
                                while refused < 2 && std::time::Instant::now() < give_up {
                                    let now = std::time::Instant::now();
                                    if s.library.try_lock().is_ok() {
                                        if held_since.is_some_and(|t| now - t >= pm_library::db::BUSY_TIMEOUT / 2) {
                                            refused += 1;
                                        }
                                        held_since = None;
                                    } else {
                                        held_since.get_or_insert(now);
                                    }
                                    std::thread::sleep(std::time::Duration::from_millis(10));
                                }
                                if accept {
                                    other.accept_suggestion(sid).unwrap().unwrap();
                                } else {
                                    assert!(other.reject_suggestion(sid).unwrap());
                                }
                                other.commit_batch().unwrap();
                            });
                            locked_rx.recv().unwrap();
                        }
                        Json(r)
                    }
                }),
            )
            .route("/ws/2/recording", get(|| async { Json(serde_json::json!({"recordings": []})) }))
    };
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    s.set_contact("tests@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    let x = album_folder(&s, dir.path(), "Band/X", 1)[0];
    let y = album_folder(&s, dir.path(), "Band/Y", 1)[0];
    let z = album_folder(&s, dir.path(), "Band/Z", 1)[0];
    // X: the owner will confirm another recording; Y: the owner will reject the one the run finds
    {
        let lib = s.library.lock().unwrap();
        lib.add_suggestions(x, &[option(&rec(7), None, "Song 1", 0.8, "search")])
            .unwrap();
        lib.add_suggestions(y, &[option(&rec(2), None, "Song 1", 0.8, "search")])
            .unwrap();
        let mut a = actions.lock().unwrap();
        a.insert(RX.into(), (lib.suggestions_for(x).unwrap()[0].id, true));
        a.insert(RY.into(), (lib.suggestions_for(y).unwrap()[0].id, false));
    }
    identify::run_with(&s, identify::RunOptions::quick(false))
        .await
        .unwrap();
    assert_eq!(lookups.load(SeqCst), 3);
    let lib = || s.library.lock().unwrap();
    let canonical = |id: i64| lib().get_track(pm_model::TrackId(id)).unwrap().unwrap().canonical;
    let release_of = |id: i64| -> Option<String> {
        lib()
            .identified_not_tagged()
            .unwrap()
            .into_iter()
            .find(|(t, _)| t.id == Some(pm_model::TrackId(id)))
            .and_then(|(_, r)| r.map(|r| r.to_string()))
    };
    let (uri, conf) = canonical(x).unwrap();
    assert_eq!(
        (uri.to_url(), conf),
        (
            format!("https://musicbrainz.org/recording/{}", rec(7)),
            pm_model::Confidence::ConfirmedByUser
        )
    );
    assert_eq!(
        release_of(x),
        None,
        "no release of the run's recording on the owner's"
    );
    assert!(canonical(y).is_none(), "the rejected recording is not stored");
    assert_eq!(
        canonical(z).unwrap().0.to_url(),
        format!("https://musicbrainz.org/recording/{}", rec(3))
    );
    assert_eq!(release_of(z).as_deref(), Some(RZ));
    let p = s.identify.lock().unwrap().clone();
    assert_eq!(p.identified, 1, "only Z was identified by the run: {p:?}");
    assert_eq!(p.identified_by.get("album_fit"), Some(&1));
    let (_, b) = call(&router(s.clone()), "GET", "/v1/library/identify", None).await;
    assert_eq!(b["counts"]["by"]["you"], 1, "{b}");
}

/// Fix round 1, item 2: a folder of a disc's ten songs fits a ten-track release and, as well, a
/// two-disc edition whose first disc has another recording at place 5 (fit counts the discs the
/// folder holds). Both are looked at; song 5 is a choice, the rest identified, the release open.
#[tokio::test]
async fn an_equally_fitting_multi_disc_edition_is_looked_at() {
    const R10: &str = "d1000000-0000-4000-8000-000000000010";
    const R20: &str = "d2000000-0000-4000-8000-000000000020";
    let album: Option<(&str, &[&str])> = Some(("Album", &[]));
    let asked = Arc::new(Asked::default());
    let port = mock_mb(
        vec![(
            "Disc".into(),
            vec![
                typed(R10, "Disc", album, 1, 10, &[]),
                typed(R20, "Disc", album, 2, 10, &[5]),
            ],
        )],
        asked.clone(),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("tests@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    let songs = album_folder(&s, dir.path(), "Band/Disc", 10);
    identify::run_with(&s, identify::RunOptions::quick(false))
        .await
        .unwrap();
    assert_eq!(asked.n().1, 2, "both editions looked up");
    let lib = || s.library.lock().unwrap();
    let canonical = |id: i64| lib().get_track(pm_model::TrackId(id)).unwrap().unwrap().canonical;
    assert!(canonical(songs[4]).is_none(), "the editions disagree on song 5");
    let mut opts: Vec<String> = lib()
        .suggestions_for(songs[4])
        .unwrap()
        .into_iter()
        .map(|o| o.recording_mbid)
        .collect();
    opts.sort();
    assert_eq!(opts, vec![rec(5), rec(905)]);
    for (k, id) in songs.iter().enumerate().filter(|(k, _)| *k != 4) {
        assert_eq!(
            canonical(*id).unwrap().0.to_url(),
            format!("https://musicbrainz.org/recording/{}", rec(k as u32 + 1))
        );
    }
    assert!(
        lib()
            .identified_not_tagged()
            .unwrap()
            .iter()
            .all(|(_, r)| r.is_none()),
        "which edition stays open"
    );
}

/// Fix round 1, item 4, at the job: another named remix or live performance at the same place
/// and length is no fit; the same version under other brackets is.
#[tokio::test]
async fn another_named_remix_or_performance_is_not_identified() {
    const RM: &str = "e1000000-0000-4000-8000-000000000001";
    let asked = Arc::new(Asked::default());
    let port = mock_mb(
        vec![(
            "Mixes".into(),
            vec![edition(
                RM,
                "Mixes",
                5,
                &[],
                &[
                    (2, "Song 2 (Bob Remix)"),
                    (3, "Song 3 (Live at Knebworth)"),
                    (4, "Song 4 (Alice Remix)"),
                ],
            )],
        )],
        asked,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("tests@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    let songs = album_folder(&s, dir.path(), "Band/Mixes", 5);
    for (k, title) in [
        (1, "Song 2 (Alice Remix)"),
        (2, "Song 3 (Live at Wembley)"),
        (3, "Song 4 [Alice Remix]"),
    ] {
        let lib = s.library.lock().unwrap();
        let mut t = lib.get_track(pm_model::TrackId(songs[k])).unwrap().unwrap();
        t.title = title.into();
        lib.upsert_track(&t).unwrap();
    }
    identify::run_with(&s, identify::RunOptions::quick(false))
        .await
        .unwrap();
    let canonical = |id: i64| {
        s.library
            .lock()
            .unwrap()
            .get_track(pm_model::TrackId(id))
            .unwrap()
            .unwrap()
            .canonical
    };
    assert!(canonical(songs[1]).is_none(), "Alice's remix is not Bob's");
    assert!(canonical(songs[2]).is_none(), "Wembley is not Knebworth");
    assert_eq!(
        canonical(songs[3]).unwrap().0.to_url(),
        format!("https://musicbrainz.org/recording/{}", rec(4))
    );
    assert!(canonical(songs[0]).is_some() && canonical(songs[4]).is_some());
}

/// Fix round 1, items 5 and 6 through the router: a review page loaded before the file was
/// replaced, or its tags corrected, can't confirm it (409, nothing stored); a stale single
/// accept or "Not this" that names another song or recording is refused (409), and an id
/// deleted before is never another suggestion's.
#[tokio::test]
async fn stale_review_choices_are_refused_at_the_router() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let a = song(&s, dir.path(), "Band/Album", 1, "Song 1");
    let b = song(&s, dir.path(), "Band/Album", 2, "Song 2");
    suggest(&s, a, &[option(&rec(1), Some(REL), "Song 1", 0.95, "album")]);
    suggest(&s, b, &[option(&rec(2), Some(REL), "Song 2", 0.95, "album")]);
    let (_, page) = call(&app, "GET", "/v1/library/suggestions/albums", None).await;
    let pick = |k: usize| {
        let x = &page["albums"][0]["songs"][k];
        serde_json::json!({"track_id": x["track"]["id"], "suggestion_id": x["options"][0]["id"], "recording_mbid": x["options"][0]["recording_mbid"]})
    };
    let (sa, sb) = (pick(0), pick(1));
    // The file of song 1 is replaced at the same path and read again
    {
        let lib = s.library.lock().unwrap();
        let mut t = lib.get_track(pm_model::TrackId(a)).unwrap().unwrap();
        t.file.size_bytes += 4096;
        t.file.mtime_unix += 30;
        lib.upsert_track(&t).unwrap();
    }
    let (st, body) = call(
        &app,
        "POST",
        "/v1/library/suggestions/confirm",
        Some(serde_json::json!({"choices": [sa.clone(), sb.clone()]})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["stale"][0]["track_id"], a);
    assert_eq!(body["stale"][0]["reason"], "gone");
    let canonical = |id: i64| {
        s.library
            .lock()
            .unwrap()
            .get_track(pm_model::TrackId(id))
            .unwrap()
            .unwrap()
            .canonical
    };
    assert!(canonical(a).is_none() && canonical(b).is_none(), "nothing stored");
    // Song 2's tags are corrected: its single accept from the old page is refused too
    {
        let lib = s.library.lock().unwrap();
        let mut t = lib.get_track(pm_model::TrackId(b)).unwrap().unwrap();
        t.title = "Song Two".into();
        lib.upsert_track(&t).unwrap();
    }
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/library/suggestions/{}/accept", sb["suggestion_id"]),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND, "its suggestion is gone");
    assert!(canonical(b).is_none());
    // New suggestions: new ids; a stale tab's DELETE of the old id rejects nothing
    suggest(&s, b, &[option(&rec(12), None, "Song Two", 0.9, "fingerprint")]);
    let new = s.library.lock().unwrap().suggestions_for(b).unwrap()[0].id;
    assert_ne!(Some(new), sb["suggestion_id"].as_i64());
    let (st, _) = call(
        &app,
        "DELETE",
        &format!("/v1/library/suggestions/{}", sb["suggestion_id"]),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert!(!s.library.lock().unwrap().is_rejected(b, &rec(12)).unwrap());
    // With the page's expectation: another song or recording is a 409, nothing changes
    let (st, _) = call(
        &app,
        "DELETE",
        &format!("/v1/library/suggestions/{new}?track_id={a}"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    let (st, _) = call(
        &app,
        "POST",
        &format!(
            "/v1/library/suggestions/{new}/accept?track_id={b}&recording_mbid={}",
            rec(2)
        ),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert!(canonical(b).is_none());
    assert_eq!(s.library.lock().unwrap().suggestions_for(b).unwrap().len(), 1);
    // The right expectation: done
    let (st, _) = call(
        &app,
        "POST",
        &format!(
            "/v1/library/suggestions/{new}/accept?track_id={b}&recording_mbid={}",
            rec(12)
        ),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(canonical(b).unwrap().1, pm_model::Confidence::ConfirmedByUser);
}

/// Fix round 1, item 7: a song already marked "suggested" with nothing left open (every option
/// rejected) whose "Which song is this?" finds only rejected recordings is "unknown", so a retry
/// still looks at it.
#[tokio::test]
async fn which_song_with_only_rejected_answers_leaves_a_suggested_song_retryable() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let one = rec(5);
    let mock = Router::new().route(
        "/ws/2/recording",
        get(move || {
            let one = one.clone();
            async move {
                Json(serde_json::json!({"recordings": [{"id": one, "score": 100, "title": "Song", "length": 200_000,
                    "artist-credit": [{"name": "Band"}], "releases": [{"id": REL, "title": "Album", "status": "Official"}]}]}))
            }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("tests@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    let mut t = untagged_track(dir.path().join("music/x/01 Song.flac"), "Song", "Band");
    t.duration_ms = Some(200_000);
    let id = s.library.lock().unwrap().upsert_track(&t).unwrap().0;
    {
        // An older run left it "suggested"; its one option was rejected outside "Not this"
        let lib = s.library.lock().unwrap();
        lib.add_suggestions(id, &[option(&rec(5), None, "Song", 0.8, "search")])
            .unwrap();
        lib.set_identify_state(id, "suggested", None).unwrap();
        let sid = lib.suggestions_for(id).unwrap()[0].id;
        lib.reject_suggestion(sid).unwrap();
        lib.set_identify_state(id, "suggested", None).unwrap();
    }
    let (st, b) = call(
        &router(s.clone()),
        "GET",
        &format!("/v1/library/tracks/{id}/candidates"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert!(b["suggestions"].as_array().unwrap().is_empty(), "{b}");
    let todo: Vec<i64> = s
        .library
        .lock()
        .unwrap()
        .tracks_to_identify(true)
        .unwrap()
        .into_iter()
        .filter_map(|t| t.id.map(|i| i.0))
        .collect();
    assert!(todo.contains(&id), "a retry looks at it again");
}

/// Fix round 1, item 10: what a "not found" song was checked with includes the services' own
/// addresses and the AcoustID key itself (only hashed into the song's input; the key is never
/// stored by identification); changing any of them re-checks it in a plain run. Kept MusicBrainz
/// answers stay within their bound when a run ends.
#[tokio::test]
async fn a_change_of_service_or_key_rechecks_not_found_songs() {
    let a1 = Arc::new(Asked::default());
    let a2 = Arc::new(Asked::default());
    let p1 = mock_mb(vec![], a1.clone()).await;
    let p2 = mock_mb(vec![], a2.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("tests@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{p1}/ws/2");
    let set_key = |key: &str, port: u16| {
        *s.acoustid.write().unwrap() = identify::AcoustIdSettings {
            api_key: key.into(),
            base_url: format!("http://127.0.0.1:{port}"),
            configured: String::new(),
        };
    };
    set_key("KeyOne1", p1);
    let path = dir.path().join("music/Loose/01 take.wav");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    tone_wav(&path, 3);
    let mut t = untagged_track(path, "Take", "Someone");
    t.track_no = None;
    let id = s.library.lock().unwrap().upsert_track(&t).unwrap().0;
    // A bound of two kept answers, held when the run ends
    let mut opts = identify::RunOptions::quick(false);
    opts.mb_cache_max = Some(2);
    // Three more songs so the run asks MusicBrainz more than twice
    for i in 2..=4 {
        let mut u = untagged_track(
            dir.path().join(format!("music/Gone/{i:02} x.wav")),
            &format!("Gone {i}"),
            "Someone",
        );
        u.track_no = None;
        s.library.lock().unwrap().upsert_track(&u).unwrap();
    }
    identify::run_with(&s, opts.clone()).await.unwrap();
    let kept: i64 = rusqlite::Connection::open(dir.path().join("lib.sqlite"))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM identify_mb_cache", [], |r| r.get(0))
        .unwrap();
    assert!(kept <= 2, "{kept} answers kept");
    let again = |s: &Arc<NodeState>| {
        let s = s.clone();
        let o = opts.clone();
        async move {
            identify::run_with(&s, o).await.unwrap();
            let p = s.identify.lock().unwrap().clone();
            p.total
        }
    };
    assert_eq!(again(&s).await, 0, "nothing changed");
    // Another MusicBrainz address (the contact as it was)
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{p2}/ws/2");
    assert_eq!(again(&s).await, 4, "MusicBrainz moved");
    assert!(a2.n().2 > 0, "asked at the new address");
    assert_eq!(again(&s).await, 0);
    // Another AcoustID key, then another AcoustID address
    set_key("KeyTwo2", p1);
    assert_eq!(again(&s).await, 4, "another key");
    assert_eq!(again(&s).await, 0);
    set_key("KeyTwo2", p2);
    assert_eq!(again(&s).await, 4, "AcoustID moved");
    // The key is nowhere in identification's tables
    let conn = rusqlite::Connection::open(dir.path().join("lib.sqlite")).unwrap();
    for table in [
        "identify_state",
        "identify_mb_cache",
        "identify_file_keys",
        "identify_suggestions",
        "fingerprints",
    ] {
        let dump: String = conn
            .query_row(
                &format!("SELECT COALESCE(group_concat(quote(x.*)), '') FROM (SELECT * FROM {table}) x"),
                [],
                |r| r.get(0),
            )
            .unwrap_or_default();
        assert!(!dump.contains("KeyTwo2") && !dump.contains("KeyOne1"), "{table}");
    }
    assert!(s
        .library
        .lock()
        .unwrap()
        .get_track(pm_model::TrackId(id))
        .unwrap()
        .unwrap()
        .canonical
        .is_none());
}

/// Fix round 1, item 10 (album context): a "not found" song of an album folder whose tags are
/// fixed is checked again by a plain run, as part of its folder (the album lane sees every file).
#[tokio::test]
async fn a_fixed_song_is_rechecked_with_its_album() {
    const RA: &str = "f0000000-0000-4000-8000-0000000000a1";
    let asked = Arc::new(Asked::default());
    let port = mock_mb(
        vec![("Fix".into(), vec![edition(RA, "Fix", 4, &[], &[])])],
        asked.clone(),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("tests@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    let songs = album_folder(&s, dir.path(), "Band/Fix", 4);
    {
        let lib = s.library.lock().unwrap();
        let mut t = lib.get_track(pm_model::TrackId(songs[3])).unwrap().unwrap();
        t.title = "Wrong Name".into();
        lib.upsert_track(&t).unwrap();
    }
    identify::run_with(&s, identify::RunOptions::quick(false))
        .await
        .unwrap();
    let canonical = |id: i64| {
        s.library
            .lock()
            .unwrap()
            .get_track(pm_model::TrackId(id))
            .unwrap()
            .unwrap()
            .canonical
    };
    assert!(canonical(songs[3]).is_none());
    {
        let lib = s.library.lock().unwrap();
        let mut t = lib.get_track(pm_model::TrackId(songs[3])).unwrap().unwrap();
        t.title = "Song 4".into();
        lib.upsert_track(&t).unwrap();
    }
    identify::run_with(&s, identify::RunOptions::quick(false))
        .await
        .unwrap();
    let p = s.identify.lock().unwrap().clone();
    assert_eq!((p.total, p.identified), (1, 1), "{p:?}");
    assert_eq!(
        canonical(songs[3]).unwrap().0.to_url(),
        format!("https://musicbrainz.org/recording/{}", rec(4))
    );
}

/// Claude r1 low 7: when "Write IDs" is on, a confirmation says how many files took the IDs and
/// which didn't (here: the files aren't there), instead of hiding it; the identities are stored
/// either way.
#[tokio::test]
async fn a_confirmation_reports_files_the_ids_could_not_be_written_to() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let a = song(&s, dir.path(), "Band/Album", 1, "Song 1");
    suggest(&s, a, &[option(&rec(1), Some(REL), "Song 1", 0.95, "album")]);
    let sid = s.library.lock().unwrap().suggestions_for(a).unwrap()[0].id;
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/library/identify/settings",
        Some(serde_json::json!({"write_ids": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, b) = call(
        &app,
        "POST",
        "/v1/library/suggestions/confirm",
        Some(
            serde_json::json!({"choices": [{"track_id": a, "suggestion_id": sid, "recording_mbid": rec(1)}]}),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["count"], 1);
    assert_eq!(b["files"]["written"], 0, "{b}");
    assert_eq!(b["files"]["skipped"].as_array().unwrap().len(), 1, "{b}");
    assert_eq!(b["files"]["skipped"][0]["track_id"], a);
}

// ---- REV2 (owner, 2026-11-04): most pending songs settle on their own ----

/// Ids of the pending replay's mock: `kind` says what it is (recording, release…), `n` the song.
fn pending_id(kind: u32, n: u32, k: u32) -> String {
    format!("{kind:08x}-0000-4000-8000-{:012x}", n as u64 * 16 + k as u64)
}

/// The pending replay's MusicBrainz: a recording search answers by the cohort letter that starts
/// the title (see [`identify_replay_pending_7k`]); release searches find nothing.
async fn pending_mock(
    recordings: Arc<std::sync::atomic::AtomicUsize>,
    releases: Arc<std::sync::atomic::AtomicUsize>,
) -> u16 {
    use std::sync::atomic::Ordering::SeqCst;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let rid = pending_id;
    let mock = {
        let (recordings, releases) = (recordings.clone(), releases.clone());
        Router::new()
            .route(
                "/ws/2/release",
                get(move || {
                    releases.fetch_add(1, SeqCst);
                    async { Json(serde_json::json!({"releases": []})) }
                }),
            )
            .route(
                "/ws/2/recording",
                get(move |Query(q): Query<std::collections::HashMap<String, String>>| {
                    recordings.fetch_add(1, SeqCst);
                    let query = q.get("query").cloned().unwrap_or_default();
                    let title = query.strip_prefix("recording:\"").and_then(|r| r.split('"').next()).unwrap_or_default().to_string();
                    let artist = query.split("artist:\"").nth(1).and_then(|r| r.split('"').next()).unwrap_or_default().to_string();
                    let cohort = title.chars().next().unwrap_or('?');
                    let n: u32 = title.get(1..6).and_then(|x| x.parse().ok()).unwrap_or(0);
                    let len: u64 = 200_000 + (n as u64 % 50) * 1000;
                    let hit = |k: u32, t: &str, a: &str, length: Option<u64>, rel: (&str, &str, Vec<&str>)| {
                        serde_json::json!({"id": rid(0xb0, n, k), "score": 100, "title": t, "length": length,
                            "artist-credit": [{"name": a}],
                            "releases": [{"id": rid(0xc0, n, k), "title": rel.0, "status": rel.1, "release-group": {"primary-type": rel.2[0], "secondary-types": rel.2[1..].to_vec()}}]})
                    };
                    let album = format!("Album {}", n / 12);
                    let hits: Vec<serde_json::Value> = match cohort {
                        'B' => (0..3)
                            .map(|k| {
                                // About one in eight has a release named like the file's album
                                let rel = if k == 1 && n.is_multiple_of(8) { album.clone() } else { format!("Elsewhere {k}") };
                                hit(k, &title, &artist, Some(len + k as u64 * 400), (&rel, "Official", vec!["Album"]))
                            })
                            .collect(),
                        'C' => vec![hit(0, &title, &artist, Some(len), ("Somewhere", "Official", vec!["Album"]))],
                        'L' => vec![hit(0, &title, &artist, Some(len), ("Live at Somewhere", "Official", vec!["Album", "Live"]))],
                        'R' => vec![hit(0, &format!("{title} (Someone Remix)"), &artist, Some(len), ("Remixes", "Official", vec!["Album", "Remix"]))],
                        'O' => vec![hit(0, &title, "Somebody Else Entirely", Some(len), ("Covers", "Official", vec!["Album"]))],
                        'D' => vec![hit(0, &title, &artist, Some(len + 20_000), ("Somewhere", "Official", vec!["Album"]))],
                        'W' => vec![hit(0, &title, &artist, None, ("Somewhere", "Official", vec!["Album"]))],
                        _ => Vec::new(),
                    };
                    async move { Json(serde_json::json!({"recordings": hits})) }
                }),
            )
    };
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    port
}

/// The pending-review replay (on demand: `cargo test -p pm-api --lib identify_replay_pending --
/// --ignored --nocapture`; `PM_REPLAY_REPORT=<file>` writes the report). 7,412 songs shaped like
/// the owner's pending review (aggregates of 2026-11-04): fingerprints that name one audio under
/// several recordings, tag searches with several (or one) recordings of exactly the file's title,
/// artist and length, and the genuinely different cases (live, remix, another artist, a length
/// far off, a weak hit, a fingerprint naming different songs). Earlier fingerprints are kept in
/// the library (as on the node); MusicBrainz is a counting mock. Reports how many songs end
/// identified, waiting for review, or not found.
#[tokio::test]
#[ignore = "benchmark: run on demand"]
async fn identify_replay_pending_7k() {
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    // (cohort letter, songs)
    const COHORTS: [(char, u32); 9] = [
        ('A', 1852), // one audio, three recordings of the same song
        ('G', 150),  // the fingerprint names three different songs
        ('B', 3000), // tag search: three exact recordings
        ('C', 1500), // tag search: one exact recording
        ('L', 250),  // tag search: only a live recording
        ('R', 200),  // tag search: only someone's remix
        ('O', 200),  // tag search: another artist's song
        ('D', 160),  // tag search: a length 20 s off
        ('W', 100),  // tag search: no length to compare
    ];
    let recordings = Arc::new(AtomicUsize::new(0));
    let releases = Arc::new(AtomicUsize::new(0));
    let port = pending_mock(recordings.clone(), releases.clone()).await;
    let rid = pending_id;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("tests@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    *s.acoustid.write().unwrap() = identify::AcoustIdSettings {
        api_key: "test-key".into(),
        base_url: format!("http://127.0.0.1:{port}"),
        configured: String::new(),
    };
    let music = dir.path().join("music");
    let built = std::time::Instant::now();
    let mut total = 0u32;
    {
        let lib = s.library.lock().unwrap();
        lib.begin_batch().unwrap();
        for (cohort, count) in COHORTS {
            for n in 0..count {
                if total.is_multiple_of(500) {
                    lib.commit_batch().unwrap();
                    lib.begin_batch().unwrap();
                }
                total += 1;
                let folder = music.join(format!("{cohort}/{:04}", n / 12));
                std::fs::create_dir_all(&folder).unwrap();
                let path = folder.join(format!("{:02} {cohort}{n:05}.flac", n % 12 + 1));
                // A few bytes of its own: the content key is the file's, nothing is decoded
                std::fs::write(&path, format!("{cohort}{n:05} {}", "x".repeat(40))).unwrap();
                let title = format!("{cohort}{n:05} Song");
                let artist = format!("Artist {}", n / 12);
                let len = 200_000 + (n as u64 % 50) * 1000;
                let mut t = untagged_track(path.clone(), &title, &artist);
                t.track_no = Some(n % 12 + 1);
                t.disc_no = None;
                t.duration_ms = Some(len);
                t.file.duration_ms = Some(len);
                // Each song of its own album, so the folder is no album and the fingerprint
                // and the search decide (as for the node's pending songs)
                t.album = Some(pm_model::Album {
                    id: None,
                    title: format!("Album {}", n / 12),
                    album_artist: None,
                    release_mbid: None,
                    release_group_mbid: None,
                    year: None,
                    upc: None,
                    total_discs: None,
                });
                if n % 2 == 1 {
                    t.artists[0].name = format!("{artist} ");
                }
                lib.upsert_track(&t).unwrap();
                // What an earlier run's fingerprint found
                let key = Library::file_key(&path).unwrap();
                lib.save_fingerprint(&key, len / 1000, "AQfake").unwrap();
                let rec = |k: u32, t: &str, kinds: (&str, Vec<&str>), relt: &str, total: u32, pos: u32| {
                    serde_json::json!({"id": rid(0xa0, n, k), "title": t, "duration": (len / 1000) as f64,
                        "artists": [{"name": artist}],
                        "releasegroups": [{"id": rid(0xd0, n, k), "title": relt, "type": kinds.0, "secondarytypes": kinds.1,
                            "releases": [{"id": rid(0xe0, n, k), "title": relt, "track_count": total, "mediums": [{"position": 1, "track_count": total, "tracks": [{"position": pos}]}]}]}]})
                };
                let results = match cohort {
                    'A' => serde_json::json!([{"id": format!("acoustid-{n}"), "score": 0.96, "recordings": [
                        rec(0, &title, ("Album", vec!["Compilation"]), "Greatest Hits", 18, 3),
                        rec(1, &title, ("Album", vec![]), &format!("Album {}", n / 12), 10, 2),
                        rec(2, &title, ("Single", vec![]), &title, 2, 1),
                    ]}]),
                    'G' => serde_json::json!([{"id": format!("acoustid-{n}"), "score": 0.95, "recordings": [
                        rec(0, &title, ("Album", vec![]), "One", 10, 2),
                        rec(1, "Quite Another Song", ("Album", vec![]), "Two", 10, 2),
                        rec(2, "A Third Song", ("Album", vec![]), "Three", 10, 2),
                    ]}]),
                    _ => serde_json::json!([]),
                };
                lib.save_lookup(
                    &key,
                    &serde_json::json!({"status": "ok", "results": results}).to_string(),
                )
                .unwrap();
            }
        }
        lib.commit_batch().unwrap();
    }
    assert_eq!(total, 7412);
    let built = built.elapsed();
    let t0 = std::time::Instant::now();
    identify::run_with(&s, identify::RunOptions::quick(false))
        .await
        .unwrap();
    let took = t0.elapsed();
    let p = s.identify.lock().unwrap().clone();
    let c = s.library.lock().unwrap().identity_counts().unwrap();
    // Per cohort: how many end identified / waiting / not found
    let mut by: std::collections::BTreeMap<char, (u32, u32, u32)> = Default::default();
    {
        let lib = s.library.lock().unwrap();
        let waiting: std::collections::HashSet<i64> = lib
            .suggestions()
            .unwrap()
            .into_iter()
            .map(|r| r.track_id)
            .collect();
        for t in lib.all_tracks().unwrap() {
            let cohort = t.title.chars().next().unwrap();
            let e = by.entry(cohort).or_default();
            if t.canonical.is_some() {
                e.0 += 1;
            } else if t.id.is_some_and(|i| waiting.contains(&i.0)) {
                e.1 += 1;
            } else {
                e.2 += 1;
            }
        }
    }
    let cohorts: serde_json::Map<String, serde_json::Value> = by
        .iter()
        .map(|(k, (i, w, u))| {
            (
                k.to_string(),
                serde_json::json!({"identified": i, "pending_review": w, "not_found": u}),
            )
        })
        .collect();
    let out = serde_json::json!({
        "songs": total, "identified": c.identified, "pending_review": c.suggested, "not_found": c.unknown,
        "identified_by": p.identified_by, "cohorts": cohorts,
        "musicbrainz_recording_searches": recordings.load(SeqCst), "musicbrainz_release_searches": releases.load(SeqCst),
        "acoustid_requests": p.work.acoustid_asked, "decoded": p.work.decoded,
        "measured_unpaced_secs": (took.as_secs_f64() * 100.0).round() / 100.0, "built_secs": built.as_secs_f64(),
        "scope": "synthetic replay against in-process mocks shaped like the node's pending aggregates; not a measurement of the node",
    });
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
    if let Ok(path) = std::env::var("PM_REPLAY_REPORT") {
        std::fs::write(path, serde_json::to_string_pretty(&out).unwrap()).unwrap();
    }
}

/// The re-check replay (owner, 2026-11-06; on demand: `cargo test -p pm-api --lib
/// identify_replay_recheck -- --ignored --nocapture`; `PM_REPLAY_REPORT=<file>` writes the report).
/// The node's starting point: 7,412 songs already waiting for review with the rows older rules
/// left (its candidate histogram, 22,443 rows, 7,122 by fingerprint), no input key, no rules
/// version; fingerprints kept. Then "Identify again" (a retry). The answers per cohort are those
/// of [`identify_replay_pending_7k`] (assumed, not the node's): reports how many songs settle,
/// keep waiting or end not found, the requests made and what is left of the old rows.
#[tokio::test]
#[ignore = "benchmark: run on demand"]
async fn identify_replay_recheck_7k() {
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    // (cohort letter, songs)
    const COHORTS: [(char, u32); 9] = [
        ('A', 1852), // one audio, three recordings of the same song
        ('G', 150),  // the fingerprint names three different songs
        ('B', 3000), // tag search: three exact recordings
        ('C', 1500), // tag search: one exact recording
        ('L', 250),  // tag search: only a live recording
        ('R', 200),  // tag search: only someone's remix
        ('O', 200),  // tag search: another artist's song
        ('D', 160),  // tag search: a length 20 s off
        ('W', 100),  // tag search: no length to compare
    ];
    let recordings = Arc::new(AtomicUsize::new(0));
    let releases = Arc::new(AtomicUsize::new(0));
    let port = pending_mock(recordings.clone(), releases.clone()).await;
    let rid = pending_id;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("tests@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    *s.acoustid.write().unwrap() = identify::AcoustIdSettings {
        api_key: "test-key".into(),
        base_url: format!("http://127.0.0.1:{port}"),
        configured: String::new(),
    };
    let music = dir.path().join("music");
    let built = std::time::Instant::now();
    let mut total = 0u32;
    let mut seeded = 0u32;
    {
        let lib = s.library.lock().unwrap();
        lib.begin_batch().unwrap();
        for (cohort, count) in COHORTS {
            for n in 0..count {
                if total.is_multiple_of(500) {
                    lib.commit_batch().unwrap();
                    lib.begin_batch().unwrap();
                }
                total += 1;
                let folder = music.join(format!("{cohort}/{:04}", n / 12));
                std::fs::create_dir_all(&folder).unwrap();
                let path = folder.join(format!("{:02} {cohort}{n:05}.flac", n % 12 + 1));
                // A few bytes of its own: the content key is the file's, nothing is decoded
                std::fs::write(&path, format!("{cohort}{n:05} {}", "x".repeat(40))).unwrap();
                let title = format!("{cohort}{n:05} Song");
                let artist = format!("Artist {}", n / 12);
                let len = 200_000 + (n as u64 % 50) * 1000;
                let mut t = untagged_track(path.clone(), &title, &artist);
                t.track_no = Some(n % 12 + 1);
                t.disc_no = None;
                t.duration_ms = Some(len);
                t.file.duration_ms = Some(len);
                // Each song of its own album, so the folder is no album and the fingerprint
                // and the search decide (as for the node's pending songs)
                t.album = Some(pm_model::Album {
                    id: None,
                    title: format!("Album {}", n / 12),
                    album_artist: None,
                    release_mbid: None,
                    release_group_mbid: None,
                    year: None,
                    upc: None,
                    total_discs: None,
                });
                if n % 2 == 1 {
                    t.artists[0].name = format!("{artist} ");
                }
                let lib_id = lib.upsert_track(&t).unwrap().0;
                // What an earlier run's fingerprint found
                let key = Library::file_key(&path).unwrap();
                lib.save_fingerprint(&key, len / 1000, "AQfake").unwrap();
                let rec = |k: u32, t: &str, kinds: (&str, Vec<&str>), relt: &str, total: u32, pos: u32| {
                    serde_json::json!({"id": rid(0xa0, n, k), "title": t, "duration": (len / 1000) as f64,
                        "artists": [{"name": artist}],
                        "releasegroups": [{"id": rid(0xd0, n, k), "title": relt, "type": kinds.0, "secondarytypes": kinds.1,
                            "releases": [{"id": rid(0xe0, n, k), "title": relt, "track_count": total, "mediums": [{"position": 1, "track_count": total, "tracks": [{"position": pos}]}]}]}]})
                };
                let results = match cohort {
                    'A' => serde_json::json!([{"id": format!("acoustid-{n}"), "score": 0.96, "recordings": [
                        rec(0, &title, ("Album", vec!["Compilation"]), "Greatest Hits", 18, 3),
                        rec(1, &title, ("Album", vec![]), &format!("Album {}", n / 12), 10, 2),
                        rec(2, &title, ("Single", vec![]), &title, 2, 1),
                    ]}]),
                    'G' => serde_json::json!([{"id": format!("acoustid-{n}"), "score": 0.95, "recordings": [
                        rec(0, &title, ("Album", vec![]), "One", 10, 2),
                        rec(1, "Quite Another Song", ("Album", vec![]), "Two", 10, 2),
                        rec(2, "A Third Song", ("Album", vec![]), "Three", 10, 2),
                    ]}]),
                    _ => serde_json::json!([]),
                };
                lib.save_lookup(
                    &key,
                    &serde_json::json!({"status": "ok", "results": results}).to_string(),
                )
                .unwrap();
                // What older rules left: the node's candidate histogram (1→2054, 2→1404, 3→825,
                // 4→539, 5→2590 songs: 22,443 rows), the first 7,122 rows by fingerprint
                let song = total - 1;
                // Spread over the cohorts: the bucket by a stride through all songs
                let rows = match (song * 7919) % 7412 {
                    s if s < 2054 => 1,
                    s if s < 2054 + 1404 => 2,
                    s if s < 2054 + 1404 + 825 => 3,
                    s if s < 2054 + 1404 + 825 + 539 => 4,
                    _ => 5,
                };
                let old: Vec<pm_library::identify::Candidate> = (0..rows)
                    .map(|k| {
                        let source = if seeded < 7122 { "fingerprint" } else { "search" };
                        seeded += 1;
                        pm_library::identify::Candidate {
                            recording: pm_model::Mbid::parse(&rid(0xf0, song, k)).unwrap(),
                            release: None,
                            title: Some(title.clone()),
                            artist: Some(artist.clone()),
                            release_title: Some(format!("Album {}", n / 12)),
                            score: 0.8,
                            source: source.into(),
                        }
                    })
                    .collect();
                lib.add_suggestions(lib_id, &old).unwrap();
                lib.set_identify_state(lib_id, "suggested", None).unwrap();
            }
        }
        lib.commit_batch().unwrap();
    }
    assert_eq!(total, 7412);
    {
        let lib = s.library.lock().unwrap();
        let c = lib.identity_counts().unwrap();
        assert_eq!((c.suggested, c.outdated, c.unchecked), (7412, 7412, 0));
        assert_eq!(lib.suggestions().unwrap().len(), 22443);
        assert_eq!(seeded, 22443);
    }
    let built = built.elapsed();
    let t0 = std::time::Instant::now();
    identify::run_with(&s, identify::RunOptions::quick(true))
        .await
        .unwrap();
    let took = t0.elapsed();
    let p = s.identify.lock().unwrap().clone();
    let c = s.library.lock().unwrap().identity_counts().unwrap();
    // Per cohort: how many end identified / waiting / not found
    let mut by: std::collections::BTreeMap<char, (u32, u32, u32)> = Default::default();
    {
        let lib = s.library.lock().unwrap();
        let waiting: std::collections::HashSet<i64> = lib
            .suggestions()
            .unwrap()
            .into_iter()
            .map(|r| r.track_id)
            .collect();
        for t in lib.all_tracks().unwrap() {
            let cohort = t.title.chars().next().unwrap();
            let e = by.entry(cohort).or_default();
            if t.canonical.is_some() {
                e.0 += 1;
            } else if t.id.is_some_and(|i| waiting.contains(&i.0)) {
                e.1 += 1;
            } else {
                e.2 += 1;
            }
        }
    }
    let cohorts: serde_json::Map<String, serde_json::Value> = by
        .iter()
        .map(|(k, (i, w, u))| {
            (
                k.to_string(),
                serde_json::json!({"identified": i, "pending_review": w, "not_found": u}),
            )
        })
        .collect();
    let old_rows_left = s
        .library
        .lock()
        .unwrap()
        .suggestions()
        .unwrap()
        .iter()
        .filter(|r| r.recording_mbid.starts_with("000000f0"))
        .count();
    let rows_now = s.library.lock().unwrap().suggestions().unwrap().len();
    let out = serde_json::json!({
        "start": {"songs_waiting": 7412, "rows": 22443, "outdated": 7412},
        "old_rows_left": old_rows_left, "rows_now": rows_now, "outdated_after": c.outdated,
        "paced_estimate_secs": recordings.load(SeqCst) + releases.load(SeqCst),
        "songs": total, "identified": c.identified, "pending_review": c.suggested, "not_found": c.unknown,
        "identified_by": p.identified_by, "cohorts": cohorts,
        "musicbrainz_recording_searches": recordings.load(SeqCst), "musicbrainz_release_searches": releases.load(SeqCst),
        "acoustid_requests": p.work.acoustid_asked, "decoded": p.work.decoded,
        "measured_unpaced_secs": (took.as_secs_f64() * 100.0).round() / 100.0, "built_secs": built.as_secs_f64(),
        "scope": "re-check of 7,412 songs already waiting with old-rule rows (the node's histogram); what MusicBrainz and AcoustID answer per cohort is the #250 replay's assumption, not the node's data",
    });
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
    if let Ok(path) = std::env::var("PM_REPLAY_REPORT") {
        std::fs::write(path, serde_json::to_string_pretty(&out).unwrap()).unwrap();
    }
}

/// REV2 at the job: an exact tag match on several recordings is identified (the one on the
/// file's album, with that release), so is one audio under several recordings; live and remix
/// hits stay for the owner; a song left waiting by an earlier version is checked again and
/// settles, its old options gone; a recording the owner rejected is never picked.
#[tokio::test]
async fn pending_songs_settle_on_their_own_and_real_differences_stay() {
    let recordings = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let port = pending_mock(recordings.clone(), Default::default()).await;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("tests@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    *s.acoustid.write().unwrap() = identify::AcoustIdSettings {
        api_key: "test-key".into(),
        base_url: format!("http://127.0.0.1:{port}"),
        configured: String::new(),
    };
    // n = 0: its album is "Album 0", the release of hit 1 is named so (n.is_multiple_of(8))
    let add = |cohort: char, n: u32, results: serde_json::Value| -> i64 {
        let folder = dir.path().join(format!("music/{cohort}{n}"));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("01 song.flac");
        std::fs::write(&path, format!("{cohort}{n} audio")).unwrap();
        let len = 200_000 + (n as u64 % 50) * 1000;
        let mut t = untagged_track(
            path.clone(),
            &format!("{cohort}{n:05} Song"),
            &format!("Artist {}", n / 12),
        );
        t.track_no = Some(1);
        t.duration_ms = Some(len);
        t.file.duration_ms = Some(len);
        t.album = Some(pm_model::Album {
            id: None,
            title: format!("Album {}", n / 12),
            album_artist: None,
            release_mbid: None,
            release_group_mbid: None,
            year: None,
            upc: None,
            total_discs: None,
        });
        let lib = s.library.lock().unwrap();
        let id = lib.upsert_track(&t).unwrap().0;
        let key = Library::file_key(&path).unwrap();
        lib.save_fingerprint(&key, len / 1000, "AQfake").unwrap();
        lib.save_lookup(
            &key,
            &serde_json::json!({"status": "ok", "results": results}).to_string(),
        )
        .unwrap();
        id
    };
    let b = add('B', 0, serde_json::json!([]));
    let c = add('C', 13, serde_json::json!([]));
    let l = add('L', 1, serde_json::json!([]));
    let r = add('R', 2, serde_json::json!([]));
    let fp_rec = |k: u32, n: u32, title: &str, artist: &str| {
        serde_json::json!({"id": pending_id(0xa0, n, k), "title": title, "duration": 203.0, "artists": [{"name": artist}],
        "releasegroups": [{"id": "g", "title": "Album 0", "type": "Album", "releases": [{"id": pending_id(0xe0, n, k), "title": "Album 0", "track_count": 9, "mediums": [{"position": 1, "track_count": 9, "tracks": [{"position": 4}]}]}]}]})
    };
    let a = add(
        'A',
        3,
        serde_json::json!([{"id": "x", "score": 0.96, "recordings": [fp_rec(0, 3, "A00003 Song", "Artist 0"), fp_rec(1, 3, "A00003 Song", "Artist 0")]}]),
    );
    // Waiting for review since an earlier version of identification (no input key), its options then
    let old = add('B', 24, serde_json::json!([]));
    {
        let lib = s.library.lock().unwrap();
        lib.add_suggestions(old, &[option(&rec(99), None, "B00024 Song", 0.9, "search")])
            .unwrap();
        lib.set_identify_state(old, "suggested", None).unwrap();
    }
    // The owner rejected the hit C's search finds (its only exact recording)
    {
        let lib = s.library.lock().unwrap();
        lib.add_suggestions(
            c,
            &[option(
                &pending_id(0xb0, 13, 0),
                None,
                "C00013 Song",
                0.9,
                "search",
            )],
        )
        .unwrap();
        let sid = lib.suggestions_for(c).unwrap()[0].id;
        lib.reject_suggestion(sid).unwrap();
    }
    // ... and the song is one this run checks (never checked as far as the run knows)
    rusqlite::Connection::open(dir.path().join("lib.sqlite"))
        .unwrap()
        .execute("DELETE FROM identify_state WHERE track_id = ?1", [c])
        .unwrap();
    identify::run_with(&s, identify::RunOptions::quick(false))
        .await
        .unwrap();
    let lib = || s.library.lock().unwrap();
    let canonical = |id: i64| lib().get_track(pm_model::TrackId(id)).unwrap().unwrap().canonical;
    let release_of = |id: i64| -> Option<String> {
        lib()
            .identified_not_tagged()
            .unwrap()
            .into_iter()
            .find(|(t, _)| t.id == Some(pm_model::TrackId(id)))
            .and_then(|(_, r)| r.map(|r| r.to_string()))
    };
    let (uri, conf) = canonical(b).expect("an exact tag match");
    assert_eq!(
        uri.to_url(),
        format!("https://musicbrainz.org/recording/{}", pending_id(0xb0, 0, 1)),
        "the one on the file's album"
    );
    assert_eq!(conf.label(), "tag_match");
    assert!(conf.publishable());
    assert_eq!(release_of(b), Some(pending_id(0xc0, 0, 1)));
    let (uri, conf) = canonical(a).expect("one audio, two recordings");
    assert_eq!(
        uri.to_url(),
        format!("https://musicbrainz.org/recording/{}", pending_id(0xa0, 3, 0))
    );
    assert!(
        matches!(conf, pm_model::Confidence::Fingerprint { .. }),
        "{conf:?}"
    );
    assert!(
        canonical(l).is_none() && !lib().suggestions_for(l).unwrap().is_empty(),
        "live: for the owner"
    );
    assert!(
        canonical(r).is_none() && !lib().suggestions_for(r).unwrap().is_empty(),
        "remix: for the owner"
    );
    assert!(canonical(c).is_none(), "the rejected recording is never picked");
    let (uri, _) = canonical(old).expect("checked again and settled");
    assert_eq!(
        uri.to_url(),
        format!("https://musicbrainz.org/recording/{}", pending_id(0xb0, 24, 1)),
        "on its album"
    );
    assert!(lib().suggestions_for(old).unwrap().is_empty());
    let p = s.identify.lock().unwrap().clone();
    assert_eq!(p.identified_by.get("tag_match"), Some(&2), "{p:?}");
    let (_, st) = call(&router(s.clone()), "GET", "/v1/library/identify", None).await;
    assert_eq!(st["counts"]["by"]["title_artist"], 2, "{st}");
}

/// REV2: the Confirm page's first answer is quick and small however much waits: 7,412 songs with
/// 22,443 suggestions in 590 folders (the node's pending review, 2026-11-04) answer the first page
/// (whole albums, about 200 songs) in well under a second and under 200 kB, with the totals for
/// the page's head.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_review_page_answers_quickly_with_a_big_backlog() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    {
        let lib = s.library.lock().unwrap();
        lib.begin_batch().unwrap();
        for n in 0..7412u32 {
            if n.is_multiple_of(500) {
                lib.commit_batch().unwrap();
                lib.begin_batch().unwrap();
            }
            let folder = n % 590;
            let mut t = untagged_track(
                dir.path().join(format!("music/F{folder:03}/{n:05}.flac")),
                &format!("Song {n}"),
                "Band",
            );
            t.track_no = Some(n / 590 + 1);
            let id = lib.upsert_track(&t).unwrap().0;
            // 207 songs with four options, the rest with three: 22,443
            let k = if n < 207 { 4 } else { 3 };
            let options: Vec<pm_library::identify::Candidate> = (0..k)
                .map(|j| option(&rec(n * 4 + j), None, &format!("Song {n}"), 0.8, "search"))
                .collect();
            lib.add_suggestions(id, &options).unwrap();
            lib.set_identify_state(id, "suggested", None).unwrap();
        }
        lib.commit_batch().unwrap();
    }
    let app = router(s.clone());
    let req = || {
        Request::builder()
            .uri("/v1/library/suggestions/albums?limit=50")
            .body(Body::empty())
            .unwrap()
    };
    // Warm (the first request also opens things)
    let _ = app.clone().oneshot(req()).await.unwrap();
    let t0 = std::time::Instant::now();
    let r = app.clone().oneshot(req()).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    let took = t0.elapsed();
    let page: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    println!("first page: {} bytes in {:?}", bytes.len(), took);
    assert_eq!(page["songs"], 7412);
    assert_eq!(page["total_albums"], 590);
    // Whole albums, up to about 200 songs a page (at least one album)
    let albums = page["albums"].as_array().unwrap();
    let on_page: usize = albums.iter().map(|a| a["songs"].as_array().unwrap().len()).sum();
    assert!(!albums.is_empty() && albums.len() <= 50);
    assert!(on_page <= identify::MAX_REVIEW_SONGS + 13, "{on_page} songs");
    assert!(page["next"].is_u64());
    // Each song as the page shows it, not the whole track record
    let t = &albums[0]["songs"][0]["track"];
    assert!(
        t["title"].is_string() && t["file"]["path"].is_string() && t["artists"][0]["name"].is_string(),
        "{t}"
    );
    assert!(
        t.get("format").is_none() && t["file"].get("format").is_none(),
        "{t}"
    );
    assert!(bytes.len() < 200_000, "{} bytes", bytes.len());
    assert!(took < std::time::Duration::from_millis(1000), "{took:?}");
}

// ---- PR #250 review, fix round 1 ----

/// PR #250 review (Kimi, high): a rescan that replaces a song's file while the run works on it
/// makes the run's answer stale: not stored, not counted, and the song is checked again by the
/// next run (which then finds it).
#[tokio::test]
async fn an_answer_for_a_song_rescanned_during_the_run_is_not_stored() {
    const RX: &str = "a9000000-0000-4000-8000-0000000000aa";
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let node: Arc<std::sync::OnceLock<(Arc<NodeState>, i64)>> = Default::default();
    let rescan = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let rel = serde_json::json!({"id": RX, "title": "X", "media": [{"position": 1, "tracks": [
        {"position": 1, "title": "Song 1", "length": 201_000, "recording": {"id": rec(1)}}]}]});
    let mock = {
        let (node, rescan) = (node.clone(), rescan.clone());
        Router::new()
            .route(
                "/ws/2/release",
                get(|| async { Json(serde_json::json!({"releases": [{"id": RX, "score": 100, "title": "X", "track-count": 1, "artist-credit": [{"name": "Band"}]}]})) }),
            )
            .route(
                "/ws/2/release/{id}",
                get(move || {
                    // While the run looks the album up, the file is replaced and read again
                    if rescan.swap(false, std::sync::atomic::Ordering::SeqCst) {
                        let (s, id) = node.get().unwrap();
                        let lib = s.library.lock().unwrap();
                        let mut t = lib.get_track(pm_model::TrackId(*id)).unwrap().unwrap();
                        t.file.size_bytes += 4096;
                        t.file.mtime_unix += 30;
                        lib.upsert_track(&t).unwrap();
                    }
                    let r = rel.clone();
                    async move { Json(r) }
                }),
            )
            .route("/ws/2/recording", get(|| async { Json(serde_json::json!({"recordings": []})) }))
    };
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    s.set_contact("tests@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    let x = album_folder(&s, dir.path(), "Band/X", 1)[0];
    node.set((s.clone(), x)).ok().unwrap();
    identify::run_with(&s, identify::RunOptions::quick(false))
        .await
        .unwrap();
    let canonical = |id: i64| {
        s.library
            .lock()
            .unwrap()
            .get_track(pm_model::TrackId(id))
            .unwrap()
            .unwrap()
            .canonical
    };
    assert!(
        canonical(x).is_none(),
        "the answer for the old file is not stored"
    );
    assert_eq!(s.identify.lock().unwrap().identified, 0);
    // The next run checks it again (the same audio, nothing changed now) and identifies it
    identify::run_with(&s, identify::RunOptions::quick(false))
        .await
        .unwrap();
    assert_eq!(
        canonical(x).unwrap().0.to_url(),
        format!("https://musicbrainz.org/recording/{}", rec(1))
    );
}

/// PR #250 review (low): a recording the owner rejected for a song is left out before the
/// fingerprint decides: its same-audio sibling is identified instead of the song ending
/// "unknown" with the sibling still open.
#[tokio::test]
async fn the_fingerprint_decides_without_rejected_recordings() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    *s.acoustid.write().unwrap() = identify::AcoustIdSettings {
        api_key: "test-key".into(),
        base_url: "http://127.0.0.1:9".into(),
        configured: String::new(),
    };
    let path = dir.path().join("music/Solo/01 song.flac");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "solo audio").unwrap();
    let mut t = untagged_track(path.clone(), "Over My Shoulder", "Band");
    t.track_no = Some(1);
    t.duration_ms = Some(232_000);
    t.file.duration_ms = Some(232_000);
    let id = s.library.lock().unwrap().upsert_track(&t).unwrap().0;
    let fp = |r: &str| {
        serde_json::json!({"id": r, "title": "Over My Shoulder", "duration": 232.0, "artists": [{"name": "Band"}],
        "releasegroups": [{"id": "g", "title": "Hits", "type": "Album", "releases": [{"id": REL, "title": "Hits", "track_count": 9, "mediums": [{"position": 1, "track_count": 9, "tracks": [{"position": 4}]}]}]}]})
    };
    {
        let lib = s.library.lock().unwrap();
        let key = Library::file_key(&path).unwrap();
        lib.save_fingerprint(&key, 232, "AQfake").unwrap();
        let answer = serde_json::json!({"status": "ok", "results": [{"id": "x", "score": 0.96, "recordings": [fp(&rec(1)), fp(&rec(2))]}]});
        lib.save_lookup(&key, &answer.to_string()).unwrap();
        // The owner said "Not this" to recording 1 (the one picked otherwise: the lowest id)
        lib.add_suggestions(
            id,
            &[option(&rec(1), None, "Over My Shoulder", 0.9, "fingerprint")],
        )
        .unwrap();
        let sid = lib.suggestions_for(id).unwrap()[0].id;
        lib.reject_suggestion(sid).unwrap();
    }
    rusqlite::Connection::open(dir.path().join("lib.sqlite"))
        .unwrap()
        .execute("DELETE FROM identify_state WHERE track_id = ?1", [id])
        .unwrap();
    identify::run_with(&s, identify::RunOptions::quick(false))
        .await
        .unwrap();
    let (uri, _) = s
        .library
        .lock()
        .unwrap()
        .get_track(pm_model::TrackId(id))
        .unwrap()
        .unwrap()
        .canonical
        .expect("its sibling");
    assert_eq!(
        uri.to_url(),
        format!("https://musicbrainz.org/recording/{}", rec(2))
    );
}

/// PR #250 review (medium): album review pages by folder: `after` the last folder shown, so
/// albums confirmed meanwhile make it skip none; `next_after` says where the next page starts
/// (`offset` still works).
#[tokio::test]
async fn album_review_pages_by_folder() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let mut first_of = Vec::new();
    for (k, folder) in ["A/One", "B/Two", "C/Three"].iter().enumerate() {
        let id = song(&s, dir.path(), folder, 1, &format!("Song {k}"));
        suggest(
            &s,
            id,
            &[option(
                &rec(k as u32 + 1),
                None,
                &format!("Song {k}"),
                0.95,
                "album",
            )],
        );
        first_of.push(id);
    }
    let (_, p1) = call(&app, "GET", "/v1/library/suggestions/albums?limit=1", None).await;
    assert_eq!(p1["albums"][0]["album"], "One");
    let after = p1["next_after"].as_str().expect("a cursor").to_string();
    assert!(after.ends_with("A/One"), "{after}");
    // The first album is confirmed whole before the next page is asked for
    let o = &p1["albums"][0]["songs"][0];
    let (st, _) = call(&app, "POST", "/v1/library/suggestions/confirm", Some(serde_json::json!({"choices": [{"track_id": o["track"]["id"], "suggestion_id": o["options"][0]["id"], "recording_mbid": o["options"][0]["recording_mbid"]}]}))).await;
    assert_eq!(st, StatusCode::OK);
    let q = format!(
        "/v1/library/suggestions/albums?limit=1&after={}",
        urlencode(&after)
    );
    let (_, p2) = call(&app, "GET", &q, None).await;
    assert_eq!(p2["albums"][0]["album"], "Two", "nothing skipped: {p2}");
    let (_, p3) = call(
        &app,
        "GET",
        &format!(
            "/v1/library/suggestions/albums?limit=1&after={}",
            urlencode(p2["next_after"].as_str().unwrap())
        ),
        None,
    )
    .await;
    assert_eq!(p3["albums"][0]["album"], "Three");
    assert!(p3["next_after"].is_null());
    // By offset, as before
    let (_, o1) = call(
        &app,
        "GET",
        "/v1/library/suggestions/albums?limit=1&offset=1",
        None,
    )
    .await;
    assert_eq!(o1["albums"][0]["album"], "Three");
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// PR #250 delta review (high): the run reads the songs and what each was (its file and tags) as
/// one snapshot. A rescan replacing a file right after the run read the library is caught: the
/// answer for the old file is not stored; the next run finds the song.
#[tokio::test]
async fn a_rescan_right_after_the_run_reads_the_library_is_caught() {
    const RX: &str = "ab000000-0000-4000-8000-0000000000ab";
    let asked = Arc::new(Asked::default());
    let port = mock_mb(
        vec![("Snap".into(), vec![edition(RX, "Snap", 1, &[], &[])])],
        asked,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("tests@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    let x = album_folder(&s, dir.path(), "Band/Snap", 1)[0];
    let key = Arc::as_ptr(&s) as usize;
    {
        let st = s.clone();
        let once = std::sync::atomic::AtomicBool::new(true);
        identify::AFTER_SNAPSHOT.lock().unwrap().push((
            key,
            Box::new(move || {
                if once.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    let lib = st.library.lock().unwrap();
                    let mut t = lib.get_track(pm_model::TrackId(x)).unwrap().unwrap();
                    t.file.size_bytes += 4096;
                    t.file.mtime_unix += 30;
                    lib.upsert_track(&t).unwrap();
                }
            }),
        ));
    }
    identify::run_with(&s, identify::RunOptions::quick(false))
        .await
        .unwrap();
    let canonical = |id: i64| {
        s.library
            .lock()
            .unwrap()
            .get_track(pm_model::TrackId(id))
            .unwrap()
            .unwrap()
            .canonical
    };
    let stale = canonical(x);
    identify::AFTER_SNAPSHOT
        .lock()
        .unwrap()
        .retain(|(k, _)| *k != key);
    assert!(stale.is_none(), "the answer for the file as it was is not stored");
    assert_eq!(s.identify.lock().unwrap().identified, 0);
    identify::run_with(&s, identify::RunOptions::quick(false))
        .await
        .unwrap();
    assert_eq!(
        canonical(x).unwrap().0.to_url(),
        format!("https://musicbrainz.org/recording/{}", rec(1))
    );
}

// ---- owner, 2026-11-06: fewer songs to review, and review optional ----

/// A song shaped like the pending replay's (`cohort`, `n`: what [`pending_mock`] answers for it),
/// in a folder of its own, with an earlier fingerprint kept.
fn pending_song(s: &NodeState, dir: &std::path::Path, cohort: char, n: u32) -> i64 {
    let folder = dir.join(format!("music/{cohort}{n}"));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("01 song.flac");
    std::fs::write(&path, format!("{cohort}{n} audio")).unwrap();
    let len = 200_000 + (n as u64 % 50) * 1000;
    let mut t = untagged_track(
        path.clone(),
        &format!("{cohort}{n:05} Song"),
        &format!("Artist {}", n / 12),
    );
    t.track_no = Some(1);
    t.duration_ms = Some(len);
    t.file.duration_ms = Some(len);
    t.album = Some(pm_model::Album {
        id: None,
        title: format!("Album {}", n / 12),
        album_artist: None,
        release_mbid: None,
        release_group_mbid: None,
        year: None,
        upc: None,
        total_discs: None,
    });
    let lib = s.library.lock().unwrap();
    let id = lib.upsert_track(&t).unwrap().0;
    let key = Library::file_key(&path).unwrap();
    lib.save_fingerprint(&key, len / 1000, "AQfake").unwrap();
    lib.save_lookup(
        &key,
        &serde_json::json!({"status": "ok", "results": []}).to_string(),
    )
    .unwrap();
    id
}

/// Waiting for review since older rules (as on a node before this version): their options, no
/// input key, no rules version.
fn left_by_older_rules(s: &NodeState, track: i64, recordings: &[u32]) {
    let options: Vec<_> = recordings
        .iter()
        .map(|r| option(&rec(*r), None, "Old option", 0.8, "search"))
        .collect();
    suggest(s, track, &options);
}

async fn recheck_state(
    dir: &std::path::Path,
    recordings: Arc<std::sync::atomic::AtomicUsize>,
    contact: bool,
) -> Arc<NodeState> {
    let port = pending_mock(recordings, Default::default()).await;
    let s = state(dir, None).await;
    if contact {
        s.set_contact("tests@example.org").unwrap();
    }
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    *s.acoustid.write().unwrap() = identify::AcoustIdSettings {
        api_key: "test-key".into(),
        base_url: format!("http://127.0.0.1:{port}"),
        configured: String::new(),
    };
    s
}

/// "Identify again" (a retry: the node has nothing unchecked) checks the songs older rules left
/// waiting: the file's song on one exact recording is identified, its old options gone; a song
/// with only a live recording keeps waiting, with the options the rules find now; a song the
/// owner left as it is isn't touched. Checked under the current rules, nothing is asked again.
#[tokio::test]
async fn identify_again_checks_songs_older_rules_left_waiting() {
    use std::sync::atomic::Ordering::SeqCst;
    let recordings = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let dir = tempfile::tempdir().unwrap();
    let s = recheck_state(dir.path(), recordings.clone(), true).await;
    let exact = pending_song(&s, dir.path(), 'C', 5);
    let live = pending_song(&s, dir.path(), 'L', 1);
    let left = pending_song(&s, dir.path(), 'C', 7);
    left_by_older_rules(&s, exact, &[91, 92]);
    left_by_older_rules(&s, live, &[93]);
    left_by_older_rules(&s, left, &[94]);
    assert_eq!(s.library.lock().unwrap().skip_review(&[left]).unwrap(), 1);
    let c = s.library.lock().unwrap().identity_counts().unwrap();
    assert_eq!((c.suggested, c.outdated, c.skipped, c.unchecked), (2, 2, 1, 0));
    identify::run_with(&s, identify::RunOptions::quick(true))
        .await
        .unwrap();
    {
        let lib = s.library.lock().unwrap();
        let t = lib.get_track(pm_model::TrackId(exact)).unwrap().unwrap();
        assert_eq!(
            t.canonical.map(|(_, c)| c),
            Some(pm_model::Confidence::TagMatch {
                score: identify::TAG_MATCH_SCORE
            })
        );
        assert!(lib.suggestions_for(exact).unwrap().is_empty());
        let options: Vec<String> = lib
            .suggestions_for(live)
            .unwrap()
            .into_iter()
            .map(|r| r.recording_mbid)
            .collect();
        assert_eq!(
            options,
            vec![pending_id(0xb0, 1, 0)],
            "the live recording, the old option gone"
        );
        assert!(lib
            .get_track(pm_model::TrackId(left))
            .unwrap()
            .unwrap()
            .canonical
            .is_none());
        assert_eq!(
            lib.suggestions_for_tracks(&[left]).unwrap().len(),
            1,
            "its options kept"
        );
        let c = lib.identity_counts().unwrap();
        assert_eq!((c.suggested, c.outdated, c.skipped), (1, 0, 1));
    }
    // Again: nothing changed, nothing asked
    let asked = recordings.load(SeqCst);
    assert!(asked > 0);
    identify::run_with(&s, identify::RunOptions::quick(true))
        .await
        .unwrap();
    assert_eq!(recordings.load(SeqCst), asked);
    assert_eq!(s.identify.lock().unwrap().total, 0);
}

/// At start the node checks songs older rules left waiting once, by itself: only when
/// MusicBrainz can be asked, and not again once they are checked.
#[tokio::test]
async fn the_node_checks_old_suggestions_again_once_by_itself() {
    let recordings = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let dir = tempfile::tempdir().unwrap();
    let s = recheck_state(dir.path(), recordings, false).await;
    let exact = pending_song(&s, dir.path(), 'C', 5);
    left_by_older_rules(&s, exact, &[91]);
    let poll = std::time::Duration::from_millis(10);
    assert!(
        !identify::recheck_when_idle(&s, poll).await,
        "no MusicBrainz contact"
    );
    s.set_contact("tests@example.org").unwrap();
    assert!(identify::recheck_when_idle(&s, poll).await);
    for _ in 0..1000 {
        if !s.identify.lock().unwrap().running {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(!s.identify.lock().unwrap().running);
    {
        let lib = s.library.lock().unwrap();
        assert!(lib
            .get_track(pm_model::TrackId(exact))
            .unwrap()
            .unwrap()
            .canonical
            .is_some());
        assert_eq!(lib.identity_counts().unwrap().outdated, 0);
    }
    assert!(
        !identify::recheck_when_idle(&s, poll).await,
        "nothing left to check again"
    );
}

/// "Leave as is" per album, by songs, or every unsure song; undone the same ways; owner only.
#[tokio::test]
async fn songs_can_be_left_as_they_are_and_brought_back() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), Some("owner-token")).await;
    let app = router(s.clone());
    let owner = |u: &'static str, b: serde_json::Value| {
        let app = app.clone();
        async move { send_as(&app, "owner-token", "POST", u, Some(b)).await }
    };
    let album: Vec<i64> = (1..=2)
        .map(|i| song(&s, dir.path(), "Band/Album", i, &format!("Song {i}")))
        .collect();
    let other = song(&s, dir.path(), "Band/Other", 1, "Other");
    for (i, t) in album.iter().chain([&other]).enumerate() {
        suggest(&s, *t, &[option(&rec(i as u32 + 1), None, "Song", 0.8, "search")]);
    }
    let (st, page) = send_as(&app, "owner-token", "GET", "/v1/library/suggestions/albums", None).await;
    assert_eq!(st, StatusCode::OK);
    let key = page["albums"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["songs"].as_array().unwrap().len() == 2)
        .unwrap()["key"]
        .as_str()
        .unwrap()
        .to_string();
    let (st, b) = owner(
        "/v1/library/suggestions/skip",
        serde_json::json!({"albums": [key]}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["skipped"], 2);
    assert_eq!(
        (b["counts"]["suggested"].as_u64(), b["counts"]["skipped"].as_u64()),
        (Some(1), Some(2))
    );
    let (_, page) = send_as(&app, "owner-token", "GET", "/v1/library/suggestions/albums", None).await;
    assert_eq!(page["songs"], 1);
    let (_, st_now) = send_as(&app, "owner-token", "GET", "/v1/library/identify", None).await;
    assert_eq!(st_now["counts"]["skipped"], 2, "{st_now}");
    let (st, b) = owner(
        "/v1/library/suggestions/unskip",
        serde_json::json!({"albums": [key]}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["restored"], 2);
    let (_, b) = owner("/v1/library/suggestions/skip", serde_json::json!({"all": true})).await;
    assert_eq!(
        (b["skipped"].as_u64(), b["counts"]["suggested"].as_u64()),
        (Some(3), Some(0))
    );
    let (_, b) = owner("/v1/library/suggestions/unskip", serde_json::json!({"all": true})).await;
    assert_eq!(b["restored"], 3);
    let (_, b) = owner(
        "/v1/library/suggestions/skip",
        serde_json::json!({"track_ids": [other]}),
    )
    .await;
    assert_eq!(b["skipped"], 1);
    // One way at a time
    for bad in [
        serde_json::json!({}),
        serde_json::json!({"all": true, "track_ids": [other]}),
    ] {
        let (st, _) = owner("/v1/library/suggestions/skip", bad).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
    }
    // The owner only
    let pk = "tbepfp8fisyiqpskbmxsmpub47gxb7xdsh5x9ut9qrb97gcq9phy";
    s.library
        .lock()
        .unwrap()
        .set_setting("people", &serde_json::json!({"people": [{"key": pk, "access": "music", "music_enabled": true, "added_at": 1}]}).to_string())
        .unwrap();
    let tok = accounts::mint(&s, pk).unwrap();
    for u in ["/v1/library/suggestions/skip", "/v1/library/suggestions/unskip"] {
        let (st, _) = send_as(&app, &tok, "POST", u, Some(serde_json::json!({"all": true}))).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{u}");
    }
    assert_eq!(s.library.lock().unwrap().identity_counts().unwrap().skipped, 1);
}
