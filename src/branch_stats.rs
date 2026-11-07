//! The diff pane's branch stats (`BranchStats`): the branch-stats skill's own script run in the branch's
//! checkout, its markdown kept as printed and parsed into structure for the modal, cached per repo root
//! and branch while the head, merge base and base ref are the ones it was computed at; and "What stands
//! out", one headless haiku call on the user's click.

use std::sync::Mutex;

use regex::Regex;
use serde_json::{json, Map, Value};

use crate::branch_diff::Failure;
use crate::command_runner;
use crate::ruby_text::strip;
use crate::work_item::WorkItem;

const PROMPT: &str = "Below is a markdown rundown of the lines a git branch changes since its\nmerge base: totals by category, the largest files, and one row per\ncommit. Write two or three plain sentences of what stands out, drawn\nfrom the numbers: which commit carries the real change, whether a big\ncommit is mostly generated fan-out (locale JSON, a generated API\nclient, lockfiles), and the approximate hand-written non-test surface\n(backend + frontend + docs + openapi + locales). Do not restate the\ntables in prose, and do not judge whether the change is too large.\nAnswer with the sentences only.\n";

static CACHE_LOCK: Mutex<()> = Mutex::new(());

type R<T> = Result<T, Failure>;

fn arg<T>(m: impl Into<String>) -> R<T> {
    Err(Failure::Arg(m.into()))
}

fn script() -> String {
    format!("{}/skills/branch-stats/scripts/branch-stats.sh", env!("CARGO_MANIFEST_DIR"))
}

fn cache_file() -> std::path::PathBuf {
    crate::data_root::root().join("branch-stats.json")
}

pub fn job_key(target: &Value) -> String {
    format!("branch-stats:{}:{}", s(&target["root"]), s(&target["branch"]))
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

/// The branch and base the stats of the item's branch are measured on.
pub fn target(item: &mut WorkItem, branch: Option<&str>) -> R<Value> {
    crate::branch_diff::tip(item, branch)
}

fn read_cache() -> Map<String, Value> {
    let fresh = || Map::from_iter([("v".to_string(), json!(1)), ("entries".to_string(), json!({}))]);
    match std::fs::read_to_string(cache_file()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        Some(Value::Object(d)) if d.get("v") == Some(&json!(1)) && d.get("entries").is_some_and(Value::is_object) => d,
        _ => fresh(),
    }
}

fn write_cache(cache: &Map<String, Value>) -> R<()> {
    let _ = crate::data_root::ensure_setup();
    let path = cache_file();
    let temp = format!("{}.{}.{:?}.tmp", path.display(), std::process::id(), std::thread::current().id());
    std::fs::write(&temp, crate::drafts::pretty(&Value::Object(cache.clone()))).map_err(|_| Failure::Unported)?;
    std::fs::rename(&temp, &path).map_err(|_| Failure::Unported)
}

fn key_fields(target: &Value) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("branch".into(), target["branch"].clone());
    m.insert("base".into(), target["base"]["ref"].clone());
    m.insert("head".into(), target["head"].clone());
    m.insert("mergeBase".into(), target["base"]["sha"].clone());
    m
}

fn matches(entry: Option<&Value>, target: &Value) -> bool {
    let Some(Value::Object(e)) = entry else { return false };
    key_fields(target).iter().all(|(k, v)| e.get(k) == Some(v))
}

fn entry_at<'a>(cache: &'a Map<String, Value>, target: &Value) -> Option<&'a Value> {
    cache.get("entries")?.get(s(&target["root"]))?.get(s(&target["branch"]))
}

fn lookup(target: &Value) -> Option<Value> {
    let cache = read_cache();
    entry_at(&cache, target).filter(|e| matches(Some(e), target)).cloned()
}

fn compute(target: &Value) -> R<Value> {
    let fields = key_fields(target);
    let result = command_runner::call_in(&[&script(), &s(&target["base"]["ref"]), &s(&target["ref"])], &s(&target["path"]));
    let markdown = result.out.clone();
    if !(result.success && !strip(&markdown).is_empty()) {
        let reason = [strip(&result.err), strip(&markdown)].into_iter().find(|t| !t.is_empty()).unwrap_or("it printed nothing").to_string();
        let mut entry = fields;
        entry.insert("error".into(), json!(format!("the branch-stats script failed: {}", reason.chars().take(500).collect::<String>())));
        return Ok(Value::Object(entry));
    }
    let mut entry = fields;
    entry.insert("computedAt".into(), json!(crate::clock::now_iso()));
    entry.insert("markdown".into(), json!(markdown));
    let _lock = CACHE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut cache = read_cache();
    let Some(Value::Object(entries)) = cache.get_mut("entries") else { return Err(Failure::Unported) };
    let root = s(&target["root"]);
    let by_branch = entries.entry(root).or_insert_with(|| json!({}));
    let Some(by_branch) = by_branch.as_object_mut() else { return Err(Failure::Unported) };
    by_branch.insert(s(&target["branch"]), Value::Object(entry.clone()));
    write_cache(&cache)?;
    Ok(Value::Object(entry))
}

fn update(target: &Value, f: impl FnOnce(&Map<String, Value>) -> Map<String, Value>) -> R<bool> {
    let _lock = CACHE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut cache = read_cache();
    let Some(current) = entry_at(&cache, target).filter(|e| matches(Some(e), target)).and_then(|e| e.as_object()).cloned() else { return Ok(false) };
    let updated = f(&current);
    cache["entries"][s(&target["root"])][s(&target["branch"])] = Value::Object(updated);
    write_cache(&cache)?;
    Ok(true)
}

/// `{branch, base, head, mergeBase, computedAt, markdown, stats, commentary, commentaryAt, error, jobKey,
/// commentaryRunning}`.
pub fn call(item: &mut WorkItem, branch: Option<&str>, refresh: bool, running: &[String]) -> R<Value> {
    let target = target(item, branch)?;
    let entry = match if refresh { None } else { lookup(&target) } {
        Some(e) => e,
        None => compute(&target)?,
    };
    let key = job_key(&target);
    let get = |k: &str| entry.get(k).cloned().unwrap_or(Value::Null);
    let markdown = get("markdown");
    let stats = match &markdown {
        Value::String(m) => parse(m).unwrap_or(Value::Null),
        _ => Value::Null,
    };
    Ok(json!({
        "branch": get("branch"), "base": get("base"), "head": get("head"), "mergeBase": get("mergeBase"),
        "computedAt": get("computedAt"), "markdown": markdown, "stats": stats,
        "commentary": get("commentary"), "commentaryAt": get("commentaryAt"), "error": get("error"),
        "jobKey": key, "commentaryRunning": running.contains(&job_key(&target)),
    }))
}

/// Blocking: what the commentary job runs. The answer is stored only while the entry still matches.
pub fn comment(target: &Value) -> R<Value> {
    let branch = s(&target["branch"]);
    let Some(entry) = lookup(target) else { return arg(format!("{branch} has no branch stats for its current head yet")) };
    let prompt = format!("{PROMPT}\n{}", s(&entry["markdown"]));
    let result = command_runner::call(&["env", "QM_SKIP=1", "claude", "-p", &prompt, "--model", "haiku", "--tools", "", "--strict-mcp-config", "--no-session-persistence"]);
    let text = strip(&result.out).to_string();
    if !(result.success && !text.is_empty()) {
        let shown = if strip(&result.err).is_empty() { text.clone() } else { strip(&result.err).to_string() };
        return arg(format!("claude -p failed: {}", shown.chars().take(200).collect::<String>()));
    }
    let stored = update(target, |current| {
        let mut m = current.clone();
        m.insert("commentary".into(), json!(text));
        m.insert("commentaryAt".into(), json!(crate::clock::now_iso()));
        m
    })?;
    if stored {
        return Ok(json!({"ok": true, "message": format!("{branch}: what stands out is ready")}));
    }
    Ok(json!({"ok": false, "error": format!("{branch} moved on while its stats were read: the commentary was dropped")}))
}

fn int(t: &str) -> i64 {
    t.parse().unwrap_or(0)
}

/// "+9 / -2" or "+983" → `{add, del}`.
fn cell_lines(cell: &str) -> Map<String, Value> {
    let grab = |p: &str| Regex::new(p).unwrap().captures(cell).map(|c| int(&c[1])).unwrap_or(0);
    Map::from_iter([("add".to_string(), json!(grab(r"\+([0-9]+)"))), ("del".to_string(), json!(grab(r"-([0-9]+)")))])
}

/// The script's markdown as structure, or `None` when it isn't the shape the script prints.
pub fn parse(markdown: &str) -> Option<Value> {
    let header = Regex::new(r"(?m)^# Branch stats: `(.+?)` vs `(.+?)` \(merge base `([0-9a-fA-F]+)`\)$").unwrap().captures(markdown)?;
    let totals = Regex::new(r"(?m)^\*\*Whole branch: ([0-9]+) files, \+([0-9]+) / -([0-9]+)\*\*$").unwrap().captures(markdown)?;
    let categories: Vec<Value> = Regex::new(r"(?m)^\| ([a-z]+) \| ([0-9]+) \| ([0-9]+) \| ([0-9]+) \| ([0-9]+)% \|$").unwrap().captures_iter(markdown).map(|c| json!({"name": &c[1], "files": int(&c[2]), "add": int(&c[3]), "del": int(&c[4]), "share": int(&c[5])})).collect();
    let largest: Vec<Value> = Regex::new(r"(?m)^- `(.+)` \+([0-9]+) / -([0-9]+)$").unwrap().captures_iter(markdown).map(|c| json!({"path": &c[1], "add": int(&c[2]), "del": int(&c[3])})).collect();
    Some(json!({
        "branch": &header[1], "base": &header[2], "mergeBase": &header[3],
        "totals": {"files": int(&totals[1]), "add": int(&totals[2]), "del": int(&totals[3])},
        "categories": categories, "largest": largest, "commits": commit_rows(markdown),
    }))
}

fn commit_rows(markdown: &str) -> Vec<Value> {
    let cell = "([^|]*)";
    let pattern = format!(r"(?m)^\| `([0-9a-fA-F]+)` (.*) \| ([0-9]+) \| {cell} \| {cell} \| {cell} \| {cell} \| {cell} \| {cell} \|$");
    Regex::new(&pattern)
        .unwrap()
        .captures_iter(markdown)
        .map(|c| {
            let mut categories = Map::new();
            for (name, i) in [("backend", 5), ("frontend", 6), ("tests", 7), ("generated", 8)] {
                if !strip(&c[i]).is_empty() {
                    categories.insert(name.into(), Value::Object(cell_lines(&c[i])));
                }
            }
            let mut other: Vec<&str> = c[9].split(", ").collect();
            while other.last() == Some(&"") {
                other.pop();
            }
            for part in other {
                if let Some(m) = Regex::new(r"^([a-z]+) (.+)$").unwrap().captures(part).filter(|_| !part.contains('\n')) {
                    categories.insert(m[1].to_string(), Value::Object(cell_lines(&m[2])));
                }
            }
            let mut row = Map::new();
            row.insert("sha".into(), json!(&c[1]));
            row.insert("subject".into(), json!(&c[2]));
            row.insert("files".into(), json!(int(&c[3])));
            row.extend(cell_lines(&c[4]));
            row.insert("categories".into(), Value::Object(categories));
            Value::Object(row)
        })
        .collect()
}
