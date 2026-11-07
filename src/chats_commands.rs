//! The commands that ask the running GUI to act on a chat: `rotate`, `caffeinate`, `pane-start`
//! and `pane-resume`. Nothing here types into a chat or ends one — the GUI's job does, after the
//! user's own command — so the rule that rotation is never automatic is untouched: these only run
//! when someone (or the `/rotate` skill, user-invoked) types them.

use std::process::{Command, Stdio};

use serde_json::{json, Value};

use crate::gui_action::{self, truthy, Failed};
use crate::outcome::{abort, done, Outcome};
use crate::session_log::SessionLog;
use crate::work_item::WorkItem;

pub fn run(command: &str, args: &[String]) -> Option<Outcome> {
    Some(match command {
        "rotate" => rotate(args),
        "caffeinate" => caffeinate(args),
        "pane-start" | "pane-resume" => pane(command, args),
        _ => return None,
    })
}

fn failed(f: Failed) -> Outcome {
    match f {
        Failed::Abort(message) => abort(&message),
        Failed::Ruby => Outcome::Delegate,
    }
}

fn caffeinate(a: &[String]) -> Outcome {
    let mode = a.first().map(String::as_str).unwrap_or("status");
    if !["on", "off", "auto", "status"].contains(&mode) {
        return abort("usage: qm caffeinate on|off|auto|status");
    }
    let reply = match gui_action::call("caffeinate", "caffeinate", None, json!({"mode": mode})) {
        Ok(r) => r,
        Err(f) => return failed(f),
    };
    let Some(Value::Object(status)) = reply.get("caffeinate") else { return abort("qm caffeinate: the qm gui did not answer like a qm gui") };
    let held = if truthy(status.get("on")) { format!("pid {}", status.get("pid").map(|p| p.as_str().map(String::from).unwrap_or_else(|| p.to_string())).unwrap_or_default().replace("null", "")) } else { "not held".to_string() };
    done(match status.get("mode").and_then(Value::as_str) {
        Some("auto") => format!("caffeinate auto ({held} — held while any session is working, until the GUI exits)\n"),
        Some("on") => format!("caffeinate on ({held}, until the GUI exits)\n"),
        _ => "caffeinate off\n".to_string(),
    })
}

fn pane(cmd: &str, a: &[String]) -> Outcome {
    let usage = || abort(if cmd == "pane-start" { "usage: qm pane-start SLUG" } else { "usage: qm pane-resume SLUG [--session ID]" });
    let mut rest: Vec<&String> = vec![];
    let mut session: Option<String> = None;
    let mut i = 0;
    while i < a.len() {
        match a[i].as_str() {
            "--session" if cmd == "pane-resume" => {
                let Some(v) = a.get(i + 1) else { return Outcome::Delegate };
                session = Some(v.clone());
                i += 2;
            }
            s if s.starts_with('-') && s != "-" => return Outcome::Delegate,
            _ => {
                rest.push(&a[i]);
                i += 1;
            }
        }
    }
    let Some(slug) = rest.first() else { return usage() };
    let args = match &session {
        Some(s) => json!({"session": s}),
        None => json!({}),
    };
    let reply = match gui_action::call(cmd, cmd, Some(slug), args) {
        Ok(r) => r,
        Err(f) => return failed(f),
    };
    done(if truthy(reply.get("attached")) {
        format!("{slug}: its chat is already running in the qm gui\n")
    } else if reply.get("mode").and_then(Value::as_str) == Some("resume") {
        format!("{slug}: resumed in its qm gui pane\n")
    } else {
        format!("{slug}: fresh chat started in its qm gui pane\n")
    })
}

/// `Quartermaster.process_lineage`: this process and its parents, up to 16, through `ps`.
pub fn process_lineage(mut pid: i64) -> Vec<i64> {
    let mut pids = vec![];
    while pid > 1 && pids.len() < 16 {
        pids.push(pid);
        let Ok(o) = Command::new("ps").args(["-o", "ppid=", "-p", &pid.to_string()]).stdin(Stdio::null()).output() else { break };
        if !o.status.success() {
            break;
        }
        pid = crate::ruby_text::strip(&String::from_utf8_lossy(&o.stdout)).split(|c: char| !c.is_ascii_digit()).next().and_then(|d| d.parse().ok()).unwrap_or(0);
    }
    pids
}

/// The item and session of the chat running this command: the live session whose claude process is
/// an ancestor, else the `QM_ITEM` it was launched with. An aside is never the item's chat.
fn calling_chat() -> Result<(String, Option<String>), Outcome> {
    if std::env::var("QM_ASIDE").is_ok_and(|v| !v.is_empty()) {
        return Err(abort("qm rotate --self: an aside can't rotate — only the item's own chat can"));
    }
    let pids = process_lineage(std::process::id() as i64);
    let log = SessionLog::read();
    let session = log.sessions().into_iter().find(|s| s.live && s.pid().is_some_and(|p| pids.contains(&p)));
    let items = match WorkItem::all() {
        Ok(i) => i,
        Err(e) => return Err(abort(&format!("qm: {}", e.message))),
    };
    let from_session = session.as_ref().and_then(|s| items.iter().find(|i| i.session_ids().iter().any(|id| *id == s.id)).map(|i| i.slug.clone()));
    let slug = from_session.unwrap_or_else(|| std::env::var("QM_ITEM").unwrap_or_default());
    if slug.is_empty() {
        return Err(abort("qm rotate --self: run it from a work item's chat in the qm gui — this process belongs to no item's chat"));
    }
    Ok((slug, session.map(|s| s.id)))
}

fn rotate(a: &[String]) -> Outcome {
    let (mut close, mut by_chat, mut rest) = (false, false, vec![]);
    for arg in a {
        match arg.as_str() {
            "--close" => close = true,
            "--self" => by_chat = true,
            s if s.starts_with('-') && s != "-" => return Outcome::Delegate, // OptionParser reads these its own way
            s => rest.push(s.to_string()),
        }
    }
    let slug = if by_chat { None } else { rest.first().cloned() };
    if !by_chat && slug.is_none() {
        return abort("usage: qm rotate SLUG [--close] | qm rotate --self [--close]");
    }
    let mut args = json!({"close": close});
    let slug = if by_chat {
        let (slug, session) = match calling_chat() {
            Ok(c) => c,
            Err(o) => return o,
        };
        args["self"] = json!(true);
        args["session"] = session.map_or(Value::Null, Value::from);
        slug
    } else {
        slug.unwrap()
    };
    let reply = match gui_action::call("rotate", "rotate", Some(&slug), args) {
        Ok(r) => r,
        Err(f) => return failed(f),
    };
    done(if by_chat {
        format!("{slug}: {} after this turn ends — end the turn now\n", if close { "closing" } else { "rotating" })
    } else if reply.get("mode").and_then(Value::as_str) == Some("distill") {
        format!("{slug}: rotating — distilling the dead chat, then a fresh one starts in its pane\n")
    } else {
        format!("{slug}: {} started — the chat wraps up in its qm gui pane\n", if close { "wrap up & close" } else { "rotation" })
    })
}

/// `Aside.session_name`: the tmux session of an item's aside.
pub fn aside_name(slug: &str) -> String {
    format!("{slug}--aside")
}
