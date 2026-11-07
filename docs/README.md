# Quartermaster docs

Short, current-truth pages, one per area. Pick the page for the code you touch; each area
page's `**Code:**` line names the files it owns. Update that page in the same commit as the
behaviour change: rewrite the paragraph, don't append. The page says what is true now; why
goes in the commit message.

## Start here

- [overview](overview.md) — goal, the two layers, the data root, tech decisions and cross-area
  invariants. Read first, or when a change spans areas.
- [rejected](rejected.md) — ideas weighed and not built, and deferred ones. Read before
  proposing a new feature or reviving an old one.

## Areas

- [capture](areas/capture.md) — read when you touch the hooks, `sessions.jsonl`, session
  liveness, transcript location or project derivation.
- [work-items](areas/work-items.md) — read when you touch the work-item file, notes and
  their kinds, the injected digest, snapshots/distill, resumability or staleness warnings.
- [linking](areas/linking.md) — read when you touch how sessions attach to items: `QM_ITEM`,
  branch auto-match, undoing an auto-link, the injected protocol, resume and launch.
- [attention](areas/attention.md) — read when you touch `events.jsonl`, whose-move-it-is
  states, the inbox of chats, background-work detection or notifications.
- [chats](areas/chats.md) — read when you touch the tmux model, the terminal WebSocket, key
  chords, pane sizing, file drops or the history above a chat.
- [rotation](areas/rotation.md) — read when you touch rotate, self-rotation, wrap up & close,
  carried notes or the context meter.
- [asides](areas/asides.md) — read when you touch the aside drawer, `QM_ASIDE` or promote.
- [loose-chats](areas/loose-chats.md) — read when you touch item-less chats: `QM_LOOSE`,
  `qm loose`, the Loose group, ✕ and → Work item.
- [diff](areas/diff.md) — read when you touch the diff pane: branch / PR / since-prompt
  scopes, prompt snapshots, stack layers, context expansion, viewed and uncommitted marks.
- [benchmarks](areas/benchmarks.md) — read when you touch the 🏁 benchmark modal, benchmark
  runs of a branch (benchmark settings, run folder, detached pipeline, cancel, re-run) or their payload.
- [review](areas/review.md) — read when you touch diff comments, review anchors, GitHub
  drafts, posting a pending review, review threads or Claude's review drafts.
- [drafts](areas/drafts.md) — read when you touch `drafts/<slug>.json`, page annotations
  (pf relay, clipboard load) or issue drafts.
- [prs](areas/prs.md) — read when you touch the PRs view, PR state per branch, PR
  prerequisites or deployment checks.
- [issues](areas/issues.md) — read when you touch starting work from an issue, the issues
  pane and Issues view, or issue comments.
- [stacks](areas/stacks.md) — read when you touch gh-stack sync, chains or restack.
- [worktrees](areas/worktrees.md) — read when you touch target-worktree choice, cleanup and
  sweep safety, or the git queries behind them.
- [gui-shell](areas/gui-shell.md) — read when you touch the GUI server, router, Origin
  checks, long actions, the state payload and pulse, page assets, popovers, search, or pf.
- [side-panel](areas/side-panel.md) — read when you touch the chat side panel or header:
  workspace, status board mirror, usage and cost, rate limits and the status line, caffeinate,
  build progress.
- [phone-page](areas/phone-page.md) — read when you touch the phone work-items page or its
  publishing.
- [cli](areas/cli.md) — read when you touch a `qm` command, the bare-`qm` fzf viewer or live
  sessions listing.
- [rust-port](areas/rust-port.md) — read when you touch the Rust crate, its fallback to the
  Ruby launcher, or the black-box characterization tests that pin both.

## Also here

- `gui-mockup.html` — the GUI's design-reference prototype (fake data).
- `showcase/` — a fake demo world for running the real `qm gui` on made-up data, scripted
  screenshots of it and the narrated overview video; how to run them is in `showcase/README.md`.
- `test/docs_shape_test.rb` checks these pages' shape: headers, code ownership, links, sizes.
