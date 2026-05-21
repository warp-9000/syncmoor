//! Debounced filesystem watcher for a single folder.
//!
//! Wraps `notify-debouncer-full` so the daemon sees one coarse
//! "something changed" signal per quiet window, instead of one event
//! per individual write.
//!
//! Critical filter: anything under `.git/` is ignored at the watcher
//! level. Our own commits write to `.git/index` etc.; without this
//! filter we'd see a self-triggered notification after every commit
//! and spam the daemon with wasted `git status` checks.

use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, FileIdMap};
use tokio::sync::mpsc::UnboundedSender;
use tracing::trace;

/// Owns the underlying notify debouncer; drop = stop watching.
pub struct FolderWatcher {
    _debouncer: Debouncer<notify::RecommendedWatcher, FileIdMap>,
    root: PathBuf,
}

impl std::fmt::Debug for FolderWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FolderWatcher")
            .field("root", &self.root)
            .finish()
    }
}

impl FolderWatcher {
    /// Start watching `root` recursively. Each debounced batch that
    /// contains at least one non-`.git/` path sends `()` on `tx`.
    /// Dropping the returned watcher stops the underlying threads.
    pub fn new(
        root: impl AsRef<Path>,
        debounce: Duration,
        tx: UnboundedSender<()>,
    ) -> Result<Self, notify::Error> {
        let root = root.as_ref().to_path_buf();
        let root_for_filter = root.clone();

        let mut debouncer = new_debouncer(debounce, None, move |result: DebounceEventResult| {
            match result {
                Ok(events) => {
                    let non_git = events.iter().any(|ev| {
                        ev.event
                            .paths
                            .iter()
                            .any(|p| !is_under_dot_git(&root_for_filter, p))
                    });
                    if non_git {
                        trace!(target: "gfs::watcher", "debounced batch: {} events", events.len());
                        // Channel closed = daemon has exited; ignore.
                        let _ = tx.send(());
                    } else {
                        trace!(target: "gfs::watcher", "all-.git/ batch, suppressed");
                    }
                }
                Err(errs) => {
                    for e in errs {
                        tracing::warn!(target: "gfs::watcher", "notify error: {e}");
                    }
                }
            }
        })?;

        debouncer.watcher().watch(&root, RecursiveMode::Recursive)?;

        Ok(Self {
            _debouncer: debouncer,
            root,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// True if `path` lies inside `<root>/.git/` (or is `<root>/.git`
/// itself). Used to suppress watcher noise from git's own writes.
fn is_under_dot_git(root: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    rel.components()
        .next()
        .is_some_and(|c| c.as_os_str() == ".git")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tokio::sync::mpsc::unbounded_channel;
    use tokio::time::{timeout, Duration as TokioDuration};

    /// notify uses native fs APIs and may take a moment to register
    /// changes on Windows. 5 s is generous for a CI runner.
    const RECV_TIMEOUT: TokioDuration = TokioDuration::from_secs(5);

    #[tokio::test(flavor = "current_thread")]
    async fn fires_on_file_write() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = unbounded_channel();

        let _w = FolderWatcher::new(dir.path(), Duration::from_millis(150), tx).unwrap();

        // Give the watcher a moment to initialise.
        tokio::time::sleep(TokioDuration::from_millis(200)).await;

        fs::write(dir.path().join("a.txt"), "hello\n").unwrap();

        let got = timeout(RECV_TIMEOUT, rx.recv())
            .await
            .expect("watcher did not fire within RECV_TIMEOUT");
        assert!(got.is_some(), "channel closed unexpectedly");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn suppresses_dot_git_only_writes() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();

        let (tx, mut rx) = unbounded_channel();
        let _w = FolderWatcher::new(dir.path(), Duration::from_millis(150), tx).unwrap();

        tokio::time::sleep(TokioDuration::from_millis(200)).await;

        // Write only inside .git/ — should NOT trigger.
        fs::write(
            dir.path().join(".git").join("HEAD"),
            "ref: refs/heads/main\n",
        )
        .unwrap();

        // Should time out (no event delivered).
        let got = timeout(TokioDuration::from_millis(750), rx.recv()).await;
        assert!(got.is_err(), "watcher fired for .git/-only batch: {got:?}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fires_when_mixed_with_dot_git() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();

        let (tx, mut rx) = unbounded_channel();
        let _w = FolderWatcher::new(dir.path(), Duration::from_millis(150), tx).unwrap();

        tokio::time::sleep(TokioDuration::from_millis(200)).await;

        // .git/ write + a real-tree write in the same window. Should fire.
        fs::write(dir.path().join(".git").join("HEAD"), "x\n").unwrap();
        fs::write(dir.path().join("real.txt"), "x\n").unwrap();

        let got = timeout(RECV_TIMEOUT, rx.recv()).await;
        assert!(
            got.is_ok() && got.unwrap().is_some(),
            "mixed batch suppressed"
        );
    }

    #[test]
    fn is_under_dot_git_detects_nested_paths() {
        let root = Path::new("/repo");
        assert!(is_under_dot_git(root, Path::new("/repo/.git")));
        assert!(is_under_dot_git(root, Path::new("/repo/.git/index")));
        assert!(is_under_dot_git(
            root,
            Path::new("/repo/.git/refs/heads/main")
        ));
        assert!(!is_under_dot_git(root, Path::new("/repo/src/main.rs")));
        assert!(!is_under_dot_git(root, Path::new("/other/.git/index")));
    }
}
