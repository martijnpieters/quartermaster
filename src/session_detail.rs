//! What the GUI's session sheet shows for one session (`Quartermaster.session_detail`): the capture
//! metadata plus, while the transcript survives, the parsed turns — each turn's tool tallies, touched
//! files and the reply's opening text, clipped so a megabyte transcript never crosses the wire.

use serde_json::{json, Map, Value};

use crate::cli_format::short_path;
use crate::session_log::{Session, SessionLog};

const TURN_PROMPT_CHARS: usize = 700;
const TURN_REPLY_CHARS: usize = 360;
const TURN_FILES_SHOWN: usize = 6;

pub(crate) fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() > limit {
        format!("{}…", crate::ruby_text::rstrip(&text.chars().take(limit).collect::<String>()))
    } else {
        text.to_string()
    }
}

struct Turn {
    ts: Value,
    prompt: String,
    tools: Vec<(String, i64)>,
    files: Vec<String>,
    reply: Option<String>,
}

/// `session_turns`: `Ok(None)` where Ruby's `rescue StandardError` makes the whole parse nil, `Err` for
/// a value this port does not read as Ruby does.
fn session_turns(path: &std::path::Path) -> Result<Option<Value>, ()> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).map_err(|_| ())?;
    let mut turns: Vec<Turn> = vec![];
    let (mut title, mut branch, mut model) = (Value::Null, Value::Null, Value::Null);
    for raw in bytes.split_inclusive(|b| *b == b'\n') {
        let Ok(line) = std::str::from_utf8(raw) else { continue };
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
        let entry = match entry {
            Value::Object(o) => o,
            Value::String(_) => continue, // "x"["isSidechain"] is nil
            _ => return Ok(None),          // anything else raises, and the whole parse is nil
        };
        if !matches!(entry.get("isSidechain"), None | Some(Value::Null) | Some(Value::Bool(false))) {
            continue;
        }
        match entry.get("gitBranch") {
            None | Some(Value::Null) | Some(Value::Bool(false)) => {}
            Some(Value::String(b)) => {
                if !b.is_empty() {
                    branch = Value::String(b.clone());
                }
            }
            Some(_) => return Err(()),
        }
        match entry.get("type").and_then(Value::as_str) {
            Some("ai-title") => title = entry.get("aiTitle").cloned().unwrap_or(Value::Null),
            Some("user") => {
                let Ok(text) = crate::transcripts::prompt_text(&entry) else { return Ok(None) };
                let Some(text) = text else { continue };
                turns.push(Turn { ts: entry.get("timestamp").cloned().unwrap_or(Value::Null), prompt: clip(&text, TURN_PROMPT_CHARS), tools: vec![], files: vec![], reply: None });
            }
            Some("assistant") => {
                let Some(turn) = turns.last_mut() else { continue };
                let message: Option<&Map<String, Value>> = match entry.get("message") {
                    None | Some(Value::Null) => None,
                    Some(Value::Object(m)) => Some(m),
                    Some(_) => return Ok(None),
                };
                if let Some(found) = message.and_then(|m| m.get("model")) {
                    if !matches!(found, Value::Null | Value::Bool(false)) {
                        model = found.clone();
                    }
                }
                let blocks: Vec<&Value> = match message.and_then(|m| m.get("content")) {
                    Some(Value::Array(items)) => items.iter().collect(),
                    _ => vec![],
                };
                for block in blocks {
                    let Value::Object(b) = block else { continue };
                    match b.get("type").and_then(Value::as_str) {
                        Some("tool_use") => {
                            let name = match b.get("name") {
                                None | Some(Value::Null) | Some(Value::Bool(false)) => "?".to_string(),
                                Some(Value::String(n)) => n.clone(),
                                Some(_) => return Err(()),
                            };
                            match turn.tools.iter_mut().find(|(n, _)| *n == name) {
                                Some((_, c)) => *c += 1,
                                None => turn.tools.push((name, 1)),
                            }
                            match b.get("input") {
                                None | Some(Value::Null) => {}
                                Some(Value::Object(input)) => {
                                    if let Some(Value::String(file)) = input.get("file_path") {
                                        turn.files.push(std::path::Path::new(file).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                                    }
                                }
                                Some(_) => return Ok(None), // `dig` into a non-hash raises
                            }
                        }
                        Some("text") => match b.get("text") {
                            None | Some(Value::Null) => turn.reply = None,
                            Some(Value::String(t)) => turn.reply = Some(t.clone()),
                            Some(_) => return Err(()),
                        },
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    let turns: Vec<Value> = turns
        .into_iter()
        .map(|t| {
            let mut files: Vec<String> = vec![];
            for f in t.files {
                if !files.contains(&f) {
                    files.push(f);
                }
            }
            files.truncate(TURN_FILES_SHOWN);
            let tools: Vec<Value> = t.tools.into_iter().map(|(name, count)| json!({"name": name, "count": count})).collect();
            let reply = t.reply.map(|r| clip(crate::ruby_text::strip(&r), TURN_REPLY_CHARS));
            json!({"ts": t.ts, "prompt": t.prompt, "tools": tools, "files": files, "reply": reply})
        })
        .collect();
    Ok(Some(json!({"turns": turns, "title": title, "branch": branch, "model": model})))
}

/// `Quartermaster.session_detail`; `None` for what only Ruby reads exactly.
pub fn session_detail(session_id: &str) -> Option<Value> {
    let session: Session = SessionLog::read().session_for(session_id).unwrap_or_else(|| Session::uncaptured(session_id));
    let parsed = match session.transcript() {
        Some(t) => session_turns(&t).ok()?,
        None => None,
    };
    let mut out = json!({
        "id": session.id, "tier": session.tier(),
        "when": session.started_ts_opt().map(|t| t.chars().take(16).collect::<String>()),
        "cwd": session.cwd().map(|c| short_path(Some(&c))), "prompt": session.first_prompt(),
        "last_ts": session.last_ts_opt()
    });
    let extra = parsed.unwrap_or_else(|| json!({"turns": null, "title": null, "branch": null, "model": null}));
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        out[k] = v;
    }
    Some(out)
}
