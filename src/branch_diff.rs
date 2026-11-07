//! What an item's branch changes, for the chat view's diff pane: read-only git in the branch's
//! worktree (a ref-only branch's: the project root), never a checkout or a fetch. Ported for the
//! `branch`, `commits` and `uncommitted` scopes of an ordinary item; the PR scope, the prompt scope
//! and Review items are Ruby's (`Unported`). Key order of the payload is Ruby's, since the page and
//! the tests read bytes.

use regex::Regex;
use serde_json::{json, Map, Value};

use crate::cli_format::short_path;
use crate::git::{self, raw};
use crate::gh_stack;
use crate::session_log::SessionLog;
use crate::stack_layers;
use crate::tmux::Tmux;
use crate::unified_diff::UnifiedDiff;
use crate::work_item::WorkItem;

const SCOPES: [&str; 4] = ["branch", "commits", "uncommitted", "pr"];
const MAX_COMMITS: usize = 200;
const MAX_LINES: i64 = 20_000;
const MAX_UNTRACKED: usize = 200;
const MAX_UNTRACKED_BYTES: u64 = 1_000_000;
const TRUNK_REFS: [&str; 4] = ["origin/main", "main", "origin/master", "master"];

pub enum Failure {
    /// Ruby's `ArgumentError` message.
    Arg(String),
    /// A scope or kind of item this port doesn't serve yet.
    Unported,
}

type R<T> = Result<T, Failure>;

fn arg<T>(m: impl Into<String>) -> R<T> {
    Err(Failure::Arg(m.into()))
}

pub struct Params {
    pub branch: Option<String>,
    pub scope: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub base: Option<String>,
    pub fresh: bool,
}

fn g(path: &str, args: &[&str]) -> Option<String> {
    git::git(Some(path), args)
}

fn is_ref(path: &str, r: &str) -> bool {
    g(path, &["rev-parse", "--verify", "--quiet", &format!("{r}^{{commit}}")]).is_some()
}

fn present(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|s| !s.is_empty())
}

/// `Quartermaster.github_repo`: "owner/name" of the checkout's origin, if it looks like one.
pub(crate) fn github_repo(root: &str) -> Option<String> {
    let remote = g(root, &["remote", "get-url", "origin"]).unwrap_or_default();
    Regex::new(r"[:/]([^/:]+/[^/]+?)(?:\.git)?/?\z").unwrap().captures(&remote).map(|c| c[1].to_string())
}

/// `Quartermaster.branch_worktrees`: branch → the worktree path it is checked out in.
pub fn branch_worktrees(root: &str) -> Vec<(String, String)> {
    let Some(listing) = g(root, &["worktree", "list", "--porcelain"]) else { return vec![] };
    let mut out: Vec<(String, String)> = vec![];
    let mut path: Option<String> = None;
    for line in listing.split('\n') {
        if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(p.to_string());
        }
        if let (Some(b), Some(p)) = (line.strip_prefix("branch refs/heads/"), &path) {
            match out.iter_mut().find(|(k, _)| k == b) {
                Some(slot) => slot.1 = p.clone(),
                None => out.push((b.to_string(), p.clone())),
            }
        }
    }
    out
}

/// `Launcher#start_cwd`: the worktree on the item's branch, else the project root, else where its
/// last session ran, else home.
pub(crate) fn start_cwd(item: &WorkItem) -> String {
    let root = crate::project_map::root_for(item.project().as_deref());
    if let Some(root) = &root {
        let worktrees = branch_worktrees(root);
        for branch in item.branches().iter().rev() {
            if let Some((_, p)) = worktrees.iter().find(|(b, _)| b == branch) {
                if std::path::Path::new(p).is_dir() {
                    return p.clone();
                }
            }
        }
    }
    if let Some(root) = root.as_ref().filter(|r| std::path::Path::new(r).is_dir()) {
        return root.clone();
    }
    let log = SessionLog::read();
    let last = item.session_ids().iter().filter_map(|id| log.last_start_for(Some(id)).and_then(|e| e.get("cwd").and_then(Value::as_str).map(String::from))).last();
    match last {
        Some(c) if std::path::Path::new(&c).is_dir() => c,
        _ => crate::data_root::home(),
    }
}

fn chat_cwd(item: &WorkItem) -> String {
    let tmux = Tmux::new();
    (tmux.live(&item.slug).then(|| tmux.pane_path(&item.slug)).flatten()).unwrap_or_else(|| start_cwd(item))
}

pub(crate) fn chat_branch(item: &WorkItem) -> Option<String> {
    let path = chat_checkout(item)?;
    g(&path, &["symbolic-ref", "--quiet", "--short", "HEAD"])
}

pub(crate) fn chat_checkout(item: &WorkItem) -> Option<String> {
    g(&chat_cwd(item), &["rev-parse", "--show-toplevel"])
}

fn in_repo(path: &str, root: &str) -> bool {
    let common = |dir: &str| -> Option<std::path::PathBuf> {
        let out = g(dir, &["rev-parse", "--git-common-dir"])?;
        std::fs::canonicalize(crate::data_root::expand_path(&if std::path::Path::new(&out).is_absolute() { out.clone() } else { format!("{dir}/{out}") })).ok()
    };
    matches!((common(path), common(root)), (Some(a), Some(b)) if a == b)
}

#[derive(Clone)]
struct Choice {
    branch: Option<String>,
    path: String,
    chat: bool,
    checkout: bool,
    reference: Option<String>,
}

impl Choice {
    fn json(&self) -> Value {
        let mut m = Map::new();
        m.insert("branch".into(), json!(self.branch));
        m.insert("path".into(), json!(self.path));
        m.insert("chat".into(), json!(self.chat));
        m.insert("checkout".into(), json!(self.checkout));
        if let Some(r) = &self.reference {
            m.insert("ref".into(), json!(r));
        }
        m.insert("short".into(), json!(short_path(Some(&self.path))));
        Value::Object(m)
    }
}

fn branch_ref(root: &str, branch: &str) -> Option<String> {
    if is_ref(root, &format!("refs/heads/{branch}")) {
        return Some(branch.to_string());
    }
    is_ref(root, &format!("refs/remotes/origin/{branch}")).then(|| format!("origin/{branch}"))
}

fn choices_for(item: &WorkItem, root: &str) -> Vec<Choice> {
    let chat_path = chat_checkout(item);
    let chat_branch = chat_path.as_deref().and_then(|p| g(p, &["symbolic-ref", "--quiet", "--short", "HEAD"]));
    let worktrees: Vec<(String, String)> = branch_worktrees(root).into_iter().filter(|(_, p)| std::path::Path::new(p).is_dir()).collect();
    let mut choices: Vec<Choice> = vec![];
    if let Some(cp) = chat_path.as_ref().filter(|p| in_repo(p, root)) {
        choices.push(Choice { branch: chat_branch, path: cp.clone(), chat: true, checkout: true, reference: None });
    }
    let branches = item.branches();
    for b in branches.iter().rev() {
        let Some((_, p)) = worktrees.iter().find(|(k, _)| k == b) else { continue };
        if choices.iter().any(|c| c.branch.as_deref() == Some(b)) {
            continue;
        }
        choices.push(Choice { branch: Some(b.clone()), path: p.clone(), chat: false, checkout: true, reference: None });
    }
    for b in branches.iter().rev() {
        if choices.iter().any(|c| c.branch.as_deref() == Some(b)) {
            continue;
        }
        let Some(r) = branch_ref(root, b) else { continue };
        choices.push(Choice { branch: Some(b.clone()), path: root.to_string(), chat: false, checkout: false, reference: Some(r) });
    }
    choices
}

fn pick(choices: &[Choice], branch: Option<&str>, item: &WorkItem) -> R<Choice> {
    if let Some(b) = branch.filter(|b| !b.is_empty()) {
        return match choices.iter().find(|c| c.branch.as_deref() == Some(b)) {
            Some(c) => Ok(c.clone()),
            None => arg(format!("{b} is not a linked branch of this item with a worktree or a ref")),
        };
    }
    let checkouts: Vec<&Choice> = choices.iter().filter(|c| c.checkout).collect();
    let issue: Vec<String> = item.branches().into_iter().filter(|b| b.starts_with("issue/")).collect();
    let newest = checkouts.iter().filter_map(|c| issue.iter().position(|b| Some(b) == c.branch.as_ref()).map(|i| (i, *c))).max_by_key(|(i, _)| *i).map(|(_, c)| c);
    match newest.or(checkouts.first().copied()).or(choices.first()) {
        Some(c) => Ok(c.clone()),
        None => arg("nothing to diff: no linked branch of the item is checked out anywhere or exists as a ref"),
    }
}

fn ref_head(root: &str, choice: &Choice) -> R<String> {
    let branch = choice.branch.clone().unwrap_or_default();
    let reference = choice.reference.clone().unwrap_or_default();
    let full = if reference == branch { format!("refs/heads/{branch}") } else { format!("refs/remotes/origin/{branch}") };
    match g(root, &["rev-parse", "--verify", "--quiet", &format!("{full}^{{commit}}")]) {
        Some(s) => Ok(s),
        None => arg(format!("{reference} no longer names a commit")),
    }
}

fn base_for(root: &str, path: &str, branch: Option<&str>, head: &str) -> R<Value> {
    let parent = branch.and_then(|b| gh_stack::for_root(root).parent_of(b));
    let parent_ref = parent.as_ref().and_then(|p| [p.clone(), format!("origin/{p}")].into_iter().find(|r| is_ref(path, r)));
    if let Some(pr) = &parent_ref {
        if let Some(sha) = g(path, &["merge-base", pr, head]) {
            return Ok(json!({"ref": pr, "kind": "stack", "sha": sha}));
        }
    }
    let trunk = crate::git_repo::GitRepo::new(path).trunk_refs().into_iter().next();
    let Some(trunk) = trunk else { return arg(format!("no base to diff against: none of {} exists", TRUNK_REFS.join(", "))) };
    match g(path, &["merge-base", &trunk, head]) {
        Some(sha) => Ok(json!({"ref": trunk, "kind": "trunk", "sha": sha})),
        None => arg(format!("{} shares no history with {trunk}", branch.unwrap_or("HEAD"))),
    }
}

fn span_base(root: &str, path: &str, branch: Option<&str>, head: &str, base: &Option<String>) -> Option<Value> {
    let (base, branch) = (present(base)?, branch?);
    let stacks = gh_stack::for_root(root);
    let found = stacks.stacks_for(&[branch.to_string()]);
    let stack = found.first()?;
    let names: Vec<&String> = stack.entries.iter().map(|e| &e.branch).collect();
    let below = names.iter().position(|n| n.as_str() == base);
    let trunk = stack.trunk.clone().or_else(|| crate::git_repo::GitRepo::new(path).trunk_refs().into_iter().next().map(|t| t.strip_prefix("origin/").unwrap_or(&t).to_string()));
    let own = names.iter().position(|n| n.as_str() == branch)?;
    let refs: Option<[String; 2]> = if below.is_some_and(|b| b < own) {
        Some([base.to_string(), format!("origin/{base}")])
    } else if trunk.as_deref() == Some(base) {
        Some([format!("origin/{base}"), base.to_string()])
    } else {
        None
    };
    let r = refs?.into_iter().find(|r| is_ref(path, r))?;
    let sha = g(path, &["merge-base", &r, head])?;
    Some(json!({"ref": r, "kind": "stack-span", "branch": base, "sha": sha}))
}

fn int_after(stat: &str, word: &str) -> i64 {
    Regex::new(&format!(r"([0-9]+) {word}")).unwrap().captures(stat).and_then(|c| c[1].parse().ok()).unwrap_or(0)
}

fn branch_commits(path: &str, base: &str, head: &str) -> Vec<Map<String, Value>> {
    let out = raw(path, &["log", "--reverse", "--shortstat", "--format=%x1e%H%x1f%h%x1f%cI%x1f%s", &format!("{base}..{head}")], &[0]).unwrap_or_default();
    out.split('\x1e')
        .filter_map(|record| {
            let (line, stat) = record.split_once('\n').unwrap_or((record, ""));
            let mut f = crate::ruby_text::strip(line).splitn(4, '\x1f');
            let sha = f.next().filter(|s| !s.is_empty())?;
            let (short, time, subject) = (f.next(), f.next(), f.next().unwrap_or(""));
            json!({"sha": sha, "short": short, "time": time, "subject": subject, "add": int_after(stat, "insertion"), "del": int_after(stat, "deletion")}).as_object().cloned()
        })
        .collect()
}

fn s_of<'a>(c: &'a Map<String, Value>, k: &str) -> &'a str {
    c.get(k).and_then(Value::as_str).unwrap_or("")
}

fn commit_range(path: &str, commits: &[Map<String, Value>], from: &Option<String>, to: &Option<String>) -> R<Option<Value>> {
    let from = crate::ruby_text::strip(from.as_deref().unwrap_or("")).to_string();
    let to = crate::ruby_text::strip(to.as_deref().unwrap_or("")).to_string();
    if from.is_empty() && to.is_empty() {
        return Ok(None);
    }
    let from = if from.is_empty() { to.clone() } else { from };
    let to = if to.is_empty() { from.clone() } else { to };
    let sha_re = Regex::new(r"\A[0-9a-fA-F]{4,64}\z").unwrap();
    let mut picked = vec![];
    for sha in [&from, &to] {
        if !sha_re.is_match(sha) {
            return arg(format!("{} is not a commit sha", crate::gui_server::inspect(sha)));
        }
        let full = g(path, &["rev-parse", "--verify", "--quiet", &format!("{sha}^{{commit}}")]);
        match commits.iter().find(|c| Some(s_of(c, "sha")) == full.as_deref()) {
            Some(c) => picked.push(c),
            None => return arg(format!("{sha} is not a commit of this branch")),
        }
    }
    let (first, last) = (picked[0], picked[1]);
    if !(s_of(first, "sha") == s_of(last, "sha") || raw(path, &["merge-base", "--is-ancestor", s_of(first, "sha"), s_of(last, "sha")], &[0]).is_some()) {
        return arg(format!("{} does not come before {}", s_of(first, "short"), s_of(last, "short")));
    }
    let label = if s_of(first, "sha") == s_of(last, "sha") { s_of(first, "short").to_string() } else { format!("{}..{}", s_of(first, "short"), s_of(last, "short")) };
    Ok(Some(json!({"from": s_of(first, "sha"), "to": s_of(last, "sha"), "label": label})))
}

fn commit_messages(path: &str, commits: &[Map<String, Value>], range: &Option<Value>) -> Vec<Value> {
    let Some(range) = range else { return vec![] };
    let first = commits.iter().position(|c| s_of(c, "sha") == range["from"].as_str().unwrap_or(""));
    let last = commits.iter().position(|c| s_of(c, "sha") == range["to"].as_str().unwrap_or(""));
    let (Some(first), Some(last)) = (first, last) else { return vec![] };
    let shas: Vec<String> = commits[first..=last].iter().map(|c| s_of(c, "sha").to_string()).collect();
    let mut args = vec!["show", "-s", "--format=%H%x1f%h%x1f%an%x1f%aI%x1f%B%x1e"];
    args.extend(shas.iter().map(String::as_str));
    let out = raw(path, &args, &[0]).unwrap_or_default();
    out.split('\x1e')
        .filter_map(|record| {
            let record = record.trim_start_matches('\n');
            let mut f = record.splitn(5, '\x1f');
            let (sha, short, author, date, body) = (f.next()?, f.next()?, f.next()?, f.next()?, f.next()?);
            Some(json!({"sha": sha, "short": short, "author": author, "date": date, "body": crate::ruby_text::rstrip(body)}))
        })
        .collect()
}

fn parent_of(path: &str, sha: &str) -> Option<String> {
    g(path, &["rev-parse", "--verify", "--quiet", &format!("{sha}^")]).or_else(|| g(path, &["hash-object", "-t", "tree", "/dev/null"]))
}

fn committed_diff(path: &str, from: &str, to: &str) -> String {
    committed_diff_with(path, from, to, &[])
}

fn committed_diff_with(path: &str, from: &str, to: &str, options: &[&str]) -> String {
    let mut args = vec!["diff", "--no-color", "--no-ext-diff", "-M"];
    args.extend(options);
    args.extend([from, to, "--"]);
    raw(path, &args, &[0]).unwrap_or_default()
}

fn untracked_diff(path: &str) -> String {
    let listing = raw(path, &["ls-files", "--others", "--exclude-standard", "-z"], &[0]).unwrap_or_default();
    listing
        .split('\0')
        .filter(|f| !f.is_empty() || false)
        .take(MAX_UNTRACKED)
        .filter_map(|file| {
            let full = std::path::Path::new(path).join(file);
            let meta = std::fs::metadata(&full).ok()?;
            if !meta.is_file() || meta.len() > MAX_UNTRACKED_BYTES {
                return None;
            }
            raw(path, &["diff", "--no-color", "--no-ext-diff", "--no-index", "--", "/dev/null", file], &[0, 1])
        })
        .collect()
}

fn working_tree_diff(path: &str, from: &str, untracked: &str) -> String {
    format!("{}{untracked}", raw(path, &["diff", "--no-color", "--no-ext-diff", "-M", from, "--"], &[0]).unwrap_or_default())
}

fn branch_files(path: &str, base: &str, parser: &mut UnifiedDiff) -> Vec<Value> {
    let untracked = untracked_diff(path);
    let mut files = parser.parse(&working_tree_diff(path, base, &untracked));
    let committed = UnifiedDiff::new(MAX_LINES).parse(&committed_diff(path, base, "HEAD"));
    let uncommitted = UnifiedDiff::new(MAX_LINES).parse(&working_tree_diff(path, "HEAD", &untracked));
    crate::uncommitted_marks::apply(&mut files, committed, uncommitted);
    files
}

/// `PrItems#review_pr`: a Review item's own PR, read from its description: (url, repo, number).
fn review_pr(item: &WorkItem) -> Option<(String, String, i64)> {
    let re = Regex::new(r"\AReview of pull request (https://github\.com/([^/\s]+/[^/\s]+)/pull/([0-9]+))").unwrap();
    let description = item.description();
    let m = re.captures(crate::ruby_text::strip(&description))?;
    item.title().starts_with("Review ").then(|| (m[1].to_string(), m[2].to_string(), m[3].parse().unwrap_or(0)))
}

/// The PRs view's cached list; `Err` when an entry is not an object (Ruby would raise on it).
fn cached_prs() -> R<Vec<Value>> {
    let text = std::fs::read_to_string(crate::data_root::root().join("pr-inbox.json")).unwrap_or_default();
    let Ok(Value::Object(cache)) = serde_json::from_str::<Value>(&text) else { return Ok(vec![]) };
    match cache.get("prs") {
        Some(Value::Array(prs)) if prs.iter().all(Value::is_object) => Ok(prs.clone()),
        Some(Value::Array(_)) => Err(Failure::Unported),
        _ => Ok(vec![]),
    }
}

/// The PR the PRs view's cache knows for this branch (never asks GitHub); a Review item's own PR
/// is always known.
fn pr_available(item: &WorkItem, repo: Option<&str>, branch: Option<&str>) -> R<bool> {
    if review_pr(item).is_some() {
        return Ok(true);
    }
    let (Some(repo), Some(branch)) = (repo, branch.filter(|b| !b.is_empty())) else { return Ok(false) };
    Ok(cached_prs()?.iter().any(|pr| pr["repo"].as_str().is_some_and(|r| r.eq_ignore_ascii_case(repo)) && pr["head"].as_str() == Some(branch)))
}

/// Where each of the item's GitHub drafts on this PR would post now (`ReviewPlan`), re-anchoring the
/// ones a push moved. Local git, but the re-anchoring rewrites the drafts file.
fn review_routes(item: &WorkItem, path: &str, pr: &Value, revs: &Value) -> R<Map<String, Value>> {
    let entries = crate::drafts::entries(&item.slug).ok_or(Failure::Unported)?;
    let number = lenient(&pr["number"]);
    let mine: Vec<Map<String, Value>> = entries
        .into_iter()
        .filter(|e| e.get("kind").and_then(Value::as_str) == Some("github-comment") && to_s(e.get("repo").unwrap_or(&Value::Null)).eq_ignore_ascii_case(pr["repo"].as_str().unwrap_or("")) && e.get("number") == Some(&json!(number)))
        .collect();
    if mine.is_empty() {
        return Ok(Map::new());
    }
    let plan = crate::review_plan::ReviewPlan::new(path, revs["baseRef"].as_str().unwrap_or(""), revs["headSha"].as_str().unwrap_or("")).map_err(Failure::Arg)?;
    crate::review_plan::reanchor(&plan, &mine, &item.slug).map_err(|e| match e {
        crate::review_anchors::E::Arg(m) => Failure::Arg(m),
        crate::review_anchors::E::Unsure => Failure::Unported,
    })
}

// --- the PR scope: PrRevisions, PrMemo ---

const MEMO_TTL: std::time::Duration = std::time::Duration::from_secs(60);
static MEMO: std::sync::Mutex<Vec<(String, std::time::Instant, Value)>> = std::sync::Mutex::new(Vec::new());
static FETCH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// `PrMemo#fetch`: the remembered value of `key` while younger than a minute and `usable`, else the
/// closure's (remembered unless null).
fn memo(key: &str, fresh: bool, usable: impl Fn(&Value) -> bool, f: impl FnOnce() -> R<Value>) -> R<Value> {
    let started = std::time::Instant::now();
    if !fresh {
        let held = MEMO.lock().unwrap().iter().find(|(k, at, _)| k == key && started.duration_since(*at) < MEMO_TTL).map(|(_, _, v)| v.clone());
        if let Some(v) = held.filter(|v| usable(v)) {
            return Ok(v);
        }
    }
    let value = f()?;
    let mut memo = MEMO.lock().unwrap();
    memo.retain(|(k, at, _)| k != key && started.duration_since(*at) < MEMO_TTL);
    if !value.is_null() {
        memo.push((key.to_string(), std::time::Instant::now(), value.clone()));
    }
    Ok(value)
}

fn label(pr: &Value) -> String {
    let repo = pr["repo"].as_str().unwrap_or("");
    let last = repo.split('/').collect::<Vec<_>>().into_iter().rev().skip_while(|p| p.is_empty()).next().unwrap_or("").to_string();
    format!("{last}#{}", to_s(&pr["number"]))
}

fn to_s(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// `gh pr view`: Null when gh finds no PR; any other failure is the pane's error.
fn gh_view(selector: &str, repo: &str) -> R<Value> {
    let result = crate::command_runner::call(&["gh", "pr", "view", selector, "--repo", repo, "--json", "number,url,title,headRefName,baseRefName,state"]);
    if !result.success {
        if Regex::new(r"(?i)no (open )?pull requests? found").unwrap().is_match(&result.err) {
            return Ok(Value::Null);
        }
        let shown = crate::ruby_text::strip(if crate::ruby_text::strip(&result.err).is_empty() { &result.out } else { &result.err }).chars().take(200).collect::<String>();
        return arg(format!("gh pr view {selector} failed: {shown}"));
    }
    let Ok(node) = serde_json::from_str::<Value>(&result.out) else { return arg(format!("gh pr view {selector} answered no JSON")) };
    if !node.is_object() {
        return Err(Failure::Unported);
    }
    Ok(json!({"repo": repo, "number": node["number"], "url": node["url"], "title": node["title"], "head": node["headRefName"], "base": node["baseRefName"], "state": node["state"]}))
}

/// `PrRevisions#find`: `{repo, number, url, title, head, base}` of the branch's open PR, or Null.
fn find_pr(item: &WorkItem, repo: Option<&str>, branch: Option<&str>, fresh: bool) -> R<Value> {
    let key = format!("find\u{1}{}\u{1}{}\u{1}{}", item.slug, repo.unwrap_or(""), branch.unwrap_or(""));
    memo(&key, fresh, |_| true, || {
        if let Some((url, review_repo, _)) = review_pr(item) {
            return match cached_prs()?.into_iter().find(|pr| pr["url"].as_str() == Some(url.as_str())) {
                Some(hit) => Ok(hit),
                None => gh_view(&url, &review_repo),
            };
        }
        let (Some(repo), Some(branch)) = (repo, branch.filter(|b| !b.is_empty())) else { return Ok(Value::Null) };
        if let Some(hit) = cached_prs()?.into_iter().find(|pr| pr["repo"].as_str().is_some_and(|r| r.eq_ignore_ascii_case(repo)) && pr["head"].as_str() == Some(branch)) {
            return Ok(hit);
        }
        let pr = gh_view(branch, repo)?;
        Ok(if pr["state"].as_str() == Some("OPEN") { pr } else { Value::Null })
    })
}

/// `String#to_i`.
fn lenient(v: &Value) -> i64 {
    match v {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        Value::String(s) => s.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0),
        _ => 0,
    }
}

/// `PrRevisions#fetch`: the PR's head into `refs/quartermaster/pr/<n>` and its base into
/// `origin/<base>`: `{headSha, baseSha, baseRef}`.
fn fetch_pr(path: &str, pr: &Value, fresh: bool) -> R<Value> {
    let number = lenient(&pr["number"]);
    let base = to_s(&pr["base"]);
    let usable_base = !base.is_empty() && !base.starts_with('-') && !base.contains("..") && base.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'/' | b'-'));
    if !(number > 0 && usable_base) {
        return arg(format!("{} has no usable base branch ({})", label(pr), crate::gui_server::inspect(&base)));
    }
    let reference = format!("refs/quartermaster/pr/{number}");
    let key = format!("fetch\u{1}{}\u{1}{number}\u{1}{base}", to_s(&pr["repo"]).to_lowercase());
    let held = |revs: &Value| {
        let args = ["rev-parse", "--quiet", &format!("{reference}^{{commit}}"), &format!("{}^{{commit}}", revs["baseSha"].as_str().unwrap_or(""))];
        g(path, &args).as_deref() == Some(&format!("{}\n{}", revs["headSha"].as_str().unwrap_or(""), revs["baseSha"].as_str().unwrap_or("")))
    };
    memo(&key, fresh, held, || fetch_now(path, pr, &reference, number, &base))
}

/// `PrRevisions#fetch` without a memo (the CLI and review posting always ask the network).
pub(crate) fn fetch_pr_uncached(path: &str, pr: &Value) -> Result<Value, Result<String, ()>> {
    let number = lenient(&pr["number"]);
    let base = to_s(&pr["base"]);
    let usable_base = !base.is_empty() && !base.starts_with('-') && !base.contains("..") && base.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'/' | b'-'));
    if !(number > 0 && usable_base) {
        return Err(Ok(format!("{} has no usable base branch ({})", label(pr), crate::gui_server::inspect(&base))));
    }
    let reference = format!("refs/quartermaster/pr/{number}");
    fetch_now(path, pr, &reference, number, &base).map_err(|f| match f {
        Failure::Arg(m) => Ok(m),
        Failure::Unported => Err(()),
    })
}

fn fetch_now(path: &str, pr: &Value, reference: &str, number: i64, base: &str) -> R<Value> {
    let result = {
        let _lock = FETCH_LOCK.lock().unwrap();
        crate::command_runner::call(&["git", "-C", path, "fetch", "--quiet", "--no-tags", "origin", &format!("+refs/pull/{number}/head:{reference}"), &format!("+refs/heads/{base}:refs/remotes/origin/{base}")])
    };
    if !result.success {
        return arg(format!("could not fetch {} from origin: {}", label(pr), crate::ruby_text::strip(&result.err).chars().take(200).collect::<String>()));
    }
    let Some(head) = g(path, &["rev-parse", "--verify", "--quiet", &format!("{reference}^{{commit}}")]) else { return arg(format!("{}'s head did not arrive in {reference}", label(pr))) };
    let Some(base_sha) = g(path, &["merge-base", &format!("origin/{base}"), &head]) else { return arg(format!("{} shares no history with origin/{base}", label(pr))) };
    Ok(json!({"headSha": head, "baseSha": base_sha, "baseRef": format!("origin/{base}")}))
}

/// A selected run shows parent(from)..to, but GitHub takes a comment anchored to `to` only on lines
/// of base..to: each shown line it won't take gets `"gh": false`.
fn mark_unanchorable(path: &str, base: &str, commit: &str, view_base: &str, files: &mut [Value]) -> R<()> {
    let anchors = crate::review_anchors::new(path, base, commit).map_err(Failure::Arg)?;
    for file in files {
        let file_path = file["path"].as_str().unwrap_or("").to_string();
        for hunk in file["hunks"].as_array_mut().into_iter().flatten() {
            for line in hunk["lines"].as_array_mut().into_iter().flatten() {
                let removed = line["t"] == "-";
                let (side, number) = if removed { ("old", line["o"].clone()) } else { ("new", line["n"].clone()) };
                let ok = anchors.anchorable(&file_path, &json!(side), &number, line["s"].as_str().unwrap_or(""), view_base).map_err(|e| match e {
                    crate::review_anchors::E::Arg(m) => Failure::Arg(m),
                    crate::review_anchors::E::Unsure => Failure::Unported,
                })?;
                if !ok {
                    line.as_object_mut().unwrap().insert("gh".into(), json!(false));
                }
            }
        }
    }
    Ok(())
}

/// The PR scope's part of the payload, in Ruby's key order; the parser is the caller's.
fn pr_scope(item: &WorkItem, repo: Option<&str>, path: &str, branch: Option<&str>, p: &Params, fresh: bool, parser: &mut UnifiedDiff) -> R<Map<String, Value>> {
    let pr = find_pr(item, repo, branch, fresh)?;
    if pr.is_null() {
        return arg(format!("{} has no open pull request", branch.unwrap_or("this item")));
    }
    if !repo.unwrap_or("").eq_ignore_ascii_case(pr["repo"].as_str().unwrap_or("")) {
        return arg(format!("{} is not in this project's repo (its origin is {})", to_s(&pr["url"]), repo.unwrap_or("unknown")));
    }
    let revs = fetch_pr(path, &pr, fresh)?;
    let (head, base_sha, base_ref) = (revs["headSha"].as_str().unwrap_or("").to_string(), revs["baseSha"].as_str().unwrap_or("").to_string(), revs["baseRef"].as_str().unwrap_or("").to_string());
    let commits = branch_commits(path, &base_sha, &head);
    let range = commit_range(path, &commits, &p.from, &p.to)?;
    let from_sha = match &range {
        Some(r) => parent_of(path, r["from"].as_str().unwrap_or("")),
        None => Some(base_sha.clone()),
    };
    let to = range.as_ref().and_then(|r| r["to"].as_str()).unwrap_or(&head).to_string();
    let mut files = parser.parse(&committed_diff_with(path, from_sha.as_deref().unwrap_or(""), &to, &[crate::review_anchors::GITHUB_HUNKS]));
    if range.is_some() {
        mark_unanchorable(path, &base_sha, &to, from_sha.as_deref().unwrap_or(""), &mut files)?;
    }
    let routes = review_routes(item, path, &pr, &revs)?;
    let tail: Vec<Value> = commits.iter().skip(commits.len().saturating_sub(MAX_COMMITS)).cloned().map(Value::Object).collect();
    let mut out = Map::new();
    out.insert("reviewRoutes".into(), Value::Object(routes));
    out.insert("base".into(), json!({"ref": base_ref, "kind": "pr", "sha": base_sha}));
    out.insert("pr".into(), json!({"repo": pr["repo"], "number": pr["number"], "url": pr["url"], "title": pr["title"], "headSha": head, "baseSha": base_sha}));
    out.insert("commits".into(), Value::Array(tail));
    out.insert("commitCount".into(), json!(commits.len()));
    out.insert("range".into(), range.clone().unwrap_or(Value::Null));
    out.insert("messages".into(), Value::Array(commit_messages(path, &commits, &range)));
    out.insert("anchor".into(), json!(to));
    out.insert("oldRev".into(), json!(from_sha));
    out.insert("files".into(), Value::Array(files));
    Ok(out)
}

pub fn call(item: &mut WorkItem, p: &Params) -> R<Map<String, Value>> {
    let review = review_pr(item).is_some();
    let scope = match present(&p.scope) {
        Some(s) => s.to_string(),
        None => (if review { "pr" } else { "branch" }).to_string(),
    };
    if !SCOPES.contains(&scope.as_str()) {
        return arg(format!("scope must be one of: {}", SCOPES.join(", ")));
    }
    if scope == "prompt" {
        return Err(Failure::Unported);
    }
    let project = item.project();
    let root = crate::project_map::root_for(project.as_deref()).filter(|r| std::path::Path::new(r).is_dir());
    let Some(root) = root else { return arg(format!("no repo root configured for project '{}'", project.unwrap_or_default())) };
    let _ = gh_stack::for_root(&root).link_into(item);
    let choices = choices_for(item, &root);
    // A Review item's PR needs no worktree: the main checkout fetches it.
    let choice = if scope == "pr" && review && choices.is_empty() && present(&p.branch).is_none() { None } else { Some(pick(&choices, p.branch.as_deref(), item)?) };
    let (path, branch, checkout) = match &choice {
        Some(c) => (c.path.clone(), c.branch.clone(), c.checkout),
        None => (root.clone(), None, false),
    };
    if !checkout && scope == "uncommitted" {
        return arg(format!("{} is not checked out anywhere: Uncommitted and Since prompt need a checkout", branch.clone().unwrap_or_else(|| "this item".into())));
    }
    let head = if checkout {
        "HEAD".to_string()
    } else {
        match &choice {
            Some(c) => ref_head(&root, c)?,
            None => String::new(),
        }
    };
    let mut parser = UnifiedDiff::new(MAX_LINES);
    let mut result = Map::new();
    result.insert("branch".into(), json!(branch));
    result.insert("path".into(), json!(path));
    result.insert("short".into(), json!(short_path(Some(&path))));
    result.insert("scope".into(), json!(scope));
    result.insert("checkout".into(), json!(checkout));
    result.insert("branches".into(), Value::Array(choices.iter().map(Choice::json).collect()));
    let repo = github_repo(&root);
    if scope != "uncommitted" {
        if let Some(l) = stack_layers::call(&root, branch.as_deref()) {
            result.insert("layers".into(), Value::Array(l.layers));
            result.insert("layerTrunk".into(), json!(l.trunk));
            result.insert("fileLayers".into(), Value::Object(l.file_layers));
        }
    }
    if scope == "pr" {
        let more = pr_scope(item, repo.as_deref(), &path, branch.as_deref(), p, p.fresh, &mut parser)?;
        result.extend(more);
        result.insert("prAvailable".into(), json!(true));
        result.insert("truncated".into(), json!(parser.truncated));
        result.insert("lineLimit".into(), json!(MAX_LINES));
        return Ok(result);
    }
    let base = if scope == "uncommitted" {
        json!({"ref": "HEAD", "kind": "head"})
    } else {
        match span_base(&root, &path, branch.as_deref(), &head, &p.base) {
            Some(b) => b,
            None => base_for(&root, &path, branch.as_deref(), &head)?,
        }
    };
    result.insert("base".into(), base.clone());
    result.insert("prAvailable".into(), json!(pr_available(item, repo.as_deref(), branch.as_deref())?));
    let base_sha = base["sha"].as_str().unwrap_or("").to_string();
    let files = match scope.as_str() {
        "branch" => {
            result.insert("oldRev".into(), json!(base_sha));
            if checkout { branch_files(&path, &base_sha, &mut parser) } else { parser.parse(&committed_diff(&path, &base_sha, &head)) }
        }
        "uncommitted" => {
            result.insert("oldRev".into(), json!(g(&path, &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])));
            parser.parse(&working_tree_diff(&path, "HEAD", &untracked_diff(&path)))
        }
        _ => {
            let commits = branch_commits(&path, &base_sha, &head);
            let range = commit_range(&path, &commits, &p.from, &p.to)?;
            let tail: Vec<Value> = commits.iter().skip(commits.len().saturating_sub(MAX_COMMITS)).cloned().map(Value::Object).collect();
            result.insert("commits".into(), Value::Array(tail));
            result.insert("commitCount".into(), json!(commits.len()));
            result.insert("range".into(), range.clone().unwrap_or(Value::Null));
            result.insert("messages".into(), Value::Array(commit_messages(&path, &commits, &range)));
            let old = match &range {
                Some(r) => parent_of(&path, r["from"].as_str().unwrap_or("")),
                None => Some(base_sha.clone()),
            };
            result.insert("oldRev".into(), json!(old));
            let to = range.as_ref().and_then(|r| r["to"].as_str()).unwrap_or(&head).to_string();
            parser.parse(&committed_diff(&path, old.as_deref().unwrap_or(""), &to))
        }
    };
    result.insert("files".into(), Value::Array(files));
    result.insert("truncated".into(), json!(parser.truncated));
    result.insert("lineLimit".into(), json!(MAX_LINES));
    Ok(result)
}

/// `BranchDiff#require_checkout`: the prompt scope diffs the chat's folder, which is no ref-only
/// branch's, so a requested branch that is one is refused.
pub fn require_checkout(item: &WorkItem, branch: Option<&str>) -> R<()> {
    let Some(branch) = branch.filter(|b| !b.is_empty()) else { return Ok(()) };
    let project = item.project();
    let Some(root) = crate::project_map::root_for(project.as_deref()).filter(|r| std::path::Path::new(r).is_dir()) else { return Ok(()) };
    match choices_for(item, &root).iter().find(|c| c.branch.as_deref() == Some(branch)) {
        Some(c) if !c.checkout => arg(format!("{branch} is not checked out anywhere: Uncommitted and Since prompt need a checkout")),
        _ => Ok(()),
    }
}

/// `BranchDiff#tip`: the branch and base the item's stats are measured on —
/// `{root, path, branch, ref, head, base}`.
pub fn tip(item: &mut WorkItem, branch: Option<&str>) -> R<Value> {
    let project = item.project();
    let root = crate::project_map::root_for(project.as_deref()).filter(|r| std::path::Path::new(r).is_dir());
    let Some(root) = root else { return arg(format!("no repo root configured for project '{}'", project.unwrap_or_default())) };
    let _ = gh_stack::for_root(&root).link_into(item);
    let choice = pick(&choices_for(item, &root), branch.filter(|b| !b.is_empty()), item)?;
    let Some(name) = choice.branch.clone() else { return arg(format!("no branch to measure: {} is detached", short_path(Some(&choice.path)))) };
    let path = choice.path.clone();
    let head = if choice.checkout {
        match g(&path, &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]) {
            Some(h) => h,
            None => return arg(format!("{name} has no commits yet")),
        }
    } else {
        ref_head(&root, &choice)?
    };
    let reference = choice.reference.clone().unwrap_or_else(|| name.clone());
    let base = base_for(&root, &path, Some(&name), &head)?;
    Ok(json!({"root": root, "path": path, "branch": name, "ref": reference, "head": head, "base": base}))
}
