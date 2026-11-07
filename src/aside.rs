//! A one-off question about an item, asked beside its chat (`Aside`): its own tmux session
//! `<slug>--aside` under `QM_ASIDE=<slug>`, never linked to the item. Closing kills it; promoting
//! turns its last answer into a note.

use serde_json::Value;

use crate::chats_commands::aside_name;
use crate::launcher::{chat_command, resume_cwd};
use crate::session_log::{Session, SessionLog};
use crate::tmux::Tmux;
use crate::work_item::WorkItem;

const MODELS: [&str; 3] = ["sonnet", "opus", "haiku"];

pub enum Halt {
    Refuse(String),
    Unported,
}

fn refuse<T>(m: impl Into<String>) -> Result<T, Halt> {
    Err(Halt::Refuse(m.into()))
}

fn aside_env(item: &WorkItem) -> Vec<(String, String)> {
    vec![("QM_ASIDE".into(), item.slug.clone()), ("QM_ITEM".into(), String::new())]
}

fn start(item: &WorkItem, cwd: &str, argv: &[String]) -> Result<bool, Halt> {
    Tmux::new().open(&aside_name(&item.slug), cwd, argv, &aside_env(item)).or_else(|e| refuse(e))
}

/// `Aside#open`: true when it started one, false when one already runs.
pub fn open(item: &WorkItem, model: &str) -> Result<bool, Halt> {
    let model = if model.is_empty() { "sonnet" } else { model };
    if !MODELS.contains(&model) {
        let shown = crate::ruby_text::inspect_ascii(model).ok_or(Halt::Unported)?;
        return refuse(format!("unknown model {shown} (sonnet, opus, haiku)"));
    }
    let command = chat_command().ok_or(Halt::Unported)?;
    let argv: Vec<String> = command.into_iter().chain(["--model".to_string(), model.to_string()]).collect();
    start(item, &crate::branch_diff::start_cwd(item), &argv)
}

pub fn close(item: &WorkItem) -> bool {
    Tmux::new().kill(&aside_name(&item.slug))
}

fn asides_of(item: &WorkItem) -> Vec<Session> {
    SessionLog::read().sessions().into_iter().filter(|s| s.aside_for().as_deref() == Some(item.slug.as_str())).collect()
}

/// `Aside#history`: the item's asides, newest first, for the drawer's list of earlier ones.
pub fn history(item: &WorkItem) -> Result<Vec<Value>, Halt> {
    let promoted: Vec<String> = item.notes().into_iter().map(|n| n.text).collect();
    let mut sessions = asides_of(item);
    sessions.sort_by_key(|s| s.started_ts());
    sessions.reverse();
    let mut rows = vec![];
    for s in sessions {
        let transcript = s.transcript();
        let answer = match &transcript {
            Some(t) => last_answer_in(t)?,
            None => None,
        };
        let model = match &transcript {
            Some(t) => model_in(t)?,
            None => None,
        };
        let is_promoted = answer.as_ref().is_some_and(|a| promoted.contains(&note_text(a)));
        rows.push(serde_json::json!({"id": s.id, "tier": s.tier(), "at": s.started_ts_opt(), "prompt": s.first_prompt(), "model": model, "answer": answer, "promoted": is_promoted}));
    }
    Ok(rows)
}

/// `Aside#reopen`: an earlier aside resumed under its marker, on the model it ran on.
pub fn reopen(item: &WorkItem, session_id: &str) -> Result<(), Halt> {
    if Tmux::new().live(&aside_name(&item.slug)) {
        return refuse(format!("an aside for {} is already open — close it first", item.slug));
    }
    let short: String = session_id.chars().take(8).collect();
    let Some(session) = asides_of(item).into_iter().find(|s| s.id == session_id) else {
        return refuse(format!("{short} is not an aside about {}", item.slug));
    };
    let cwd = if session.tier() == "resumable" { resume_cwd(&session) } else { None };
    let Some(cwd) = cwd else { return refuse(format!("{short} has no surviving transcript to resume")) };
    let model = match session.transcript() {
        Some(t) => model_in(&t)?,
        None => None,
    };
    let command = chat_command().ok_or(Halt::Unported)?;
    let mut argv: Vec<String> = command;
    if let Some(m) = model {
        argv.extend(["--model".to_string(), m]);
    }
    argv.extend(["--resume".to_string(), session.id.clone()]);
    start(item, &cwd, &argv).map(|_| ())
}

/// `Aside.note_text`.
fn note_text(answer: &str) -> String {
    format!("aside: {}", answer.split([' ', '\t', '\n', '\u{b}', '\u{c}', '\r']).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" "))
}

/// `Aside#promote`: the aside's last answer becomes a note on the item; returns the note text.
pub fn promote(item: &mut WorkItem, session_id: Option<&str>) -> Result<String, Halt> {
    let sessions = asides_of(item);
    let answer = match session_id {
        Some(id) => match sessions.iter().find(|s| s.id == id).and_then(|s| s.transcript()) {
            Some(t) => last_answer_in(&t)?,
            None => None,
        },
        None => {
            // only the aside running now counts
            let mut newest: Option<&Session> = None;
            for s in sessions.iter().filter(|s| s.live) {
                if newest.is_none_or(|n| s.started_ts() > n.started_ts()) {
                    newest = Some(s);
                }
            }
            match newest.and_then(|s| s.transcript()) {
                Some(t) => last_answer_in(&t)?,
                None => None,
            }
        }
    };
    let Some(answer) = answer else { return refuse(format!("the aside for {} has no answer to promote yet", item.slug)) };
    let text = note_text(&answer);
    item.add_note(&text, crate::note::DEFAULT_NOTE_KIND).map_err(|e| if e.class == crate::error::UNEMITTABLE { Halt::Unported } else { Halt::Refuse(e.message) })?;
    Ok(text)
}

/// The transcript's lines as parsed JSON, `None` for the ones Ruby skips; `Err` when the file isn't
/// text this port reads as Ruby does.
fn entries(path: &std::path::Path) -> Result<Vec<Option<Value>>, Halt> {
    let text = String::from_utf8(std::fs::read(path).map_err(|_| Halt::Unported)?).map_err(|_| Halt::Unported)?;
    Ok(text.split_inclusive('\n').map(|l| serde_json::from_str::<Value>(l.trim_end_matches('\n')).ok()).collect())
}

/// The text blocks of an assistant entry (`Aside.assistant_texts`).
fn assistant_texts(entry: &Value) -> Result<Vec<String>, Halt> {
    let message = match entry.get("message") {
        None | Some(Value::Null) => return Ok(vec![]),
        Some(m @ Value::Object(_)) => m,
        Some(_) => return Err(Halt::Unported),
    };
    match message.get("content") {
        Some(Value::String(s)) if !crate::ruby_text::strip(s).is_empty() => Ok(vec![s.clone()]),
        Some(Value::Array(blocks)) => {
            let mut out = vec![];
            for b in blocks {
                if b.get("type").and_then(Value::as_str) == Some("text") {
                    match b.get("text") {
                        Some(Value::String(t)) if !crate::ruby_text::strip(t).is_empty() => out.push(t.clone()),
                        None | Some(Value::Null) => {}
                        Some(Value::String(_)) => {}
                        Some(_) => return Err(Halt::Unported),
                    }
                }
            }
            Ok(out)
        }
        _ => Ok(vec![]),
    }
}

/// `Aside.last_answer_in`: the text of the last assistant message that has any, its blocks joined.
fn last_answer_in(path: &std::path::Path) -> Result<Option<String>, Halt> {
    if !path.exists() {
        return Ok(None);
    }
    let mut message_id: Option<Value> = None;
    let mut parts: Vec<String> = vec![];
    for entry in entries(path)?.into_iter().flatten() {
        if !entry.is_object() || entry.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let texts = assistant_texts(&entry)?;
        if texts.is_empty() {
            continue;
        }
        let id = entry.get("message").and_then(|m| m.get("id")).cloned().filter(|v| !v.is_null() && *v != Value::Bool(false));
        if !(id.is_some() && id == message_id) {
            parts.clear();
        }
        message_id = id;
        parts.extend(texts);
    }
    let answer = crate::ruby_text::strip(&parts.join("\n\n")).to_string();
    Ok((!answer.is_empty()).then_some(answer))
}

/// `Aside.model_in`: the model the transcript's last assistant message ran on.
fn model_in(path: &std::path::Path) -> Result<Option<String>, Halt> {
    if !path.exists() {
        return Ok(None);
    }
    let text = String::from_utf8(std::fs::read(path).map_err(|_| Halt::Unported)?).map_err(|_| Halt::Unported)?;
    let mut model = None;
    for line in text.split_inclusive('\n').filter(|l| l.contains("\"model\"")) {
        let Ok(entry) = serde_json::from_str::<Value>(line.trim_end_matches('\n')) else { continue };
        if entry.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        match entry.get("message") {
            Some(Value::Object(m)) => {
                if let Some(Value::String(found)) = m.get("model") {
                    if !found.is_empty() && found != "<synthetic>" {
                        model = Some(found.clone());
                    }
                }
            }
            None | Some(Value::Null) => {}
            Some(_) => return Err(Halt::Unported),
        }
    }
    Ok(model)
}
