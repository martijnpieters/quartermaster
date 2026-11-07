//! Local bookkeeping that a PR may only merge once an earlier PR or commit of the same repo is
//! deployed (`pr-prerequisites.json`), and the cached verdict on whether it is. Nothing here is
//! written to GitHub: `gh` is only asked read-only questions to resolve what the CLI was given, and
//! the deploy cache is read, never refreshed (that needs a thread that outlives the command).

use std::os::unix::io::AsRawFd;
use std::process::{Command, Stdio};

use regex::Regex;
use serde_json::{Map, Value};

use crate::data_root;
use crate::ruby_text::strip;

pub enum Fail {
    /// Ruby's `ArgumentError`: printed as `qm: <message>`.
    Arg(String),
    /// Something only Ruby words or reads exactly.
    Ruby,
}

pub type R<T> = Result<T, Fail>;

fn arg<T>(message: impl Into<String>) -> R<T> {
    Err(Fail::Arg(message.into()))
}

pub type Prs = Map<String, Value>;

fn pr_url_re() -> Regex {
    Regex::new(r"(?i)\Ahttps?://github\.com/([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)/pull/([0-9]+)/?\z").unwrap()
}

pub fn pr_url(repo: &str, number: &str) -> String {
    format!("https://github.com/{repo}/pull/{number}")
}

fn file() -> std::path::PathBuf {
    data_root::root().join("pr-prerequisites.json")
}

/// The store: every PR URL with its prerequisites, in insertion order. `Err` when the file holds a
/// shape Ruby would trip over (a list that isn't a list, an entry that isn't an object).
fn read() -> R<(Map<String, Value>, Prs)> {
    let empty = || (Map::from_iter([("v".to_string(), Value::from(1)), ("prs".to_string(), Value::Object(Map::new()))]), Map::new());
    let Ok(text) = std::fs::read_to_string(file()) else { return Ok(empty()) };
    let Ok(Value::Object(data)) = serde_json::from_str::<Value>(&text) else { return Ok(empty()) };
    let Some(Value::Object(prs)) = data.get("prs").cloned() else { return Ok(empty()) };
    for list in prs.values() {
        let Value::Array(entries) = list else { return Err(Fail::Ruby) };
        if entries.iter().any(|e| !e.is_object()) {
            return Err(Fail::Ruby);
        }
    }
    Ok((data, prs))
}

pub fn all() -> R<Prs> {
    Ok(read()?.1)
}

pub fn for_pr(url: &str) -> R<Vec<Map<String, Value>>> {
    Ok(match all()?.get(url) {
        Some(Value::Array(list)) => list.iter().filter_map(|e| e.as_object().cloned()).collect(),
        _ => vec![],
    })
}

/// Read-modify-write under the store's flock, then temp file + rename, as Ruby's `update`.
fn update<T>(f: impl FnOnce(&mut Prs) -> T) -> R<T> {
    let _ = data_root::ensure_setup();
    let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(data_root::root().join("pr-prerequisites.lock")).map_err(|_| Fail::Ruby)?;
    unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) };
    let (mut data, mut prs) = read()?;
    let result = f(&mut prs);
    data.insert("prs".into(), Value::Object(prs));
    data.insert("v".into(), Value::from(1));
    let temp = format!("{}.{}.{:?}.tmp", file().display(), std::process::id(), std::thread::current().id());
    let text = crate::drafts::pretty(&Value::Object(data));
    std::fs::write(&temp, text).and_then(|_| std::fs::rename(&temp, file())).map_err(|_| Fail::Ruby)?;
    Ok(result)
}

pub fn repo_of_pr_url(url: &str) -> R<String> {
    match pr_url_re().captures(url) {
        Some(c) => Ok(c[1].to_string()),
        None => arg(format!("'{url}' is not a GitHub PR URL")),
    }
}

/// Returns the stored entry, or `None` when that prerequisite was already there.
pub fn add(pr: &str, kind: &str, reference: &str, session: Option<&str>, note: Option<&str>) -> R<Option<Map<String, Value>>> {
    if !["pr", "commit"].contains(&kind) {
        return arg(format!("unknown prerequisite kind '{kind}'"));
    }
    let repo = repo_of_pr_url(pr)?;
    let ref_repo = if kind == "pr" { repo_of_pr_url(reference)? } else { repo.clone() };
    if kind == "commit" && !Regex::new(r"\A[0-9a-f]{40}\z").unwrap().is_match(reference) {
        return arg(format!("a commit prerequisite needs the full 40-char sha, not '{reference}'"));
    }
    if !ref_repo.eq_ignore_ascii_case(&repo) {
        return arg(format!("{reference} is in {ref_repo}, not {repo}: a prerequisite must be in the same repo as its PR"));
    }
    if reference.eq_ignore_ascii_case(pr) {
        return arg("a PR can't require itself");
    }
    update(|prs| {
        let list = prs.entry(pr.to_string()).or_insert_with(|| Value::Array(vec![]));
        let Value::Array(list) = list else { return None };
        if list.iter().any(|e| e.get("ref").and_then(Value::as_str) == Some(reference)) {
            return None;
        }
        let mut entry = Map::new();
        entry.insert("kind".into(), kind.into());
        entry.insert("ref".into(), reference.into());
        entry.insert("repo".into(), repo.clone().into());
        entry.insert("addedAt".into(), crate::clock::now_utc_iso().into());
        entry.insert("session".into(), session.map_or(Value::Null, Value::from));
        if let Some(n) = note.filter(|n| !strip(n).is_empty()) {
            entry.insert("note".into(), strip(n).into());
        }
        list.push(Value::Object(entry.clone()));
        Some(entry)
    })
}

/// Sets, replaces or (blank) clears an existing prerequisite's note.
pub fn set_note(pr: &str, reference: &str, note: &str) -> R<Option<Map<String, Value>>> {
    update(|prs| {
        let Some(Value::Array(list)) = prs.get_mut(pr) else { return None };
        let entry = list.iter_mut().find(|e| e.get("ref").and_then(Value::as_str) == Some(reference))?.as_object_mut()?;
        if strip(note).is_empty() {
            entry.shift_remove("note");
        } else {
            entry.insert("note".into(), strip(note).into());
        }
        Some(entry.clone())
    })
}

/// Drop the entries of dependent PRs GitHub confirmed merged or closed (`PrPrerequisites#prune`).
pub fn prune(closed: &[String]) -> R<Vec<String>> {
    if closed.is_empty() || all()?.is_empty() {
        return Ok(vec![]);
    }
    update(|prs| {
        let gone: Vec<String> = prs.keys().filter(|k| closed.contains(k)).cloned().collect();
        for url in &gone {
            prs.shift_remove(url);
        }
        gone
    })
}

pub fn remove(pr: &str, reference: &str) -> R<bool> {
    update(|prs| {
        let Some(Value::Array(list)) = prs.get_mut(pr) else { return false };
        let before = list.len();
        list.retain(|e| e.get("ref").and_then(Value::as_str) != Some(reference));
        let removed = list.len() != before;
        if list.is_empty() {
            prs.shift_remove(pr);
        }
        removed
    })
}

// --- What the CLI was given -> canonical refs (read-only gh/git) ---

fn parse_pr(spec: &str, cwd: &str, repo: Option<&str>) -> R<Option<String>> {
    if let Some(c) = pr_url_re().captures(spec) {
        return Ok(Some(pr_url(&c[1], &c[2])));
    }
    if let Some(c) = Regex::new(r"\A([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)#([0-9]+)\z").unwrap().captures(spec) {
        return Ok(Some(pr_url(&c[1], &c[2])));
    }
    if let Some(c) = Regex::new(r"\A#?([0-9]+)\z").unwrap().captures(spec) {
        let repo = match repo {
            Some(r) => r.to_string(),
            None => origin_repo(cwd)?,
        };
        return Ok(Some(pr_url(&repo, &c[1])));
    }
    Ok(None)
}

fn origin_repo(cwd: &str) -> R<String> {
    let url = crate::git::git(Some(cwd), &["remote", "get-url", "origin"]);
    let origin = Regex::new(r"github\.com[:/]([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+?)(?:\.git)?/?\z").unwrap();
    match url.as_deref().and_then(|u| origin.captures(u)) {
        Some(c) => Ok(c[1].to_string()),
        None => arg(format!("no GitHub origin remote in {cwd} to resolve a bare PR number against")),
    }
}

fn gh(args: &[&str], chdir: Option<&str>) -> R<String> {
    let mut cmd = Command::new("gh");
    cmd.args(args).stdin(Stdio::null());
    if let Some(dir) = chdir {
        cmd.current_dir(dir);
    }
    let Ok(o) = cmd.output() else { return Err(Fail::Ruby) };
    let (out, err) = (String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned());
    if !o.status.success() {
        let shown = strip(if strip(&err).is_empty() { &out } else { &err });
        let first = strip(shown.split_inclusive('\n').next().unwrap_or("")).chars().take(200).collect::<String>();
        return arg(format!("gh {} failed: {first}", args.iter().take(2).copied().collect::<Vec<_>>().join(" ")));
    }
    Ok(out)
}

/// `gh pr view` of a URL, or of cwd's branch when `target` is empty: proves the PR exists and gives
/// GitHub's casing of owner/repo.
fn viewed_pr_url(target: Option<&str>, cwd: Option<&str>) -> R<String> {
    let mut args = vec!["pr", "view"];
    args.extend(target);
    args.extend(["--json", "url"]);
    let shown = target.unwrap_or("(current branch)");
    let out = gh(&args, cwd)?;
    let Ok(parsed) = serde_json::from_str::<Value>(&out) else { return arg(format!("gh pr view {shown} returned no JSON")) };
    let Value::Object(o) = parsed else { return Err(Fail::Ruby) };
    match o.get("url") {
        Some(Value::String(u)) if pr_url_re().is_match(u) => Ok(u.clone()),
        _ => arg(format!("gh pr view {shown} gave no PR URL")),
    }
}

pub fn resolve_pr(spec: Option<&str>, cwd: &str) -> R<String> {
    match spec {
        None | Some("") => viewed_pr_url(None, Some(cwd)),
        Some(s) => match parse_pr(s, cwd, None)? {
            Some(url) => viewed_pr_url(Some(&url), None),
            None => arg(format!("'{s}' is not a PR (URL, owner/repo#n or #n)")),
        },
    }
}

/// `(kind, ref)` for a prerequisite spec: the PR forms, or a commit sha resolved to the full sha.
pub fn resolve_prerequisite(spec: &str, repo: &str, cwd: &str) -> R<(&'static str, String)> {
    if spec.is_empty() {
        return arg("no prerequisite given");
    }
    if let Some(url) = parse_pr(spec, cwd, Some(repo))? {
        return Ok(("pr", viewed_pr_url(Some(&url), None)?));
    }
    if Regex::new(r"(?i)\A[0-9a-f]{7,40}\z").unwrap().is_match(spec) {
        let full = gh(&["api", &format!("repos/{repo}/commits/{spec}"), "--jq", ".sha"], None)?;
        let full = strip(&full).to_string();
        if !Regex::new(r"\A[0-9a-f]{40}\z").unwrap().is_match(&full) {
            return arg(format!("{spec} did not resolve to a commit in {repo}"));
        }
        return Ok(("commit", full));
    }
    arg(format!("'{spec}' is neither a PR (URL, owner/repo#n, #n) nor a commit sha"))
}

/// The stored ref a removal spec names, without asking GitHub.
pub fn stored_ref(pr: &str, spec: &str, cwd: &str) -> R<String> {
    let entries = for_pr(pr)?;
    let text = |e: &Map<String, Value>, k: &str| e.get(k).and_then(Value::as_str).map(String::from);
    if let Some(url) = parse_pr(spec, cwd, Some(&repo_of_pr_url(pr)?))? {
        if let Some(stored) = entries.iter().filter_map(|e| text(e, "ref")).find(|r| r.eq_ignore_ascii_case(&url)) {
            return Ok(stored);
        }
    } else if Regex::new(r"(?i)\A[0-9a-f]{7,40}\z").unwrap().is_match(spec) {
        let prefix = spec.to_ascii_lowercase();
        let matches: Vec<String> = entries.iter().filter(|e| text(e, "kind").as_deref() == Some("commit")).filter_map(|e| text(e, "ref")).filter(|r| r.starts_with(&prefix)).collect();
        if matches.len() > 1 {
            return arg(format!("'{spec}' matches {} commit prerequisites of {pr}", matches.len()));
        }
        if let Some(m) = matches.into_iter().next() {
            return Ok(m);
        }
    }
    arg(format!("{pr} has no prerequisite '{spec}'"))
}

// --- The deploy check's cached verdict (read only) ---

fn cache() -> Option<Map<String, Value>> {
    let text = std::fs::read_to_string(data_root::root().join("pr-deployment.json")).ok()?;
    let Value::Object(mut data) = serde_json::from_str::<Value>(&text).ok()? else { return None };
    if data.get("v").and_then(Value::as_f64) != Some(1.0) {
        return None;
    }
    for key in ["releases", "pulls", "verdicts"] {
        if !data.get(key).is_some_and(Value::is_object) {
            data.insert(key.into(), Value::Object(Map::new()));
        }
    }
    Some(data)
}

/// The label the list shows after a prerequisite: `Ok(None)` for "unknown", `Err` for a cache whose
/// shapes make Ruby raise or word things its own way.
pub fn verdict_label(entry: &Map<String, Value>) -> R<Option<String>> {
    let cache = cache();
    let section = |name: &str| cache.as_ref().and_then(|c| c.get(name)).and_then(Value::as_object).cloned().unwrap_or_default();
    let reference = entry.get("ref").and_then(Value::as_str).ok_or(Fail::Ruby)?;
    let sha: String = if entry.get("kind").and_then(Value::as_str) == Some("commit") {
        reference.to_string()
    } else {
        let pull = match section("pulls").get(reference) {
            None | Some(Value::Null) => None,
            Some(Value::Object(o)) => Some(o.clone()),
            Some(_) => return Err(Fail::Ruby),
        };
        match pull.as_ref().and_then(|p| p.get("state")).and_then(Value::as_str) {
            Some("OPEN") => return Ok(Some("not merged".into())),
            Some("CLOSED") => return Ok(Some("closed without merging".into())),
            Some("MERGED") => match pull.as_ref().and_then(|p| p.get("mergeCommit")) {
                Some(Value::String(s)) => s.clone(),
                None | Some(Value::Null) | Some(Value::Bool(false)) => return Ok(None),
                Some(_) => return Err(Fail::Ruby),
            },
            _ => return Ok(None),
        }
    };
    let checked = match section("verdicts").get(&sha) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Object(o)) => o.clone(),
        Some(_) => return Err(Fail::Ruby),
    };
    let tag = match checked.get("tag") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err(Fail::Ruby),
    };
    let deployed = !matches!(checked.get("deployed"), None | Some(Value::Null) | Some(Value::Bool(false)));
    Ok(Some(if deployed { format!("✓ deployed in {tag}") } else { format!("not in {tag}") }))
}

