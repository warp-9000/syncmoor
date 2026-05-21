//! `gfs-core` — the headless daemon library powering the syncmoor
//! tray app and `gfs` CLI.
//!
//! Phase 0 ships the public type contracts only. The actual daemon loop,
//! filesystem watcher, and git operations land in Phases 1–2 (see
//! `~/.copilot/session-state/<id>/plan.md` §13).
//!
//! ## Module map (subject to Phase-1 fleshing)
//!
//! ```text
//! gfs_core
//! ├── status        // SyncStatus / SyncPhase / GitStep / SyncError  (§5)
//! ├── config        // per-folder TOML schema                         (§6)
//! ├── watcher       // notify-debouncer-full wrapper                  (§7)
//! ├── git/
//! │   ├── commit    // stage + commit via gix
//! │   ├── push      // push via gix
//! │   ├── pull      // shell-out to `git pull --rebase --autostash`
//! │   └── conflict  // detect / parse rebase conflicts
//! ├── daemon        // the per-folder tokio task                      (§7)
//! ├── state         // SQLite persistence
//! └── ipc           // event channel published to UI / CLI
//! ```

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(missing_debug_implementations)]

pub mod config;
pub mod daemon;
pub mod git;
pub mod status;
pub mod watcher;

// The following modules are stubbed out in Phase 0 and filled in later
// phases. Declaring them now keeps `pub use` paths stable.
//
// pub mod state;
// pub mod ipc;

pub use config::{
    AuthMethod, CommitConfig, CommitStrategy, ConflictConfig, ConflictMode, FolderConfig,
    IgnoreConfig, RemoteConfig, SyncConfig,
};
pub use daemon::{sync_once, Daemon, SyncOutcome};
pub use git::GitCmd;
pub use status::{GitStep, SyncError, SyncPhase, SyncStatus};
pub use watcher::FolderWatcher;
