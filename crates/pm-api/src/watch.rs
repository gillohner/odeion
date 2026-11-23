//! Automatic library pickup: watch the configured folders and rescan after
//! a quiet period, so anything another app drops into a shared folder (a
//! download client, a NAS sync) shows up without a manual scan.
//!
//! The debounce matters: a file being written changes size repeatedly; a
//! scan in the middle would index half a file or fail to probe it. Waiting
//! until no event arrived for `quiet` avoids most of that, and the scanner's
//! size+mtime check re-indexes anything that changed afterwards anyway.
//!
//! Only the library folders the changes are in are rescanned: a rescan looks
//! at every file of a folder, which over a network share takes long.

use crate::state::NodeState;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Changed paths kept per quiet period; past this every folder is rescanned.
const MAX_PATHS: usize = 4096;

/// Start watching `folders`; the returned watcher must be kept alive.
pub fn spawn(
    state: Arc<NodeState>,
    folders: Vec<PathBuf>,
    quiet: Duration,
) -> notify::Result<RecommendedWatcher> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<PathBuf>>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| match res {
        Ok(ev) if ev.kind.is_create() || ev.kind.is_modify() || ev.kind.is_remove() => {
            let _ = tx.send(ev.paths);
        }
        Ok(_) => {}
        Err(e) => tracing::warn!("folder watch: {e}"),
    })?;
    for f in &folders {
        match watcher.watch(f, RecursiveMode::Recursive) {
            Ok(()) => tracing::info!(folder = %f.display(), "watching for changes"),
            Err(e) => tracing::warn!(folder = %f.display(), "cannot watch: {e}"),
        }
    }
    tokio::spawn(async move {
        while let Some(first) = rx.recv().await {
            let mut changed = Changed::default();
            changed.add(first);
            // Drain until quiet.
            loop {
                match tokio::time::timeout(quiet, rx.recv()).await {
                    Ok(Some(paths)) => changed.add(paths),
                    Ok(None) => return,
                    Err(_) => break,
                }
            }
            let roots = changed.roots(state.all_folders());
            match state.start_scan(roots) {
                Ok(true) => tracing::info!("folder change: rescanning"),
                Ok(false) => tracing::debug!("folder change: scan already running"),
                Err(e) => tracing::warn!("folder change: {e}"),
            }
        }
    });
    Ok(watcher)
}

/// The paths changed in one quiet period.
#[derive(Default)]
struct Changed {
    paths: HashSet<PathBuf>,
    /// An event without a path, or more paths than are kept: every folder is rescanned.
    all: bool,
}

impl Changed {
    fn add(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            self.all = true;
        }
        if self.all {
            return;
        }
        self.paths.extend(paths);
        if self.paths.len() > MAX_PATHS {
            self.all = true;
            self.paths = HashSet::new();
        }
    }

    /// The library folders to rescan, in their order: the ones a changed path is in. A path in
    /// none of them (a platform that reports another form of the path) rescans them all, as
    /// before.
    fn roots(&self, folders: Vec<PathBuf>) -> Vec<PathBuf> {
        let under = |p: &Path| folders.iter().any(|f| p.starts_with(f));
        if self.all || !self.paths.iter().all(|p| under(p)) {
            return folders;
        }
        folders
            .iter()
            .filter(|f| self.paths.iter().any(|p| p.starts_with(f)))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changed(paths: &[&str]) -> Changed {
        let mut c = Changed::default();
        c.add(paths.iter().map(PathBuf::from).collect());
        c
    }

    fn folders() -> Vec<PathBuf> {
        ["/music/rock", "/music/jazz", "/nas/share"]
            .iter()
            .map(PathBuf::from)
            .collect()
    }

    #[test]
    fn only_the_folders_with_changes_are_rescanned() {
        let c = changed(&["/music/jazz/Album/01.flac", "/music/jazz/Album/02.flac"]);
        assert_eq!(c.roots(folders()), vec![PathBuf::from("/music/jazz")]);
        let c = changed(&["/nas/share/a.mp3", "/music/rock/b.mp3"]);
        assert_eq!(
            c.roots(folders()),
            vec![PathBuf::from("/music/rock"), PathBuf::from("/nas/share")]
        );
        // The folder itself (removed, renamed)
        assert_eq!(changed(&["/music/rock"]).roots(folders()), vec![PathBuf::from("/music/rock")]);
        // A name that only starts like a folder is not in it
        assert_eq!(changed(&["/music/rockabilly/x.mp3"]).roots(folders()), folders());
    }

    #[test]
    fn unknown_changes_rescan_every_folder() {
        // A path outside every folder (another form of it)
        assert_eq!(changed(&["/private/music/jazz/a.flac"]).roots(folders()), folders());
        // An event without a path
        let mut c = changed(&["/music/jazz/a.flac"]);
        c.add(Vec::new());
        assert_eq!(c.roots(folders()), folders());
        // More paths than are kept
        let mut c = Changed::default();
        c.add(
            (0..=MAX_PATHS)
                .map(|i| PathBuf::from(format!("/music/jazz/{i}.flac")))
                .collect(),
        );
        assert!(c.all && c.paths.is_empty());
        assert_eq!(c.roots(folders()), folders());
    }
}
