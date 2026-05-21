//! `gfs` — headless command-line interface for syncmoor.
//!
//! Phase 0 ships only the argument-parser surface area defined in
//! plan.md §9. Subcommands return `unimplemented` until Phase 3 wires
//! them to the daemon's IPC channel.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

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
        /// Local working tree path.
        path: PathBuf,
        /// Remote URL (e.g. git@github.com:you/repo.git).
        #[arg(long)]
        remote: String,
        /// Branch to track. Defaults to the currently checked-out branch.
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
    /// Force an immediate sync cycle.
    #[command(name = "sync-now")]
    SyncNow { folder: String },
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

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    tracing::debug!(?cli, "parsed cli");

    // Phase-0 placeholder: every subcommand prints a "not yet" message.
    // Phase 3 replaces the body with IPC calls into the daemon.
    eprintln!(
        "gfs: subcommand {:?} is not implemented yet — see plan.md Phase 3.",
        cli.cmd
    );
    std::process::exit(64);
}
