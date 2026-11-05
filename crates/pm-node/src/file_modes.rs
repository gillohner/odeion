//! The node's data folder and the files in it that hold sign-ins and keys are its own: at start,
//! ones a group or anyone else may read are made the node's alone (each change is logged). The
//! folder keeps others' "search" permission, so another service that reads one public file in it
//! (the homeserver reads `tunnel/domain`) still can; nobody else can list or open what's inside.

use std::path::{Path, PathBuf};

/// The files [`tighten`] makes the node's alone: the library database and its journal files,
/// the config (it may hold the API token and service secrets), the Pubky session secret, the
/// remote-access key, the app players' keys and a config a restore brought back. Only those that
/// are in the data folder are changed ([`tighten`]).
pub fn private_files(
    data_dir: &Path,
    db: &Path,
    config: Option<&Path>,
    session_secret: &Path,
) -> Vec<PathBuf> {
    let mut v = vec![db.to_path_buf()];
    for ext in ["-wal", "-shm", "-journal"] {
        let mut p = db.as_os_str().to_owned();
        p.push(ext);
        v.push(PathBuf::from(p));
    }
    v.extend(config.map(Path::to_path_buf));
    v.push(session_secret.to_path_buf());
    for name in ["remote.key", "endpoint-keys.json", "node.restored.toml"] {
        v.push(data_dir.join(name));
    }
    v
}

/// What [`tighten`] did.
#[derive(Debug, Default)]
pub struct Tightened {
    /// Made the node's own: the path, its mode before and after.
    pub changed: Vec<(PathBuf, u32, u32)>,
    /// Others may read it (or it couldn't be looked at) and the node couldn't change that: the
    /// path, what is wrong, and the mode it should have. The owner is told how to fix it.
    pub left: Vec<(PathBuf, String, u32)>,
}

/// Makes the data folder 0711 at most and each of `files` that exists 0600 at most. Only what is
/// the node's to change: a file is changed only when it is a regular file (never through a link,
/// never a link's target) whose real folder is the data folder or one inside it. A file kept
/// elsewhere (a config in `/etc`) is its installer's to set, and is left as it is.
#[cfg(unix)]
pub fn tighten(data_dir: &Path, files: &[PathBuf]) -> Tightened {
    use std::os::unix::fs::PermissionsExt;
    let mut out = Tightened::default();
    let Ok(root) = data_dir.canonicalize() else {
        return out;
    };
    let mut fix = |p: &Path, keep: u32, should: u32, dir: bool| {
        // Not followed: a link is never changed, nor what it points to
        let m = match std::fs::symlink_metadata(p) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                out.left.push((p.to_path_buf(), e.to_string(), should));
                return;
            }
        };
        let kind_ok = if dir {
            m.file_type().is_dir()
        } else {
            m.file_type().is_file()
        };
        if !kind_ok {
            return;
        }
        if !dir {
            // Its real folder, links in the path resolved: inside the data folder, or not ours
            let inside = p
                .parent()
                .and_then(|d| d.canonicalize().ok())
                .is_some_and(|d| d.starts_with(&root));
            if !inside {
                return;
            }
        }
        let mode = m.permissions().mode() & 0o7777;
        let want = mode & keep;
        if want == mode {
            return;
        }
        match std::fs::set_permissions(p, std::fs::Permissions::from_mode(want)) {
            Ok(()) => out.changed.push((p.to_path_buf(), mode, want)),
            Err(e) => out
                .left
                .push((p.to_path_buf(), format!("mode {mode:o}: {e}"), should)),
        }
    };
    // The folder: no reading or writing by others; searching stays
    fix(&root, !0o066, 0o711, true);
    for f in files {
        fix(f, 0o7700, 0o600, false);
    }
    out
}

#[cfg(not(unix))]
pub fn tighten(_data_dir: &Path, _files: &[PathBuf]) -> Tightened {
    Tightened::default()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn files_with_secrets_become_the_nodes_alone() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let db = data.join("library.sqlite");
        let conf = data.join("node.toml");
        for f in [
            &db,
            &conf,
            &data.join("library.sqlite-wal"),
            &data.join("remote.key"),
        ] {
            std::fs::write(f, "x").unwrap();
            std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        let tunnel = data.join("tunnel");
        std::fs::create_dir(&tunnel).unwrap();
        std::fs::write(tunnel.join("domain"), "hs.example.org").unwrap();
        std::fs::set_permissions(tunnel.join("domain"), std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;

        let secret = data.join("pubky-session.secret");
        let more = [
            data.join("library.sqlite-journal"),
            secret.clone(),
            data.join("endpoint-keys.json"),
            data.join("node.restored.toml"),
        ];
        for f in &more {
            std::fs::write(f, "x").unwrap();
            std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        let changed = tighten(&data, &private_files(&data, &db, Some(&conf), &secret)).changed;
        for f in &more {
            assert_eq!(mode(f), 0o600, "{f:?}");
        }
        assert_eq!(mode(&data), 0o711);
        for f in [
            &db,
            &conf,
            &data.join("library.sqlite-wal"),
            &data.join("remote.key"),
        ] {
            assert_eq!(mode(f), 0o600, "{f:?}");
        }
        assert_eq!(changed.len(), 9, "{changed:?}");
        // What another service reads stays readable: the folder can be searched, the file is its own
        assert_eq!(mode(&tunnel.join("domain")), 0o644);
        // Already tight: nothing changes, nothing is logged; a missing file is skipped
        let again = tighten(&data, &private_files(&data, &db, Some(&conf), &secret));
        assert!(again.changed.is_empty() && again.left.is_empty(), "{again:?}");
    }

    /// A file the node can't look at or change is reported, never passed over in silence.
    #[test]
    fn what_the_node_cant_fix_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let locked = data.join("locked");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&locked).unwrap();
        let conf = locked.join("node.toml");
        std::fs::write(&conf, "x").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let out = tighten(&data, std::slice::from_ref(&conf));
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        // As root every file can be looked at; the check means something only for a service user
        if std::fs::metadata(&conf).is_ok() && out.left.is_empty() && running_as_root() {
            return;
        }
        assert_eq!(out.left.len(), 1, "{out:?}");
        assert_eq!(out.left[0].0, conf);
        assert_eq!(out.left[0].2, 0o600);
    }

    fn running_as_root() -> bool {
        std::fs::metadata("/root")
            .map(|m| std::os::unix::fs::MetadataExt::uid(&m) == 0)
            .unwrap_or(false)
            && std::fs::read_dir("/root").is_ok()
    }

    /// Never through a link, never outside the data folder: a link named like a secret file, a
    /// config kept in another folder and a folder in place of a file are left as they are.
    #[test]
    fn only_the_data_folders_own_files_are_changed() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let outside = dir.path().join("etc-node.toml");
        let target = dir.path().join("someone-elses");
        for f in [&outside, &target] {
            std::fs::write(f, "x").unwrap();
            std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        std::os::unix::fs::symlink(&target, data.join("remote.key")).unwrap();
        std::fs::create_dir(data.join("endpoint-keys.json")).unwrap();
        std::fs::set_permissions(
            data.join("endpoint-keys.json"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        // A folder inside reached through a link to outside: not the data folder's own
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("node.toml"), "x").unwrap();
        std::fs::set_permissions(
            elsewhere.join("node.toml"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        std::os::unix::fs::symlink(&elsewhere, data.join("conf")).unwrap();
        let mode = |p: &Path| std::fs::symlink_metadata(p).unwrap().permissions().mode() & 0o777;
        let db = data.join("library.sqlite");
        let files = private_files(&data, &db, Some(&outside), &data.join("pubky-session.secret"));
        let mut files = files;
        files.push(data.join("conf").join("node.toml"));
        let out = tighten(&data, &files);
        assert_eq!(mode(&target), 0o644, "a link's target is never changed");
        assert_eq!(
            mode(&outside),
            0o644,
            "a config outside the data folder is the installer's"
        );
        assert_eq!(mode(&data.join("endpoint-keys.json")), 0o755, "not a file");
        assert_eq!(
            mode(&elsewhere.join("node.toml")),
            0o644,
            "reached through a link"
        );
        assert!(
            out.changed
                .iter()
                .all(|(p, _, _)| p == &data.canonicalize().unwrap()),
            "{out:?}"
        );
    }
}
