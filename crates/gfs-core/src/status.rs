//! Public status / progress / error contracts for the sync daemon.
//!
//! These are the types the UI and CLI subscribe to. Stability matters:
//! breaking changes here ripple to every IPC consumer.
//!
//! Design references (architecture only — no code copied):
//!
//! * SparkleShare's `SyncStatus` / `ErrorStatus` enum vocabulary.
//! * `tiddly-gittly/git-sync-js`'s 25-step `GitStep` progress model
//!   and typed-error-per-failure-mode approach.

use std::path::PathBuf;

use serde::Serialize;
use thiserror::Error;

/// High-level state surfaced to the tray icon. Exactly one value at a
/// time per folder. The aggregate tray icon picks the "worst" state
/// across all folders (Conflict > Error > Syncing > Watching > Idle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    /// Daemon up, no folders configured.
    Idle,
    /// Watching files, nothing in flight.
    Watching,
    /// Staging + committing local changes.
    Committing,
    /// Pushing to remote.
    Pushing,
    /// Fetching from remote (no merge yet).
    Fetching,
    /// Applying remote changes via rebase.
    Pulling,
    /// User-initiated pause; watcher is disabled.
    Paused,
    /// A rebase conflict needs manual resolution.
    Conflict,
    /// Unrecoverable error — see the latest `SyncError`.
    Error,
}

/// Fine-grained progress within a sync cycle. Useful for the "what is
/// it doing right now" tooltip and for tests that assert the exact
/// step ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncPhase {
    StartCycle,
    CheckingStatus,
    Staging,
    Committing,
    CommitComplete,
    Fetching,
    FetchComplete,
    BehindRemote,
    AheadOfRemote,
    Diverged,
    Rebasing,
    RebaseComplete,
    Pushing,
    PushComplete,
    UpToDate,
    ConflictDetected,
    Paused,
}

/// A single emission on the daemon's progress stream.
#[derive(Debug, Clone, Serialize)]
pub struct GitStep {
    pub phase: SyncPhase,
    pub message: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl GitStep {
    pub fn new(phase: SyncPhase, message: impl Into<String>) -> Self {
        Self {
            phase,
            message: message.into(),
            timestamp: chrono::Utc::now(),
        }
    }
}

/// Every failure mode the daemon can hit. The UI uses these to decide
/// what (if any) toast or modal to show; the CLI surfaces them as
/// exit codes.
#[derive(Debug, Clone, Error, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyncError {
    #[error("Repository not found at {0}")]
    RepoNotFound(PathBuf),

    #[error("Not a git working tree: {0}")]
    NotAGitRepo(PathBuf),

    #[error("HEAD is detached - sync requires a checked-out branch")]
    DetachedHead,

    #[error("Authentication failed for remote {remote}")]
    AuthFailed { remote: String },

    #[error("Network unreachable: {0}")]
    NetworkUnreachable(String),

    #[error("Rebase conflict - manual resolution required")]
    RebaseConflict {
        conflicted_paths: Vec<PathBuf>,
        ours_sha: String,
        theirs_sha: String,
    },

    #[error("Working tree dirty in a way auto-sync cannot resolve")]
    DirtyWorkingTree { paths: Vec<PathBuf> },

    #[error("Push rejected: {0}")]
    PushRejected(String),

    #[error("Diverged history, autoresolve disabled by config")]
    DivergedWithAutoresolveOff,

    #[error("Git command failed ({exit}): {stderr}")]
    GitCmdFailed { exit: i32, stderr: String },
}

// ---------------------------------------------------------------------------
// Sanity tests — these run on `cargo test -p gfs-core` and only check
// that the contracts round-trip through serde without surprises.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_status_serializes_snake_case() {
        let json = serde_json::to_string(&SyncStatus::Conflict).unwrap();
        assert_eq!(json, "\"conflict\"");
    }

    #[test]
    fn sync_phase_serializes_snake_case() {
        let json = serde_json::to_string(&SyncPhase::ConflictDetected).unwrap();
        assert_eq!(json, "\"conflict_detected\"");
    }

    #[test]
    fn git_step_includes_timestamp() {
        let step = GitStep::new(SyncPhase::Staging, "staged 3 files");
        let json = serde_json::to_value(&step).unwrap();
        assert_eq!(json["phase"], "staging");
        assert_eq!(json["message"], "staged 3 files");
        assert!(json["timestamp"].is_string());
    }

    #[test]
    fn sync_error_tags_kind() {
        let err = SyncError::AuthFailed {
            remote: "origin".into(),
        };
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["kind"], "auth_failed");
        assert_eq!(json["remote"], "origin");
    }

    #[test]
    fn sync_error_display_round_trip() {
        let err = SyncError::PushRejected("non-fast-forward".into());
        assert_eq!(err.to_string(), "Push rejected: non-fast-forward");
    }
}
