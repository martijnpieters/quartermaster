//! How far along each sub-agent of an item's running Workflow runs is, as a cheap model reads it from
//! their live transcripts (`BuildProgress`). Only ever on the user's click: the board shows what the
//! agents chose to report, this answers "and now?" in between.

use std::io::{Read, Seek, SeekFrom, Write};
use std::process::{Command, Stdio};

use regex::Regex;
use serde_json::{json, Map, Value};

use crate::ruby_text::strip;
use crate::session_log::SessionLog;
use crate::work_item::WorkItem;

const PROMPT: &str = "You estimate the progress of AI coding sub-agents that are working right\nnow. Stdin holds one block per agent: its label, the task brief it was\ngiven, and its recent activity (oldest first; its tool calls and what it\nsaid). For each agent, estimate how far it is through its task as a\npercentage, and say in one short plain sentence what it is doing now\n(e.g. \"core change committed, fixing 2 failing tests\"). Judge by what the\nbrief asks for versus what the activity shows done; never mention tool\ncall counts. Answer every agent, using its label exactly as given.\n";
const ANSWER_SCHEMA: &str = "{\"type\":\"object\",\"required\":[\"agents\"],\"properties\":{\"agents\":{\"type\":\"array\",\"items\":{\"type\":\"object\",\"required\":[\"label\",\"percent\",\"doing\"],\"properties\":{\"label\":{\"type\":\"string\"},\"percent\":{\"type\":\"integer\"},\"doing\":{\"type\":\"string\"}}}}}}";
const BRIEF_CHARS: usize = 2_000;
const TAIL_BYTES: u64 = 200_000;
const ACTIVITIES: usize = 20;
const ACTIVITY_CHARS: usize = 200;
const INPUT_KEYS: [&str; 10] = ["description", "command", "file_path", "pattern", "path", "query", "prompt", "action", "url", "skill"];

pub enum Fail {
    Arg(String),
    Unported,
}

type R<T> = Result<T, Fail>;

#[derive(Clone)]
pub struct Agent {
    pub run_id: String,
    pub agent_id: String,
    pub label: String,
    pub task: String,
    pub phase: Option<String>,
    pub transcript: String,
}

impl Agent {
    /// Which agent an estimate line belongs to, across state builds.
    pub fn key(&self) -> String {
        format!("{}/{}", self.run_id, self.agent_id)
    }
}

pub fn job_key(slug: &str) -> String {
    format!("progress:{slug}")
}

/// An agent in the user's terms; `None` for the workflow's internal helpers.
pub fn task_name(label: &str) -> Option<String> {
    let build = Regex::new(r"\Abuild:(.+)\z").unwrap();
    let review = Regex::new(r"\Areview:(.+)\z").unwrap();
    if let Some(c) = build.captures(label) {
        return Some(format!("{} · building", &c[1]));
    }
    if let Some(c) = review.captures(label) {
        return Some(format!("{} · reviewing", &c[1]));
    }
    if label == "integrate" {
        return Some("integrating".into());
    }
    if label.starts_with("queue#") || label.to_lowercase().contains("check") {
        return None;
    }
    Some(label.to_string())
}

fn s(v: Option<&Value>) -> R<String> {
    match v {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(t)) => Ok(t.clone()),
        Some(Value::Bool(b)) => Ok(b.to_string()),
        Some(Value::Number(n)) if n.is_i64() || n.is_u64() => Ok(n.to_string()),
        _ => Err(Fail::Unported),
    }
}

fn truthy(v: Option<&Value>) -> Option<&Value> {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => None,
        other => other,
    }
}

fn read_lines(path: &str) -> Option<Vec<String>> {
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Some(text.split_inclusive('\n').map(|l| l.trim_end_matches('\n').to_string()).collect())
}

fn json_line(line: &str) -> Option<Value> {
    serde_json::from_str(line).ok()
}

/// The last `TAIL_BYTES` of a transcript as entries (a cut-off first line dropped).
fn tail_entries(path: &str) -> Vec<Map<String, Value>> {
    let Ok(mut f) = std::fs::File::open(path) else { return vec![] };
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(size.saturating_sub(TAIL_BYTES)));
    let mut data = vec![];
    let _ = f.read_to_end(&mut data);
    let text = String::from_utf8_lossy(&data).into_owned();
    let mut lines: Vec<&str> = text.split('\n').collect();
    while lines.last() == Some(&"") {
        lines.pop();
    }
    if size > TAIL_BYTES && !lines.is_empty() {
        lines.remove(0);
    }
    lines.into_iter().filter_map(json_line).filter_map(|v| v.as_object().cloned()).collect()
}

fn tool_uses(entry: &Map<String, Value>) -> Vec<Value> {
    match entry.get("message").and_then(|m| m.get("content")) {
        Some(Value::Array(a)) => a.iter().filter(|b| b.get("type") == Some(&json!("tool_use"))).cloned().collect(),
        _ => vec![],
    }
}

fn handed_back(path: &str) -> bool {
    let entries = tail_entries(path);
    let last = entries.iter().rev().find(|e| e.get("type") == Some(&json!("assistant")));
    last.is_some_and(|e| tool_uses(e).iter().any(|u| u.get("name") == Some(&json!("StructuredOutput"))))
}

fn read_json(path: &str) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn running(session_dir: &str, run_id: &str) -> R<bool> {
    let result = format!("{session_dir}/workflows/{run_id}.json");
    if !std::path::Path::new(&result).exists() {
        return Ok(true);
    }
    let Ok(text) = std::fs::read_to_string(&result) else { return Err(Fail::Unported) };
    match serde_json::from_str::<Value>(&text) {
        Err(_) => Ok(true), // half-written: the run is ending right now
        Ok(Value::Object(o)) => Ok(match o.get("status") {
            None | Some(Value::Null) => true,
            Some(Value::String(st)) => matches!(st.as_str(), "" | "running" | "pending"),
            Some(_) => false,
        }),
        Ok(_) => Err(Fail::Unported),
    }
}

fn open_from_journal(journal: &str) -> Vec<(String, Value)> {
    let mut open: Vec<(Value, Value)> = vec![];
    for line in read_lines(journal).unwrap_or_default() {
        let Some(Value::Object(entry)) = json_line(&line) else { continue };
        let key = entry.get("key").cloned().unwrap_or(Value::Null);
        match entry.get("type").and_then(Value::as_str) {
            Some("started") => match open.iter_mut().find(|(k, _)| *k == key) {
                Some((_, v)) => *v = Value::Object(entry),
                None => open.push((key, Value::Object(entry))),
            },
            Some("result") | Some("failed") => open.retain(|(k, _)| *k != key),
            _ => {}
        }
    }
    let mut by_agent: Vec<(String, Value)> = vec![];
    for (_, e) in open {
        let id = match e.get("agentId") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(t)) => t.clone(),
            Some(other) => other.to_string(),
        };
        if id.is_empty() {
            continue;
        }
        match by_agent.iter_mut().find(|(k, _)| *k == id) {
            Some((_, v)) => *v = e,
            None => by_agent.push((id, e)),
        }
    }
    by_agent
}

fn sorted_glob(dir: &str, prefix: &str, suffix: &str) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).map(|d| d.filter_map(|e| e.ok()).filter_map(|e| e.file_name().into_string().ok()).filter(|n| !n.starts_with('.') && n.starts_with(prefix) && n.ends_with(suffix)).collect()).unwrap_or_default();
    names.sort();
    names
}

fn run_agents(run_dir: &str, run_id: &str) -> R<Vec<Agent>> {
    let journal = format!("{run_dir}/journal.jsonl");
    let open: Vec<(String, Value)> = if std::path::Path::new(&journal).exists() {
        open_from_journal(&journal)
    } else {
        sorted_glob(run_dir, "agent-", ".jsonl")
            .into_iter()
            .map(|n| format!("{run_dir}/{n}"))
            .filter(|p| !handed_back(p))
            .map(|p| (std::path::Path::new(&p).file_stem().unwrap().to_string_lossy().trim_start_matches("agent-").to_string(), json!({})))
            .collect()
    };
    let mut out = vec![];
    for (agent_id, started) in open {
        let transcript = format!("{run_dir}/agent-{agent_id}.jsonl");
        if !std::path::Path::new(&transcript).exists() {
            continue;
        }
        let meta = read_json(&format!("{run_dir}/agent-{agent_id}.meta.json")).unwrap_or_else(|| json!({}));
        if !meta.is_object() {
            return Err(Fail::Unported);
        }
        let label = match truthy(started.get("label")).or_else(|| truthy(meta.get("description"))) {
            Some(Value::String(t)) => t.clone(),
            Some(_) => return Err(Fail::Unported),
            None => agent_id.clone(),
        };
        let Some(task) = task_name(&label) else { continue };
        let phase = match truthy(started.get("phase")).or_else(|| truthy(meta.get("workflowPhase"))) {
            Some(v) => Some(s(Some(v))?),
            None => None,
        };
        out.push(Agent { run_id: run_id.to_string(), agent_id, label, task, phase, transcript });
    }
    Ok(out)
}

/// Every still-working sub-agent of every running run of the item's live sessions.
pub fn active_agents(item: &WorkItem, log: &SessionLog) -> R<Vec<Agent>> {
    let mut out = vec![];
    for id in item.session_ids() {
        let Some(session) = log.session_for(&id).filter(|s| s.live) else { continue };
        let Some(transcript) = session.transcript() else { continue };
        let session_dir = transcript.to_string_lossy().trim_end_matches(".jsonl").to_string();
        let base = format!("{session_dir}/subagents/workflows");
        let mut runs: Vec<String> = std::fs::read_dir(&base).map(|d| d.filter_map(|e| e.ok()).filter_map(|e| e.file_name().into_string().ok()).filter(|n| !n.starts_with('.')).collect()).unwrap_or_default();
        runs.sort();
        for run_id in runs {
            let run_dir = format!("{base}/{run_id}");
            if !std::path::Path::new(&run_dir).is_dir() {
                continue;
            }
            if running(&session_dir, &run_id)? {
                out.extend(run_agents(&run_dir, &run_id)?);
            }
        }
    }
    Ok(out)
}

fn clip(text: &str, max: usize) -> String {
    let squished = squish(text);
    if squished.chars().count() > max { format!("{}…", squished.chars().take(max).collect::<String>()) } else { squished }
}

fn squish(text: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in text.chars() {
        if matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r') {
            space = true;
        } else {
            if space {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    if space {
        out.push(' ');
    }
    strip(&out).to_string()
}

/// The task the agent was given: its last plain user message before it first answered.
fn brief(path: &str) -> String {
    let mut texts: Vec<String> = vec![];
    for line in read_lines(path).unwrap_or_default() {
        let Some(Value::Object(entry)) = json_line(&line) else { continue };
        match entry.get("type").and_then(Value::as_str) {
            Some("assistant") => break,
            Some("user") => {}
            _ => continue,
        }
        let content = entry.get("message").and_then(|m| m.get("content"));
        let text = match content {
            Some(Value::String(t)) => t.clone(),
            Some(Value::Array(blocks)) => blocks.iter().filter(|b| b.get("type") == Some(&json!("text"))).filter_map(|b| b.get("text").and_then(Value::as_str).map(String::from)).collect::<Vec<_>>().join("\n"),
            _ => String::new(),
        };
        if !strip(&text).is_empty() {
            texts.push(text);
        }
    }
    let last = texts.last().cloned().unwrap_or_default();
    Regex::new(r"(?s)\A[ \t\r\n\x0c\x0b]*\[Workflow harness[^\]\n]*\].*?text follows:[ \t\r\n\x0c\x0b]*").unwrap().replace(&last, "").into_owned()
}

fn first_timestamp(path: &str) -> R<Option<f64>> {
    for line in read_lines(path).unwrap_or_default() {
        let Some(Value::Object(entry)) = json_line(&line) else { continue };
        let stamp = match truthy(entry.get("timestamp")) {
            Some(Value::String(t)) => t.clone(),
            Some(_) => return Err(Fail::Unported),
            None => continue,
        };
        return match chrono::DateTime::parse_from_rfc3339(&stamp) {
            Ok(t) => Ok(Some(t.timestamp_millis() as f64 / 1000.0)),
            Err(_) => Err(Fail::Unported),
        };
    }
    Ok(None)
}

fn input_summary(input: Option<&Value>) -> R<String> {
    match input {
        Some(Value::Object(m)) => {
            for k in INPUT_KEYS {
                if let Some(Value::String(t)) = m.get(k) {
                    if !strip(t).is_empty() {
                        return Ok(t.clone());
                    }
                }
            }
            Ok(Value::Object(m.clone()).to_string())
        }
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(t)) => Ok(t.clone()),
        Some(Value::Bool(b)) => Ok(b.to_string()),
        Some(Value::Number(n)) if n.is_i64() || n.is_u64() => Ok(n.to_string()),
        _ => Err(Fail::Unported),
    }
}

/// What the agent said and called, newest last; tool outputs left out.
fn recent_activity(path: &str) -> R<Vec<String>> {
    let mut out = vec![];
    for entry in tail_entries(path).iter().filter(|e| e.get("type") == Some(&json!("assistant"))) {
        let Some(Value::Array(blocks)) = entry.get("message").and_then(|m| m.get("content")) else { continue };
        for block in blocks.iter().filter(|b| b.is_object()) {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    let text = s(block.get("text"))?;
                    if !strip(&text).is_empty() {
                        out.push(format!("said: {}", clip(&text, ACTIVITY_CHARS)));
                    }
                }
                Some("tool_use") => out.push(format!("{}: {}", s(block.get("name"))?, clip(&input_summary(block.get("input"))?, ACTIVITY_CHARS))),
                _ => {}
            }
        }
    }
    let skip = out.len().saturating_sub(ACTIVITIES);
    Ok(out.split_off(skip))
}

/// One agent's block of the model's input: label, brief, recent activity.
fn condense(agent: &Agent, now: f64) -> R<String> {
    let started = first_timestamp(&agent.transcript)?;
    let mut head = format!("## {} (run {}", agent.label, agent.run_id);
    if let Some(p) = &agent.phase {
        head += &format!(", phase {p}");
    }
    if let Some(t) = started {
        head += &format!(", working for {} min", ((now - t) / 60.0).round() as i64);
    }
    head += ")";
    let activity = recent_activity(&agent.transcript)?;
    let shown = if activity.is_empty() { "(none yet)".to_string() } else { activity.iter().map(|a| format!("- {a}")).collect::<Vec<_>>().join("\n") };
    Ok([head, "Brief:".into(), clip(&brief(&agent.transcript), BRIEF_CHARS), "Recent activity:".into(), shown].join("\n"))
}

fn headless_claude(stdin: &str) -> R<(String, String, bool)> {
    let mut child = Command::new("claude")
        .args(["-p", PROMPT, "--model", "haiku", "--tools", "", "--strict-mcp-config", "--no-session-persistence", "--output-format", "json", "--json-schema", ANSWER_SCHEMA])
        .env("QM_SKIP", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| Fail::Unported)?;
    if let Some(mut input) = child.stdin.take() {
        let _ = input.write_all(stdin.as_bytes());
    }
    let out = child.wait_with_output().map_err(|_| Fail::Unported)?;
    Ok((String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned(), out.status.success()))
}

fn parse(out: &str) -> Option<Vec<Map<String, Value>>> {
    let v: Value = serde_json::from_str(out).ok()?;
    let list = v.get("structured_output")?.get("agents")?.as_array()?;
    list.iter().map(|e| e.as_object().cloned()).collect()
}

fn answer_text(out: &str) -> String {
    match serde_json::from_str::<Value>(out) {
        Ok(Value::Object(o)) => match o.get("result") {
            Some(Value::String(r)) if !strip(r).is_empty() => strip(r).to_string(),
            _ => strip(out).to_string(),
        },
        _ => strip(out).to_string(),
    }
}

fn lines(agents: &[Agent], estimates: &[Map<String, Value>]) -> R<Vec<Value>> {
    let mut by_label: Vec<(String, &Map<String, Value>)> = vec![];
    for e in estimates {
        let label = s(e.get("label"))?;
        match by_label.iter_mut().find(|(k, _)| *k == label) {
            Some((_, v)) => *v = e,
            None => by_label.push((label, e)),
        }
    }
    let empty = Map::new();
    let mut out = vec![];
    for agent in agents {
        let estimate = by_label.iter().find(|(k, _)| *k == agent.label).map(|(_, e)| *e).unwrap_or(&empty);
        let percent = match estimate.get("percent") {
            Some(Value::Number(n)) => match n.as_i64() {
                Some(i) => json!(i.clamp(0, 100)),
                None => json!(n.as_f64().unwrap_or(0.0).clamp(0.0, 100.0).round() as i64),
            },
            _ => Value::Null,
        };
        let doing = strip(&s(estimate.get("doing"))?).to_string();
        out.push(json!({"task": agent.task, "run": agent.run_id, "agent": agent.key(), "percent": percent, "doing": if doing.is_empty() { "no estimate".to_string() } else { doing }}));
    }
    Ok(out)
}

/// `{ok, taken_at, agents, raw, message}`; nothing running answers without a model call.
pub fn call(item: &WorkItem) -> R<Value> {
    let log = SessionLog::read();
    let agents = active_agents(item, &log)?;
    let now = crate::clock::now_f64();
    let taken = crate::clock::now_iso();
    if agents.is_empty() {
        return Ok(json!({"ok": true, "taken_at": taken, "agents": [], "message": "nothing running"}));
    }
    let mut blocks = vec![];
    for a in &agents {
        blocks.push(condense(a, now)?);
    }
    let (out, err, success) = headless_claude(&blocks.join("\n\n"))?;
    if !success {
        let shown = if strip(&err).is_empty() { strip(&out).to_string() } else { strip(&err).to_string() };
        return Err(Fail::Arg(format!("claude -p failed: {shown}")));
    }
    match parse(&out) {
        None => Ok(json!({"ok": true, "taken_at": taken, "agents": [], "raw": answer_text(&out)})),
        Some(estimates) => Ok(json!({"ok": true, "taken_at": taken, "agents": lines(&agents, &estimates)?})),
    }
}
