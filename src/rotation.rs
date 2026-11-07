//! A context rotation (`Rotation`): hand an item's chat over to a fresh session that starts from the work
//! item instead of a bloated or cold context. Never automatic — only the user's click starts one.
//!
//! A live chat in its pane hands over itself: the wrap-up prompt is typed into it, the turn's Stop is
//! awaited, `/exit` ends it, the rotation is recorded and a fresh QM_ITEM session starts in the same tmux
//! window. A dead chat gets the Haiku distill of its latest transcript instead. "Close" is the live
//! hand-off without the fresh start; "finish" is that close with another prompt and the item resolved; a
//! self-rotation (`qm rotate --self`) waits for the chat's first Stop after the request.

use serde_json::{json, Value};

use crate::chat_attention::ChatAttention;
use crate::session_log::{Session, SessionLog};
use crate::tmux::Tmux;
use crate::work_item::WorkItem;

const WRAP_UP_PROMPT: &str = "Wrap up for a context rotation: record any decisions and open items not yet in the work item with qm note / qm retag, then reply DONE.";
const FINISH_PROMPT: &str = "This work item is done: record any remaining decisions with qm note, retag open notes that are handled with qm retag, then reply DONE.";
const EXIT_COMMAND: &str = "/exit";
const STOP_TIMEOUT: f64 = 20.0 * 60.0;
const EXIT_TIMEOUT: f64 = 60.0;
const BACKGROUND_TIMEOUT: f64 = 60.0 * 60.0;
const WAITING_LABEL: &str = "rotating: waiting for background work…";

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    WrapUp,
    SelfRotate,
    Distill,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::WrapUp => "wrap_up",
            Mode::SelfRotate => "self",
            Mode::Distill => "distill",
        }
    }
}

pub struct Plan {
    pub mode: Mode,
    pub session: Session,
    /// `None` for a distill, as the Ruby plan leaves it nil.
    pub close: Option<bool>,
    pub finish: bool,
    pub requested_at: f64,
}

/// `Err(Arg)` is the `ArgumentError` Ruby raises; `Unported` a value only Ruby reads exactly.
pub enum Fail {
    Arg(String),
    Unported,
}

type R<T> = Result<T, Fail>;

fn arg<T>(m: impl Into<String>) -> R<T> {
    Err(Fail::Arg(m.into()))
}

fn iso(secs: f64) -> String {
    chrono::DateTime::from_timestamp(secs.floor() as i64, 0).map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)).unwrap_or_default()
}

fn turn_of(session_id: &str) -> Option<Value> {
    crate::event_log::turns().get(session_id).cloned()
}

fn field(turn: &Value, key: &str) -> String {
    match turn.get(key) {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// The cheap, synchronous half: which kind of rotation this item gets, or why it can't rotate now.
pub fn plan(item: &WorkItem, close: bool, finish: bool, by_chat: bool, session_id: Option<&str>) -> R<Plan> {
    let close = close || finish;
    let tmux = Tmux::new();
    if by_chat {
        let mut p = self_plan(item, close, session_id)?;
        p.finish = finish;
        return Ok(p);
    }
    let log = SessionLog::read();
    let sessions = log.item_sessions(item).map_err(|_| Fail::Unported)?;
    let live: Vec<&Session> = sessions.iter().filter(|s| s.live).collect();
    if tmux.live(&item.slug) {
        let Some(session) = live.first() else { return arg(format!("{}'s chat has no captured session yet — rotate once it has answered", item.slug)) };
        let turn = ChatAttention::new().turn_for(session);
        if turn.state == "working" || turn.state == "blocked" {
            return arg(format!("{}'s chat is mid-turn ({}) — rotate once it is your turn", item.slug, turn.state));
        }
        if turn.state == "waiting" {
            let on: Vec<String> = turn.waiting_on.unwrap_or_default().into_iter().map(Option::unwrap_or_default).collect();
            return arg(format!("{}'s chat is waiting on background work ({}) — rotate once it has reported back", item.slug, on.join(", ")));
        }
        return Ok(Plan { mode: Mode::WrapUp, session: (*session).clone(), close: Some(close), finish, requested_at: 0.0 });
    }
    if !live.is_empty() {
        return arg(format!("{}'s chat runs outside the GUI — end it there first", item.slug));
    }
    if close {
        return arg(format!("{} has no chat open to wrap up", item.slug));
    }
    let Some(latest) = sessions.iter().find(|s| s.captured) else { return arg(format!("{} has no session to rotate from — just Start one", item.slug)) };
    Ok(Plan { mode: Mode::Distill, session: latest.clone(), close: None, finish: false, requested_at: 0.0 })
}

fn in_pane(session: &Session, pane: Option<i64>) -> bool {
    let (Some(pane), Some(pid)) = (pane, session.pid()) else { return true };
    crate::chats_commands::process_lineage(pid).contains(&pane)
}

fn self_plan(item: &WorkItem, close: bool, session_id: Option<&str>) -> R<Plan> {
    let tmux = Tmux::new();
    let slug = &item.slug;
    if !tmux.live(slug) {
        return arg(format!("{slug}'s chat isn't open in the qm gui — qm rotate --self only works from a GUI chat"));
    }
    let sessions = SessionLog::read().item_sessions(item).map_err(|_| Fail::Unported)?;
    let live: Vec<&Session> = sessions.iter().filter(|s| s.live).collect();
    let pane = tmux.pane_pid(slug);
    let session = match session_id {
        Some(id) => live.iter().find(|s| s.id == id).copied(),
        None => live.iter().find(|s| pane.is_some() && s.pid().is_some() && in_pane(s, pane)).or(live.first()).copied(),
    };
    if let (Some(id), None) = (session_id, session) {
        return arg(format!("session {id} is not {slug}'s live chat — nothing to rotate"));
    }
    let Some(session) = session else { return arg(format!("{slug}'s chat has no captured session yet — nothing to rotate")) };
    if !in_pane(session, pane) {
        return arg(format!("{} runs outside {slug}'s qm gui pane — qm rotate --self only rotates the pane's own chat", session.id));
    }
    Ok(Plan { mode: Mode::SelfRotate, session: session.clone(), close: Some(close), finish: false, requested_at: crate::clock::now_f64() })
}

fn type_into(slug: &str, text: &str) -> R<()> {
    let tmux = Tmux::new();
    if !tmux.send_text(slug, text) {
        return arg(format!("could not type into {slug}'s chat"));
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    if !tmux.send_enter(slug) {
        return arg(format!("could not type into {slug}'s chat"));
    }
    Ok(())
}

fn wait_for(timeout: f64, failure: String, mut ready: impl FnMut() -> R<bool>) -> R<()> {
    let deadline = crate::clock::now_f64() + timeout;
    while !ready()? {
        if crate::clock::now_f64() >= deadline {
            return arg(failure);
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    Ok(())
}

/// The wrap-up's own turn ended: a prompt submitted since it was typed, then a Stop after that prompt.
fn stopped_since(session_id: &str, sent_at: f64) -> bool {
    let Some(turn) = turn_of(session_id) else { return false };
    if turn.get("state") != Some(&json!("your_turn")) {
        return false;
    }
    let (floor, prompt) = (iso(sent_at), field(&turn, "prompt_ts"));
    prompt >= floor && field(&turn, "stop_ts") >= prompt
}

/// The requesting turn's Stop: the chat's turn ended at or after the request, and after its latest prompt.
fn stopped_after(session_id: &str, requested_at: f64) -> bool {
    let Some(turn) = turn_of(session_id) else { return false };
    if turn.get("state") != Some(&json!("your_turn")) {
        return false;
    }
    let stop = field(&turn, "stop_ts");
    stop >= iso(requested_at) && stop >= field(&turn, "prompt_ts")
}

fn ended_after(session_id: &str, since: Option<f64>, stop_ts: &str) -> bool {
    let stop = turn_of(session_id).map(|t| field(&t, "stop_ts")).unwrap_or_default();
    stop.as_str() > stop_ts || since.is_some_and(|s| iso(s).as_str() > stop_ts)
}

/// After a Stop: while the chat waits on background work, the work's notification wakes it for one more
/// turn. Settled once nothing is out and a turn ended after the last one seen waiting.
fn await_background(slug: &str, session: &Session, relabel: &dyn Fn(&str)) -> R<()> {
    let mut waited: Option<String> = None;
    let failure = format!("{slug}'s background work did not finish within {} min — the chat is left as is", (BACKGROUND_TIMEOUT / 60.0) as i64);
    wait_for(BACKGROUND_TIMEOUT, failure, || {
        if !Tmux::new().live(slug) {
            return arg(format!("{slug}'s chat ended while its rotation waited on background work"));
        }
        let turn = ChatAttention::new().turn_for(session);
        if turn.state == "waiting" {
            if waited.is_none() {
                relabel(WAITING_LABEL);
            }
            waited = Some(turn_of(&session.id).map(|t| field(&t, "stop_ts")).unwrap_or_default());
            return Ok(false);
        }
        Ok((turn.state == "your_turn" || turn.state == "none") && waited.as_deref().is_none_or(|w| ended_after(&session.id, turn.since, w)))
    })
}

fn hand_over(item: &mut WorkItem, session: &Session, via: &str, close: bool) -> R<Value> {
    let tmux = Tmux::new();
    let slug = item.slug.clone();
    tmux.keep_on_exit(&slug, true);
    type_into(&slug, EXIT_COMMAND)?;
    if let Err(e) = wait_for(EXIT_TIMEOUT, format!("{slug}'s chat did not exit after the wrap-up"), || Ok(tmux.pane_dead(&slug) || !tmux.live(&slug))) {
        tmux.keep_on_exit(&slug, false);
        return Err(e);
    }
    let rotation = item.record_rotation(&session.id, via, crate::clock::now_f64()).map_err(|_| Fail::Unported)?;
    if close {
        tmux.kill(&slug);
    } else {
        start_fresh(item)?;
    }
    Ok(rotation)
}

/// In the window the old chat left behind when there is one (the pane stays attached), else a new tmux session.
fn start_fresh(item: &WorkItem) -> R<()> {
    let Some(plan) = crate::launcher::fresh_chat_plan(item) else { return Err(Fail::Unported) };
    let tmux = Tmux::new();
    let env: Vec<(String, String)> = plan.env.clone();
    if tmux.live(&item.slug) {
        if let Err(m) = tmux.respawn(&item.slug, &plan.cwd, &plan.argv, &env) {
            // a kept window with a dead pane would block Start ("already live"): close it
            tmux.kill(&item.slug);
            return arg(m);
        }
        tmux.keep_on_exit(&item.slug, false);
        return Ok(());
    }
    match tmux.open(&item.slug, &plan.cwd, &plan.argv, &env) {
        Ok(_) => Ok(()),
        Err(m) => arg(m),
    }
}

/// Runs the plan; a GUI job result. `relabel` is called with a new job label while the hand-off waits on
/// background work.
pub fn run(item: &mut WorkItem, plan: &Plan, relabel: &dyn Fn(&str)) -> R<Value> {
    let slug = item.slug.clone();
    let rotation = match plan.mode {
        Mode::WrapUp => {
            let sent_at = crate::clock::now_f64();
            type_into(&slug, if plan.finish { FINISH_PROMPT } else { WRAP_UP_PROMPT })?;
            wait_for(STOP_TIMEOUT, format!("no Stop from {slug}'s wrap-up within {} min — the chat is left as is", (STOP_TIMEOUT / 60.0) as i64), || Ok(stopped_since(&plan.session.id, sent_at)))?;
            await_background(&slug, &plan.session, relabel)?;
            hand_over(item, &plan.session, "wrap-up", plan.close == Some(true))?
        }
        Mode::SelfRotate => {
            wait_for(STOP_TIMEOUT, format!("no Stop from {slug}'s chat within {} min of its rotate request — the chat is left as is", (STOP_TIMEOUT / 60.0) as i64), || Ok(stopped_after(&plan.session.id, plan.requested_at)))?;
            await_background(&slug, &plan.session, relabel)?;
            hand_over(item, &plan.session, "self", plan.close == Some(true))?
        }
        Mode::Distill => {
            if plan.session.transcript().is_some() && !item.snapshot_for(&plan.session.id) {
                match crate::distill::session(item, &plan.session) {
                    Ok(_) => {}
                    Err(crate::distill::Fail::Arg(m)) | Err(crate::distill::Fail::Already(m)) => return arg(m),
                    Err(crate::distill::Fail::Unported) => return Err(Fail::Unported),
                }
            }
            let rotation = item.record_rotation(&plan.session.id, "distill", crate::clock::now_f64()).map_err(|_| Fail::Unported)?;
            start_fresh(item)?;
            rotation
        }
    };
    if plan.finish {
        return resolve(&slug);
    }
    let done = if plan.close == Some(true) { "wrapped up and closed" } else { "rotated" };
    Ok(json!({"ok": true, "message": format!("{slug}: {done} — carried {} decisions, {} open", rotation["decisions"], rotation["open"])}))
}

/// Re-read: the wrap-up wrote notes after the item was loaded.
fn resolve(slug: &str) -> R<Value> {
    let mut item = WorkItem::find(slug).map_err(|_| Fail::Unported)?;
    item.update_state("resolved").map_err(|e| Fail::Arg(e.message))?;
    let open = item.notes().iter().filter(|n| n.is_open()).count();
    let tail = match open {
        0 => String::new(),
        1 => " — 1 open note remains".to_string(),
        n => format!(" — {n} open notes remain"),
    };
    Ok(json!({"ok": true, "message": format!("{slug}: resolved{tail}")}))
}

pub fn label(plan: &Plan) -> &'static str {
    if plan.mode == Mode::SelfRotate {
        return if plan.close == Some(true) { "closing after this turn…" } else { "rotating after this turn…" };
    }
    if plan.finish {
        "finishing: final wrap-up…"
    } else if plan.close == Some(true) {
        "wrapping up to close…"
    } else if plan.mode == Mode::WrapUp {
        "rotating: wrapping up…"
    } else {
        "rotating: distilling…"
    }
}
