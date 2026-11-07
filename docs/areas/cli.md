# CLI and viewer

**Code:** `lib/quartermaster/cli.rb`, `lib/quartermaster/cli_format.rb`, `lib/quartermaster/cli_viewer.rb`, `lib/quartermaster/cli_attention.rb`, `lib/quartermaster/cli_chats.rb`, `lib/quartermaster/cli_prs.rb`, `lib/quartermaster/cli_cleanup.rb`, `lib/quartermaster/cli_workspace.rb`, `lib/quartermaster/viewer.rb`, `lib/quartermaster/feature_use.rb`, `quartermaster`
**Tests:** `test/viewer_test.rb`, `test/feature_use_test.rb`
**Depends on:** [work-items](work-items.md), [linking](linking.md), [gui-shell](gui-shell.md)

## What it does

`qm` is shared by the user, Claude and the hooks. `./quartermaster` is a launcher only: it
requires `lib/quartermaster` through its realpath, so the `~/bin/qm` and `~/bin/quartermaster`
symlinks keep working, and runs `CLI#run` (or, with `QM_USE_RUST=1`, `exec`s the Rust binary → rust-port.md). Help text comes from `CliFormat::USAGE_SECTIONS`;
`qm help` is authoritative. Commands, one line each (flags in `qm help`):

**Viewer and GUI**
- `qm` — the fzf viewer in a terminal with fzf; otherwise (piped, no fzf) `qm list`.
- `qm viewer` — the fzf viewer explicitly.
- `qm gui` — the loopback web UI ([gui-shell](gui-shell.md)).
- `qm caffeinate on|off|auto|status` — the running GUI's ☕ ([side-panel](side-panel.md)).
- `qm open-chat SLUG` — bring the GUI up on an item's chat (a notification's click; [attention](attention.md));
  `--benchmark BRANCH` opens that branch's benchmark modal over it.

**Work items**
- `qm new TITLE` — create an item, print its slug (`--project`, `--branch`, `--session`, `--description`, `--pf-mode`).
- `qm attach SLUG --session ID` / `qm detach SLUG --session ID` — link or unlink a session; attach moves it off any other item.
- `qm note SLUG TEXT [--kind open|decision|history]` / `qm retag SLUG POSITION KIND` — add a note, change a note's kind.
- `qm state SLUG STATE` — in_progress | fridge | resolved.
- `qm link-branch SLUG [BRANCH] [--temp]` / `qm unlink-branch SLUG BRANCH` — branch links (default: current branch; `--temp` marks a scratch branch cleanup may delete).
- `qm board SLUG [URL|clear] [--answers-dir DIR]` / `qm board-rows SLUG [--json]` / `qm board-answer SLUG ROW TEXT` — status board URL, mirrored rows, answering a blocked row.
- `qm move SLUG PROJECT` / `qm retitle SLUG TITLE` / `qm describe SLUG [TEXT|-]` — project, title (slug never changes), description (stdin-capable).
- `qm show SLUG` — the item with sessions, warnings, stack lines and usage.
- `qm list` — table of all items.
- `qm draft add SLUG …` / `qm draft list SLUG [--json]` — Claude's review drafts ([review](review.md)).
- `qm issues-dir SLUG` — the item's issue-drafts directory ([drafts](drafts.md)).
- `qm cleanup SLUG [--yes] [--force P]` / `qm sweep [--yes] [--force P]` — plan or run worktree cleanup ([worktrees](worktrees.md)).
- `qm feature-use [--days N]` — per kind, each name used in the last N days (default 30) with its count and last use, then the known names nobody used; dates the log's first line.
- `qm ask QUERY` — find items by meaning (headless claude); `qm grep PHRASE` — sessions whose prompts or replies contain it ([gui-shell](gui-shell.md)).

**Attention**
- `qm attention` — chats by whose move it is, plus expiring transcripts, stale items, merged PRs.
- `qm overview --json [PATH] | --url | --page | --page-published` — the phone page ([phone-page](phone-page.md)).
- `qm ack SLUG [SESSION_ID]` / `qm snooze SLUG 1d|7d|Nd|turn|clear` — quiet a warning or the current your-turn.
- `qm distill SLUG [--session ID] [--force]` — snapshot expiring transcripts ([work-items](work-items.md)).

**Sessions and workspace**
- `qm resume SLUG [--session ID] [--here]` / `qm start SLUG [--here]` — resume or start in a new Warp tab (or this terminal).
- `qm pane-start SLUG` / `qm pane-resume SLUG [--session ID]` — the same in the running GUI's pane ([chats](chats.md)).
- `qm start-issue N [--project P]` — item + issue worktree + kickoff chat ([issues](issues.md)).
- `qm loose [--project P] [--model M] [--starter NAME] ["prompt"]` — a chat with no work item in the GUI; `--starter` opens it with a config.yml `loose_starters:` kickoff ([loose-chats](loose-chats.md)).
- `qm rotate SLUG [--close]` / `qm rotate --self [--close]` — context rotation ([rotation](rotation.md)).
- `qm pr SLUG` / `qm pr-requires PR-OR-SHA …` — open the PR; PR prerequisites ([prs](prs.md)).
- `qm work-on SLUG` / `qm pf-switch SLUG` / `qm ide SLUG` / `qm term SLUG` — pf + IDE, pf, IDE, Warp tab on the target worktree (`--worktree PATH`; `--mode M` for the pf ones). The IDE is config.yml's `ide_command:`; without one `qm ide` refuses naming it and `qm work-on` only switches.
- `qm pf-mode SLUG [MODE]` / `qm pf-status` / `qm pf-stop` — an item's dev-server mode (one of its project's `dev_server.modes`); what the dev server serves; stop it. The `pf-*` command names stay; their output names the configured command.
- `qm sessions [-n N]` / `qm inbox [-n N]` / `qm dismiss ID` / `qm show-session ID` — captured sessions, the unlinked inbox.
- `qm session-item ID` — the linked item's title (for the status line).

**Hooks**: `qm hook-session-start`, `qm hook-session-end`, `qm install-hooks` (prints the
settings snippet) — see [capture](capture.md).

Internal, called by the viewer's fzf bindings: `viewer-rows`, `sessions-view`, `inbox-view`,
`cycle-filter`, `session-rows`, `inbox-rows`, `choose-state`, `choose-project`,
`choose-snooze`, `attach-session`, `item-from-session`. Internal, spawned by the GUI: `bench-run
<run dir>`, a benchmark run's detached pipeline ([benchmarks](benchmarks.md)).

**Command groups.** `CLI` keeps the work-item commands; the rest live in modules mixed into it,
one file each: `CliViewer` (viewer rows and pickers, `show`, sessions), `CliAttention` (inbox,
attention, overview, ack, snooze, distill), `CliChats` (commands acting on the running GUI),
`CliPrs` (pr, pr-requires, draft), `CliCleanup` (cleanup, sweep), `CliWorkspace` (resume, start,
pf, IDE, terminal), and `CliFormat` (shared output helpers and the help text). Each group's frozen
`COMMANDS` maps a word to the method `CLI#run` calls with argv; `CLI.commands` merges them and
refuses a word claimed twice. A new command touches only its group's file (and `qm help`). Hooks
and commands called otherwise (no argv, extra arguments, another class) stay explicit `when`
branches in `CLI#run`.

**The viewer** (`Viewer`, bare `qm`). One fzf list of items: a state badge, ▶ when a session is
live, attention reasons; items with attention entries first, then by activity. The preview pane
is `qm show`. The filter cycles open → in_progress → fridge → resolved → all (`^F`, kept in
`viewer-filter` under the data root). Bindings: Enter or double-click resume (new Warp tab), `^N`
new session, `^X` the item's sessions (Enter resumes one), `^B` the inbox (Enter attach, `^W` new
item from session, `^D` dismiss), `^S` state, `^P` project, `^O` PR, `^T` distill, `^A` ack,
`^Z` snooze, `^R` reload. The viewer stays open while actions run.

**Feature use** (`FeatureUse`). Every `qm` command (by its word; not hooks, not `QM_SKIP`
runs), every GUI action (by `cmd`; other POST routes by route) and every GUI `/api/` read
except the `/api/state` and `/api/pulse` polls (by route, cut to `/api/<word>`) appends one
`{v, at, kind, name}` line to `feature-use.jsonl`: the evidence for deleting unused features.
`qm feature-use` reads it back. Its never-used lists come from the dispatchers themselves —
`GuiActions#dispatch` and `Gui#respond`'s other POST routes, `Gui#respond`'s `/api/` GET routes,
`CLI.commands` and `CLI#run`'s explicit words — read by `FeatureUse.known_names`, so a new action, route or
command shows up without touching a list.

**Live sessions.** SessionStart records the claude pid; SessionEnd clears it. `qm sessions`
marks `● live` sessions; the viewer and GUI show a live badge. Resuming a live session is
refused ("switch to its Warp tab"): it would fork the conversation.

## Rules

- Never rename or move `./quartermaster`: the `~/bin` symlinks and the hook registrations point at it.
- Commands that act on the GUI (`caffeinate`, `pane-start`, `pane-resume`, `rotate`, `start-issue`, `loose`) POST `/api/action` to `127.0.0.1:$QM_GUI_PORT` (default 49737) without an Origin; with no GUI they say so, do nothing and exit 1 — except `start-issue`, which then makes the item and worktree itself and prints how to start the chat, and `loose`, which prints how to start it. Each needs its action in `LoopbackGuard::LOCAL_ACTIONS`.
- `qm install-hooks` prints the snippet and never writes `~/.claude/settings.json`; that file is the user's.
- Hook commands never print except SessionStart's context, honour `QM_SKIP`, rescue everything and exit 0.
- `qm board SLUG` with no URL prints only the URL and exits non-zero when unset, so a shell can test it in one call.
- `qm rotate --self` refuses from an aside and from a process that belongs to no item's chat; only the `/rotate` skill runs it.
- Liveness is checked at read time (`kill(0, pid)`), never trusted from the log: SessionEnd doesn't fire on crashes.
- The viewer's actions are plain `qm` subcommands, so the viewer, the GUI and Claude share one implementation; mouse support is double-click resume only — fzf's `click-header` + `execute` handoff didn't reliably hand the terminal to picker actions, so clickable actions live in the GUI.
- The feature-use log holds a kind and a name only — never a slug, argument, path or session id — is only appended to, and recording is best-effort: a failed write goes to `qm.log` and never fails the command or request.
- `FeatureUse.known_names` reads `CLI.commands`, plus `when "…" then` branches and `method == … && route …` conditions; a dispatcher reshaped away from that loses its names from the never-used lists (the known-names test pins a few).
- `ArgumentError` from any command becomes `qm: <message>` and exit 1, never a backtrace.

## Data

- `viewer-filter` — the viewer's current state filter.
- `feature-use.jsonl` — one line per feature use, append-only, never pruned.
