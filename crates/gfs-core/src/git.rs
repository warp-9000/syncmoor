//! Shell-out wrappers around the `git` CLI.
//!
//! Phase 1 deliberately uses the system `git` binary for all operations
//! rather than gix. Rationale:
//!
//! 1. plan.md §7 already shells out for `git pull --rebase --autostash`
//!    because gix doesn't implement rebase. Extending that to all of
//!    Phase 1 means one code path, not two.
//! 2. The system `git` honours every config knob the user has set
//!    (signing, hooks, credential helpers, ssh keys via core.sshCommand,
//!    etc.) for free.
//! 3. gix's higher-level APIs (`add`, `commit`, `push`) are still
//!    maturing as of mid-2026. Selective migration to gix per-operation
//!    is a Phase 2+ task once the daemon loop is proven end-to-end.
//!
//! The downside is fork/exec cost (a few ms per call on Windows). At
//! the daemon's debounce cadence (commits ~once every 5 s by default)
//! this is negligible.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::status::SyncError;

/// Wrapper around `git` invocations rooted at a specific working tree.
#[derive(Debug, Clone)]
pub struct GitCmd {
    repo_path: PathBuf,
}

impl GitCmd {
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.repo_path
    }

    /// Run `git -C <repo> <args...>`. Returns stdout on exit-0,
    /// classifies non-zero exits into typed `SyncError` variants.
    fn run(&self, args: &[&str]) -> Result<String, SyncError> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.repo_path)
            .args(args)
            .output()
            .map_err(|e| SyncError::GitCmdFailed {
                exit: -1,
                stderr: format!("failed to spawn git: {e}"),
            })?;

        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }

        let exit = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(classify_git_error(exit, &stderr, args, &self.repo_path))
    }

    /// `git status --porcelain=v1` — empty string if clean.
    pub fn status_porcelain(&self) -> Result<String, SyncError> {
        self.run(&["status", "--porcelain=v1"])
    }

    /// True if there is anything to stage (untracked, modified, or deleted).
    pub fn has_changes(&self) -> Result<bool, SyncError> {
        Ok(!self.status_porcelain()?.trim().is_empty())
    }

    /// Count of changed paths reported by `status --porcelain`.
    pub fn change_count(&self) -> Result<usize, SyncError> {
        Ok(self
            .status_porcelain()?
            .lines()
            .filter(|l| !l.is_empty())
            .count())
    }

    /// `git add -A` — stage every change including deletions.
    pub fn add_all(&self) -> Result<(), SyncError> {
        self.run(&["add", "-A"])?;
        Ok(())
    }

    /// `git commit -m <msg>`. Returns the new HEAD SHA.
    pub fn commit(&self, message: &str) -> Result<String, SyncError> {
        self.run(&["commit", "-m", message])?;
        let sha = self.run(&["rev-parse", "HEAD"])?;
        Ok(sha.trim().to_owned())
    }

    /// Name of the currently checked-out branch (e.g. `main`).
    /// Returns `DetachedHead` if HEAD is detached.
    pub fn current_branch(&self) -> Result<String, SyncError> {
        let out = self.run(&["symbolic-ref", "--quiet", "--short", "HEAD"]);
        match out {
            Ok(name) => Ok(name.trim().to_owned()),
            Err(SyncError::GitCmdFailed { exit: 1, .. }) => Err(SyncError::DetachedHead),
            Err(e) => Err(e),
        }
    }

    /// `git push <remote> <branch>` — push the local branch to the named remote.
    pub fn push(&self, remote: &str, branch: &str) -> Result<(), SyncError> {
        self.run(&["push", remote, branch])?;
        Ok(())
    }

    /// `git fetch <remote>` — does NOT merge. Used in the pull phase
    /// (Phase 2) to refresh remote-tracking refs before deciding.
    pub fn fetch(&self, remote: &str) -> Result<(), SyncError> {
        self.run(&["fetch", remote])?;
        Ok(())
    }

    /// `git rev-parse HEAD` — current commit SHA.
    pub fn head_sha(&self) -> Result<String, SyncError> {
        Ok(self.run(&["rev-parse", "HEAD"])?.trim().to_owned())
    }
}

/// Map a non-zero git exit + stderr into the most specific `SyncError`
/// variant we can recognise. Phase 1 has best-effort heuristics; Phase
/// 2 will refine as we encounter real-world failure modes in the e2e
/// tests and on user machines.
fn classify_git_error(exit: i32, stderr: &str, args: &[&str], repo: &Path) -> SyncError {
    let s = stderr.to_ascii_lowercase();

    if s.contains("not a git repository") {
        return SyncError::NotAGitRepo(repo.to_path_buf());
    }
    if s.contains("repository '") && s.contains("' does not exist") {
        return SyncError::RepoNotFound(repo.to_path_buf());
    }
    // `git symbolic-ref HEAD` returns 1 on detached HEAD with stderr empty;
    // callers handle that case explicitly.
    if args.first() == Some(&"push") {
        if s.contains("rejected") || s.contains("non-fast-forward") {
            return SyncError::PushRejected(stderr.to_owned());
        }
        if s.contains("permission denied")
            || s.contains("authentication failed")
            || s.contains("could not read username")
        {
            return SyncError::AuthFailed {
                remote: args.get(1).copied().unwrap_or("?").to_owned(),
            };
        }
        if s.contains("could not resolve host")
            || s.contains("network is unreachable")
            || s.contains("connection timed out")
        {
            return SyncError::NetworkUnreachable(stderr.to_owned());
        }
    }

    SyncError::GitCmdFailed {
        exit,
        stderr: stderr.to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Tests — exercise against a real `git` binary in a tempdir.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    /// Helper: `git init -b main` a tempdir and set local user.{name,email}
    /// so commits work without a global git config.
    fn fresh_repo() -> (tempfile::TempDir, GitCmd) {
        let dir = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["-C", dir.path().to_str().unwrap(), "init", "-b", "main"])
            .output()
            .unwrap();
        Command::new("git")
            .args([
                "-C",
                dir.path().to_str().unwrap(),
                "config",
                "user.email",
                "test@example.invalid",
            ])
            .output()
            .unwrap();
        Command::new("git")
            .args([
                "-C",
                dir.path().to_str().unwrap(),
                "config",
                "user.name",
                "Test User",
            ])
            .output()
            .unwrap();
        let git = GitCmd::new(dir.path());
        (dir, git)
    }

    #[test]
    fn has_changes_false_on_clean_repo() {
        let (_dir, git) = fresh_repo();
        assert!(!git.has_changes().unwrap());
        assert_eq!(git.change_count().unwrap(), 0);
    }

    #[test]
    fn has_changes_true_after_adding_file() {
        let (dir, git) = fresh_repo();
        fs::write(dir.path().join("hello.txt"), "hi\n").unwrap();
        assert!(git.has_changes().unwrap());
        assert_eq!(git.change_count().unwrap(), 1);
    }

    #[test]
    fn add_all_then_commit_creates_a_commit() {
        let (dir, git) = fresh_repo();
        fs::write(dir.path().join("a.txt"), "alpha\n").unwrap();
        fs::write(dir.path().join("b.txt"), "beta\n").unwrap();
        git.add_all().unwrap();
        let sha = git.commit("test: initial").unwrap();
        assert_eq!(sha.len(), 40, "expected 40-char hex SHA, got {sha:?}");
        assert!(!git.has_changes().unwrap(), "should be clean post-commit");
    }

    #[test]
    fn current_branch_returns_main_after_init() {
        let (dir, git) = fresh_repo();
        // Need a commit before symbolic-ref returns "main" reliably on
        // some git versions when the branch hasn't been written yet.
        fs::write(dir.path().join("x.txt"), "x\n").unwrap();
        git.add_all().unwrap();
        git.commit("test: x").unwrap();
        assert_eq!(git.current_branch().unwrap(), "main");
    }

    #[test]
    fn not_a_git_repo_is_classified() {
        let dir = tempfile::tempdir().unwrap();
        let git = GitCmd::new(dir.path());
        let err = git.status_porcelain().unwrap_err();
        assert!(matches!(err, SyncError::NotAGitRepo(_)), "got {err:?}");
    }

    #[test]
    fn detached_head_is_classified() {
        let (dir, git) = fresh_repo();
        // Make a commit, then detach.
        fs::write(dir.path().join("a.txt"), "a\n").unwrap();
        git.add_all().unwrap();
        let sha = git.commit("a").unwrap();
        Command::new("git")
            .args([
                "-C",
                dir.path().to_str().unwrap(),
                "checkout",
                "--detach",
                &sha,
            ])
            .output()
            .unwrap();
        let err = git.current_branch().unwrap_err();
        assert!(matches!(err, SyncError::DetachedHead), "got {err:?}");
    }
}
