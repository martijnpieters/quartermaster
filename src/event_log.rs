//! events.jsonl, appended by hooks/record-event.sh on every prompt, tool call, permission request
//! and turn end. It grows forever, so it is never re-read whole: each session's turn is folded into
//! a small summary, persisted with the byte offset it covers (events-fold.json), and every read
//! folds only the bytes appended since.
//!
//! A turn is {since, state, prompt_ts, stop_ts, before_block, tool, last_ts}: state is
//! working | blocked | your_turn, `since` is when it entered that state.

use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::data_root;

const FOLD_VERSION: i64 = 1;
/// Sessions quiet this long drop out of the fold (they read as no turn, like a fresh chat).
const FORGET_AFTER_SECS: i64 = 30 * 24 * 3600;

pub type Turn = Map<String, Value>;

fn path() -> PathBuf {
    data_root::root().join("events.jsonl")
}

fn fold_path() -> PathBuf {
    data_root::root().join("events-fold.json")
}

fn empty_fold(inode: Option<u64>) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("v".into(), FOLD_VERSION.into());
    m.insert("inode".into(), inode.map(Value::from).unwrap_or(Value::Null));
    m.insert("offset".into(), 0.into());
    m.insert("sessions".into(), Value::Object(Map::new()));
    m
}

fn load_fold() -> Map<String, Value> {
    match std::fs::read_to_string(fold_path()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        Some(Value::Object(m)) if m.get("v").and_then(Value::as_i64) == Some(FOLD_VERSION) && m.get("sessions").is_some_and(Value::is_object) => m,
        _ => empty_fold(None),
    }
}

/// `value.to_s.empty?` for a JSON value.
fn to_s_empty(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s.is_empty(),
        Some(_) => false,
    }
}

fn enter(turn: &mut Turn, state: &str, ts: &Value, force: bool) {
    if force || turn.get("state").and_then(Value::as_str) != Some(state) {
        turn.insert("since".into(), ts.clone());
    }
    turn.insert("state".into(), state.into());
}

/// One event folded into its session's turn. Only the order of events matters: UserPromptSubmit and
/// PostToolUse mean working, PermissionRequest means blocked, Stop ends every turn. The *Failure
/// variants stand in for their success events. A sub-agent's tool call only resolves a permission
/// it was blocked on — background sub-agents keep running after the main turn's Stop.
pub fn apply(sessions: &mut Map<String, Value>, event: &Value) {
    let Some(id) = event.get("session_id").and_then(Value::as_str).filter(|s| !s.is_empty()) else { return };
    let ts = event.get("ts").cloned().unwrap_or(Value::Null);
    let tool = if to_s_empty(event.get("tool")) { Value::Null } else { event["tool"].clone() };
    let agent = !to_s_empty(event.get("agent"));
    let slot = sessions.entry(id.to_string()).or_insert_with(|| Value::Object(Map::new()));
    let Some(turn) = slot.as_object_mut() else { return };
    match event.get("event").and_then(Value::as_str) {
        Some("UserPromptSubmit") => {
            enter(turn, "working", &ts, true);
            turn.insert("prompt_ts".into(), ts.clone());
        }
        Some("PostToolUse") | Some("PostToolUseFailure") => {
            if agent {
                if turn.get("state").and_then(Value::as_str) != Some("blocked") {
                    return;
                }
                let back = turn.get("before_block").and_then(Value::as_str).unwrap_or("working").to_string();
                enter(turn, &back, &ts, false);
            } else {
                enter(turn, "working", &ts, false);
            }
        }
        Some("PermissionRequest") => {
            if turn.get("state").and_then(Value::as_str) != Some("blocked") {
                turn.insert("before_block".into(), turn.get("state").cloned().unwrap_or(Value::Null));
            }
            enter(turn, "blocked", &ts, false);
        }
        Some("Stop") | Some("StopFailure") => {
            enter(turn, "your_turn", &ts, false);
            turn.insert("stop_ts".into(), ts.clone());
        }
        _ => {
            if turn.is_empty() {
                sessions.remove(id);
            }
            return;
        }
    }
    turn.insert("tool".into(), tool);
    turn.insert("last_ts".into(), ts);
}

fn iso_utc(secs: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0).unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn save_fold(fold: &mut Map<String, Value>) {
    let cutoff = iso_utc(chrono::Utc::now().timestamp() - FORGET_AFTER_SECS);
    if let Some(Value::Object(sessions)) = fold.get_mut("sessions") {
        sessions.retain(|_, turn| {
            let last = match turn.get("last_ts") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Null) | None => String::new(),
                Some(other) => other.to_string(),
            };
            last >= cutoff
        });
    }
    let path = fold_path();
    let tmp = PathBuf::from(format!("{}.{}.tmp", path.display(), std::process::id()));
    let result = std::fs::write(&tmp, Value::Object(fold.clone()).to_string()).and_then(|_| std::fs::rename(&tmp, &path));
    if let Err(e) = result {
        data_root::log_error("event-fold", "IOError", &e.to_string());
    }
}

/// session_id → turn, reading only what was appended since the persisted fold.
pub fn turns() -> Map<String, Value> {
    let mut fold = load_fold();
    let Ok(meta) = std::fs::metadata(path()) else { return Map::new() };
    let size = meta.len();
    let offset = fold.get("offset").and_then(Value::as_u64).unwrap_or(0);
    if fold.get("inode").and_then(Value::as_u64) != Some(meta.ino()) || size < offset {
        fold = empty_fold(Some(meta.ino()));
    }
    let offset = fold.get("offset").and_then(Value::as_u64).unwrap_or(0);
    if size > offset {
        fold_new_bytes(&mut fold, offset, size);
    }
    fold.get("sessions").and_then(Value::as_object).cloned().unwrap_or_default()
}

fn fold_new_bytes(fold: &mut Map<String, Value>, offset: u64, size: u64) {
    let Ok(mut file) = std::fs::File::open(path()) else { return };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return;
    }
    let mut chunk = vec![];
    if file.take(size - offset).read_to_end(&mut chunk).is_err() {
        return;
    }
    // a line still being appended has no newline yet: leave it for next time
    let Some(last) = chunk.iter().rposition(|b| *b == b'\n') else { return };
    let complete = &chunk[..=last];
    let mut sessions = fold.get("sessions").and_then(Value::as_object).cloned().unwrap_or_default();
    for line in complete.split_inclusive(|b| *b == b'\n') {
        let Ok(text) = std::str::from_utf8(line) else { continue };
        if let Ok(event) = serde_json::from_str::<Value>(text) {
            apply(&mut sessions, &event);
        }
    }
    fold.insert("sessions".into(), Value::Object(sessions));
    fold.insert("offset".into(), Value::from(offset + complete.len() as u64));
    save_fold(fold);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(events: &[Value]) -> Map<String, Value> {
        let mut s = Map::new();
        for e in events {
            apply(&mut s, e);
        }
        s
    }

    fn ev(name: &str, ts: &str) -> Value {
        json!({"session_id": "s", "event": name, "ts": ts})
    }

    #[test]
    fn a_prompt_works_a_permission_blocks_and_a_stop_hands_over() {
        let s = run(&[ev("UserPromptSubmit", "t1"), ev("PermissionRequest", "t2"), ev("PostToolUse", "t3"), ev("Stop", "t4")]);
        let turn = &s["s"];
        assert_eq!((turn["state"].as_str(), turn["since"].as_str(), turn["stop_ts"].as_str(), turn["prompt_ts"].as_str()), (Some("your_turn"), Some("t4"), Some("t4"), Some("t1")));
    }

    #[test]
    fn a_sub_agents_tool_call_only_resolves_a_block() {
        let mut agent = ev("PostToolUse", "t3");
        agent["agent"] = json!("sub");
        let blocked = run(&[ev("UserPromptSubmit", "t1"), ev("PermissionRequest", "t2"), agent.clone()]);
        assert_eq!(blocked["s"]["state"], "working");
        let stopped = run(&[ev("UserPromptSubmit", "t1"), ev("Stop", "t2"), agent]);
        assert_eq!(stopped["s"]["state"], "your_turn");
    }

    #[test]
    fn a_block_after_the_stop_returns_to_the_users_turn() {
        let mut agent = ev("PostToolUse", "t4");
        agent["agent"] = json!("sub");
        let s = run(&[ev("UserPromptSubmit", "t1"), ev("Stop", "t2"), ev("PermissionRequest", "t3"), agent]);
        assert_eq!((s["s"]["state"].as_str(), s["s"]["since"].as_str(), s["s"]["before_block"].as_str()), (Some("your_turn"), Some("t4"), Some("your_turn")));
    }

    #[test]
    fn unknown_events_leave_no_empty_turn_but_an_ignored_sub_agent_call_does() {
        assert!(run(&[ev("Notification", "t")]).is_empty());
        let mut agent = ev("PostToolUse", "t");
        agent["agent"] = json!("sub");
        assert_eq!(run(&[agent])["s"], json!({}));
    }

    #[test]
    fn key_order_follows_insertion() {
        let s = run(&[ev("UserPromptSubmit", "t1"), ev("PermissionRequest", "t2")]);
        let keys: Vec<&String> = s["s"].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["since", "state", "prompt_ts", "tool", "last_ts", "before_block"]);
    }
}
