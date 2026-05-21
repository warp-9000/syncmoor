# Architecture

> A 30-minute read covering everything new contributors need before
> touching the daemon. Updated incrementally as Phases 1-2 land.

## 1. Big picture

```
┌─────────────────────────────────────────────────────────────────┐
│                          User                                  │
└────────────┬──────────────────────────────┬─────────────────────┘
             │                              │
             │ click tray menu              │ `gfs <cmd>`
             ▼                              ▼
   ┌────────────────────┐        ┌─────────────────────┐
   │  Tauri shell       │        │  gfs CLI (binary)   │
   │  (app/src-tauri)   │        │  (crates/gfs-cli)   │
   └─────────┬──────────┘        └─────────┬───────────┘
             │ tauri commands              │ UDS / named pipe
             ▼                              ▼
   ┌──────────────────────────────────────────────────┐
   │            gfs-core daemon (this process)        │
   │  ┌──────────────────────────────────────────┐    │
   │  │  Folder task #1  ─┐                      │    │
   │  │  Folder task #2  ─┼─ each is a tokio     │    │
   │  │  Folder task #N  ─┘   task running the   │    │
   │  │                       sync loop          │    │
   │  └──────────────────────────────────────────┘    │
   │  ▲       ▲              ▲                        │
   │  │       │              │                        │
   │ fs       timer       sleep/wake                  │
   │ events   tick        signal                      │
   └──┬──────────────────────────────────────────────┘
      │
      ▼ (per folder)
   ┌────────────────────────────┐
   │  gix (pure-Rust git)       │
   │  +                         │
   │  shelled `git pull --rebase│
   │  --autostash` fallback     │
   └─────────┬──────────────────┘
             ▼
      Remote (any git host)
```

## 2. Process model

- **Single daemon process** owns all folder tasks.
- The **Tauri app** in tray mode starts the daemon as a child process
  on first launch and keeps it alive across UI restarts.
- The **`gfs` CLI** connects to the existing daemon if one is running;
  if not, `gfs daemon start` launches it detached.
- One IPC endpoint, two transports:
  - **Linux/macOS**: Unix domain socket at
    `$XDG_RUNTIME_DIR/syncmoor.sock` (falls back to `/tmp/`).
  - **Windows**: named pipe `\\.\pipe\syncmoor`.

## 3. The per-folder sync loop

See plan.md §7 for the canonical pseudocode. Key invariants:

1. **Conflict-marker dominates.** If `<folder>/.git/gfs-conflict.json`
   exists, the loop emits `SyncStatus::Conflict` and *only* responds
   to the explicit `resolve` / `abort` IPC commands. No automatic
   action is taken on a conflicted folder.
2. **Watcher ignores `.git/` and the conflict marker.** Otherwise
   every internal git write would trigger a re-commit storm.
3. **Rebase, never merge.** We shell out to
   `git pull --rebase --autostash` because gix doesn't implement
   rebase as of mid-2026 (tracked in `gix` issue #1610).
4. **Push happens once per cycle** and only after fetch. If we're
   behind, we rebase first and push the new HEAD.

## 4. Why gix and not libgit2

- **No C dependency.** Cross-compilation is dramatically simpler; the
  Windows release does not require Visual Studio Build Tools at
  install time.
- **Pure-Rust TLS** via rustls keeps the dependency surface aligned
  with the rest of the workspace (we ban OpenSSL in `deny.toml`).
- **MIT/Apache-2.0 dual licensed**, matching our release.
- **Known gap**: rebase. We shell out for that one operation. When
  `gix` rebase lands we drop the shell call.

If a contributor proposes switching to `git2-rs`, they need to
present a concrete capability we cannot get from gix, AND a plan for
the link-time complexity on Windows + macOS universal builds.

## 5. State machine

```
                ┌───────────────────────────────────────────┐
                │                                           │
        ┌────► Idle ─►Watching ──► Committing ──► Pushing ──┘
        │       │         │                          │
        │       │         │                          ▼
        │       │         └────► Fetching ──► (behind?) ──► Pulling ──► Pushing
        │       │                                                            │
        │       ▼                                                            │
        │     Paused  ◄──── user "pause"                                     │
        │       │                                                            │
        │       └────► user "resume" ────────────────────────────────────────┘
        │
        └────── any state ────► Conflict (latch; only resolve/abort exit)
                              └► Error    (transient; auto-retry with backoff)
```

## 6. Persistence

Per-user SQLite DB at `<state dir>/state.sqlite`:

| Table | Purpose |
|---|---|
| `folders` | id, path, remote, branch, enabled, debounce_ms, pull_interval_sec |
| `events`  | append-only log of GitStep emissions (capped, oldest pruned) |
| `conflicts` | id, folder_id, sha_ours, sha_theirs, paths_json, opened_at, resolved_at |
| `daemon_meta` | schema_version, last_clean_shutdown, pid |

Config files (per-folder TOML) live separately under
`<config dir>/folders/`. The TOML is the source of truth; the DB is
runtime state.

## 7. Threading model

- One **tokio runtime** for the whole daemon.
- One **task per folder**: drives the watcher, the timer, and git ops.
- One **shared task** for IPC accept-loop.
- One **broadcast channel** for status events (UI and CLI both subscribe).
- Git operations use `spawn_blocking` for the gix/shell calls so they
  don't stall the watcher.

## 8. What's out of scope for v1

- Git LFS handling beyond "we don't break it" (large binaries still
  go through the same pipeline; performance is undefined).
- Submodules (we won't recurse).
- Auto-resolving conflicts with anything fancier than "ours" / "theirs"
  per-file.
- Hosted control plane / multi-user sharing. This is a local tool.
