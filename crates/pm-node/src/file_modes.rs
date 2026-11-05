//! The node's data folder and the files in it that hold sign-ins and keys are its own: at start,
//! ones a group or anyone else may read are made the node's alone (each change is logged). The
//! folder keeps others' "search" permission, so another service that reads one public file in it
//! (the homeserver reads `tunnel/domain`) still can; nobody else can list or open what's inside.

use std::path::{Path, PathBuf};

/// The files [`tighten`] makes the node's alone: the library database and its journal files,
/// the config (it may hold the API token and service secrets) and the remote-access key.
pub fn private_files(data_dir: &Path, db: &Path, config: Option<&Path>) -> Vec<PathBuf> {
    let mut v = vec![db.to_path_buf()];
    for ext in ["-wal", "-shm"] {
        let mut p = db.as_os_str().to_owned();
        p.push(ext);
        v.push(PathBuf::from(p));
    }
    v.extend(config.map(Path::to_path_buf));
    v.push(data_dir.join("remote.key"));
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

/// Makes the data folder 0711 at most and each of `files` that exists 0600 at most.
#[cfg(unix)]
pub fn tighten(data_dir: &Path, files: &[PathBuf]) -> Tightened {
    use std::os::unix::fs::PermissionsExt;
    let mut out = Tightened::default();
    let mut fix = |p: &Path, keep: u32, should: u32| {
        let m = match std::fs::metadata(p) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                out.left.push((p.to_path_buf(), e.to_string(), should));
                return;
            }
        };
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
    fix(data_dir, !0o066, 0o711);
    for f in files {
        fix(f, 0o7700, 0o600);
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

        let changed = tighten(&data, &private_files(&data, &db, Some(&conf))).changed;
        assert_eq!(mode(&data), 0o711);
        for f in [
            &db,
            &conf,
            &data.join("library.sqlite-wal"),
            &data.join("remote.key"),
        ] {
            assert_eq!(mode(f), 0o600, "{f:?}");
        }
        assert_eq!(changed.len(), 5, "{changed:?}");
        // What another service reads stays readable: the folder can be searched, the file is its own
        assert_eq!(mode(&tunnel.join("domain")), 0o644);
        // Already tight: nothing changes, nothing is logged; a missing file is skipped
        let again = tighten(&data, &private_files(&data, &db, Some(&conf)));
        assert!(again.changed.is_empty() && again.left.is_empty(), "{again:?}");
    }

    /// A file the node can't look at or change is reported, never passed over in silence.
    #[test]
    fn what_the_node_cant_fix_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let locked = dir.path().join("locked");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&locked).unwrap();
        let conf = locked.join("node.toml");
        std::fs::write(&conf, "x").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let out = tighten(&data, &[conf.clone()]);
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
}
