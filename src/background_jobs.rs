//! The background agents, workflows and shell commands a session launched that have not reported
//! back yet, read from its transcript. A launch is a tool result with status "async_launched", one
//! carrying a backgroundTaskId (a Bash run in the background), a SendMessage result carrying
//! resumedAgentId, or a status-less <task-notification> saying the user resumed that agent. Its
//! end is the next <task-notification> naming that id or the successful TaskStop result, but not
//! an interim notification, which only refreshes its age. Hooks see neither end, so the transcript
//! is the only record that needs nothing from the session itself.
//!
//! The Ruby module keeps a process-wide cache for the long-lived GUI; a CLI run folds once.

use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{Map, Value};

use crate::clock;

/// A job still out is dropped past this age since its launch, or its last interim notification.
const MAX_AGE_SECS: f64 = 12.0 * 3600.0;
const INTERIM: &str = "the result below may be interim";
const BASH_USE: &str = "\"name\":\"Bash\"";
const STOPPED: &str = "Successfully stopped task";
const SHELL_DESCRIPTION_MAX: usize = 80;
const MARKERS: [&str; 5] = [STOPPED, "<task-notification>", "async_launched", "resumedAgentId", BASH_USE];

#[derive(Clone)]
pub struct Job {
    pub id: Value,
    pub description: Value,
    pub workflow: bool,
    pub launched_at: Option<f64>,
    pub command: Option<String>,
    pub seen_at: Option<f64>,
}

/// Ruby's `\w` and `\s` are ASCII-only while its `\b` follows Unicode word characters, so the
/// patterns spell the classes out to match that.
fn regexes() -> &'static (Regex, Regex, Regex, Regex) {
    static R: OnceLock<(Regex, Regex, Regex, Regex)> = OnceLock::new();
    R.get_or_init(|| {
        (
            Regex::new(r"(?i)\b(watch|monitor|babysit)(?-u:\w)*\b").unwrap(),
            Regex::new(r"\bgh(?-u:\s)+(pr(?-u:\s)+checks\b[^\n;&|]*(?-u:\s)--watch\b|run(?-u:\s)+watch\b)").unwrap(),
            Regex::new(r"<task-id>([^<\\]+)</task-id>").unwrap(),
            Regex::new(r#"<summary>Agent "(.*)" was resumed by the user</summary>"#).unwrap(),
        )
    })
}

fn to_s(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

impl Job {
    /// A watcher (a CI or PR watch) waits on the world, not on work: while only watchers run, the
    /// turn is still the user's.
    pub fn watcher(&self) -> bool {
        let (name, command, ..) = regexes();
        !self.workflow && (name.is_match(&to_s(&self.description)) || command.is_match(self.command.as_deref().unwrap_or("")))
    }

    pub fn description_text(&self) -> String {
        to_s(&self.description)
    }
}

/// The error class of a failure Ruby's `rescue` in `apply` does not catch (it ends the whole search).
struct Abort;

fn truthy(v: Option<&Value>) -> bool {
    !matches!(v, None | Some(Value::Null) | Some(Value::Bool(false)))
}

/// `entry["timestamp"] && Time.parse(…)`: Err(true) for what Ruby's `rescue ArgumentError` skips the line over,
/// Err(false) for what aborts the search.
fn launched_at(entry: &Value) -> Result<Option<f64>, bool> {
    match entry.get("timestamp") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => Ok(None),
        Some(Value::String(s)) => clock::parse_f64(s).map(Some).ok_or(true),
        Some(_) => Err(false),
    }
}

/// `entry.dig("message", "content")`.
fn content(entry: &Value) -> Result<Option<&Value>, ()> {
    match entry.get("message") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(m)) => Ok(m.get("content")),
        Some(_) => Err(()),
    }
}

fn blocks(v: Option<&Value>) -> Vec<&Value> {
    match v {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(_)) | None | Some(Value::Null) => vec![],
        Some(other) => vec![other],
    }
}

struct Fold {
    jobs: Vec<(String, Job)>,
    shells: Vec<(String, (Value, String))>,
    agents: Vec<(String, Value)>,
}

impl Fold {
    fn job_index(&self, id: &Value) -> Option<usize> {
        let key = id.to_string();
        self.jobs.iter().position(|(k, _)| *k == key)
    }

    fn put(&mut self, job: Job) {
        let key = job.id.to_string();
        match self.jobs.iter_mut().find(|(k, _)| *k == key) {
            Some((_, slot)) => *slot = job,
            None => self.jobs.push((key, job)),
        }
    }

    fn remove(&mut self, id: &Value) {
        if let Some(i) = self.job_index(id) {
            self.jobs.remove(i);
        }
    }

    fn agent(&self, id: &Value) -> Option<Value> {
        let key = id.to_string();
        self.agents.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone())
    }

    fn remember_agent(&mut self, id: &Value, description: &Value) {
        let key = id.to_string();
        match self.agents.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => *v = description.clone(),
            None => self.agents.push((key, description.clone())),
        }
    }
}

fn first_truthy<'a>(a: Option<&'a Value>, b: Option<&'a Value>) -> Option<&'a Value> {
    if truthy(a) { a } else if truthy(b) { b } else { None }
}

fn launch(entry: &Value) -> Result<Option<Job>, bool> {
    let result = entry.get("toolUseResult");
    let Some(r) = result.and_then(Value::as_object) else { return Ok(None) };
    if truthy(entry.get("isSidechain")) || r.get("status") != Some(&Value::String("async_launched".into())) {
        return Ok(None);
    }
    let Some(id) = first_truthy(r.get("agentId"), r.get("taskId")) else { return Ok(None) };
    let description = first_truthy(r.get("description"), r.get("workflowName")).cloned().unwrap_or(Value::Null);
    Ok(Some(Job { id: id.clone(), description, workflow: r.get("taskType") == Some(&Value::String("local_workflow".into())), launched_at: launched_at(entry)?, command: None, seen_at: None }))
}

/// SendMessage reviving a sub-agent that had stopped: a launch of that agent again.
fn resume(entry: &Value, fold: &Fold) -> Result<Option<Job>, bool> {
    let Some(r) = entry.get("toolUseResult").and_then(Value::as_object) else { return Ok(None) };
    if truthy(entry.get("isSidechain")) || entry.get("type").and_then(Value::as_str) != Some("user") {
        return Ok(None);
    }
    let Some(id) = r.get("resumedAgentId").filter(|v| v.as_str().is_some_and(|s| !s.is_empty())) else { return Ok(None) };
    let description = fold.agent(id).filter(|d| truthy(Some(d))).or_else(|| r.get("message").cloned()).unwrap_or(Value::Null);
    Ok(Some(Job { id: id.clone(), description, workflow: false, launched_at: launched_at(entry)?, command: None, seen_at: None }))
}

/// The text of a `<task-notification>`: only the notification itself ends a job.
fn notification(entry: &Value) -> Result<Option<String>, ()> {
    if truthy(entry.get("isSidechain")) {
        return Ok(None);
    }
    let text = match entry.get("type").and_then(Value::as_str) {
        Some("queue-operation") => entry.get("content").and_then(Value::as_str).map(String::from),
        Some("user") => match content(entry)? {
            Some(Value::String(s)) => Some(s.clone()),
            other => blocks(other).into_iter().find(|b| b.get("type") == Some(&Value::String("text".into())) && b.is_object()).and_then(|b| b.get("text")).and_then(Value::as_str).map(String::from),
        },
        _ => None,
    };
    Ok(text.filter(|t| t.trim_start().starts_with("<task-notification>")))
}

/// Only TaskStop's own structured result.
fn stopped_id(entry: &Value) -> Option<Value> {
    let r = entry.get("toolUseResult").and_then(Value::as_object)?;
    if truthy(entry.get("isSidechain")) || entry.get("type").and_then(Value::as_str) != Some("user") {
        return None;
    }
    let id = first_truthy(r.get("task_id"), r.get("shell_id"))?;
    let id_text = id.as_str()?;
    to_s(r.get("message").unwrap_or(&Value::Null)).starts_with(&format!("{STOPPED}: {id_text}")).then(|| id.clone())
}

fn remember_shells(fold: &mut Fold, entry: &Value) -> Result<(), ()> {
    if truthy(entry.get("isSidechain")) || entry.get("type").and_then(Value::as_str) != Some("assistant") {
        return Ok(());
    }
    for block in blocks(content(entry)?) {
        let Some(b) = block.as_object() else { continue };
        if b.get("type") != Some(&Value::String("tool_use".into())) || b.get("name") != Some(&Value::String("Bash".into())) || !truthy(b.get("id")) {
            continue;
        }
        let empty = Map::new();
        let input = b.get("input").and_then(Value::as_object).unwrap_or(&empty);
        let key = b["id"].to_string();
        let value = (input.get("description").cloned().unwrap_or(Value::Null), to_s(input.get("command").unwrap_or(&Value::Null)));
        match fold.shells.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => *v = value,
            None => fold.shells.push((key, value)),
        }
    }
    Ok(())
}

/// The result answering a pending Bash call: it stops being pending, and is a launch when it
/// carries a backgroundTaskId. Hook attachments mention the id too, so only a tool_result counts.
fn shell_result(fold: &mut Fold, line: &str) -> Result<bool, bool> {
    let Some(pos) = fold.shells.iter().position(|(k, _)| line.contains(k.trim_matches('"'))) else { return Ok(false) };
    let use_id = fold.shells[pos].0.clone();
    let Ok(entry) = serde_json::from_str::<Value>(line) else { return Err(true) };
    let answers = blocks(content(&entry).map_err(|_| false)?).iter().any(|b| b.get("type") == Some(&Value::String("tool_result".into())) && b.get("tool_use_id").map(|v| v.to_string()) == Some(use_id.clone()));
    if !answers {
        return Ok(false);
    }
    let (description, command) = fold.shells.remove(pos).1;
    let Some(id) = entry.get("toolUseResult").and_then(Value::as_object).and_then(|r| r.get("backgroundTaskId")).filter(|v| truthy(Some(v))) else { return Ok(true) };
    let description = if to_s(&description).trim().is_empty() {
        Value::String(command.lines().next().unwrap_or("").trim().chars().take(SHELL_DESCRIPTION_MAX).collect())
    } else {
        description
    };
    let launched = launched_at(&entry)?;
    fold.put(Job { id: id.clone(), description, workflow: false, launched_at: launched, command: Some(command), seen_at: None });
    Ok(true)
}

fn apply(fold: &mut Fold, line: &str) -> Result<(), Abort> {
    // Err(true): Ruby's rescue (bad JSON or time) skips the line; Err(false): it ends the search
    let skip_or_abort = |e: bool| if e { Ok(()) } else { Err(Abort) };
    if !fold.shells.is_empty() {
        match shell_result(fold, line) {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(e) => return skip_or_abort(e),
        }
    }
    if !MARKERS.iter().any(|m| line.contains(m)) {
        return Ok(());
    }
    let Ok(entry) = serde_json::from_str::<Value>(line) else { return Ok(()) };
    if !entry.is_object() && !entry.is_string() {
        return Err(Abort);
    }
    if !entry.is_object() {
        return Ok(());
    }
    let (_, _, task_id, user_resumed) = regexes();
    if let Some(id) = stopped_id(&entry) {
        fold.remove(&id);
    } else if let Some(text) = notification(&entry).map_err(|_| Abort)? {
        let resumed = if text.contains("<status>") {
            None
        } else if let (Some(name), Some(id)) = (user_resumed.captures(&text).map(|c| c[1].to_string()), task_id.captures(&text).map(|c| c[1].to_string())) {
            let id = Value::String(id);
            let description = fold.agent(&id).filter(|d| truthy(Some(d))).unwrap_or(Value::String(name));
            match launched_at(&entry) {
                Ok(at) => Some(Job { id, description, workflow: false, launched_at: at, command: None, seen_at: None }),
                Err(e) => return skip_or_abort(e),
            }
        } else {
            None
        };
        if let Some(job) = resumed {
            fold.put(job);
        } else if text.contains(INTERIM) {
            for c in task_id.captures_iter(&text) {
                let id = Value::String(c[1].to_string());
                let at = match launched_at(&entry) {
                    Ok(at) => at,
                    Err(e) => return skip_or_abort(e),
                };
                if let Some(i) = fold.job_index(&id) {
                    fold.jobs[i].1.seen_at = at;
                }
            }
        } else {
            for c in task_id.captures_iter(&text) {
                fold.remove(&Value::String(c[1].to_string()));
            }
        }
    } else {
        let found = match launch(&entry) {
            Ok(Some(j)) => Some(j),
            Ok(None) => match resume(&entry, fold) {
                Ok(j) => j,
                Err(e) => return skip_or_abort(e),
            },
            Err(e) => return skip_or_abort(e),
        };
        match found {
            Some(job) => {
                if !job.workflow {
                    fold.remember_agent(&job.id, &job.description);
                }
                fold.put(job);
            }
            None => remember_shells(fold, &entry).map_err(|_| Abort)?,
        }
    }
    Ok(())
}

/// Jobs still running, oldest launch first; empty when the transcript cannot be read as Ruby reads it.
pub fn pending(path: Option<&Path>, now: f64) -> Vec<Job> {
    let Some(path) = path.filter(|p| p.exists()) else { return vec![] };
    let Ok(data) = std::fs::read(path) else { return vec![] };
    let mut fold = Fold { jobs: vec![], shells: vec![], agents: vec![] };
    for line in data.split_inclusive(|b| *b == b'\n') {
        if line.last() != Some(&b'\n') {
            break; // a line still being written: next time
        }
        let Ok(text) = std::str::from_utf8(line) else { continue };
        if apply(&mut fold, text).is_err() {
            return vec![];
        }
    }
    let mut jobs: Vec<Job> = fold.jobs.into_iter().map(|(_, j)| j).filter(|j| j.seen_at.or(j.launched_at).is_none_or(|seen| now - seen < MAX_AGE_SECS)).collect();
    jobs.sort_by(|a, b| a.launched_at.unwrap_or(0.0).partial_cmp(&b.launched_at.unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal));
    jobs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(description: &str, command: Option<&str>, workflow: bool) -> Job {
        Job { id: "i".into(), description: description.into(), workflow, launched_at: None, command: command.map(String::from), seen_at: None }
    }

    #[test]
    fn watchers_are_judged_by_name_or_by_a_watching_gh_command() {
        assert!(job("Watch CI for the PR", None, false).watcher());
        assert!(job("babysit deploy", None, false).watcher());
        assert!(!job("stopwatch module", None, false).watcher());
        assert!(!job("Watch CI", None, true).watcher());
        assert!(job("run it", Some("gh pr checks 5 --watch"), false).watcher());
        assert!(job("run it", Some("gh run watch 123"), false).watcher());
        assert!(!job("run it", Some("gh pr view 5"), false).watcher());
    }
}
