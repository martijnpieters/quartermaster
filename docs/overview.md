# Overview

Quartermaster (`qm`) tracks every Claude Code session automatically and curates them into
**work items** with a state (`in_progress` / `fridge` / `resolved`). The user filters, inspects,
resumes and manages them from a terminal viewer (bare `qm`), the loopback web GUI (`qm gui`)
and a phone page — with links to branch, worktree, issue and PR, notes, staleness and
transcript-expiry warnings, and one-click resume. It is a global tool: every project and
no-project session is captured, not just one repo's.

## Two layers: capture, then curation

1. **Session log** (automatic, exhaustive) — append-only JSONL written by global
   SessionStart/SessionEnd hooks. It never misses a session and needs no model cooperation.
   See [capture](areas/capture.md).
2. **Work items** (curated) — one markdown file per item. Sessions attach to items by branch
   auto-match, the `QM_ITEM` launcher, or Claude asking. Unattached sessions sit in a silent
   **inbox** and age out with their transcripts. See [work items](areas/work-items.md) and
   [linking](areas/linking.md).

Hooks, not memory instructions, carry the protocol: memory and CLAUDE.md are best-effort,
hooks are harness-guaranteed, and **SessionStart hook stdout is injected into Claude's
context** — so the protocol announces itself every session. The fallback text in the global
`~/.claude/CLAUDE.md` is only a backup.

Everything else — chat attention, the GUI's chats, diffs, reviews, PRs, issues, stacks,
worktrees — derives from these two layers plus a few more append-only hook logs.

## Data root

`~/.claude/quartermaster/` (override with `QM_ROOT`; tests always do):

| File | What | Page |
|---|---|---|
| `sessions.jsonl` | session start/end events, append-only | [capture](areas/capture.md) |
| `work-items/<slug>.md` | one work item each; `work-items/.lock` serialises writes | [work items](areas/work-items.md) |
| `config.yml` | projects and their settings (below), `prices:` overrides | [capture](areas/capture.md) |
| `qm.log` | hook and background errors | [capture](areas/capture.md) |
| `events.jsonl` + `events-fold.json` | per-prompt/tool/turn hook events and their fold cache | [attention](areas/attention.md) |
| `board-writes.jsonl` | every `ArtifactData` call, mirrored status boards | [side panel](areas/side-panel.md) |
| `prompt-snapshots.jsonl` | the session's worktree as a git tree at every prompt | [diff](areas/diff.md) |
| `usage-cache.json` | per-session token tallies, kept forever | [side panel](areas/side-panel.md) |
| `rate-limits.json` | account 5h/weekly limits as the status line last saw them | [side panel](areas/side-panel.md) |
| `drops/` | files dropped onto a GUI terminal, pruned after a week | [chats](areas/chats.md) |
| `drafts/<slug>.json` | an item's unsent diff comments and page annotations | [drafts](areas/drafts.md) |
| `overview-page.sha256` | the phone page as last published | [phone page](areas/phone-page.md) |
| `viewed/<slug>.json` | the diff pane's viewed marks per item | [diff](areas/diff.md) |
| `branch-stats.json` | branch stats per (repo, branch), keyed on head and merge-base sha; safe to delete | [diff](areas/diff.md) |
| `benchmarks/<run id>/` | one benchmark run each: `meta.json` (`v: 1`), events, script, table, verdict, logs; never pruned | [benchmarks](areas/benchmarks.md) |
| `issues/<slug>/*.md` (+ `.prev/`) | Claude's issue drafts and qm's revision snapshots | [drafts](areas/drafts.md) |
| `pr-cache.json`, `pr-inbox.json` | PR state per linked branch; the PRs view's list | [prs](areas/prs.md) |
| `pr-prerequisites.json`, `pr-deployment.json` | PR prerequisites and their deploy-check cache | [prs](areas/prs.md) |
| `viewer-filter` | the fzf viewer's state filter | [cli](areas/cli.md) |
| `feature-use.jsonl` | one line per CLI command, GUI action and GUI read, append-only | [cli](areas/cli.md) |
| `plans/<slug>/<batch id>.md` (+ `-exploration.md`) | each delegate-build run's plan, written by that skill; qm never reads it | none |

Hooks and the global fallback instruction live at **user level** (`~/.claude/settings.json`,
`~/.claude/CLAUDE.md`), because capture must cover every project and no-project cwd.
`qm install-hooks` prints the settings snippet; the user merges it themselves.

The code is a git repo at `~/code/quartermaster`: the realpath-resolving launcher
`./quartermaster` (symlinked from `~/bin/quartermaster` and `~/bin/qm` — never rename or move
it), one intent-named file per class under `lib/quartermaster/`, the GUI page as real assets
under `lib/quartermaster/gui/`, POSIX hooks under `hooks/`, and a minitest suite run with
`ruby test/quartermaster_test.rb` (shared setup in `test/test_helper.rb`; one file per area).

## config.yml

`ProjectConfig` reads it (re-parsed only when its text changes; a broken file reads as no
projects, and a missing or malformed key leaves its feature off). The seed has no projects,
only commented examples. Under `projects:` a project is a repo root path (`my-app: ~/code/my-app`)
or a mapping with `root:` plus optional feature keys; a project without a key has that feature off
(buttons hidden, actions refused naming the setting):

- `dev_server:` — a command name, or `{command, modes, default_mode, status_url, state_file}`.
  qm runs `<command> switch PATH MODE`, `<command> stop`, `<command> status` (exit 0 = serving);
  `status_url` answers JSON `{mode, root, switching}`, `state_file` holds `<mode> <root>`. Enables
  Work on this / Switch, the mode chip (work-item key `pf_mode`), the header badge, the PRs view's
  switch button and the session's dev-server line. One server is assumed across projects.
- `test_db:` — a command: `<test_db> prepare` in each new worktree; cleanup lists
  `<test_db> drop PATH --dry-run` (one DB per line) and runs `<test_db> drop PATH`.
- `review_skill:` — the PRs view's Review skill (default `/code-review`).
- `pr_prerequisites: true` — PRs ship by release: sessions are told about `qm pr-requires`.
- `benchmark:` — `{database, skill, notes}`: the Postgres database runs measure on (or clone),
  the benchmark skill's folder, optional prompt lines; enables 🏁 ([benchmarks](areas/benchmarks.md)).
- `ide_command:` / `ide_label:` — this project's own IDE, overriding the top-level one below.

Top-level: `ide_command:` (an argv array with a literal `PATH` placeholder, e.g.
`[open, -a, MyIDE, PATH]`: an element that is exactly `PATH` becomes the checkout's path, `PATH`
inside a longer element — a `[zsh, -ic, "myide PATH"]` script — the shell-escaped path) and
`ide_label:` (the buttons' and messages' name, default `IDE`) enable Open in <label>, Work on this's
IDE half, the PRs view's IDE button and `qm ide`; unset, or without a `PATH`, they are hidden and
`qm ide` refuses naming `ide_command`. `default_project:` (where a bare issue number starts; default the first project),
`loose_starters:` (key → `label` + `prompt`, the loose-chat presets; none by default; a prompt
ends mid-sentence, e.g. "… Then give a one-line count of them and", which `LooseChat.kickoff`
finishes with " answer my question about them: …" or " ask me what I want to know about them."),
`prices:` ([side panel](areas/side-panel.md)), `overview_artifact:` ([phone page](areas/phone-page.md)).

## Tech decisions

- **Ruby, `#!/usr/bin/env ruby`**, resolved through asdf shims to whatever the session's cwd
  pins; a user-level asdf default (`~/.tool-versions`) must exist or capture dies outside pinned repos.
  A Rust port is under way and serves the hooks first ([rust-port](areas/rust-port.md)).
- **Stdlib only** (yaml, json, open3, optparse, socket, …), no gems: gems install per Ruby
  version, so a hook firing in a repo pinned to another Ruby would not find them.
- **Ruby 3.3 is the syntax floor** for the same reason — no 4.x-only features.
- **`statusline/statusline.rb` is the exception:** standalone, macOS system Ruby 2.6, never loads `lib/`.
- **The skills qm drives ship in `skills/`** (rotate, work-items, branch-stats, adopt-worktree,
  draft-issues), symlinked into `~/.claude/skills/`; qm's code reads these skills' files from
  its own checkout (`branch-stats.sh`, `page.html`), never through `~/.claude/skills/`.
- `fzf` for the terminal viewer, `gh` for GitHub (always offline-tolerant), tmux for chats.
- **Data formats are versioned**: a `v:` field in JSONL events and work-item frontmatter;
  bump on a breaking schema change, readers tolerate unknown fields.
- One file per class, named by intent (`SessionLog`, `WorkItem`, `HookHandler`, `CLI`, …).
- Rationale lives in commit messages, not code comments or these pages.

## Cross-area invariants

- **Hooks never block or break a session**: rescue everything, log to `qm.log`, exit 0.
  SessionEnd and error paths print nothing — SessionStart stdout is context.
- **Per-tool-call hooks are plain POSIX `sh`**, never Ruby: they never print, always exit 0,
  honor `QM_SKIP`, and only append. All derivation happens in qm at read time.
- **Append-only logs are read incrementally** from a persisted byte offset (`EventLog`,
  `BoardMirror`, `TranscriptUsage`); rewriting earlier lines is invisible, only a shrink refolds.
- **Work-item writes go through one global flock** (`WorkItem.with_lock`) spanning the whole
  read-modify-write, re-reading the file first; reentrant per thread.
- **External tools that change state go through `CommandRunner`** (the dev server, the IDE, `git worktree
  add`, the test-DB command, `osascript`). Tests pass a stub runner and never start real servers,
  IDEs, worktrees or notifications.
- **Scripts first**: every integration is deterministic code; a model runs only where real
  summarising happens (the chat itself, distill, ask, build progress, a benchmark's plan and
  verdict). Any new agent use needs
  the user's OK first.
- **Nothing types into the user's chat or ends it without their click**: rotation, restack and
  sending drafts are user-triggered only, never from a state build, poll or notifier.
- **qm never checks out a branch in an existing worktree** and never deletes what it cannot
  prove safe; remote branches are never touched.
- **`QM_ASIDE` beats every linking rule**: an aside session is never attached, logged as
  attention, distilled or shown in the item's history. Every new per-session surface honors it.
  A loose chat (`QM_LOOSE`) is never linked or distilled either, but does count for attention.
- **The GUI is loopback-only and runs until Ctrl-C**: no liveness timer, foreign `Origin`s refused.
- **Tests never touch the user's live state**: `QM_ROOT`, `QM_TMUX_SOCKET=qm-test-…`, stub runners;
  throwaway test repos commit with `-c commit.gpgsign=false`, this repo's commits are signed.
- **Big files only shrink** (`test/size_ratchet_test.rb`): code over 400 lines, tests over 800 and
  `CLAUDE.md` (bytes) are pinned in `test/size_limits.json`; run it with `--lower` after a shrink.
