//! A pull request's review threads as the diff pane's PR scope shows them (`ReviewThreads#fetch`):
//! `gh api graphql` read page by page, cached per PR and head for 30 seconds.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

use crate::command_runner;
use crate::ruby_text::{inspect_ascii, strip};

const TTL: f64 = 30.0;
const PAGE: usize = 50;
const MAX_PAGES: usize = 10;

static CACHE: Mutex<Option<HashMap<String, (f64, Value)>>> = Mutex::new(None);

fn query() -> String {
    format!(
        "query($owner: String!, $name: String!, $number: Int!, $cursor: String) {{\n  viewer {{ login }}\n  repository(owner: $owner, name: $name) {{\n    pullRequest(number: $number) {{\n      id\n      reviews(states: PENDING, first: 5) {{ nodes {{ id viewerDidAuthor }} }}\n      reviewThreads(first: {PAGE}, after: $cursor) {{\n        pageInfo {{ hasNextPage endCursor }}\n        nodes {{\n          id path line originalLine startLine originalStartLine diffSide subjectType\n          isResolved isOutdated viewerCanResolve viewerCanUnresolve viewerCanReply\n          resolvedBy {{ login }}\n          comments(first: 100) {{\n            nodes {{ id body createdAt url state diffHunk author {{ login }} }}\n          }}\n        }}\n      }}\n    }}\n  }}\n}}\n"
    )
}

/// `Err(Some(message))` is the `ArgumentError` Ruby answers with; `Err(None)` a shape only Ruby handles.
pub type Failure = Option<String>;

fn word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

pub(crate) fn repo_ok(repo: &str) -> bool {
    let part = |s: &str| !s.is_empty() && s.chars().all(|c| word(c) || c == '.' || c == '-');
    matches!(repo.split_once('/'), Some((a, b)) if part(a) && part(b))
}

/// `Integer(text, 10)`: surrounding whitespace, a sign and single underscores between digits.
fn integer(text: &str) -> Result<i64, Failure> {
    let bad = || Err(inspect_ascii(text).map(|shown| format!("invalid value for Integer(): {shown}")));
    let t = text.trim_matches(|c: char| c.is_ascii_whitespace() || c == '\0');
    let (neg, digits) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") || !digits.chars().all(|c| c.is_ascii_digit() || c == '_') {
        return bad();
    }
    match digits.replace('_', "").parse::<i64>() {
        Ok(n) => Ok(if neg { -n } else { n }),
        Err(_) => Err(None),
    }
}

fn checked(repo: Option<&str>, number: Option<&str>) -> Result<(String, i64), Failure> {
    let repo = repo.unwrap_or("");
    if !repo_ok(repo) {
        return Err(Some(format!("'{repo}' is not an owner/name repo")));
    }
    let n = integer(number.unwrap_or(""))?;
    if n <= 0 {
        return Err(Some("a pull request number must be positive".into()));
    }
    Ok((repo.to_string(), n))
}

fn graphql(query: &str, vars: &[(&str, Value)]) -> Result<Value, Failure> {
    let mut owned: Vec<String> = vec!["gh".into(), "api".into(), "graphql".into(), "-f".into(), format!("query={query}")];
    for (name, value) in vars {
        match value {
            Value::Number(n) => owned.extend(["-F".into(), format!("{name}={n}")]),
            Value::String(s) => owned.extend(["-f".into(), format!("{name}={s}")]),
            _ => return Err(None),
        }
    }
    let argv: Vec<&str> = owned.iter().map(String::as_str).collect();
    let result = command_runner::call(&argv);
    let body = serde_json::from_str::<Value>(&result.out).ok();
    let errors: Vec<Value> = match body.as_ref().and_then(|b| b.as_object()) {
        Some(o) => match o.get("errors") {
            None | Some(Value::Null) => vec![],
            Some(Value::Array(a)) => a.clone(),
            Some(_) => return Err(None),
        },
        None => vec![],
    };
    let Some(object) = body.as_ref().and_then(|b| b.as_object()).filter(|_| result.success && errors.is_empty()) else {
        let mut why = vec![];
        for e in &errors {
            match e.get("message") {
                Some(Value::String(m)) => why.push(m.clone()),
                Some(Value::Null) | None if e.is_object() => {}
                _ => return Err(None),
            }
        }
        let mut why = why.join("; ");
        if why.is_empty() {
            let text = if strip(&result.err).is_empty() { &result.out } else { &result.err };
            why = strip(text).to_string();
        }
        return Err(Some(format!("gh api graphql failed: {}", why.chars().take(300).collect::<String>())));
    };
    match object.get("data") {
        None | Some(Value::Null) => Ok(json!({})),
        Some(d @ Value::Object(_)) => Ok(d.clone()),
        Some(_) => Err(None),
    }
}

fn nodes(value: Option<&Value>) -> Result<Vec<Value>, Failure> {
    match value {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Array(a)) => Ok(a.iter().filter(|v| !v.is_null()).cloned().collect()),
        Some(_) => Err(None),
    }
}

fn text(value: Option<&Value>) -> Result<String, Failure> {
    match value {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(None),
    }
}

fn login(value: Option<&Value>) -> Option<Value> {
    value.and_then(|v| v.get("login")).filter(|l| !l.is_null() && **l != Value::Bool(false)).cloned()
}

fn thread(node: &Value) -> Result<Value, Failure> {
    if !node.is_object() {
        return Err(None);
    }
    let comments = nodes(node.get("comments").and_then(|c| c.get("nodes")))?;
    let quote = match comments.first() {
        None => Value::Null,
        Some(first) => {
            let hunk = text(first.get("diffHunk"))?;
            let mut parts: Vec<&str> = hunk.split('\n').collect();
            while parts.last() == Some(&"") {
                parts.pop();
            }
            parts.last().map(|l| json!(l)).unwrap_or(Value::Null)
        }
    };
    let flag = |key: &str| json!(node.get(key) == Some(&Value::Bool(true)));
    let get = |key: &str| node.get(key).cloned().unwrap_or(Value::Null);
    let mut shown = vec![];
    for c in &comments {
        shown.push(json!({
            "id": c.get("id").cloned().unwrap_or(Value::Null),
            "author": login(c.get("author")).unwrap_or_else(|| json!("ghost")),
            "body": text(c.get("body"))?,
            "createdAt": c.get("createdAt").cloned().unwrap_or(Value::Null),
            "url": c.get("url").cloned().unwrap_or(Value::Null),
            "pending": c.get("state") == Some(&json!("PENDING")),
        }));
    }
    Ok(json!({
        "id": get("id"), "path": get("path"), "side": if node.get("diffSide") == Some(&json!("LEFT")) { "old" } else { "new" },
        "line": get("line"), "startLine": get("startLine"), "originalLine": get("originalLine"),
        "file": node.get("subjectType") == Some(&json!("FILE")), "resolved": flag("isResolved"),
        "outdated": flag("isOutdated"), "resolvedBy": login(node.get("resolvedBy")).unwrap_or(Value::Null),
        "canResolve": flag("viewerCanResolve"), "canUnresolve": flag("viewerCanUnresolve"),
        "canReply": flag("viewerCanReply"), "quote": quote, "comments": shown,
    }))
}

fn read(repo: &str, number: i64) -> Result<Value, Failure> {
    let (owner, name) = repo.split_once('/').unwrap_or((repo, ""));
    let mut threads = vec![];
    let mut cursor: Option<String> = None;
    let mut data = json!({});
    for _ in 0..MAX_PAGES {
        let mut vars = vec![("owner", json!(owner)), ("name", json!(name)), ("number", json!(number))];
        if let Some(c) = &cursor {
            vars.push(("cursor", json!(c)));
        }
        data = graphql(&query(), &vars)?;
        let pr = match data.get("repository") {
            None | Some(Value::Null) => Value::Null,
            Some(r @ Value::Object(_)) => r.get("pullRequest").cloned().unwrap_or(Value::Null),
            Some(_) => return Err(None),
        };
        if pr.is_null() || pr == Value::Bool(false) {
            return Err(Some(format!("{repo}#{number} is not a pull request GitHub knows")));
        }
        let page = match pr.get("reviewThreads") {
            None | Some(Value::Null) => json!({}),
            Some(p @ Value::Object(_)) => p.clone(),
            Some(_) => return Err(None),
        };
        for n in nodes(page.get("nodes"))? {
            threads.push(thread(&n)?);
        }
        let info = page.get("pageInfo").filter(|i| i.is_object()).cloned().unwrap_or_else(|| json!({}));
        let end = info.get("endCursor").and_then(|c| c.as_str()).filter(|c| !c.is_empty());
        match (info.get("hasNextPage") == Some(&Value::Bool(true)), end) {
            (true, Some(c)) => cursor = Some(c.to_string()),
            _ => break,
        }
    }
    let pr = data["repository"]["pullRequest"].clone();
    let pending = nodes(pr.get("reviews").and_then(|r| r.get("nodes")))?.into_iter().find(|r| r.get("viewerDidAuthor").is_some_and(|v| !v.is_null() && *v != Value::Bool(false)));
    Ok(json!({
        "threads": threads, "viewer": data.get("viewer").and_then(|v| v.get("login")).cloned().unwrap_or(Value::Null),
        "prId": pr.get("id").cloned().unwrap_or(Value::Null),
        "pendingReview": pending.map(|p| json!({"id": p.get("id").cloned().unwrap_or(Value::Null)})).unwrap_or(Value::Null),
    }))
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// `GET /api/pr-threads`; `None` for what only Ruby answers exactly.
pub fn payload(params: &Map<String, Value>) -> Option<Value> {
    let get = |k: &str| params.get(k).and_then(|v| v.as_str());
    let fresh = get("fresh") == Some("1");
    match fetch(get("repo"), get("number"), get("head"), fresh) {
        Ok(value) => {
            let mut out = json!({"ok": true});
            for (k, v) in value.as_object().cloned().unwrap_or_default() {
                out[k] = v;
            }
            Some(out)
        }
        Err(Some(message)) => Some(json!({"ok": false, "error": message})),
        Err(None) => None,
    }
}

fn fetch(repo: Option<&str>, number: Option<&str>, head: Option<&str>, fresh: bool) -> Result<Value, Failure> {
    let (repo, number) = checked(repo, number)?;
    let key = format!("{}#{}@{}", repo.to_lowercase(), number, head.unwrap_or(""));
    let at = now();
    if !fresh {
        let cache = CACHE.lock().unwrap();
        if let Some((t, v)) = cache.as_ref().and_then(|c| c.get(&key)) {
            if at - t < TTL {
                return Ok(v.clone());
            }
        }
    }
    let value = read(&repo, number)?;
    let mut cache = CACHE.lock().unwrap();
    let map = cache.get_or_insert_with(HashMap::new);
    map.retain(|_, (t, _)| at - *t < TTL);
    map.insert(key, (at, value.clone()));
    Ok(value)
}

const RESOLVE: &str = "mutation($id: ID!) { resolveReviewThread(input: {threadId: $id}) { thread { id isResolved } } }";
const UNRESOLVE: &str = "mutation($id: ID!) { unresolveReviewThread(input: {threadId: $id}) { thread { id isResolved } } }";
const REPLY: &str = "mutation($review: ID!, $thread: ID!, $body: String!) {\n  addPullRequestReviewThreadReply(input: {pullRequestReviewId: $review, pullRequestReviewThreadId: $thread, body: $body}) {\n    comment { id url }\n  }\n}\n";

/// A write through this module forgets its PR's cached threads, so the refetch after it is fresh.
pub fn forget(repo: &str, number: &str) {
    let prefix = format!("{}#{}@", repo.to_lowercase(), number);
    if let Some(map) = CACHE.lock().unwrap().as_mut() {
        map.retain(|key, _| !key.starts_with(&prefix));
    }
}

fn node_id(id: &str) -> bool {
    (1..=200).contains(&id.chars().count()) && id.chars().all(|c| word(c) || c == '-')
}

/// `ReviewThreads#resolve`: resolves (or unresolves) a thread at once; its new `isResolved`.
pub fn resolve(repo: &str, number: &str, thread: &Value, resolved: bool) -> Result<Value, Failure> {
    let result = (|| {
        let _ = checked(Some(repo), Some(number))?;
        let Some(id) = thread.as_str().filter(|t| node_id(t)) else {
            let shown = match thread {
                Value::Null => String::new(),
                Value::String(t) => t.clone(),
                Value::Bool(b) => b.to_string(),
                Value::Number(n) if n.is_i64() || n.is_u64() => n.to_string(),
                _ => return Err(None),
            };
            return Err(Some(format!("'{shown}' is not a review thread id")));
        };
        let data = graphql(if resolved { RESOLVE } else { UNRESOLVE }, &[("id", json!(id))])?;
        let key = if resolved { "resolveReviewThread" } else { "unresolveReviewThread" };
        match data.get(key).and_then(|v| v.get("thread")).filter(|t| !t.is_null() && **t != Value::Bool(false)) {
            Some(t) => Ok(t.get("isResolved").cloned().unwrap_or(Value::Null)),
            None => Err(Some(format!("GitHub did not {} the thread", if resolved { "resolve" } else { "unresolve" }))),
        }
    })();
    forget(repo, number);
    result
}

/// `ReviewThreads#reply`: a reply into the pending review `review_id`; the comment's `{id, url}`.
pub fn reply(repo: &str, number: &str, review_id: &str, thread_id: &str, body: &str) -> Result<Value, Failure> {
    let result = (|| {
        if crate::ruby_text::strip(body).is_empty() {
            return Err(Some("a reply can't be blank".to_string()));
        }
        let data = graphql(REPLY, &[("review", json!(review_id)), ("thread", json!(thread_id)), ("body", json!(body))])?;
        match data.get("addPullRequestReviewThreadReply").and_then(|v| v.get("comment")).filter(|c| !c.is_null() && **c != Value::Bool(false)) {
            Some(c) => Ok(c.clone()),
            None => Err(Some(format!("GitHub took no reply on thread {thread_id}"))),
        }
    })();
    forget(repo, number);
    result
}
