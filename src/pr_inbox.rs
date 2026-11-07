//! The GUI's PRs view: every open pull request the user wrote, is asked to review, or is assigned, across
//! all repos — one `gh api graphql` call with a search per role. That call takes ~10 s, so the answer is
//! cached on disk and refetched on a thread: a list request never waits on GitHub, it says "refreshing"
//! and the page polls.

use std::process::{Command, Stdio};
use std::sync::Mutex;

use serde_json::{json, Map, Value};

use crate::data_root;
use crate::ruby_text::strip;

const TTL_SECONDS: i64 = 5 * 60;
const ROLES: [(&str, &str); 3] = [("author", "author:@me"), ("review", "user-review-requested:@me"), ("assignee", "assignee:@me")];
const FIELDS: &str = "number title url isDraft updatedAt headRefName baseRefName reviewDecision latestReviews(first: 20) { nodes { state } } author { login } repository { nameWithOwner } commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 100) { nodes { ... on CheckRun { name status conclusion startedAt checkSuite { databaseId createdAt app { slug } workflowRun { workflow { name } } } } ... on StatusContext { context state createdAt } } } } } } }";
const CI_OK: [&str; 3] = ["SUCCESS", "NEUTRAL", "SKIPPED"];
const CI_BAD: [&str; 7] = ["FAILURE", "ERROR", "CANCELLED", "TIMED_OUT", "ACTION_REQUIRED", "STARTUP_FAILURE", "STALE"];

static IN_FLIGHT: Mutex<bool> = Mutex::new(false);
static CACHE_LOCK: Mutex<()> = Mutex::new(());

fn cache_file() -> std::path::PathBuf {
    data_root::root().join("pr-inbox.json")
}

fn read_cache() -> Map<String, Value> {
    match std::fs::read_to_string(cache_file()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

fn write_cache(cache: &Map<String, Value>) {
    let _ = data_root::ensure_setup();
    let _guard = CACHE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let _ = std::fs::write(cache_file(), Value::Object(cache.clone()).to_string());
}

fn in_flight() -> bool {
    *IN_FLIGHT.lock().unwrap_or_else(|p| p.into_inner())
}

pub fn refreshing() -> bool {
    in_flight()
}

/// `{"prs", "fetchedAt", "error", "refreshing"}`: an expired or missing cache (or `force`, the page's
/// Refresh) starts a background fetch and answers with what it has meanwhile.
pub fn list(force: bool) -> Value {
    let cache = read_cache();
    let ts = cache.get("ts").and_then(Value::as_i64);
    let stale = force || ts.is_none_or(|t| crate::clock::now_f64() as i64 - t >= TTL_SECONDS);
    let refreshing = if stale { refresh_in_background() } else { in_flight() };
    let fetched = ts.and_then(|t| chrono::DateTime::<chrono::Utc>::from_timestamp(t, 0)).map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    json!({"prs": cache.get("prs").cloned().filter(|v| !matches!(v, Value::Null | Value::Bool(false))).unwrap_or_else(|| json!([])), "fetchedAt": fetched, "error": cache.get("error").cloned().unwrap_or(Value::Null), "refreshing": refreshing})
}

/// The cached PR at this URL: actions name a PR by URL and take every other field from here.
pub fn find(url: &str) -> Result<Value, String> {
    cached_prs().into_iter().find(|pr| pr.get("url").and_then(Value::as_str) == Some(url)).ok_or_else(|| format!("no open PR {url} in the PR list — refresh it"))
}

/// The cached list as it stands: never fetches.
pub fn cached_prs() -> Vec<Value> {
    match read_cache().get("prs") {
        Some(Value::Array(items)) => items.clone(),
        _ => vec![],
    }
}

fn refresh_in_background() -> bool {
    {
        let mut flying = IN_FLIGHT.lock().unwrap_or_else(|p| p.into_inner());
        if *flying {
            return true;
        }
        *flying = true;
    }
    std::thread::spawn(|| {
        refresh();
        *IN_FLIGHT.lock().unwrap_or_else(|p| p.into_inner()) = false;
    });
    true
}

/// Blocking fetch; the background refresh calls it. A failure keeps the last good list and records why.
pub fn refresh() {
    let now = crate::clock::now_f64() as i64;
    match fetch_via_gh() {
        Ok(prs) => write_cache(&Map::from_iter([("prs".to_string(), Value::Array(prs)), ("ts".to_string(), json!(now)), ("error".to_string(), Value::Null)])),
        Err(message) => {
            data_root::log_error("pr-inbox", "RuntimeError", &message);
            let mut cache = read_cache();
            cache.insert("ts".into(), json!(now));
            cache.insert("error".into(), json!(message));
            write_cache(&cache);
        }
    }
}

fn fetch_via_gh() -> Result<Vec<Value>, String> {
    let searches: Vec<String> = ROLES.iter().map(|(role, qualifier)| format!("{role}: search(query: \"is:open is:pr archived:false {qualifier}\", type: ISSUE, first: 50) {{ nodes {{ ...pr }} }}")).collect();
    let query = format!("query {{ {} }} fragment pr on PullRequest {{ {FIELDS} }}", searches.join(" "));
    let out = Command::new("gh").args(["api", "graphql", "-f", &format!("query={query}")]).stdin(Stdio::null()).output().map_err(|e| format!("No such file or directory - gh ({e})"))?;
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned());
    if !out.status.success() {
        let shown = strip(if strip(&stderr).is_empty() { &stdout } else { &stderr }).chars().take(200).collect::<String>();
        return Err(format!("gh api graphql failed: {shown}"));
    }
    let body: Value = serde_json::from_str(&stdout).map_err(|e| format!("unexpected token ({e})"))?;
    if let Some(errors) = body.get("errors").filter(|e| !matches!(e, Value::Null | Value::Bool(false))) {
        let joined = errors.as_array().cloned().unwrap_or_default().iter().map(|e| e.get("message").and_then(Value::as_str).unwrap_or("").to_string()).collect::<Vec<_>>().join("; ");
        return Err(format!("gh api graphql: {}", joined.chars().take(200).collect::<String>()));
    }
    let data = body.get("data").cloned().unwrap_or(Value::Null);
    let by_role: Vec<(&str, Vec<Value>)> = ROLES.iter().map(|(role, _)| (*role, data.get(*role).and_then(|r| r.get("nodes")).and_then(Value::as_array).cloned().unwrap_or_default())).collect();
    Ok(merge(&by_role))
}

fn dig<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(v, |acc, k| acc.get(*k)).filter(|v| !v.is_null())
}

/// Three searches' results merged by URL, each PR carrying every role it matched, newest activity first.
pub fn merge(by_role: &[(&str, Vec<Value>)]) -> Vec<Value> {
    let mut merged: Vec<(String, Map<String, Value>)> = vec![];
    for (role, nodes) in by_role {
        for node in nodes {
            let Some(url) = node.get("url").and_then(Value::as_str).filter(|u| !u.is_empty()) else { continue };
            let at = match merged.iter().position(|(u, _)| u == url) {
                Some(i) => i,
                None => {
                    let mut pr = Map::new();
                    pr.insert("url".into(), json!(url));
                    pr.insert("number".into(), node.get("number").cloned().unwrap_or(Value::Null));
                    pr.insert("title".into(), node.get("title").cloned().unwrap_or(Value::Null));
                    pr.insert("repo".into(), dig(node, &["repository", "nameWithOwner"]).cloned().unwrap_or(Value::Null));
                    pr.insert("author".into(), dig(node, &["author", "login"]).cloned().unwrap_or(Value::Null));
                    pr.insert("head".into(), node.get("headRefName").cloned().unwrap_or(Value::Null));
                    pr.insert("base".into(), node.get("baseRefName").cloned().unwrap_or(Value::Null));
                    pr.insert("draft".into(), json!(!matches!(node.get("isDraft"), None | Some(Value::Null) | Some(Value::Bool(false)))));
                    pr.insert("reviewDecision".into(), review_decision(node));
                    pr.insert("ci".into(), ci(node));
                    pr.insert("updatedAt".into(), node.get("updatedAt").cloned().unwrap_or(Value::Null));
                    pr.insert("roles".into(), json!([]));
                    merged.push((url.to_string(), pr));
                    merged.len() - 1
                }
            };
            if let Some(Value::Array(roles)) = merged[at].1.get_mut("roles") {
                roles.push(json!(role));
            }
        }
    }
    let mut prs: Vec<Map<String, Value>> = merged.into_iter().map(|(_, m)| m).collect();
    prs.sort_by(|a, b| a["updatedAt"].as_str().unwrap_or("").cmp(b["updatedAt"].as_str().unwrap_or("")));
    prs.reverse();
    prs.into_iter().map(Value::Object).collect()
}

/// GitHub only sets reviewDecision where branch protection requires reviews, so a PR onto a stacked branch
/// has none however it was reviewed: then each reviewer's latest review decides, changes requested over approved.
fn review_decision(node: &Value) -> Value {
    if let Some(d) = node.get("reviewDecision").filter(|v| !matches!(v, Value::Null | Value::Bool(false))) {
        return d.clone();
    }
    let states: Vec<&str> = dig(node, &["latestReviews", "nodes"]).and_then(Value::as_array).map(|n| n.iter().filter_map(|r| r.get("state").and_then(Value::as_str)).collect()).unwrap_or_default();
    ["CHANGES_REQUESTED", "APPROVED"].iter().find(|s| states.contains(s)).map(|s| json!(s)).unwrap_or(Value::Null)
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// The head commit's checks as GitHub's PR page shows them: passing / total and SUCCESS, FAILURE or
/// PENDING; null for a PR without any. Like GitHub, only each workflow's newest check suite counts, and
/// within it each check's latest attempt.
fn ci(node: &Value) -> Value {
    let nodes = dig(node, &["commits", "nodes"]).and_then(Value::as_array).and_then(|n| n.first()).and_then(|c| dig(c, &["commit", "statusCheckRollup", "contexts", "nodes"])).and_then(Value::as_array).cloned().unwrap_or_default();
    let contexts: Vec<&Value> = nodes.iter().filter(|c| !c.is_null()).collect();
    let (statuses, runs): (Vec<&Value>, Vec<&Value>) = contexts.into_iter().partition(|c| c.get("context").is_some());
    // runs grouped by (app slug, workflow name), in first-seen order
    let mut groups: Vec<(Value, Vec<&Value>)> = vec![];
    for run in runs {
        let key = json!([dig(run, &["checkSuite", "app", "slug"]).cloned().unwrap_or(Value::Null), dig(run, &["checkSuite", "workflowRun", "workflow", "name"]).cloned().unwrap_or(Value::Null)]);
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, list)) => list.push(run),
            None => groups.push((key, vec![run])),
        }
    }
    let mut suites: Vec<Vec<&Value>> = vec![];
    for (_, rs) in groups {
        // each group's newest check suite (its runs)
        let mut by_suite: Vec<(Value, Vec<&Value>)> = vec![];
        for r in rs {
            let suite = r.get("checkSuite").cloned().unwrap_or(Value::Null);
            match by_suite.iter_mut().find(|(s, _)| *s == suite) {
                Some((_, list)) => list.push(r),
                None => by_suite.push((suite, vec![r])),
            }
        }
        let rank = |s: &Value| (text(s, "createdAt").to_string(), s.get("databaseId").and_then(Value::as_i64).unwrap_or(0));
        let mut best: Option<usize> = None;
        for (i, (s, _)) in by_suite.iter().enumerate() {
            if best.is_none_or(|b| rank(s) > rank(&by_suite[b].0)) {
                best = Some(i);
            }
        }
        if let Some(b) = best {
            suites.push(by_suite.swap_remove(b).1);
        }
    }
    suites.push(statuses);
    let mut latest: Vec<&Value> = vec![];
    for checks in suites {
        let mut by_name: Vec<(Option<String>, Vec<&Value>)> = vec![];
        for c in checks {
            let name = c.get("name").filter(|v| !matches!(v, Value::Null | Value::Bool(false))).or_else(|| c.get("context")).and_then(Value::as_str).map(String::from);
            match by_name.iter_mut().find(|(n, _)| *n == name) {
                Some((_, list)) => list.push(c),
                None => by_name.push((name, vec![c])),
            }
        }
        for (_, attempts) in by_name {
            let when = |c: &Value| c.get("startedAt").filter(|v| !matches!(v, Value::Null | Value::Bool(false))).or_else(|| c.get("createdAt")).and_then(Value::as_str).unwrap_or("").to_string();
            let mut best = attempts[0];
            for a in &attempts[1..] {
                if when(a) > when(best) {
                    best = a;
                }
            }
            latest.push(best);
        }
    }
    if latest.is_empty() {
        return Value::Null;
    }
    let results: Vec<&str> = latest.iter().map(|c| if c.get("context").is_some() { text(c, "state") } else { text(c, "conclusion") }).collect();
    let state = if results.iter().any(|r| CI_BAD.contains(r)) {
        "FAILURE"
    } else if results.iter().all(|r| CI_OK.contains(r)) {
        "SUCCESS"
    } else {
        "PENDING"
    };
    json!({"state": state, "ok": results.iter().filter(|r| CI_OK.contains(r)).count(), "total": latest.len()})
}
