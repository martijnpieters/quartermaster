//! A local mirror of the delegate-build status boards (`BoardMirror`), folded from the PostToolUse hook's
//! `board-writes.jsonl`: set replaces, update deep-merges (a `{"__delete__": true}` value drops the
//! field), str_replace edits one string field in place, delete removes, batch applies each entry in
//! order; last write wins. The file only grows, so a mirror keeps its byte offset and folds just the new
//! complete lines on each refresh; a shrunken file is refolded from the start.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Mutex;

use regex::Regex;
use serde_json::{json, Map, Value};

use crate::data_root;
use crate::ruby_text::strip;
use crate::work_item::WorkItem;

const ROW_COLLECTION: &str = "tasks";
const ROW_FIELDS: [&str; 8] = ["status", "title", "note", "order", "dependsOn", "batch", "batchLabel", "updatedAt"];
const WRITE_ACTIONS: [&str; 5] = ["set", "update", "str_replace", "delete", "batch"];

type Docs = Map<String, Value>;

#[derive(Default)]
struct Mirror {
    offset: u64,
    /// board key → collection → doc id → data
    boards: Map<String, Value>,
}

static SHARED: Mutex<Option<HashMap<String, Mirror>>> = Mutex::new(None);

pub fn writes_file() -> std::path::PathBuf {
    data_root::root().join("board-writes.jsonl")
}

/// A `Err` is a value this port does not read exactly.
pub struct Unported;

fn scalar(v: &Value) -> Result<String, Unported> {
    match v {
        Value::Null => Ok(String::new()),
        Value::String(t) => Ok(t.clone()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Number(n) if n.is_i64() || n.is_u64() => Ok(n.to_string()),
        _ => Err(Unported),
    }
}

/// The id of a claude.ai artifact link (with or without a query or trailing slash), else the cleaned text.
pub fn board_key(url: &Value) -> Result<String, Unported> {
    let text = scalar(url)?;
    let url = strip(&text);
    if let Some(c) = Regex::new(r"/artifact/([^/?#]+)").unwrap().captures(url) {
        return Ok(c[1].to_string());
    }
    Ok(url.trim_end_matches(['/', '?', '#']).to_string())
}

fn truthy(v: Option<&Value>) -> Option<&Value> {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => None,
        other => other,
    }
}

fn delete_marker(v: &Value) -> bool {
    matches!(v, Value::Object(m) if m.len() == 1 && m.get("__delete__") == Some(&Value::Bool(true)))
}

fn strip_deletes(data: &Map<String, Value>) -> Map<String, Value> {
    let mut clean = Map::new();
    for (k, v) in data {
        if delete_marker(v) {
            continue;
        }
        clean.insert(k.clone(), match v {
            Value::Object(m) => Value::Object(strip_deletes(m)),
            other => other.clone(),
        });
    }
    clean
}

fn deep_merge(base: &Map<String, Value>, changes: &Map<String, Value>) -> Map<String, Value> {
    let mut merged = base.clone();
    for (k, v) in changes {
        if delete_marker(v) {
            merged.shift_remove(k);
        } else if let (Value::Object(change), Some(Value::Object(existing))) = (v, merged.get(k)) {
            let next = deep_merge(existing, change);
            merged.insert(k.clone(), Value::Object(next));
        } else {
            merged.insert(k.clone(), match v {
                Value::Object(m) => Value::Object(strip_deletes(m)),
                other => other.clone(),
            });
        }
    }
    merged
}

/// A write's fields come inline, or from a local JSON file it named.
fn write_data(write: &Map<String, Value>) -> Option<Map<String, Value>> {
    if let Some(Value::Object(d)) = write.get("data") {
        return Some(d.clone());
    }
    let Some(Value::String(path)) = write.get("file_path") else { return None };
    if !std::path::Path::new(path).is_file() {
        return None;
    }
    match serde_json::from_str::<Value>(&std::fs::read_to_string(path).ok()?).ok()? {
        Value::Object(d) => Some(d),
        _ => None,
    }
}

fn stamp(mut data: Map<String, Value>, ts: Option<&Value>) -> Map<String, Value> {
    if let Some(ts) = ts {
        if truthy(data.get("updatedAt")).is_none() {
            data.insert("updatedAt".into(), ts.clone());
        }
    }
    data
}

fn str_replace(doc: Option<&Value>, write: &Map<String, Value>) -> Result<Option<Map<String, Value>>, Unported> {
    let Some(Value::Object(doc)) = doc else { return Ok(None) };
    let (Some(Value::String(field)), Some(Value::String(old))) = (write.get("field"), write.get("old_str")) else { return Ok(None) };
    let Some(Value::String(current)) = doc.get(field) else { return Ok(None) };
    if old.is_empty() {
        return Ok(None);
    }
    let count = current.matches(old.as_str()).count();
    let all = write.get("replace_all") == Some(&Value::Bool(true));
    if count == 0 || (count > 1 && !all) {
        return Ok(None);
    }
    let new = scalar(write.get("new_str").unwrap_or(&Value::Null))?;
    let value = if all { current.replace(old.as_str(), &new) } else { current.replacen(old.as_str(), &new, 1) };
    let mut out = doc.clone();
    out.insert(field.clone(), json!(value));
    Ok(Some(out))
}

fn failed(response: Option<&Value>) -> bool {
    let pattern = Regex::new(r#"(?i)(?-u:\A\s*(<tool_use_error>|error\b)|nothing (is |was )?written|\brefused\b|\bconflict\b)"#).unwrap();
    match response {
        Some(Value::Object(m)) => {
            if m.get("is_error") == Some(&Value::Bool(true)) || m.get("isError") == Some(&Value::Bool(true)) {
                return true;
            }
            if truthy(m.get("error")).is_some() {
                return true;
            }
            failed(truthy(m.get("content")).or_else(|| truthy(m.get("text"))).or_else(|| truthy(m.get("result"))))
        }
        Some(Value::Array(parts)) => parts.iter().any(|p| failed(Some(p))),
        Some(Value::String(text)) => pattern.is_match(&Regex::new(r#""[^"]*""#).unwrap().replace_all(text, "\"\"")),
        _ => false,
    }
}

impl Mirror {
    fn fold_line(&mut self, line: &[u8]) -> Result<(), Unported> {
        let Ok(Value::Object(entry)) = serde_json::from_slice::<Value>(line) else { return Ok(()) };
        let payload = match entry.get("payload") {
            Some(Value::Object(p)) => p.clone(),
            _ => entry.clone(),
        };
        let ts = truthy(entry.get("ts")).or_else(|| truthy(payload.get("ts"))).cloned();
        self.fold(&payload, ts.as_ref())
    }

    fn fold(&mut self, payload: &Map<String, Value>, ts: Option<&Value>) -> Result<(), Unported> {
        let Some(Value::String(tool)) = payload.get("tool_name") else { return Ok(()) };
        if !tool.ends_with("ArtifactData") {
            return Ok(());
        }
        let Some(Value::Object(input)) = payload.get("tool_input") else { return Ok(()) };
        let action = match input.get("action") {
            Some(Value::String(a)) if WRITE_ACTIONS.contains(&a.as_str()) => a.clone(),
            _ => return Ok(()),
        };
        if failed(payload.get("tool_response")) {
            return Ok(());
        }
        let board = board_key(input.get("url").unwrap_or(&Value::Null))?;
        if board.is_empty() {
            return Ok(());
        }
        let writes: Vec<Map<String, Value>> = if action == "batch" {
            match input.get("writes") {
                Some(Value::Array(list)) => list.iter().filter_map(|w| w.as_object().cloned()).collect(),
                _ => vec![],
            }
        } else {
            let mut w = input.clone();
            w.insert("op".into(), json!(action));
            vec![w]
        };
        for write in &writes {
            self.apply(&board, write, ts)?;
        }
        Ok(())
    }

    fn apply(&mut self, board: &str, write: &Map<String, Value>, ts: Option<&Value>) -> Result<(), Unported> {
        let (Some(Value::String(collection)), Some(Value::String(doc_id))) = (write.get("collection"), write.get("doc_id")) else { return Ok(()) };
        let boards = self.boards.entry(board.to_string()).or_insert_with(|| json!({}));
        let Some(collections) = boards.as_object_mut() else { return Err(Unported) };
        let docs = collections.entry(collection.clone()).or_insert_with(|| json!({}));
        let Some(docs) = docs.as_object_mut() else { return Err(Unported) };
        match write.get("op").and_then(Value::as_str) {
            Some("delete") => {
                docs.shift_remove(doc_id);
            }
            Some("set") => {
                if let Some(data) = write_data(write) {
                    docs.insert(doc_id.clone(), Value::Object(stamp(strip_deletes(&data), ts)));
                }
            }
            Some("update") => {
                if let Some(data) = write_data(write) {
                    let base = match docs.get(doc_id) {
                        Some(Value::Object(b)) => b.clone(),
                        None => Map::new(),
                        Some(_) => return Err(Unported),
                    };
                    docs.insert(doc_id.clone(), Value::Object(stamp(deep_merge(&base, &data), ts)));
                }
            }
            Some("str_replace") => {
                if let Some(replaced) = str_replace(docs.get(doc_id), write)? {
                    docs.insert(doc_id.clone(), Value::Object(replaced));
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Folds whatever was appended since the last call.
    fn refresh(&mut self, path: &std::path::Path) -> Result<(), Unported> {
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if size < self.offset {
            *self = Mirror::default();
        }
        if size > self.offset {
            let Ok(mut f) = std::fs::File::open(path) else { return Ok(()) };
            let _ = f.seek(SeekFrom::Start(self.offset));
            let mut chunk = vec![];
            let _ = f.read_to_end(&mut chunk);
            let Some(complete) = chunk.iter().rposition(|b| *b == b'\n') else { return Ok(()) };
            for line in chunk[..=complete].split_inclusive(|b| *b == b'\n') {
                self.fold_line(line)?;
            }
            self.offset += complete as u64 + 1;
        }
        Ok(())
    }
}

/// The board's task rows, newest batch first and in plan order within a batch.
pub fn rows(url: &Value) -> Result<Vec<Value>, Unported> {
    let path = writes_file();
    let key = board_key(url)?;
    let docs: Docs = {
        let mut all = SHARED.lock().unwrap_or_else(|p| p.into_inner());
        let mirror = all.get_or_insert_with(HashMap::new).entry(path.to_string_lossy().into_owned()).or_default();
        mirror.refresh(&path)?;
        mirror.boards.get(&key).and_then(|b| b.get(ROW_COLLECTION)).and_then(Value::as_object).cloned().unwrap_or_default()
    };
    let mut rows: Vec<Map<String, Value>> = vec![];
    for (doc_id, data) in &docs {
        let Value::Object(data) = data else { return Err(Unported) };
        let mut row = Map::new();
        row.insert("doc_id".into(), json!(doc_id));
        for f in ROW_FIELDS {
            row.insert(f.into(), data.get(f).cloned().unwrap_or(Value::Null));
        }
        rows.push(row);
    }
    let mut batches: Vec<String> = vec![];
    for r in &rows {
        let b = scalar(&r["batch"])?;
        if !b.is_empty() && !batches.contains(&b) {
            batches.push(b);
        }
    }
    batches.sort();
    batches.reverse();
    let mut keyed: Vec<((usize, u8, f64, String), Map<String, Value>)> = vec![];
    for r in rows {
        let batch = scalar(&r["batch"])?;
        let order = r["order"].as_f64();
        let key = (batches.iter().position(|b| *b == batch).unwrap_or(batches.len()), u8::from(order.is_none()), order.unwrap_or(0.0), r["doc_id"].as_str().unwrap_or("").to_string());
        keyed.push((key, r));
    }
    keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    Ok(keyed.into_iter().map(|(_, r)| Value::Object(r)).collect())
}

/// Rows of an item's board (via its `board:` record), or `[]` without one.
pub fn rows_for(item: &WorkItem) -> Result<Vec<Value>, Unported> {
    match item.board() {
        Some(url) => rows(&json!(url)),
        None => Ok(vec![]),
    }
}

/// Answers a BLOCKED row by writing `<answers_dir>/<doc_id>.md`, the file the blocked sub-agent polls for.
pub fn answer(item: &WorkItem, doc_id: &str, text: &str) -> Result<std::path::PathBuf, Result<String, Unported>> {
    let Some(board) = item.board() else { return Err(Ok(format!("'{}' has no status board", item.slug))) };
    let Some(dir) = item.board_answers_dir() else { return Err(Ok(format!("'{}' has no answers directory (qm board {} URL --answers-dir DIR)", item.slug, item.slug))) };
    let valid = !doc_id.is_empty() && doc_id.chars().count() <= 200 && doc_id != "." && doc_id != ".." && doc_id.chars().all(|c| c.is_ascii_alphanumeric() || "_-.~:@+".contains(c));
    if !valid {
        return Err(Ok(format!("invalid row id '{doc_id}'")));
    }
    if strip(text).is_empty() {
        return Err(Ok("an empty answer".into()));
    }
    let all = rows(&json!(board)).map_err(Err)?;
    let Some(current) = all.iter().find(|r| r["doc_id"] == doc_id) else { return Err(Ok(format!("no row '{doc_id}' on the board"))) };
    let status = match &current["status"] {
        v if truthy(Some(v)).is_none() => "unset".to_string(),
        v => scalar(v).map_err(Err)?,
    };
    if !scalar(&current["status"]).map_err(Err)?.eq_ignore_ascii_case("blocked") {
        return Err(Ok(format!("row '{doc_id}' is {status}, not blocked")));
    }
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).map_err(|_| Err(Unported))?;
    let target = dir.join(format!("{doc_id}.md"));
    let temp = dir.join(format!(".{doc_id}.md.{}.tmp", std::process::id()));
    let body = if text.ends_with('\n') { text.to_string() } else { format!("{text}\n") };
    std::fs::write(&temp, body).map_err(|_| Err(Unported))?;
    std::fs::rename(&temp, &target).map_err(|_| Err(Unported))?;
    Ok(target)
}
