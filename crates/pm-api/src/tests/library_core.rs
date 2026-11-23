//! The library itself: folders, the folder tree, files, covers and lyrics.
//!
//! Part of pm-api's unit tests (`mod tests` in lib.rs, which holds the helpers these use). A new
//! test of this area goes at the end of this file.

use super::*;

#[tokio::test]
async fn folders_can_be_added_and_removed_through_the_api() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("music")).unwrap();
    let extra = dir.path().join("more");
    std::fs::create_dir(&extra).unwrap();
    write_wav(&extra.join("extra.wav"), 4410);
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (st, _) = call(
        &app,
        "POST",
        "/v1/library/folders",
        Some(serde_json::json!({"path": "/nope/missing"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, r) = call(
        &app,
        "POST",
        "/v1/library/folders",
        Some(serde_json::json!({"path": extra.to_string_lossy()})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["scan_started"], true);
    assert!(r["folders"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f[0] == extra.to_string_lossy().as_ref()));
    for _ in 0..500 {
        if !s.is_scanning() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let (_, tracks) = call(&app, "GET", "/v1/library/tracks?limit=10", None).await;
    assert_eq!(tracks["tracks"].as_array().unwrap().len(), 1);
    let (st, r) = call(
        &app,
        "DELETE",
        &format!("/v1/library/folders?path={}", extra.to_string_lossy()),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["removed_tracks"], 1);
    // Only the config's folder is left
    let music = dir.path().join("music");
    assert_eq!(r["folders"], serde_json::json!([[music.to_string_lossy(), true]]));
    let (_, tracks) = call(&app, "GET", "/v1/library/tracks?limit=10", None).await;
    assert!(tracks["tracks"].as_array().unwrap().is_empty());
}

/// Only one of the library's folders can be forgotten: songs go by path, with their plays,
/// so a path that is not a folder of the library (none, `/`, one above or inside a folder)
/// is refused and nothing changes.
#[tokio::test]
async fn only_one_of_the_librarys_folders_can_be_removed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("music")).unwrap();
    let extra = dir.path().join("extra");
    std::fs::create_dir_all(extra.join("album")).unwrap();
    write_wav(&extra.join("album/one.wav"), 4410);
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (st, r) = call(
        &app,
        "POST",
        "/v1/library/folders",
        Some(serde_json::json!({"path": extra.to_string_lossy()})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    for _ in 0..500 {
        if !s.is_scanning() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    // The folder as the node names it
    let added = r["folders"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f[0].as_str())
        .find(|p| p.ends_with("extra"))
        .unwrap_or_else(|| panic!("no added folder: {r}"))
        .to_string();
    let (_, tracks) = call(&app, "GET", "/v1/library/tracks?limit=10", None).await;
    let id = tracks["tracks"][0]["id"].as_i64().unwrap();
    s.library
        .lock()
        .unwrap()
        .record_play(pm_model::TrackId(id), None)
        .unwrap();
    let kept = |s: &Arc<NodeState>| {
        let lib = s.library.lock().unwrap();
        (lib.stats().unwrap().tracks, lib.history(10).unwrap().len())
    };
    let above = std::path::Path::new(&added)
        .parent()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    for (path, want) in [
        ("".to_string(), StatusCode::BAD_REQUEST),
        ("%20".to_string(), StatusCode::BAD_REQUEST),
        ("//".to_string(), StatusCode::NOT_FOUND),
        ("/".to_string(), StatusCode::NOT_FOUND),
        (above, StatusCode::NOT_FOUND),
        (format!("{added}/album"), StatusCode::NOT_FOUND),
        ("/nope".to_string(), StatusCode::NOT_FOUND),
    ] {
        let (st, r) = call(&app, "DELETE", &format!("/v1/library/folders?path={path}"), None).await;
        assert_eq!(st, want, "{path:?}: {r}");
        assert_eq!(kept(&s), (1, 1), "after {path:?}");
    }
    // The folder itself, also written with a slash at the end
    let (st, r) = call(
        &app,
        "DELETE",
        &format!("/v1/library/folders?path={added}/"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["removed_tracks"], 1);
    assert!(!r["folders"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f[0] == added.as_str()));
    assert_eq!(kept(&s).0, 0);

    // A folder stored with a slash at the end (as a config may write it) goes whichever way
    // it is asked for, and is no longer listed: it would be scanned back in otherwise
    for asked in ["", "/"] {
        let stored = format!("{}/", extra.to_string_lossy());
        s.library
            .lock()
            .unwrap()
            .add_folder(std::path::Path::new(&stored))
            .unwrap();
        let listed = |s: &Arc<NodeState>| {
            s.library
                .lock()
                .unwrap()
                .list_folders()
                .unwrap()
                .iter()
                .any(|(p, _)| p.to_string_lossy() == stored.as_str())
        };
        assert!(listed(&s));
        let path = format!("{}{asked}", extra.to_string_lossy());
        let (st, r) = call(&app, "DELETE", &format!("/v1/library/folders?path={path}"), None).await;
        assert_eq!(st, StatusCode::OK, "{path}: {r}");
        assert!(!listed(&s), "{path}: still listed");
    }
}

#[tokio::test]
async fn tunnel_settings_go_into_the_shared_folder_and_out_again() {
    use base64::Engine;
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let tun = dir.path().join("tunnel");
    std::fs::create_dir_all(&tun).unwrap();
    s.set_tunnel_dir(tun.clone());
    let app = router(s.clone());
    let tok = base64::engine::general_purpose::STANDARD.encode(
        r#"{"a":"0123456789abcdef","t":"6ff42ae2-765d-4adf-8112-31c55c1551ef","s":"c2VjcmV0c2VjcmV0"}"#,
    );
    let (st, _) = call(&app, "PUT", "/v1/remote/tunnel", Some(serde_json::json!({"token": tok, "homeserver_domain": "hs.example.org", "music_hostname": "hs.example.org"}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "two different hostnames");
    let (st, _) = call(
        &app,
        "PUT",
        "/v1/remote/tunnel",
        Some(serde_json::json!({"token": "abc", "homeserver_domain": "hs.example.org"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "not a tunnel token");
    let (st, v) = call(&app, "PUT", "/v1/remote/tunnel", Some(serde_json::json!({"token": tok, "homeserver_domain": "HS.example.org", "music_hostname": "music.example.org"}))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        (
            v["configured"].as_bool(),
            v["homeserver_domain"].as_str(),
            v["music_hostname"].as_str()
        ),
        (Some(true), Some("hs.example.org"), Some("music.example.org"))
    );
    assert!(!v.to_string().contains(&tok), "never the token");
    assert_eq!(std::fs::read_to_string(tun.join("token")).unwrap().trim(), tok);
    assert_eq!(
        std::fs::read_to_string(tun.join("domain")).unwrap().trim(),
        "hs.example.org"
    );
    let (_, v) = call(&app, "DELETE", "/v1/remote/tunnel", None).await;
    assert_eq!(v["configured"], false);
    assert!(!tun.join("token").exists() && !tun.join("domain").exists());
    // Without a tunnel folder the install has no tunnel setup
    let other = tempfile::tempdir().unwrap();
    let s2 = state(other.path(), None).await;
    let (_, v) = call(&router(s2), "GET", "/v1/remote/tunnel", None).await;
    assert_eq!(v["available"], false);
}

#[tokio::test]
async fn lyrics_come_from_a_sidecar_lrc() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("music")).unwrap();
    write_wav(&dir.path().join("music/song.wav"), 4410);
    std::fs::write(
        dir.path().join("music/song.lrc"),
        "[ti:Song]\n[00:01.00]Hello\n[00:02.50]World\n",
    )
    .unwrap();
    write_wav(&dir.path().join("music/mute.wav"), 4410);
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
    let id_of = |name: &str| {
        tracks["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["file"]["path"].as_str().unwrap().ends_with(name))
            .map(|t| t["id"].as_i64().unwrap())
            .unwrap()
    };
    let (st, l) = call(
        &app,
        "GET",
        &format!("/v1/library/tracks/{}/lyrics", id_of("song.wav")),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{l}");
    assert_eq!(l["source"], "lrc");
    assert_eq!(l["synced"], true);
    assert_eq!(l["lines"].as_array().unwrap().len(), 2);
    assert_eq!(l["lines"][1]["ms"], 2500);
    assert_eq!(l["lines"][1]["text"], "World");
    let (st, _) = call(
        &app,
        "GET",
        &format!("/v1/library/tracks/{}/lyrics", id_of("mute.wav")),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

/// A body that doesn't fit answers in plain words, with the detail apart.
#[tokio::test]
async fn bad_bodies_answer_plainly() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (st, e) = call(
        &app,
        "POST",
        "/v1/playlists",
        Some(serde_json::json!({"uris": "not a list"})),
    )
    .await;
    assert!(st.is_client_error(), "{st}");
    assert_eq!(
        e["error"], "The node didn't understand that request. Reload the page and try again.",
        "{e}"
    );
    assert!(e["detail"].as_str().unwrap().contains("uris"), "{e}");
}

/// Lyrics from LRCLIB for a streamed song and for a library song without its own; a song
/// LRCLIB doesn't know answers 404 and is asked about once.
#[tokio::test]
async fn lyrics_come_from_lrclib_when_the_files_have_none() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let a2 = asked.clone();
    let mock = Router::new().route(
        "/api/get",
        get(move |Query(q): Query<std::collections::HashMap<String, String>>| {
            a2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move {
                if q.get("track_name").map(String::as_str) == Some("Known Song") {
                    (StatusCode::OK, Json(serde_json::json!({"syncedLyrics": "[00:01.00]First\n[00:03.50]Second", "plainLyrics": "First\nSecond"})))
                } else {
                    (StatusCode::NOT_FOUND, Json(serde_json::json!({"message": "not found"})))
                }
            }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    crate::lrclib::set_base(&format!("http://127.0.0.1:{port}"));
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), None).await;
    s.set_contact("test@example.org").unwrap();
    let tid = {
        let lib = s.library.lock().unwrap();
        lib.upsert_track(&untagged_track(
            dir.path().join("k.flac"),
            "Known Song",
            "Someone",
        ))
        .unwrap()
        .0
    };
    let app = router(s.clone());
    let (st, l) = call(
        &app,
        "GET",
        "/v1/lyrics?title=Known%20Song&artist=Someone&duration_ms=200000",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{l}");
    assert_eq!(
        (l["source"].as_str(), l["synced"].as_bool()),
        (Some("lrclib"), Some(true))
    );
    assert_eq!(l["lines"][1]["ms"], 3500);
    let (st, l) = call(&app, "GET", &format!("/v1/library/tracks/{tid}/lyrics"), None).await;
    assert_eq!(st, StatusCode::OK, "{l}");
    assert_eq!(l["source"], "lrclib");
    let (st, _) = call(&app, "GET", "/v1/lyrics?title=Nothing&artist=Nobody", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let before = asked.load(std::sync::atomic::Ordering::SeqCst);
    call(&app, "GET", "/v1/lyrics?title=Nothing&artist=Nobody", None).await;
    assert_eq!(
        asked.load(std::sync::atomic::Ordering::SeqCst),
        before,
        "a none is kept too"
    );
}

/// Browsers seek by asking for byte ranges; the library file endpoint
/// must answer 206 with Accept-Ranges or a seek restarts the track.
#[tokio::test]
async fn library_files_serve_byte_ranges() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("music")).unwrap();
    write_wav(&dir.path().join("music/one.wav"), 4410);
    let s = state(dir.path(), None).await;
    let app = router(s.clone());
    let (st, _) = call(&app, "POST", "/v1/library/scan", Some(serde_json::json!({}))).await;
    assert_eq!(st, StatusCode::OK);
    for _ in 0..500 {
        if !s.is_scanning() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let (_, tracks) = call(&app, "GET", "/v1/library/tracks?limit=10", None).await;
    let id = tracks["tracks"][0]["id"].as_i64().unwrap();
    let full = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/library/tracks/{id}/file"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(full.status(), StatusCode::OK);
    assert_eq!(full.headers()["accept-ranges"], "bytes");
    let total = full.into_body().collect().await.unwrap().to_bytes().len();
    let part = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/library/tracks/{id}/file"))
                .header("range", "bytes=100-199")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(part.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(part.headers()["content-range"], format!("bytes 100-199/{total}"));
    assert_eq!(part.into_body().collect().await.unwrap().to_bytes().len(), 100);
}

/// The folder tree, one level at a time from the library: the music folders at the top, the
/// folders in one with their files and bytes, `leaf` without folders inside, paging, and what
/// the homeserver has once its list is known (TASK-copy-library-folders).
#[tokio::test]
async fn the_folder_tree_counts_each_level_from_the_library() {
    let hs = Arc::new(FakeHomeserver::default());
    let (_dir, s, app, src) = copy_setup(hs.clone(), roomy(), 0).await;
    let songs = [
        src.join("A/01 one.wav"),
        src.join("A/Disc 2/02 two.wav"),
        src.join("B/03 three.wav"),
        src.join("loose.wav"),
    ];
    songs_at(&s, &songs);
    let (there, _) = copied_name(&s, &songs[0]);
    hs.listed.lock().unwrap().push(pm_pubky::homeserver::RemoteFile {
        path: there,
        size: None,
    });
    let (st, top) = call(&app, "GET", "/v1/library/tree", None).await;
    assert_eq!(st, StatusCode::OK, "{top}");
    let root = top["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["path"] == src.to_string_lossy().as_ref())
        .cloned()
        .unwrap_or_else(|| panic!("{top}"));
    assert_eq!(
        (root["files"].as_u64(), root["leaf"].as_bool()),
        (Some(4), Some(false))
    );
    assert_eq!(
        root["present_pending"], true,
        "the homeserver's list isn't known yet: {root}"
    );
    assert!(root.get("present").is_none());

    // Once known (asked in the background): what is already there
    let mut level = serde_json::Value::Null;
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        level = call(
            &app,
            "GET",
            &format!("/v1/library/tree?path={}", src.display()),
            None,
        )
        .await
        .1;
        if level["present_pending"] != true {
            break;
        }
    }
    let kids = level["children"].as_array().unwrap();
    let names: Vec<&str> = kids.iter().map(|k| k["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["A", "B"], "{level}");
    assert_eq!(
        (level["files"].as_u64(), kids[0]["files"].as_u64()),
        (Some(4), Some(2))
    );
    assert_eq!(
        (kids[0]["present"].as_u64(), kids[1]["present"].as_u64()),
        (Some(1), Some(0))
    );
    assert_eq!(
        (kids[0]["leaf"].as_bool(), kids[1]["leaf"].as_bool()),
        (Some(false), Some(true))
    );
    let bytes_a: u64 = songs[..2]
        .iter()
        .map(|p| std::fs::metadata(p).unwrap().len())
        .sum();
    assert_eq!(kids[0]["bytes"].as_u64(), Some(bytes_a));

    let page = |after: Option<&str>| {
        let app = app.clone();
        let mut uri = format!("/v1/library/tree?path={}&limit=1", src.display());
        if let Some(a) = after {
            uri.push_str(&format!("&after={a}"));
        }
        async move { call(&app, "GET", &uri, None).await.1 }
    };
    let one = page(None).await;
    assert_eq!(
        (one["children"][0]["name"].as_str(), one["next"].as_str()),
        (Some("A"), Some("A"))
    );
    let two = page(Some("A")).await;
    assert_eq!(two["children"][0]["name"], "B");
    assert!(two.get("next").is_none(), "{two}");
}

/// The folder tree stays quick on a big library: a level of 20 000 songs.
#[tokio::test]
async fn the_folder_tree_answers_quickly_for_twenty_thousand_songs() {
    let hs = Arc::new(FakeHomeserver::default());
    let (_dir, s, app, src) = copy_setup(hs, roomy(), 0).await;
    {
        let lib = s.library.lock().unwrap();
        lib.begin_batch().unwrap();
        for i in 0..20_000 {
            let p = src.join(format!("Artist {}/Album {}/{i:05}.flac", i / 200, i / 20));
            let mut t = untagged_track(p, &format!("Song {i}"), "Band");
            t.file.size_bytes = 30_000_000;
            lib.upsert_track(&t).unwrap();
        }
        lib.commit_batch().unwrap();
    }
    for uri in [
        "/v1/library/tree".to_string(),
        format!("/v1/library/tree?path={}", src.display()),
    ] {
        let t = std::time::Instant::now();
        let (st, v) = call(&app, "GET", &uri, None).await;
        assert_eq!(st, StatusCode::OK);
        let ms = t.elapsed().as_millis();
        assert!(ms < 1500, "{uri}: {ms} ms");
        eprintln!(
            "{uri}: {ms} ms, {} children",
            v["children"].as_array().unwrap().len()
        );
    }
}

/// A cover is served by what its bytes are, never by a type stored with it: a picture whose
/// tag claimed another type is served as the picture it is, bytes that are no picture are not
/// served at all, and every picture says it is only that (`nosniff`, a sandboxing policy).
#[tokio::test]
async fn covers_are_served_by_their_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(dir.path(), Some("s3cret")).await;
    let mut album_ids = Vec::new();
    for (n, bytes, claimed) in [
        ("a", b"\x89PNG\r\n\x1a\n-picture".to_vec(), "text/html"),
        ("b", b"<html><body>page</body></html>".to_vec(), "text/html"),
        (
            "c",
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec(),
            "image/svg+xml",
        ),
    ] {
        let lib = s.library.lock().unwrap();
        let mut t = untagged_track(dir.path().join(format!("{n}.flac")), n, "Someone");
        t.album = Some(pm_model::Album {
            id: None,
            title: format!("Album {n}"),
            album_artist: None,
            release_mbid: None,
            release_group_mbid: None,
            year: None,
            upc: None,
            total_discs: None,
        });
        let id = lib.upsert_track(&t).unwrap();
        let album = lib.get_track(id).unwrap().unwrap().album.unwrap().id.unwrap();
        let file = dir.path().join(format!("{n}.cover"));
        std::fs::write(&file, &bytes).unwrap();
        lib.set_album_art(&pm_library::db::AlbumArt {
            album_id: album,
            file,
            mime: claimed.into(),
            source: "embedded".into(),
        })
        .unwrap();
        album_ids.push(album);
    }
    let app = router(s.clone());
    let get = |id: i64| {
        let app = app.clone();
        async move {
            let req = Request::builder()
                .uri(format!("/v1/library/albums/{id}/cover"))
                .header("authorization", "Bearer s3cret")
                .body(Body::empty())
                .unwrap();
            let r = app.oneshot(req).await.unwrap();
            (r.status(), r.headers().clone())
        }
    };
    let (st, h) = get(album_ids[0]).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(h[header::CONTENT_TYPE], "image/png");
    assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(h[header::CONTENT_SECURITY_POLICY], "default-src 'none'; sandbox");
    for id in &album_ids[1..] {
        let (st, h) = get(*id).await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{h:?}");
    }
}

/// Files relayed from another server keep their type when it is audio, video or a raster
/// picture; anything else is relayed as what the node asked for.
#[test]
fn relayed_files_keep_only_media_types() {
    assert_eq!(
        art::safe_media_type(Some("audio/flac"), "audio/mpeg"),
        "audio/flac"
    );
    assert_eq!(
        art::safe_media_type(Some("image/jpeg; charset=x"), "image/png"),
        "image/jpeg"
    );
    for other in [
        "text/html",
        "image/svg+xml",
        "application/xhtml+xml",
        "",
        "text/html; image/png",
    ] {
        assert_eq!(
            art::safe_media_type(Some(other), "image/jpeg"),
            "image/jpeg",
            "{other}"
        );
    }
    assert_eq!(art::safe_media_type(None, "audio/flac"), "audio/flac");
}

/// A picture another server relays is answered by its bytes, whatever its headers say: an SVG
/// or a page sent as a picture is not served; a picture sent as a page is served as a picture.
#[tokio::test]
async fn relayed_pictures_are_served_by_their_bytes() {
    let png = b"\x89PNG\r\n\x1a\n-relayed".to_vec();
    let p2 = png.clone();
    let mock = Router::new()
        .route(
            "/svg",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "image/png")],
                    "<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
                )
            }),
        )
        .route(
            "/page",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "image/jpeg")],
                    "<html><body>x</body></html>",
                )
            }),
        )
        .route(
            "/png",
            get(move || {
                let b = p2.clone();
                async move { ([(header::CONTENT_TYPE, "text/html")], b) }
            }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, mock).await.unwrap() });
    let fetch = |path: &str| {
        let url = format!("{base}{path}");
        async move { art::relayed_picture(reqwest::get(url).await.unwrap(), "private").await }
    };
    for bad in ["/svg", "/page"] {
        let r = fetch(bad).await;
        assert_eq!(r.err().map(|e| e.status), Some(StatusCode::NOT_FOUND), "{bad}");
    }
    let r = fetch("/png").await.unwrap();
    assert_eq!(r.headers()[header::CONTENT_TYPE], "image/png");
    assert_eq!(r.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    let body = r.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], &png[..]);
}
