//! The benchmark modal's payload and files (`BenchmarkReport`), folded read-only from the run folders
//! under `benchmarks/`: each run's meta.json, events.jsonl (the bench script's progress events),
//! agent.jsonl (the planning agent's stream, for the activity line) and verdict.md.

use std::io::{Read, Seek, SeekFrom};

use serde_json::{json, Map, Value};

use crate::benchmark_run as run;
use crate::ruby_text::strip;
use crate::work_item::WorkItem;

const AGENT_TAIL_BYTES: u64 = 64 * 1024;
const LOG_TAIL_BYTES: u64 = 64 * 1024;
const LOGS: [&str; 5] = ["bench.log", "pipeline.log", "bench.out", "run.json", "bench.rb"];

pub type Reply = (&'static str, &'static str, Vec<u8>);

/// What stopped a payload: the `ArgumentError` Ruby raises, or a value only Ruby reads exactly.
pub enum Fail {
    Arg(String),
    Unported,
}

type R<T> = Result<T, Fail>;

fn arg<T>(m: impl Into<String>) -> R<T> {
    Err(Fail::Arg(m.into()))
}

fn json_reply(v: Value) -> Reply {
    ("200 OK", "application/json; charset=utf-8", v.to_string().into_bytes())
}

fn run_dir(id: &Option<String>) -> R<std::path::PathBuf> {
    match id {
        Some(i) if run::run_id(i) => Ok(run::dir().join(i)),
        other => {
            let shown = match other {
                None => "nil".to_string(),
                Some(i) => crate::ruby_text::inspect_ascii(i).ok_or(Fail::Unported)?,
            };
            arg(format!("bad benchmark run id {shown}"))
        }
    }
}

fn split_rest(rest: &str) -> (Option<String>, Option<String>) {
    if rest.is_empty() {
        return (None, None);
    }
    let (id, file) = match rest.split_once('/') {
        Some((a, b)) => (a, Some(b)),
        None => (rest, None),
    };
    (Some(crate::gui_server::decode(id)), file.map(crate::gui_server::decode))
}

fn plan_file(id: &Option<String>, file: &Option<String>) -> R<Vec<u8>> {
    let plans = run_dir(id)?.join("plans");
    let name = file.clone().unwrap_or_default();
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return arg("bad plan file name");
    }
    let path = plans.join(&name);
    if !path.is_file() {
        return arg(format!("no plan {name} in run {}", id.clone().unwrap_or_default()));
    }
    let (real, base) = (std::fs::canonicalize(&path).map_err(|_| Fail::Unported)?, std::fs::canonicalize(&plans).map_err(|_| Fail::Unported)?);
    if !real.starts_with(&base) || real == base {
        return arg("bad plan file name");
    }
    std::fs::read(&path).map_err(|_| Fail::Unported)
}

fn log_file(id: &Option<String>, name: &Option<String>) -> R<Vec<u8>> {
    let given = name.clone().unwrap_or_default();
    if !LOGS.contains(&given.as_str()) {
        let shown = match name {
            None => "nil".to_string(),
            Some(n) => crate::ruby_text::inspect_ascii(n).ok_or(Fail::Unported)?,
        };
        return arg(format!("no log {shown}"));
    }
    let path = run_dir(id)?.join(&given);
    if !path.is_file() {
        return arg(format!("no {given} in run {}", id.clone().unwrap_or_default()));
    }
    Ok(tail(&path, LOG_TAIL_BYTES).into_bytes())
}

fn tail(path: &std::path::Path, bytes: u64) -> String {
    let Ok(mut f) = std::fs::File::open(path) else { return String::new() };
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(size.saturating_sub(bytes)));
    let mut data = vec![];
    let _ = f.read_to_end(&mut data);
    String::from_utf8_lossy(&data).into_owned()
}

/// `GET /api/benchmark-plan/…`, `/api/benchmark-log/…` and `/api/benchmark/<slug>`; `None` for what only
/// Ruby answers exactly.
pub fn respond(route: &str, query: &std::collections::HashMap<String, String>) -> Option<Reply> {
    for (prefix, file) in [("/api/benchmark-plan/", true), ("/api/benchmark-log/", false)] {
        if let Some(rest) = route.strip_prefix(prefix) {
            let (id, name) = split_rest(rest);
            return match if file { plan_file(&id, &name) } else { log_file(&id, &name) } {
                Ok(body) => Some(("200 OK", "text/plain; charset=utf-8", body)),
                Err(Fail::Arg(m)) => Some(("404 Not Found", "text/plain; charset=utf-8", m.into_bytes())),
                Err(Fail::Unported) => None,
            };
        }
    }
    let slug = crate::gui_server::decode(route.strip_prefix("/api/benchmark/").unwrap_or(""));
    let item = match WorkItem::find(&slug) {
        Ok(i) => i,
        Err(e) => {
            if e.class != "ArgumentError" {
                crate::data_root::log_error("gui-benchmark", e.class, &e.message);
            }
            return Some(json_reply(json!({"ok": false, "error": e.message})));
        }
    };
    match call(item, query.get("branch").map(String::as_str), query.get("run").map(String::as_str)) {
        Ok(Value::Object(m)) => {
            let mut out = Map::new();
            out.insert("ok".into(), json!(true));
            out.extend(m);
            Some(json_reply(Value::Object(out)))
        }
        Ok(_) => None,
        Err(Fail::Arg(m)) => Some(json_reply(json!({"ok": false, "error": m}))),
        Err(Fail::Unported) => None,
    }
}

fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn all_metas() -> Vec<Value> {
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(run::dir()).map(|d| d.filter_map(|e| e.ok()).map(|e| e.path().join("meta.json")).filter(|p| p.exists()).collect()).unwrap_or_default();
    paths.sort();
    paths.iter().filter_map(|p| read_meta(p)).collect()
}

fn read_meta(path: &std::path::Path) -> Option<Value> {
    let id = path.parent()?.file_name()?.to_string_lossy().into_owned();
    if !run::run_id(&id) {
        return None;
    }
    let Value::Object(meta) = serde_json::from_str::<Value>(&std::fs::read_to_string(path).ok()?).ok()? else { return None };
    let request = text(meta.get("request"));
    let mut meta = Value::Object(meta);
    meta = run::merged(&meta, &[("id", json!(id)), ("request", json!(request))]);
    if run::is_final(&meta) || run::live_now(&meta) {
        return Some(meta);
    }
    Some(run::merged(&meta, &[("state", json!("failed")), ("error", json!("pipeline died"))]))
}

fn slice(meta: &Value, keys: &[&str]) -> Map<String, Value> {
    keys.iter().filter_map(|k| meta.get(*k).map(|v| ((*k).to_string(), v.clone()))).collect()
}

fn call(mut item: WorkItem, branch: Option<&str>, run_id: Option<&str>) -> R<Value> {
    let branch = branch.filter(|b| !b.is_empty());
    let metas = all_metas();
    let mut mine: Vec<&Value> = metas.iter().filter(|m| m.get("slug").and_then(Value::as_str) == Some(item.slug.as_str()) && m.get("branch").and_then(Value::as_str) == branch && (branch.is_some() || m.get("branch").is_none_or(Value::is_null))).collect();
    mine.sort_by_key(|m| (text(m.get("startedAt")), text(m.get("id"))));
    mine.reverse();
    let chosen: Option<Value> = match run_id.filter(|r| !r.is_empty()) {
        None => mine.first().map(|m| (*m).clone()),
        Some(id) => {
            run_dir(&Some(id.to_string()))?;
            match metas.iter().find(|m| m.get("id").and_then(Value::as_str) == Some(id)) {
                Some(m) => Some(m.clone()),
                None => return arg(format!("no benchmark run {id}")),
            }
        }
    };
    let busy = match metas.iter().find(|m| !run::is_final(m)) {
        Some(m) => Value::Object(slice(m, &["id", "slug", "branch", "state"])),
        None => Value::Null,
    };
    let run = match &chosen {
        Some(meta) => full(&mut item, meta)?,
        None => Value::Null,
    };
    Ok(json!({
        "enabled": run::enabled(&item), "branch": branch, "busy": busy,
        "runs": mine.iter().map(|m| Value::Object(slice(m, &["id", "startedAt", "state", "request", "rerunOf"]))).collect::<Vec<_>>(),
        "run": run,
    }))
}

fn read_events(path: &std::path::Path) -> Vec<Map<String, Value>> {
    let Ok(text) = std::fs::read_to_string(path) else { return vec![] };
    text.split_inclusive('\n').filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter_map(|v| v.as_object().cloned()).collect()
}

fn full(item: &mut WorkItem, meta: &Value) -> R<Value> {
    let dir = run::dir().join(text(meta.get("id")));
    let events = read_events(&dir.join("events.jsonl"));
    let current: Value = if matches!(meta.get("head"), None | Some(Value::Null) | Some(Value::Bool(false))) {
        Value::Null
    } else {
        match crate::branch_diff::tip(item, meta.get("branch").and_then(Value::as_str)) {
            Ok(tip) => tip["head"].clone(),
            Err(_) => Value::Null,
        }
    };
    let mut out = slice(meta, &["id", "slug", "branch", "request", "model", "startedAt", "finishedAt", "state", "error", "warning", "head", "rerunOf"]);
    out.insert("durationS".into(), duration(meta));
    out.insert("activity".into(), if meta.get("state") == Some(&json!("planning")) { activity(&dir)? } else { Value::Null });
    out.insert("currentHead".into(), current.clone());
    out.insert("moved".into(), json!(!current.is_null() && current != meta.get("head").cloned().unwrap_or(Value::Null)));
    out.insert("stages".into(), Value::Array(stages(meta).into_iter().map(Value::Object).collect()));
    out.insert("subjects".into(), subjects(&events));
    out.insert("rows".into(), rows(&events));
    out.insert("verdict".into(), verdict(&dir));
    out.insert("canRerun".into(), json!(can_rerun(meta, &dir)));
    out.insert("logs".into(), json!(LOGS.iter().filter(|n| dir.join(n).is_file()).collect::<Vec<_>>()));
    Ok(Value::Object(out))
}

fn stages(meta: &Value) -> Vec<Map<String, Value>> {
    match meta.get("stages") {
        Some(Value::Array(list)) => list.iter().filter(|s| s.is_object()).map(|s| slice(s, &["name", "ref", "sha"])).collect(),
        _ => vec![],
    }
}

fn subjects(events: &[Map<String, Value>]) -> Value {
    let mut by: Vec<(Value, Value)> = vec![];
    for e in events.iter().filter(|e| e.get("t") == Some(&json!("subject"))) {
        let (subject, about) = (e.get("subject").cloned().unwrap_or(Value::Null), e.get("about").cloned().unwrap_or(Value::Null));
        match by.iter_mut().find(|(k, _)| *k == subject) {
            Some((_, v)) => *v = about,
            None => by.push((subject, about)),
        }
    }
    Value::Array(by.into_iter().map(|(subject, about)| json!({"subject": subject, "about": about})).collect())
}

/// One row per planned subject × case (the last plan event's), then any measured one the plan didn't
/// name; cells keyed by stage name.
fn rows(events: &[Map<String, Value>]) -> Value {
    let get = |e: &Map<String, Value>, k: &str| e.get(k).cloned().unwrap_or(Value::Null);
    let mut rows: Vec<((Value, Value), Map<String, Value>)> = vec![];
    let upsert = |rows: &mut Vec<((Value, Value), Map<String, Value>)>, key: (Value, Value), row: Map<String, Value>, replace: bool| {
        match rows.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => {
                if replace {
                    *v = row;
                }
            }
            None => rows.push((key, row)),
        }
    };
    if let Some(plan) = events.iter().rev().find(|e| e.get("t") == Some(&json!("plan"))) {
        if let Some(Value::Array(cases)) = plan.get("cases") {
            for c in cases.iter().filter_map(Value::as_object) {
                let key = (get(c, "subject"), get(c, "case"));
                let row = Map::from_iter([("subject".to_string(), get(c, "subject")), ("case".to_string(), get(c, "case")), ("why".to_string(), get(c, "why")), ("cells".to_string(), json!({}))]);
                upsert(&mut rows, key, row, true);
            }
        }
    }
    for e in events {
        let key = (get(e, "subject"), get(e, "case"));
        match e.get("t").and_then(Value::as_str) {
            Some("case") => {
                if !rows.iter().any(|(k, _)| *k == key) {
                    rows.push((key.clone(), Map::from_iter([("subject".to_string(), get(e, "subject")), ("case".to_string(), get(e, "case")), ("why".to_string(), Value::Null), ("cells".to_string(), json!({}))])));
                }
                let row = &mut rows.iter_mut().find(|(k, _)| *k == key).unwrap().1;
                let mut cell: Map<String, Value> = ["ms", "timeout", "queries", "plan"].iter().filter_map(|k| e.get(*k).map(|v| ((*k).to_string(), v.clone()))).collect();
                cell.insert("ids".into(), Value::Null);
                cell.insert("vs".into(), Value::Null);
                if let Some(Value::Object(cells)) = row.get_mut("cells") {
                    cells.insert(text(e.get("stage")), Value::Object(cell));
                }
            }
            Some("ids") => {
                if let Some((_, row)) = rows.iter_mut().find(|(k, _)| *k == key) {
                    if let Some(Value::Object(cells)) = row.get_mut("cells") {
                        if let Some(Value::Object(cell)) = cells.get_mut(&text(e.get("stage"))) {
                            for k in ["ids", "vs"] {
                                if let Some(v) = e.get(k) {
                                    cell.insert(k.into(), v.clone());
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Value::Array(rows.into_iter().map(|(_, r)| Value::Object(r)).collect())
}

fn duration(meta: &Value) -> Value {
    if run::is_final(meta) && matches!(meta.get("finishedAt"), None | Some(Value::Null) | Some(Value::Bool(false))) {
        return Value::Null;
    }
    let started = chrono::DateTime::parse_from_rfc3339(&text(meta.get("startedAt"))).ok();
    let finished = match meta.get("finishedAt") {
        Some(Value::String(t)) => chrono::DateTime::parse_from_rfc3339(t).ok().map(|d| d.timestamp_millis() as f64 / 1000.0),
        _ => Some(crate::clock::now_f64()),
    };
    match (started, finished) {
        (Some(s), Some(f)) => json!((f - s.timestamp_millis() as f64 / 1000.0).round() as i64),
        _ => Value::Null,
    }
}

/// The agent's latest tool call as one line, read from the stream's tail.
fn activity(dir: &std::path::Path) -> R<Value> {
    let path = dir.join("agent.jsonl");
    if !path.is_file() {
        return Ok(Value::Null);
    }
    let tail = tail(&path, AGENT_TAIL_BYTES);
    for line in tail.split('\n').rev() {
        let Ok(Value::Object(entry)) = serde_json::from_str::<Value>(line) else { continue };
        if entry.get("type") != Some(&json!("assistant")) {
            continue;
        }
        let blocks = match entry.get("message").and_then(|m| m.get("content")) {
            Some(Value::Array(a)) => a.clone(),
            _ => continue,
        };
        if let Some(call) = blocks.iter().rev().find(|b| b.get("type") == Some(&json!("tool_use")) && b.is_object()) {
            return match crate::chat_history::tool_line_of(call.as_object().unwrap()) {
                Some(l) => Ok(json!(l)),
                None => Err(Fail::Unported),
            };
        }
    }
    Ok(Value::Null)
}

fn verdict(dir: &std::path::Path) -> Value {
    let path = dir.join("verdict.md");
    let text = if path.is_file() { std::fs::read_to_string(&path).map(|t| strip(&t).to_string()).unwrap_or_default() } else { String::new() };
    if text.is_empty() { Value::Null } else { json!(text) }
}

/// Re-running needs the script and its run.json, a final run, and every stage ref still naming the sha
/// it was measured at.
fn can_rerun(meta: &Value, dir: &std::path::Path) -> bool {
    if !run::is_final(meta) || !["bench.rb", "run.json"].iter().all(|f| dir.join(f).is_file()) {
        return false;
    }
    let stages = stages(meta);
    let root = text(meta.get("root"));
    !stages.is_empty() && stages.iter().all(|s| {
        let (Some(r), Some(sha)) = (s.get("ref").filter(|v| !v.is_null()), s.get("sha").filter(|v| !v.is_null())) else { return false };
        crate::git::git(Some(&root), &["rev-parse", "--verify", "--quiet", &format!("{}^{{commit}}", text(Some(r)))]).as_deref() == sha.as_str()
    })
}

