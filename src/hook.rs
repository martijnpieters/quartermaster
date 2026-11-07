use serde_json::{Map, Value};

use crate::clock;
use crate::data_root;
use crate::error::{QmError, Res};
use crate::git::git;
use crate::note::{Note, DEFAULT_NOTE_KIND, NOTE_KINDS};
use crate::project_config::ProjectConfig;
use crate::project_map;
use crate::session_log::{Event, SessionLog};
use crate::transcripts;
use crate::work_item::WorkItem;

const WORKTREE_SEGMENT: &str = "/.claude/worktrees/";
const MAINLINE_BRANCHES: [&str; 3] = ["main", "master", "develop"];
const STATES: [&str; 3] = ["in_progress", "fridge", "resolved"];

fn field(input: &Value, key: &str) -> Value {
    input.get(key).cloned().unwrap_or(Value::Null)
}

fn str_field<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str)
}

fn opt(v: Option<String>) -> Value {
    v.map(Value::from).unwrap_or(Value::Null)
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The claude process this hook runs under, as the SessionStart hook finds it; `ps` lineage, else the parent.
fn claude_pid() -> i64 {
    // SAFETY: getppid has no preconditions.
    let parent = unsafe { libc::getppid() } as i64;
    let mut pid = parent;
    for _ in 0..6 {
        let Ok(out) = std::process::Command::new("ps").args(["-o", "ppid=,command=", "-p", &pid.to_string()]).output() else {
            return parent;
        };
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !out.status.success() || text.is_empty() {
            break;
        }
        let mut parts = text.splitn(2, ' ');
        let ppid = parts.next().unwrap_or("");
        if parts.next().is_some_and(|c| c.to_lowercase().contains("claude")) {
            return pid;
        }
        pid = ppid.parse().unwrap_or(0);
        if pid <= 1 {
            break;
        }
    }
    parent
}

pub fn session_started(input: &Value) -> Res<Option<String>> {
    if !input.is_object() {
        return Err(QmError::new("NoMethodError", "hook input is not an object"));
    }
    let session_id = str_field(input, "session_id");
    let cwd = str_field(input, "cwd");
    let branch = git(cwd, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let project = project_map::project_for(cwd);

    if let Some(slug) = env_nonempty("QM_ASIDE") {
        return aside_started(input, &slug, branch, &project);
    }
    let loose = env_nonempty("QM_LOOSE");
    let mut item = match &loose {
        None => linked_work_item(session_id, branch.as_deref(), &project)?,
        Some(id) => promoted_item(session_id, id)?,
    };
    if let (Some(id), None) = (&loose, &item) {
        return loose_started(input, id, branch, &project);
    }

    if let (Some(item), Some(id)) = (item.as_mut(), session_id) {
        item.attach_session(id)?;
    }
    if let Some(item) = item.as_mut() {
        record_temp_branch(item, cwd, branch.as_deref());
    }

    let mut e = start_event(input);
    e.insert("branch".into(), opt(branch));
    e.insert("head".into(), opt(git(cwd, &["rev-parse", "HEAD"])));
    e.insert("pid".into(), claude_pid().into());
    e.insert("project".into(), project.clone().into());
    e.insert("work_item".into(), opt(item.as_ref().map(|i| i.slug.clone())));
    e.insert("ts".into(), clock::now_iso().into());
    SessionLog::append(e)?;

    let id = session_id.unwrap_or("");
    Ok(Some(match &item {
        Some(item) => linked_context(id, item),
        None => unlinked_context(id, &project)?,
    }))
}

fn start_event(input: &Value) -> Event {
    let mut e = Map::new();
    e.insert("event".into(), "start".into());
    e.insert("session_id".into(), field(input, "session_id"));
    e.insert("source".into(), field(input, "source"));
    e.insert("cwd".into(), field(input, "cwd"));
    e.insert("transcript_path".into(), field(input, "transcript_path"));
    e
}

pub fn session_ended(input: &Value) -> Res<()> {
    if !input.is_object() {
        return Err(QmError::new("NoMethodError", "hook input is not an object"));
    }
    let session_id = str_field(input, "session_id");
    let cwd = str_field(input, "cwd");
    let log = SessionLog::read();
    let start = log.last_start_for(session_id).cloned();

    let mut commits = Value::Null;
    if let Some(head) = start.as_ref().and_then(|s| s.get("head")).and_then(Value::as_str) {
        let range = format!("{head}..HEAD");
        if let Some(out) = git(cwd, &["log", "--format=%h %s", &range]) {
            if !out.is_empty() {
                commits = Value::Array(out.split('\n').map(Value::from).collect());
            }
        }
    }

    let recorded = match input.get("transcript_path") {
        Some(Value::String(p)) => Some(p.clone()),
        _ => start.as_ref().and_then(|s| s.get("transcript_path")).and_then(Value::as_str).map(String::from),
    };
    let empty = transcripts::locate(session_id, recorded.as_deref()).is_none();

    let duration = match start.as_ref() {
        Some(s) => {
            let ts = s.get("ts").and_then(Value::as_str).unwrap_or("");
            Value::from(clock::seconds_since(ts).ok_or_else(|| QmError::argument(format!("no time information in {ts:?}")))?)
        }
        None => Value::Null,
    };

    let mut e = Map::new();
    e.insert("event".into(), "end".into());
    e.insert("session_id".into(), field(input, "session_id"));
    e.insert("reason".into(), field(input, "reason"));
    e.insert("duration_s".into(), duration);
    e.insert("commits".into(), commits);
    e.insert("diff_stat".into(), opt(git(cwd, &["diff", "--shortstat", "HEAD"])));
    e.insert("ts".into(), clock::now_iso().into());
    if empty {
        e.insert("empty".into(), true.into());
    }
    SessionLog::append(e)?;
    if !empty {
        return Ok(());
    }

    if let Some(id) = session_id {
        for mut item in WorkItem::all()? {
            item.detach_session(id)?;
        }
        SessionLog::dismiss(id)?;
    }
    Ok(())
}

fn aside_started(input: &Value, slug: &str, branch: Option<String>, project: &str) -> Res<Option<String>> {
    let mut e = start_event(input);
    e.insert("branch".into(), opt(branch));
    e.insert("pid".into(), claude_pid().into());
    e.insert("project".into(), project.into());
    e.insert("aside".into(), slug.into());
    e.insert("ts".into(), clock::now_iso().into());
    SessionLog::append(e)?;
    let item = match WorkItem::find(slug) {
        Ok(item) => item,
        Err(e) if e.class == "ArgumentError" => return Ok(None),
        Err(e) => return Err(e),
    };
    Ok(Some(format!(
        "[qm] This Claude Code session is an aside about work item '{}': a one-off question asked beside the item's chat. It is deliberately not linked to the item — don't run qm commands for it (no note, state, attach or new); the user promotes an answer to the item themselves.\n\n{}\n",
        item.slug,
        item_digest(&item)
    )))
}

fn loose_started(input: &Value, id: &str, branch: Option<String>, project: &str) -> Res<Option<String>> {
    let cwd = str_field(input, "cwd");
    let mut e = start_event(input);
    e.insert("branch".into(), opt(branch));
    e.insert("head".into(), opt(git(cwd, &["rev-parse", "HEAD"])));
    e.insert("pid".into(), claude_pid().into());
    e.insert("project".into(), project.into());
    e.insert("loose".into(), id.into());
    e.insert("ts".into(), clock::now_iso().into());
    SessionLog::append(e)?;
    Ok(Some(format!(
        "[qm] This Claude Code session ({}) is a loose chat, by intent: the user started it without a work item (project {project}). Don't create or attach a work item (no qm new, attach or note) unless the user asks. Before changing any code, ask the user to promote this chat (→ Work item in qm's chat view) so it gets an item and a worktree.\n",
        str_field(input, "session_id").unwrap_or("")
    )))
}

/// A promoted loose chat keeps QM_LOOSE, so a restart follows the item it became.
fn promoted_item(session_id: Option<&str>, id: &str) -> Res<Option<WorkItem>> {
    let mut ids: Vec<String> = session_id.map(String::from).into_iter().collect();
    ids.extend(SessionLog::read().sessions().into_iter().filter(|s| s.loose_id().as_deref() == Some(id)).map(|s| s.id));
    Ok(WorkItem::all()?.into_iter().find(|i| i.session_ids().iter().any(|s| ids.contains(s))))
}

fn record_temp_branch(item: &mut WorkItem, cwd: Option<&str>, branch: Option<&str>) {
    let (Some(cwd), Some(branch)) = (cwd, branch) else { return };
    if !cwd.contains(WORKTREE_SEGMENT) || branch == "HEAD" || MAINLINE_BRANCHES.contains(&branch) {
        return;
    }
    if let Err(e) = item.add_temp_branch(branch) {
        data_root::log_error("temp-branch", e.class, &e.message);
    }
}

/// An item this session already belongs to wins, then QM_ITEM (unknown slugs fall through), then the exact branch in this project.
fn linked_work_item(session_id: Option<&str>, branch: Option<&str>, project: &str) -> Res<Option<WorkItem>> {
    let all = WorkItem::all()?;
    if let Some(id) = session_id {
        if let Some(pos) = all.iter().position(|i| i.session_ids().iter().any(|s| s == id)) {
            return Ok(all.into_iter().nth(pos));
        }
    }
    if let Ok(slug) = std::env::var("QM_ITEM") {
        if let Ok(item) = WorkItem::find(&slug) {
            return Ok(Some(item));
        }
    }
    let Some(branch) = branch else { return Ok(None) };
    let mut candidates: Vec<WorkItem> =
        all.into_iter().filter(|i| i.branches().iter().any(|b| b == branch) && i.project().as_deref() == Some(project)).collect();
    let open = candidates.iter().position(|i| i.state().as_deref() != Some("resolved"));
    Ok(match open {
        Some(pos) => Some(candidates.remove(pos)),
        None => candidates.into_iter().next(),
    })
}

fn linked_context(session_id: &str, item: &WorkItem) -> String {
    let slug = &item.slug;
    format!(
        "[qm] This Claude Code session ({session_id}) is linked to work item '{slug}' — the session id has been attached to it automatically.

{digest}
Work item protocol:
- Append a succinct progress note at meaningful milestones: qm note {slug} --kind <{kinds}> \"...\"
  - open — still unresolved and user-facing: a decision the user owes, a blocked dependency, a known gap, anything you'd describe as \"flagged to the user\". Always re-injected regardless of age, so keep it short and say what would unblock it.
  - decision — a choice made, or a finding about how the system actually is: something a later session would get wrong without knowing it.
  - history — how it got done: commits, rebases, folds, message fixups, push status, specs green. Kept in the file, never re-injected.
  One kind per note: if a milestone has both a decision and its mechanics, write two notes. If a note contains anything still open, it is 'open' whatever else it says. Omitting --kind means '{default_kind}'. Don't repeat the date in the text — qm stamps it.
- When an open item is handled, retag it so it stops being injected: qm retag {slug} <position> decision (positions are the #n above). Same command backfills a kind you got wrong.
- Keep the Description as the item's current truth: when a note records a decision or constraint that still governs the work, fold it into the description (qm describe {slug} \"...\"). Notes fall out of the injection window; the description doesn't.
- Update the state when it changes: qm state {slug} ({states})
- A 'status board' above means the item already has one: reuse that URL instead of publishing a new board (record one with qm board {slug} URL).
- Refine a stale title when asked (or when it's clearly outdated): qm retitle {slug} \"...\"
- The session sits on the item it is working on right now. The moment its work moves to another item (a different issue, a follow-up, refinement prep), re-link it then and there; likewise when this link is a wrong branch-based guess, telling the user:
  continues another item → qm attach <other-slug> --session {session_id}; substantial new work → qm new \"Title\" --project \"...\" --session {session_id} (both move the session off this item); trivial or throwaway → qm detach {slug} --session {session_id} and don't mention tracking again.{pf}{pr}
",
        digest = item_digest(item),
        kinds = NOTE_KINDS.join("|"),
        default_kind = DEFAULT_NOTE_KIND,
        states = STATES.join(" | "),
        pf = pf_protocol_line(item),
        pr = pr_requires_protocol_line(item),
    )
}

/// Only a project with a dev server (config.yml) pays these tokens.
fn pf_protocol_line(item: &WorkItem) -> String {
    let config = ProjectConfig::current();
    let project = item.project();
    let Some(server) = config.settings(project.as_deref()).and_then(|s| s.dev_server.as_ref()) else { return String::new() };
    let mode = item.pf_mode_override().unwrap_or_else(|| server.default_mode.clone());
    let ide = config.ide(project.as_deref()).map(|i| format!(" + {}", i.label)).unwrap_or_default();
    format!("\n- Dev server: qm work-on {slug} ({} {mode}{ide}); mode: qm pf-mode {slug} MODE", server.command, slug = item.slug)
}

fn pr_requires_protocol_line(item: &WorkItem) -> String {
    let config = ProjectConfig::current();
    match config.settings(item.project().as_deref()) {
        Some(s) if s.pr_prerequisites => "\n- PR may merge only after an earlier PR/commit is deployed: qm pr-requires <pr-or-sha> [--note TEXT]".to_string(),
        _ => String::new(),
    }
}

/// A digest rather than the raw file: the note history of a long item would dwarf everything else.
fn item_digest(item: &WorkItem) -> String {
    let notes = item.notes();
    let shown = item.injectable_notes();
    let mut lines = vec![format!("Work item ({}):", WorkItem::path_for(&item.slug).display()), format!("  title: {}", item.title())];
    lines.push(format!("  state: {}", item.state().unwrap_or_default()));
    if let Some(p) = item.project() {
        lines.push(format!("  project: {p}"));
    }
    if let Some(c) = item.created() {
        lines.push(format!("  created: {c}"));
    }
    let branches = item.branches();
    if !branches.is_empty() {
        lines.push(format!("  branches: {}", branches.join(", ")));
    }
    if let Some(b) = item.board() {
        lines.push(format!("  status board: {b}"));
    }
    if item.session_count() > 0 {
        lines.push(format!("  sessions: {} (ids in the file)", item.session_count()));
    }
    let description = item.description();
    lines.extend(["".into(), "## Description".into(), "".into(), if description.is_empty() { "(none)".into() } else { description }]);
    let suffix = if notes.len() <= shown.len() {
        String::new()
    } else {
        format!(" ({} of {} — the rest are history or older; read them all with `qm show {}`)", shown.len(), notes.len(), item.slug)
    };
    lines.extend(["".into(), format!("## Notes{suffix}")]);
    if shown.is_empty() {
        lines.push("(none yet)".into());
    } else {
        lines.extend(shown.iter().map(note_line));
    }
    if item.snapshot_count() > 0 {
        lines.extend(["".into(), format!("## Snapshots: {} distilled transcript(s), read with `qm show {}`", item.snapshot_count(), item.slug)]);
    }
    format!("{}\n", lines.join("\n"))
}

fn note_line(n: &Note) -> String {
    format!("- #{} {} [{}]: {}", n.position, n.stamp, n.kind, n.text)
}

fn unlinked_context(session_id: &str, project: &str) -> Res<String> {
    let mut listing: Vec<String> = WorkItem::all()?
        .iter()
        .filter(|i| i.state().as_deref() != Some("resolved"))
        .map(|i| {
            let mut details: Vec<String> = i.project().into_iter().collect();
            details.extend(i.branches());
            details.extend(i.state());
            format!("- {} — {} ({})", i.slug, i.title(), details.join(", "))
        })
        .collect();
    if listing.is_empty() {
        listing.push("(none yet)".into());
    }
    Ok(format!(
        "[qm] This Claude Code session ({session_id}) is not linked to a work item. Project baseline from cwd: {project}.

Open work items:
{items}

Linking protocol — link once the session's purpose is clear from the first message or two, and re-link the moment its work moves to a different item (attach and new both move the session):
- Continues one of the items above → qm attach <slug> --session {session_id}
- Substantial new work → qm new \"Title\" --project \"{project}\" --session {session_id} --description \"...\" — but sanity-check the project against the topic first; if it seems wrong, ask the user instead of guessing.
- Trivial or throwaway (quick question, one-off lookup) → do nothing, and don't mention any of this.
After linking, keep the work item current: qm note <slug> \"...\" at meaningful milestones.
",
        items = listing.join("\n")
    ))
}
