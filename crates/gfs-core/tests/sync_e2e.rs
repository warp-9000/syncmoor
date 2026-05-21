//! End-to-end sync test: a real `Daemon::run` loop on a real git
//! working tree, against a real bare remote.
//!
//! Verifies the full Phase-1 contract: file write inside the watched
//! folder triggers a debounced commit-and-push that lands on the
//! remote within a bounded wait.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use gfs_core::{
    AuthMethod, CommitConfig, ConflictConfig, Daemon, FolderConfig, IgnoreConfig, RemoteConfig,
    SyncConfig,
};
use tempfile::TempDir;

/// Initialise a bare remote + a working clone with local user config.
/// Returns (bare_tempdir, clone_tempdir).
fn bare_and_clone() -> (TempDir, TempDir) {
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

    // Seed commit so `main` exists on the bare side.
    fs::write(clone_dir.path().join("README"), "seed\n").unwrap();
    for args in [
        vec!["-C", clone_dir.path().to_str().unwrap(), "add", "-A"],
        vec![
            "-C",
            clone_dir.path().to_str().unwrap(),
            "commit",
            "-m",
            "seed",
        ],
        vec![
            "-C",
            clone_dir.path().to_str().unwrap(),
            "push",
            "origin",
            "main",
        ],
    ] {
        Command::new("git").args(args).output().unwrap();
    }

    (bare, clone_dir)
}

fn folder_config(path: PathBuf) -> FolderConfig {
    FolderConfig {
        id: "01E2EE2E".into(),
        display_name: "e2e".into(),
        path: path.clone(),
        branch: Some("main".into()),
        enabled: true,
        // Short debounce so the test doesn't sit around for 5 s.
        sync: SyncConfig {
            debounce_ms: 250,
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

/// File-write inside the watched tree fires the daemon, which commits
/// and pushes within 10 s. After we shut the daemon down, a fresh
/// clone of the bare remote contains the file.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fs_write_triggers_commit_and_push() {
    let (bare, clone) = bare_and_clone();
    let cfg = folder_config(clone.path().to_path_buf());
    let daemon = Daemon::new(cfg.clone());
    let mut events = daemon.subscribe();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    // Spawn the daemon. The runtime tearing down at end of test drops it.
    let handle = tokio::spawn(async move { daemon.run(shutdown_rx).await });

    // Give the watcher a moment to come up.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Make a change.
    fs::write(clone.path().join("hello.txt"), "hi from e2e\n").unwrap();

    // Wait for a PushComplete event (with a generous timeout).
    let push_complete = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match events.recv().await {
                Ok(step) if step.phase == gfs_core::SyncPhase::PushComplete => return true,
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
    })
    .await;
    assert!(
        push_complete.is_ok() && push_complete.unwrap(),
        "daemon did not push within 10 s"
    );

    // Shut down the daemon cleanly.
    let _ = shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;

    // Verify on the bare side by cloning into a fresh dir.
    let verify = tempfile::tempdir().unwrap();
    Command::new("git")
        .args([
            "clone",
            bare.path().to_str().unwrap(),
            verify.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let landed = verify.path().join("hello.txt");
    assert!(
        landed.exists(),
        "hello.txt did not land on the bare remote at {}",
        landed.display()
    );
    let content = fs::read_to_string(&landed).unwrap();
    // Trim line endings to dodge Windows autocrlf normalization in CI.
    assert_eq!(content.trim_end(), "hi from e2e");
}

/// Two rapid writes coalesce into one commit (debounce works).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rapid_writes_coalesce_into_one_commit() {
    let (_bare, clone) = bare_and_clone();
    let cfg = folder_config(clone.path().to_path_buf());
    let daemon = Daemon::new(cfg.clone());
    let mut events = daemon.subscribe();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move { daemon.run(shutdown_rx).await });

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Burst of 5 writes, all within the 250 ms debounce window.
    for i in 0..5 {
        fs::write(clone.path().join(format!("burst-{i}.txt")), b"x\n").unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Wait for one PushComplete. Then wait a bit longer to see if a
    // SECOND commit cycle fires (it should NOT, since the repo is clean).
    let mut pushes = 0usize;
    let _ = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            match events.recv().await {
                Ok(step) if step.phase == gfs_core::SyncPhase::PushComplete => {
                    pushes += 1;
                    if pushes >= 1 {
                        // Allow a small grace period to catch any unexpected second cycle.
                        tokio::time::sleep(Duration::from_millis(800)).await;
                        return;
                    }
                }
                Ok(_) => {}
                Err(_) => return,
            }
        }
    })
    .await;

    assert_eq!(
        pushes, 1,
        "expected exactly one push (debounce coalesced); got {pushes}"
    );

    let _ = shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
}
