//! `gfs` — headless command-line interface for SyncMoor.
//!
//! Phase 1: `gfs sync-now <path>` is functional and runs a single
//! commit-push cycle against an existing git working tree. The other
//! subcommands surface only their argument shapes and will be wired
//! to the daemon's IPC channel in Phase 3.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use gfs_core::{
    sync_once, AuthMethod, CommitConfig, ConflictConfig, FolderConfig, GitCmd, IgnoreConfig,
    RemoteConfig, SyncConfig, SyncOutcome,
};
use tokio::sync::broadcast;

/// Continuous git folder sync — headless CLI.
#[derive(Debug, Parser)]
#[command(name = "gfs", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Register a folder for continuous sync.
    Add {
        path: PathBuf,
        #[arg(long)]
        remote: String,
        #[arg(long)]
        branch: Option<String>,
    },
    /// List all registered folders.
    Ls,
    /// Print sync status for one or all folders.
    Status { folder: Option<String> },
    /// Pause sync on a folder (watcher disabled until resumed).
    Pause { folder: String },
    /// Resume a paused folder.
    Resume { folder: String },
    /// Force an immediate sync cycle on an existing working tree.
    /// In Phase 1 this runs the cycle in-process (no daemon required).
    #[command(name = "sync-now")]
    SyncNow {
        /// Path to the git working tree.
        path: PathBuf,
        /// Remote to push to (default: `origin`).
        #[arg(long, default_value = "origin")]
        remote: String,
        /// Branch to track. Default: whatever HEAD is on.
        #[arg(long)]
        branch: Option<String>,
        /// Override the commit message template.
        #[arg(long)]
        message: Option<String>,
    },
    /// List conflicted paths for one or all folders.
    Conflicts { folder: Option<String> },
    /// Non-interactively resolve a conflicted folder.
    Resolve {
        folder: String,
        #[arg(long, value_enum)]
        strategy: ResolveStrategy,
    },
    /// Manage the background daemon process.
    Daemon {
        #[command(subcommand)]
        op: DaemonOp,
    },
    /// Tail the log for one or all folders.
    Logs {
        #[arg(long)]
        follow: bool,
        folder: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum ResolveStrategy {
    Ours,
    Theirs,
}

#[derive(Debug, Subcommand)]
enum DaemonOp {
    Start,
    Stop,
    Restart,
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,gfs=debug".into()),
        )
        .init();

    let cli = Cli::parse();
    tracing::debug!(?cli, "parsed cli");

    match cli.cmd {
        Cmd::SyncNow {
            path,
            remote,
            branch,
            message,
        } => match run_sync_now(path, remote, branch, message) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("gfs: {e:#}");
                ExitCode::from(2)
            }
        },
        other => {
            eprintln!(
                "gfs: subcommand {other:?} is not implemented in Phase 1 — wires up in Phase 3."
            );
            ExitCode::from(64)
        }
    }
}

/// Drive one Phase-1 sync cycle on `path`. Returns `0` on success
/// (commit pushed OR clean tree).
fn run_sync_now(
    path: PathBuf,
    remote: String,
    branch: Option<String>,
    message_override: Option<String>,
) -> Result<ExitCode> {
    let path = path
        .canonicalize()
        .with_context(|| format!("path not found: {}", path.display()))?;
    let git = GitCmd::new(&path);

    let resolved_branch = match branch.clone() {
        Some(b) => b,
        None => git
            .current_branch()
            .context("could not determine current branch")?,
    };

    let mut cfg = FolderConfig {
        id: format!("adhoc:{}", path.display()),
        display_name: path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "adhoc".into()),
        path: path.clone(),
        branch: Some(resolved_branch.clone()),
        enabled: true,
        sync: SyncConfig::default(),
        commit: CommitConfig::default(),
        conflict: ConflictConfig::default(),
        ignore: IgnoreConfig::default(),
        remote: RemoteConfig {
            url: remote.clone(),
            auth: AuthMethod::SshAgent,
        },
    };
    if let Some(t) = message_override {
        cfg.commit.message_template = t;
    }

    let (tx, mut rx) = broadcast::channel::<gfs_core::GitStep>(64);

    let log_handle = std::thread::spawn(move || {
        while let Ok(step) = rx.blocking_recv() {
            eprintln!("[{:?}] {}", step.phase, step.message);
        }
    });

    let outcome = sync_once(&cfg, &git, &tx).context("sync cycle failed")?;
    drop(tx);
    let _ = log_handle.join();

    match outcome {
        SyncOutcome::Clean => {
            println!("up to date — nothing to commit");
            Ok(ExitCode::SUCCESS)
        }
        SyncOutcome::Pushed { sha, changes } => {
            println!(
                "pushed {} (commit {}, {} change{})",
                resolved_branch,
                &sha[..sha.len().min(12)],
                changes,
                if changes == 1 { "" } else { "s" },
            );
            Ok(ExitCode::SUCCESS)
        }
    }
}
