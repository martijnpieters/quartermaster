//! Claude Code transcripts: where a session's file is, and what the user first typed in it.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::data_root;

/// The recorded transcript path when it exists, else the session's file under any project
/// directory (a `--worktree --resume` reports the wrong one).
pub fn locate(session_id: Option<&str>, recorded: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = recorded {
        if Path::new(p).exists() {
            return Some(p.into());
        }
    }
    let id = session_id?;
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(data_root::transcripts_dir()).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    dirs.sort();
    dirs.into_iter().map(|d| d.join(format!("{id}.jsonl"))).find(|p| p.exists())
}

fn truthy(v: Option<&Value>) -> bool {
    !matches!(v, None | Some(Value::Null) | Some(Value::Bool(false)))
}

/// What a part of an array `content` contributes when joined.
fn joined_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The human-typed text of a user entry: not tool results, meta entries or injected content.
/// `Err(())` where Ruby's `dig` would raise and abandon the whole search.
pub(crate) fn prompt_text(entry: &serde_json::Map<String, Value>) -> Result<Option<String>, ()> {
    if truthy(entry.get("isMeta")) || truthy(entry.get("toolUseResult")) {
        return Ok(None);
    }
    let content = match entry.get("message") {
        None | Some(Value::Null) => None,
        Some(Value::Object(m)) => m.get("content"),
        Some(_) => return Err(()),
    };
    let text = match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|c| match c {
                Value::Object(o) if o.get("type") == Some(&Value::String("text".into())) => match o.get("text") {
                    None | Some(Value::Null) | Some(Value::Bool(false)) => None,
                    Some(t) => Some(joined_text(t)),
                },
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    };
    let text = crate::ruby_text::strip(&text);
    Ok(if text.is_empty() || text.starts_with('<') { None } else { Some(text.to_string()) })
}

/// The first prompt the user typed, or None.
pub fn first_prompt(path: Option<&Path>) -> Option<String> {
    let text = std::fs::read_to_string(path?).ok()?;
    for line in text.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
        let obj = match entry {
            Value::Object(o) => o,
            Value::String(_) => continue,
            _ => return None,
        };
        if obj.get("type") != Some(&Value::String("user".into())) {
            continue;
        }
        match prompt_text(&obj) {
            Ok(Some(t)) => return Some(t),
            Ok(None) => {}
            Err(()) => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(lines: &[&str]) -> Option<String> {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        std::fs::write(&p, lines.join("\n")).unwrap();
        first_prompt(Some(&p))
    }

    #[test]
    fn skips_meta_tool_results_injected_text_and_broken_lines() {
        let got = first(&[
            r#"{"type":"user","isMeta":true,"message":{"content":"meta"}}"#,
            r#"{"type":"user","toolUseResult":"x","message":{"content":"r"}}"#,
            r#"{"type":"user","message":{"content":"<command-name>/x</command-name>"}}"#,
            "not json",
            r#"{"type":"user","message":{"content":"real"}}"#,
        ]);
        assert_eq!(got.as_deref(), Some("real"));
    }

    #[test]
    fn array_parts_are_joined_with_a_space() {
        let got = first(&[r#"{"type":"user","message":{"content":[{"type":"text","text":"a "},{"type":"image"},{"type":"text","text":"b"}]}}"#]);
        assert_eq!(got.as_deref(), Some("a  b"));
    }

    #[test]
    fn a_message_that_is_not_an_object_ends_the_search_like_rubys_dig_raising() {
        assert_eq!(first(&[r#"{"type":"user","message":"text"}"#, r#"{"type":"user","message":{"content":"later"}}"#]), None);
        assert_eq!(first(&["5", r#"{"type":"user","message":{"content":"later"}}"#]), None);
        assert_eq!(first(&["\"a string\"", r#"{"type":"user","message":{"content":"later"}}"#]).as_deref(), Some("later"));
    }

    #[test]
    fn a_missing_file_has_no_prompt() {
        assert_eq!(first_prompt(None), None);
        assert_eq!(first_prompt(Some(Path::new("/nonexistent/x"))), None);
    }
}
