//! `pm-node scan`, `tracks`, `search`, `stats`, `resolve`: the library side of M1.

use crate::config::NodeConfig;
use anyhow::Context;
use pm_library::{Library, Scanner};
use pm_model::{MusicUri, Track};
use std::path::{Path, PathBuf};

fn open(cfg: &NodeConfig) -> anyhow::Result<Library> {
    std::fs::create_dir_all(&cfg.data_dir)?;
    let path = cfg.db_path();
    Library::open(&path).with_context(|| format!("opening library {}", path.display()))
}

/// The work folders `backup` makes in the data folder: `backup-work-<pid>`.
const WORK_PREFIX: &str = "backup-work-";

/// Work folders an earlier `backup` left (stopped midway) go; one of a backup still running is
/// younger than an hour and stays.
fn clear_old_work(data_dir: &Path) {
    let Ok(rd) = std::fs::read_dir(data_dir) else {
        return;
    };
    for e in rd.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > std::time::Duration::from_secs(3600));
        if old && e.file_name().to_string_lossy().starts_with(WORK_PREFIX) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Snapshot the library database to `file` (works while the node runs).
pub fn backup(cfg: &NodeConfig, file: &Path, force: bool) -> anyhow::Result<()> {
    if !force && file.exists() {
        anyhow::bail!("{} exists (--force replaces it)", file.display());
    }
    let lib = open(cfg)?;
    // Made and cleaned in a private folder on the node's own disk: a copy that still holds
    // secrets never reaches the target's disk or stick (which may keep deleted data). Only the
    // clean copy goes beside the target, as a file of its maker's alone (0600), then into place.
    let work = cfg.data_dir.join(format!("{WORK_PREFIX}{}", std::process::id()));
    clear_old_work(&cfg.data_dir);
    let mut b = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(&work)
        .with_context(|| format!("creating {}", work.display()))?;
    let made = (|| -> anyhow::Result<()> {
        let tmp = work.join("library.sqlite");
        lib.backup_to(&tmp)
            .with_context(|| format!("writing backup {}", file.display()))?;
        {
            // No sign-in, signup token, invite link, party code, passphrase or key of this node
            // goes into a plain database copy (Settings › Backup locks sign-ins with a passphrase)
            let copy = Library::open(&tmp)?;
            pm_api::backup::scrub_plain_copy(&copy)?;
            copy.vacuum_into_place()?;
        }
        let dir = file
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let name = file
            .file_name()
            .with_context(|| format!("{} names no file", file.display()))?;
        let part = dir.join(format!(".{}.part-{}", name.to_string_lossy(), std::process::id()));
        let placed = (|| -> std::io::Result<()> {
            let mut o = std::fs::OpenOptions::new();
            o.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                o.mode(0o600);
            }
            let mut out = o.open(&part)?;
            std::io::copy(&mut std::fs::File::open(&tmp)?, &mut out)?;
            out.sync_all()?;
            drop(out);
            // An old database's journal files beside the target (its WAL may hold its sign-ins,
            // and SQLite would read it with the new file): gone before the new file goes in. Not
            // when the target stays (no --force over an existing file)
            if force || !file.exists() {
                for ext in ["-wal", "-shm", "-journal"] {
                    let mut side = file.as_os_str().to_owned();
                    side.push(ext);
                    match std::fs::remove_file(PathBuf::from(side)) {
                        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                        _ => {}
                    }
                }
            }
            // Into place only once whole: with --force over the old one in one step (it stays
            // until then), without it never over a file that appeared meanwhile
            if force {
                std::fs::rename(&part, file)
            } else {
                match std::fs::hard_link(&part, file) {
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(e),
                    // A filesystem without links (FAT, exFAT on a stick): moved, if still free
                    Err(_) if !file.exists() => std::fs::rename(&part, file),
                    r => r,
                }
            }
        })();
        // Gone after a rename; the second name after a link, or what a failure left
        let _ = std::fs::remove_file(&part);
        placed.with_context(|| format!("writing backup {}", file.display()))?;
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&work);
    made?;
    let s = lib.stats()?;
    let size = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
    println!(
        "backup written: {} ({} tracks, {} albums, {:.1} MB)",
        file.display(),
        s.tracks,
        s.albums,
        size as f64 / 1e6
    );
    Ok(())
}

/// Replace the library database with a backup. Stop the node first: the
/// previous database is kept as `library.db.bak`, and stale WAL files are
/// removed so SQLite cannot replay them into the restored copy.
pub fn restore(cfg: &NodeConfig, file: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(&cfg.data_dir)?;
    let db = cfg.db_path();
    // Validate (and migrate) a scratch copy first so a broken file never
    // replaces a working library.
    let staged = db.with_extension("db.restoring");
    let _ = std::fs::remove_file(&staged);
    let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    if bytes.starts_with(b"PK") {
        // A backup file from Settings: its library only here (restore it in the web app to
        // put the sign-ins back too)
        let parsed = pm_api::backup::parse(&bytes)?;
        std::fs::write(&staged, &parsed.database)?;
        if parsed.manifest.secrets.is_some() {
            println!("note: sign-ins are restored only from Settings › Backup in the web app");
        }
        if !parsed.manifest.left_out.is_empty() {
            println!(
                "note: this backup was made without a passphrase and holds no sign-ins, invite links, \
                 signup token or parties; this node's own stay as they are"
            );
        }
    } else {
        std::fs::write(&staged, &bytes)?;
    }
    let stats = {
        let lib =
            Library::open(&staged).with_context(|| format!("{} is not a library backup", file.display()))?;
        // What is this node's own (stream key, id, backup schedule, files jobs) stays, and the
        // sign-ins, invites and parties a plain copy doesn't carry stay as they are here. A current
        // database that won't open (damaged, or from a newer version) is the reason to restore:
        // then nothing is carried over, and the restore goes on
        if db.exists() {
            match Library::open(&db) {
                Ok(here) => pm_api::backup::keep_this_nodes(&here, &lib)?,
                Err(e) => println!(
                    "note: the current database can't be read ({e}); its own settings (backup schedule, \
                     stream key, sign-ins) are not carried over"
                ),
            }
        }
        lib.stats()?
    };
    for suffix in ["db-wal", "db-shm"] {
        let _ = std::fs::remove_file(db.with_extension(suffix));
    }
    if db.exists() {
        std::fs::rename(&db, db.with_extension("db.bak"))?;
    }
    std::fs::rename(&staged, &db)?;
    println!(
        "restored {} into {} ({} tracks, {} albums); the previous database is {}",
        file.display(),
        db.display(),
        stats.tracks,
        stats.albums,
        db.with_extension("db.bak").display()
    );
    Ok(())
}

/// Scan the given folders, or the configured ones when none are given.
pub fn scan(cfg: &NodeConfig, folders: &[std::path::PathBuf]) -> anyhow::Result<()> {
    let lib = open(cfg)?;
    let folders: Vec<&Path> = if folders.is_empty() {
        cfg.library.folders.iter().map(|p| p.as_path()).collect()
    } else {
        folders.iter().map(|p| p.as_path()).collect()
    };
    anyhow::ensure!(
        !folders.is_empty(),
        "no folders to scan: pass paths or set [library] folders"
    );
    let scanner = Scanner::new(&lib).with_art_dir(cfg.data_dir.join("art"));
    for root in folders {
        lib.add_folder(root)?;
        let r = scanner
            .scan(root)
            .with_context(|| format!("scanning {}", root.display()))?;
        println!(
            "{}: seen {} added {} updated {} unchanged {} removed {} errors {}",
            root.display(),
            r.files_seen,
            r.files_added,
            r.files_updated,
            r.files_unchanged,
            r.files_removed,
            r.errors.len()
        );
        for (p, e) in r.errors.iter().take(20) {
            println!("  ! {}: {e}", p.display());
        }
    }
    print_stats(&lib)
}

pub fn stats(cfg: &NodeConfig) -> anyhow::Result<()> {
    print_stats(&open(cfg)?)
}

fn print_stats(lib: &Library) -> anyhow::Result<()> {
    let s = lib.stats()?;
    println!(
        "library: {} tracks, {} albums, {} artists, {} files ({} missing), {:.1} h",
        s.tracks,
        s.albums,
        s.artists,
        s.files,
        s.missing_files,
        s.total_duration_ms as f64 / 3_600_000.0
    );
    Ok(())
}

pub fn tracks(cfg: &NodeConfig, limit: u32) -> anyhow::Result<()> {
    let lib = open(cfg)?;
    print_tracks(&lib.list_tracks(None, limit)?);
    Ok(())
}

pub fn search(cfg: &NodeConfig, q: &str, limit: u32) -> anyhow::Result<()> {
    let lib = open(cfg)?;
    print_tracks(&lib.search_tracks(q, limit)?);
    Ok(())
}

fn print_tracks(tracks: &[Track]) {
    for t in tracks {
        let artist = t
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let album = t.album.as_ref().map(|a| a.title.as_str()).unwrap_or("-");
        let canon = t
            .canonical
            .as_ref()
            .map(|(u, c)| format!("{} ({:.2})", u.to_url(), c.score()))
            .unwrap_or_else(|| "-".into());
        println!(
            "{:>6}  {:<32} {:<24} {:<24} {}  {}",
            t.id.map(|i| i.0).unwrap_or(0),
            t.title.chars().take(32).collect::<String>(),
            artist.chars().take(24).collect::<String>(),
            album.chars().take(24).collect::<String>(),
            t.file.format,
            canon
        );
    }
}

/// Resolve a music URL to its canonical MusicBrainz form via the MusicBrainz API.
pub async fn resolve(cfg: &NodeConfig, input: &str) -> anyhow::Result<()> {
    let uri = MusicUri::parse(input).with_context(|| format!("parsing {input}"))?;
    if uri.is_canonical() {
        println!("already canonical: {}", uri.to_url());
        return Ok(());
    }
    let contact = cfg.pubky_config()?.user_agent_contact;
    let mut mb_cfg = pm_library::musicbrainz::MusicBrainzConfig::new(contact);
    if let Some(root) = cfg.musicbrainz_root() {
        mb_cfg.base_url = format!("{root}/ws/2");
    }
    let mb = pm_library::musicbrainz::MusicBrainzClient::new(mb_cfg)?;
    match mb.canonicalize(&uri).await? {
        Some(c) => println!("{} -> {}", uri.to_url(), c.to_url()),
        None => println!("{}: MusicBrainz has no link for this URL yet", uri.to_url()),
    }
    Ok(())
}

/// `pm-node import-playlist`: parse the file, resolve against the library and
/// store a private v1 playlist post (the same object the API and sync use).
pub fn import_playlist(cfg: &NodeConfig, file: &Path, name: Option<&str>) -> anyhow::Result<()> {
    use pm_library::import::{detect_format, name_from_file, parse};
    use pm_pubky::{MusicContent, MusicPost, Playlist, PlaylistItem};

    let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let format = detect_format(file.to_str(), &bytes).context("cannot tell the playlist format")?;
    let list = parse(&bytes, format, file.parent())?;
    let lib = open(cfg)?;
    let mut items = Vec::new();
    let mut unresolved = Vec::new();
    for t in &list.tracks {
        match lib.resolve_import(t)? {
            Some(track) => match track.publishable_uri() {
                Some(u) => items.push(PlaylistItem::new(u)),
                None => unresolved.push(format!("{} (in library, no canonical id)", t.title)),
            },
            None => unresolved.push(match (&t.artist, &t.path) {
                (Some(a), _) => format!("{} — {a}", t.title),
                (None, Some(p)) => format!("{} ({p})", t.title),
                _ => t.title.clone(),
            }),
        }
    }
    let name = name
        .map(str::to_string)
        .or(list.name.clone())
        .unwrap_or_else(|| name_from_file(file));
    println!(
        "{}: {} entries, {} matched, {} unmatched",
        name,
        list.tracks.len(),
        items.len(),
        unresolved.len()
    );
    for u in &unresolved {
        println!("  unmatched: {u}");
    }
    anyhow::ensure!(
        !items.is_empty(),
        "nothing matched the library; scan the folders first"
    );
    let playlist = Playlist::new(name, None, None, items);
    let post = MusicPost::new(MusicContent::Playlist(playlist));
    let id = post.create_id();
    post.validate(Some(&id))?;
    let json = serde_json::to_string(&post.post)?;
    let (created_at, updated_at) = match &post.content {
        MusicContent::Playlist(p) => (p.created_at, p.updated_at),
        other => (other.created_micros(), other.created_micros()),
    };
    lib.upsert_music_post(&pm_library::MusicPostRow {
        id: id.clone(),
        kind: post.content.type_name().to_string(),
        visibility: "private".into(),
        json,
        created_at,
        updated_at,
        synced_at: None,
        deleted: false,
    })?;
    println!("stored playlist {id} (private; `pm-node serve` syncs it to the homeserver when signed in)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `pm-node backup` leaves every sign-in out of its plain copy: the owner's and each
    /// listener's own (`person.<key>.*`); other settings stay.
    #[test]
    fn the_backup_command_leaves_out_every_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = NodeConfig {
            data_dir: dir.path().join("data"),
            ..Default::default()
        };
        {
            let lib = open(&cfg).unwrap();
            lib.set_setting("tidal.user", "OWNER-SECRET").unwrap();
            lib.set_setting("person.abc.tidal.user", "LISTENER-SECRET")
                .unwrap();
            lib.set_setting("person.abc.qobuz.unofficial", "LISTENER-QOBUZ")
                .unwrap();
            lib.set_setting("person.abc.tidal.signin_way", "app").unwrap();
            lib.set_setting("home.layout", "kept").unwrap();
        }
        let file = dir.path().join("copy.sqlite");
        backup(&cfg, &file, false).unwrap();
        let copy = Library::open(&file).unwrap();
        for k in [
            "tidal.user",
            "person.abc.tidal.user",
            "person.abc.qobuz.unofficial",
        ] {
            assert!(copy.get_setting(k).unwrap().is_none(), "{k} in the plain copy");
        }
        assert_eq!(copy.get_setting("home.layout").unwrap().as_deref(), Some("kept"));
        assert_eq!(
            copy.get_setting("person.abc.tidal.signin_way")
                .unwrap()
                .as_deref(),
            Some("app"),
            "not a sign-in"
        );
    }

    /// The plain copy keeps no way in at all: besides sign-ins, not the backup schedule's
    /// passphrase, a vault record, the stream key, an unused invite's token, the signup token or a
    /// party; invites stay without their link. The file is its maker's alone.
    #[test]
    fn the_backup_command_keeps_no_way_in() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = NodeConfig {
            data_dir: dir.path().join("data"),
            ..Default::default()
        };
        // Fake values, put together at run time
        let v = |n: &str| ["fixture", "value", n].join("-");
        {
            let lib = open(&cfg).unwrap();
            lib.set_setting(
                "backup.schedule",
                &format!("{{\"passphrase\":\"{}\"}}", v("pass")),
            )
            .unwrap();
            lib.set_setting("vault.signed_out.tidal", &v("vault")).unwrap();
            lib.set_setting("stream.secret", &v("stream")).unwrap();
            lib.set_setting("homeserver.owner_signup_token", &v("signup"))
                .unwrap();
            lib.set_setting("party.parties", &format!("[{{\"code\":\"{}\"}}]", v("party")))
                .unwrap();
            lib.set_setting(
                "people",
                &format!(
                    "{{\"people\":[],\"invites\":[{{\"id\":\"i1\",\"access\":\"music\",\"token\":\"{}\",\"created_at\":1}}]}}",
                    v("invite")
                ),
            )
            .unwrap();
        }
        let file = dir.path().join("copy.sqlite");
        backup(&cfg, &file, false).unwrap();
        let raw = String::from_utf8_lossy(&std::fs::read(&file).unwrap()).into_owned();
        for n in ["pass", "vault", "stream", "signup", "party", "invite"] {
            assert!(!raw.contains(&v(n)), "{n} in the plain copy");
        }
        let copy = Library::open(&file).unwrap();
        let people = copy.get_setting("people").unwrap().unwrap();
        assert!(
            people.contains("\"i1\""),
            "the invite stays, without its link: {people}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        // Nothing left beside it
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
        // An existing file isn't overwritten without --force
        assert!(backup(&cfg, &file, false).is_err());
        backup(&cfg, &file, true).unwrap();
    }

    /// A backup that fails leaves the one already there as it was, with --force too; and a list
    /// of invites in a shape the copy doesn't know is not kept rather than kept with a token.
    #[test]
    fn a_failed_backup_keeps_the_old_one_and_odd_invites_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = NodeConfig {
            data_dir: dir.path().join("data"),
            ..Default::default()
        };
        let secret = ["fixture", "value", "odd"].join("-");
        open(&cfg)
            .unwrap()
            .set_setting("people", &format!("{{\"invites\":{{\"token\":\"{secret}\"}}}}"))
            .unwrap();
        let file = dir.path().join("copy.sqlite");
        backup(&cfg, &file, false).unwrap();
        let copy = Library::open(&file).unwrap();
        assert_eq!(copy.get_setting("people").unwrap(), None);
        drop(copy);
        assert!(!String::from_utf8_lossy(&std::fs::read(&file).unwrap()).contains(&secret));
        // The target's folder can't take the work folder: the backup fails, the old file stays
        let before = std::fs::read(&file).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
            let r = backup(&cfg, &file, true);
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
            // As root the folder takes it anyway; the check means something for a service user
            if r.is_err() {
                assert_eq!(
                    std::fs::read(&file).unwrap(),
                    before,
                    "the old backup is untouched"
                );
            }
        }
    }

    /// Backup then restore on the same node: its own settings (schedule, id, stream key), its
    /// sign-ins and its jobs are as they were; the work happens in the data folder, and nothing is
    /// left there or beside the copy.
    #[test]
    fn a_backup_restored_on_the_same_node_keeps_what_is_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = NodeConfig {
            data_dir: dir.path().join("data"),
            ..Default::default()
        };
        let v = |n: &str| ["fixture", "value", n].join("-");
        let own = [
            ("backup.schedule", format!("{{\"passphrase\":\"{}\"}}", v("pass"))),
            ("node.id", v("id")),
            ("stream.secret", v("stream")),
            ("homeserver.owner_signup_token", v("signup")),
        ];
        {
            let lib = open(&cfg).unwrap();
            for (k, val) in &own {
                lib.set_setting(k, val).unwrap();
            }
        }
        let out = dir.path().join("out");
        std::fs::create_dir(&out).unwrap();
        let file = out.join("copy.sqlite");
        backup(&cfg, &file, false).unwrap();
        let beside: Vec<_> = std::fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(beside, vec![std::ffi::OsString::from("copy.sqlite")]);
        let work_left = std::fs::read_dir(&cfg.data_dir)
            .unwrap()
            .any(|e| e.unwrap().file_name().to_string_lossy().starts_with(WORK_PREFIX));
        assert!(!work_left, "no work folder left in the data folder");

        // Something the node changed after the backup, to see the restore happened
        open(&cfg).unwrap().set_setting("node.later", "x").unwrap();
        restore(&cfg, &file).unwrap();
        let lib = open(&cfg).unwrap();
        for (k, val) in &own {
            assert_eq!(lib.get_setting(k).unwrap().as_deref(), Some(val.as_str()), "{k}");
        }
        assert_eq!(
            lib.get_setting("node.later").unwrap(),
            None,
            "the backup's library is in"
        );
    }

    /// A current database that won't open (damaged, or a newer version's) is what a restore is
    /// for: it goes ahead, carrying nothing over.
    #[test]
    fn a_restore_over_a_damaged_database_goes_ahead() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = NodeConfig {
            data_dir: dir.path().join("data"),
            ..Default::default()
        };
        open(&cfg).unwrap().set_setting("node.id", "old").unwrap();
        let file = dir.path().join("copy.sqlite");
        backup(&cfg, &file, false).unwrap();
        std::fs::write(cfg.db_path(), b"not a database at all").unwrap();
        restore(&cfg, &file).unwrap();
        let lib = open(&cfg).unwrap();
        assert_eq!(
            lib.get_setting("node.id").unwrap(),
            None,
            "nothing carried from the damaged one"
        );
        assert!(cfg.db_path().with_extension("db.bak").exists());
    }

    /// `backup --force` over an old database: its WAL and shared-memory files (which may hold that
    /// database's sign-ins) go, so nothing of it is left beside the new copy or read with it.
    #[test]
    fn backup_over_an_old_database_takes_its_journal_files_too() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = NodeConfig {
            data_dir: dir.path().join("data"),
            ..Default::default()
        };
        open(&cfg).unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir(&out).unwrap();
        let file = out.join("copy.sqlite");
        let secret = ["fixture", "value", "old-wal"].join("-");
        // An old database whose last change is still in its WAL (its connection open)
        let old = Library::open(&out.join("old.sqlite")).unwrap();
        old.set_setting("tidal.user", &secret).unwrap();
        std::fs::copy(out.join("old.sqlite"), &file).unwrap();
        for ext in ["-wal", "-shm"] {
            let from = out.join(format!("old.sqlite{ext}"));
            if from.exists() {
                std::fs::copy(&from, out.join(format!("copy.sqlite{ext}"))).unwrap();
            }
        }
        std::fs::write(out.join("copy.sqlite-wal"), {
            let mut w = std::fs::read(out.join("copy.sqlite-wal")).unwrap_or_default();
            w.extend_from_slice(secret.as_bytes());
            w
        })
        .unwrap();
        drop(old);
        for f in ["old.sqlite", "old.sqlite-wal", "old.sqlite-shm"] {
            let _ = std::fs::remove_file(out.join(f));
        }
        backup(&cfg, &file, true).unwrap();
        for ext in ["-wal", "-shm", "-journal"] {
            assert!(
                !out.join(format!("copy.sqlite{ext}")).exists(),
                "{ext} left beside the copy"
            );
        }
        for e in std::fs::read_dir(&out).unwrap() {
            let raw = std::fs::read(e.unwrap().path()).unwrap();
            assert!(
                !String::from_utf8_lossy(&raw).contains(&secret),
                "the old sign-in is gone"
            );
        }
        let copy = Library::open(&file).unwrap();
        assert_eq!(copy.get_setting("tidal.user").unwrap(), None);
        // Without --force an existing target and its files stay as they are
        std::fs::write(out.join("copy.sqlite-wal"), b"someone's").unwrap();
        assert!(backup(&cfg, &file, false).is_err());
        assert_eq!(std::fs::read(out.join("copy.sqlite-wal")).unwrap(), b"someone's");
    }
}
