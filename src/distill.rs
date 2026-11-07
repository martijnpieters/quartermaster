//! Preserving surviving transcripts as work-item snapshots before Claude Code prunes them (`distill`
//! in `Quartermaster`): which sessions a distill covers, and one headless haiku call per session.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

use crate::attention::Attention;
use crate::ruby_text::strip;
use crate::session_log::{Session, SessionLog};
use crate::work_item::WorkItem;

const DISTILL_PROMPT: &str = "You are distilling a Claude Code session transcript into a durable context\nsnapshot. The original transcript will be deleted; this snapshot is what a\nfuture session gets to continue the work. Write, in under 300 words:\nwhat the goal was, what was done and decided (with key files, commands,\nand gotchas), the state things were left in, and the concrete next steps.\nPlain markdown, no heading, no preamble.\n";
const TRANSCRIPT_TAIL_BYTES: usize = 400_000;
const TRANSCRIPT_MIN_TAIL_BYTES: usize = 50_000;

pub enum Fail {
    /// The `ArgumentError` Ruby raises.
    Arg(String),
    /// `AlreadyDistilledError`: the page turns it into a confirmation.
    Already(String),
    Unported,
}

type R<T> = Result<T, Fail>;

fn guard_already_distilled(item: &WorkItem, session_id: &str, force: bool) -> R<()> {
    if force || !item.snapshot_for(session_id) {
        return Ok(());
    }
    Err(Fail::Already(format!("session {} of '{}' is already distilled — re-run with --force to replace its snapshot", session_id.chars().take(8).collect::<String>(), item.slug)))
}

/// Which sessions a distill covers: an explicit one exactly; else every session carrying a degradation
/// warning; else the newest resumable one.
pub fn targets(item: &WorkItem, log: &SessionLog, session_id: Option<&str>, force: bool) -> R<Vec<Session>> {
    if let Some(id) = session_id {
        let session = item.session_ids().iter().any(|s| s == id).then(|| log.session_for(id)).flatten();
        let Some(session) = session.filter(|s| s.tier() == "resumable") else {
            return Err(Fail::Arg(format!("session {id} of '{}' has no surviving transcript to distill", item.slug)));
        };
        guard_already_distilled(item, id, force)?;
        return Ok(vec![session]);
    }
    let attention = Attention::new(std::time::SystemTime::now());
    let expiring: Vec<Session> = attention.degradations(item).into_iter().filter_map(|e| e.session_id).filter_map(|id| log.session_for(&id)).filter(|s| s.tier() == "resumable").collect();
    if !expiring.is_empty() {
        return Ok(expiring);
    }
    let mut latest: Option<Session> = None;
    for session in item.session_ids().iter().filter_map(|id| log.session_for(id)).filter(|s| s.tier() == "resumable") {
        if latest.as_ref().is_none_or(|l| session.last_ts() > l.last_ts()) {
            latest = Some(session);
        }
    }
    let Some(latest) = latest else { return Err(Fail::Arg(format!("no session of '{}' has a surviving transcript to distill", item.slug))) };
    guard_already_distilled(item, &latest.id, force)?;
    Ok(vec![latest])
}

/// The cheap half: the item and its target sessions, everything the caller must surface before any run.
pub fn plan(slug: &str, session_id: Option<&str>, force: bool) -> R<(WorkItem, Vec<Session>)> {
    let item = WorkItem::find(slug).map_err(|e| if e.class == "ArgumentError" { Fail::Arg(e.message) } else { Fail::Unported })?;
    let sessions = targets(&item, &SessionLog::read(), session_id, force)?;
    Ok((item, sessions))
}

/// One headless haiku call over the session's transcript tail; the snapshot it answers is added to the item.
pub fn session(item: &mut WorkItem, session: &Session) -> R<Value> {
    let path = session.transcript().ok_or(Fail::Unported)?;
    let raw = std::fs::read(&path).map_err(|_| Fail::Unported)?;
    let mut tail = TRANSCRIPT_TAIL_BYTES;
    loop {
        let mut transcript: Vec<u8> = raw.clone();
        if raw.len() > tail {
            transcript = format!("(transcript truncated — only the tail follows)\n").into_bytes();
            transcript.extend_from_slice(&raw[raw.len() - tail..]);
        }
        let mut child = Command::new(crate::command_runner::resolve_program("claude")).args(["-p", DISTILL_PROMPT, "--model", "haiku"]).env("QM_SKIP", "1").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|_| Fail::Unported)?;
        if let Some(mut input) = child.stdin.take() {
            let _ = input.write_all(&transcript);
        }
        let result = child.wait_with_output().map_err(|_| Fail::Unported)?;
        let (out, err) = (String::from_utf8_lossy(&result.stdout).into_owned(), String::from_utf8_lossy(&result.stderr).into_owned());
        if result.status.success() && !strip(&out).is_empty() {
            item.add_snapshot(&session.id, strip(&out)).map_err(|_| Fail::Unported)?;
            return Ok(json!({"session_id": session.id, "transcript_bytes": transcript.len(), "snapshot": strip(&out)}));
        }
        if out.to_lowercase().contains("prompt is too long") && tail > TRANSCRIPT_MIN_TAIL_BYTES {
            tail /= 2;
            continue;
        }
        let shown = if strip(&err).is_empty() { strip(&out) } else { strip(&err) };
        return Err(Fail::Arg(format!("claude -p failed: {shown}")));
    }
}
