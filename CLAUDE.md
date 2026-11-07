# CLAUDE.md

**Quartermaster** (command `qm`) tracks every Claude Code session automatically
(hooks → JSONL log) and curates them into work items (markdown files with
state/notes) that the user manages from a terminal viewer, the `qm gui` web UI and a
phone page. Ruby executable `./quartermaster` (a realpath-resolving launcher; code in
`lib/quartermaster/`), symlinked from `~/bin/quartermaster` and `~/bin/qm` — the
symlinks and hook registrations point at it, so never rename or move it. A Rust port
(`src/`) replaces it area by area, falling back to Ruby → `docs/areas/rust-port.md`.

## Where things live

- **Docs:** `docs/README.md` indexes `overview.md`, `rejected.md` and one page per area
  (`docs/areas/*.md`): its `**Code:**` line names the files it owns, `## Rules` the constraints.
  Read it before touching that code and update it in the same commit — rewrite, don't append;
  why goes in the commit message.
- **Data:** `~/.claude/quartermaster/`, overridden by `QM_ROOT` (tests always do). Every file
  in it and its owning page → `docs/overview.md`.
- **GUI page scripts are classic `<script>` tags in load order, not ES modules**: shared
  globals, inline `onclick=`; load-time calls only into earlier files, `events.js` last, a new
  asset needs its tag in page.html → `docs/areas/gui-shell.md`.
- Hooks live in user-level `~/.claude/settings.json` → `docs/areas/capture.md`.

## Hard constraints

One line each; reasons are on the page pointed to.

**Across every area**
- **Stdlib only, no gems; Ruby 3.3 syntax floor**: asdf shims resolve `env ruby` per cwd, so a
  hook would die on a missing gem or newer syntax → `docs/overview.md`.
- **Except `statusline/statusline.rb`: macOS system Ruby 2.6, standalone, never loads `lib/`**,
  must never break → `docs/areas/side-panel.md`.
- **Hooks never block or break a session**: rescue everything, log to `qm.log`, exit 0. Only
  SessionStart prints (its stdout is Claude's context).
- **Per-tool-call hooks are plain POSIX `sh`**: never print, always exit 0, honor `QM_SKIP`,
  only append → `docs/areas/capture.md`.
- **External tools that change state go through `CommandRunner`**; tests pass a stub
  (`quiet_runner` for a GUI): no real server, IDE, worktree or notification.
- **Tests never touch the user's live state**: `QM_ROOT`, `QM_TMUX_SOCKET=qm-test-…` (kill that
  server after), never `tmux -L qm`.
- **Data formats carry `v:`** (JSONL events, work-item frontmatter): bump on a breaking change,
  readers tolerate unknown fields.
- **A port changes no behavior**: output, exit codes, file bytes match Ruby, quirks included;
  ported once `tests/` pins it under `cargo test` (Ruby) and `QM_BIN=$PWD/target/debug/qm
  cargo test` (Rust) → `docs/areas/rust-port.md`.
- **Rust edits user files minimally**: the changed key's lines only, flocked, atomic; what it
  can't write like Psych goes to Ruby → same page.
- **`QM_ASIDE` beats every linking rule**: an aside is never attached, logged as attention or
  distilled; any new per-session surface honors it → `docs/areas/asides.md`.

**The user's data, sessions and machine**
- **`qm install-hooks` prints, never writes** `~/.claude/settings.json` — that file is the user's.
- **Append-only files stay append-only**: `sessions.jsonl`, `events.jsonl` (read only through
  `EventLog`), `board-writes.jsonl` (through `BoardMirror`); folds never see a rewritten line
  → `docs/areas/capture.md`, `docs/areas/attention.md`, `docs/areas/side-panel.md`.
- **Work-item notes are append-only, addressed by position** (`#n`, rotation's `carried`) →
  `docs/areas/work-items.md`.
- **`usage-cache.json` is a record, not a cache**: never prune or rebuild it → `docs/areas/side-panel.md`.
- **Nothing types into the user's chat or ends it without their click**: no automatic rotation or
  restack, never from a state build, poll or notifier; `/rotate` stays
  `disable-model-invocation: true` → `docs/areas/rotation.md`, `docs/areas/stacks.md`.
- **No new agent use without the user's OK** → `docs/overview.md`.
- **qm never checks out a branch in an existing worktree**, and cleanup never removes what it
  can't prove safe; remote branches are never touched → `docs/areas/worktrees.md`.
- **`qm gui` has no self-shutdown**: never reintroduce a liveness timer or heartbeat → `docs/areas/gui-shell.md`.
- **GUI POSTs with a foreign `Origin` are refused**; Origin-less only for
  `LoopbackGuard::LOCAL_ACTIONS`; the terminal WebSocket and `/api/drop` also need the
  per-start token → `docs/areas/gui-shell.md`.
- **Chats: one tmux session per item on the `qm` socket, named by slug; the program runs with
  every `CLAUDE*` var unset** (else transcript saving turns off) → `docs/areas/chats.md`.
- **GUI long actions run on `GuiJobs` threads, never under the action mutex** → `docs/areas/gui-shell.md`.

## Conventions

- **Build through `/delegate-build`** if a change needs a new test, a full-suite run or >3
  files; small features take its light path (one task, builder + reviewer).
- Name by intent, not implementation; rationale goes in commit messages, not code comments.
- One file per class under `lib/quartermaster/`, intent-named (`SessionLog`, `WorkItem`, …);
  the GUI page is real assets under `lib/quartermaster/gui/`.
- Tests: `ruby test/quartermaster_test.rb` (minitest), `cargo test`; mutation-check ports with
  `QM_BIN` set (else CLI tests run Ruby). Test capture changes before starting real sessions.
  Throwaway test repos need `-c commit.gpgsign=false` and one commit (unborn branches have no
  branch name); commits to this repo are always signed.
- Big files and this file may only shrink (`test/size_ratchet_test.rb`): split along a real
  seam, or raise `test/size_limits.json` deliberately, said in the commit message.
