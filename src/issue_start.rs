//! Any GitHub issue as a work item with its own worktree (`IssueStart`): preview it (read-only —
//! `gh issue view`), then start it: an `issue/<n>-<title>` branch off freshly fetched origin, its
//! worktree, the item linked to it. qm only reads GitHub.

use serde_json::{json, Value};

use crate::command_runner;
use crate::ruby_text::strip;
use crate::work_item::WorkItem;

const BRANCH_SLUG_MAX: usize = 40;

pub enum Fail {
    /// The `ArgumentError` Ruby raises.
    Arg(String),
    /// A value only Ruby reads exactly.
    Unported,
}

type R<T> = Result<T, Fail>;

fn arg<T>(m: impl Into<String>) -> R<T> {
    Err(Fail::Arg(m.into()))
}

pub fn kickoff(number: i64) -> String {
    format!("Read issue #{number} (`gh issue view {number} --comments`), summarise the problem, then run a grilling session (the grilling skill) with me on how to solve it. Don't change code yet.")
}

fn to_s(v: &Value) -> R<String> {
    match v {
        Value::Null => Ok(String::new()),
        Value::String(t) => Ok(t.clone()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Number(n) if n.is_i64() || n.is_u64() => Ok(n.to_string()),
        _ => Err(Fail::Unported),
    }
}

/// "#1234", "1234" → 1234; anything else is refused before gh runs.
pub fn number(value: &Value) -> R<i64> {
    let shown = to_s(value)?;
    let text = strip(&shown);
    let text = text.strip_prefix('#').unwrap_or(text);
    if text.is_empty() || text.starts_with('0') || !text.bytes().all(|b| b.is_ascii_digit()) {
        return arg(format!("an issue number is required, not '{shown}'"));
    }
    text.parse().map_err(|_| Fail::Unported)
}

fn repo_for(project: &str) -> R<(String, String)> {
    let Some(root) = crate::project_map::root_for(Some(project)) else { return arg(format!("no repo root configured for project '{project}'")) };
    let Some(repo) = crate::branch_diff::github_repo(&root) else { return arg(format!("project '{project}' has no GitHub origin remote — qm can't tell which repo its issues are in")) };
    Ok((root, repo))
}

fn first_text(result: &command_runner::CmdResult, fallback: &str) -> String {
    [&result.err, &result.out].iter().map(|t| strip(t)).find(|t| !t.is_empty()).unwrap_or(fallback).to_string()
}

fn fetch_issue(root: &str, repo: &str, number: i64) -> R<Value> {
    let result = command_runner::call_in(&["gh", "issue", "view", &number.to_string(), "--repo", repo, "--json", "number,title,state,url"], root);
    if !result.success {
        return arg(format!("no issue #{number} in {repo}: {}", first_text(&result, "gh issue view failed")));
    }
    match serde_json::from_str::<Value>(&result.out) {
        Ok(v) if v.is_object() => Ok(v),
        Ok(_) => Err(Fail::Unported),
        Err(_) => arg(format!("gh issue view #{number} did not answer JSON")),
    }
}

fn issue_branch(branch: &str, number: i64) -> bool {
    branch == format!("issue/{number}") || branch.starts_with(&format!("issue/{number}-"))
}

/// issue/<n>-<slug of the title>, the slug cut at a word boundary so the branch stays readable.
fn branch_name(number: i64, title: &str) -> String {
    let slug = WorkItem::slugify(title);
    let mut words: Vec<&str> = slug.split('-').collect();
    while words.last() == Some(&"") {
        words.pop();
    }
    let mut kept: Vec<&str> = vec![];
    for word in words {
        let mut joined = kept.clone();
        joined.push(word);
        if !kept.is_empty() && joined.join("-").chars().count() > BRANCH_SLUG_MAX {
            break;
        }
        kept.push(word);
    }
    let slug = kept.join("-");
    if slug.is_empty() { format!("issue/{number}") } else { format!("issue/{number}-{slug}") }
}

fn reusable_branch(root: &str, number: i64) -> Option<String> {
    let listing = crate::git::git(Some(root), &["for-each-ref", "--format=%(refname:short)", "refs/heads/issue/"]).unwrap_or_default();
    let linked: Vec<String> = WorkItem::all().unwrap_or_default().iter().flat_map(|i| i.branches()).collect();
    listing.split('\n').find(|b| issue_branch(b, number) && !linked.contains(&b.to_string())).map(String::from)
}

/// The item that already holds the issue: an unresolved one wins, then one holding the branch.
fn existing_item(project: &str, number: i64, url: &str) -> Option<WorkItem> {
    let pattern = (!url.is_empty()).then(|| regex::Regex::new(&format!("{}(?:[^0-9]|$)", regex::escape(url))).unwrap());
    let mut best: Option<((u8, u8), WorkItem)> = None;
    for item in WorkItem::all().unwrap_or_default() {
        let by_branch = item.project().as_deref() == Some(project) && item.branches().iter().any(|b| issue_branch(b, number));
        let by_url = pattern.as_ref().is_some_and(|p| p.is_match(&item.description()));
        if !(by_branch || by_url) {
            continue;
        }
        let key = (u8::from(item.state().as_deref() == Some("resolved")), u8::from(!by_branch));
        if best.as_ref().is_none_or(|(k, _)| key < *k) {
            best = Some((key, item));
        }
    }
    best.map(|(_, i)| i)
}

/// What Start would do, as the JSON the GUI shows.
pub fn preview(number_value: &Value, project: &str) -> R<Value> {
    let n = number(number_value)?;
    let (root, repo) = repo_for(project)?;
    let issue = fetch_issue(&root, &repo, n)?;
    let title = to_s(&issue["title"])?;
    let state = to_s(&issue["state"])?;
    let url = to_s(&issue["url"])?;
    let existing = existing_item(project, n, &url);
    let reuse = if existing.is_some() { None } else { reusable_branch(&root, n) };
    let branch = match (&existing, &reuse) {
        (Some(_), _) => Value::Null,
        (None, Some(r)) => json!(r),
        (None, None) => json!(branch_name(n, &title)),
    };
    let slug = match &existing {
        Some(e) => e.slug.clone(),
        None => WorkItem::slugify(&format!("{title} (#{n})")),
    };
    Ok(json!({
        "number": n, "title": title, "state": state.to_lowercase(), "closed": state.eq_ignore_ascii_case("closed"), "url": url,
        "project": project, "repo": repo, "branch": branch, "reuse_branch": reuse.is_some(), "slug": slug,
        "existing": existing.map(|e| json!({"slug": e.slug, "title": e.title(), "state": e.state()})).unwrap_or(Value::Null),
    }))
}

fn default_branch(root: &str, repo: &str) -> R<String> {
    if let Some(r) = crate::git::git(Some(root), &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]).filter(|r| !r.is_empty()) {
        return Ok(r.strip_prefix("origin/").unwrap_or(&r).to_string());
    }
    let result = command_runner::call_in(&["gh", "repo", "view", repo, "--json", "defaultBranchRef", "--jq", ".defaultBranchRef.name"], root);
    let name = strip(&result.out).to_string();
    if !(result.success && !name.is_empty()) {
        return arg(format!("can't tell {repo}'s default branch: {}", strip(&result.err)));
    }
    Ok(name)
}

fn run_checked(argv: &[&str], label: &str, root: &str) -> R<()> {
    let result = command_runner::call_in(argv, root);
    if result.success {
        return Ok(());
    }
    arg(format!("{label} failed: {}", first_text(&result, "exit status non-zero")))
}

/// Creates the worktree first, then the item: a retry after a failed item step reuses the branch.
pub fn start(preview: &Value, from: Option<&str>, progress: &dyn Fn(&str)) -> R<(WorkItem, String)> {
    if let Some(existing) = preview["existing"].as_object() {
        return arg(format!("#{} is already {}", preview["number"], existing["slug"].as_str().unwrap_or("")));
    }
    let project = to_s(&preview["project"])?;
    let n = preview["number"].as_i64().ok_or(Fail::Unported)?;
    let title = format!("{} (#{n})", to_s(&preview["title"])?);
    let slug = WorkItem::slugify(&title);
    if WorkItem::path_for(&slug).exists() {
        return arg(format!("work item '{slug}' already exists"));
    }
    let (root, repo) = repo_for(&project)?;
    let branch = to_s(&preview["branch"])?;
    let target = if preview["reuse_branch"] == Value::Bool(true) {
        progress("opening the worktree…");
        crate::workspace::materialize_branch(&project, &branch, None)
    } else {
        let default = default_branch(&root, &repo)?;
        progress(&format!("fetching origin/{default}…"));
        run_checked(&["git", "fetch", "origin", &default], "git fetch", &root)?;
        progress("creating the worktree…");
        crate::workspace::materialize_branch(&project, &branch, Some(&format!("origin/{default}")))
    }
    .map_err(Fail::Arg)?;
    let description = format!("{}\n\n{}", to_s(&preview["url"])?, to_s(&preview["title"])?);
    let created = WorkItem::create(&title, &project, None, None, Some(&description), None).map_err(|_| Fail::Unported)?;
    let mut item = WorkItem::find(&created.slug).map_err(|_| Fail::Unported)?;
    item.link_branch(&branch).map_err(|e| Fail::Arg(e.message))?;
    if let Some(from) = from {
        let mut source = WorkItem::find(from).map_err(|e| Fail::Arg(e.message))?;
        source.add_note(&format!("#{n} started as {}", item.slug), "history").map_err(|e| Fail::Arg(e.message))?;
    }
    let item = WorkItem::find(&item.slug).map_err(|_| Fail::Unported)?;
    Ok((item, target.path))
}
