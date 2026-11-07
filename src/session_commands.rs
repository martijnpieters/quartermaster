//! Session views and the static commands, ported from `CliViewer`, `CliFormat` and `Cli`.

use serde_json::{json, Map, Value};

use crate::cli_format::{format_duration, paint, puts, short_path};
use crate::delegate;
use crate::error::QmError;
use crate::outcome::{abort, done, Outcome};
use crate::ruby_text::prefix;
use crate::session_log::SessionLog;
use crate::work_item::WorkItem;

const USAGE: &str = include_str!("usage.txt");

/// `Some(outcome)` for the commands ported here, `None` for every other word.
pub fn run(command: &str, args: &[String]) -> Option<Outcome> {
    Some(match command {
        "viewer-rows" | "session-rows" | "inbox-rows" | "cycle-filter" => crate::viewer_commands::run(command, args),
        "rotate" | "caffeinate" | "pane-start" | "pane-resume" => return crate::chats_commands::run(command, args),
        "gui" if std::env::var_os("QM_RUST_GUI").is_some_and(|v| !v.is_empty()) => crate::gui_server::run(),
        "ask" => crate::ask_command::ask(args),
        "grep" => crate::grep_command::grep(args),
        "attention" => crate::attention_command::attention(),
        "show" => crate::show_command::show(args),
        "show-session" => show_session(args),
        "session-item" => session_item(args),
        "help" | "--help" | "-h" => done(USAGE.to_string()),
        "install-hooks" => install_hooks(),
        _ => return None,
    })
}

fn failed(e: QmError) -> Outcome {
    abort(&format!("qm: {}", e.message))
}

fn show_session(a: &[String]) -> Outcome {
    let Some(id) = a.first() else { return abort("usage: qm show-session SESSION_ID") };
    let Some(session) = SessionLog::read().session_for(id) else { return abort(&format!("qm: unknown session {id}")) };

    let mut out = String::new();
    out.push_str(&puts(&paint(&format!("Session {id}"), 1)));
    out.push_str(&puts(&format!("tier: {}", session.tier())));
    out.push_str(&puts(&format!("cwd: {}", short_path(session.cwd().as_deref()))));
    out.push_str(&puts(&format!(
        "branch: {} · project: {} · work item: {}",
        session.branch().unwrap_or_else(|| "-".into()),
        session.project().unwrap_or_else(|| "-".into()),
        session.item_label().unwrap_or_else(|| "-".into())
    )));
    out.push_str(&puts(&format!("started: {}", session.started_ts())));
    if session.ended() {
        out.push_str(&puts(&format!(
            "ended: {} ({}, {})",
            session.last_ts(),
            format_duration(session.duration_s()),
            session.end_reason().unwrap_or_default()
        )));
        for c in session.commits() {
            out.push_str(&puts(&format!("  commit: {c}")));
        }
    }
    if let Some(prompt) = session.first_prompt() {
        out.push_str("\n");
        out.push_str(&puts(&paint("First prompt", 1)));
        out.push_str(&puts(&prefix(&prompt, 400)));
    }
    done(out)
}

fn session_item(a: &[String]) -> Outcome {
    let Some(id) = a.first() else { return done(String::new()) };
    let items = match WorkItem::all() {
        Ok(items) => items,
        Err(e) => return failed(e),
    };
    match items.iter().find(|i| i.session_ids().iter().any(|s| s == id)) {
        Some(item) => done(puts(&item.title())),
        None => done(String::new()),
    }
}

/// Prints, never writes: settings.json is the user's to edit. Paths go through the launcher's realpath.
fn install_hooks() -> Outcome {
    let launcher = std::fs::canonicalize(delegate::checkout_launcher()).unwrap_or_else(|_| delegate::checkout_launcher());
    let hooks_dir = launcher.parent().map(|p| p.join("hooks")).unwrap_or_default();
    let hook = |name: &str| hooks_dir.join(name).to_string_lossy().into_owned();
    let command = |cmd: &str| json!({ "hooks": [{ "type": "command", "command": cmd }] });
    let launcher = launcher.to_string_lossy().into_owned();
    let events = hook("record-event.sh");
    let mut hooks = Map::new();
    hooks.insert("SessionStart".into(), json!([command(&format!("{launcher} hook-session-start"))]));
    hooks.insert("SessionEnd".into(), json!([command(&format!("{launcher} hook-session-end"))]));
    hooks.insert(
        "UserPromptSubmit".into(),
        json!([{ "hooks": [
            { "type": "command", "command": events },
            { "type": "command", "command": hook("snapshot-worktree.sh") }
        ] }]),
    );
    for event in ["PermissionRequest", "Stop", "StopFailure", "PostToolUseFailure"] {
        hooks.insert(event.into(), json!([command(&events)]));
    }
    hooks.insert(
        "PostToolUse".into(),
        json!([command(&events), { "matcher": "ArtifactData", "hooks": [{ "type": "command", "command": hook("record-board-write.sh") }] }]),
    );
    let snippet = serde_json::to_string_pretty(&json!({ "hooks": Value::Object(hooks) })).expect("serialisable");
    done(format!(
        "# Merge into the \"hooks\" key of ~/.claude/settings.json (qm never writes it).\n# The SessionStart/SessionEnd entries replace the existing qm ones - kept side by\n# side, every session would be captured and injected twice.\n{snippet}\n"
    ))
}
