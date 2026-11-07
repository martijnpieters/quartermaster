//! Where each GitHub draft of one PR goes when it is posted, decided with local git against the PR's
//! head as fetched now: on the head (a review thread), on the commit it was made on, or outdated.
//! A port of `ReviewPlan` and of `PendingReview.reanchor`, which also rewrites the drafts a push
//! moved (`Drafts#relocate`, revs unchanged).

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use crate::review_anchors::{self, ReviewAnchors, E};

const CONTEXT: i64 = 3;

type R<T> = Result<T, E>;
type Entry = Map<String, Value>;

pub struct ReviewPlan {
    repo: String,
    base_ref: String,
    pub head: ReviewAnchors,
    anchors: std::cell::RefCell<HashMap<String, Option<std::rc::Rc<ReviewAnchors>>>>,
    pr_commits: std::cell::OnceCell<Vec<String>>,
    pr_patches: std::cell::OnceCell<Vec<u8>>,
}

fn text(v: Option<&Value>) -> R<String> {
    crate::draft_command::s(v).ok_or(E::Unsure)
}

fn truthy(v: Option<&Value>) -> bool {
    !matches!(v, None | Some(Value::Null) | Some(Value::Bool(false)))
}

fn int(v: Option<&Value>) -> R<i64> {
    match v {
        Some(Value::Number(n)) => n.as_i64().ok_or(E::Unsure),
        Some(Value::String(s)) => Ok(s.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0)),
        None | Some(Value::Null) => Ok(0),
        _ => Err(E::Unsure),
    }
}

fn is_hex(s: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn get(e: &Entry, k: &str) -> Value {
    e.get(k).cloned().unwrap_or(Value::Null)
}

impl ReviewPlan {
    pub fn new(repo: &str, base_ref: &str, head: &str) -> Result<ReviewPlan, String> {
        let anchors = review_anchors::new(repo, base_ref, head)?;
        let mut map = HashMap::new();
        let sha = anchors.commit.clone();
        let rc = std::rc::Rc::new(review_anchors::new(repo, base_ref, head)?);
        map.insert(sha, Some(rc));
        Ok(ReviewPlan { repo: repo.to_string(), base_ref: base_ref.to_string(), head: anchors, anchors: std::cell::RefCell::new(map), pr_commits: Default::default(), pr_patches: Default::default() })
    }

    pub fn head_sha(&self) -> &str {
        &self.head.commit
    }

    pub fn base_sha(&self) -> &str {
        &self.head.base_sha
    }

    pub fn commit_anchored(&self, e: &Entry) -> R<bool> {
        Ok(!text(e.get("range"))?.is_empty())
    }

    pub fn pinned(&self, e: &Entry) -> bool {
        e.get("pin").and_then(Value::as_str) == Some("commit")
    }

    /// id → route, in the entries' order.
    pub fn routes(&self, entries: &[Entry]) -> R<Vec<(Value, Value)>> {
        entries.iter().map(|e| Ok((get(e, "id"), self.route(e)?))).collect()
    }

    pub fn route(&self, e: &Entry) -> R<Value> {
        let path = text(e.get("path"))?;
        if e.get("subject").and_then(Value::as_str) == Some("file") {
            if self.head.file(&path).is_some() {
                return Ok(head_route(e, json!({"subject": "file"})));
            }
            return self.outdated(e, &format!("{path} is no longer in the pull request"), true);
        }
        if self.pinned(e) {
            return match self.commit_route(e)? {
                Some(r) => Ok(r),
                None => self.outdated(e, "its commit's line can't take a comment any more", false),
            };
        }
        if self.head.refusal(e, &self.head.base_sha)?.is_none() {
            return Ok(head_route(e, at(e)));
        }
        if self.commit_anchored(e)? {
            return match self.commit_route(e)? {
                Some(r) => Ok(r),
                None => {
                    let why = self.gone_reason(e)?;
                    self.outdated(e, &why, false)
                }
            };
        }
        if let Some(mut moved) = self.reanchor(e)? {
            moved.insert("moved".into(), json!(true));
            return Ok(head_route(e, Value::Object(moved)));
        }
        let in_pr = self.head.file(&path).is_some();
        let reason = if in_pr { "since the push its line isn't in the PR's diff once, unchanged".to_string() } else { format!("{path} is no longer in the pull request") };
        self.outdated(e, &reason, !in_pr)
    }

    fn anchors_at(&self, sha: &str) -> Option<std::rc::Rc<ReviewAnchors>> {
        if let Some(found) = self.anchors.borrow().get(sha) {
            return found.clone();
        }
        let made = if is_hex(sha, 7, 64) { review_anchors::new(&self.repo, &self.base_ref, sha).ok().map(std::rc::Rc::new) } else { None };
        self.anchors.borrow_mut().insert(sha.to_string(), made.clone());
        made
    }

    fn pr_commits(&self) -> &Vec<String> {
        self.pr_commits.get_or_init(|| crate::git::git(Some(&self.repo), &["rev-list", &format!("{}..{}", self.base_sha(), self.head_sha())]).unwrap_or_default().split_whitespace().map(String::from).collect())
    }

    /// The draft's commit, or the PR commit with its patch, when its line there still reads as the
    /// draft says.
    fn commit_route(&self, e: &Entry) -> R<Option<Value>> {
        let commit = text(e.get("commit"))?;
        let sha = if self.pr_commits().contains(&commit) { Some(commit.clone()) } else { self.patch_match(&commit) };
        let Some(sha) = sha else { return Ok(None) };
        let Some(anchors) = self.anchors_at(&sha) else { return Ok(None) };
        if anchors.refusal(e, &anchors.base_sha)?.is_some() {
            return Ok(None);
        }
        let position = anchors.position(&text(e.get("path"))?, &get(e, "side"), &get(e, "line"))?;
        Ok(Some(json!({"route": "commit", "path": get(e, "path"), "commit": sha, "moved": sha != commit, "position": position})))
    }

    fn outdated(&self, e: &Entry, reason: &str, file_gone: bool) -> R<Value> {
        let old = if e.get("subject").and_then(Value::as_str) == Some("file") { None } else { self.commit_route(e)? };
        Ok(json!({"route": "outdated", "path": get(e, "path"), "reason": reason, "fileGone": file_gone, "oldCommit": old.map(|o| o["commit"].clone()), "context": self.context(e)?}))
    }

    fn gone_reason(&self, e: &Entry) -> R<String> {
        let commit = text(e.get("commit"))?;
        let short: String = commit.chars().take(7).collect();
        if self.pr_commits().contains(&commit) {
            return Ok(format!("its line on {short} can't take a comment any more"));
        }
        Ok(format!("commit {short} is no longer in the pull request and no commit there has its changes"))
    }

    /// The stored end line's text exactly once on its side of the head's diff of that file; a range
    /// keeps its length and must still read the same.
    fn reanchor(&self, e: &Entry) -> R<Option<Map<String, Value>>> {
        let path = text(e.get("path"))?;
        let code = text(e.get("code"))?;
        let texts: Vec<&str> = code.split('\n').collect();
        if self.head.file(&path).is_none() {
            return Ok(None);
        }
        let side = get(e, "side");
        let last = texts[texts.len() - 1];
        let mut found = vec![];
        for n in self.head.lines(&path, &side)? {
            if self.head.text(&path, &side, n)?.as_deref() == Some(last) {
                found.push(n);
            }
        }
        if found.len() != 1 {
            return Ok(None);
        }
        let mut moved = Map::new();
        moved.insert("line".into(), json!(found[0]));
        moved.insert("side".into(), side.clone());
        moved.insert("subject".into(), json!("line"));
        if truthy(e.get("start_line")) {
            let start_side = if truthy(e.get("start_side")) { get(e, "start_side") } else { side.clone() };
            if start_side != side {
                return Ok(None);
            }
            moved.insert("start_line".into(), json!(found[0] - (int(e.get("line"))? - int(e.get("start_line"))?)));
            moved.insert("start_side".into(), side.clone());
        }
        let mut merged = e.clone();
        for (k, v) in &moved {
            merged.insert(k.clone(), v.clone());
        }
        Ok(if self.head.refusal(&merged, &self.head.base_sha)?.is_none() { Some(moved) } else { None })
    }

    /// The lines around the draft's line as its commit shows it, else just its stored text.
    fn context(&self, e: &Entry) -> R<Value> {
        let key = if matches!(e.get("side").and_then(Value::as_str), Some("old") | Some("LEFT")) { "o" } else { "n" };
        let line = int(e.get("line"))?;
        let anchors = self.anchors_at(&text(e.get("commit"))?);
        let path = text(e.get("path"))?;
        let hunk = anchors.as_ref().and_then(|a| a.file(&path)).and_then(|f| f["hunks"].as_array()).and_then(|hunks| hunks.iter().find(|h| h["lines"].as_array().is_some_and(|ls| ls.iter().any(|l| l[key].as_i64() == Some(line)))));
        let Some(hunk) = hunk else {
            let code = text(e.get("code"))?;
            return Ok(Value::Array(code.split('\n').map(|s| json!({"t": " ", "s": s, "at": true})).collect()));
        };
        let lines = hunk["lines"].as_array().unwrap();
        let at = lines.iter().position(|l| l[key].as_i64() == Some(line)).unwrap() as i64;
        let span = if truthy(e.get("start_line")) { line - int(e.get("start_line"))? } else { 0 };
        let from = at - CONTEXT - span;
        let (a, b) = (from.max(0) as usize, ((at + CONTEXT) as usize).min(lines.len() - 1));
        Ok(Value::Array(lines[a..=b].iter().enumerate().map(|(i, l)| json!({"t": l["t"], "s": l["s"], "o": l["o"], "n": l["n"], "at": a + i == at as usize})).collect()))
    }

    /// The PR commit whose patch-id equals the gone commit's: a rebase or amend that kept its changes.
    fn patch_match(&self, sha: &str) -> Option<String> {
        if !(sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit())) || crate::git::git(Some(&self.repo), &["cat-file", "-e", &format!("{sha}^{{commit}}")]).is_none() {
            return None;
        }
        let mine = self.patch_ids(&git_raw(&self.repo, &["show", "--no-color", sha])).into_iter().next()?.0;
        self.patch_ids(self.pr_patches()).into_iter().find(|(id, _)| *id == mine).map(|(_, c)| c)
    }

    fn pr_patches(&self) -> &Vec<u8> {
        self.pr_patches.get_or_init(|| git_raw(&self.repo, &["log", "-p", "--no-color", &format!("{}..{}", self.base_sha(), self.head_sha())]))
    }

    /// (patch-id, commit) pairs in output order, the last of a repeated id winning as Ruby's hash does.
    fn patch_ids(&self, patches: &[u8]) -> Vec<(String, String)> {
        if patches.is_empty() {
            return vec![];
        }
        let out = crate::git::git_bytes(Some(&self.repo), &["patch-id", "--stable"], patches).unwrap_or_default();
        let mut pairs: Vec<(String, String)> = vec![];
        for line in String::from_utf8_lossy(&out).lines() {
            let mut f = line.split_whitespace();
            if let (Some(id), Some(commit)) = (f.next(), f.next()) {
                match pairs.iter_mut().find(|(i, _)| i == id) {
                    Some(slot) => slot.1 = commit.to_string(),
                    None => pairs.push((id.to_string(), commit.to_string())),
                }
            }
        }
        pairs
    }
}

fn git_raw(repo: &str, args: &[&str]) -> Vec<u8> {
    match std::process::Command::new("git").args(["-C", repo]).args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output() {
        Ok(o) if o.status.success() => o.stdout,
        _ => vec![],
    }
}

fn head_route(e: &Entry, fields: Value) -> Value {
    let mut route = Map::new();
    route.insert("route".into(), json!("head"));
    route.insert("path".into(), get(e, "path"));
    route.insert("moved".into(), json!(false));
    for (k, v) in fields.as_object().cloned().unwrap_or_default() {
        route.insert(k, v);
    }
    Value::Object(route)
}

/// The draft's own address on the head: `line`, `side`, `start_line`, `start_side` (those present).
fn at(e: &Entry) -> Value {
    let mut m = Map::new();
    for k in ["line", "side", "start_line", "start_side"] {
        if let Some(v) = e.get(k) {
            m.insert(k.into(), v.clone());
        }
    }
    m.insert("subject".into(), json!("line"));
    Value::Object(m)
}

/// `PendingReview.moved_fields`: what a draft takes from its route when a push moved it.
fn moved_fields(plan: &ReviewPlan, e: &Entry, route: &Value) -> R<Option<Map<String, Value>>> {
    let kind = route["route"].as_str().unwrap_or("");
    if kind == "head" && !plan.commit_anchored(e)? && !plan.pinned(e) && (truthy(Some(&route["moved"])) || e.get("commit").and_then(Value::as_str) != Some(plan.head_sha())) {
        let mut m = Map::new();
        for k in ["line", "side", "start_line", "start_side"] {
            if let Some(v) = route.get(k) {
                m.insert(k.into(), v.clone());
            }
        }
        m.insert("commit".into(), json!(plan.head_sha()));
        m.insert("base".into(), json!(plan.base_sha()));
        m.insert("start_line".into(), route.get("start_line").cloned().unwrap_or(Value::Null));
        let start_side = if truthy(route.get("start_line")) { route.get("start_side").cloned().unwrap_or(Value::Null) } else { route.get("start_line").cloned().unwrap_or(Value::Null) };
        m.insert("start_side".into(), start_side);
        return Ok(Some(m));
    }
    if kind == "commit" && truthy(Some(&route["moved"])) {
        let mut m = Map::new();
        m.insert("commit".into(), route["commit"].clone());
        return Ok(Some(m));
    }
    Ok(None)
}

/// `PendingReview.reanchor`: the routes as the diff pane shows them, re-anchoring stored drafts on
/// the way. Nothing the user wrote changes, so revs stay.
pub fn reanchor(plan: &ReviewPlan, entries: &[Entry], slug: &str) -> R<Map<String, Value>> {
    let routes = plan.routes(entries)?;
    for (entry, (id, route)) in entries.iter().zip(&routes) {
        if let Some(fields) = moved_fields(plan, entry, route)? {
            crate::drafts::relocate(slug, id.as_str().unwrap_or(""), &fields, entry.get("rev")).map_err(|_| E::Unsure)?;
        }
    }
    let mut out = Map::new();
    for (id, route) in routes {
        out.insert(match id {
            Value::String(s) => s,
            other => other.to_string(),
        }, route);
    }
    Ok(out)
}
