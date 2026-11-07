//! An item's earlier sessions as one continuous history above its live chat (`ChatHistory`,
//! `TranscriptHistory`): every session that isn't running, oldest first, with a divider wherever a
//! context rotation handed over to a fresh session; the page loads the timeline first, then a page of
//! entries at a time from the newest end. A pruned transcript falls back to its distilled snapshot.

use serde_json::{json, Map, Value};

use crate::cli_format::short_path;
use crate::session_detail::clip;
use crate::session_log::{Session, SessionLog};
use crate::work_item::WorkItem;

const PAGE_SIZE: usize = 60;
const TOOL_LINE_CHARS: usize = 160;
const TOOL_SUBJECT_KEYS: [&str; 10] = ["command", "file_path", "notebook_path", "pattern", "path", "url", "query", "skill", "description", "prompt"];

/// Why a history answer is not made here: Ruby's `ArgumentError` message, or what only Ruby reads exactly.
pub enum Stop {
    Refuse(String),
    Unported,
}

fn words(s: &str) -> String {
    s.split([' ', '\t', '\n', '\u{b}', '\u{c}', '\r']).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

fn nonblank(v: &Value) -> Option<&str> {
    v.as_str().filter(|s| !crate::ruby_text::strip(s).is_empty())
}

/// "Bash  git status": the tool's name and the one input that identifies the call, folded onto one line.
fn tool_line(block: &Map<String, Value>) -> Result<String, Stop> {
    let empty = Map::new();
    let input = block.get("input").and_then(Value::as_object).unwrap_or(&empty);
    let subject = TOOL_SUBJECT_KEYS.iter().find_map(|k| input.get(*k).and_then(nonblank)).or_else(|| input.values().find_map(nonblank)).map(|s| short_path(Some(&words(s))));
    let name = match block.get("name") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => "?".to_string(),
        Some(Value::String(n)) => n.clone(),
        Some(_) => return Err(Stop::Unported),
    };
    let line = match subject {
        Some(s) => format!("{name}  {s}"),
        None => name,
    };
    Ok(clip(&line, TOOL_LINE_CHARS))
}

struct Entries {
    list: Vec<Value>,
    message_id: Option<Value>,
}

impl Entries {
    fn push(&mut self, kind: &str, text: String, ts: Value) {
        if kind != "assistant" {
            self.message_id = None;
        }
        self.list.push(json!({"kind": kind, "text": text, "ts": ts}));
    }
}

/// `TranscriptHistory.entries`: what the user typed and Claude answered, in full, and every tool call as one line.
fn transcript_entries(path: &std::path::Path) -> Result<Option<Vec<Value>>, Stop> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).map_err(|_| Stop::Unported)?;
    let mut e = Entries { list: vec![], message_id: None };
    for raw in bytes.split_inclusive(|b| *b == b'\n') {
        let Ok(line) = std::str::from_utf8(raw) else { continue };
        let Ok(Value::Object(entry)) = serde_json::from_str::<Value>(line) else { continue };
        if !matches!(entry.get("isSidechain"), None | Some(Value::Null) | Some(Value::Bool(false))) {
            continue;
        }
        let ts = entry.get("timestamp").cloned().unwrap_or(Value::Null);
        match entry.get("type").and_then(Value::as_str) {
            Some("user") => {
                if !matches!(entry.get("isCompactSummary"), None | Some(Value::Null) | Some(Value::Bool(false))) {
                    continue;
                }
                let text = crate::transcripts::prompt_text(&entry).map_err(|_| Stop::Unported)?;
                let Some(text) = text else { continue };
                let kind = if text.starts_with(crate::transcript_vitals::INTERRUPT_MARK) { "event" } else { "user" };
                e.push(kind, text, ts);
            }
            Some("assistant") => {
                let message = match entry.get("message") {
                    None | Some(Value::Null) => None,
                    Some(Value::Object(m)) => Some(m),
                    Some(_) => return Err(Stop::Unported),
                };
                let id = message.and_then(|m| m.get("id")).cloned().filter(|v| !matches!(v, Value::Null | Value::Bool(false)));
                let blocks: Vec<Value> = match message.and_then(|m| m.get("content")) {
                    Some(Value::String(s)) => vec![json!({"type": "text", "text": s})],
                    Some(Value::Array(items)) => items.clone(),
                    _ => vec![],
                };
                for block in &blocks {
                    let Value::Object(b) = block else { continue };
                    match b.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            let text = match b.get("text") {
                                None | Some(Value::Null) => String::new(),
                                Some(Value::String(t)) => t.clone(),
                                Some(Value::Bool(v)) => v.to_string(),
                                Some(Value::Number(n)) if n.is_i64() || n.is_u64() => n.to_string(),
                                Some(_) => return Err(Stop::Unported),
                            };
                            let text = crate::ruby_text::strip(&text).to_string();
                            if text.is_empty() {
                                continue;
                            }
                            let joins = e.list.last().is_some_and(|l| l["kind"] == "assistant") && id.is_some() && id == e.message_id;
                            if joins {
                                let last = e.list.last_mut().unwrap();
                                last["text"] = json!(format!("{}\n\n{text}", last["text"].as_str().unwrap_or("")));
                            } else {
                                e.push("assistant", text, ts.clone());
                            }
                        }
                        Some("tool_use") => {
                            let line = tool_line(b)?;
                            e.push("tool", line, ts.clone());
                        }
                        _ => {}
                    }
                }
                e.message_id = id;
            }
            _ => {}
        }
    }
    Ok(Some(e.list))
}

fn earlier_sessions(item: &WorkItem) -> Result<Vec<Session>, Stop> {
    let all = SessionLog::read().item_sessions(item).map_err(|e| Stop::Refuse(e.message))?;
    Ok(all.into_iter().filter(|s| !s.live).collect())
}

fn moment(ts: &Value) -> Result<f64, Stop> {
    match ts {
        Value::String(s) => crate::clock::parse_f64(s).ok_or(Stop::Unported),
        Value::Null => Ok(0.0),
        _ => Err(Stop::Unported),
    }
}

fn to_i(v: Option<&serde_yaml::Value>) -> Result<i64, Stop> {
    match v {
        None | Some(serde_yaml::Value::Null) => Ok(0),
        Some(serde_yaml::Value::Number(n)) => n.as_i64().ok_or(Stop::Unported),
        Some(_) => Err(Stop::Unported),
    }
}

fn yaml_scalar(v: Option<&serde_yaml::Value>) -> Result<Value, Stop> {
    match v {
        None | Some(serde_yaml::Value::Null) => Ok(Value::Null),
        Some(serde_yaml::Value::String(s)) => Ok(json!(s)),
        Some(_) => Err(Stop::Unported),
    }
}

/// `ChatHistory#timeline`: session and divider segments, oldest first.
pub fn timeline(item: &WorkItem) -> Result<Value, Stop> {
    let mut segments: Vec<Value> = vec![];
    for s in earlier_sessions(item)? {
        let source = if s.transcript().is_some() { "transcript" } else if item.snapshot_for(&s.id) { "snapshot" } else { "none" };
        segments.push(json!({"type": "session", "id": s.id, "at": s.started_ts_opt(), "last": s.last_ts_opt(), "tier": s.tier(), "source": source}));
    }
    let notes = item.notes();
    for rotation in item.rotations() {
        let serde_yaml::Value::Mapping(m) = &rotation else { return Err(Stop::Unported) };
        let carried: Vec<Value> = match crate::yaml_scalar::get(m, "carried") {
            None | Some(serde_yaml::Value::Null) => vec![],
            Some(serde_yaml::Value::Sequence(items)) => items.iter().filter_map(|p| p.as_i64()).filter_map(|p| notes.iter().find(|n| n.position as i64 == p)).map(|n| json!({"stamp": n.stamp, "kind": n.kind, "text": n.text, "position": n.position})).collect(),
            Some(_) => return Err(Stop::Unported),
        };
        segments.push(json!({"type": "divider", "at": yaml_scalar(crate::yaml_scalar::get(m, "at"))?, "from": yaml_scalar(crate::yaml_scalar::get(m, "from"))?, "via": yaml_scalar(crate::yaml_scalar::get(m, "via"))?, "decisions": to_i(crate::yaml_scalar::get(m, "decisions"))?, "open": to_i(crate::yaml_scalar::get(m, "open"))?, "notes": carried}));
    }
    let mut keyed: Vec<(f64, usize, Value)> = vec![];
    for (i, seg) in segments.into_iter().enumerate() {
        keyed.push((moment(&seg["at"])?, i, seg));
    }
    keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.cmp(&b.1)));
    Ok(Value::Array(keyed.into_iter().map(|k| k.2).collect()))
}

/// `ChatHistory#page`: one page of a session's entries, ending just before `before` (the newest page when None).
pub fn page(item: &WorkItem, session_id: &str, before: Option<i64>) -> Result<Value, Stop> {
    let sessions = earlier_sessions(item)?;
    let Some(session) = sessions.iter().find(|s| s.id == session_id) else {
        return Err(Stop::Refuse(format!("{session_id} is not an earlier session of {}", item.slug)));
    };
    let entries: Vec<Value> = match session.transcript() {
        Some(path) => transcript_entries(&path)?.unwrap_or_default(),
        None => match item.snapshots().into_iter().find(|s| s["session"] == session.id.as_str()) {
            Some(s) => vec![json!({"kind": "snapshot", "text": s["text"], "ts": s["stamp"]})],
            None => vec![json!({"kind": "event", "text": "Transcript expired and was never distilled — nothing of this session survives.", "ts": session.last_ts_opt()})],
        },
    };
    let total = entries.len();
    let stop = before.map_or(total, |b| b.clamp(0, total as i64) as usize);
    let start = stop.saturating_sub(PAGE_SIZE);
    Ok(json!({"entries": entries[start..stop], "before": if start > 0 { json!(start) } else { Value::Null }, "total": total}))
}

/// `TranscriptHistory.tool_line` for the other modules; `None` for a block only Ruby prints exactly.
pub(crate) fn tool_line_of(block: &Map<String, Value>) -> Option<String> {
    tool_line(block).ok()
}
