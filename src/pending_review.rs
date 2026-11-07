//! The user's pending review on one PR, filled from their GitHub drafts (`PendingReview`): each draft
//! goes where `ReviewPlan` routes it, replies join their threads, and what GitHub takes leaves the
//! drafts store. qm never submits the review.

use std::cell::OnceCell;

use serde_json::{json, Map, Value};

use crate::review_anchors::E;
use crate::review_plan::ReviewPlan;
use crate::ruby_text::strip;

const POSITION_FEATURE: &str = "GraphQL-Features: pull_request_review_comment_position";

const PR_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) {\n  repository(owner: $owner, name: $name) {\n    pullRequest(number: $number) {\n      id baseRefName headRefOid state\n      reviews(states: PENDING, first: 20) { nodes { id viewerDidAuthor comments { totalCount } } }\n    }\n  }\n}\n";
const ADD_REVIEW: &str = "mutation($pr: ID!, $commit: GitObjectID!) {\n  addPullRequestReview(input: { pullRequestId: $pr, commitOID: $commit }) { pullRequestReview { id } }\n}\n";
const ADD_THREAD: &str = "mutation($review: ID!, $path: String!, $body: String!, $subject: PullRequestReviewThreadSubjectType,\n         $line: Int, $side: DiffSide, $startLine: Int, $startSide: DiffSide) {\n  addPullRequestReviewThread(input: { pullRequestReviewId: $review, path: $path, body: $body,\n                                      subjectType: $subject, line: $line, side: $side,\n                                      startLine: $startLine, startSide: $startSide }) { thread { id } }\n}\n";
const ADD_COMMENT: &str = "mutation($review: ID!, $commit: GitObjectID!, $path: String!, $position: Int!, $body: String!) {\n  addPullRequestReviewComment(input: { pullRequestReviewId: $review, commitOID: $commit, path: $path,\n                                       position: $position, body: $body }) { comment { id } }\n}\n";

/// `Err(Arg)` is the `ArgumentError` Ruby raises; `Unsure` a value only Ruby reads exactly.
pub type PErr = E;
type R<T> = Result<T, PErr>;
pub type Entry = Map<String, Value>;

fn arg<T>(m: impl Into<String>) -> R<T> {
    Err(E::Arg(m.into()))
}

pub struct PendingReview {
    path: String,
    pub repo: String,
    pub number: i64,
    pr: OnceCell<Value>,
    plan: OnceCell<ReviewPlan>,
}

fn present(v: &Value) -> bool {
    !matches!(v, Value::Null | Value::Bool(false))
}

fn s(v: Option<&Value>) -> R<String> {
    crate::draft_command::s(v).ok_or(E::Unsure)
}

impl PendingReview {
    /// `PendingReview.new(path, repo:, number:)`.
    pub fn new(path: &str, repo: &Value, number: &Value) -> R<PendingReview> {
        let repo_text = s(Some(repo))?;
        if !crate::review_threads::repo_ok(&repo_text) {
            return arg(format!("'{repo_text}' is not a GitHub repo"));
        }
        let number_text = match number {
            Value::Number(n) if n.is_i64() || n.is_u64() => n.to_string(),
            Value::String(t) => t.clone(),
            Value::Null => String::new(),
            _ => return Err(E::Unsure),
        };
        let parsed = crate::ruby_text::kernel_integer(&Value::String(number_text.clone())).map_err(|_| E::Unsure)?;
        match parsed {
            Some(n) if n > 0 => Ok(PendingReview { path: path.to_string(), repo: repo_text, number: n, pr: OnceCell::new(), plan: OnceCell::new() }),
            _ => arg(format!("'{number_text}' is not a PR number")),
        }
    }

    pub fn label(&self) -> String {
        format!("{}#{}", self.repo.rsplit('/').next().unwrap_or(""), self.number)
    }

    fn pr(&self) -> R<&Value> {
        if let Some(p) = self.pr.get() {
            return Ok(p);
        }
        let (owner, name) = self.repo.split_once('/').unwrap_or((&self.repo, ""));
        let data = graphql(PR_QUERY, &[("owner", owner.to_string()), ("name", name.to_string())], &[("number", self.number)], None)?;
        let node = match data.get("repository").and_then(|r| r.get("pullRequest")) {
            Some(n) if present(n) => n.clone(),
            _ => return arg(format!("{} wasn't found on GitHub", self.label())),
        };
        let state = s(node.get("state"))?;
        if state != "OPEN" {
            return arg(format!("{} is {} — there is no review to add to", self.label(), state.to_lowercase()));
        }
        let _ = self.pr.set(node);
        Ok(self.pr.get().unwrap())
    }

    fn plan(&self) -> R<&ReviewPlan> {
        if let Some(p) = self.plan.get() {
            return Ok(p);
        }
        let pr = self.pr()?;
        let base = s(pr.get("baseRefName"))?;
        let revs = crate::branch_diff::fetch_pr_uncached(&self.path, &json!({"repo": self.repo, "number": self.number, "base": base})).map_err(|e| match e {
            Ok(m) => E::Arg(m),
            Err(()) => E::Unsure,
        })?;
        if revs["headSha"] != pr["headRefOid"] {
            return arg(format!("{}'s head moved while it was fetched — try again", self.label()));
        }
        let plan = ReviewPlan::new(&self.path, revs["baseRef"].as_str().unwrap_or(""), revs["headSha"].as_str().unwrap_or("")).map_err(E::Arg)?;
        let _ = self.plan.set(plan);
        Ok(self.plan.get().unwrap())
    }

    fn pending_nodes(&self) -> R<Vec<Value>> {
        match self.pr()?.get("reviews").and_then(|r| r.get("nodes")) {
            Some(Value::Array(a)) if a.iter().all(Value::is_object) => Ok(a.clone()),
            _ => Err(E::Unsure),
        }
    }

    /// What the confirmation says before anything is posted.
    pub fn preview(&self, entries: &[Entry]) -> R<Value> {
        let (replies, comments): (Vec<&Entry>, Vec<&Entry>) = entries.iter().partition(|e| is_reply(e));
        let owned: Vec<Entry> = comments.iter().map(|e| (*e).clone()).collect();
        let routes = if owned.is_empty() { vec![] } else { self.plan()?.routes(&owned)? };
        let count = |kind: &str| routes.iter().filter(|(_, r)| r["route"] == kind).count();
        let pending = self.pending_nodes()?.into_iter().find(|r| r.get("viewerDidAuthor").is_some_and(present));
        let moved = routes.iter().filter(|(_, r)| present(&r["moved"]) && r["route"] != "outdated").count();
        Ok(json!({
            "pr": self.label(), "url": crate::pr_prerequisites::pr_url(&self.repo, &self.number.to_string()), "count": entries.len(),
            "head": count("head"), "commit": count("commit"), "outdated": count("outdated"), "replies": replies.len(),
            "moved": moved,
            "pending": pending.map(|p| p["comments"]["totalCount"].clone()).unwrap_or(Value::Null),
        }))
    }

    fn pending_review_id(&self) -> R<String> {
        if let Some(existing) = self.pending_nodes()?.into_iter().find(|r| r.get("viewerDidAuthor").is_some_and(present)) {
            return s(existing.get("id"));
        }
        let pr = self.pr()?;
        let data = graphql(ADD_REVIEW, &[("pr", s(pr.get("id"))?), ("commit", s(pr.get("headRefOid"))?)], &[], None)?;
        match data.get("addPullRequestReview").and_then(|v| v.get("pullRequestReview")).and_then(|v| v.get("id")) {
            Some(id) if present(id) => s(Some(id)),
            _ => arg(format!("GitHub made no pending review on {}", self.label())),
        }
    }

    /// Posts the drafts into the pending review and updates the store: `{posted, failed, fallbacks, outdated}`.
    pub fn post(&self, entries: &[Entry], slug: &str) -> R<Value> {
        let (replies, comments): (Vec<&Entry>, Vec<&Entry>) = entries.iter().partition(|e| is_reply(e));
        let owned: Vec<Entry> = comments.iter().map(|e| (*e).clone()).collect();
        let routes = if owned.is_empty() { vec![] } else { self.plan()?.routes(&owned)? };
        let route_of = |entry: &Entry| routes.iter().find(|(id, _)| Some(id) == entry.get("id")).map(|(_, r)| r.clone()).unwrap_or(Value::Null);
        let postable: Vec<&Entry> = comments.iter().copied().filter(|e| route_of(e)["route"] != "outdated").collect();
        let outdated = comments.len() - postable.len();
        let mut failed: Vec<String> = vec![];
        let mut fallbacks = 0;
        if postable.is_empty() && replies.is_empty() {
            return Ok(json!({"posted": 0, "failed": failed, "fallbacks": 0, "outdated": outdated}));
        }
        let review = self.pending_review_id()?;
        let mut posted: Vec<(Value, Value)> = vec![];
        for entry in postable.iter().chain(replies.iter()) {
            let outcome = if is_reply(entry) { self.post_reply(&review, entry)? } else { self.post_one(&review, entry, &route_of(entry))? };
            match outcome {
                Ok(fallback) => {
                    posted.push((entry.get("id").cloned().unwrap_or(Value::Null), entry.get("rev").cloned().unwrap_or(Value::Null)));
                    if fallback {
                        fallbacks += 1;
                    }
                }
                Err(message) => {
                    mark_failed(slug, entry.get("id"), entry.get("rev"), &message)?;
                    let place = if is_reply(entry) {
                        format!("reply on {}", match entry.get("path") { Some(p) if present(p) => s(Some(p))?, _ => "a thread".to_string() })
                    } else {
                        format!("{}:{}", s(entry.get("path"))?, s(entry.get("line"))?)
                    };
                    failed.push(format!("{place}: {message}"));
                }
            }
        }
        if !posted.is_empty() {
            remove(slug, &posted)?;
        }
        crate::review_threads::forget(&self.repo, &self.number.to_string());
        Ok(json!({"posted": posted.len(), "failed": failed, "fallbacks": fallbacks, "outdated": outdated}))
    }

    /// A reply joins its thread inside the pending review; a reply GitHub answers with no comment fails.
    fn post_reply(&self, review: &str, entry: &Entry) -> R<Result<bool, String>> {
        let thread = s(entry.get("thread"))?;
        let body = s(entry.get("comment"))?;
        match crate::review_threads::reply(&self.repo, &self.number.to_string(), review, &thread, &body) {
            Ok(_) => Ok(Ok(false)),
            Err(Some(m)) => Ok(Err(m)),
            Err(None) => Err(E::Unsure),
        }
    }

    fn post_one(&self, review: &str, entry: &Entry, route: &Value) -> R<Result<bool, String>> {
        if route["route"] == "head" {
            return Ok(self.post_thread(review, entry, route)?.map(|_| false));
        }
        let commit = s(route.get("commit"))?;
        let position = match route.get("position") {
            Some(Value::Number(n)) if n.is_i64() => n.as_i64().unwrap(),
            _ => return Err(E::Unsure),
        };
        let strings = [("review", review.to_string()), ("commit", commit.clone()), ("path", s(entry.get("path"))?), ("body", commit_body(entry)?)];
        let error = match graphql(ADD_COMMENT, &strings, &[("position", position)], Some(POSITION_FEATURE)) {
            Ok(data) => {
                if data.get("addPullRequestReviewComment").and_then(|v| v.get("comment")).and_then(|v| v.get("id")).is_some_and(present) {
                    return Ok(Ok(false));
                }
                format!("GitHub took no comment at {}", commit.chars().take(7).collect::<String>())
            }
            Err(E::Arg(m)) => m,
            Err(E::Unsure) => return Err(E::Unsure),
        };
        self.fallback(review, entry, route, &error)
    }

    /// The commit route errored: a file comment on the head that says which line of which commit.
    fn fallback(&self, review: &str, entry: &Entry, route: &Value, error: &str) -> R<Result<bool, String>> {
        let path = s(entry.get("path"))?;
        if self.plan()?.head.file(&path).is_none() {
            return Ok(Err(format!("{error}; {path} isn't in the head's diff for a file comment")));
        }
        let code = s(entry.get("code"))?;
        let quoted = code.split('\n').map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n");
        let label = crate::draft_messages::lines_label(entry).map_err(|_| E::Unsure)?;
        let commit = s(route.get("commit"))?;
        let body = format!("on {}, {path}:{}:\n{quoted}\n\n{}", commit.chars().take(7).collect::<String>(), label.strip_prefix(':').unwrap_or(&label), s(entry.get("comment"))?);
        let mut moved = entry.clone();
        moved.insert("comment".into(), json!(body));
        Ok(self.post_thread(review, &moved, &json!({"subject": "file"}))?.map(|_| true))
    }

    fn post_thread(&self, review: &str, entry: &Entry, route: &Value) -> R<Result<(), String>> {
        let mut strings = vec![("review", review.to_string()), ("path", s(entry.get("path"))?), ("body", s(entry.get("comment"))?)];
        let mut ints: Vec<(&str, i64)> = vec![];
        if route["subject"] == "file" {
            strings.push(("subject", "FILE".into()));
        } else {
            strings.push(("subject", "LINE".into()));
            strings.push(("side", github_side(route.get("side"))?));
            let line = int(route.get("line"))?;
            ints.push(("line", line));
            if let Some(start) = route.get("start_line").filter(|v| present(v)) {
                let start = int(Some(start))?;
                if start != line {
                    ints.push(("startLine", start));
                    let side = match route.get("start_side") {
                        Some(v) if present(v) => v.clone(),
                        _ => route.get("side").cloned().unwrap_or(Value::Null),
                    };
                    strings.push(("startSide", github_side(Some(&side))?));
                }
            }
        }
        match graphql(ADD_THREAD, &strings, &ints, None) {
            Ok(data) => {
                if data.get("addPullRequestReviewThread").and_then(|v| v.get("thread")).and_then(|v| v.get("id")).is_some_and(present) {
                    Ok(Ok(()))
                } else {
                    Ok(Err("GitHub didn't take a comment on that line (no thread came back)".to_string()))
                }
            }
            Err(E::Arg(m)) => Ok(Err(m)),
            Err(E::Unsure) => Err(E::Unsure),
        }
    }
}

fn is_reply(entry: &Entry) -> bool {
    entry.get("kind") == Some(&json!("thread-reply"))
}

fn int(v: Option<&Value>) -> R<i64> {
    match v {
        Some(Value::Number(n)) if n.is_i64() => Ok(n.as_i64().unwrap()),
        _ => Err(E::Unsure),
    }
}

/// The commit route takes one position: a range says which lines in words.
fn commit_body(entry: &Entry) -> R<String> {
    let comment = s(entry.get("comment"))?;
    let (Some(start), Some(line)) = (entry.get("start_line").filter(|v| present(v)), entry.get("line")) else { return Ok(comment) };
    if start == line {
        return Ok(comment);
    }
    Ok(format!("Lines {}–{}:\n\n{comment}", s(Some(start))?, s(Some(line))?))
}

fn github_side(side: Option<&Value>) -> R<String> {
    let text = s(side)?;
    Ok(if text == "old" || text == "LEFT" { "LEFT" } else { "RIGHT" }.to_string())
}

/// `gh api graphql`: strings as -f, ints as -F; the data, or the message GitHub gave.
fn graphql(query: &str, strings: &[(&str, String)], ints: &[(&str, i64)], header: Option<&str>) -> R<Value> {
    let mut owned: Vec<String> = vec!["gh".into(), "api".into(), "graphql".into()];
    if let Some(h) = header {
        owned.extend(["-H".into(), h.into()]);
    }
    owned.extend(["-f".into(), format!("query={query}")]);
    for (k, v) in strings {
        owned.extend(["-f".into(), format!("{k}={v}")]);
    }
    for (k, v) in ints {
        owned.extend(["-F".into(), format!("{k}={v}")]);
    }
    let argv: Vec<&str> = owned.iter().map(String::as_str).collect();
    let result = crate::command_runner::call(&argv);
    let body: Value = serde_json::from_str(&result.out).unwrap_or_else(|_| json!({}));
    let Value::Object(body) = body else { return Err(E::Unsure) };
    let errors: Vec<String> = match body.get("errors") {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(list)) => {
            let mut out = vec![];
            for e in list {
                match e.get("message") {
                    Some(Value::String(m)) => out.push(m.clone()),
                    Some(Value::Null) | None if e.is_object() => {}
                    _ => return Err(E::Unsure),
                }
            }
            out
        }
        Some(_) => return Err(E::Unsure),
    };
    let data = body.get("data").filter(|d| present(d));
    match (result.success && errors.is_empty(), data) {
        (true, Some(d)) => Ok(d.clone()),
        _ => {
            let detail = match errors.first() {
                Some(e) => e.clone(),
                None => {
                    let err = strip(&result.err);
                    if err.is_empty() { strip(&result.out).to_string() } else { err.to_string() }
                }
            };
            arg(format!("GitHub: {}", detail.chars().take(300).collect::<String>()))
        }
    }
}

/// `Drafts#mark_failed`: what posting said about a draft GitHub didn't take.
fn mark_failed(slug: &str, id: Option<&Value>, rev: Option<&Value>, message: &str) -> R<()> {
    let (id, rev) = (id.cloned().unwrap_or(Value::Null), rev.cloned().unwrap_or(Value::Null));
    crate::drafts::update(slug, |list| {
        if let Some(e) = list.iter_mut().find(|e| e.get("id").unwrap_or(&Value::Null) == &id && e.get("rev").unwrap_or(&Value::Null) == &rev) {
            if let Some(m) = e.as_object_mut() {
                m.insert("failed".into(), json!(message));
            }
        }
        Ok(())
    })
    .map_err(|_| E::Unsure)
}

/// `Drafts#remove` of exactly these `{id => rev}` entries.
fn remove(slug: &str, revs: &[(Value, Value)]) -> R<()> {
    crate::drafts::update(slug, |list| {
        list.retain(|e| {
            let id = e.get("id").cloned().unwrap_or(Value::Null);
            let want = revs.iter().rev().find(|(k, _)| *k == id).map(|(_, v)| v.clone()).unwrap_or(Value::Null);
            want != e.get("rev").cloned().unwrap_or(Value::Null)
        });
        Ok(())
    })
    .map_err(|_| E::Unsure)
}
