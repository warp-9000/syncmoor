# Conflict resolution UX

The single screen that no other open-source tool in this niche has,
and the reason this project exists. This document is the design contract
the Phase-5 implementation has to satisfy.

## 1. What counts as a conflict here

A "conflict" in `syncmoor` is precisely a **rebase conflict**:
`git pull --rebase --autostash` was attempted, autostash didn't pick
up all dirty state, or `git` reported one or more paths with
non-trivially mergeable hunks.

Three things matter:

1. The daemon **never silently picks a side**. Auto-resolve modes
   exist (`auto_keep_local`, `auto_keep_remote`) but they are explicit
   per-folder opt-ins, off by default.
2. On conflict the daemon **aborts the rebase** (`git rebase --abort`)
   so the working tree is back to the pre-pull state. A
   `gfs-conflict.json` marker is written at
   `<folder>/.git/gfs-conflict.json` recording the failed-rebase SHAs
   and the conflicted paths.
3. The daemon **stops touching the folder** until the user resolves
   it. The tray icon goes red. A system toast says "Conflict in
   `<name>` — click to resolve".

## 2. The conflict view

Opened by clicking the toast, by clicking a Conflict-state folder in
the main window, or by `gfs conflicts <folder>` in the CLI (which
emits a path + a one-line summary per conflicted file, JSON or text).

### Layout

```
┌─────────────────────────────────────────────────────────────────┐
│  ◀  Conflict — dotfiles                              ✕  Abort   │
├──────────────────────┬──────────────────────────────────────────┤
│ Conflicted files (3) │  src/auth/login.ts                       │
│                      │  ┌──────────┬──────────┬──────────┐      │
│ ► src/auth/login.ts  │  │  Ours    │  Base    │ Theirs   │      │
│   README.md          │  │ (your    │ (common  │ (remote) │      │
│   package.json       │  │  side)   │ ancestor)│          │      │
│                      │  │          │          │          │      │
│                      │  │ ...diff content with 3-pane    │      │
│                      │  │ side-by-side rendering...      │      │
│                      │  │                                 │      │
│                      │  └──────────┴──────────┴──────────┘      │
│                      │                                          │
│                      │  ┌──────────────────────────────────────┐│
│                      │  │ Keep mine | Keep theirs | Open in    ││
│                      │  │ $EDITOR                              ││
│                      │  └──────────────────────────────────────┘│
│                      │                                          │
├──────────────────────┴──────────────────────────────────────────┤
│                       [ Continue rebase ]  (only when all resolved) │
└─────────────────────────────────────────────────────────────────┘
```

### Components

- **Left pane**: simple list of conflicted paths. Status badge per
  row: ⚠️ unresolved · ✓ resolved (kept) · ✓ resolved (edited).
- **Right pane**: 3-way diff using `codemirror-merge` (MIT) or the
  Monaco diff editor wrapper (MIT). Both render the
  Ours / Base / Theirs columns natively.
- **Per-file toolbar**:
  - **Keep mine** — write the "ours" blob into the file and mark as
    resolved.
  - **Keep theirs** — write the "theirs" blob and mark as resolved.
  - **Open in $EDITOR** — launch the user's `$EDITOR` (or
    `code --wait` if VSCode is detected, or `notepad` on Windows
    fallback). After the editor closes, re-evaluate conflict markers
    in the file; if none remain, mark resolved.
- **Global toolbar**:
  - **Continue rebase** — enabled only when every file is resolved.
    Stages the resolved files, runs `git rebase --continue`. On
    success: delete `gfs-conflict.json`, drop status back to
    `Watching`, push (because we were ahead before the rebase).
  - **Abort** — runs `git rebase --abort` (no-op if rebase has
    already been aborted by the daemon, which is the common case),
    deletes `gfs-conflict.json`, drops status to `Watching`. User
    keeps whatever was in their working tree pre-pull.

## 3. Why this matters

Every other tool in the niche either:

- **silently resolves with one side**, which loses data,
- **dumps the user into a CLI rebase**, which scares non-CLI users, or
- **does nothing and leaves a mystery red icon**, which is the
  current SparkleShare-on-Windows experience.

We do the third option deliberately, but we put the resolution UX
*one click away* from the red icon. That's the entire UX bet.

## 4. Edge cases the view must handle

- **Binary file conflicts**: 3-way diff shows file metadata only and
  exposes "Keep mine" / "Keep theirs" buttons. No textual diff.
- **Both sides deleted vs one side modified**: render as a synthetic
  "vs deleted" pane; resolution buttons re-add or stay deleted.
- **Conflict where one side is a rename**: detect via `git diff
  --find-renames` and render with rename arrows in the file list.
- **Symlink conflicts**: rare but possible on Linux/macOS; treat as
  binary.
- **User edits a file in their editor while the conflict view is
  open**: notify-rs picks it up; if the change resolves the
  conflict (no `<<<<<<<` markers remain) we automatically mark
  the file resolved.

## 5. CLI counterpart

Same operations, headless:

```
gfs conflicts <folder>                    # list, JSON or text
gfs resolve <folder> --strategy ours
gfs resolve <folder> --strategy theirs    # not interactive; bulk apply
gfs resolve <folder> --continue           # alias for the GUI button
gfs resolve <folder> --abort
```

A scriptable resolution path matters for users who want to put
`gfs resolve --strategy theirs` in a "I just want my dotfiles to
match the laptop" wrapper script.

## 6. Test matrix for v1.0 (definition of done)

The Phase-5 PR must include automated tests for each of these on at
least Linux:

1. Conflicting line in same file, text. Keep mine → continue → push succeeds.
2. Conflicting line in same file, text. Keep theirs → continue → push succeeds.
3. Conflicting line in same file, text. Open in $EDITOR, resolve manually.
4. Binary file conflict. Keep mine.
5. Both sides modified, then both reverted before continue → empty diff
   continue is a no-op.
6. Rename vs edit conflict.
7. Add-add conflict (same path created on both sides).
8. Delete vs modify conflict.
9. Three conflicting files, mixed strategies.
10. User edits a file outside the GUI to resolve it, then clicks
    Continue — verify automatic re-detection.
