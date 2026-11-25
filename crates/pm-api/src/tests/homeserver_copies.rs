//! Copies and uploads to the homeserver: the guards, jobs, retries and restarts.
//!
//! Part of pm-api's unit tests (`mod tests` in lib.rs, which holds the helpers these use). A new
//! test of this area goes at the end of this file.

use super::*;

/// A copy to the bundled homeserver whose disk is 95 % full is refused before anything is
/// sent (NUC, 2026-11-01: a folder copy filled the disk and took the homeserver's database
/// down); the dry run shows what it needs against what is free.
#[tokio::test]
async fn a_copy_into_the_disks_reserve_is_refused_before_it_starts() {
    let hs = Arc::new(FakeHomeserver::default());
    let disk = Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 98 * GB / 20,
        total: 98 * GB,
    })));
    let (_dir, _s, app, src) = copy_setup(hs.clone(), disk, 2).await;
    let body = |dry: bool| Some(serde_json::json!({"path": src, "dry_run": dry}));
    let (st, dry) = call(&app, "POST", "/v1/library/imports", body(true)).await;
    assert_eq!(st, StatusCode::OK, "{dry}");
    assert_eq!(dry["reserve_bytes"], 5 * GB, "{dry}");
    assert_eq!(dry["free_bytes"], 98 * GB / 20, "{dry}");
    assert!(dry["needed_bytes"].as_u64().unwrap() > 0, "{dry}");
    let (st, e) = call(&app, "POST", "/v1/library/imports", body(false)).await;
    assert_eq!(st, StatusCode::INSUFFICIENT_STORAGE, "{e}");
    assert!(
        e["error"]
            .as_str()
            .unwrap()
            .starts_with("not enough space on this machine"),
        "{e}"
    );
    assert!(hs.uploads.lock().unwrap().is_empty());
}

/// Space running low partway: the copy stops before the file that would eat into the
/// reserve, and says why.
#[tokio::test]
async fn a_copy_stops_before_the_disk_reaches_its_reserve() {
    // Room for the three songs (176, 352 and 529 KB) at the start; each upload also uses
    // 800 KB more on the disk, so after the first the second would eat into the reserve
    let disk = Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 5 * GB + 1_100_000,
        total: 98 * GB,
    })));
    let hs = Arc::new(FakeHomeserver {
        disk: Some(disk.clone()),
        extra: 800_000,
        ..Default::default()
    });
    let (_dir, _s, app, src) = copy_setup(hs.clone(), disk, 0).await;
    // Songs that differ: the same content twice in one job is sent once
    for i in 1..=3 {
        tone_wav(&src.join(format!("{i:02} song.wav")), i);
    }
    let (st, job) = call(
        &app,
        "POST",
        "/v1/library/imports",
        Some(serde_json::json!({"path": src})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{job}");
    let done = copy_finished(&app, job["id"].as_str().unwrap()).await;
    assert_eq!(done["done"], 1, "{done}");
    assert!(
        done["stopped_reason"]
            .as_str()
            .unwrap()
            .contains("not enough space on this machine"),
        "{done}"
    );
    assert_eq!(hs.uploads.lock().unwrap().len(), 1);
}

/// The homeserver failing (500s, as when its disk was full on the NUC) stops the copy after
/// three failures in a row instead of trying every file; nothing half sent is left there.
#[tokio::test]
async fn three_failures_in_a_row_stop_a_copy() {
    let hs = Arc::new(FakeHomeserver::default());
    hs.fails.store(true, std::sync::atomic::Ordering::SeqCst);
    let disk = Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 500 * GB,
        total: 1000 * GB,
    })));
    let (_dir, _s, app, src) = copy_setup(hs.clone(), disk, 6).await;
    let (_, job) = call(
        &app,
        "POST",
        "/v1/library/imports",
        Some(serde_json::json!({"path": src})),
    )
    .await;
    let done = copy_finished(&app, job["id"].as_str().unwrap()).await;
    assert_eq!(done["failed"], 3, "{done}");
    assert!(
        done["stopped_reason"]
            .as_str()
            .unwrap()
            .contains("3 failures in a row"),
        "{done}"
    );
    let uploads = hs.uploads.lock().unwrap().clone();
    assert_eq!(uploads.len(), 3, "no 4th file is sent");
    assert_eq!(
        *hs.deletes.lock().unwrap(),
        uploads,
        "each failed one is deleted there"
    );
}

/// macOS's `._*` AppleDouble files and files that only look like audio are not copied; a
/// short upload is deleted on the homeserver, never left as a truncated song.
#[tokio::test]
async fn only_whole_audio_files_are_copied() {
    let hs = Arc::new(FakeHomeserver::default());
    let disk = Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 500 * GB,
        total: 1000 * GB,
    })));
    let (_dir, _s, app, src) = copy_setup(hs.clone(), disk, 1).await;
    std::fs::write(src.join("._01 song.wav"), vec![0u8; 4096]).unwrap();
    std::fs::write(src.join("02 notes.flac"), b"not audio at all").unwrap();
    // The `._` file isn't counted in the space needed either
    let song = std::fs::metadata(src.join("01 song.wav")).unwrap().len();
    let (_, dry) = call(
        &app,
        "POST",
        "/v1/library/imports",
        Some(serde_json::json!({"path": src, "dry_run": true})),
    )
    .await;
    assert_eq!(dry["needed_bytes"], song + 16, "{dry}");
    let (_, job) = call(
        &app,
        "POST",
        "/v1/library/imports",
        Some(serde_json::json!({"path": src})),
    )
    .await;
    let done = copy_finished(&app, job["id"].as_str().unwrap()).await;
    assert_eq!(
        (done["done"].clone(), done["not_audio"].clone()),
        (1.into(), 2.into()),
        "{done}"
    );
    let uploads = hs.uploads.lock().unwrap().clone();
    assert_eq!(uploads.len(), 1, "{uploads:?}");
    assert!(uploads[0].ends_with("song.wav"), "{uploads:?}");

    // The homeserver keeps less than was sent: deleted there, counted as failed
    hs.short.store(10, std::sync::atomic::Ordering::SeqCst);
    std::fs::remove_file(src.join("._01 song.wav")).unwrap();
    tone_wav(&src.join("03 other.wav"), 2);
    let (_, job) = call(
        &app,
        "POST",
        "/v1/library/imports",
        Some(serde_json::json!({"path": src})),
    )
    .await;
    let done = copy_finished(&app, job["id"].as_str().unwrap()).await;
    assert!(done["failed"].as_u64().unwrap() >= 1, "{done}");
    assert!(
        done["errors"].to_string().contains("the homeserver kept"),
        "{done}"
    );
    let deletes = hs.deletes.lock().unwrap().clone();
    assert!(deletes.iter().any(|d| d.ends_with("other.wav")), "{deletes:?}");
}

/// The library copy (`POST /v1/pubky/files/upload`) has the same guards: three failures in a
/// row stop it, and a disk at 95 % refuses it before it starts.
#[tokio::test]
async fn the_library_copy_has_the_same_guards() {
    let hs = Arc::new(FakeHomeserver::default());
    hs.fails.store(true, std::sync::atomic::Ordering::SeqCst);
    let disk = Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 500 * GB,
        total: 1000 * GB,
    })));
    let (_dir, s, app, src) = copy_setup(hs.clone(), disk.clone(), 5).await;
    for i in 1..=5 {
        let t = untagged_track(src.join(format!("{i:02} song.wav")), &format!("Song {i}"), "Band");
        s.library.lock().unwrap().upsert_track(&t).unwrap();
    }
    let (st, job) = call(
        &app,
        "POST",
        "/v1/pubky/files/upload",
        Some(serde_json::json!({"all": true})),
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED, "{job}");
    let done = copy_finished(&app, job["id"].as_str().unwrap()).await;
    assert_eq!(done["failed"], 3, "{done}");
    assert!(
        done["stopped_reason"].to_string().contains("3 failures in a row"),
        "{done}"
    );
    assert_eq!(hs.uploads.lock().unwrap().len(), 3);

    *disk.0.lock().unwrap() = copy_guard::Space {
        free: 98 * GB / 20,
        total: 98 * GB,
    };
    let (st, e) = call(
        &app,
        "POST",
        "/v1/pubky/files/upload",
        Some(serde_json::json!({"all": true})),
    )
    .await;
    assert_eq!(st, StatusCode::INSUFFICIENT_STORAGE, "{e}");
}

/// A file the homeserver has at another size than the song (its name is the content's hash,
/// so a broken copy) is sent again; when that fails too, what it left is deleted like any half
/// written file, never kept as a truncated song (#224 follow-ups, Kimi 4).
#[tokio::test]
async fn a_failed_repair_of_a_broken_copy_leaves_nothing_half_written() {
    let hs = Arc::new(FakeHomeserver::default());
    hs.fails.store(true, std::sync::atomic::Ordering::SeqCst);
    let disk = Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 500 * GB,
        total: 1000 * GB,
    })));
    let (_dir, s, app, src) = copy_setup(hs.clone(), disk, 1).await;
    let path = src.join("01 song.wav");
    let (remote, len) = copied_name(&s, &path);
    hs.listed.lock().unwrap().push(pm_pubky::homeserver::RemoteFile {
        path: remote.clone(),
        size: None,
    });
    hs.sizes.lock().unwrap().insert(remote.clone(), len / 3);
    let t = untagged_track(path, "Song 1", "Band");
    s.library.lock().unwrap().upsert_track(&t).unwrap();
    let (_, job) = call(
        &app,
        "POST",
        "/v1/pubky/files/upload",
        Some(serde_json::json!({"all": true})),
    )
    .await;
    let done = copy_finished(&app, job["id"].as_str().unwrap()).await;
    assert_eq!(done["failed"], 1, "{done}");
    assert_eq!(
        *hs.uploads.lock().unwrap(),
        std::slice::from_ref(&remote),
        "it was sent again"
    );
    assert_eq!(*hs.deletes.lock().unwrap(), [remote], "the half sent is removed");
}

/// An upload a stopped node left in flight (a crash, a restart) is settled at the next start:
/// one it made that the homeserver keeps at another size is deleted, an empty one included; a
/// complete one, or a file that was there before, stays. One whose size the homeserver doesn't
/// say, or that another account made, stays recorded (#224 review; follow-ups Kimi 2 and 5).
#[tokio::test]
async fn uploads_a_stopped_node_left_half_written_are_removed_at_start() {
    let hs = Arc::new(FakeHomeserver::default());
    let disk = Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 500 * GB,
        total: 1000 * GB,
    })));
    let (_dir, s, _app, _src) = copy_setup(hs.clone(), disk, 0).await;
    let f = |n: &str| format!("/priv/pubky.music/files/{n}");
    {
        let mut sizes = hs.sizes.lock().unwrap();
        for (n, size) in [
            ("half", 40),
            ("whole", 70),
            ("before", 10),
            ("empty", 0),
            ("unknown", 0),
            ("elsewhere", 5),
            ("mine", 5),
        ] {
            sizes.insert(f(n), size);
        }
        hs.no_size.lock().unwrap().insert(f("unknown"));
    }
    s.library
        .lock()
        .unwrap()
        .set_setting(
            copy_guard::IN_FLIGHT_KEY,
            &serde_json::json!({
                f("half"): {"size": 100, "existed": false},
                f("whole"): {"size": 70, "existed": false},
                f("before"): {"size": 50, "existed": true},
                f("empty"): {"size": 30},
                f("unknown"): {"size": 30, "existed": false},
                f("elsewhere"): {"size": 30, "account": "someone-else"},
                f("mine"): {"size": 30, "account": "fake-account"},
            })
            .to_string(),
        )
        .unwrap();
    copy_guard::settle_in_flight(&s, 1, std::time::Duration::ZERO).await;
    let mut deleted = hs.deletes.lock().unwrap().clone();
    deleted.sort();
    assert_eq!(deleted, [f("empty"), f("half"), f("mine")]);
    let left: serde_json::Value = serde_json::from_str(
        &s.library
            .lock()
            .unwrap()
            .get_setting(copy_guard::IN_FLIGHT_KEY)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let mut left: Vec<&String> = left.as_object().unwrap().keys().collect();
    left.sort();
    assert_eq!(left, [&f("elsewhere"), &f("unknown")]);
}

/// Copying a library mostly there already: only what it sends counts against the disk's
/// reserve, so running it again isn't refused for the files it skips (#224 review).
#[tokio::test]
async fn a_copy_run_again_counts_only_what_it_sends() {
    let hs = Arc::new(FakeHomeserver::default());
    // Room for one more song at most: the two already there would not fit again
    let disk = Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 5 * GB + 200_000,
        total: 98 * GB,
    })));
    let (_dir, s, app, src) = copy_setup(hs.clone(), disk, 2).await;
    for i in 1..=2 {
        let path = src.join(format!("{i:02} song.wav"));
        let (remote, len) = copied_name(&s, &path);
        hs.listed.lock().unwrap().push(pm_pubky::homeserver::RemoteFile {
            path: remote,
            size: Some(len),
        });
        let mut t = untagged_track(path, &format!("Song {i}"), "Band");
        t.file.size_bytes = len;
        s.library.lock().unwrap().upsert_track(&t).unwrap();
    }
    let (st, job) = call(
        &app,
        "POST",
        "/v1/pubky/files/upload",
        Some(serde_json::json!({"all": true})),
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED, "{job}");
    assert_eq!(job["needed_bytes"], 0, "{job}");
    let done = copy_finished(&app, job["id"].as_str().unwrap()).await;
    assert_eq!(done["skipped"], 2, "{done}");
    assert!(hs.uploads.lock().unwrap().is_empty());
}

/// A song dropped on the Add music page is refused (507) when it would eat into the reserve
/// of the disk it lands on, as a copy is (#224 review).
#[tokio::test]
async fn a_browser_upload_into_the_disks_reserve_is_refused() {
    let hs = Arc::new(FakeHomeserver::default());
    let disk = Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 98 * GB / 20,
        total: 98 * GB,
    })));
    let (_dir, _s, app, _src) = copy_setup(hs, disk, 0).await;
    let (st, e) = call(
        &app,
        "POST",
        "/v1/library/uploads",
        Some(serde_json::json!({"name": "01 Song.flac", "size": 10_000_000})),
    )
    .await;
    assert_eq!(st, StatusCode::INSUFFICIENT_STORAGE, "{e}");
}

/// An import to another homeserver also writes each file into this node's mirror folder:
/// the reserve that counts is that disk's, not none (#224 review).
#[tokio::test]
async fn an_import_to_another_homeserver_keeps_the_nodes_disk_reserve() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    assert!(
        copy_guard::room_of(&s).space().is_none(),
        "no bundled homeserver here"
    );
    assert!(
        copy_guard::room_for_store(&s).space().is_some(),
        "the mirror folder's disk is measured"
    );
}

/// The crash settle at start waits for the homeserver session, which the node restores only
/// after it looked at its jobs (#224 delta review): it runs once the session is there.
#[tokio::test]
async fn uploads_left_in_flight_are_settled_once_the_session_arrives() {
    let hs = Arc::new(FakeHomeserver::default());
    let disk = Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 500 * GB,
        total: 1000 * GB,
    })));
    let (_dir, s, _app, _src) = copy_setup(hs.clone(), disk, 0).await;
    // No session yet
    s.copy_seams.lock().unwrap().0 = None;
    hs.sizes
        .lock()
        .unwrap()
        .insert("/priv/pubky.music/files/half".into(), 40);
    s.library
        .lock()
        .unwrap()
        .set_setting(
            copy_guard::IN_FLIGHT_KEY,
            &serde_json::json!({"/priv/pubky.music/files/half": {"size": 100, "existed": false}}).to_string(),
        )
        .unwrap();
    let s2 = s.clone();
    let settling = tokio::spawn(async move {
        copy_guard::settle_in_flight(&s2, 50, std::time::Duration::from_millis(20)).await
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        hs.deletes.lock().unwrap().is_empty(),
        "nothing before the session"
    );
    s.copy_seams.lock().unwrap().0 = Some(hs.clone());
    settling.await.unwrap();
    assert_eq!(*hs.deletes.lock().unwrap(), ["/priv/pubky.music/files/half"]);
}

/// A folder imported before the node remembered its files' hashes: running the import again
/// counts what the homeserver lists under the file's name and size as there, so it isn't
/// refused for the songs it would skip (#224 delta review; the NUC's 82 GB).
#[tokio::test]
async fn an_import_run_again_skips_what_was_stored_before_hashes_were_remembered() {
    let hs = Arc::new(FakeHomeserver::default());
    // Room for one more song at most: the two already there would not fit again
    let disk = Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 5 * GB + 200_000,
        total: 98 * GB,
    })));
    let (_dir, _s, app, src) = copy_setup(hs.clone(), disk, 2).await;
    for i in 1..=2 {
        let name = format!("{i:02} song.wav");
        let size = std::fs::metadata(src.join(&name)).unwrap().len();
        hs.listed.lock().unwrap().push(pm_pubky::homeserver::RemoteFile {
            path: format!(
                "{}0123456789abcde{i}-{}",
                pm_pubky::homeserver::PRIVATE_FILES_DIR,
                add_music::safe_name(&name)
            ),
            size: Some(size),
        });
    }
    let (st, job) = call(
        &app,
        "POST",
        "/v1/library/imports",
        Some(serde_json::json!({"path": src})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{job}");
    assert_eq!(job["needed_bytes"], 0, "{job}");
}

/// The single-file upload (`POST /v1/pubky/files`) keeps the bundled homeserver's reserve too.
#[tokio::test]
async fn a_single_file_upload_into_the_disks_reserve_is_refused() {
    let hs = Arc::new(FakeHomeserver::default());
    let disk = Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 98 * GB / 20,
        total: 98 * GB,
    })));
    let (_dir, _s, app, src) = copy_setup(hs, disk, 1).await;
    let (st, e) = call(
        &app,
        "POST",
        "/v1/pubky/files",
        Some(serde_json::json!({"path": src.join("01 song.wav")})),
    )
    .await;
    assert_eq!(st, StatusCode::INSUFFICIENT_STORAGE, "{e}");
}

/// A file the homeserver lists without its size is checked before a copy sends over it: the
/// homeserver has no rename, so a failed upload over the good copy would leave it truncated
/// (#224 review). Kept when its size is the source's, sent over only when known to differ.
#[tokio::test]
async fn a_copy_never_sends_over_a_whole_file_it_could_not_size() {
    let hs = Arc::new(FakeHomeserver::default());
    hs.fails.store(true, std::sync::atomic::Ordering::SeqCst);
    let disk = Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 500 * GB,
        total: 1000 * GB,
    })));
    let (_dir, s, app, src) = copy_setup(hs.clone(), disk, 1).await;
    let path = src.join("01 song.wav");
    let (remote, len) = copied_name(&s, &path);
    hs.listed.lock().unwrap().push(pm_pubky::homeserver::RemoteFile {
        path: remote.clone(),
        size: None,
    });
    hs.sizes.lock().unwrap().insert(remote.clone(), len);
    let mut t = untagged_track(path, "Song 1", "Band");
    t.file.size_bytes = len;
    s.library.lock().unwrap().upsert_track(&t).unwrap();
    let (_, job) = call(
        &app,
        "POST",
        "/v1/pubky/files/upload",
        Some(serde_json::json!({"all": true})),
    )
    .await;
    let done = copy_finished(&app, job["id"].as_str().unwrap()).await;
    assert_eq!(done["skipped"], 1, "{done}");
    assert!(hs.uploads.lock().unwrap().is_empty(), "sent over the good copy");
    assert_eq!(hs.sizes.lock().unwrap()[&remote], len);
}

/// Two files of one job under the same homeserver name: the second isn't sent again, so its
/// failure can never delete the first's good copy (#224 review). The library copy and an
/// import both.
#[tokio::test]
async fn a_second_file_of_a_job_with_the_same_name_leaves_the_first_alone() {
    let hs = Arc::new(FakeHomeserver {
        fail_after: Some(1),
        ..Default::default()
    });
    let disk = Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 500 * GB,
        total: 1000 * GB,
    })));
    let (_dir, s, app, src) = copy_setup(hs.clone(), disk, 0).await;
    for d in ["a", "b"] {
        std::fs::create_dir_all(src.join(d)).unwrap();
        tone_wav(&src.join(d).join("01 song.wav"), 1);
        let t = untagged_track(src.join(d).join("01 song.wav"), "Song 1", "Band");
        s.library.lock().unwrap().upsert_track(&t).unwrap();
    }
    let (_, job) = call(
        &app,
        "POST",
        "/v1/pubky/files/upload",
        Some(serde_json::json!({"all": true})),
    )
    .await;
    let done = copy_finished(&app, job["id"].as_str().unwrap()).await;
    assert_eq!(
        (done["done"].clone(), done["skipped"].clone()),
        (1.into(), 1.into()),
        "{done}"
    );
    assert_eq!(hs.uploads.lock().unwrap().len(), 1);
    assert!(hs.deletes.lock().unwrap().is_empty());

    hs.uploads.lock().unwrap().clear();
    hs.sizes.lock().unwrap().clear();
    let (_, job) = call(
        &app,
        "POST",
        "/v1/library/imports",
        Some(serde_json::json!({"path": src})),
    )
    .await;
    let done = copy_finished(&app, job["id"].as_str().unwrap()).await;
    assert_eq!(
        (done["done"].clone(), done["skipped"].clone()),
        (1.into(), 1.into()),
        "{done}"
    );
    assert!(hs.deletes.lock().unwrap().is_empty());
}

/// When the homeserver can't be asked what it kept after an upload, the file counts as failed
/// and its in-flight record stays for the next start to settle (#224 review).
#[tokio::test]
async fn an_upload_whose_size_cant_be_checked_is_failed_and_kept_in_flight() {
    let hs = Arc::new(FakeHomeserver {
        stat_fails_after_upload: true,
        ..Default::default()
    });
    let disk = Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 500 * GB,
        total: 1000 * GB,
    })));
    let (_dir, s, app, src) = copy_setup(hs.clone(), disk, 1).await;
    let path = src.join("01 song.wav");
    let (remote, _) = copied_name(&s, &path);
    let t = untagged_track(path, "Song 1", "Band");
    s.library.lock().unwrap().upsert_track(&t).unwrap();
    let (_, job) = call(
        &app,
        "POST",
        "/v1/pubky/files/upload",
        Some(serde_json::json!({"all": true})),
    )
    .await;
    let done = copy_finished(&app, job["id"].as_str().unwrap()).await;
    assert_eq!(
        (done["done"].clone(), done["failed"].clone()),
        (0.into(), 1.into()),
        "{done}"
    );
    let flight = s
        .library
        .lock()
        .unwrap()
        .get_setting(copy_guard::IN_FLIGHT_KEY)
        .unwrap()
        .unwrap_or_default();
    assert!(flight.contains(&remote), "{flight}");
}

/// The single-file upload (`POST /v1/pubky/files`) goes the guarded way of a copy: a good
/// copy there is never sent over, and a failed upload leaves nothing half written (#224
/// follow-ups, Kimi 1).
#[tokio::test]
async fn a_single_file_upload_never_sends_over_a_good_copy() {
    let hs = Arc::new(FakeHomeserver::default());
    hs.fails.store(true, std::sync::atomic::Ordering::SeqCst);
    let disk = Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 500 * GB,
        total: 1000 * GB,
    })));
    let (_dir, s, app, src) = copy_setup(hs.clone(), disk, 2).await;
    let one = src.join("01 song.wav");
    let (remote, len) = copied_name(&s, &one);
    hs.sizes.lock().unwrap().insert(remote.clone(), len);
    let (st, v) = call(
        &app,
        "POST",
        "/v1/pubky/files",
        Some(serde_json::json!({"path": one})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["path"], remote.as_str());
    assert!(hs.uploads.lock().unwrap().is_empty(), "sent over the good copy");

    // Nothing there, and the upload fails: what it left is deleted
    std::fs::write(src.join("02 song.wav"), b"RIFF and then some other bytes").unwrap();
    let two = src.join("02 song.wav");
    let (st, v) = call(
        &app,
        "POST",
        "/v1/pubky/files",
        Some(serde_json::json!({"path": two})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_GATEWAY, "{v}");
    let sent = hs.uploads.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(*hs.deletes.lock().unwrap(), sent);
}

/// The homeserver not saying a file's size is never taken for empty: an upload it doesn't
/// confirm counts as failed and stays recorded, and a listed file of a size it doesn't say
/// is kept, not sent over (#224 follow-ups, Kimi 2).
#[tokio::test]
async fn a_size_the_homeserver_doesnt_say_is_not_confirmed() {
    let hs = Arc::new(FakeHomeserver::default());
    let disk = Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 500 * GB,
        total: 1000 * GB,
    })));
    let (_dir, s, app, src) = copy_setup(hs.clone(), disk, 0).await;
    tone_wav(&src.join("01 song.wav"), 1);
    tone_wav(&src.join("02 song.wav"), 2);
    let (one, _) = copied_name(&s, &src.join("01 song.wav"));
    let (two, len2) = copied_name(&s, &src.join("02 song.wav"));
    hs.no_size.lock().unwrap().extend([one.clone(), two.clone()]);
    hs.sizes.lock().unwrap().insert(two.clone(), len2);
    hs.listed.lock().unwrap().push(pm_pubky::homeserver::RemoteFile {
        path: two.clone(),
        size: None,
    });
    for (i, p) in ["01 song.wav", "02 song.wav"].iter().enumerate() {
        let t = untagged_track(src.join(p), &format!("Song {i}"), "Band");
        s.library.lock().unwrap().upsert_track(&t).unwrap();
    }
    let (_, job) = call(
        &app,
        "POST",
        "/v1/pubky/files/upload",
        Some(serde_json::json!({"all": true})),
    )
    .await;
    let done = copy_finished(&app, job["id"].as_str().unwrap()).await;
    assert_eq!(
        (
            done["done"].clone(),
            done["failed"].clone(),
            done["skipped"].clone()
        ),
        (0.into(), 1.into(), 1.into()),
        "{done}"
    );
    assert_eq!(*hs.uploads.lock().unwrap(), std::slice::from_ref(&one));
    assert!(hs.deletes.lock().unwrap().is_empty());
    let flight = s
        .library
        .lock()
        .unwrap()
        .get_setting(copy_guard::IN_FLIGHT_KEY)
        .unwrap()
        .unwrap_or_default();
    assert!(flight.contains(&one), "{flight}");
}

/// An import to another homeserver writes each song into the node's own folder too, never
/// over another file there: a different one keeps its name and the song gets the next free
/// one (#224 follow-ups, Kimi 3).
#[tokio::test]
async fn an_imports_own_copy_never_replaces_another_file() {
    let hs = Arc::new(FakeHomeserver::default());
    let disk = Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 500 * GB,
        total: 1000 * GB,
    })));
    let (_dir, s, app, src) = copy_setup(hs.clone(), disk, 1).await;
    assert!(storage::homeserver_music_dir(&s).is_none(), "another homeserver");
    std::fs::create_dir_all(&s.mirror_dir).unwrap();
    std::fs::write(s.mirror_dir.join("01_song.wav"), b"another song").unwrap();
    let (_, job) = call(
        &app,
        "POST",
        "/v1/library/imports",
        Some(serde_json::json!({"path": src})),
    )
    .await;
    let done = copy_finished(&app, job["id"].as_str().unwrap()).await;
    assert_eq!(done["done"], 1, "{done}");
    assert_eq!(
        std::fs::read(s.mirror_dir.join("01_song.wav")).unwrap(),
        b"another song"
    );
    assert_eq!(
        std::fs::read(s.mirror_dir.join("01_song (2).wav")).unwrap(),
        std::fs::read(src.join("01 song.wav")).unwrap()
    );
}

/// Songs dropped on the Add music page wait in the node's data folder until whole: that disk
/// keeps its reserve too, counting what the other uploads still have to send, and checked
/// again for each piece (#224 follow-ups, Kimi 6).
#[tokio::test]
async fn browser_uploads_keep_the_reserve_of_the_disk_they_wait_on() {
    let hs = Arc::new(FakeHomeserver::default());
    let disk = Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 5 * GB + 1_000_000,
        total: 98 * GB,
    })));
    let (_dir, _s, app, _src) = copy_setup(hs, disk.clone(), 0).await;
    let start = |name: &str| {
        let app = app.clone();
        let body = serde_json::json!({"name": name, "size": 600_000});
        async move { call(&app, "POST", "/v1/library/uploads", Some(body)).await }
    };
    let (st, a) = start("01 Song.flac").await;
    assert_eq!(st, StatusCode::OK, "{a}");
    let (st, b) = start("02 Song.flac").await;
    assert_eq!(st, StatusCode::INSUFFICIENT_STORAGE, "both wouldn't fit: {b}");

    // The disk fills from elsewhere while the first arrives
    disk.0.lock().unwrap().free = 5 * GB + 10;
    let id = a["id"].as_str().unwrap();
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/v1/library/uploads/{id}?offset=0"))
        .body(Body::from(vec![0u8; 1000]))
        .unwrap();
    let r = app.clone().oneshot(req).await.unwrap();
    assert_eq!(r.status(), StatusCode::INSUFFICIENT_STORAGE);
}

/// Picking two folders copies their songs and nothing else; the dry run first says what it
/// would be, without what is already there (TASK-copy-library-folders).
#[tokio::test]
async fn a_copy_of_two_folders_sends_only_their_songs() {
    let hs = Arc::new(FakeHomeserver::default());
    let (_dir, s, app, src) = copy_setup(hs.clone(), roomy(), 0).await;
    let songs = [
        src.join("A/01 one.wav"),
        src.join("B/02 two.wav"),
        src.join("B/sub/03 three.wav"),
        src.join("C/04 four.wav"),
    ];
    songs_at(&s, &songs);
    // In the library, but no audio any more: left out, its bytes still counted as gone through
    std::fs::write(src.join("A/05 notes.flac"), b"not audio at all").unwrap();
    let mut notes = untagged_track(src.join("A/05 notes.flac"), "Notes", "Band");
    notes.file.size_bytes = 16;
    s.library.lock().unwrap().upsert_track(&notes).unwrap();
    let (there, len) = copied_name(&s, &songs[0]);
    hs.listed.lock().unwrap().push(pm_pubky::homeserver::RemoteFile {
        path: there,
        size: Some(len),
    });
    let pick = |dry: bool| {
        Some(serde_json::json!({
            "paths": [src.join("A"), src.join("B")],
            "dry_run": dry,
        }))
    };
    let (st, plan) = call(&app, "POST", "/v1/pubky/files/upload", pick(true)).await;
    assert_eq!(st, StatusCode::OK, "{plan}");
    assert_eq!(
        (plan["total"].as_u64(), plan["skipped"].as_u64()),
        (Some(4), Some(1)),
        "{plan}"
    );
    assert_eq!(plan["bytes_skipped"].as_u64(), Some(len));
    assert!(hs.uploads.lock().unwrap().is_empty(), "a dry run sends nothing");

    let (st, job) = call(&app, "POST", "/v1/pubky/files/upload", pick(false)).await;
    assert_eq!(st, StatusCode::ACCEPTED, "{job}");
    assert_eq!(
        (job["state"].as_str(), job["label"].as_str()),
        (Some("running"), Some("2 folders"))
    );
    let done = copy_ended(&app, job["id"].as_str().unwrap()).await;
    assert_eq!(
        (
            done["state"].as_str(),
            done["done"].as_u64(),
            done["skipped"].as_u64()
        ),
        (Some("finished"), Some(2), Some(1)),
        "{done}"
    );
    assert_eq!(done["not_audio"], 1, "{done}");
    assert_eq!(
        done["bytes_done"], done["bytes_total"],
        "the bar reaches the end: {done}"
    );
    let sent = hs.uploads.lock().unwrap().clone();
    let mut names: Vec<&str> = sent.iter().map(|u| u.split_once('-').unwrap().1).collect();
    names.sort();
    assert_eq!(names, ["02_two.wav", "03_three.wav"]);
    let (_, jobs) = call(&app, "GET", "/v1/pubky/files/jobs", None).await;
    assert!(jobs["current"].is_null(), "{jobs}");
    assert_eq!(jobs["last"]["id"], job["id"]);
    let (st, _) = call(&app, "DELETE", "/v1/pubky/files/jobs/last", None).await;
    assert_eq!(st, StatusCode::OK);
    let (_, jobs) = call(&app, "GET", "/v1/pubky/files/jobs", None).await;
    assert!(jobs["last"].is_null(), "{jobs}");
}

/// One copy at a time: a second waits in the queue and starts when the first is done. A
/// running copy pauses after the file in hand and carries on with Resume; Cancel stops it
/// within seconds, and a queued one leaves the queue (TASK-copy-library-folders).
#[tokio::test]
async fn copies_wait_their_turn_and_can_be_paused_and_cancelled() {
    let hs = Arc::new(FakeHomeserver {
        delay: Some(std::time::Duration::from_millis(150)),
        ..Default::default()
    });
    let (_dir, s, app, src) = copy_setup(hs.clone(), roomy(), 0).await;
    let songs: Vec<_> = (1..=8).map(|i| src.join(format!("A/{i:02} song.wav"))).collect();
    songs_at(&s, &songs);
    songs_at(&s, &[src.join("B/01 other.wav")]);
    let mut events = s.events.subscribe();
    let start = |folder: &str| {
        let app = app.clone();
        let body = serde_json::json!({"paths": [src.join(folder)]});
        async move { call(&app, "POST", "/v1/pubky/files/upload", Some(body)).await.1 }
    };
    let first = start("A").await;
    let second = start("B").await;
    assert_eq!(
        (first["state"].as_str(), second["state"].as_str()),
        (Some("running"), Some("queued"))
    );
    let (one, two) = (first["id"].as_str().unwrap(), second["id"].as_str().unwrap());
    let (_, jobs) = call(&app, "GET", "/v1/pubky/files/jobs", None).await;
    assert_eq!(
        (jobs["current"]["id"].as_str(), jobs["queued"][0]["id"].as_str()),
        (Some(one), Some(two))
    );
    // An import waits for the copies
    let (st, _) = call(
        &app,
        "POST",
        "/v1/library/imports",
        Some(serde_json::json!({"path": src.join("B")})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);

    let act = |id: &str, what: &str| {
        let app = app.clone();
        let uri = format!("/v1/pubky/files/upload/{id}/{what}");
        async move { call(&app, "POST", &uri, None).await }
    };
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let (st, p) = act(one, "pause").await;
    assert_eq!((st, p["state"].as_str()), (StatusCode::OK, Some("paused")), "{p}");
    // The file in hand finishes first
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let held = hs.uploads.lock().unwrap().len();
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert_eq!(
        hs.uploads.lock().unwrap().len(),
        held,
        "nothing is sent while paused"
    );
    assert!(held < 8);
    let (st, p) = act(one, "resume").await;
    assert_eq!((st, p["state"].as_str()), (StatusCode::OK, Some("running")));

    // The queued one is cancelled: it leaves the queue
    let (st, p) = act(two, "cancel").await;
    assert_eq!((st, p["state"].as_str()), (StatusCode::OK, Some("cancelled")));
    let (_, jobs) = call(&app, "GET", "/v1/pubky/files/jobs", None).await;
    assert_eq!(jobs["queued"].as_array().map(Vec::len), Some(0), "{jobs}");

    // The running one stops within seconds (the old route still cancels)
    let asked = std::time::Instant::now();
    let (st, _) = call(&app, "POST", &format!("/v1/pubky/files/jobs/{one}/cancel"), None).await;
    assert_eq!(st, StatusCode::OK);
    let done = copy_ended(&app, one).await;
    assert!(asked.elapsed() < std::time::Duration::from_secs(2));
    assert_eq!(done["state"], "cancelled", "{done}");
    assert!(done["done"].as_u64().unwrap() < 8, "{done}");
    assert!(!hs
        .uploads
        .lock()
        .unwrap()
        .iter()
        .any(|u| u.ends_with("other.wav")));

    // Each change went out to the owner's pages, and only theirs
    let mut seen = 0;
    while let Ok(ev) = events.try_recv() {
        if matches!(ev.event, ApiEvent::Jobs { .. }) {
            seen += 1;
            let listener = accounts::Caller {
                role: accounts::Role::Listener,
                key: Some("o4dksfbqk85ogzdb5osziw6befigbuxmuxkuxq8434q89uj56uyy".into()),
            };
            let owner = accounts::Caller {
                role: accounts::Role::Owner,
                key: None,
            };
            assert!(events::for_caller(&s, &listener, &ev.event).is_none());
            assert!(events::for_caller(&s, &owner, &ev.event).is_some());
        }
    }
    assert!(seen >= 4, "{seen} jobs events");
}

/// A copy the node was running when it stopped carries on after the restart from where it
/// got, also when the library's list changed meanwhile (a song of it gone: the next one isn't
/// skipped, #237 review); a paused one stays paused until Resume (TASK-copy-library-folders).
#[tokio::test]
async fn a_copy_carries_on_after_a_restart() {
    let hs = Arc::new(FakeHomeserver::default());
    let (_dir, s, app, src) = copy_setup(hs.clone(), roomy(), 0).await;
    let songs: Vec<_> = (0..=3).map(|i| src.join(format!("A/{i:02} song.wav"))).collect();
    songs_at(&s, &songs);
    // 00 was sent before; 01 too, and then left the library (a rescan found it missing)
    {
        let lib = s.library.lock().unwrap();
        let gone = lib
            .list_tracks(None, 100)
            .unwrap()
            .into_iter()
            .find(|t| t.file.path == songs[1]);
        lib.hide_track(gone.unwrap().id.unwrap()).unwrap();
    }
    let after = songs[1].to_string_lossy().into_owned();
    let kept = |state: &str| {
        let progress = types::UploadProgress {
            id: format!("kept-{state}"),
            kind: "copy".into(),
            state: state.into(),
            total: 4,
            done: 2,
            started_at: Some(1),
            ..Default::default()
        };
        serde_json::json!({
            "current": {"progress": progress, "selection": {"paths": [src.join("A")]}, "after": after, "account": "fake-account"},
            "queued": [],
            "last": null,
        })
        .to_string()
    };
    // Stopped while running: the first song was sent before
    s.library
        .lock()
        .unwrap()
        .set_setting(copy_jobs::JOBS_KEY, &kept("running"))
        .unwrap();
    files::resume_jobs(&s);
    let done = copy_ended(&app, "kept-running").await;
    assert_eq!(
        (done["state"].as_str(), done["done"].as_u64()),
        (Some("finished"), Some(4)),
        "{done}"
    );
    let sent = hs.uploads.lock().unwrap().clone();
    let mut names: Vec<&str> = sent.iter().map(|u| u.split_once('-').unwrap().1).collect();
    names.sort();
    assert_eq!(names, ["02_song.wav", "03_song.wav"], "only what was left");

    // Paused before the restart: still paused, nothing sent, until Resume
    hs.uploads.lock().unwrap().clear();
    hs.sizes.lock().unwrap().clear();
    *s.copy_jobs.queue.lock().unwrap() = Default::default();
    s.library
        .lock()
        .unwrap()
        .set_setting(copy_jobs::JOBS_KEY, &kept("paused"))
        .unwrap();
    files::resume_jobs(&s);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(hs.uploads.lock().unwrap().is_empty());
    let (_, jobs) = call(&app, "GET", "/v1/pubky/files/jobs", None).await;
    assert_eq!(jobs["current"]["state"], "paused", "{jobs}");
    let (st, _) = call(&app, "POST", "/v1/pubky/files/upload/kept-paused/resume", None).await;
    assert_eq!(st, StatusCode::OK);
    let done = copy_ended(&app, "kept-paused").await;
    assert_eq!(done["done"].as_u64(), Some(4), "{done}");

    // Cancelled just before the stop: it ends at the start, and the copy queued after it runs
    hs.uploads.lock().unwrap().clear();
    hs.sizes.lock().unwrap().clear();
    *s.copy_jobs.queue.lock().unwrap() = Default::default();
    let mut kept_cancelled: serde_json::Value = serde_json::from_str(&kept("cancelled")).unwrap();
    let mut next = kept_cancelled["current"].clone();
    next["progress"]["id"] = "kept-next".into();
    next["progress"]["state"] = "queued".into();
    next["after"] = serde_json::Value::Null;
    kept_cancelled["queued"] = serde_json::json!([next]);
    s.library
        .lock()
        .unwrap()
        .set_setting(copy_jobs::JOBS_KEY, &kept_cancelled.to_string())
        .unwrap();
    files::resume_jobs(&s);
    let done = copy_ended(&app, "kept-next").await;
    assert_eq!(done["state"], "finished", "{done}");
    let (_, gone) = call(&app, "GET", "/v1/pubky/files/upload/kept-cancelled", None).await;
    assert_eq!(gone["state"], "cancelled", "{gone}");
}

/// Retry after a copy stopped early (failures in a row) sends the failed files and every one
/// it never got to, once; the speed limit for copies stays 2.5 MB/s by default (#237 review).
#[tokio::test]
async fn retry_after_a_stop_sends_what_was_never_tried() {
    let hs = Arc::new(FakeHomeserver::default());
    let (_dir, s, app, src) = copy_setup(hs.clone(), roomy(), 0).await;
    let songs: Vec<_> = (1..=6).map(|i| src.join(format!("A/{i:02} song.wav"))).collect();
    songs_at(&s, &songs);
    hs.fails.store(true, std::sync::atomic::Ordering::SeqCst);
    let (_, job) = call(
        &app,
        "POST",
        "/v1/pubky/files/upload",
        Some(serde_json::json!({"paths": [src.join("A")]})),
    )
    .await;
    let id = job["id"].as_str().unwrap().to_string();
    let done = copy_ended(&app, &id).await;
    assert_eq!(
        (
            done["state"].as_str(),
            done["failed"].as_u64(),
            done["failed_retryable"].as_u64()
        ),
        (Some("failed"), Some(3), Some(6)),
        "{done}"
    );
    hs.fails.store(false, std::sync::atomic::Ordering::SeqCst);
    hs.uploads.lock().unwrap().clear();
    let retry = format!("/v1/pubky/files/upload/{id}/retry");
    let (st, again) = call(&app, "POST", &retry, None).await;
    assert_eq!(
        (st, again["total"].as_u64()),
        (StatusCode::ACCEPTED, Some(6)),
        "{again}"
    );
    let (st, _) = call(&app, "POST", &retry, None).await;
    assert_eq!(st, StatusCode::CONFLICT, "once");
    let done = copy_ended(&app, again["id"].as_str().unwrap()).await;
    assert_eq!(done["done"].as_u64(), Some(6), "{done}");

    let (_, settings) = call(&app, "GET", "/v1/node/settings", None).await;
    assert_eq!(settings["copy_rate"], 2_500_000, "{settings}");
}

/// Retry failed copies again only the files that failed, as a new job (TASK-copy-library-
/// folders); a song whose file can't be read is a failure with its reason, not "not audio"
/// (FOLLOWUP-copy-guard-224, item 2).
#[tokio::test]
async fn retry_sends_the_failed_files_and_unreadable_ones_are_failures() {
    let hs = Arc::new(FakeHomeserver::default());
    let (_dir, s, app, src) = copy_setup(hs.clone(), roomy(), 0).await;
    songs_at(&s, &[src.join("A/01 one.wav"), src.join("A/02 two.wav")]);
    // In the library, but gone from the disk (a share that went away)
    let mut gone = untagged_track(src.join("A/03 gone.wav"), "Gone", "Band");
    gone.file.size_bytes = 1000;
    s.library.lock().unwrap().upsert_track(&gone).unwrap();
    hs.fails.store(true, std::sync::atomic::Ordering::SeqCst);
    let (_, job) = call(
        &app,
        "POST",
        "/v1/pubky/files/upload",
        Some(serde_json::json!({"paths": [src.join("A")]})),
    )
    .await;
    let id = job["id"].as_str().unwrap().to_string();
    let done = copy_ended(&app, &id).await;
    assert_eq!(
        (
            done["failed"].as_u64(),
            done["not_audio"].as_u64(),
            done["failed_retryable"].as_u64()
        ),
        (Some(3), Some(0), Some(3)),
        "{done}"
    );
    assert_eq!(done["bytes_done"], done["bytes_total"], "the bar reaches the end");
    let reasons: Vec<&serde_json::Value> = done["errors"].as_array().unwrap().iter().collect();
    assert!(
        reasons
            .iter()
            .any(|e| e["path"].as_str().unwrap().ends_with("03 gone.wav")
                && e["reason"].as_str().unwrap().contains("can't be read")),
        "{done}"
    );
    assert_eq!(done["errors_count"], 4, "three files and the stop: {done}");

    hs.fails.store(false, std::sync::atomic::Ordering::SeqCst);
    hs.uploads.lock().unwrap().clear();
    let (st, again) = call(&app, "POST", &format!("/v1/pubky/files/upload/{id}/retry"), None).await;
    assert_eq!(st, StatusCode::ACCEPTED, "{again}");
    assert_eq!(again["label"], "Retry: 3 files");
    let done = copy_ended(&app, again["id"].as_str().unwrap()).await;
    assert_eq!(
        (done["done"].as_u64(), done["failed"].as_u64()),
        (Some(2), Some(1)),
        "{done}"
    );
    assert_eq!(hs.uploads.lock().unwrap().len(), 2);
}

/// A copy bigger than what the owner's homeserver quota leaves is refused before it starts,
/// with what it needs and what is free (TASK-copy-library-folders).
#[tokio::test]
async fn a_copy_past_the_homeserver_quota_is_refused() {
    let hs = Arc::new(FakeHomeserver::default());
    let (_dir, s, app, src) = copy_setup(hs.clone(), roomy(), 0).await;
    songs_at(&s, &[src.join("A/01 one.wav")]);
    *s.copy_jobs.quota_seam.lock().unwrap() = Some(1000);
    for dry in [true, false] {
        let (st, v) = call(
            &app,
            "POST",
            "/v1/pubky/files/upload",
            Some(serde_json::json!({"paths": [src.join("A")], "dry_run": dry})),
        )
        .await;
        assert_eq!(st, StatusCode::INSUFFICIENT_STORAGE, "{v}");
        assert_eq!(v["free_bytes"], 1000, "{v}");
        assert!(v["needed_bytes"].as_u64().unwrap() > 1000);
        assert!(v["error"].as_str().unwrap().contains("quota"), "{v}");
    }
    assert!(hs.uploads.lock().unwrap().is_empty());
}

/// The homeserver's file list comes a page at a time, and searched (TASK-copy-library-
/// folders: lists of 10k files).
#[tokio::test]
async fn my_music_on_the_homeserver_comes_in_pages() {
    let hs = Arc::new(FakeHomeserver::default());
    let (_dir, _s, app, _src) = copy_setup(hs.clone(), roomy(), 0).await;
    for i in 0..5 {
        hs.listed.lock().unwrap().push(pm_pubky::homeserver::RemoteFile {
            path: format!("/priv/pubky.music/files/{i:016x}-song_{i}.flac"),
            size: Some(10),
        });
    }
    let get = |q: &str| {
        let app = app.clone();
        let uri = format!("/v1/me/music?{q}");
        async move { call(&app, "GET", &uri, None).await }
    };
    let (st, v) = get("limit=2").await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        (
            v["files"].as_array().unwrap().len(),
            v["count"].as_u64(),
            v["bytes"].as_u64()
        ),
        (2, Some(5), Some(50))
    );
    let (_, v) = get("offset=4&limit=2").await;
    assert_eq!(v["files"].as_array().unwrap().len(), 1);
    let (_, v) = get("q=SONG_3").await;
    assert_eq!(
        (v["count"].as_u64(), v["files"][0]["name"].as_str()),
        (Some(1), Some("song_3.flac")),
        "{v}"
    );
}

/// At start, what a stopped node half wrote on the homeserver is settled before a kept copy
/// carries on: the settling never deletes what the copy has just sent again (#237 review, K3).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_start_settles_half_written_uploads_before_a_copy_carries_on() {
    let hs = Arc::new(FakeHomeserver {
        stat_delay: Some(std::time::Duration::from_millis(300)),
        ..Default::default()
    });
    let (_dir, s, app, src) = copy_setup(hs.clone(), roomy(), 0).await;
    let song = src.join("A/01 song.wav");
    songs_at(&s, std::slice::from_ref(&song));
    let (remote, _) = copied_name(&s, &song);
    // The stopped node was sending it, and the homeserver kept part
    hs.sizes.lock().unwrap().insert(remote.clone(), 3);
    let progress = types::UploadProgress {
        id: "kept".into(),
        kind: "copy".into(),
        state: "running".into(),
        total: 1,
        ..Default::default()
    };
    {
        let lib = s.library.lock().unwrap();
        lib.set_setting(
            copy_guard::IN_FLIGHT_KEY,
            &serde_json::json!({&remote: {"size": 7, "existed": false}}).to_string(),
        )
        .unwrap();
        lib.set_setting(
            copy_jobs::JOBS_KEY,
            &serde_json::json!({"current": {"progress": progress, "selection": {"paths": [src.join("A")]}, "account": "fake-account"}, "queued": []})
                .to_string(),
        )
        .unwrap();
    }
    files::resume_jobs(&s);
    let done = copy_ended(&app, "kept").await;
    assert_eq!(done["done"], 1, "{done}");
    let ops = hs.ops.lock().unwrap().clone();
    assert_eq!(
        ops,
        [format!("del:{remote}"), format!("up:{remote}")],
        "settled first"
    );
}

/// A copy and an import (or bringing files here) are let in under one lock: the check that
/// nothing else runs and the start happen together (#237 review, K2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
// The test holds the door on purpose while the copy's request waits on another worker
#[allow(clippy::await_holding_lock)]
async fn files_jobs_are_let_in_one_at_a_time() {
    /// The fake homeserver with its size checks and uploads held while `hold` is set: an
    /// import let in stays running (its file is checked there first) until the test lets go.
    struct Held {
        hs: Arc<FakeHomeserver>,
        hold: std::sync::atomic::AtomicBool,
    }
    impl Held {
        async fn pass(&self) {
            while self.hold.load(std::sync::atomic::Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        }
        fn set(&self, on: bool) {
            self.hold.store(on, std::sync::atomic::Ordering::SeqCst);
        }
    }
    #[async_trait::async_trait]
    impl copy_guard::Target for Held {
        async fn list(&self) -> Result<Vec<pm_pubky::homeserver::RemoteFile>, String> {
            self.hs.list().await
        }
        async fn upload(
            &self,
            remote: &str,
            bytes: Vec<u8>,
            rate: Option<u64>,
        ) -> Result<(), copy_guard::UploadError> {
            self.pass().await;
            self.hs.upload(remote, bytes, rate).await
        }
        async fn stat(&self, remote: &str) -> Result<Option<Option<u64>>, String> {
            self.pass().await;
            self.hs.stat(remote).await
        }
        async fn delete(&self, remote: &str) -> Result<(), String> {
            self.hs.delete(remote).await
        }
        fn account(&self) -> String {
            self.hs.account()
        }
    }
    let hs = Arc::new(FakeHomeserver {
        delay: Some(std::time::Duration::from_millis(100)),
        ..Default::default()
    });
    let (_dir, s, app, src) = copy_setup(hs.clone(), roomy(), 0).await;
    let held = Arc::new(Held {
        hs,
        hold: false.into(),
    });
    s.copy_seams.lock().unwrap().0 = Some(held.clone());
    songs_at(&s, &[src.join("A/01 one.wav"), src.join("A/02 two.wav")]);
    tone_wav(&src.join("B.wav"), 1);
    std::fs::create_dir_all(src.join("B")).unwrap();
    tone_wav(&src.join("B/01 b.wav"), 1);
    // While another start holds the door, a copy waits instead of checking and starting
    let door = copy_jobs::admit(&s);
    let copy = {
        let app = app.clone();
        let body = serde_json::json!({"paths": [src.join("A")]});
        tokio::spawn(async move { call(&app, "POST", "/v1/pubky/files/upload", Some(body)).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(copy_jobs::busy(&s).is_none(), "the copy went in past the lock");
    drop(door);
    let (st, job) = copy.await.unwrap();
    assert_eq!(st, StatusCode::ACCEPTED, "{job}");
    copy_ended(&app, job["id"].as_str().unwrap()).await;

    let copy_a = || Some(serde_json::json!({"paths": [src.join("A")]}));
    let import_b = || Some(serde_json::json!({"path": src.join("B")}));
    // An import running: a copy asked for then waits in line
    held.set(true);
    let (st, i) = call(&app, "POST", "/v1/library/imports", import_b()).await;
    assert_eq!(st, StatusCode::OK, "{i}");
    let (st, c) = call(&app, "POST", "/v1/pubky/files/upload", copy_a()).await;
    assert_eq!(st, StatusCode::ACCEPTED, "{c}");
    assert_eq!(
        c["state"], "queued",
        "a copy started while an import ran: {c} {i}"
    );
    let iid = i["id"].as_str().unwrap();
    let cid = c["id"].as_str().unwrap();
    call(&app, "POST", &format!("/v1/pubky/files/jobs/{iid}/cancel"), None).await;
    held.set(false);
    copy_finished(&app, iid).await;
    call(
        &app,
        "POST",
        &format!("/v1/pubky/files/upload/{cid}/cancel"),
        None,
    )
    .await;
    copy_ended(&app, cid).await;
    // A copy running: an import asked for then is refused
    held.set(true);
    let (st, c) = call(&app, "POST", "/v1/pubky/files/upload", copy_a()).await;
    assert_eq!(c["state"], "running", "{st} {c}");
    let (st, i) = call(&app, "POST", "/v1/library/imports", import_b()).await;
    assert_eq!(
        st,
        StatusCode::CONFLICT,
        "an import started while a copy ran: {i} {c}"
    );
    let cid = c["id"].as_str().unwrap();
    call(
        &app,
        "POST",
        &format!("/v1/pubky/files/upload/{cid}/cancel"),
        None,
    )
    .await;
    held.set(false);
    copy_ended(&app, cid).await;

    // Started side by side, many times: never both running. The homeserver is held until
    // both have answered, so an import let in is still running when the copy is let in.
    for _ in 0..10 {
        held.set(true);
        let (c, i) = tokio::join!(
            call(
                &app,
                "POST",
                "/v1/pubky/files/upload",
                Some(serde_json::json!({"paths": [src.join("A")]}))
            ),
            call(
                &app,
                "POST",
                "/v1/library/imports",
                Some(serde_json::json!({"path": src.join("B")}))
            ),
        );
        let import_in = i.0 == StatusCode::OK;
        if import_in {
            let id = i.1["id"].as_str().unwrap();
            let (_, now) = call(&app, "GET", &format!("/v1/pubky/files/upload/{id}"), None).await;
            assert_eq!(now["finished"], false, "the import ended before the copy: {now}");
        }
        assert!(
            !(import_in && c.1["state"] == "running"),
            "both started: {c:?} {i:?}"
        );
        held.set(false);
        if import_in {
            call(
                &app,
                "POST",
                &format!("/v1/pubky/files/jobs/{}/cancel", i.1["id"].as_str().unwrap()),
                None,
            )
            .await;
            copy_finished(&app, i.1["id"].as_str().unwrap()).await;
        }
        call(
            &app,
            "POST",
            &format!("/v1/pubky/files/upload/{}/cancel", c.1["id"].as_str().unwrap()),
            None,
        )
        .await;
        copy_ended(&app, c.1["id"].as_str().unwrap()).await;
    }
}

/// A backup taken while a copy ran, restored later on an idle node, never starts that copy
/// again at the next start: the files jobs a node keeps (copies, bringing files here, uploads
/// it left half written) are its own, as its stream key is; the node's own stay (#237 review,
/// M2).
#[tokio::test]
async fn a_restored_backup_never_starts_an_old_copy() {
    let hs = Arc::new(FakeHomeserver::default());
    let (_dir, s, app, src) = copy_setup(hs.clone(), roomy(), 0).await;
    songs_at(&s, &[src.join("A/01 one.wav"), src.join("A/02 two.wav")]);
    let keys = [
        copy_jobs::JOBS_KEY,
        files::BRING_RESUME_KEY,
        copy_guard::IN_FLIGHT_KEY,
    ];
    let set = |k: &str, v: String| s.library.lock().unwrap().set_setting(k, &v).unwrap();
    set(copy_jobs::JOBS_KEY, kept_copy("old", &src, Some("fake-account")));
    set(
        files::BRING_RESUME_KEY,
        serde_json::json!({"dest": src.join("brought")}).to_string(),
    );
    set(
        copy_guard::IN_FLIGHT_KEY,
        serde_json::json!({"/pub/old.wav": {"size": 7, "existed": false}}).to_string(),
    );
    let (taken, _) = backup::make(&s, None).unwrap();
    let open = backup::parse(&taken).unwrap();
    let has = |hay: &[u8], needle: &str| hay.windows(needle.len()).any(|w| w == needle.as_bytes());
    for k in keys {
        assert!(!has(&open.database, k), "{k} in the backup");
    }
    // Since then that copy ended and the node is idle
    for k in keys {
        s.library.lock().unwrap().delete_setting(k).unwrap();
    }
    backup::restore(&s, taken.clone(), None, false).await.unwrap();
    for k in keys {
        assert_eq!(
            s.library.lock().unwrap().get_setting(k).unwrap(),
            None,
            "{k} came back"
        );
    }
    files::resume_jobs(&s);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(hs.uploads.lock().unwrap().is_empty(), "the old copy ran");
    let (_, jobs) = call(&app, "GET", "/v1/pubky/files/jobs", None).await;
    assert!(jobs["current"].is_null(), "{jobs}");

    // A copy this node keeps itself stays through a restore
    let own = kept_copy("own", &src, Some("fake-account"));
    set(copy_jobs::JOBS_KEY, own.clone());
    backup::restore(&s, taken, None, false).await.unwrap();
    assert_eq!(
        s.library
            .lock()
            .unwrap()
            .get_setting(copy_jobs::JOBS_KEY)
            .unwrap(),
        Some(own)
    );
}

/// A copy sends only into the Pubky account it was started for: kept over a restart or
/// paused while someone else signed in, it stops before sending anything, also when the
/// sign-in changes while it runs, and Retry waits for that account; a copy kept by a node
/// that didn't note its account doesn't run (#237 review, L6).
#[tokio::test]
async fn a_copy_never_sends_into_another_account() {
    let hs = Arc::new(FakeHomeserver {
        delay: Some(std::time::Duration::from_millis(150)),
        ..Default::default()
    });
    let (_dir, s, app, src) = copy_setup(hs.clone(), roomy(), 0).await;
    let songs: Vec<_> = (1..=3).map(|i| src.join(format!("A/{i:02} song.wav"))).collect();
    songs_at(&s, &songs);
    let sign_in_as = |who: Option<&str>| *hs.who.lock().unwrap() = who.map(str::to_string);
    let sent = || hs.uploads.lock().unwrap().len();
    let refused = |done: &serde_json::Value| {
        done["state"] == "failed"
            && done["error"]
                .as_str()
                .is_some_and(|e| e.contains("another Pubky account"))
    };

    // Kept over a restart; the node came back signed in as someone else
    s.library
        .lock()
        .unwrap()
        .set_setting(
            copy_jobs::JOBS_KEY,
            &kept_copy("kept", &src, Some("fake-account")),
        )
        .unwrap();
    sign_in_as(Some("someone-else"));
    files::resume_jobs(&s);
    let done = copy_ended(&app, "kept").await;
    assert!(refused(&done), "{done}");
    assert_eq!(sent(), 0);
    let retry = "/v1/pubky/files/upload/kept/retry";
    let (st, e) = call(&app, "POST", retry, None).await;
    assert_eq!(st, StatusCode::CONFLICT, "{e}");
    assert_eq!(sent(), 0);
    // Back as the owner: Retry sends it
    sign_in_as(None);
    let (st, again) = call(&app, "POST", retry, None).await;
    assert_eq!(st, StatusCode::ACCEPTED, "{again}");
    let done = copy_ended(&app, again["id"].as_str().unwrap()).await;
    assert_eq!(done["done"], 3, "{done}");

    // Paused, then someone else signs in and it is resumed
    hs.uploads.lock().unwrap().clear();
    hs.sizes.lock().unwrap().clear();
    let start = || {
        call(
            &app,
            "POST",
            "/v1/pubky/files/upload",
            Some(serde_json::json!({"paths": [src.join("A")]})),
        )
    };
    let (st, job) = start().await;
    assert_eq!(st, StatusCode::ACCEPTED, "{job}");
    let id = job["id"].as_str().unwrap().to_string();
    let (st, _) = call(&app, "POST", &format!("/v1/pubky/files/upload/{id}/pause"), None).await;
    assert_eq!(st, StatusCode::OK);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    let before = sent();
    assert!(before < 3, "it paused");
    sign_in_as(Some("someone-else"));
    let (st, _) = call(&app, "POST", &format!("/v1/pubky/files/upload/{id}/resume"), None).await;
    assert_eq!(st, StatusCode::OK);
    let done = copy_ended(&app, &id).await;
    assert!(refused(&done), "{done}");
    assert_eq!(sent(), before, "nothing went to someone else");

    // Someone else signs in while it runs: it stops before the next file
    sign_in_as(None);
    hs.uploads.lock().unwrap().clear();
    hs.sizes.lock().unwrap().clear();
    let (_, job) = start().await;
    let id = job["id"].as_str().unwrap().to_string();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    sign_in_as(Some("someone-else"));
    let done = copy_ended(&app, &id).await;
    assert!(refused(&done), "{done}");
    assert!(sent() < 3, "{done}");

    // Kept by a node that didn't note the account: it doesn't run
    sign_in_as(None);
    hs.uploads.lock().unwrap().clear();
    hs.sizes.lock().unwrap().clear();
    *s.copy_jobs.queue.lock().unwrap() = Default::default();
    s.library
        .lock()
        .unwrap()
        .set_setting(copy_jobs::JOBS_KEY, &kept_copy("unnoted", &src, None))
        .unwrap();
    files::resume_jobs(&s);
    let done = copy_ended(&app, "unnoted").await;
    assert_eq!(done["state"], "failed", "{done}");
    assert_eq!(sent(), 0);
}

/// While the start settles what a stopped node left, no import or bring job is let in (it
/// would race the settling, and keep a bring the node had from resuming); only the bring job
/// the node resumes itself goes in. Once settled, imports come in again (#237 review, Kimi M2).
#[tokio::test]
async fn imports_wait_while_the_start_settles() {
    let hs = Arc::new(FakeHomeserver {
        stat_delay: Some(std::time::Duration::from_millis(1500)),
        ..Default::default()
    });
    let (_dir, s, app, src) = copy_setup(hs.clone(), roomy(), 0).await;
    std::fs::create_dir_all(src.join("B")).unwrap();
    tone_wav(&src.join("B/01 b.wav"), 1);
    // A stopped node left an upload to settle; checking it takes a while
    s.library
        .lock()
        .unwrap()
        .set_setting(
            copy_guard::IN_FLIGHT_KEY,
            &serde_json::json!({"/pub/left.wav": {"size": 7, "existed": false}}).to_string(),
        )
        .unwrap();
    files::resume_jobs(&s);
    let body = Some(serde_json::json!({"path": src.join("B")}));
    let (st, e) = call(&app, "POST", "/v1/library/imports", body.clone()).await;
    assert_eq!(st, StatusCode::CONFLICT, "{e}");
    assert!(e["error"].as_str().unwrap().contains("settling"), "{e}");
    let job = types::UploadProgress {
        id: "resumed".into(),
        kind: "bring".into(),
        total: 1,
        ..Default::default()
    };
    assert!(files::claim_job(&s, &job).is_err(), "a bring job asked for");
    files::claim_job_as(&s, &job, true).expect("the bring job the node resumes");
    s.upload_jobs.lock().unwrap().remove("resumed");
    for _ in 0..100 {
        if !copy_jobs::held(&s) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(!copy_jobs::held(&s), "settled");
    let (st, e) = call(&app, "POST", "/v1/library/imports", body).await;
    assert_eq!(st, StatusCode::OK, "{e}");
}

/// Retry asked twice at once makes one job; a copy's summary always says how many files a
/// retry would send, `0` too (left out, the page fell back to `failed` and kept offering
/// Retry failed) (#237 review).
#[tokio::test]
async fn retry_asked_twice_at_once_makes_one_job() {
    let hs = Arc::new(FakeHomeserver {
        delay: Some(std::time::Duration::from_millis(100)),
        ..Default::default()
    });
    let (_dir, s, app, src) = copy_setup(hs.clone(), roomy(), 0).await;
    let songs: Vec<_> = (1..=4).map(|i| src.join(format!("A/{i:02} song.wav"))).collect();
    songs_at(&s, &songs);
    hs.fails.store(true, std::sync::atomic::Ordering::SeqCst);
    let (_, job) = call(
        &app,
        "POST",
        "/v1/pubky/files/upload",
        Some(serde_json::json!({"paths": [src.join("A")]})),
    )
    .await;
    let id = job["id"].as_str().unwrap().to_string();
    let done = copy_ended(&app, &id).await;
    assert_eq!(done["state"], "failed", "{done}");
    hs.fails.store(false, std::sync::atomic::Ordering::SeqCst);
    let retry = format!("/v1/pubky/files/upload/{id}/retry");
    let (a, b) = tokio::join!(call(&app, "POST", &retry, None), call(&app, "POST", &retry, None));
    let mut st = [a.0, b.0];
    st.sort();
    assert_eq!(st, [StatusCode::ACCEPTED, StatusCode::CONFLICT], "{a:?} {b:?}");
    let (_, jobs) = call(&app, "GET", "/v1/pubky/files/jobs", None).await;
    assert_eq!(jobs["last"]["id"], id.as_str(), "{jobs}");
    assert_eq!(jobs["last"]["failed_retryable"], 0, "{jobs}");
    assert!(jobs["queued"].as_array().unwrap().is_empty(), "{jobs}");
    let again = if a.0 == StatusCode::ACCEPTED { a.1 } else { b.1 };
    let done = copy_ended(&app, again["id"].as_str().unwrap()).await;
    assert_eq!(
        (done["done"].as_u64(), done["failed_retryable"].as_u64()),
        (Some(4), Some(0)),
        "{done}"
    );
    assert_eq!(
        hs.uploads.lock().unwrap().len(),
        3 + 4,
        "three failed, then four sent"
    );
}

/// Two uploads started at once can't both take the same room on the disk they wait on: the
/// room is counted, checked and the upload added in one step.
#[tokio::test]
async fn two_uploads_started_at_once_cant_both_take_the_same_room() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    // 300 MB usable: one 200 MB upload fits, two don't
    let disk = Arc::new(FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 5 * GB + 300 * 1024 * 1024,
        total: 100 * GB,
    })));
    let hs = Arc::new(FakeHomeserver::default());
    *s.copy_seams.lock().unwrap() = (Some(hs), Some(disk));
    let upload = |name: &str| add_music::NewUpload {
        name: name.into(),
        size: 200 * 1024 * 1024,
        relative_path: None,
        blake3: None,
    };
    let (a, b) = tokio::join!(
        add_music::start(State(s.clone()), Json(upload("a.flac"))),
        add_music::start(State(s.clone()), Json(upload("b.flac")))
    );
    let statuses: Vec<Option<StatusCode>> = [a, b]
        .iter()
        .map(|r| r.as_ref().err().map(|e| e.status))
        .collect();
    assert_eq!(
        statuses.iter().filter(|st| st.is_none()).count(),
        1,
        "one start goes through: {statuses:?}"
    );
    assert!(
        statuses.contains(&Some(StatusCode::INSUFFICIENT_STORAGE)),
        "the other is told there is no room: {statuses:?}"
    );
}

/// Two different songs for one path in the mirror folder, at once, both land whole under two
/// names; the same song twice lands once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_songs_for_one_mirror_path_both_land() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("song.flac");
    let (one, two) = (vec![1u8; 4096], vec![2u8; 4096]);
    let hash = |b: &[u8]| blake3::hash(b).to_hex().to_string();
    let (h1, h2) = (hash(&one), hash(&two));
    let (a, b) = tokio::join!(
        add_music::place_in_mirror(&dest, &h1, one.clone()),
        add_music::place_in_mirror(&dest, &h2, two.clone())
    );
    let (a, b) = (a.unwrap().unwrap(), b.unwrap().unwrap());
    assert_ne!(a, b, "two names");
    let mut names: Vec<String> = [&a, &b]
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["song (2).flac", "song.flac"]);
    let mut contents = vec![std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap()];
    contents.sort();
    assert_eq!(contents, [one.clone(), two]);
    // The same song again: kept as it is, nothing new
    assert_eq!(
        add_music::place_in_mirror(&dest, &hash(&one), one).await.unwrap(),
        None
    );
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        2,
        "no part file left"
    );
}

/// An upload whose in-flight record can't be written isn't sent: a crash would leave a
/// remote file nobody settles at the next start.
#[tokio::test]
async fn an_upload_whose_record_cant_be_written_is_not_sent() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let hs = Arc::new(FakeHomeserver::default());
    let disk = FakeDisk(std::sync::Mutex::new(copy_guard::Space {
        free: 50 * GB,
        total: 100 * GB,
    }));
    let db = rusqlite::Connection::open(dir.path().join("lib.sqlite")).unwrap();
    db.execute_batch(&format!(
        "CREATE TRIGGER no_record BEFORE INSERT ON settings WHEN NEW.key = '{k}'
             BEGIN SELECT RAISE(ABORT, 'disk I/O error'); END;",
        k = copy_guard::IN_FLIGHT_KEY
    ))
    .unwrap();
    let r =
        copy_guard::upload_guarded(&s, hs.as_ref(), &disk, "/pub/files/x.flac", vec![1u8; 10], None).await;
    assert!(
        matches!(r, Err(copy_guard::Refused::Other(ref why)) if why.contains("record")),
        "{r:?}"
    );
    assert_eq!(hs.uploads.lock().unwrap().len(), 0, "nothing was sent");
}

/// The mirror folder's locks go when their last holder is done, and two files for names in one
/// folder at once land under two names.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mirror_folder_locks_are_dropped_and_names_stay_distinct() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Album");
    std::fs::create_dir_all(&folder).unwrap();
    // x.flac is taken by another song: a file for x.flac goes to x (2).flac, the name the
    // other one asks for directly
    std::fs::write(folder.join("x.flac"), [9u8; 64]).unwrap();
    let (one, two) = (vec![1u8; 4096], vec![2u8; 4096]);
    let hash = |b: &[u8]| blake3::hash(b).to_hex().to_string();
    let (h1, h2) = (hash(&one), hash(&two));
    let (p1, p2) = (folder.join("x.flac"), folder.join("x (2).flac"));
    let (a, b) = tokio::join!(
        add_music::place_in_mirror(&p1, &h1, one.clone()),
        add_music::place_in_mirror(&p2, &h2, two.clone())
    );
    let (a, b) = (a.unwrap().unwrap(), b.unwrap().unwrap());
    assert_ne!(a, b, "two names");
    assert_eq!(std::fs::read(&a).unwrap(), one);
    assert_eq!(std::fs::read(&b).unwrap(), two);
    assert!(
        !add_music::mirror_lock_held(&folder),
        "this folder's lock went with its holders"
    );
}

/// A file that was not on the homeserver when a copy listed it, but is there whole by the
/// time the copy reaches it (another node of the owner, an upload), is not sent again: an
/// upload that failed over it would be removed as half written, and the good copy with it.
#[tokio::test]
async fn a_copy_checks_a_file_written_since_its_listing_before_sending() {
    let hs = Arc::new(FakeHomeserver::default());
    let (_dir, s, _app, _src) = copy_setup(hs.clone(), roomy(), 0).await;
    let disk = roomy();
    let remote = "/pub/pubky.app/files/music/abcd-song.flac";
    hs.sizes.lock().unwrap().insert(remote.into(), 5);
    let mut present = std::collections::HashMap::new();
    let sent = files::send_one(&s, &*hs, &*disk, &mut present, remote, b"12345".to_vec()).await;
    assert!(matches!(sent, Ok(false)), "the good copy there counts");
    assert!(hs.uploads.lock().unwrap().is_empty(), "nothing sent over it");
    assert_eq!(present.get(remote), Some(&Some(5)), "noted as there");
    // Not there: sent
    let other = "/pub/pubky.app/files/music/ef01-other.flac";
    let sent = files::send_one(&s, &*hs, &*disk, &mut present, other, b"123".to_vec()).await;
    assert!(matches!(sent, Ok(true)));
    assert_eq!(*hs.uploads.lock().unwrap(), [other.to_string()]);
}

/// A file the copy's listing did not have, whose check fails when the copy reaches it, is
/// not sent: a good copy may have been written there since, and a failed upload's clean-up
/// would remove it.
#[tokio::test]
async fn a_copy_does_not_send_a_file_it_cannot_check() {
    let hs = Arc::new(FakeHomeserver::default());
    let (_dir, s, _app, _src) = copy_setup(hs.clone(), roomy(), 0).await;
    let disk = roomy();
    let remote = "/pub/pubky.app/files/music/abcd-song.flac";
    hs.sizes.lock().unwrap().insert(remote.into(), 5);
    hs.stat_fails.store(true, std::sync::atomic::Ordering::SeqCst);
    hs.fails.store(true, std::sync::atomic::Ordering::SeqCst);
    let mut present = std::collections::HashMap::new();
    let sent = files::send_one(&s, &*hs, &*disk, &mut present, remote, b"12345".to_vec()).await;
    assert!(
        matches!(&sent, Err(copy_guard::Refused::Other(e)) if e.starts_with("couldn't check the copy there")),
        "{sent:?}"
    );
    assert!(hs.uploads.lock().unwrap().is_empty(), "nothing sent");
    assert!(hs.deletes.lock().unwrap().is_empty(), "nothing removed");
    assert_eq!(
        hs.sizes.lock().unwrap().get(remote),
        Some(&5),
        "the copy there stays whole"
    );
    assert!(present.is_empty());
}

/// An import (a folder, or files from a browser) checks a file its listing did not have
/// right before it sends it, as a copy does: a whole copy written there since is kept as it
/// is, and when the check fails nothing is sent (a failed upload's clean-up would remove a
/// good copy written since).
#[tokio::test]
async fn an_import_checks_a_file_its_listing_did_not_have() {
    use std::sync::atomic::Ordering::SeqCst;
    let hs = Arc::new(FakeHomeserver::default());
    let (_dir, s, _app, _src) = copy_setup(hs.clone(), roomy(), 0).await;
    let disk = roomy();
    let bytes = b"12345".to_vec();
    let h16 = blake3::hash(&bytes).to_hex()[..16].to_string();
    let remote = format!("{}{h16}-song.flac", pm_pubky::homeserver::PRIVATE_FILES_DIR);
    let said = |r: &Result<add_music::Stored, String>| match r {
        Ok(add_music::Stored::Sent(p)) => format!("sent {p}"),
        Ok(add_music::Stored::Already(p)) => format!("already {p}"),
        Err(e) => format!("refused: {e}"),
    };
    // Written there since the listing: kept, nothing sent over it
    hs.sizes.lock().unwrap().insert(remote.clone(), 5);
    let mut have = std::collections::HashMap::new();
    let stored = add_music::store(
        &s,
        &*hs,
        &*disk,
        &mut have,
        "song.flac",
        None,
        bytes.clone(),
        None,
    )
    .await;
    assert_eq!(said(&stored), format!("already {remote}"));
    assert!(hs.uploads.lock().unwrap().is_empty(), "nothing sent over it");
    assert_eq!(have.get(&h16), Some(&Some(5)), "noted as there");
    // The check fails: refused, nothing sent, the copy there stays
    hs.stat_fails.store(true, SeqCst);
    hs.fails.store(true, SeqCst);
    let mut have = std::collections::HashMap::new();
    let stored = add_music::store(
        &s,
        &*hs,
        &*disk,
        &mut have,
        "song.flac",
        None,
        bytes.clone(),
        None,
    )
    .await;
    assert!(
        said(&stored).starts_with("refused: song.flac: couldn't check the copy there"),
        "{}",
        said(&stored)
    );
    assert!(hs.uploads.lock().unwrap().is_empty(), "nothing sent");
    assert!(hs.deletes.lock().unwrap().is_empty(), "nothing removed");
    assert_eq!(
        hs.sizes.lock().unwrap().get(&remote),
        Some(&5),
        "the copy there stays whole"
    );
    // Not there: sent
    hs.stat_fails.store(false, SeqCst);
    hs.fails.store(false, SeqCst);
    hs.sizes.lock().unwrap().clear();
    let stored = add_music::store(&s, &*hs, &*disk, &mut have, "song.flac", None, bytes, None).await;
    assert_eq!(said(&stored), format!("sent {remote}"));
    assert_eq!(*hs.uploads.lock().unwrap(), [remote]);
}

/// A bring that was cancelled is not started again after a restart; the part folder its
/// record names is cleared and the record goes. A part folder of another name stays.
#[tokio::test]
async fn a_cancelled_bring_is_not_started_again_and_its_parts_go() {
    let hs = Arc::new(FakeHomeserver::default());
    let (_dir, s, app, src) = copy_setup(hs.clone(), roomy(), 0).await;
    let dest = src.join("brought");
    for name in [".odeion-bring-old", ".odeion-bring-other"] {
        std::fs::create_dir_all(dest.join(name)).unwrap();
        std::fs::write(dest.join(name).join("1.part"), b"x").unwrap();
    }
    s.library
        .lock()
        .unwrap()
        .set_setting(
            files::BRING_RESUME_KEY,
            &serde_json::json!({"dest": dest, "parts": ".odeion-bring-old", "cancelled": true}).to_string(),
        )
        .unwrap();
    files::resume_jobs(&s);
    let mut gone = false;
    for _ in 0..100 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if s.library
            .lock()
            .unwrap()
            .get_setting(files::BRING_RESUME_KEY)
            .unwrap()
            .is_none()
        {
            gone = true;
            break;
        }
    }
    assert!(gone, "the record of the cancelled bring went");
    assert!(
        !dest.join(".odeion-bring-old").exists(),
        "its part folder was cleared"
    );
    assert!(
        dest.join(".odeion-bring-other/1.part").exists(),
        "another folder stays"
    );
    let (_, jobs) = call(&app, "GET", "/v1/pubky/files/jobs", None).await;
    assert!(jobs["current"].is_null(), "not started again: {jobs}");
}
