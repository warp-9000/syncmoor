//! Per-folder TOML configuration (plan.md §6).
//!
//! Loaded from `<config dir>/folders/<id>.toml`. Source of truth at
//! rest; the SQLite state DB is for runtime state only.
//!
//! Phase 1 keeps this module pure: just types + serde. Loading,
//! validation, and on-disk layout land in Phase 3 when the daemon
//! gains real config discovery.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Top-level per-folder config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderConfig {
    /// ULID identifying the folder across machines (stable across renames).
    pub id: String,

    /// Human-readable label shown in the UI and CLI.
    pub display_name: String,

    /// Absolute path to the working tree.
    pub path: PathBuf,

    /// Branch to track. `None` = follow whatever branch is currently
    /// checked out (re-read on every cycle).
    #[serde(default)]
    pub branch: Option<String>,

    /// Whether the daemon should actively sync this folder. `false`
    /// is a soft pause that survives daemon restarts.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    #[serde(default)]
    pub sync: SyncConfig,

    #[serde(default)]
    pub commit: CommitConfig,

    #[serde(default)]
    pub conflict: ConflictConfig,

    #[serde(default)]
    pub ignore: IgnoreConfig,

    pub remote: RemoteConfig,
}

fn default_enabled() -> bool {
    true
}

/// `[sync]` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    /// Filesystem-event debounce window. Commits trigger after this
    /// many milliseconds of quiet following the last write.
    pub debounce_ms: u64,

    /// How often to pull remote changes (in seconds), independent of
    /// filesystem activity.
    pub pull_interval_sec: u64,

    /// Trigger a pull when the OS notifies us of wake-from-sleep.
    pub sync_on_resume_from_sleep: bool,

    /// Pause sync while running on battery (saves disk + network).
    pub pause_on_battery: bool,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            debounce_ms: 5_000,
            pull_interval_sec: 300,
            sync_on_resume_from_sleep: true,
            pause_on_battery: false,
        }
    }
}

/// `[commit]` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitConfig {
    /// When to fire a commit cycle.
    #[serde(default)]
    pub strategy: CommitStrategy,

    /// For `strategy = "fixed_interval"`, the period in seconds.
    pub fixed_interval_sec: u64,

    /// Tera template for the commit message.
    ///
    /// Available variables: `utc_iso8601`, `local_iso8601`, `hostname`,
    /// `change_count`, `folder_name`. See `daemon::render_commit_message`.
    pub message_template: String,

    /// Append `(N files changed)` to the commit subject.
    pub include_change_count: bool,
}

impl Default for CommitConfig {
    fn default() -> Self {
        Self {
            strategy: CommitStrategy::default(),
            fixed_interval_sec: 600,
            message_template: "autosync: {{utc_iso8601}}".into(),
            include_change_count: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitStrategy {
    /// Commit `debounce_ms` after the last filesystem change.
    DebouncedFileChange,
    /// Commit every `fixed_interval_sec` regardless of activity.
    FixedInterval,
}

impl Default for CommitStrategy {
    fn default() -> Self {
        Self::DebouncedFileChange
    }
}

/// `[conflict]` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictConfig {
    #[serde(default)]
    pub mode: ConflictMode,

    /// Pop a system toast when a conflict halts the loop.
    pub notify_via_toast: bool,
}

impl Default for ConflictConfig {
    fn default() -> Self {
        Self {
            mode: ConflictMode::default(),
            notify_via_toast: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictMode {
    /// Default — pause the folder, write a conflict marker, surface
    /// the 3-way merge UI.
    HaltAndNotify,
    /// Auto-resolve by keeping the local side. Explicit opt-in.
    AutoKeepLocal,
    /// Auto-resolve by keeping the remote side. Explicit opt-in.
    AutoKeepRemote,
}

impl Default for ConflictMode {
    fn default() -> Self {
        Self::HaltAndNotify
    }
}

/// `[ignore]` section. Patterns in addition to the repo's `.gitignore` —
/// never auto-staged even if the user has not added them to `.gitignore`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IgnoreConfig {
    pub patterns: Vec<String>,
}

impl Default for IgnoreConfig {
    fn default() -> Self {
        Self {
            patterns: vec![
                "*.swp".into(),
                "*.tmp".into(),
                ".DS_Store".into(),
                "Thumbs.db".into(),
            ],
        }
    }
}

/// `[remote]` section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteConfig {
    /// Remote URL (e.g. `git@github.com:user/dotfiles.git`).
    pub url: String,

    /// Which credential helper to consult.
    #[serde(default)]
    pub auth: AuthMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMethod {
    SshAgent,
    GitCredentialManager,
    GhCli,
    Pat,
}

impl Default for AuthMethod {
    fn default() -> Self {
        Self::SshAgent
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_TOML: &str = r#"
id = "01HXYZABCDEFGHJKMNPQRSTVWX"
display_name = "dotfiles"
path = "/home/peter/dotfiles"
branch = "main"

[remote]
url = "git@github.com:peter/dotfiles.git"
"#;

    #[test]
    fn parses_minimum_toml_with_all_defaults() {
        let cfg: FolderConfig = toml::from_str(SAMPLE_TOML).unwrap();
        assert_eq!(cfg.display_name, "dotfiles");
        assert_eq!(cfg.branch.as_deref(), Some("main"));
        assert!(cfg.enabled);
        assert_eq!(cfg.sync.debounce_ms, 5_000);
        assert_eq!(cfg.sync.pull_interval_sec, 300);
        assert_eq!(cfg.commit.strategy, CommitStrategy::DebouncedFileChange);
        assert_eq!(cfg.conflict.mode, ConflictMode::HaltAndNotify);
        assert!(cfg.ignore.patterns.iter().any(|p| p == ".DS_Store"));
        assert_eq!(cfg.remote.auth, AuthMethod::SshAgent);
    }

    #[test]
    fn round_trips_via_toml() {
        let cfg: FolderConfig = toml::from_str(SAMPLE_TOML).unwrap();
        let serialized = toml::to_string(&cfg).unwrap();
        let reparsed: FolderConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(reparsed.id, cfg.id);
        assert_eq!(reparsed.path, cfg.path);
        assert_eq!(reparsed.sync.debounce_ms, cfg.sync.debounce_ms);
    }

    #[test]
    fn null_branch_means_follow_current() {
        let toml_src = r#"
id = "01ABC"
display_name = "x"
path = "/x"

[remote]
url = "git@x:x/x.git"
"#;
        let cfg: FolderConfig = toml::from_str(toml_src).unwrap();
        assert!(cfg.branch.is_none());
    }

    #[test]
    fn commit_strategy_serializes_snake_case() {
        let cfg = CommitConfig::default();
        let s = toml::to_string(&cfg).unwrap();
        assert!(s.contains("debounced_file_change"));
    }

    #[test]
    fn auth_method_serializes_kebab_case() {
        let cfg = RemoteConfig {
            url: "x".into(),
            auth: AuthMethod::GitCredentialManager,
        };
        let s = toml::to_string(&cfg).unwrap();
        assert!(s.contains("git-credential-manager"));
    }
}
