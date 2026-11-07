# Rust port

**Code:** `Cargo.toml`, `src/ask_command.rs`, `src/attention.rs`, `src/attention_command.rs`, `src/background_jobs.rs`, `src/branch_diff.rs`, `src/chat_attention.rs`, `src/chats_commands.rs`, `src/cleanup.rs`, `src/cleanup_command.rs`, `src/cleanup_run.rs`, `src/cli.rs`, `src/cli_format.rs`, `src/aside.rs`, `src/caffeinate.rs`, `src/clock.rs`, `src/command_runner.rs`, `src/data_root.rs`, `src/delegate.rs`, `src/draft_command.rs`, `src/drafts.rs`, `src/error.rs`, `src/event_log.rs`, `src/feature_use.rs`, `src/front_edit.rs`, `src/gh_stack.rs`, `src/git.rs`, `src/git_repo.rs`, `src/grep_command.rs`, `src/gui_action.rs`, `src/gui_actions.rs`, `src/gui_draft_actions.rs`, `src/gui_jobs.rs`, `src/workspace.rs`, `src/gui_server.rs`, `src/gui_state.rs`, `src/hook.rs`, `src/issue_drafts.rs`, `src/item_usage.rs`, `src/launcher.rs`, `src/lib.rs`, `src/main.rs`, `src/note.rs`, `src/outcome.rs`, `src/pane_follows_session.rs`, `src/pf.rs`, `src/pr_prerequisites.rs`, `src/pr_requires_command.rs`, `src/pr_status.rs`, `src/price_defaults.rs`, `src/price_table.rs`, `src/project_config.rs`, `src/project_map.rs`, `src/prompt_diff.rs`, `src/psych.rs`, `src/review_anchors.rs`, `src/review_plan.rs`, `src/ruby_format.rs`, `src/ruby_num.rs`, `src/ruby_text.rs`, `src/sections.rs`, `src/session_commands.rs`, `src/session_log.rs`, `src/session_rows.rs`, `src/sha1.rs`, `src/sha256.rs`, `src/show_command.rs`, `src/stack_layers.rs`, `src/tmux.rs`, `src/transcript_search.rs`, `src/transcript_usage.rs`, `src/transcript_vitals.rs`, `src/transcripts.rs`, `src/uncommitted_marks.rs`, `src/unified_diff.rs`, `src/usage_ledger.rs`, `src/viewed_marks.rs`, `src/viewer_commands.rs`, `src/work_item.rs`, `src/work_item_commands.rs`, `src/yaml_scalar.rs`, `src/config_seed.yml`, `src/usage.txt`
**Tests:** `tests/ask.rs`, `tests/attention.rs`, `tests/attention_commands.rs`, `tests/characterization.rs`, `tests/chat_attention.rs`, `tests/chats.rs`, `tests/cleanup.rs`, `tests/grep.rs`, `tests/gui.rs`, `tests/interop.rs`, `tests/launcher_switch.rs`, `tests/issue_drafts.rs`, `tests/pane_follow.rs`, `tests/pr_requires.rs`, `tests/psych_scalars.rs`, `tests/sessions.rs`, `tests/show.rs`, `tests/usage.rs`, `tests/viewer.rs`, `tests/work_items.rs`, `tests/common/mod.rs`, the golden files in `tests/fixtures/golden/`, and the unit tests inside the `src` files
**Depends on:** [capture](capture.md), [work-items](work-items.md), [cli](cli.md)

## What it does

`qm` is being ported from Ruby to Rust one area at a time. The Rust binary (`target/debug/qm`,
crate `quartermaster`) serves the commands it has ported and `exec`s the Ruby launcher
`./quartermaster` — arguments and stdin untouched — for every other word, so any command works
from either binary while the port is partial. `QM_RUBY_LAUNCHER` overrides where the fallback
looks; the default is the checkout the binary was built from.

Ported so far: the SessionStart and SessionEnd hooks (the same events, the same injected
context, byte for byte), `sessions`, `inbox`, `dismiss`, feature-use recording for them, the
session log, config.yml's project side, the project map, and the work-item commands `new` (with `--pf-mode`, checked against the project's dev-server modes under the lock),
`note`, `retag`, `state`, `link-branch`, `unlink-branch`, `board`, `move`, `retitle`,
`describe`, `detach` and `list` (bare `qm` without a terminal lists too), and `show-session`,
`session-item`, `help`, `install-hooks`, `ack`, `snooze`, `show`, `attach`, `new --session`, `cleanup SLUG`, `state … resolved`, `issues-dir`, `draft list`, `sweep` (the plan) and the viewer's read-side commands `viewer-rows`, `session-rows`, `inbox-rows` and `cycle-filter`, `grep`, `ask`, `rotate`, `caffeinate`, `pane-start`, `pane-resume` and `pr-requires` (add, note, remove, list; `pr`, which opens a browser, stays Ruby). `attach` and `new --session` move the session between items under the work-item lock (reentrant per thread, as Ruby's, so a
compound move nests) and then rename the chat's tmux session to follow it, unless it is an aside, its
pane runs another process, or the new name already has a chat (then a warning). `Tmux` is ported only
as far as listing, liveness, pane pid and rename; starting a chat, typing into it, respawn and kill
stay Ruby, since nothing types into the user's chat without their click. `show` keeps the Ruby command's
side effects, in its order: it auto-links the item's gh-stack branches (`GhStack`, read from the
repo's common git dir; refs read from loose files and `packed-refs`, or git for a reftable repo),
refreshes `pr-cache.json` through `gh`, and saves the usage ledger. `GhStack` has no in-process
caches — they only help a long-lived GUI — and its `chains` (stacks and loose branches with ahead
counts and restack flags) are ported whole for the GUI port to use. The attention rules themselves
(`Attention`: expiring transcripts, quiet items, merged PRs, with `PrStatus` and its `pr-cache.json`)
are ported as a library, and so are the chat rules under `qm attention`, which is ported too:
`EventLog` folds `events.jsonl` into `events-fold.json` (rewritten by a read that finds new bytes,
and only when a live session needs a turn), `TranscriptVitals` reads a transcript's tail,
`BackgroundJobs` finds pending background work, and `ChatAttention` ranks the chats from them.
So is the usage side of the side panel — `TranscriptUsage`, `UsageLedger` (`usage-cache.json`),
`PriceTable` and `ItemUsage` — which `show` will print. The ledger is a record: Rust reads it, adds
to it and saves it atomically over whatever another process saved meanwhile, and never prunes or
rebuilds it. Like Ruby, a file that does not parse reads as empty and the next save replaces it. `src/usage.txt` is `qm help`'s output as Ruby prints it;
the tests compare it with Ruby's, so a change to `CliFormat::USAGE_SECTIONS` fails until the file
is regenerated (`./quartermaster help > src/usage.txt`). `install-hooks` prints the snippet for
the checkout's Ruby launcher, never for the Rust binary. **The launcher can hand over the other way, opt-in:** with `QM_USE_RUST=1` (exactly `1`) `./quartermaster` `exec`s `target/release/qm` beside it (`QM_RUST_BIN` names another) before loading any Ruby, stdin and exit code the binary's; a missing or non-executable binary changes nothing. The binary sets `QM_FROM_RUST` when it falls back to the launcher, which skips the hand-over and removes the variable from its environment, so the two never bounce and the marker never reaches what Ruby starts (`tests/launcher_switch.rs`).

**The GUI server is being ported route by route.** `qm gui` is still Ruby's. `QM_RUST_GUI=1 qm gui`
runs `gui_server` instead, for comparing the two side by side on one data root (different ports): it
binds only 127.0.0.1, has no self-shutdown, applies `LoopbackGuard`'s `Host` and `Origin` rules and its
origin-less allowlist, records the feature use of every `/api/` request, and serves what is ported — the
page (taxonomies and a per-start terminal token injected), `/assets/css|js/…`, `/api/pulse` (with
`Drafts.signature` and `IssueDrafts.signature`, a SHA-1 of file names, sizes and exact mtimes as
rationals), the PRs view (`GET /api/prs` via `gui_prs`: `pr_inbox` caches the GraphQL list in `pr-inbox.json` for 5 minutes and refreshes in a background thread, `pr_deployment` the release/merge-commit verdicts in `pr-deployment.json`, `pr_items` matches PRs to items and review items; actions `pr-unrequire`, `pr-chat`, `pr-review`, `pr-pf`, `pr-ide`; `prerequisitesRefreshing` is timing-dependent, so the differential test normalizes it), `GET /api/pr-threads` (`review_threads`: the PR scope's threads by `gh api graphql`, 50 per page up to 10 pages, cached 30 s per repo/number/head; `pendingReview` comes from the last page read, as in Ruby), the terminal WebSocket (`terminal_bridge`: `/ws/terminal` bridged to a PTY running a fresh `tmux attach` per connection — binary frames are keystrokes, text frames `{cols, rows}` resizes, refused unless Host, Origin and the per-start token are ours and the chat is live; closing only kills that client), `POST /api/drop` (`drops`: a file dropped on a terminal saved under `drops/` with a one-word name, kept a week; a body past 32 MB is refused unread with a 413 on any route), the turn notifier (`turn_notifier`: a watcher thread raises a notification for a chat that is blocked or whose long turn ended — never for an aside, a snoozed turn or work handed off — through the page when its pulse is fresh, else terminal-notifier or osascript; the notifications, their count in the pulse and `GET /api/attention`), `GET /api/history/<slug>[/<session>]` (`chat_history`: the timeline of earlier sessions and rotation dividers, then pages of 60 transcript entries from the newest end — what the user typed, what Claude answered with a message's blocks joined, one line per tool call; a pruned transcript falls back to its snapshot), `GET /api/workspace/<slug>` (where "work on this" would land: the target by `workspace::resolve`, the dev server and IDE, the chat's branch and the default selection; creates nothing), `GET /api/issues` and `GET /api/issues/<slug>` (`gui_issues`, `issue_drafts::read`: each file's front matter and body, the revision snapshot's `acknowledged` and `previous`, the GitHub repo its numbers link into; reading writes the `.prev` snapshots as Ruby's does), `GET /api/asides/<slug>` (`aside::history`) and `GET /api/session/<id>` (`session_detail`: the capture metadata and, while the transcript survives, each turn's tool tallies, files and reply, clipped; an entry Ruby's parse would raise on gives `turns: null`), `POST /api/ask` (`ask_command::search`, shared with `qm ask`: the headless `claude -p` call over the item index, hits limited to real slugs, a claude failure or odd answer worded as Ruby's `ArgumentError`), `POST /api/grep` (`transcript_search::Found::to_json`: the sessions whose conversation mentions the phrase, grouped by item, with each first hit's own timestamp, transcript path and cwd; what only Ruby judges exactly answers `501`), `/api/state` (`gui_state`: the item, inbox, attention, loose-chat list (each open `loose--<id>` pane titled by its first prompt, cut at 60 characters, and left out of the inbox), dev-server, usage, drafts and issues payload; PR states are served stale and refetched on threads under a cache lock; a build that meets a subsystem still Ruby's — answers `501` when it is active, inactive defaults otherwise), `/api/diff/<slug>` (below) and `/api/diff-file/<slug>` (`FileAtRevision`: one file at a revision as lines, 1 MB and
20,000 lines caps, invalid bytes scrubbed to `?`). Everything else answers `501 not ported yet`. `/api/diff/<slug>`
serves the `branch`, `commits` and `uncommitted` scopes of an ordinary item (`BranchDiff`:
the chat's checkout from its tmux pane or `Launcher#start_cwd`, the branch choices, the gh-stack base
and `base=` spans, commit runs and messages, untracked files; `UnifiedDiff`; `UncommittedMarks`;
`StackLayers`; `ViewedMarks`' digests, SHA-256 of the file's diff as shown) with Ruby's key order;
the first load links the item's other stack branches, as Ruby's does. The `prompt` scope
(`PromptDiff`, `PromptSnapshots`) is served too: the chat's session (the live one in its tmux pane,
else any live one, else the newest captured), its prompt snapshots labelled from the transcript,
the worktree written as a tree through a throwaway index, branches that moved since the prompt from
the reflogs, and the rebase replay through `git merge-tree` (which git 2.43 refuses with a tree, so
there it reports "git merge-tree failed", Ruby alike). Timestamps only Ruby's `Time.iso8601` accepts,
transcripts that are not UTF-8 and the like answer `501`. The `pr` scope is served as well, Review items included
(`PrRevisions`: the PR from the PRs view's cache or `gh pr view`, only an open one for a branch; its
head fetched into `refs/quartermaster/pr/<n>` and its base into `origin/<base>` with
`git fetch origin` through the runner, under one lock; the memo that reuses a lookup or a fetch for a
minute; `ReviewAnchors` marking the lines of a selected run that GitHub won't take with `"gh": false`).
That fetch writes those two refs in the project's repo, as Ruby's does. The item's GitHub drafts on that PR
are routed as Ruby routes them (`ReviewPlan`: on the head, on the commit they were made on — followed
through a rebase by patch-id — or outdated, with the lines around them) and re-anchored in place
(`PendingReview.reanchor`, `Drafts#relocate`: moved drafts take their new line, the head and base,
revs unchanged, `failed` cleared), so a GET of the PR scope rewrites `drafts/<slug>.json` here too.
Next: the state build.

**`POST /api/action` is ported command by command.** `gui_actions::dispatch` runs under one mutex, as Ruby's action mutex does, and answers what `GuiActions#dispatch` answers: `{"ok":true}`, `{"ok":false,"error":…}` (an error that is not an `ArgumentError` also lands in `qm.log` as `[gui-action]`), `unknown action '…'` for a word Ruby does not know, and the bare `false` Ruby's `detach` answers when nothing was attached. Ported: `note`, `retag`, `describe`, `retitle`, `move`, `state` (`resolved` answers with the item's cleanup plan, `Cleanup::Plan#to_h` plus each path shown short, planned from the PR cache the background lookups keep; issue files Psych might read differently stay Ruby's), `snooze`, `ack`, `detach`, `new`, `attach`, `item-from-session` (an item on the session's start event: its project, its branch unless mainline, the session claimed from other items) and `dismiss`, the moved session's chat pane following it to the new item or saying why not (`pane_follows_session`), and `loose-start` (a chat in its own tmux session `loose--<id>` under `QM_LOOSE`, in the project's checkout or home; a starter from `config.yml` stays Ruby's) `start`, `resume`, `resume-session` and `pane-warp` (a Warp tab through a tab config under `~/.warp/tab_configs` and `open warp://…`; `start` answers `null` as Ruby's `Launcher#start && ok` does, a Warp failure "operation aborted"; `launcher::resume_plan`: a given session id, even an empty one, narrows the item's sessions to it), `caffeinate` (`caffeinate`: `/usr/bin/caffeinate -i -w <gui pid>` held as one child and ended with SIGTERM, auto mode re-checked every 15 s by a server thread; `QM_CAFFEINATE_BINARY` names another program, a Rust-only seam for tests; shown in the state and the pulse), `adopt` and `restack` (the item's chat gets `/adopt-worktree PATH` or the one-line gh-stack prompt typed in once it is the user's turn — refused mid-turn or blocked, and for restack while background work is out — else a fresh chat starts with it as its first prompt; typing is only ever on the user's click), `work-on`, `pf-switch`, `ide`, `terminal`, `pf-stop` and `run-update` (`workspace`: the target worktree by the gh-stack top, the last linked branch with a worktree or a new one under `.claude/worktrees/`, created with `git worktree add` and the test-DB `prepare`; the dev server, IDE and Warp steps on jobs, the dev-server ones exclusive; a GUI off the live data root is fenced from every dev-server command but `status` in `command_runner`, as Ruby's `PfFence`; `tests/gui.rs` runs two real repos with an origin, a stack and fake tools that log their calls), `cleanup` and `POST /api/cleanup-plan` (the item's or the sweep's plan, re-planned on a job keyed by the scope that relabels itself `cleaning up i/n…`; issue files or drafts only Ruby reads exactly stay Ruby's; `tests/gui.rs` runs it over two real repos, one per server, with real worktree and branch removal), `loose-close` (`/exit` typed into the pane, five seconds of waiting for claude to go, then the tmux session killed and the chat's sessions dismissed, all on a job keyed by the pane) and `loose-promote` (an item from the chat's newest session, the tmux session renamed to its slug or a warning that it could not be; "Tell the chat", which types into the chat, stays Ruby's), the aside drawer's `aside-open`, `aside-reopen`, `aside-close` and `aside-promote` (`aside`: a tmux session `<slug>--aside` under `QM_ASIDE`, the model checked and defaulting to sonnet, a reopened aside on the model its transcript last ran on, a promoted answer folded onto one line as a note; a transcript that is not UTF-8 text or holds a message that is not an object answers `501`), `pane-start` and `pane-resume` (`launcher::chat_plan` and `Tmux::open`: one tmux session per item on the `qm` socket, the chat run through the `CLAUDE*`-scrubbing shell with the server options re-applied, `QM_CHAT_COMMAND` read only as plain words; a live session outside the GUI refuses, a running pane answers `attached`), `issue-acknowledge` and `issues-clear-created` (`issue_drafts::acknowledge` rewrites the file's `.prev` snapshot with `previous` and `acked`; clearing reads once, as every read does, then deletes each file with a number through `issue_drafts::delete`; an issue file Psych might read differently, or a drafts file holding a float, answers `501` with nothing written), `pf-mode` (checked against the project's dev server like `qm new --pf-mode`), `diff-viewed` (`viewed_marks::set`: a digest of 16–64 hex digits, a path's newest six digests, the 2000 most recently marked paths, the file removed when no mark is left; refusals quote the bad digest as Ruby's `inspect` does, `ruby_text::inspect_ascii`, ASCII only) and the draft actions `draft-put`, `draft-edit`, `draft-delete`, `drafts-discard`, `draft-move`, `issue-comment-put`, `thread-reply-put`, `drafts-send` and `issues-create` (`gui_draft_send` pastes one message into the pane through a tmux buffer after 0.5 s of settling — `draft_messages` builds diff-comment, page-annotation and issue-comment texts, and a page URL it cannot parse as Ruby does stays 501; `distill` (`distill`: the plan at once so the already-distilled confirmation works, one headless haiku call per session on a thread with the tail halved while the prompt is too long; `distilling`/`distilled` in the state) and `rotate` (`rotation`: wrap-up typed into the pane, the chat's Stop awaited, `/exit`, `WorkItem::record_rotation`, a fresh chat respawned in the same window, or the item's dead chat distilled first; the waits keep Ruby's 20 min / 1 min / 1 h bounds); the benchmarks (`benchmark_run` folders, meta.json under a flock, the one-live-run rule by `ps`, start/re-run/cancel off the action mutex; `benchmark_pipeline` is the detached `qm bench-run` process with its own process group and a SIGTERM that ends it as Ruby's `SignalException` did; `benchmark_report` serves `/api/benchmark*`; `run.json` that does not parse reports json 2.7's message for a top-level failure only); the status boards (`board_mirror` folds `board-writes.jsonl` incrementally by byte offset — complete lines only, a shrunken file refolded — into the state's `boardRows`; `board-answer` writes the blocked row's answer file by temp name and rename); loose starters (`ProjectConfig::loose_starters`, the state's `looseStarters`, `loose-start`'s `starter` and `loose-promote`'s `tell`, which types one line into the renamed chat once it has settled); `build-progress` and the state's `buildActive`/`buildProgress` (`build_progress`: the sub-agent runs of an item's live sessions read from the journal or the agent transcripts, one headless haiku call as the item's `progress:<slug>` job, its estimate kept in `gui_jobs` and trimmed to the tasks still running; a transcript line that is not an object stays a Ruby crash, not reproduced); `GET /api/branch-stats/<slug>` and `branch-stats-commentary` (`branch_stats`: the skill's script run in the branch's checkout, cached in `branch-stats.json` per root and branch while head, merge base and base ref still match, parsed for the modal; the commentary is one headless haiku call as a job per branch, stored only while the entry still matches; `branch_diff::tip` picks the branch and base); `issue-preview` and `issue-start` (`issue_start`: `gh issue view` read-only, an `issue/<n>-<slug>` branch cut at a word boundary off freshly fetched origin, the worktree through `workspace::materialize_branch(…, base)`, the chat opened as the item's background job with the kickoff prompt; the project defaults to `default_project:`, else the first one); the review actions (`gui_review_actions`: `review-preview`, `review-post` as the item's background job, `pr-thread-resolve`, `draft-place`, and GitHub drafts in `draft-put`/`draft-move` checked with `review_anchors::refusal_stored`; `pending_review` posts through `gh api graphql` like Ruby — a draft value only Ruby reads exactly ends the job with an error rather than falling back, since a job cannot hand over mid-way); `page-annotations` and `drafts-load` (`clipboard_annotations`: the last ```pf-annotations block; owners get `code` from the chat checkout's `app/javascript` under Ruby's file rules; a drafts file holding a float stays Ruby's); `gui_draft_actions` over `drafts::update`: the item's lock and directory exist even when the block refuses, an item left with no drafts and no load ids loses its file, a put on a taken address reshapes it and bumps `rev` only when something changed; a GitHub draft, a PR address and a move to the review need git and answer `501`). Every other known command, and any value whose Ruby reading this port does not reproduce (a non-scalar `text`, a float position, `args` that is not an object, a snooze too long to write), answers `501` before anything is read or written. The arguments are read as Ruby does: `to_s` then `strip`, empty text refused before the item is looked up, `kind` empty is the default, `position` through `Integer()`. `tests/gui.rs` (`the_data_actions_match_ruby`) sends sixty bodies to Ruby's GUI and the Rust one on two copies of one data root and compares each reply, every item file and `qm.log` after each step; `resolving_offers_the_cleanup_plan_as_ruby_does` resolves items over real worktrees (merged, dirty, with unpreserved commits), issue drafts and shared branches; `the_issue_draft_actions_match_ruby` does the same over issue files of every kind, changing a file between acknowledgements; `diff_viewed_and_pf_mode_match_ruby` does the same for viewed marks (a seeded file at the path limit, odd values, junk) and dev-server modes; `the_draft_actions_match_ruby` does the same over seeded drafts of every kind (new ids and stamps named, the junk-file and last-draft cases included). Mutation survivors of the data actions: the `qm.log` line for a non-`ArgumentError` (no data action raises one over a readable item) and a number-valued snooze duration (`to_s` of a number never matches `Nd`, so both give the same error).

**Rotation's CLI is Rust; the rotation is the GUI's.** `qm rotate SLUG [--close]`, `qm rotate --self`
(the user-invoked `/rotate` skill finds the calling chat: the live session whose claude pid is an
ancestor of the process, else `QM_ITEM`; an aside is refused) and the other commands that act on the
running GUI's chats (`caffeinate`, `pane-start`, `pane-resume`) only `POST /api/action` to
`127.0.0.1:$QM_GUI_PORT` through `gui_action`, as Ruby's `Net::HTTP` does, and say what the reply
says; with no GUI they say so and do nothing. The wrap-up prompt, `/exit`, `respawn-pane` and the
recorded rotation run in the GUI process (`Rotation`, the tmux calls), which stays Ruby, so nothing
in Rust types into a chat or ends one — rotation is still only started by the user's own command.
A `QM_GUI_PORT` that is not plain digits, and flag shapes `OptionParser` reads its own way, go to Ruby.

**Cleanup is planned and carried out in Rust.** `state … resolved` (which also drops the item's
viewed marks), `qm cleanup SLUG` and a bare `qm sweep` (every known repo's `.claude/worktrees` worktrees
and `worktree-*` branches, plus the issue drafts of resolved or gone items) build the item's plan with `GitRepo` (worktrees, dirt, and the
commits a head would lose: not in any base, no patch-equivalent among what the bases hold, not a
squash merge into trunk or the head's upstream; a git failure reads as "couldn't check", never as
nothing to lose) and `Cleanup`, and print it. The dev server's status and the project's test-DB
dry run go through `command_runner` — read-only probes. The plan lists the item's issue drafts too
(`IssueDrafts`: each file's front matter, its unsent comments from `drafts/<slug>.json`), and reading
them keeps Ruby's `.prev` revision snapshots — what a file held when last seen and before that — and
prunes those whose file is gone.

`--yes` and `--force` run the plan (`cleanup_run`, the only Rust code that removes anything), exactly
as Ruby's `Cleanup#execute`: the plan is built once and trusted; only `[x]` entries (ticked, so
`ok`) and `--force` ones (a path, a branch, an id or `SLUG/FILE`) are touched; a blocked entry never
runs; worktrees go first (the test DBs are dropped, then `git worktree remove`, `--force` only for a
forced entry; a worktree whose directory is gone is pruned), a branch follows only when its worktree
was removed and no worktree has it checked out (`git branch -D`; a deleted temp branch is unlinked
from its item), and an issue draft takes its snapshot and unsent comments with it. Every `git` and
test-DB call goes through `command_runner` with Ruby's messages. `--yes` on a terminal (Ruby's
progress bar), an unreadable issue-draft set, a drafts file Rust can't rewrite like Ruby and any
other flag shape go to Ruby before anything is touched. Remote branches are never touched.

**Issue drafts are read in two steps.** `issue_drafts::load` only reads: it parses every `*.md`
file's front matter with the rules of `IssueDrafts#parse` (the error texts included) and says "not
mine" for anything Psych's `safe_load` would read differently from the YAML parser here — a YAML
error, an anchor, alias, tag or merge key, a date, a float, `1_000`, `010`, `0x1F`, a plain `yes`,
a file that is not UTF-8 or not a regular file. A command that gets "not mine" goes to Ruby before
anything is written (`state … resolved` checks first, since it changes the item); `Loaded::record`
is the step that writes snapshots, and repeating it changes nothing. `qm draft list` prints the
drafts file's entries (`--json` in the json 2.7 gem's layout, where an empty array is `[` blank line
`]`); `qm draft add` stays Ruby (it resolves a PR through `gh` and writes the drafts file).

Still Ruby, even for these words: `draft add`, `state … resolved` and `cleanup SLUG`
for an item whose issue drafts `load` declines, and every argument shape the
port does not parse exactly as Ruby's `OptionParser` does — abbreviated or `=`-joined flags,
`--`, a dash-leading title or note, `retag` positions Ruby's `Integer()` would read as octal or
hex, `~user` paths, arguments that are not UTF-8. Such a command is handed to Ruby before
anything is written, so the choice is invisible except in speed.

**Writing work items.** A new item's front matter is written key by key in Psych's style:
`psych.rs` picks the style Psych picks (plain, single-quoted when the string would read back as
a number, boolean, null or date, double-quoted when it starts with a non-word character or needs
escapes), applies libyaml's rule for whether that style is allowed, and folds long values at
column 80 with a two-space indent. An existing item is edited by replacing or adding only the
lines of the key being changed, under the `work-items/.lock` flock the Ruby side takes, and
saved through a temp file and rename; every other key and the body stay byte for byte. A value
the emitter cannot reproduce with certainty (line breaks, `<<`, timestamp-like strings, a first
character Rust and Onigmo classify differently) makes the write fail before anything is
written, and the command goes to Ruby.

`./quartermaster`, the `~/bin` symlinks and the hook registrations still point at Ruby. Switching
a hook to the Rust binary is the user's edit of `~/.claude/settings.json`; `qm install-hooks`
still prints the Ruby snippet.

**Switching the hooks** (the user's edit; qm never writes that file). Only the two Ruby hooks
change — SessionStart and SessionEnd; the per-tool-call hooks are `sh` scripts already. Build once
with `cargo build --release` (the binary is `target/release/qm`, ~11 ms a hook against Ruby's ~250 ms,
same context byte for byte) and in `~/.claude/settings.json` replace the two commands with

    "command": "<checkout>/target/release/qm hook-session-start || <checkout>/quartermaster hook-session-start"
    "command": "<checkout>/target/release/qm hook-session-end || <checkout>/quartermaster hook-session-end"

The `||` is the safety net: a missing or unrunnable binary (127) falls back to Ruby, while the
Rust hook itself always exits 0 and never double-runs. Rebuild after every `git pull` — a stale
binary keeps serving old behavior for the words it has ported (every other word still runs the
checkout's Ruby). Rollback: put the Ruby-only commands back (`qm install-hooks` prints them).
Check a switched hook with `QM_ROOT=$(mktemp -d)` and the payload on stdin before trusting it.

## How it is tested

`tests/characterization.rs` is black-box: it runs the executable `QM_BIN` names (default the
Ruby launcher) against a throwaway `QM_ROOT` and compares stdout, stderr, exit codes and the
files written. The same file passes against Ruby and against Rust:
`QM_BIN=$PWD/target/debug/qm cargo test --test characterization`. The hook contexts and the
seeded `config.yml` are golden files in `tests/fixtures/golden/`, generated from Ruby with
`QM_UPDATE_GOLDEN=1` — regenerate only against the Ruby launcher. `tests/interop.rs` mixes the
two binaries on one data root. `tests/work_items.rs` records command transcripts (exit code, streams, the item file after
every step); `tests/chat_attention.rs` builds about 35 chats (every transition, interrupt, background-job and
snooze case) twice, runs Ruby's `qm attention` on one copy and the Rust binary on the other, and
compares the output and the fold file's bytes, then a second run after appends, a truncated log and
no log; it also fuzzes the two transcript parsers and the watcher patterns against Ruby (it names
both binaries itself, so `QM_BIN` does not matter there). `tests/attention.rs` builds one world twice (items, sessions, transcripts, a fake `gh`) and
compares Ruby's and Rust's entries, `pr-cache.json` and `gh` calls; `tests/psych_scalars.rs` dumps about 3,400 generated strings with the real Psych and
compares. `tests/cleanup.rs` builds a git repo with worktrees in every state cleanup cares about (fixed commit dates, so shas agree), runs Ruby's plan and `state … resolved` on one copy and Rust's on the other and compares every printed line and trace, down to how often the dev server was asked. `tests/cleanup.rs` also runs `cleanup SLUG --yes`, `--force` and `sweep --yes` for every item on one copy of the world with Ruby and on the other with Rust, and compares the printed outcomes and what is left: every repo's worktrees and branches, item files, issue drafts and their comments, and what the dev server, test-DB tool and `gh` were asked. `tests/issue_drafts.rs` does the same for issue drafts (every error text, revisions moving between reads, the cases that go to Ruby, `issues-dir`, and `draft list` over every kind of draft); its Rust runs point `QM_RUBY_LAUNCHER` at nothing, so a command that quietly fell back to Ruby fails. `tests/viewer.rs` pins the viewer's rows (ranking, badges, column widths, the stored filter and its cycle, the inbox's snippets) as transcripts generated from Ruby. `tests/grep.rs` searches a transcripts directory holding every kind of entry (tool output, sidechains, compact summaries, asides, junk lines, a sub-agent run) and compares the printed groups, snippets and limits with Ruby's; a phrase Onigmo's multi-character case folding could read differently (`ss` against `ß`), a transcript that is not UTF-8 or a `message` that is not an object hand the search to Ruby. `tests/ask.rs` puts a fake `claude` on `PATH` that records its arguments, environment and stdin and answers with canned text, and pins what Rust sends (prompt, flags, `QM_SKIP`, the item index with its character caps, byte for byte) and what it prints for every answer shape against Ruby's; a missing `claude` or an answer whose shape makes Ruby raise goes to Ruby. `tests/pr_requires.rs` runs `pr-requires` in a git checkout whose origin is a GitHub repo, with a fake `gh` that answers from files and logs its calls, and compares every message, the store's bytes after each step (written like the json 2.7 gem, under the store's flock) and the `gh` calls with Ruby's; the list reads the deploy cache but never refreshes it, as Ruby's CLI doesn't. `tests/chats.rs` stands a fake GUI up on a loopback port and checks that Ruby and Rust send the same `POST /api/action` (path, `Content-Type`, `Host`, no `Origin`, body bytes) and print the same for every reply, a refusal, junk, and no GUI at all, including `rotate --self` finding its chat through the process lineage. `tests/gui.rs` starts Ruby's GUI and the Rust one on one data root and sends both the same requests — the page, assets (good and path-traversal ones), the pulse before and after files change, `/api/diff/` over an item with a checked-out feature branch (committed, uncommitted and untracked changes, renames, deletions, binaries, CRLF, a file without a final newline, a 25,000-line untracked file), an issue branch pair, two gh-stacks (one with a leftover on the trunk, one of 205 commits), a ref-only and an origin-only branch, viewed marks, the PR cache and a live tmux pane choosing the checkout, every scope and `from`/`to`/`base` combination, and `diff-file` over every edge (bad shas, NUL paths, non-UTF-8, 25,000 lines, 30,000 long lines, missing files and items), wrong `Host`s and foreign or missing `Origin`s — and compares status, content type and body, then the feature-use log. `the_state_matches_ruby` compares `/api/state` byte for byte over an attention world on two copies of the data root (a build rewrites caches), after a warm-up build. `the_state_matches_ruby_over_a_richer_world` adds usage transcripts (priced, unpriced and 1M-token sessions), chat events (working, blocked, a snoozed turn), asides, inbox sessions, notes of every kind, rotations, a linked worktree, drafts, issues, `rate-limits.json` and sessions in every relative-time bucket; mutation checks of the state port are killed except the exact second and minute boundaries of `relative_when` (a live clock cannot sit on them) and `round4`'s base rounding (its correction step makes it equivalent). Not covered: a configured dev server, gh-stack chains with worktrees, board rows and benchmarks (still gated). The prompt scope is compared over a worktree with snapshots recorded by the real hook (dated afterwards), a transcript with interrupts, sidechains and a compact summary, a rebase onto a moved trunk, branches that moved before, at and after a prompt, and sessions that are uncaptured, live-but-older, pruned or whose folder is gone. The PR scope is compared over an origin published as `<base>/acme/widgets.git` with `refs/pull/<n>/head` refs, a fake `gh` and a `git` wrapper that log what each server asked (the two logs must match, memo hits and `fresh=1` included): cached PRs, PRs found through gh, closed ones, a gh failure, a wrong repo, a hostile base name, a failed fetch, selected runs and Review items of every kind. The draft re-anchoring runs Ruby and Rust on two copies of one data root (the first GET rewrites the drafts) over about thirty drafts — unchanged, moved by a push, ranges, ambiguous text, on a commit, on a rebased twin, pinned, whole-file, outdated with and without context, old side — and compares the routes and the drafts each left behind. `tests/pane_follow.rs` runs a real tmux server on a throwaway socket (killed afterwards, never the
`qm` socket) with chat-like panes whose pids are written into `sessions.jsonl`. `tests/show.rs` builds a git repo with gh-stack stacks (packed and loose refs, an origin-only branch,
a restack), a fake `gh`, transcripts and sessions, runs `show` over every item and compares the output
and every trace it leaves with Ruby's; mutation checks of the port need `QM_BIN` set to the Rust binary,
or the CLI-level tests only exercise Ruby. `tests/usage.rs` runs Ruby and Rust over the same transcripts pass after pass (appends, rewrites,
a pruned transcript, another process's save) and compares every row and the ledger's bytes; it also
fuzzes costs, labels and `mtime` floats. A command is ported only after the characterization suite
covers it and passes on both.

## Rules

- **No behavior change in a port**: output, exit codes and file contents match Ruby exactly,
  including its quirks; a bug found on the way is fixed in Ruby first, then ported.
- **Hooks never block or break a session**: errors are logged to `qm.log` and the exit is 0;
  only SessionStart prints.
- **A front-matter edit re-parses its own result and refuses to write** if the key is not a
  block list or the text no longer parses, rather than risk the user's work item.
- **Append-only stays append-only**: `sessions.jsonl` is written under flock, one line per event,
  with `v` first; readers skip unparsable lines and tolerate unknown fields.
- **Tests never touch live state**: `QM_ROOT`, throwaway git repos with signing off.
- **Stdlib-light**: crates are `serde_json` (order preserved), `serde_yaml`, `chrono`, `libc`, `regex`.
  Ruby's `\w` and `\s` are ASCII-only while its `\b` is Unicode, so patterns spell the classes out.
- **Numbers match Ruby, not C**: `Array#sum` compensates (Kahan-Babuska) once a Float appears and
  keeps Integers exact before that (`ruby_num.rs`); `Time#to_f` is the nanosecond count as a double
  divided by 1e9; and `format("%.Nf")` rounds a near-tie to even on the last digit generated,
  which Rust's own formatting does not (`ruby_format.rs` ports Ruby's dtoa quick path).
- **Known Ruby race, not fixed here**: Ruby reads an item before taking the lock and saves with a
  plain `File.write`, so concurrent Ruby writers often fail with "malformed work item file". Rust
  writers cannot produce that state; the mixed-writer test allows only that failure, from Ruby.
- **Not ported yet** (still Ruby): the fzf viewer itself and its pickers, every other command, `qm gui`, chats, worktrees,
  PRs, diffs, rotation, benchmarks and `statusline/statusline.rb`.
