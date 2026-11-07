//! The diff pane's "Since prompt" scope: what changed in the chat's worktree since one of its
//! session's prompts. The "before" is the tree the UserPromptSubmit hook recorded
//! (`prompt-snapshots.jsonl`); the "now" is the same worktree written as a tree through a throwaway
//! copy of its index, and the two trees are diffed. Branches that moved since the prompt come from
//! git's reflogs; a rebase since the prompt is hidden by replaying the prompt's tree onto the new
//! base with `git merge-tree`. A port of `PromptDiff` and `PromptSnapshots`; anything it can't judge
//! as Ruby does is `Unported`.

use std::process::{Command, Stdio};

use regex::Regex;
use serde_json::{json, Map, Value};

use crate::branch_diff::Failure;
use crate::cli_format::short_path;
use crate::gh_stack;
use crate::session_log::{Session, SessionLog};
use crate::unified_diff::UnifiedDiff;
use crate::work_item::WorkItem;

const MAX_LINES: i64 = 20_000;
const PROMPT_CHARS: usize = 80;
const SLACK: f64 = 3.0;
const INTERRUPT_MARK: &str = "[Request interrupted by user";
const TRUNK_REFS: [&str; 4] = ["origin/main", "main", "origin/master", "master"];

type R<T> = Result<T, Failure>;

/// Ruby's `git` helper of this class: stdout unstripped and scrubbed, None on failure; `GIT_DIR`,
/// `GIT_WORK_TREE` and `GIT_INDEX_FILE` are cleared unless given.
fn git(path: &str, args: &[&str], index: Option<&str>) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.args(["-c", "core.quotepath=off", "-C", path]).args(args).env_remove("GIT_DIR").env_remove("GIT_WORK_TREE").env_remove("GIT_INDEX_FILE").stdin(Stdio::null()).stderr(Stdio::null());
    if let Some(i) = index {
        cmd.env("GIT_INDEX_FILE", i);
    }
    let out = cmd.output().ok()?;
    out.status.success().then(|| scrub(&out.stdout))
}

fn scrub(bytes: &[u8]) -> String {
    let mut text = String::new();
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push('?');
        }
    }
    text
}

fn strip(s: Option<String>) -> Option<String> {
    s.map(|s| crate::ruby_text::strip(&s).to_string())
}

/// `Time.iso8601(value).to_f`: `Ok(None)` for what Ruby rejects, `Err` for strings only it accepts.
fn time(value: &str) -> R<Option<f64>> {
    if let Some(t) = crate::clock::parse_f64(value) {
        return Ok(Some(t));
    }
    if Regex::new(r"\A[0-9]{4}-[0-9]{2}-[0-9]{2}").unwrap().is_match(value) {
        return Err(Failure::Unported);
    }
    Ok(None)
}

#[derive(Clone)]
struct Snapshot {
    tree: String,
    at: String,
    cwd: Option<String>,
    toplevel: Option<String>,
    prompt: Option<String>,
    prompt_at: Option<String>,
}

fn snapshots_for(session: &Session, log: &SessionLog) -> R<Vec<Snapshot>> {
    let path = crate::data_root::root().join("prompt-snapshots.jsonl");
    let Ok(bytes) = std::fs::read(&path) else { return Ok(vec![]) };
    let text = String::from_utf8(bytes).map_err(|_| Failure::Unported)?;
    let tree = Regex::new(r"\A[0-9a-fA-F]{40}([0-9a-fA-F]{24})?\z").unwrap();
    let mut out: Vec<Snapshot> = vec![];
    for line in text.split_inclusive('\n') {
        let Ok(Value::Object(e)) = serde_json::from_str::<Value>(line) else { continue };
        let t = match e.get("tree") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(s)) => s.clone(),
            Some(_) => return Err(Failure::Unported),
        };
        let at = match e.get("at") {
            Some(Value::String(s)) => s.clone(),
            _ => continue,
        };
        if !tree.is_match(&t) || time(&at)?.is_none() || e.get("session_id").and_then(Value::as_str) != Some(session.id.as_str()) {
            continue;
        }
        let text_of = |k: &str| match e.get(k) {
            Some(Value::String(s)) => Ok(Some(s.clone())),
            None | Some(Value::Null) => Ok(None),
            Some(_) => Err(Failure::Unported),
        };
        out.push(Snapshot { tree: t, at, cwd: text_of("cwd")?, toplevel: text_of("toplevel")?, prompt: None, prompt_at: None });
    }
    if out.is_empty() {
        return Ok(out);
    }
    let recorded = log.session_for(&session.id).and_then(|s| s.transcript());
    let transcript = crate::transcripts::locate(Some(&session.id), recorded.as_ref().and_then(|p| p.to_str()));
    label(&mut out, &prompts(transcript.as_deref())?)?;
    Ok(out)
}

/// What the user typed, in transcript order: (text, timestamp, time).
fn prompts(transcript: Option<&std::path::Path>) -> R<Vec<(String, String, f64)>> {
    let Some(path) = transcript.filter(|p| p.exists()) else { return Ok(vec![]) };
    let text = String::from_utf8(std::fs::read(path).map_err(|_| Failure::Unported)?).map_err(|_| Failure::Unported)?;
    let mut out = vec![];
    for line in text.split_inclusive('\n') {
        let Ok(Value::Object(e)) = serde_json::from_str::<Value>(line) else { continue };
        let truthy = |k: &str| !matches!(e.get(k), None | Some(Value::Null) | Some(Value::Bool(false)));
        if e.get("type").and_then(Value::as_str) != Some("user") || truthy("isSidechain") || truthy("isCompactSummary") {
            continue;
        }
        let prompt = crate::transcripts::prompt_text(&e).map_err(|_| Failure::Unported)?;
        let ts = match e.get("timestamp") {
            Some(Value::String(s)) => Some(s.clone()),
            None | Some(Value::Null) => None,
            Some(_) => return Err(Failure::Unported),
        };
        let at = match &ts {
            Some(s) => time(s)?,
            None => None,
        };
        let (Some(prompt), Some(ts), Some(at)) = (prompt, ts, at) else { continue };
        if prompt.starts_with(INTERRUPT_MARK) {
            continue;
        }
        out.push((prompt, ts, at));
    }
    Ok(out)
}

/// Each snapshot owns the prompts from its own `at` up to the next snapshot's (both less SLACK),
/// and takes the first of them.
fn label(snapshots: &mut [Snapshot], prompts: &[(String, String, f64)]) -> R<()> {
    let mut starts = vec![];
    for s in snapshots.iter() {
        starts.push(time(&s.at)?.unwrap_or(0.0) - SLACK);
    }
    for i in 0..snapshots.len() {
        let from = starts[i];
        let upto = starts.get(i + 1).copied();
        let Some((text, ts, _)) = prompts.iter().find(|(_, _, t)| *t >= from && upto.is_none_or(|u| *t < u)) else { continue };
        let first = crate::ruby_text::strip(text.split_inclusive('\n').next().unwrap_or("")).to_string();
        let clipped = if first.chars().count() > PROMPT_CHARS { format!("{}…", crate::ruby_text::rstrip(&first.chars().take(PROMPT_CHARS).collect::<String>())) } else { first };
        snapshots[i].prompt = Some(clipped);
        snapshots[i].prompt_at = Some(ts.clone());
    }
    Ok(())
}

fn chat_session(item: &WorkItem, log: &SessionLog) -> R<Option<Session>> {
    let sessions: Vec<Session> = log.item_sessions(item).map_err(|_| Failure::Unported)?.into_iter().filter(|s| s.captured).collect();
    let live: Vec<&Session> = sessions.iter().filter(|s| s.live).collect();
    let tmux = crate::tmux::Tmux::new();
    let pane = if tmux.live(&item.slug) { tmux.pane_pid(&item.slug) } else { None };
    let found = pane.and_then(|p| live.iter().find(|s| s.pid().is_some_and(|pid| crate::chats_commands::process_lineage(pid).contains(&p))));
    Ok(found.map(|s| (*s).clone()).or_else(|| live.first().map(|s| (*s).clone())).or_else(|| sessions.first().cloned()))
}

/// `Integer(n, 10, exception: false)`.
fn base10(s: &str) -> Option<i64> {
    let t = s.trim_matches(|c: char| c.is_ascii_whitespace());
    let digits = t.strip_prefix(['+', '-']).unwrap_or(t);
    let ok = !digits.is_empty() && !digits.starts_with('_') && !digits.ends_with('_') && !digits.contains("__") && digits.bytes().all(|b| b.is_ascii_digit() || b == b'_');
    if !ok {
        return None;
    }
    let n: i64 = digits.replace('_', "").parse().ok()?;
    Some(if t.starts_with('-') { -n } else { n })
}

/// Reflogs as `ref → [(sha, unix time)]`, newest first, refs in order of first appearance.
fn reflogs(path: &str, refs: &[&str]) -> Vec<(String, Vec<(String, i64)>)> {
    let mut args = vec!["log", "--walk-reflogs", "--date=unix", "--format=%H %gD"];
    args.extend(refs);
    let out = git(path, &args, None).unwrap_or_default();
    let mut logs: Vec<(String, Vec<(String, i64)>)> = vec![];
    for line in out.split_inclusive('\n') {
        let line = crate::ruby_text::strip(line);
        let (sha, selector) = line.split_once(' ').unwrap_or((line, ""));
        let (reference, time) = match selector.rfind("@{") {
            Some(i) => (&selector[..i], &selector[i + 2..]),
            None => ("", selector),
        };
        let Some(time) = base10(time.strip_suffix('}').unwrap_or(time)) else { continue };
        if !refs.contains(&reference) {
            continue;
        }
        match logs.iter_mut().find(|(r, _)| r == reference) {
            Some((_, v)) => v.push((sha.to_string(), time)),
            None => logs.push((reference.to_string(), vec![(sha.to_string(), time)])),
        }
    }
    logs
}

struct Moved {
    branch: String,
    commits: i64,
    from: String,
    to: String,
    moved: i64,
}

fn count(path: &str, from: &str, to: &str) -> i64 {
    git(path, &["rev-list", "--count", &format!("{from}..{to}")], None).map(|s| crate::ruby_text::strip(&s).parse().unwrap_or(0)).unwrap_or(0)
}

fn moved_branches(path: &str, at: &str) -> R<Vec<Moved>> {
    if !std::path::Path::new(path).is_dir() {
        return Ok(vec![]);
    }
    let Some(since) = time(at)? else { return Ok(vec![]) };
    let since = since as i64;
    let listing = git(path, &["for-each-ref", "--format=%(refname)", "refs/heads"], None).unwrap_or_default();
    let refs: Vec<&str> = listing.split('\n').filter(|r| !r.is_empty()).collect();
    if refs.is_empty() {
        return Ok(vec![]);
    }
    let mut out: Vec<Moved> = vec![];
    for (reference, entries) in reflogs(path, &refs) {
        if entries[0].1 <= since {
            continue;
        }
        let from = entries.iter().find(|(_, t)| *t <= since).unwrap_or_else(|| entries.last().unwrap()).0.clone();
        let Some(to) = strip(git(path, &["rev-parse", "--verify", "--quiet", &format!("{reference}^{{commit}}")], None)) else { continue };
        out.push(Moved { branch: reference.strip_prefix("refs/heads/").unwrap_or(&reference).to_string(), commits: count(path, &from, &to), from, to, moved: entries[0].1 });
    }
    out.sort_by_key(|m| -m.moved);
    Ok(out)
}

fn head_at(path: &str, at: &str) -> R<Option<String>> {
    let Some(since) = time(at)? else { return Ok(None) };
    let since = since as i64;
    Ok(reflogs(path, &["HEAD"]).into_iter().next().and_then(|(_, e)| e.into_iter().find(|(_, t)| *t <= since).map(|(sha, _)| sha)))
}

fn candidates(path: &str, branch: Option<&str>, root: Option<&str>) -> Vec<String> {
    let parent = branch.and_then(|b| root.filter(|r| std::path::Path::new(r).is_dir()).and_then(|r| gh_stack::for_root(r).parent_of(b)));
    let mut refs: Vec<String> = parent.map(|p| vec![p.clone(), format!("origin/{p}")]).unwrap_or_default();
    refs.extend(TRUNK_REFS.iter().map(|s| s.to_string()));
    refs.into_iter()
        .filter(|r| !branch.is_some_and(|b| r == b || *r == format!("origin/{b}")))
        .filter(|r| git(path, &["rev-parse", "--verify", "--quiet", &format!("{r}^{{commit}}")], None).is_some())
        .collect()
}

struct Move {
    reference: String,
    from: String,
    to: String,
}

fn base_move(path: &str, branch: Option<&str>, old_head: &str, new_head: &str, root: Option<&str>) -> Option<Move> {
    for reference in candidates(path, branch, root) {
        let from = strip(git(path, &["merge-base", &reference, old_head], None));
        let to = strip(git(path, &["merge-base", &reference, new_head], None));
        let (Some(from), Some(to)) = (from, to) else { continue };
        if from == to || to == new_head {
            continue;
        }
        if git(path, &["merge-base", "--is-ancestor", &from, &to], None).is_none() {
            continue;
        }
        return Some(Move { reference, from, to });
    }
    None
}

/// `before` with the base's move applied: `(Some(tree), [])`, a conflict `(None, paths)`, else `(None, [])`.
fn replay(path: &str, from: &str, to: &str, before: &str) -> (Option<String>, Vec<String>) {
    let out = Command::new("git")
        .args(["-c", "core.quotepath=off", "-C", path, "merge-tree", "--write-tree", "--name-only", "--no-messages", &format!("--merge-base={from}"), to, before])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let Ok(out) = out else { return (None, vec![]) };
    let text = scrub(&out.stdout);
    let mut lines: Vec<&str> = text.split('\n').collect();
    while lines.last() == Some(&"") {
        lines.pop();
    }
    if out.status.success() && !lines.is_empty() {
        return (Some(lines[0].to_string()), vec![]);
    }
    if out.status.code() != Some(1) {
        return (None, vec![]);
    }
    (None, lines.iter().skip(1).take_while(|l| !l.is_empty()).map(|l| l.to_string()).collect())
}

/// `(the tree to diff from, payload keys)`.
fn unrebased(path: &str, branch: Option<&str>, old_head: Option<&str>, new_head: Option<&str>, before: &str, hide: bool, root: Option<&str>) -> (String, Vec<(&'static str, Value)>) {
    let moved = match (old_head, new_head) {
        (Some(o), Some(n)) if o != n => base_move(path, branch, o, n, root),
        _ => None,
    };
    let Some(m) = moved else { return (before.to_string(), vec![]) };
    let commits = count(path, &m.from, &m.to);
    let info = |hidden: bool| json!({"onto": m.reference, "commits": commits, "hidden": hidden});
    if !hide {
        return (before.to_string(), vec![("rebase", info(false))]);
    }
    let (tree, conflicts) = replay(path, &m.from, &m.to, before);
    if let Some(t) = tree {
        return (t, vec![("rebase", info(true))]);
    }
    let mut seen: Vec<&String> = vec![];
    for c in &conflicts {
        if !seen.contains(&c) {
            seen.push(c);
        }
    }
    let files = if conflicts.is_empty() { "git merge-tree failed".to_string() } else { format!("{} conflict", seen.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")) };
    (before.to_string(), vec![("rebase", info(false)), ("rebaseNotice", json!(format!("Couldn't hide the rebase: {files}.")))])
}

/// The worktree as a tree object, written the way hooks/snapshot-worktree.sh does.
fn tree_now(path: &str) -> Option<String> {
    let index = strip(git(path, &["rev-parse", "--path-format=absolute", "--git-path", "index"], None))?;
    let dir = std::env::temp_dir().join(format!("qm-prompt-diff-{}-{:?}", std::process::id(), std::thread::current().id()));
    std::fs::create_dir_all(&dir).ok()?;
    let tmp = dir.join("index");
    if std::path::Path::new(&index).is_file() {
        std::fs::copy(&index, &tmp).ok()?;
    }
    let t = tmp.to_string_lossy().into_owned();
    let tree = git(path, &["add", "-A"], Some(&t)).and_then(|_| strip(git(path, &["write-tree"], Some(&t))));
    let _ = std::fs::remove_dir_all(&dir);
    tree
}

fn merge_into(result: &mut Map<String, Value>, extra: Vec<(&'static str, Value)>) {
    for (k, v) in extra {
        result.insert(k.into(), v);
    }
}

pub struct Params {
    pub prompt: Option<String>,
    pub moved: Option<String>,
    pub rebase: bool,
}

pub fn call(item: &WorkItem, p: &Params) -> R<Map<String, Value>> {
    let log = SessionLog::read();
    let session = chat_session(item, &log)?;
    let snapshots = match &session {
        Some(s) => snapshots_for(s, &log)?,
        None => vec![],
    };
    let prompts: Vec<Value> = match &session {
        Some(s) => snapshots.iter().enumerate().map(|(i, sn)| json!({"id": format!("{}:{}", s.id, i + 1), "prompt": sn.prompt, "at": sn.prompt_at.clone().unwrap_or_else(|| sn.at.clone())})).rev().collect(),
        None => vec![],
    };
    let mut result = Map::new();
    for (k, v) in [
        ("scope", json!("prompt")),
        ("branches", json!([])),
        ("session", json!(session.as_ref().map(|s| s.id.clone()))),
        ("prompts", Value::Array(prompts)),
        ("movedBranches", json!([])),
        ("movedBranch", Value::Null),
        ("files", json!([])),
        ("truncated", json!(false)),
        ("lineLimit", json!(MAX_LINES)),
    ] {
        result.insert(k.into(), v);
    }
    if snapshots.is_empty() {
        let notice = if session.is_none() {
            "This item has no captured chat session to diff from.".to_string()
        } else {
            "No prompt snapshots for this chat's session yet: they start with the next prompt once the snapshot hook is installed (qm install-hooks), and only in a git worktree.".to_string()
        };
        result.insert("notice".into(), json!(notice));
        return Ok(result);
    }
    let session = session.unwrap();
    let wanted = p.prompt.clone().unwrap_or_default();
    let (id, n) = wanted.split_once(':').unwrap_or((wanted.as_str(), ""));
    let n = match base10(n) {
        Some(n) if id == session.id && n >= 1 && n as usize <= snapshots.len() => n as usize,
        _ => snapshots.len(),
    };
    let snapshot = &snapshots[n - 1];
    let path = snapshot.toplevel.clone().or_else(|| snapshot.cwd.clone());
    let path_text = path.clone().unwrap_or_default();
    let label = match &snapshot.prompt {
        Some(p) => format!("before “{p}”"),
        None => format!("before the prompt at {}", snapshot.at),
    };
    result.insert("prompt".into(), json!(format!("{}:{n}", session.id)));
    result.insert("path".into(), json!(path));
    result.insert("short".into(), json!(short_path(Some(&path_text))));
    result.insert("base".into(), json!({"ref": label, "kind": "prompt", "sha": snapshot.tree}));
    let branches = match path.as_deref() {
        Some(pa) => moved_branches(pa, &snapshot.at)?,
        None => vec![],
    };
    result.insert("movedBranches".into(), Value::Array(branches.iter().map(|b| json!({"branch": b.branch, "commits": b.commits})).collect()));
    let root = crate::project_map::root_for(item.project().as_deref());
    let moved = branches.iter().find(|b| Some(b.branch.as_str()) == p.moved.as_deref());
    let (path, root) = (path.unwrap_or_default(), root.as_deref());
    match moved {
        Some(b) => moved_diff_into(&mut result, &path, b, p.rebase, root),
        None => diff_into(&mut result, &path, snapshot, p.rebase, root)?,
    }
    Ok(result)
}

fn diff_into(result: &mut Map<String, Value>, path: &str, snapshot: &Snapshot, hide: bool, root: Option<&str>) -> R<()> {
    let shown = short_path(Some(path));
    let notice = |result: &mut Map<String, Value>, text: String| {
        result.insert("notice".into(), json!(text));
    };
    let has_path = snapshot.toplevel.is_some() || snapshot.cwd.is_some();
    if !has_path || !std::path::Path::new(path).is_dir() {
        notice(result, format!("{shown} is gone — nothing to diff."));
        return Ok(());
    }
    if git(path, &["cat-file", "-e", &format!("{}^{{tree}}", snapshot.tree)], None).is_none() {
        notice(result, "That prompt's snapshot is gone: git pruned it (unreferenced objects go after about two weeks).".into());
        return Ok(());
    }
    let Some(now) = tree_now(path) else {
        notice(result, format!("Could not snapshot {shown} as it is now."));
        return Ok(());
    };
    let head = strip(git(path, &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"], None));
    let branch = strip(git(path, &["symbolic-ref", "--quiet", "--short", "HEAD"], None));
    let old_head = head_at(path, &snapshot.at)?;
    let (before, rebase) = unrebased(path, branch.as_deref(), old_head.as_deref(), head.as_deref(), &snapshot.tree, hide, root);
    let mut parser = UnifiedDiff::new(MAX_LINES);
    let files = parser.parse(&git(path, &["diff", "--no-color", "--no-ext-diff", "-M", &before, &now, "--"], None).unwrap_or_default());
    result.insert("files".into(), Value::Array(files));
    result.insert("truncated".into(), json!(parser.truncated));
    result.insert("oldRev".into(), json!(before));
    merge_into(result, rebase);
    Ok(())
}

fn moved_diff_into(result: &mut Map<String, Value>, path: &str, b: &Moved, hide: bool, root: Option<&str>) {
    let (before, rebase) = unrebased(path, Some(&b.branch), Some(&b.from), Some(&b.to), &format!("{}^{{tree}}", b.from), hide, root);
    let mut parser = UnifiedDiff::new(MAX_LINES);
    let files = parser.parse(&git(path, &["diff", "--no-color", "--no-ext-diff", "-M", &before, &b.to, "--"], None).unwrap_or_default());
    result.insert("movedBranch".into(), json!(b.branch));
    result.insert("branch".into(), json!(b.branch));
    result.insert("files".into(), Value::Array(files));
    result.insert("truncated".into(), json!(parser.truncated));
    result.insert("oldRev".into(), json!(strip(git(path, &["rev-parse", "--verify", "--quiet", &before], None))));
    result.insert("base".into(), json!({"ref": format!("{} at the prompt", b.branch), "kind": "prompt-branch", "sha": b.from}));
    merge_into(result, rebase);
}
