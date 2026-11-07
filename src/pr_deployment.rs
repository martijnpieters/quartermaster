//! Whether each PR prerequisite is deployed: contained in its repo's latest published release (`PrDeployment`).
//! Answers come from a disk cache only; GitHub is asked (read-only) on a background thread, so no caller
//! ever waits on gh. States: deployed (with the release tag), pending, unmerged, closed, unknown. The same
//! refresh asks each dependent PR's state and prunes the store's entries whose PR GitHub confirms merged or
//! closed.

use std::process::{Command, Stdio};
use std::sync::Mutex;

use serde_json::{json, Map, Value};

use crate::data_root;
use crate::ruby_text::strip;

const TTL_SECONDS: i64 = 5 * 60;

static IN_FLIGHT: Mutex<bool> = Mutex::new(false);

pub fn refreshing() -> bool {
    *IN_FLIGHT.lock().unwrap_or_else(|p| p.into_inner())
}

fn file() -> std::path::PathBuf {
    data_root::root().join("pr-deployment.json")
}

fn now() -> i64 {
    crate::clock::now_f64() as i64
}

fn read_cache() -> Map<String, Value> {
    let fresh = || Map::from_iter([("v".to_string(), json!(1)), ("releases".to_string(), json!({})), ("pulls".to_string(), json!({})), ("verdicts".to_string(), json!({}))]);
    let Some(Value::Object(mut data)) = std::fs::read_to_string(file()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { return fresh() };
    if data.get("v").and_then(Value::as_f64) != Some(1.0) {
        return fresh();
    }
    for key in ["releases", "pulls", "verdicts"] {
        if !data.get(key).is_some_and(Value::is_object) {
            data.insert(key.into(), json!({}));
        }
    }
    data
}

fn write_cache(cache: &Map<String, Value>) {
    let _ = data_root::ensure_setup();
    let temp = format!("{}.{}.{:?}.tmp", file().display(), std::process::id(), std::thread::current().id());
    if std::fs::write(&temp, crate::drafts::pretty(&Value::Object(cache.clone()))).is_ok() {
        let _ = std::fs::rename(&temp, file());
    }
}

fn section<'a>(cache: &'a mut Map<String, Value>, name: &str) -> &'a mut Map<String, Value> {
    cache.get_mut(name).and_then(Value::as_object_mut).expect("sections exist")
}

fn get<'a>(cache: &'a Map<String, Value>, name: &str, key: &str) -> Option<&'a Value> {
    cache.get(name).and_then(Value::as_object).and_then(|s| s.get(key)).filter(|v| !v.is_null())
}

fn ts_of(v: Option<&Value>) -> i64 {
    v.and_then(|v| v.get("ts")).and_then(Value::as_i64).unwrap_or(0)
}

fn expired(v: Option<&Value>) -> bool {
    v.is_none() || now() - ts_of(v) >= TTL_SECONDS
}

fn pull_stale(cache: &Map<String, Value>, url: &str) -> bool {
    let pull = get(cache, "pulls", url);
    let is_final = pull.is_some_and(|p| p.get("state").and_then(Value::as_str) == Some("MERGED") && p.get("mergeCommit").is_some_and(|m| !m.is_null()));
    !is_final && expired(pull)
}

/// The commit an entry waits on (`Ok`), or its whole verdict when it has none (`Err`).
fn entry_sha(cache: &Map<String, Value>, entry: &Map<String, Value>) -> Result<String, Value> {
    let reference = entry.get("ref").and_then(Value::as_str).unwrap_or("");
    if entry.get("kind").and_then(Value::as_str) == Some("commit") {
        return Ok(reference.to_string());
    }
    let pull = get(cache, "pulls", reference);
    match pull.and_then(|p| p.get("state")).and_then(Value::as_str) {
        Some("OPEN") => Err(json!({"state": "unmerged"})),
        Some("CLOSED") => Err(json!({"state": "closed"})),
        Some("MERGED") => match pull.and_then(|p| p.get("mergeCommit")).and_then(Value::as_str) {
            Some(sha) => Ok(sha.to_string()),
            None => Err(json!({"state": "unknown"})),
        },
        _ => Err(json!({"state": "unknown"})),
    }
}

fn sha_stale(cache: &Map<String, Value>, entry: &Map<String, Value>) -> bool {
    let Ok(sha) = entry_sha(cache, entry) else { return false };
    let checked = get(cache, "verdicts", &sha);
    if checked.is_some_and(|c| !matches!(c.get("deployed"), None | Some(Value::Null) | Some(Value::Bool(false)))) {
        return false;
    }
    let repo = entry.get("repo").and_then(Value::as_str).unwrap_or("");
    let tag = get(cache, "releases", repo).and_then(|r| r.get("tag")).and_then(Value::as_str);
    match tag {
        None => false,
        Some(t) => checked.is_none() || checked.and_then(|c| c.get("tag")).and_then(Value::as_str) != Some(t) || expired(checked),
    }
}

fn stale(cache: &Map<String, Value>, prs: &Map<String, Value>) -> bool {
    let entries: Vec<&Map<String, Value>> = prs.values().filter_map(Value::as_array).flatten().filter_map(Value::as_object).collect();
    let mut repos: Vec<&str> = vec![];
    for e in &entries {
        let repo = e.get("repo").and_then(Value::as_str).unwrap_or("");
        if !repos.contains(&repo) {
            repos.push(repo);
        }
    }
    repos.iter().any(|r| expired(get(cache, "releases", r)))
        || prs.keys().any(|u| pull_stale(cache, u))
        || entries.iter().any(|e| e.get("kind").and_then(Value::as_str) == Some("pr") && pull_stale(cache, e.get("ref").and_then(Value::as_str).unwrap_or("")))
        || entries.iter().any(|e| sha_stale(cache, e))
}

fn verdict(cache: &Map<String, Value>, entry: &Map<String, Value>) -> Map<String, Value> {
    let sha = match entry_sha(cache, entry) {
        Err(v) => return v.as_object().cloned().unwrap_or_default(),
        Ok(s) => s,
    };
    match get(cache, "verdicts", &sha) {
        Some(c) if !matches!(c.get("deployed"), None | Some(Value::Null) | Some(Value::Bool(false))) => json!({"state": "deployed", "tag": c.get("tag").cloned().unwrap_or(Value::Null)}),
        Some(c) => json!({"state": "pending", "tag": c.get("tag").cloned().unwrap_or(Value::Null)}),
        None => json!({"state": "unknown"}),
    }
    .as_object()
    .cloned()
    .unwrap_or_default()
}

/// Every dependent PR's prerequisites, each with "state" (and "tag" when it names a release), from the cache.
/// A stale or missing answer starts a background refresh.
pub fn states(prs: &Map<String, Value>, refresh: bool) -> Map<String, Value> {
    let cache = read_cache();
    if refresh && stale(&cache, prs) {
        refresh_in_background();
    }
    prs.iter()
        .map(|(url, list)| {
            let entries: Vec<Value> = list.as_array().cloned().unwrap_or_default().into_iter().map(|e| {
                let Value::Object(mut m) = e else { return e };
                for (k, v) in verdict(&cache, &m.clone()) {
                    m.insert(k, v);
                }
                Value::Object(m)
            }).collect();
            (url.clone(), Value::Array(entries))
        })
        .collect()
}

fn refresh_in_background() {
    {
        let mut flying = IN_FLIGHT.lock().unwrap_or_else(|p| p.into_inner());
        if *flying {
            return;
        }
        *flying = true;
    }
    std::thread::spawn(|| {
        refresh();
        *IN_FLIGHT.lock().unwrap_or_else(|p| p.into_inner()) = false;
    });
}

fn gh(args: &[&str]) -> Result<String, String> {
    let out = Command::new("gh").args(args).stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned());
    if !out.status.success() {
        let shown = strip(if strip(&stderr).is_empty() { &stdout } else { &stderr }).chars().take(200).collect::<String>();
        return Err(format!("gh {} failed: {shown}", args.iter().take(2).copied().collect::<Vec<_>>().join(" ")));
    }
    Ok(stdout)
}

fn fetch_release(cache: &mut Map<String, Value>, repo: &str) {
    if !expired(get(cache, "releases", repo)) {
        return;
    }
    let result = gh(&["api", &format!("repos/{repo}/releases/latest"), "--jq", ".tag_name"]).map(|t| strip(&t).to_string()).and_then(|t| if t.is_empty() { Err(format!("{repo} has no latest release")) } else { Ok(t) });
    match result {
        Ok(tag) => {
            section(cache, "releases").insert(repo.to_string(), json!({"tag": tag, "ts": now()}));
        }
        Err(m) => {
            data_root::log_error("pr-deployment", "RuntimeError", &m);
            stamp_failure(cache, "releases", repo);
        }
    }
}

fn stamp_failure(cache: &mut Map<String, Value>, name: &str, key: &str) {
    let mut current = get(cache, name, key).and_then(Value::as_object).cloned().unwrap_or_default();
    current.insert("ts".into(), json!(now()));
    section(cache, name).insert(key.to_string(), Value::Object(current));
}

/// A dependent PR's state, fetched when stale: `None` when the lookup failed.
fn fetch_pull(cache: &mut Map<String, Value>, url: &str) -> Option<Value> {
    if !pull_stale(cache, url) {
        return get(cache, "pulls", url).cloned();
    }
    let fetched = gh(&["pr", "view", url, "--json", "state,mergeCommit"]).and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()));
    match fetched {
        Ok(data) => {
            let merge = data.get("mergeCommit").and_then(|m| m.get("oid")).cloned().unwrap_or(Value::Null);
            let pull = json!({"state": data.get("state").cloned().unwrap_or(Value::Null), "mergeCommit": merge, "ts": now()});
            section(cache, "pulls").insert(url.to_string(), pull.clone());
            Some(pull)
        }
        Err(m) => {
            data_root::log_error("pr-deployment", "RuntimeError", &m);
            stamp_failure(cache, "pulls", url);
            None
        }
    }
}

fn check_entry(cache: &mut Map<String, Value>, entry: &Map<String, Value>) {
    let reference = entry.get("ref").and_then(Value::as_str).unwrap_or("");
    if entry.get("kind").and_then(Value::as_str) == Some("pr") {
        fetch_pull(cache, reference);
    }
    if !sha_stale(cache, entry) {
        return;
    }
    let Ok(sha) = entry_sha(cache, entry) else { return };
    let repo = entry.get("repo").and_then(Value::as_str).unwrap_or("");
    let Some(tag) = get(cache, "releases", repo).and_then(|r| r.get("tag")).and_then(Value::as_str).map(String::from) else { return };
    match gh(&["api", &format!("repos/{repo}/compare/{tag}...{sha}"), "--jq", ".status"]) {
        Ok(status) => {
            let deployed = matches!(strip(&status), "behind" | "identical");
            section(cache, "verdicts").insert(sha, json!({"tag": tag, "deployed": deployed, "ts": now()}));
        }
        Err(m) => data_root::log_error("pr-deployment", "RuntimeError", &m),
    }
}

/// Blocking: what the background thread runs. Each lookup fails on its own and keeps that part's last good answer.
pub fn refresh() {
    let Ok(prs) = crate::pr_prerequisites::all() else { return };
    let mut cache = read_cache();
    let entries: Vec<Map<String, Value>> = prs.values().filter_map(Value::as_array).flatten().filter_map(Value::as_object).cloned().collect();
    let mut repos: Vec<String> = vec![];
    for e in &entries {
        let repo = e.get("repo").and_then(Value::as_str).unwrap_or("").to_string();
        if !repos.contains(&repo) {
            repos.push(repo);
        }
    }
    for repo in &repos {
        fetch_release(&mut cache, repo);
    }
    let closed: Vec<String> = prs.keys().filter(|url| fetch_pull(&mut cache, url).is_some_and(|p| matches!(p.get("state").and_then(Value::as_str), Some("MERGED" | "CLOSED")))).cloned().collect();
    for e in &entries {
        check_entry(&mut cache, e);
    }
    // keep the cache to what the store still names
    let mut urls: Vec<String> = prs.keys().cloned().collect();
    urls.extend(entries.iter().filter(|e| e.get("kind").and_then(Value::as_str) == Some("pr")).filter_map(|e| e.get("ref").and_then(Value::as_str).map(String::from)));
    let shas: Vec<String> = entries.iter().filter_map(|e| entry_sha(&cache, e).ok()).collect();
    section(&mut cache, "pulls").retain(|u, _| urls.contains(u));
    section(&mut cache, "verdicts").retain(|s, _| shas.contains(s));
    section(&mut cache, "releases").retain(|r, _| entries.iter().any(|e| e.get("repo").and_then(Value::as_str) == Some(r.as_str())));
    write_cache(&cache);
    if !closed.is_empty() {
        let _ = crate::pr_prerequisites::prune(&closed);
    }
}
