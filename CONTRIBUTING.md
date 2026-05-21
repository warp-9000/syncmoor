# Contributing to syncmoor

Thanks for considering a contribution. Read this once; it'll save us both
some back-and-forth.

## Ground rules

1. **License compatibility is non-negotiable.** This project is MIT.
   Direct dependencies must be MIT, Apache-2.0, BSD-2-Clause,
   BSD-3-Clause, ISC, MPL-2.0, or CC0. The allowlist is enforced by
   `cargo-deny` in CI (see `deny.toml`). If a PR adds a copyleft
   dependency it will fail CI and we will not merge it.
2. **No copy-paste from copyleft projects.** SparkleShare in particular
   is LGPLv3 — borrow architectural concepts (enum names, event shapes)
   only, never code. See `NOTICE` for the list of projects we have
   intentionally only studied.
3. **Conventional commit messages**, single-line subject under 72
   chars, imperative mood: `feat:`, `fix:`, `docs:`, `refactor:`,
   `test:`, `ci:`, `build:`, `chore:`. Subjects that don't parse get
   reformatted at merge time.
4. **Rebase, don't merge.** This project's whole reason for existing is
   linear history. Keep PRs rebased on `main`.

## Dev loop

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test  --workspace
cargo deny check
pnpm -C app run lint
pnpm -C app run check
```

CI runs all of the above on Windows, macOS, and Linux. If `cargo deny
check` fails locally and you don't understand why, look at the
`[licenses]` section of `deny.toml` — chances are something pulled in a
new transitive dep that needs explicit allowing.

## Filing issues

- **Bug**: include OS, Tauri/Rust versions (`cargo --version`,
  `cargo tauri --version`), the contents of the affected folder's
  config TOML (redact remotes if private), and what the tray icon shows.
- **Feature request**: explain the workflow first, the implementation
  second. We are not going to merge "add a setting for X" without
  understanding the scenario.

## Code review

- For larger changes (anything that touches `daemon.rs`,
  `conflict.rs`, or the IPC layer), open a draft PR with the design
  first. A 200-line refactor we can't undo is harder to land than a
  paragraph of prose.
- For UI changes, include a before/after screenshot or a short
  screen-capture.

## Releases (maintainers)

- Tag `v0.x.y`. The `release.yml` workflow builds Windows/macOS/Linux
  artifacts and attaches them to a GitHub Release.
- Update `CHANGELOG.md` *before* tagging, following Keep a Changelog
  format.

## Security

Please don't file security issues as public bug reports. See
[`SECURITY.md`](SECURITY.md) for the disclosure process. *(That file
will land in Phase 7.)*
