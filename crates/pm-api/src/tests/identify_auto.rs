//! Automatic matching: a run after a scan, automatic settling of unsure songs (`auto`), and the
//! owner's "Wrong match?" correction. Router and job seams with mock providers; nothing reaches
//! the network.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

const REL: &str = "1a5b6a49-4c4f-4d18-9e3d-1c4d1b0a7a3e";
const REL2: &str = "3c7d8c6b-6e6b-4f3a-9a5f-3e6f3d2c9c5a";

fn rec(i: u32) -> String {
    format!("00000000-0000-4000-8000-{i:012}")
}

fn mbid(s: &str) -> pm_model::Mbid {
    pm_model::Mbid::parse(s).unwrap()
}

/// MusicBrainz and AcoustID: searches find nothing, a fingerprint lookup answers `answer`
/// (counted).
async fn mock(lookups: Arc<AtomicUsize>, answer: serde_json::Value) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = Router::new()
        .route(
            "/ws/2/release",
            get(|| async { Json(serde_json::json!({"releases": []})) }),
        )
        .route(
            "/ws/2/recording",
            get(|| async { Json(serde_json::json!({"recordings": []})) }),
        )
        .route(
            "/v2/lookup",
            post(move || {
                lookups.fetch_add(1, SeqCst);
                let a = answer.clone();
                async move { Json(a) }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    port
}

/// A node that asks the mock, with automatic runs after its scans.
async fn auto_node(dir: &std::path::Path, port: u16) -> Arc<NodeState> {
    let s = state(dir, None).await;
    s.set_contact("tests@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    *s.acoustid.write().unwrap() = identify::AcoustIdSettings {
        api_key: "test-key".into(),
        base_url: format!("http://127.0.0.1:{port}"),
        configured: String::new(),
    };
    identify::AUTO_RUNS_FOR.lock().unwrap().push(Arc::downgrade(&s));
    s
}

async fn until(what: &str, mut ok: impl FnMut() -> bool) {
    for _ in 0..3000 {
        if ok() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {what}");
}

fn canonical(s: &NodeState, id: i64) -> Option<(pm_model::MusicUri, pm_model::Confidence)> {
    s.library
        .lock()
        .unwrap()
        .get_track(pm_model::TrackId(id))
        .unwrap()
        .unwrap()
        .canonical
}

fn recording_of(s: &NodeState, id: i64) -> Option<String> {
    canonical(s, id).map(|(u, _)| match u {
        pm_model::MusicUri::Recording { mbid } => mbid.to_string(),
        other => other.to_url(),
    })
}

fn track_id_by_name(s: &NodeState, name: &str) -> i64 {
    s.library
        .lock()
        .unwrap()
        .all_tracks()
        .unwrap()
        .into_iter()
        .find(|t| {
            t.file
                .path
                .file_name()
                .is_some_and(|f| f.to_string_lossy() == name)
        })
        .and_then(|t| t.id)
        .map(|i| i.0)
        .unwrap()
}

/// B1: a scan that adds a song starts a run for it on its own (on by default, said in the
/// status), and the song is identified; with "Match new songs automatically" off, a scan adds a
/// song and nothing runs.
#[tokio::test]
async fn a_scan_matches_new_songs_by_itself_unless_turned_off() {
    let lookups = Arc::new(AtomicUsize::new(0));
    let port = mock(
        lookups.clone(),
        serde_json::json!({"status": "ok", "results": [{"id": "9ff43b6a-4f16-427c-93c2-92307ca505e0", "score": 0.95,
            "recordings": [{"id": rec(1), "title": "Song One", "artists": [{"name": "Band"}]}]}]}),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let s = auto_node(dir.path(), port).await;
    let app = router(s.clone());
    let (st, b) = call(&app, "GET", "/v1/library/identify", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(b["auto_match"], true, "on by default: {b}");
    let music = dir.path().join("music");
    let album = music.join("Band/Album");
    std::fs::create_dir_all(&album).unwrap();
    tone_wav(&album.join("01 Song One.wav"), 10);
    assert!(s.start_scan(vec![music.clone()]).unwrap());
    until("the scan", || !s.is_scanning()).await;
    let one = track_id_by_name(&s, "01 Song One.wav");
    until("the automatic run", || canonical(&s, one).is_some()).await;
    assert_eq!(recording_of(&s, one), Some(rec(1)));
    until("the run's end", || !s.identify.lock().unwrap().running).await;
    assert_eq!(lookups.load(SeqCst), 1);

    // Turned off: a scan adds a song and no run starts
    let (st, b) = call(
        &app,
        "PUT",
        "/v1/library/identify/settings",
        Some(serde_json::json!({"auto_match": false})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["auto_match"], false, "{b}");
    let finished = s.identify.lock().unwrap().finished_at;
    tone_wav(&album.join("02 Song Two.wav"), 10);
    assert!(s.start_scan(vec![music.clone()]).unwrap());
    until("the scan", || !s.is_scanning()).await;
    until("the waiter", || !s.identify_waiting.load(SeqCst)).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let two = track_id_by_name(&s, "02 Song Two.wav");
    assert!(canonical(&s, two).is_none());
    assert_eq!(s.identify.lock().unwrap().finished_at, finished, "no run");
    assert_eq!(lookups.load(SeqCst), 1);
    let c = s.library.lock().unwrap().identity_counts().unwrap();
    assert_eq!(c.unchecked, 1);
    // On again: the next scan's run takes it
    call(
        &app,
        "PUT",
        "/v1/library/identify/settings",
        Some(serde_json::json!({"auto_match": true})),
    )
    .await;
    identify::after_scan(&s, 1);
    until("the automatic run", || canonical(&s, two).is_some()).await;
}

/// B1: an automatic run never runs beside another run: it waits for the one going, then runs
/// once (one waiter however many scans end), and a scan with nothing new starts none.
#[tokio::test]
async fn an_automatic_run_waits_for_the_run_going_and_runs_once() {
    let lookups = Arc::new(AtomicUsize::new(0));
    let port = mock(lookups, serde_json::json!({"status": "ok", "results": []})).await;
    let dir = tempfile::tempdir().unwrap();
    let s = auto_node(dir.path(), port).await;
    let t = untagged_track(dir.path().join("music/Band/Album/01 Song.flac"), "Song", "Band");
    s.library.lock().unwrap().upsert_track(&t).unwrap();
    // A run going (Identify pressed)
    s.identify.lock().unwrap().running = true;
    identify::after_scan(&s, 1);
    identify::after_scan(&s, 3);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(s.identify_waiting.load(SeqCst), "one waiter");
    assert!(
        s.identify.lock().unwrap().started_at.is_none(),
        "nothing starts while a run goes"
    );
    s.identify.lock().unwrap().running = false;
    until("the automatic run", || {
        let p = s.identify.lock().unwrap();
        p.started_at.is_some() && !p.running && p.finished_at.is_some()
    })
    .await;
    let p = s.identify.lock().unwrap().clone();
    assert_eq!((p.total, p.done), (1, 1), "{p:?}");
    assert!(!s.identify_waiting.load(SeqCst));
    // Nothing new since: no run (the last run's summary stays)
    identify::after_scan(&s, 1);
    until("the waiter", || !s.identify_waiting.load(SeqCst)).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(s.identify.lock().unwrap().finished_at, p.finished_at);
    // Review r1: a WebDAV or drive scan going (no library-folder scan slot): the run waits for it
    let t = untagged_track(dir.path().join("music/Band/Album/02 More.flac"), "More", "Band");
    s.library.lock().unwrap().upsert_track(&t).unwrap();
    s.side_scans.fetch_add(1, SeqCst);
    identify::after_scan(&s, 1);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let unchecked = || s.library.lock().unwrap().identity_counts().unwrap().unchecked;
    assert_eq!(unchecked(), 1, "the run waits for the scan");
    s.side_scans.fetch_sub(1, SeqCst);
    until("the automatic run", || {
        unchecked() == 0 && !s.identify.lock().unwrap().running
    })
    .await;
}

fn fp(i: u32, title: &str, artist: &str, score: f32) -> pm_library::identify::Candidate {
    pm_library::identify::Candidate {
        recording: mbid(&rec(i)),
        release: None,
        title: Some(title.into()),
        artist: Some(artist.into()),
        release_title: None,
        score,
        source: "fingerprint".into(),
    }
}

/// A song waiting for review under the current rules with these options (no file: nothing reads
/// it).
fn waiting(
    s: &NodeState,
    dir: &std::path::Path,
    folder: &str,
    title: &str,
    options: &[pm_library::identify::Candidate],
) -> i64 {
    let t = untagged_track(
        dir.join("music").join(folder).join(format!("{title}.flac")),
        title,
        "Band",
    );
    let lib = s.library.lock().unwrap();
    let id = lib.upsert_track(&t).unwrap().0;
    let done = lib
        .settle(
            id,
            &pm_library::db::SettleAnswer::Suggest {
                candidates: options.to_vec(),
                input_key: None,
                replace: true,
            },
        )
        .unwrap();
    assert_eq!(done, pm_library::db::Settled::Stored);
    id
}

/// A song with an album (`release`: the release its album's tags name).
fn waiting_on(
    s: &NodeState,
    dir: &std::path::Path,
    folder: &str,
    title: &str,
    album: (&str, Option<&str>),
    options: &[pm_library::identify::Candidate],
) -> i64 {
    let id = waiting(s, dir, folder, title, options);
    let lib = s.library.lock().unwrap();
    let mut t = lib.get_track(pm_model::TrackId(id)).unwrap().unwrap();
    t.album = Some(pm_model::Album {
        id: None,
        title: album.0.into(),
        album_artist: Some("Band".into()),
        release_mbid: album.1.map(mbid),
        release_group_mbid: None,
        year: None,
        upc: None,
        total_discs: None,
    });
    lib.upsert_track(&t).unwrap();
    // (a new album: its songs' options were for what the song was, so they are given again)
    lib.settle(
        id,
        &pm_library::db::SettleAnswer::Suggest {
            candidates: options.to_vec(),
            input_key: None,
            replace: true,
        },
    )
    .unwrap();
    id
}

/// The run's kept AcoustID answer for a song's file, as a run keeps it (no file needed): its
/// content key, fingerprint and answer. `recs`: recording, title, artist and length (seconds;
/// `None`: not known).
fn answered(s: &NodeState, id: i64, score: f32, recs: &[(u32, &str, &str, Option<f64>)]) {
    let lib = s.library.lock().unwrap();
    let t = lib.get_track(pm_model::TrackId(id)).unwrap().unwrap();
    let key = format!("key-{id}");
    lib.save_file_key(&t.file.path, 1, 1, &key).unwrap();
    lib.save_fingerprint(&key, 10, "AQfake").unwrap();
    let recordings: Vec<serde_json::Value> = recs
        .iter()
        .map(|(r, title, artist, secs)| {
            serde_json::json!({"id": rec(*r), "title": title, "duration": secs, "artists": [{"name": artist}]})
        })
        .collect();
    let answer = serde_json::json!({"status": "ok", "results": [{"id": "9ff43b6a-4f16-427c-93c2-92307ca505e0", "score": score, "recordings": recordings}]});
    lib.save_lookup(&key, &answer.to_string()).unwrap();
}

/// The start's re-check run (songs older rules left; here none) and the settling at its end.
async fn recheck_run(s: &Arc<NodeState>) {
    identify::run_with(
        s,
        identify::RunOptions {
            outdated_only: true,
            ..identify::RunOptions::quick(false)
        },
    )
    .await
    .unwrap();
}

/// B2 through the job: unsure songs whose evidence is clear settle as `auto` (one audio under
/// several recordings, a title tag the audio contradicts by the same artist, the tags agreeing
/// with one of several songs, the folder's release), counted as `auto`, their options kept;
/// live against studio, another artist, different songs and a song with a rejected recording
/// stay. Write IDs never writes an automatic pick.
#[tokio::test]
async fn unsure_songs_settle_by_clear_evidence_and_real_conflicts_stay() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let d = dir.path();
    let r1 = waiting(
        &s,
        d,
        "A",
        "Song A",
        &[
            fp(12, "Song A", "Band", 0.95),
            fp(11, "Song A (2011 Remaster)", "Band", 0.96),
        ],
    );
    answered(
        &s,
        r1,
        0.95,
        &[
            (12, "Song A", "Band", Some(10.5)),
            (11, "Song A (2011 Remaster)", "Band", Some(10.0)),
        ],
    );
    let r1b = waiting(&s, d, "B", "Track 01", &[fp(21, "Real Song", "Band", 0.96)]);
    answered(&s, r1b, 0.96, &[(21, "Real Song", "Band", Some(10.0))]);
    let r2 = waiting(
        &s,
        d,
        "C",
        "Song C",
        &[
            fp(31, "Song C", "Band", 0.95),
            fp(32, "Something Else", "Other", 0.95),
        ],
    );
    answered(
        &s,
        r2,
        0.95,
        &[
            (31, "Song C", "Band", Some(10.0)),
            (32, "Something Else", "Other", Some(10.0)),
        ],
    );
    // Review r1: a sure recording the run kept no option for (a live take) is still seen
    let hidden = waiting(
        &s,
        d,
        "I",
        "Song I",
        &[fp(91, "Song I", "Band", 0.95), fp(92, "Song I", "Band", 0.95)],
    );
    answered(
        &s,
        hidden,
        0.95,
        &[
            (91, "Song I", "Band", Some(10.0)),
            (92, "Song I", "Band", Some(10.0)),
            (93, "Song I (Live)", "Band", Some(12.0)),
        ],
    );
    // ... no recording of a known length: stays
    let unknown = waiting(
        &s,
        d,
        "K",
        "Song K",
        &[fp(94, "Song K", "Band", 0.95), fp(95, "Song K", "Band", 0.95)],
    );
    answered(
        &s,
        unknown,
        0.95,
        &[(94, "Song K", "Band", None), (95, "Song K", "Band", None)],
    );
    // ... no kept answer at all: stays
    let unanswered = waiting(&s, d, "L", "Song L", &[fp(96, "Song L", "Band", 0.95)]);
    // ... a weak fingerprint and a strong search of one recording stay weak (each source its own
    // score)
    let merged = waiting(&s, d, "M", "Song M", &[fp(97, "Song M", "Band", 0.8)]);
    let mut strong_search = fp(97, "Song M", "Band", 1.0);
    strong_search.source = "search".into();
    s.library
        .lock()
        .unwrap()
        .add_suggestions(merged, &[strong_search])
        .unwrap();
    let row = s.library.lock().unwrap().suggestions_for(merged).unwrap()[0].clone();
    assert_eq!((row.source.as_str(), row.score), ("fingerprint", 0.8));
    // ... a song whose file is gone: stays
    let gone = waiting(&s, d, "N", "Song N", &[fp(98, "Song N", "Band", 0.95)]);
    answered(&s, gone, 0.95, &[(98, "Song N", "Band", Some(10.0))]);
    rusqlite::Connection::open(d.join("lib.sqlite"))
        .unwrap()
        .execute("UPDATE files SET missing = 1 WHERE track_id = ?1", [gone])
        .unwrap();
    // An automatic pick's album tags still vote for its folder's release
    let tag_vote = waiting_on(
        &s,
        d,
        "O",
        "Song O",
        ("Album O", Some(REL2)),
        &[fp(99, "Song O", "Band", 0.95)],
    );
    answered(&s, tag_vote, 0.95, &[(99, "Song O", "Band", Some(10.0))]);
    // R3: the folder's other song is identified on REL; of two searches, one is on it
    let mate = untagged_track(d.join("music/D/Song Z.flac"), "Song Z", "Band");
    let mate = s.library.lock().unwrap().upsert_track(&mate).unwrap().0;
    s.library
        .lock()
        .unwrap()
        .settle(
            mate,
            &pm_library::db::SettleAnswer::Identified {
                recording: pm_model::MusicUri::Recording { mbid: mbid(&rec(40)) },
                confidence: pm_model::Confidence::AlbumFit { score: 0.95 },
                acoustid: None,
                detail: "album_fit".into(),
                release: Some(mbid(REL)),
                input_key: None,
            },
        )
        .unwrap();
    let mut on_rel = fp(41, "Song D", "Band", 0.9);
    on_rel.source = "search".into();
    on_rel.release = Some(mbid(REL));
    let mut elsewhere = fp(42, "Song D", "Band", 0.95);
    elsewhere.source = "search".into();
    elsewhere.release = Some(mbid(REL2));
    let r3 = waiting(&s, d, "D", "Song D", &[on_rel, elsewhere]);
    let live = waiting(
        &s,
        d,
        "E",
        "Song E",
        &[
            fp(51, "Song E", "Band", 0.95),
            fp(52, "Song E (Live)", "Band", 0.95),
        ],
    );
    let other = waiting(&s, d, "F", "Song F", &[fp(61, "Song F", "Other Band", 0.95)]);
    let songs = waiting(
        &s,
        d,
        "G",
        "Song G",
        &[fp(71, "This", "Band", 0.95), fp(72, "That", "Band", 0.95)],
    );
    let rejected = waiting(
        &s,
        d,
        "H",
        "Song H",
        &[fp(81, "Song H", "Band", 0.95), fp(82, "Song H", "Band", 0.95)],
    );
    let row = s
        .library
        .lock()
        .unwrap()
        .suggestions_for(rejected)
        .unwrap()
        .into_iter()
        .find(|r| r.recording_mbid == rec(82))
        .unwrap();
    let (st, _) = call(
        &app,
        "DELETE",
        &format!(
            "/v1/library/suggestions/{}?track_id={rejected}&recording_mbid={}",
            row.id,
            rec(82)
        ),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    recheck_run(&s).await;
    let auto = |id: i64| matches!(canonical(&s, id), Some((_, pm_model::Confidence::Auto { .. })));
    assert_eq!(recording_of(&s, r1), Some(rec(11)), "R1: the lowest id");
    assert_eq!(recording_of(&s, r1b), Some(rec(21)), "R1b");
    assert_eq!(recording_of(&s, r2), Some(rec(31)), "R2");
    assert_eq!(recording_of(&s, r3), Some(rec(41)), "R3");
    assert!(auto(r1) && auto(r1b) && auto(r2) && auto(r3));
    assert_eq!(
        s.library.lock().unwrap().identify_release(r3).unwrap().as_deref(),
        Some(REL),
        "the folder names the release"
    );
    assert_eq!(
        s.library.lock().unwrap().identify_release(r1).unwrap(),
        None,
        "the edition stays open"
    );
    for id in [
        live, other, songs, rejected, hidden, unknown, unanswered, merged, gone,
    ] {
        assert!(canonical(&s, id).is_none(), "{id} stays for the owner");
    }
    assert!(auto(tag_vote));
    // The options an automatic pick was chosen from stay, for "Wrong match?"
    assert_eq!(s.library.lock().unwrap().suggestions_for(r1).unwrap().len(), 2);
    let p = s.identify.lock().unwrap().clone();
    assert_eq!(p.settled.values().sum::<usize>(), 5, "{p:?}");
    let (_, b) = call(&app, "GET", "/v1/library/identify", None).await;
    assert_eq!(b["counts"]["by"]["auto"], 5, "{b}");
    assert_eq!(b["counts"]["suggested"], 9, "{b}");
    // Again: the same picks; a song added to the automatic pick's folder since has its album's
    // release from the pick's tags
    let mut on_tag = fp(100, "Song P", "Band", 0.9);
    on_tag.source = "search".into();
    on_tag.release = Some(mbid(REL2));
    let late = waiting_on(&s, d, "O", "Song P", ("Album P", None), &[on_tag]);
    recheck_run(&s).await;
    assert_eq!(recording_of(&s, r1), Some(rec(11)));
    assert_eq!(
        recording_of(&s, late),
        Some(rec(100)),
        "R3 by the pick's album tags"
    );
    let (_, b) = call(&app, "GET", "/v1/library/identify", None).await;
    assert_eq!(b["counts"]["by"]["auto"], 6, "{b}");
    // Write IDs writes the folder's song, never an automatic pick
    call(
        &app,
        "PUT",
        "/v1/library/identify/settings",
        Some(serde_json::json!({"write_ids": true})),
    )
    .await;
    let (st, w) = call(&app, "POST", "/v1/library/identify/write", None).await;
    assert_eq!(st, StatusCode::OK, "{w}");
    let tried: Vec<i64> = w["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["track_id"].as_i64().unwrap())
        .collect();
    assert_eq!(tried, vec![mate], "{w}");
}

/// A song with a file and an earlier run's fingerprint naming two recordings of its song, one of
/// them of no known length (so the run's own rule leaves it unsure; the settling picks the one
/// whose length fits).
fn fingerprinted(s: &NodeState, dir: &std::path::Path) -> i64 {
    let path = dir.join("music/Band/Album/01 Song.flac");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "the song's audio").unwrap();
    let mut t = untagged_track(path.clone(), "Song", "Band");
    t.album = None;
    let lib = s.library.lock().unwrap();
    let id = lib.upsert_track(&t).unwrap().0;
    let key = Library::file_key(&path).unwrap();
    lib.save_fingerprint(&key, 200, "AQfake").unwrap();
    let answer = serde_json::json!({"status": "ok", "results": [{"id": "9ff43b6a-4f16-427c-93c2-92307ca505e0", "score": 0.95,
        "recordings": [{"id": rec(1), "title": "Song", "duration": 10.0, "artists": [{"name": "Band"}]}, {"id": rec(2), "title": "Song", "artists": [{"name": "Band"}]}]}]});
    lib.save_lookup(&key, &answer.to_string()).unwrap();
    id
}

/// B3 and the rejection rules: an automatic pick shown in "Wrong match?" with the options it was
/// chosen from; "Not matched" rejects it and leaves the song as it is; neither a retry, an
/// automatic run, the settling, bringing the song back nor a fresh check brings the rejected
/// recording back.
#[tokio::test]
async fn a_rejected_match_never_comes_back() {
    let lookups = Arc::new(AtomicUsize::new(0));
    let port = mock(
        lookups.clone(),
        serde_json::json!({"status": "ok", "results": []}),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let s = auto_node(dir.path(), port).await;
    let app = router(s.clone());
    let id = fingerprinted(&s, dir.path());
    identify::run_with(&s, identify::RunOptions::quick(false))
        .await
        .unwrap();
    assert_eq!(
        recording_of(&s, id),
        Some(rec(1)),
        "one audio, two recordings: R1"
    );
    assert!(matches!(
        canonical(&s, id),
        Some((_, pm_model::Confidence::Auto { .. }))
    ));
    let (st, v) = call(
        &app,
        "GET",
        &format!("/v1/library/identify/fix?tracks={id}"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let song = &v["albums"][0]["songs"][0];
    assert_eq!(song["current"]["recording_mbid"], rec(1), "{v}");
    assert_eq!(song["current"]["auto"], true, "{v}");
    assert_eq!(song["status"], "current", "{v}");
    let options: Vec<&str> = song["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["recording_mbid"].as_str().unwrap())
        .collect();
    assert!(options.contains(&rec(2).as_str()), "{v}");
    // A page that saw another match changes nothing
    let (st, _) = call(
        &app,
        "POST",
        &format!("/v1/library/tracks/{id}/match"),
        Some(serde_json::json!({"none": true, "expect": rec(2)})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    let (st, b) = call(
        &app,
        "POST",
        &format!("/v1/library/tracks/{id}/match"),
        Some(serde_json::json!({"none": true, "expect": rec(1)})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert!(b.get("files").is_none(), "Write IDs is off: {b}");
    assert!(canonical(&s, id).is_none());
    assert!(s.library.lock().unwrap().is_rejected(id, &rec(1)).unwrap());
    assert_eq!(s.library.lock().unwrap().identity_counts().unwrap().skipped, 1);
    // A retry, an automatic run: the song stays as the owner left it
    identify::run_with(&s, identify::RunOptions::quick(true))
        .await
        .unwrap();
    assert!(!identify::run_after_scan(&s, std::time::Duration::from_millis(10)).await);
    assert!(canonical(&s, id).is_none());
    // Brought back: it waits for review with the other option, and the settling leaves a song
    // the owner rejected a recording for to the owner
    let (st, b) = call(
        &app,
        "POST",
        "/v1/library/suggestions/unskip",
        Some(serde_json::json!({"track_ids": [id]})),
    )
    .await;
    assert_eq!((st, b["restored"].as_u64()), (StatusCode::OK, Some(1)), "{b}");
    identify::run_with(&s, identify::RunOptions::quick(true))
        .await
        .unwrap();
    assert!(canonical(&s, id).is_none());
    let left: Vec<String> = s
        .library
        .lock()
        .unwrap()
        .suggestions_for(id)
        .unwrap()
        .into_iter()
        .map(|r| r.recording_mbid)
        .collect();
    assert_eq!(left, vec![rec(2)]);
    // Its tags corrected: checked again from the kept fingerprint, never the rejected recording
    {
        let lib = s.library.lock().unwrap();
        let mut t = lib.get_track(pm_model::TrackId(id)).unwrap().unwrap();
        t.title = "Song (Album Version)".into();
        lib.upsert_track(&t).unwrap();
    }
    assert!(identify::run_after_scan(&s, std::time::Duration::from_millis(10)).await);
    until("the automatic run", || !s.identify.lock().unwrap().running).await;
    assert_eq!(recording_of(&s, id), Some(rec(2)));
    assert_eq!(lookups.load(SeqCst), 0, "all from the kept answer");
}

/// B3: "Wrong match?" with Write IDs off leaves the file as it is; on, the file's IDs are
/// replaced (backed up first) and removed for "Not matched". Keeping an automatic pick makes it
/// the owner's. Pubky tags on the old recording stay. A song matched by its own tags, a stale
/// page and a listener are refused.
#[tokio::test]
async fn wrong_match_rewrites_the_file_only_with_write_ids_on() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let music = dir.path().join("music/Band/Album");
    std::fs::create_dir_all(&music).unwrap();
    let identified = |name: &str, recording: u32| {
        let path = music.join(name);
        tone_wav(&path, 1);
        let t = untagged_track(path, name, "Band");
        let lib = s.library.lock().unwrap();
        let id = lib.upsert_track(&t).unwrap().0;
        lib.settle(
            id,
            &pm_library::db::SettleAnswer::Identified {
                recording: pm_model::MusicUri::Recording {
                    mbid: mbid(&rec(recording)),
                },
                confidence: pm_model::Confidence::AlbumFit { score: 0.95 },
                acoustid: None,
                detail: "album_fit".into(),
                release: Some(mbid(REL)),
                input_key: None,
            },
        )
        .unwrap();
        id
    };
    let path_of = |id: i64| {
        s.library
            .lock()
            .unwrap()
            .get_track(pm_model::TrackId(id))
            .unwrap()
            .unwrap()
            .file
            .path
    };
    let set = |id: i64, body: serde_json::Value| {
        let app = app.clone();
        async move {
            call(
                &app,
                "POST",
                &format!("/v1/library/tracks/{id}/match"),
                Some(body),
            )
            .await
        }
    };

    // Write IDs off: the match changes, the file doesn't
    let a = identified("01 A.wav", 1);
    let before = std::fs::read(path_of(a)).unwrap();
    let old_url = format!("https://musicbrainz.org/recording/{}", rec(1));
    s.library
        .lock()
        .unwrap()
        .add_tag("tag-1", &old_url, "favourite", 1)
        .unwrap();
    let (st, b) = set(a, serde_json::json!({"recording_mbid": rec(2), "expect": rec(1)})).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["replaced"], rec(1), "{b}");
    assert!(b.get("files").is_none(), "{b}");
    assert_eq!(
        std::fs::read(path_of(a)).unwrap(),
        before,
        "the file is untouched"
    );
    assert_eq!(recording_of(&s, a), Some(rec(2)));
    assert_eq!(canonical(&s, a).unwrap().1, pm_model::Confidence::ConfirmedByUser);
    assert!(s.library.lock().unwrap().is_rejected(a, &rec(1)).unwrap());
    // Pubky tags on the old recording stay where they are
    let tags = s
        .library
        .lock()
        .unwrap()
        .list_tags(Some(old_url.as_str()))
        .unwrap();
    assert_eq!(tags.len(), 1);
    assert!(!tags[0].deleted);
    // A stale page, a bad body
    let (st, _) = set(a, serde_json::json!({"recording_mbid": rec(3), "expect": rec(1)})).await;
    assert_eq!(st, StatusCode::CONFLICT);
    let (st, _) = set(a, serde_json::json!({"expect": rec(2)})).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = set(a, serde_json::json!({"recording_mbid": "nope", "expect": rec(2)})).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    // Matched by its own tags: corrected in the file, not here
    let mut tagged = untagged_track(music.join("03 T.wav"), "T", "Band");
    tagged.canonical = Some((
        pm_model::MusicUri::Recording { mbid: mbid(&rec(7)) },
        pm_model::Confidence::TaggedMbid,
    ));
    let tagged = s.library.lock().unwrap().upsert_track(&tagged).unwrap().0;
    let (st, _) = set(
        tagged,
        serde_json::json!({"recording_mbid": rec(8), "expect": rec(7)}),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    // Review r1: IDs Write IDs put into the file (no scan since) count as the file's own
    let written = identified("05 W.wav", 30);
    s.library
        .lock()
        .unwrap()
        .set_identify_state(written, "identified", Some("written into the file"))
        .unwrap();
    let (st, _) = set(
        written,
        serde_json::json!({"recording_mbid": rec(31), "expect": rec(30)}),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert_eq!(recording_of(&s, written), Some(rec(30)));

    // Write IDs on: the file's IDs replaced, its tags backed up first
    call(
        &app,
        "PUT",
        "/v1/library/identify/settings",
        Some(serde_json::json!({"write_ids": true})),
    )
    .await;
    // ... a song whose tags name a recording, whose file can't take the change (it is gone):
    // nothing changes (the next scan would bring the old recording back)
    let (st, b) = set(
        tagged,
        serde_json::json!({"recording_mbid": rec(8), "expect": rec(7)}),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{b}");
    assert_eq!(recording_of(&s, tagged), Some(rec(7)));
    assert!(!s.library.lock().unwrap().is_rejected(tagged, &rec(7)).unwrap());
    // ... one whose file can: its IDs replaced, and the next scan keeps the owner's choice
    let path = music.join("04 U.wav");
    tone_wav(&path, 1);
    pm_library::tagwrite::write_ids(
        &path,
        &pm_library::tagwrite::Ids {
            recording: mbid(&rec(20)),
            release: Some(mbid(REL)),
            acoustid: None,
        },
        &dir.path().join("first-ids"),
    )
    .unwrap();
    let mut u = untagged_track(path.clone(), "U", "Band");
    u.recording_mbid = Some(mbid(&rec(20)));
    u.canonical = Some((
        pm_model::MusicUri::Recording { mbid: mbid(&rec(20)) },
        pm_model::Confidence::TaggedMbid,
    ));
    let u = s.library.lock().unwrap().upsert_track(&u).unwrap().0;
    let (st, b) = set(
        u,
        serde_json::json!({"recording_mbid": rec(21), "expect": rec(20)}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["files"]["written"], true, "{b}");
    let (_, tags) = pm_library::probe_and_read(&path).unwrap();
    assert_eq!(tags.recording_mbid.map(|m| m.to_string()), Some(rec(21)));
    let b2 = identified("02 B.wav", 5);
    let (st, b) = set(
        b2,
        serde_json::json!({"recording_mbid": rec(6), "release_mbid": REL2, "expect": rec(5)}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["files"]["written"], true, "{b}");
    let backups = dir.path().join("tag-backups");
    assert!(
        std::fs::read_dir(&backups).unwrap().count() >= 1,
        "backed up first"
    );
    let (_, tags) = pm_library::probe_and_read(&path_of(b2)).unwrap();
    assert_eq!(tags.recording_mbid.map(|m| m.to_string()), Some(rec(6)));
    assert_eq!(tags.release_mbid.map(|m| m.to_string()).as_deref(), Some(REL2));
    // Not matched: the IDs come out of the file
    let (st, b) = set(b2, serde_json::json!({"none": true, "expect": rec(6)})).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["files"]["written"], true, "{b}");
    let (_, tags) = pm_library::probe_and_read(&path_of(b2)).unwrap();
    assert_eq!(tags.recording_mbid, None);
    assert_eq!(tags.release_mbid, None);
    assert!(canonical(&s, b2).is_none());
    // Review r1: the next scan of the rewritten files keeps the owner's choices: the corrected
    // recording, and the song left without a match out of later runs
    assert!(s.start_scan(vec![dir.path().join("music")]).unwrap());
    until("the scan", || !s.is_scanning()).await;
    assert_eq!(recording_of(&s, u), Some(rec(21)));
    assert!(canonical(&s, b2).is_none());
    let c = s.library.lock().unwrap().identity_counts().unwrap();
    assert_eq!(c.skipped, 1, "{c:?}");

    // Keeping an automatic pick makes it the owner's (and Write IDs then writes it)
    let c = waiting(
        &s,
        dir.path(),
        "Band/Album",
        "Song C",
        &[fp(9, "Song C", "Band", 0.95)],
    );
    {
        let lib = s.library.lock().unwrap();
        let was = lib.input_generations().unwrap().get(&c).cloned();
        assert!(was.is_some());
        assert!(lib
            .settle_auto(c, &mbid(&rec(9)), None, 0.95, "auto: R1", was.as_deref())
            .unwrap());
    }
    let (st, b) = set(c, serde_json::json!({"recording_mbid": rec(9), "expect": rec(9)})).await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(canonical(&s, c).unwrap().1, pm_model::Confidence::ConfirmedByUser);
    assert!(!s.library.lock().unwrap().is_rejected(c, &rec(9)).unwrap());
}

/// "Wrong match?" is the owner's: a listener's requests are refused at the router.
#[tokio::test]
async fn a_listener_cannot_correct_a_match() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), Some("owner-token")).await;
    let app = router(s.clone());
    let key = "tbepfp8fisyiqpskbmxsmpub47gxb7xdsh5x9ut9qrb97gcq9phy";
    s.library
        .lock()
        .unwrap()
        .set_setting(
            "people",
            &serde_json::json!({"people": [{"key": key, "access": "music", "music_enabled": true, "added_at": 1}]}).to_string(),
        )
        .unwrap();
    let listener = accounts::mint(&s, key).unwrap();
    let id = s
        .library
        .lock()
        .unwrap()
        .upsert_track(&untagged_track(dir.path().join("music/a.flac"), "A", "Band"))
        .unwrap()
        .0;
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
            r.status()
        }
    };
    for (m, p, b) in [
        ("GET", format!("/v1/library/identify/fix?tracks={id}"), None),
        (
            "POST",
            format!("/v1/library/tracks/{id}/match"),
            Some(serde_json::json!({"none": true})),
        ),
        ("GET", format!("/v1/library/tracks/{id}/candidates?q=a"), None),
    ] {
        assert_eq!(
            send(m, p.clone(), listener.clone(), b.clone()).await,
            StatusCode::FORBIDDEN,
            "{m} {p}"
        );
        // The owner gets through (a 200, or an answer of the handler's own)
        assert_ne!(
            send(m, p.clone(), "owner-token".into(), b).await,
            StatusCode::FORBIDDEN,
            "{m} {p}"
        );
    }
}

/// The owner's words find MusicBrainz recordings for "Wrong match?", also for a song with a match
/// (nothing is stored for it).
#[tokio::test]
async fn the_wrong_match_search_takes_the_owners_words() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let asked = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let a2 = asked.clone();
    let mock = Router::new().route(
        "/ws/2/recording",
        get(move |Query(q): Query<std::collections::HashMap<String, String>>| {
            a2.lock().unwrap().push(q.get("query").cloned().unwrap_or_default());
            async move {
                Json(serde_json::json!({"recordings": [
                    {"id": rec(3), "score": 97, "title": "The Right Song", "length": 200000, "artist-credit": [{"name": "Band"}],
                     "releases": [{"id": REL, "title": "Album", "status": "Official"}]},
                    {"id": rec(4), "score": 20, "title": "Far Off", "artist-credit": [{"name": "Someone"}]}
                ]}))
            }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("tests@example.org").unwrap();
    *s.mb_base.write().unwrap() = format!("http://127.0.0.1:{port}/ws/2");
    let app = router(s.clone());
    let mut t = untagged_track(dir.path().join("music/a.flac"), "Wrong", "Band");
    t.canonical = None;
    let id = s.library.lock().unwrap().upsert_track(&t).unwrap().0;
    s.library
        .lock()
        .unwrap()
        .settle(
            id,
            &pm_library::db::SettleAnswer::Identified {
                recording: pm_model::MusicUri::Recording { mbid: mbid(&rec(1)) },
                confidence: pm_model::Confidence::Fingerprint { score: 0.95 },
                acoustid: None,
                detail: "fingerprint".into(),
                release: None,
                input_key: None,
            },
        )
        .unwrap();
    let (st, b) = call(
        &app,
        "GET",
        &format!("/v1/library/tracks/{id}/candidates?q=right%20song%20band"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    let found = b["found"].as_array().unwrap();
    assert_eq!(found.len(), 1, "the strong hits only: {b}");
    assert_eq!(found[0]["recording"], rec(3), "{b}");
    assert_eq!(asked.lock().unwrap().as_slice(), ["right song band"]);
    assert!(
        s.library.lock().unwrap().suggestions_for(id).unwrap().is_empty(),
        "nothing stored"
    );
}

/// The replay of a node's open suggestions (`GET /v1/library/suggestions`, saved to a file) under
/// the current rules, offline: `PM_REPLAY_SUGGESTIONS=<file> cargo test -p pm-api --lib
/// identify_replay_resettle -- --ignored --nocapture`; or, with the evidence the settling reads
/// (kept AcoustID answers, lengths), a copy of a node's library database (`<data dir>/library.sqlite`,
/// copied with `sqlite3 library.sqlite ".backup /tmp/copy.sqlite"`): `PM_REPLAY_DB=/tmp/copy.sqlite`
/// ([`replay_db`]). Each song and its options go into a fresh
/// library as left by older rules, twice: once the settling alone decides on the stored options
/// (`pass_only`), once the start's re-check run goes first with no MusicBrainz contact and no
/// AcoustID key (nothing is asked, nothing read from a cache; a check that fails keeps the
/// song's options) and its settling at the end decides (`recheck_then_pass`). Prints one JSON line
/// of counts: never a title, artist, path or id (asserted).
#[tokio::test]
#[ignore = "replay: PM_REPLAY_SUGGESTIONS=<suggestions.json>"]
async fn identify_replay_resettle() {
    // A copy of a node's library database: its kept answers and lengths decide
    if let Ok(db) = std::env::var("PM_REPLAY_DB") {
        let (out, private) = replay_db(std::path::Path::new(&db)).await;
        assert_counts_only(&out, &private);
        println!("{out}");
        return;
    }
    let path = std::env::var("PM_REPLAY_SUGGESTIONS")
        .expect("PM_REPLAY_DB=<a copy of library.sqlite> or PM_REPLAY_SUGGESTIONS=<file>");
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    // Every text of the input, to make sure none comes out
    let mut private: std::collections::HashSet<String> = Default::default();
    fn texts(v: &serde_json::Value, out: &mut std::collections::HashSet<String>) {
        match v {
            serde_json::Value::String(s) if s.chars().count() >= 4 => {
                out.insert(s.clone());
            }
            serde_json::Value::Array(a) => a.iter().for_each(|x| texts(x, out)),
            serde_json::Value::Object(o) => o.values().for_each(|x| texts(x, out)),
            _ => {}
        }
    }
    for r in &rows {
        texts(r, &mut private);
    }
    let t0 = std::time::Instant::now();
    let (dir1, s1, unreadable) = replay_library(&rows).await;
    let before = s1.library.lock().unwrap().identity_counts().unwrap();
    let pass = identify::auto_settle_pass(&s1).await.unwrap();
    let pass_only = replay_counts(&s1, dir1.path(), &pass.by_rule, &pass.stay);
    let (dir2, s2, _) = replay_library(&rows).await;
    identify::run_with(
        &s2,
        identify::RunOptions {
            outdated_only: true,
            ..identify::RunOptions::quick(false)
        },
    )
    .await
    .unwrap();
    let p = s2.identify.lock().unwrap().clone();
    // Why the rest stays: the settling once more over what is left (it settles nothing more)
    let again = identify::auto_settle_pass(&s2).await.unwrap();
    assert_eq!(again.settled, 0, "one pass settles all it can");
    let recheck = replay_counts(&s2, dir2.path(), &p.settled, &again.stay);
    let took = t0.elapsed().as_secs_f64();
    let out = serde_json::json!({
        "rows": rows.len(),
        "unreadable": unreadable,
        "before": {"suggested": before.suggested},
        "pass_only": pass_only,
        "recheck_then_pass": recheck,
        "secs": (took * 100.0).round() / 100.0,
    });
    assert_counts_only(&out, &private);
    println!("{out}");
}

/// A fresh library with the replay's songs and their options, left waiting by older rules; how
/// many rows could not be read.
async fn replay_library(rows: &[serde_json::Value]) -> (tempfile::TempDir, Arc<NodeState>, usize) {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let mut ids: std::collections::HashMap<i64, i64> = Default::default();
    let mut options: std::collections::BTreeMap<i64, Vec<pm_library::identify::Candidate>> =
        Default::default();
    let mut unreadable = 0usize;
    {
        let lib = s.library.lock().unwrap();
        lib.begin_batch().unwrap();
        for r in rows {
            let old = r["track_id"].as_i64().unwrap_or_default();
            let new = match ids.get(&old) {
                Some(n) => *n,
                None => {
                    let Ok(mut t) = serde_json::from_value::<pm_model::Track>(r["track"].clone()) else {
                        unreadable += 1;
                        continue;
                    };
                    t.id = None;
                    t.canonical = None;
                    let n = lib.upsert_track(&t).unwrap().0;
                    ids.insert(old, n);
                    n
                }
            };
            let Ok(recording) = pm_model::Mbid::parse(r["recording_mbid"].as_str().unwrap_or_default())
            else {
                unreadable += 1;
                continue;
            };
            let text = |k: &str| r[k].as_str().map(str::to_string);
            options
                .entry(new)
                .or_default()
                .push(pm_library::identify::Candidate {
                    recording,
                    release: r["release_mbid"]
                        .as_str()
                        .and_then(|m| pm_model::Mbid::parse(m).ok()),
                    title: text("title"),
                    artist: text("artist"),
                    release_title: text("release_title"),
                    score: r["score"].as_f64().unwrap_or_default() as f32,
                    source: text("source").unwrap_or_default(),
                });
        }
        for (id, c) in &options {
            lib.add_suggestions(*id, c).unwrap();
            // Left by older rules, as on the node before this version
            lib.set_identify_state(*id, "suggested", None).unwrap();
        }
        lib.commit_batch().unwrap();
    }
    (dir, s, unreadable)
}

/// What a replay library holds after the settling: counts only.
fn replay_counts(
    s: &NodeState,
    dir: &std::path::Path,
    rules: &std::collections::BTreeMap<String, usize>,
    stay: &std::collections::BTreeMap<String, usize>,
) -> serde_json::Value {
    let c = s.library.lock().unwrap().identity_counts().unwrap();
    let with_release: i64 = rusqlite::Connection::open(dir.join("lib.sqlite"))
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM tracks t JOIN identify_state s ON s.track_id = t.id
             WHERE t.canonical_confidence_source = 'auto' AND s.release_mbid IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::json!({
        "identified_auto": c.by.auto,
        "suggested": c.suggested,
        "unknown": c.unknown,
        "with_release": with_release,
        "edition_open": c.by.auto as i64 - with_release,
        "rules": rules,
        "stay": stay,
    })
}

/// B1: once a week, a run over "not found" songs last checked before their kept answers expired,
/// their AcoustID answer asked again (the fingerprint reused); songs checked lately wait, and the
/// retry doesn't come back within the week, nor with "Match new songs automatically" off.
#[tokio::test]
async fn the_weekly_retry_takes_old_not_found_songs_once_a_week() {
    let lookups = Arc::new(AtomicUsize::new(0));
    let port = mock(
        lookups.clone(),
        serde_json::json!({"status": "ok", "results": [{"id": "9ff43b6a-4f16-427c-93c2-92307ca505e0", "score": 0.95,
            "recordings": [{"id": rec(5), "title": "Song", "artists": [{"name": "Band"}]}]}]}),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let s = auto_node(dir.path(), port).await;
    let not_found = |name: &str, days: i64| {
        let path = dir.path().join(format!("music/Band/{name}.flac"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("{name} audio")).unwrap();
        let mut t = untagged_track(path.clone(), name, "Band");
        t.album = None;
        let lib = s.library.lock().unwrap();
        let id = lib.upsert_track(&t).unwrap().0;
        let key = Library::file_key(&path).unwrap();
        lib.save_fingerprint(&key, 200, "AQfake").unwrap();
        lib.save_lookup(&key, r#"{"status": "ok", "results": []}"#)
            .unwrap();
        lib.settle(
            id,
            &pm_library::db::SettleAnswer::State {
                state: "unknown".into(),
                detail: None,
                input_key: None,
            },
        )
        .unwrap();
        let then = pm_library::db::now_unix() - days * 24 * 3600;
        let db = rusqlite::Connection::open(dir.path().join("lib.sqlite")).unwrap();
        db.execute(
            "UPDATE identify_state SET checked_at = ?1 WHERE track_id = ?2",
            rusqlite::params![then, id],
        )
        .unwrap();
        db.execute(
            "UPDATE fingerprints SET looked_up_at = ?1 WHERE file_key = ?2",
            rusqlite::params![then, key],
        )
        .unwrap();
        id
    };
    let old = not_found("Old", 40);
    let recent = not_found("Recent", 3);
    assert!(identify::weekly_retry(&s));
    until("the weekly run", || !s.identify.lock().unwrap().running).await;
    assert_eq!(recording_of(&s, old), Some(rec(5)), "asked again, found now");
    assert_eq!(
        lookups.load(SeqCst),
        1,
        "only the old song's answer is asked again"
    );
    assert!(canonical(&s, recent).is_none());
    assert_eq!(s.identify.lock().unwrap().total, 1);
    // Not again this week
    assert!(!identify::weekly_retry(&s));
    // Due again, but turned off
    s.library
        .lock()
        .unwrap()
        .set_setting("identify.weekly_at", "0")
        .unwrap();
    call(
        &router(s.clone()),
        "PUT",
        "/v1/library/identify/settings",
        Some(serde_json::json!({"auto_match": false})),
    )
    .await;
    assert!(!identify::weekly_retry(&s));
}

/// Review r1: "Not this" on the recording an automatic pick just settled on (the owner's page
/// loaded before it) takes the pick back: the song waits again with the other option, and the
/// settling never picks the rejected recording.
#[tokio::test]
async fn rejecting_an_automatic_pick_takes_it_back() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let id = waiting(
        &s,
        dir.path(),
        "Q",
        "Song Q",
        &[fp(1, "Song Q", "Band", 0.95), fp(2, "Song Q", "Band", 0.95)],
    );
    answered(
        &s,
        id,
        0.95,
        &[
            (1, "Song Q", "Band", Some(10.0)),
            (2, "Song Q", "Band", Some(10.0)),
        ],
    );
    recheck_run(&s).await;
    assert_eq!(recording_of(&s, id), Some(rec(1)));
    let row = s
        .library
        .lock()
        .unwrap()
        .suggestions_for(id)
        .unwrap()
        .into_iter()
        .find(|r| r.recording_mbid == rec(1))
        .unwrap();
    let (st, _) = call(
        &app,
        "DELETE",
        &format!(
            "/v1/library/suggestions/{}?track_id={id}&recording_mbid={}",
            row.id,
            rec(1)
        ),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert!(canonical(&s, id).is_none(), "the pick is taken back");
    assert!(s.library.lock().unwrap().is_rejected(id, &rec(1)).unwrap());
    let c = s.library.lock().unwrap().identity_counts().unwrap();
    assert_eq!((c.suggested, c.by.auto), (1, 0), "{c:?}");
    recheck_run(&s).await;
    assert!(
        canonical(&s, id).is_none(),
        "a song with a rejection waits for the owner"
    );
}

/// Why a database can't be replayed as given: it looks like a node's own, in use (its data folder,
/// or a write-ahead log not folded in). A copy made with `sqlite3 library.sqlite ".backup /tmp/copy.sqlite"`
/// replays.
fn replay_refusal(path: &std::path::Path) -> Option<&'static str> {
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    if ["usage.sqlite", "feedback.sqlite", "art", "tag-backups"]
        .iter()
        .any(|n| dir.join(n).exists())
    {
        return Some("this looks like a node's data folder: replay a copy made elsewhere (sqlite3 .backup)");
    }
    let wal = std::path::PathBuf::from(format!("{}-wal", path.display()));
    if std::fs::metadata(&wal).is_ok_and(|m| m.len() > 0) {
        return Some("the database has a write-ahead log in use: replay a copy made with sqlite3 .backup");
    }
    None
}

/// A fresh copy of a library database (the given file is only read), opened as the node opens
/// its own (migrated to this version), with MusicBrainz and AcoustID at a closed port; and every
/// text of its songs and options, to make sure none comes out.
async fn replay_copy(
    path: &std::path::Path,
) -> (
    tempfile::TempDir,
    Arc<NodeState>,
    std::collections::HashSet<String>,
) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(path, dir.path().join("lib.sqlite")).unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("replay@example.org").unwrap();
    *s.mb_base.write().unwrap() = "http://127.0.0.1:9/ws/2".into();
    *s.acoustid.write().unwrap() = identify::AcoustIdSettings {
        api_key: "replay".into(),
        base_url: "http://127.0.0.1:9".into(),
        configured: String::new(),
    };
    let mut private: std::collections::HashSet<String> = Default::default();
    {
        let lib = s.library.lock().unwrap();
        for t in lib.all_tracks().unwrap() {
            private.insert(t.title.clone());
            private.insert(t.file.path.display().to_string());
            private.extend(t.artists.iter().map(|a| a.name.clone()));
            if let Some(a) = &t.album {
                private.insert(a.title.clone());
            }
        }
        for r in lib.suggestions().unwrap() {
            private.insert(r.recording_mbid.clone());
            private.extend(r.title.clone());
            private.extend(r.artist.clone());
            private.extend(r.release_title.clone());
            private.extend(r.release_mbid.clone());
        }
    }
    private.retain(|p| p.chars().count() >= 4);
    (dir, s, private)
}

/// A replay library's counts.
fn replay_identity(c: &pm_library::IdentityCounts) -> serde_json::Value {
    serde_json::json!({"identified": c.identified, "suggested": c.suggested, "unknown": c.unknown, "by": c.by})
}

/// What a replay library holds after the settling, with what settled and stayed and why.
fn replay_after(
    s: &NodeState,
    dir: &std::path::Path,
    rules: &std::collections::BTreeMap<String, usize>,
    stay: &std::collections::BTreeMap<String, usize>,
    why: &std::collections::BTreeMap<String, usize>,
) -> serde_json::Value {
    let c = s.library.lock().unwrap().identity_counts().unwrap();
    let with_release: i64 = rusqlite::Connection::open(dir.join("lib.sqlite"))
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM tracks t JOIN identify_state s ON s.track_id = t.id
             WHERE t.canonical_confidence_source = 'auto' AND s.release_mbid IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::json!({
        "after": replay_identity(&c),
        "rules": rules,
        "stay": stay,
        "why": why,
        "with_release": with_release,
        "edition_open": c.by.auto as i64 - with_release,
    })
}

/// The replay of a copy of a node's library database (`<data dir>/library.sqlite`, copied with
/// `sqlite3 library.sqlite ".backup /tmp/copy.sqlite"`), twice from the same copy: the settling
/// alone on the library as stored (`pass_only`: the songs, their options, the kept AcoustID
/// answers and lengths, the files the library knows present), and what the start-up re-check runs
/// first (`recheck_then_pass`; offline its checks fail and the songs keep their options, so it
/// shows what the re-check can't change). Counts only.
async fn replay_db(path: &std::path::Path) -> (serde_json::Value, std::collections::HashSet<String>) {
    if let Some(why) = replay_refusal(path) {
        panic!("{why}");
    }
    let (dir1, s1, private) = replay_copy(path).await;
    let before = s1.library.lock().unwrap().identity_counts().unwrap();
    let pass = identify::auto_settle_pass(&s1).await.unwrap();
    let pass_only = replay_after(&s1, dir1.path(), &pass.by_rule, &pass.stay, &pass.why);
    let (dir2, s2, _) = replay_copy(path).await;
    identify::run_with(
        &s2,
        identify::RunOptions {
            outdated_only: true,
            ..identify::RunOptions::quick(false)
        },
    )
    .await
    .unwrap();
    let p = s2.identify.lock().unwrap().clone();
    let again = identify::auto_settle_pass(&s2).await.unwrap();
    assert_eq!(again.settled, 0, "one pass settles all it can");
    let mut recheck = replay_after(&s2, dir2.path(), &p.settled, &again.stay, &again.why);
    recheck["rechecked"] = p.total.into();
    recheck["errors"] = p.errors.into();
    let out = serde_json::json!({
        "before": replay_identity(&before),
        "pass_only": pass_only,
        "recheck_then_pass": recheck,
    });
    (out, private)
}

/// The keys a replay line may hold.
const REPLAY_KEYS: &[&str] = &[
    "rows",
    "unreadable",
    "before",
    "after",
    "pass_only",
    "recheck_then_pass",
    "identified",
    "suggested",
    "identified_auto",
    "unknown",
    "by",
    "tags",
    "album",
    "fingerprint",
    "you",
    "title_artist",
    "auto",
    "rechecked",
    "errors",
    "with_release",
    "edition_open",
    "rules",
    "stay",
    "why",
    "secs",
    "R1",
    "R1b",
    "R2",
    "R3",
    "rejected",
    "version",
    "artist",
    "album_says_version",
    "folder",
    "different_songs",
    "weak",
    "no_tags",
    "empty",
    "editions",
    "no_answer",
    "length_unknown",
    "no_file",
    "changed",
    "unreadable",
];

/// A replay line holds counts only: its keys from [`REPLAY_KEYS`], numbers for values, and no
/// text of the input (a text that is one of the line's own words, or has no letter, tells nothing).
fn assert_counts_only(out: &serde_json::Value, private: &std::collections::HashSet<String>) {
    use pm_library::auto_settle::{MARK_WORDS, WHY_WORDS};
    // A key of the line, or a reason's detail (`version/title:live`): listed words only
    fn listed(k: &str) -> bool {
        k.split(['/', ':'])
            .all(|w| REPLAY_KEYS.contains(&w) || WHY_WORDS.contains(&w) || MARK_WORDS.contains(&w))
    }
    fn check(v: &serde_json::Value) {
        match v {
            serde_json::Value::Object(o) => o.iter().for_each(|(k, x)| {
                assert!(listed(k), "unexpected key in the output");
                check(x);
            }),
            serde_json::Value::Number(_) => {}
            _ => panic!("only counts in the output"),
        }
    }
    check(out);
    let line = out.to_string();
    let leaked = private.iter().any(|p| {
        p.chars().any(char::is_alphabetic)
            && !REPLAY_KEYS
                .iter()
                .chain(WHY_WORDS)
                .chain(MARK_WORDS)
                .any(|a| a.contains(p.as_str()))
            && line.contains(p.as_str())
    });
    assert!(!leaked, "the output holds a text of the input");
}

/// The replay of a library database copy, on a scratch library: an unsure song with a kept answer
/// settles, a live take stays, the line holds counts only; a node's data folder is refused.
#[tokio::test]
async fn the_replay_of_a_library_copy_counts_only() {
    let src = tempfile::tempdir().unwrap();
    let path = src.path().join("copy.sqlite");
    {
        let lib = Library::open(&path).unwrap();
        let song = |title: &str, options: &[pm_library::identify::Candidate]| {
            let t = untagged_track(
                src.path().join(format!("music/Private/{title}.flac")),
                title,
                "Band",
            );
            let id = lib.upsert_track(&t).unwrap().0;
            lib.add_suggestions(id, options).unwrap();
            lib.set_identify_state(id, "suggested", None).unwrap();
            let key = format!("key-{id}");
            lib.save_file_key(&t.file.path, 1, 1, &key).unwrap();
            lib.save_fingerprint(&key, 10, "AQfake").unwrap();
            let recordings: Vec<serde_json::Value> = options
                .iter()
                .map(|o| serde_json::json!({"id": o.recording.to_string(), "title": o.title, "duration": 10.0, "artists": [{"name": "Band"}]}))
                .collect();
            let answer = serde_json::json!({"status": "ok", "results": [{"id": "9ff43b6a-4f16-427c-93c2-92307ca505e0", "score": 0.95, "recordings": recordings}]});
            lib.save_lookup(&key, &answer.to_string()).unwrap();
        };
        song(
            "Secret Song",
            &[
                fp(1, "Secret Song", "Band", 0.95),
                fp(2, "Secret Song", "Band", 0.95),
            ],
        );
        song(
            "Hidden Tune",
            &[
                fp(3, "Hidden Tune", "Band", 0.95),
                fp(4, "Hidden Tune (Live)", "Band", 0.95),
            ],
        );
    }
    let (out, private) = replay_db(&path).await;
    assert_counts_only(&out, &private);
    assert!(private.contains("Secret Song"));
    assert_eq!(out["before"]["suggested"], 2, "{out}");
    for mode in ["pass_only", "recheck_then_pass"] {
        let m = &out[mode];
        assert_eq!(m["after"]["suggested"], 1, "{mode}: {out}");
        assert_eq!(m["after"]["by"]["auto"], 1, "{mode}: {out}");
        assert_eq!(m["rules"]["R1"], 1, "{mode}: {out}");
        assert_eq!(m["stay"]["version"], 1, "{mode}: {out}");
        assert_eq!(m["why"]["version/title:live"], 1, "{mode}: {out}");
    }
    assert_eq!(out["recheck_then_pass"]["rechecked"], 2, "{out}");
    // The copy given was only read
    let lib = Library::open(&path).unwrap();
    assert_eq!(lib.identity_counts().unwrap().by.auto, 0);
    // A node's own data folder is refused
    let node = tempfile::tempdir().unwrap();
    std::fs::write(node.path().join("usage.sqlite"), b"").unwrap();
    assert!(replay_refusal(&node.path().join("library.sqlite")).is_some());
    assert!(replay_refusal(&path).is_none());
}

/// An album whose tags name no release is matched when its identified songs agree on one (its
/// page then takes tags); songs on different releases, or only automatic picks, leave it
/// unmatched.
#[tokio::test]
async fn an_album_takes_the_release_its_identified_songs_agree_on() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let song = |album: &str, no: u32, release: &str, confidence: pm_model::Confidence| {
        let mut t = untagged_track(
            dir.path().join(format!("music/{album}/{no:02}.flac")),
            &format!("Song {no}"),
            "Band",
        );
        t.album = Some(pm_model::Album {
            id: None,
            title: album.into(),
            album_artist: Some("Band".into()),
            release_mbid: None,
            release_group_mbid: None,
            year: None,
            upc: None,
            total_discs: None,
        });
        let lib = s.library.lock().unwrap();
        let id = lib.upsert_track(&t).unwrap();
        lib.settle(
            id.0,
            &pm_library::db::SettleAnswer::Identified {
                recording: pm_model::MusicUri::Recording {
                    mbid: mbid(&rec(no + album.len() as u32 * 10)),
                },
                confidence,
                acoustid: None,
                detail: "test".into(),
                release: Some(mbid(release)),
                input_key: None,
            },
        )
        .unwrap();
        lib.get_track(id).unwrap().unwrap().album.unwrap().id.unwrap()
    };
    let fit = pm_model::Confidence::AlbumFit { score: 0.95 };
    let agreed = song("Four", 1, REL, fit);
    song("Four", 2, REL, fit);
    let split = song("Split", 1, REL, fit);
    song("Split", 2, REL2, fit);
    let auto = song("Auto", 1, REL, pm_model::Confidence::Auto { score: 0.95 });
    let tags = |album: i64| {
        let app = app.clone();
        async move {
            call(&app, "GET", &format!("/v1/tags?album_id={album}"), None)
                .await
                .1
        }
    };
    let t = tags(agreed).await;
    assert_eq!(t["uri"], format!("https://musicbrainz.org/release/{REL}"), "{t}");
    assert!(t.get("needs_match").is_none(), "{t}");
    assert_eq!(tags(split).await["needs_match"], true);
    assert_eq!(tags(auto).await["needs_match"], true);
}
