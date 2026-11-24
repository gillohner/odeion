//! `pm-node scan`, `tracks`, `search`, `stats`, `resolve`: the library side of M1.

use crate::config::NodeConfig;
use anyhow::Context;
use pm_library::{Library, Scanner};
use pm_model::{MusicUri, Track};
use std::path::{Path, PathBuf};

fn open(cfg: &NodeConfig) -> anyhow::Result<Library> {
    std::fs::create_dir_all(&cfg.data_dir)?;
    let path = cfg.db_path();
    Library::open_node_library(&path).with_context(|| format!("opening library {}", path.display()))
}

/// A running node's hold on its data folder: a lock on `node.lock` there, kept as long as this
/// lives. The system lets go of it when the process ends, however it ends, so no stale hold
/// stays behind.
#[derive(Debug)]
pub struct DataDirLock {
    _file: std::fs::File,
}

/// What [`lock_data_dir`] found.
#[derive(Debug)]
pub enum Hold {
    /// This process has the data folder now.
    Taken(DataDirLock),
    /// Another process (a running node, a restore) has it.
    Held,
    /// The folder's file system takes no locks (some network shares): nobody can tell.
    NoLocks(std::io::Error),
}

/// Open `node.lock` without following a link at its name (a link there could point a restore
/// run with sudo at another file, which it would then make readable to everyone), and only if it
/// is a plain file with one name. `write`: also create it, open for writing.
fn open_lock_file(path: &Path, write: bool) -> std::io::Result<std::fs::File> {
    let mut opts = std::fs::OpenOptions::new();
    if write {
        opts.create(true).truncate(false).write(true);
    } else {
        opts.read(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // (non-blocking: a FIFO there would otherwise hold the open until someone reads it;
        // on a plain file it changes nothing, and the lock is taken without waiting anyway)
        opts.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = opts.open(path).map_err(|e| {
        // O_NOFOLLOW on a link: ELOOP
        #[cfg(unix)]
        if e.raw_os_error() == Some(libc::ELOOP) {
            return std::io::Error::other(format!(
                "{} is a link, not the node's lock file: remove it",
                path.display()
            ));
        }
        e
    })?;
    let meta = file.metadata()?;
    #[cfg(unix)]
    let one_name = {
        use std::os::unix::fs::MetadataExt;
        meta.nlink() == 1
    };
    #[cfg(not(unix))]
    let one_name = true;
    if !meta.is_file() || !one_name {
        return Err(std::io::Error::other(format!(
            "{} is not a plain file of its own (a folder, a device or a second name of another file): remove it",
            path.display()
        )));
    }
    Ok(file)
}

/// Whether a failed lock means the file system takes no locks at all (some network shares), as
/// opposed to a lock that failed on one that does: only then may the caller go on unchecked.
fn takes_no_locks(e: &std::io::Error) -> bool {
    #[cfg(unix)]
    if e.raw_os_error() == Some(libc::ENOLCK) {
        return true;
    }
    e.kind() == std::io::ErrorKind::Unsupported
}

/// Take the hold on the data folder. `Err`: `node.lock` cannot be made or opened, is not a plain
/// file (a link there is refused), it is another user's file that cannot be locked (a restore run
/// with sudo made it), or the lock failed for another reason than a file system without locks:
/// whoever asked stops rather than go on blind.
pub fn lock_data_dir(data_dir: &Path) -> std::io::Result<Hold> {
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join("node.lock");
    let (file, own) = match open_lock_file(&path, true) {
        Ok(f) => {
            // Readable by everyone (it is empty): a restore run as another user can still lock
            // it, whatever the umask made of it. On the opened file, never by name
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = f.set_permissions(std::fs::Permissions::from_mode(0o644));
            }
            (f, true)
        }
        // Another user's file: reading it is enough to lock it, where the file system allows.
        // (A folder this user cannot write and no file yet: the error as it is)
        Err(e)
            if e.kind() == std::io::ErrorKind::PermissionDenied
                && std::fs::symlink_metadata(&path).is_ok() =>
        {
            (
                open_lock_file(&path, false).map_err(|e| {
                    std::io::Error::new(
                        e.kind(),
                        format!(
                        "{} cannot be opened ({e}): it belongs to another user; give it to this one (chown)",
                        path.display()
                    ),
                    )
                })?,
                false,
            )
        }
        Err(e) => return Err(e),
    };
    match file.try_lock() {
        Ok(()) => Ok(Hold::Taken(DataDirLock { _file: file })),
        Err(std::fs::TryLockError::WouldBlock) => Ok(Hold::Held),
        Err(std::fs::TryLockError::Error(e)) if takes_no_locks(&e) => Ok(Hold::NoLocks(e)),
        Err(std::fs::TryLockError::Error(e)) if own => Err(std::io::Error::new(
            e.kind(),
            format!("{} cannot be locked ({e})", path.display()),
        )),
        Err(std::fs::TryLockError::Error(e)) => Err(std::io::Error::new(
            e.kind(),
            format!(
                "{} cannot be locked ({e}): it belongs to another user; give it to this one (chown)",
                path.display()
            ),
        )),
    }
}

/// How long a starting node waits for the data folder when someone else holds it: the node it
/// replaces, still on its way out, or a restore about to end. Then it does not start (a service
/// manager or Docker starts it again).
pub const HOLD_PATIENCE: std::time::Duration = std::time::Duration::from_secs(15);

/// Take the hold on the data folder for a starting node ([`lock_data_dir`]), waiting up to
/// `patience` for whoever has it. `Ok(None)`: the file system takes no locks (said in the log),
/// the node starts as before. `Err`: someone still holds it after `patience`, or `node.lock`
/// cannot be used: the node does not start, so it never opens a database a restore is replacing
/// or another node writes.
pub async fn wait_for_data_dir(
    data_dir: &Path,
    patience: std::time::Duration,
) -> anyhow::Result<Option<DataDirLock>> {
    let started = std::time::Instant::now();
    let mut said = false;
    loop {
        match lock_data_dir(data_dir)
            .with_context(|| format!("the lock on the data folder {}", data_dir.display()))?
        {
            Hold::Taken(hold) => return Ok(Some(hold)),
            Hold::NoLocks(e) => {
                tracing::warn!(
                    data_dir = %data_dir.display(),
                    "the data folder takes no locks ({e}): `odeiond restore` cannot tell that this node runs"
                );
                return Ok(None);
            }
            Hold::Held if started.elapsed() >= patience => anyhow::bail!(
                "another node or `odeiond restore` is using the data folder {}: stop that node, or let the \
                 restore finish, then start this one",
                data_dir.display()
            ),
            Hold::Held => {
                if !said {
                    tracing::info!(data_dir = %data_dir.display(), "the data folder is in use: waiting for it");
                    said = true;
                }
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
        }
    }
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

/// The copy a restore works on, made readable by the node's user alone before anything is
/// written to it: this node's sign-ins, invite links and signup token go into it before it
/// becomes the library.
fn write_staged(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// Replace the library database with a backup. Stop the node first: the
/// previous database is kept as `library.db.bak`, and stale WAL files are
/// removed so SQLite cannot replay them into the restored copy. While a node runs on the data
/// folder this is refused: that node would go on writing into the database it has open (now
/// `library.db.bak`, without its WAL files), and nothing it stores would be in the restored one.
pub fn restore(cfg: &NodeConfig, file: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(&cfg.data_dir)?;
    // Held to the end of the restore, also when it fails
    let _hold = match lock_data_dir(&cfg.data_dir)
        .with_context(|| format!("the lock on the data folder {}", cfg.data_dir.display()))?
    {
        Hold::Taken(hold) => Some(hold),
        Hold::Held => anyhow::bail!(
            "a node (or another restore) is running on {}: stop it first, or restore from Settings › Backup \
             in the web app, which works while it runs",
            cfg.data_dir.display()
        ),
        // A folder without locks: as before, on the word that the node is stopped
        Hold::NoLocks(e) => {
            println!("note: this folder takes no locks ({e}), so whether a node runs on it cannot be checked; going on");
            None
        }
    };
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
        write_staged(&staged, &parsed.database)?;
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
        write_staged(&staged, &bytes)?;
    }
    let stats = {
        let lib =
            Library::open(&staged).with_context(|| format!("{} is not a library backup", file.display()))?;
        // What is this node's own (stream key, id, backup schedule, files jobs, the owner's
        // sessions) stays, and the sign-ins, invites and parties a plain copy doesn't carry stay as
        // they are here. A current database that won't open (damaged, or from a newer version) is
        // the reason to restore: then nothing is carried over, and the restore goes on. Without a
        // node to keep them from, none of the backup's own settings goes in either (an older backup
        // still holds them, its ended sessions among them)
        let here = if db.exists() {
            // (a newer binary's restore upgrades it: with the copy a start would keep)
            match Library::open_node_library(&db) {
                Ok(here) => Some(here),
                Err(e) => {
                    println!(
                        "note: the current database can't be read ({e}); its own settings (backup schedule, \
                         stream key, sign-ins) are not carried over"
                    );
                    None
                }
            }
        } else {
            None
        };
        match here {
            Some(here) => pm_api::backup::keep_this_nodes(&here, &lib)?,
            None => pm_api::backup::forget_node_own(&lib)?,
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
        // Closed before looking beside the copy: an open connection has its own -wal and -shm there
        drop(copy);
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
        // The restored library (it got this node's sign-ins) is the node's user's alone, from the
        // moment it was made; nothing of the work is left
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(cfg.db_path()).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
            assert!(!cfg.db_path().with_extension("db.restoring").exists());
        }
    }

    /// The stopped-node CLI uses the same membership rule as the web restore: continuing and
    /// paused music access keep the current name (legacy None too); regained access gets a new
    /// name. Empty or unreadable current rosters cannot revive the backup's old identity.
    #[test]
    fn the_restore_command_reconciles_report_memberships_before_replacing_the_library() {
        for (access, name, enabled) in [
            ("music", Some("membership-b"), true),
            ("music", None, true),
            ("music", Some("paused-b"), false),
            ("homeserver", None, false),
            ("absent", None, false),
            ("unreadable", None, false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let cfg = NodeConfig {
                data_dir: dir.path().join("data"),
                ..Default::default()
            };
            let roster = |access, reports: Option<&str>, enabled| {
                serde_json::json!({"people": [{
                    "key": "ann", "access": access, "music_enabled": enabled,
                    "added_at": 1, "reports": reports
                }]})
                .to_string()
            };
            open(&cfg)
                .unwrap()
                .set_setting("people", &roster("music", Some("membership-a"), true))
                .unwrap();
            let file = dir.path().join("copy.sqlite");
            backup(&cfg, &file, false).unwrap();
            let current = match access {
                "absent" => "{\"people\":[]}".into(),
                "unreadable" => "{not a roster".into(),
                _ => roster(access, name, enabled),
            };
            open(&cfg).unwrap().set_setting("people", &current).unwrap();
            restore(&cfg, &file).unwrap();
            let read = |cfg: &NodeConfig| {
                serde_json::from_str::<pm_api::people::Roster>(
                    &open(cfg).unwrap().get_setting("people").unwrap().unwrap(),
                )
                .unwrap()
            };
            let restored = read(&cfg);
            let ann = &restored.people[0];
            assert_eq!(ann.access, pm_api::people::Access::Music);
            if access == "music" {
                assert_eq!(ann.reports.as_deref(), name);
            } else {
                assert!(ann.reports.is_some());
                assert_ne!(ann.reports.as_deref(), Some("membership-a"));
            }
            let empty = NodeConfig {
                data_dir: dir.path().join("empty"),
                ..Default::default()
            };
            restore(&empty, &file).unwrap();
            let restored = read(&empty);
            assert!(restored.people[0].reports.is_some());
            assert_ne!(restored.people[0].reports.as_deref(), Some("membership-a"));
        }
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
        // Closed before its journal files are written to: they would be this connection's
        drop(copy);
        // Without --force an existing target and its files stay as they are
        std::fs::write(out.join("copy.sqlite-wal"), b"someone's").unwrap();
        assert!(backup(&cfg, &file, false).is_err());
        assert_eq!(std::fs::read(out.join("copy.sqlite-wal")).unwrap(), b"someone's");
    }

    /// A backup from before the node's own settings were left out, restored onto an empty data
    /// folder: none of them goes in. The owner's sessions it holds stay ended, and the node makes
    /// its own id and stream key; the library comes back.
    #[test]
    fn an_older_backup_onto_an_empty_node_brings_no_ended_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("older-backup.sqlite");
        let hash = ["fixture", "session", "hash"].join("-");
        {
            let lib = Library::open(&old).unwrap();
            let list = format!("[{{\"hash\":\"{hash}\",\"key\":\"k\",\"created_at\":1}}]");
            lib.set_setting("accounts.owner_sessions", &list).unwrap();
            lib.set_setting("accounts.device_sessions", &list).unwrap();
            lib.set_setting("node.id", "the-old-node").unwrap();
            lib.set_setting("stream.secret", "the-old-stream-key").unwrap();
            lib.set_setting("home.layout", "kept").unwrap();
        }
        let cfg = NodeConfig {
            data_dir: dir.path().join("fresh"),
            ..Default::default()
        };
        assert!(!cfg.db_path().exists(), "an empty data folder");
        restore(&cfg, &old).unwrap();
        let lib = open(&cfg).unwrap();
        for k in [
            "accounts.owner_sessions",
            "accounts.device_sessions",
            "node.id",
            "stream.secret",
        ] {
            assert_eq!(lib.get_setting(k).unwrap(), None, "{k} came in from the backup");
        }
        assert_eq!(lib.get_setting("home.layout").unwrap().as_deref(), Some("kept"));
        // A current database that can't be read is the same: nothing of the backup's own goes in
        drop(lib);
        std::fs::write(cfg.db_path(), b"not a database at all").unwrap();
        restore(&cfg, &old).unwrap();
        let lib = open(&cfg).unwrap();
        assert_eq!(lib.get_setting("accounts.owner_sessions").unwrap(), None);
        assert_eq!(lib.get_setting("home.layout").unwrap().as_deref(), Some("kept"));
    }

    /// `restore` does not replace the database while a node holds the data folder, and does
    /// once that node is gone.
    #[test]
    fn the_restore_command_is_refused_while_a_node_runs_on_the_data_folder() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = NodeConfig {
            data_dir: dir.path().join("data"),
            ..Default::default()
        };
        open(&cfg).unwrap().set_setting("home.layout", "old").unwrap();
        let file = dir.path().join("copy.sqlite");
        backup(&cfg, &file, false).unwrap();
        open(&cfg).unwrap().set_setting("home.layout", "new").unwrap();
        let bak = cfg.db_path().with_extension("db.bak");

        let Hold::Taken(running) = lock_data_dir(&cfg.data_dir).unwrap() else {
            panic!("no node holds it yet")
        };
        assert!(
            matches!(lock_data_dir(&cfg.data_dir).unwrap(), Hold::Held),
            "held once"
        );
        let e = restore(&cfg, &file).unwrap_err().to_string();
        assert!(e.contains("is running on"), "{e}");
        assert!(!bak.exists(), "the database was not moved");
        assert_eq!(
            open(&cfg).unwrap().get_setting("home.layout").unwrap().as_deref(),
            Some("new")
        );

        drop(running);
        restore(&cfg, &file).unwrap();
        assert!(bak.exists());
        assert_eq!(
            open(&cfg).unwrap().get_setting("home.layout").unwrap().as_deref(),
            Some("old")
        );
        // The restore let go of it again
        assert!(matches!(lock_data_dir(&cfg.data_dir).unwrap(), Hold::Taken(_)));
    }

    /// A starting node waits for the data folder, gets it once whoever held it lets go, and does
    /// not start (an error) when it is still held after the wait. A restore holds it to its end,
    /// also when it fails, and a second restore meanwhile is refused.
    #[tokio::test]
    async fn a_starting_node_waits_for_the_data_folder_then_does_not_start() {
        use std::time::{Duration, Instant};
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let short = Duration::from_millis(300);
        let taken = |h: Hold| match h {
            Hold::Taken(l) => l,
            Hold::Held => panic!("held"),
            Hold::NoLocks(e) => panic!("no locks here: {e}"),
        };
        // Nobody holds it: at once
        drop(wait_for_data_dir(&data, short).await.unwrap().expect("free"));
        // Another holds it the whole time: the wait, then no start
        let other = taken(lock_data_dir(&data).unwrap());
        let asked = Instant::now();
        let e = wait_for_data_dir(&data, short).await.unwrap_err().to_string();
        assert!(e.contains("is using the data folder"), "{e}");
        assert!(asked.elapsed() >= short && asked.elapsed() < Duration::from_secs(10));
        // It lets go during the wait: the hold
        let gone = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            drop(other);
        });
        let hold = wait_for_data_dir(&data, Duration::from_secs(5)).await.unwrap();
        assert!(hold.is_some(), "the hold, once the other let go");
        gone.await.unwrap();
        // While this node holds it a restore is refused and changes nothing; a failed restore
        // lets go of it
        let cfg = NodeConfig {
            data_dir: data.clone(),
            ..Default::default()
        };
        let refused = restore(&cfg, &dir.path().join("no-such-backup")).unwrap_err();
        assert!(refused.to_string().contains("is running on"), "{refused}");
        drop(hold);
        assert!(restore(&cfg, &dir.path().join("no-such-backup")).is_err());
        assert!(
            matches!(lock_data_dir(&data).unwrap(), Hold::Taken(_)),
            "let go after a failure"
        );
        // The lock file can be locked by another user too: made readable to all, also when it
        // was not (whatever the umask)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let lock = data.join("node.lock");
            std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o600)).unwrap();
            drop(taken(lock_data_dir(&data).unwrap()));
            let mode = std::fs::metadata(&lock).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o644);
        }
    }

    /// A link at `node.lock` is refused and its target left as it was (a restore run with sudo
    /// would otherwise make another user's file readable to everyone), and so is a second name
    /// of another file.
    #[cfg(unix)]
    #[test]
    fn the_lock_file_is_never_a_link_or_a_second_name() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let secret = dir.path().join("secret");
        std::fs::write(&secret, "x").unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;

        std::os::unix::fs::symlink(&secret, data.join("node.lock")).unwrap();
        let e = lock_data_dir(&data).unwrap_err().to_string();
        assert!(e.contains("is a link"), "{e}");
        assert_eq!(mode(&secret), 0o600, "the link's target was not touched");

        std::fs::remove_file(data.join("node.lock")).unwrap();
        std::fs::hard_link(&secret, data.join("node.lock")).unwrap();
        let e = lock_data_dir(&data).unwrap_err().to_string();
        assert!(e.contains("not a plain file of its own"), "{e}");
        assert_eq!(mode(&secret), 0o600, "the other name's file was not touched");

        std::fs::remove_file(data.join("node.lock")).unwrap();
        assert!(matches!(lock_data_dir(&data).unwrap(), Hold::Taken(_)));
    }

    /// Only a file system without locks lets a caller go on unchecked; any other failed lock
    /// stops it.
    #[test]
    fn only_a_file_system_without_locks_goes_on_unchecked() {
        use std::io::{Error, ErrorKind};
        assert!(takes_no_locks(&Error::from(ErrorKind::Unsupported)));
        #[cfg(unix)]
        {
            assert!(takes_no_locks(&Error::from_raw_os_error(libc::ENOLCK)));
            assert!(takes_no_locks(&Error::from_raw_os_error(libc::EOPNOTSUPP)));
            assert!(!takes_no_locks(&Error::from_raw_os_error(libc::EIO)));
            assert!(!takes_no_locks(&Error::from_raw_os_error(libc::EBADF)));
        }
        assert!(!takes_no_locks(&Error::from(ErrorKind::PermissionDenied)));
    }

    /// A FIFO at `node.lock` (anyone who can write the data folder can make one) is refused at
    /// once: the open does not wait for a reader, so neither a start nor a restore hangs.
    #[cfg(unix)]
    #[test]
    fn a_fifo_at_the_lock_file_is_refused_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let fifo = std::ffi::CString::new(data.join("node.lock").to_str().unwrap()).unwrap();
        // SAFETY: a NUL-terminated path that lives through the call
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(lock_data_dir(&data).map(|_| ()).map_err(|e| e.to_string()));
        });
        let r = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the open waited for a reader");
        // (no reader: opening a FIFO without blocking for writing fails at once, ENXIO)
        assert!(r.is_err(), "{r:?}");
    }
}
