//! A project's dev server (config.yml `dev_server:`), driven through its command: what it serves
//! right now. It serves one checkout at a time, so this is a single shared resource, not per item.
//! Only the read-only status is ported; switching and stopping stay in Ruby.

use serde_json::Value;

use crate::command_runner;
use crate::data_root;
use crate::project_config::{DevServer, ProjectConfig};

pub struct Status {
    pub mode: Value,
    pub root: Option<String>,
    pub running: bool,
    pub switching: bool,
}

/// The dev server qm reads: the first one any project configures.
pub fn configured_server(config: &ProjectConfig) -> Option<&DevServer> {
    config.dev_server()
}

/// The status URL answers from inside the running server — authoritative, but only while it is up.
fn from_endpoint(server: &DevServer) -> Option<Status> {
    let url = server.status_url.as_deref()?;
    let result = command_runner::call(&["curl", "-s", "--max-time", "0.5", url]);
    if !result.success {
        return None;
    }
    let data: Value = serde_json::from_str(&result.out).ok()?;
    let obj = data.as_object()?;
    obj.get("mode").filter(|m| !matches!(m, Value::Null | Value::Bool(false)))?;
    Some(Status { mode: obj["mode"].clone(), root: obj.get("root").and_then(Value::as_str).map(String::from), running: true, switching: obj.get("switching") == Some(&Value::Bool(true)) })
}

/// `<command> status` exits non-zero when nothing runs; the state file, written on every switch and
/// never cleared on stop, names what it was (or still is) serving.
fn from_files(server: &DevServer) -> Status {
    let text = server.state_file.as_deref().and_then(|f| std::fs::read_to_string(f).ok());
    let stripped = text.as_deref().map(|t| crate::ruby_text::strip(t).to_string());
    let mode = stripped.as_deref().and_then(|t| t.split_whitespace().next()).map(|m| Value::String(m.to_string())).unwrap_or(Value::Null);
    let root = stripped.as_deref().and_then(|t| t.split_once(' ').map(|(_, r)| r.trim_start_matches(' ').to_string()));
    let running = command_runner::call(&[server.command.as_str(), "status"]).success;
    Status { mode, root, running, switching: false }
}

pub fn status(server: Option<&DevServer>) -> Option<Status> {
    let server = server?;
    from_endpoint(server).or_else(|| Some(from_files(server)))
}

/// Whether the status says it is serving `path` right now.
pub fn serves(status: &Option<Status>, path: &str) -> bool {
    match status {
        Some(Status { running: true, root: Some(root), .. }) => data_root::normalize(root) == data_root::normalize(path),
        _ => false,
    }
}

/// `Pf#switch` / `Pf#stop`: the command run, its own message when it fails.
fn run(argv: &[&str], label: &str) -> Result<(), String> {
    let result = command_runner::call(argv);
    if result.success {
        return Ok(());
    }
    let message = [result.err.as_str(), result.out.as_str()].iter().map(|t| crate::ruby_text::strip(t)).find(|t| !t.is_empty()).unwrap_or("exit status non-zero").to_string();
    Err(format!("{label} failed: {message}"))
}

pub fn switch(server: &DevServer, path: &str, mode: &str) -> Result<(), String> {
    run(&[server.command.as_str(), "switch", path, mode], &format!("{} switch", server.command))
}

pub fn stop(server: &DevServer) -> Result<(), String> {
    run(&[server.command.as_str(), "stop"], &format!("{} stop", server.command))
}
