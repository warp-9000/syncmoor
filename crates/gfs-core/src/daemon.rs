//! Per-folder sync loop.
//!
//! Phase 1 scope (plan.md §13): status → commit → push. No pull yet.
//! No pause/resume IPC yet (Phase 3). The loop reads filesystem
//! events off the watcher channel and runs one `sync_once` cycle
//! per debounced batch.

use std::time::Duration;

use chrono::Utc;
use tokio::sync::{broadcast, mpsc};
use tracing::{info, instrument, warn};

use crate::config::FolderConfig;
use crate::git::GitCmd;
use crate::status::{GitStep, SyncError, SyncPhase};
use crate::watcher::FolderWatcher;

/// Capacity of the broadcast channel that surfaces `GitStep` events
/// to subscribers (UI / CLI / tests). Old events are dropped if a
/// subscriber lags — they're informational, not authoritative.
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Outcome of a single sync cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// Nothing changed. The repo was clean.
    Clean,
    /// Files were committed and pushed. The new HEAD SHA is included.
    Pushed { sha: String, changes: usize },
}

/// Run exactly one Phase-1 sync cycle (status → add → commit → push).
/// The function does NOT loop and does NOT watch the filesystem;
/// `Daemon::run` is responsible for that.
///
/// Emits one `GitStep` per phase on `events`, but never blocks on it
/// (channel full = subscriber lag; we proceed).
///
/// Phase 2 will extend this to fetch + rebase before push.
#[instrument(skip_all, fields(folder = %config.display_name))]
pub fn sync_once(
    config: &FolderConfig,
    git: &GitCmd,
    events: &broadcast::Sender<GitStep>,
) -> Result<SyncOutcome, SyncError> {
    let _ = events.send(GitStep::new(SyncPhase::StartCycle, "starting sync cycle"));

    let _ = events.send(GitStep::new(
        SyncPhase::CheckingStatus,
        "git status --porcelain",
    ));
    let changes = git.change_count()?;
    if changes == 0 {
        let _ = events.send(GitStep::new(SyncPhase::UpToDate, "no changes"));
        return Ok(SyncOutcome::Clean);
    }

    let _ = events.send(GitStep::new(
        SyncPhase::Staging,
        format!("staging {changes} change(s)"),
    ));
    git.add_all()?;

    let message = render_commit_message(config, changes);
    let _ = events.send(GitStep::new(
        SyncPhase::Committing,
        format!("committing: {message}"),
    ));
    let sha = git.commit(&message)?;
    let _ = events.send(GitStep::new(
        SyncPhase::CommitComplete,
        format!("commit {} ok", &sha[..sha.len().min(12)]),
    ));

    let branch = match config.branch.clone() {
        Some(b) => b,
        None => git.current_branch()?,
    };
    let remote = "origin"; // Phase 3 will resolve from config.remote.url.

    let _ = events.send(GitStep::new(
        SyncPhase::Pushing,
        format!("push {remote} {branch}"),
    ));
    git.push(remote, &branch)?;
    let _ = events.send(GitStep::new(SyncPhase::PushComplete, "push ok"));

    Ok(SyncOutcome::Pushed { sha, changes })
}

/// Render the commit message from the config's template. Phase 1
/// supports a small fixed set of placeholders; Phase 2+ will swap in
/// a real templating engine (tera is already in the workspace deps).
fn render_commit_message(config: &FolderConfig, change_count: usize) -> String {
    let utc = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut msg = config
        .commit
        .message_template
        .replace("{{utc_iso8601}}", &utc)
        .replace("{{folder_name}}", &config.display_name)
        .replace("{{change_count}}", &change_count.to_string());
    if config.commit.include_change_count && !msg.contains(&change_count.to_string()) {
        msg.push_str(&format!(
            " ({change_count} file{})",
            if change_count == 1 { "" } else { "s" }
        ));
    }
    msg
}

/// Long-running per-folder sync loop.
pub struct Daemon {
    config: FolderConfig,
    events: broadcast::Sender<GitStep>,
}

impl std::fmt::Debug for Daemon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Daemon")
            .field("config", &self.config.display_name)
            .finish()
    }
}

impl Daemon {
    pub fn new(config: FolderConfig) -> Self {
        let (tx, _rx) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self { config, events: tx }
    }

    /// Subscribe to `GitStep` events. Each call returns a fresh receiver.
    pub fn subscribe(&self) -> broadcast::Receiver<GitStep> {
        self.events.subscribe()
    }

    /// Run the watch + commit loop until `shutdown` resolves.
    ///
    /// Returns `Ok(())` on clean shutdown; otherwise the last error
    /// that escaped the cycle. Errors *inside* a cycle are logged and
    /// the loop continues — one bad cycle shouldn't kill the folder.
    pub async fn run(
        self,
        mut shutdown: tokio::sync::oneshot::Receiver<()>,
    ) -> Result<(), SyncError> {
        let git = GitCmd::new(&self.config.path);
        let (fs_tx, mut fs_rx) = mpsc::unbounded_channel();
        let _watcher = FolderWatcher::new(
            &self.config.path,
            Duration::from_millis(self.config.sync.debounce_ms),
            fs_tx,
        )
        .map_err(|e| SyncError::GitCmdFailed {
            exit: -1,
            stderr: format!("watcher init failed: {e}"),
        })?;

        info!(target: "gfs::daemon", folder = %self.config.display_name, "daemon started");

        loop {
            tokio::select! {
                biased;
                _ = &mut shutdown => {
                    info!(target: "gfs::daemon", folder = %self.config.display_name, "shutdown received");
                    return Ok(());
                }
                event = fs_rx.recv() => {
                    if event.is_none() {
                        warn!(target: "gfs::daemon", "watcher channel closed");
                        return Ok(());
                    }
                    match sync_once(&self.config, &git, &self.events) {
                        Ok(SyncOutcome::Clean) => {}
                        Ok(SyncOutcome::Pushed { sha, changes }) => {
                            info!(target: "gfs::daemon", sha=%&sha[..12], changes, "committed and pushed");
                        }
                        Err(e) => {
                            warn!(target: "gfs::daemon", error=?e, "sync cycle failed");
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    /// Build a `FolderConfig` pointing at `path` with safe Phase-1 defaults.
    fn folder_config(path: PathBuf) -> FolderConfig {
        FolderConfig {
            id: "01TESTTEST".into(),
            display_name: "test-folder".into(),
            path: path.clone(),
            branch: Some("main".into()),
            enabled: true,
            sync: SyncConfig {
                debounce_ms: 200,
                ..SyncConfig::default()
            },
            commit: CommitConfig::default(),
            conflict: ConflictConfig::default(),
            ignore: IgnoreConfig::default(),
            remote: RemoteConfig {
                url: "origin".into(),
                auth: AuthMethod::SshAgent,
            },
        }
    }

    /// Initialise a bare remote + a working clone with local user config.
    /// Returns (bare_tempdir, clone_tempdir).
    fn bare_and_clone() -> (tempfile::TempDir, tempfile::TempDir) {
        let bare = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init", "--bare", "-b", "main"])
            .current_dir(bare.path())
            .output()
            .unwrap();

        let clone_dir = tempfile::tempdir().unwrap();
        Command::new("git")
            .args([
                "clone",
                bare.path().to_str().unwrap(),
                clone_dir.path().to_str().unwrap(),
            ])
            .output()
            .unwrap();

        for (k, v) in [
            ("user.email", "test@example.invalid"),
            ("user.name", "Test User"),
        ] {
            Command::new("git")
                .args(["-C", clone_dir.path().to_str().unwrap(), "config", k, v])
                .output()
                .unwrap();
        }

        // Make an initial commit so `main` exists on the bare side.
        fs::write(clone_dir.path().join("README"), "seed\n").unwrap();
        Command::new("git")
            .args(["-C", clone_dir.path().to_str().unwrap(), "add", "-A"])
            .output()
            .unwrap();
        Command::new("git")
            .args([
                "-C",
                clone_dir.path().to_str().unwrap(),
                "commit",
                "-m",
                "seed",
            ])
            .output()
            .unwrap();
        Command::new("git")
            .args([
                "-C",
                clone_dir.path().to_str().unwrap(),
                "push",
                "origin",
                "main",
            ])
            .output()
            .unwrap();

        (bare, clone_dir)
    }

    #[test]
    fn sync_once_is_noop_when_clean() {
        let (_bare, clone) = bare_and_clone();
        let git = GitCmd::new(clone.path());
        let cfg = folder_config(clone.path().to_path_buf());
        let (tx, _rx) = broadcast::channel(8);
        let out = sync_once(&cfg, &git, &tx).unwrap();
        assert_eq!(out, SyncOutcome::Clean);
    }

    #[test]
    fn sync_once_commits_and_pushes_on_dirty_repo() {
        let (bare, clone) = bare_and_clone();
        let git = GitCmd::new(clone.path());
        let cfg = folder_config(clone.path().to_path_buf());

        // Dirty the working tree.
        fs::write(clone.path().join("hello.txt"), "hi\n").unwrap();
        fs::write(clone.path().join("subdir/world.txt"), "world\n").unwrap_or_else(|_| {
            fs::create_dir_all(clone.path().join("subdir")).unwrap();
            fs::write(clone.path().join("subdir/world.txt"), "world\n").unwrap();
        });

        let (tx, mut rx) = broadcast::channel(64);
        let out = sync_once(&cfg, &git, &tx).unwrap();

        let SyncOutcome::Pushed { sha, changes } = out else {
            panic!("expected Pushed, got {out:?}");
        };
        assert_eq!(sha.len(), 40);
        assert!(changes >= 2, "expected >=2 changes, got {changes}");

        // Confirm a stream of GitStep events came through.
        let mut phases = Vec::new();
        while let Ok(step) = rx.try_recv() {
            phases.push(step.phase);
        }
        assert!(phases.contains(&SyncPhase::Staging), "{phases:?}");
        assert!(phases.contains(&SyncPhase::Committing), "{phases:?}");
        assert!(phases.contains(&SyncPhase::PushComplete), "{phases:?}");

        // Verify the push actually landed on the bare side: clone again
        // and check the file exists.
        let verify = tempfile::tempdir().unwrap();
        Command::new("git")
            .args([
                "clone",
                bare.path().to_str().unwrap(),
                verify.path().to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            verify.path().join("hello.txt").exists(),
            "hello.txt missing on bare side"
        );
        assert!(
            verify.path().join("subdir/world.txt").exists(),
            "subdir/world.txt missing"
        );
    }

    #[test]
    fn render_commit_message_substitutes_placeholders() {
        let mut cfg = folder_config(PathBuf::from("/x"));
        cfg.commit.message_template =
            "autosync({{folder_name}}): {{change_count}} files at {{utc_iso8601}}".into();
        let msg = render_commit_message(&cfg, 3);
        assert!(msg.contains("autosync(test-folder)"), "{msg}");
        assert!(msg.contains("3 files"), "{msg}");
        assert!(
            msg.contains("T") && msg.contains("Z"),
            "missing iso8601 marker in {msg}"
        );
    }
}
