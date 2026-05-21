# syncmoor

> A cross-platform system-tray app that keeps a local folder continuously
> synced with a git remote — auto-commit on change, auto-pull on remote
> update, and a focused 3-way merge UI when (not if) the two sides disagree.

[![CI](https://github.com/warp-9000/syncmoor/actions/workflows/ci.yml/badge.svg)](https://github.com/warp-9000/syncmoor/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Built with Tauri](https://img.shields.io/badge/Built%20with-Tauri%202-24c8db.svg)](https://tauri.app)

---

> ⚠️ **Status: Phase 0 (scaffolding).** Not yet usable. See
> [Roadmap](#roadmap) and the development plan in
> `~/.copilot/session-state/<id>/plan.md`.

## Why

Every other open-source tool in this niche has at least one of these gaps:

| Tool | Gap |
|---|---|
| SparkleShare | EOL; Windows installer broken since 2018 |
| GitJournal/git-auto-sync | Headless CLI only — no GUI |
| obsidian-git | Best UX in class, but only inside Obsidian |
| Tether CLI | No Windows; dotfiles-only |
| gitomatic | Stale since 2020; no UI |
| simonthum/git-sync | Best sync logic, but bash-only |
| chezmoi / yadm | Wrong scope: manages a dotfile *store*, not an arbitrary folder |

`syncmoor` aims to be the boring, native, MIT-licensed tray app
that does the thing — for any local folder, on Windows / macOS / Linux.

## What it does

- **Watches** an arbitrary local git working tree with debounced
  filesystem events.
- **Auto-commits** changes on a configurable debounce window
  (default: 5 s after the last write).
- **Auto-pulls** the remote on a configurable interval (default: 5 min),
  plus on resume-from-sleep.
- **Rebases**, doesn't merge — so history stays linear and the daemon
  never silently produces "Merge branch 'main' of ..." commits.
- **Halts on conflict**, fires a system notification, and opens a
  dedicated 3-way merge UI on click. You see the conflict; the daemon
  doesn't guess.
- **Coexists** with manual `git` use — pause any folder, do whatever
  you want by hand, resume.

## What it does NOT do

- It is not a Dropbox replacement for binary files. Large/binary content
  belongs in `git-lfs` (or just Dropbox). See [docs/architecture.md](docs/architecture.md).
- It does not invent its own VCS. Your repo is a normal git repo;
  uninstalling `syncmoor` leaves it intact.
- It does not require a hosted service. Push to any git remote you can
  reach: GitHub, GitLab, Gitea, Forgejo, SSH, a USB drive — anything.

## Roadmap

Tracking [issue #1](https://github.com/warp-9000/syncmoor/issues/1).
Summary:

- [ ] **Phase 0** — Repo scaffold + CI + license hygiene *(in progress)*
- [ ] **Phase 1** — `gfs-core` minimum loop (status / commit / push)
- [ ] **Phase 2** — Pull + rebase + conflict detection
- [ ] **Phase 3** — `gfs-cli` + daemon IPC
- [ ] **Phase 4** — Tauri tray app + folder list view
- [ ] **Phase 5** — 3-way merge conflict resolver (the differentiator)
- [ ] **Phase 6** — History view, settings, autostart, toast notifications
- [ ] **Phase 7** — Signed `.msi`, `.dmg`, `.deb`, `.AppImage`; 0.1.0 release
- [ ] **Phase 8** — Homebrew tap, Scoop bucket, Flathub

## Install

> Not yet shipping binaries — these instructions are forward-looking.

| OS | Channel | Command |
|---|---|---|
| Windows | Scoop | `scoop install syncmoor` |
| Windows | winget | `winget install warp-9000.syncmoor` |
| macOS | Homebrew | `brew install warp-9000/tap/syncmoor` |
| Linux | AppImage | grab from [Releases](https://github.com/warp-9000/syncmoor/releases) |
| Linux | Flatpak | `flatpak install flathub com.warp-9000.gfs` |
| Linux | Debian/Ubuntu | `apt install syncmoor` (from PPA) |

## Building from source

Requires Rust 1.80+, Node 20+, and pnpm 9+.
On Linux you additionally need the Tauri 2 prerequisites: see
[tauri.app/start/prerequisites](https://tauri.app/start/prerequisites/).

```bash
git clone https://github.com/warp-9000/syncmoor
cd syncmoor
pnpm install
cargo run --bin gfs -- --help     # CLI
cargo tauri dev                   # GUI (dev mode)
cargo tauri build                 # release bundle
```

## Architecture

A 30-second tour:

- **`crates/gfs-core/`** — pure-Rust sync daemon library. No Tauri, no UI
  dependencies. The IPC channel is a stream of typed events
  (`SyncStatus`, `GitStep`, `SyncError`).
- **`crates/gfs-cli/`** — the `gfs` binary. Talks to the daemon over a
  Unix socket (Linux/macOS) or named pipe (Windows). Usable without
  the GUI installed.
- **`app/src-tauri/`** — Tauri 2 shell. Owns the tray icon, the main
  window, and the conflict-resolution view. Subscribes to the same IPC
  channel as the CLI.
- **`app/src/`** — Svelte frontend. Talks to `src-tauri` over Tauri's
  command bridge.

See [`docs/architecture.md`](docs/architecture.md) for the full picture
and [`docs/conflict-resolution.md`](docs/conflict-resolution.md) for the
3-way merge UI's design notes.

## Contributing

Issues and PRs welcome. Please read [`CONTRIBUTING.md`](CONTRIBUTING.md)
first — there's a license-allowlist enforced by `cargo-deny` in CI, and
some non-obvious decisions about why we don't use libgit2.

## License

MIT. See [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE) for attribution to
the projects whose ideas informed this one.
