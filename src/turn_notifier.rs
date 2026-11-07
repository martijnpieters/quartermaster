//! Tells the user when a chat needs them (`TurnNotifier`): a permission prompt or question (always, with
//! sound), or a turn that ran long enough that they have likely looked away. Never for an Esc interrupt
//! (no hook fires), a turn that delegated to background work or a snoozed turn. The watcher thread calls
//! `check` when `events.jsonl` grows; the channel depends on the page: a pulse younger than three seconds
//! means a visible tab, which shows the notification itself (`recent`, flagged "page"), else macOS shows one
//! through terminal-notifier or osascript.

use std::sync::Mutex;

use serde_json::{json, Map, Value};

use crate::command_runner;
use crate::data_root;
use crate::session_log::{Session, SessionLog};
use crate::work_item::WorkItem;

const LONG_TURN: f64 = 30.0;
const PAGE_FRESH: f64 = 3.0;
const KEEP: usize = 20;
const SOUND: &str = "Glass";
const NOTIFIER_DIRS: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];

struct Page {
    at: f64,
    focused: bool,
    chat: Option<String>,
}

struct State {
    seen: Option<Map<String, Value>>,
    page: Option<Page>,
    recent: Vec<Value>,
    seq: u64,
}

static STATE: Mutex<State> = Mutex::new(State { seen: None, page: None, recent: Vec::new(), seq: 0 });

fn lock() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|p| p.into_inner())
}

/// Bumped per notification raised, so the page's pulse moves with it.
pub fn seq() -> u64 {
    lock().seq
}

/// The newest notifications, oldest first, for the page to show the ones it hasn't yet.
pub fn recent() -> Vec<Value> {
    lock().recent.clone()
}

/// The page's pulse: it is visible, and whether it has focus on a chat.
pub fn page_seen(focused: bool, chat: Option<String>) {
    lock().page = Some(Page { at: crate::clock::now_f64(), focused, chat });
}

/// The page's tab was hidden: macOS shows notifications again.
pub fn page_hidden() {
    lock().page = None;
}

fn find_terminal_notifier() -> Option<String> {
    let mut dirs: Vec<String> = std::env::var("PATH").unwrap_or_default().split(':').map(String::from).collect();
    dirs.extend(NOTIFIER_DIRS.iter().map(|d| d.to_string()));
    dirs.iter().map(|d| std::path::Path::new(d).join("terminal-notifier")).find(|p| {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }).map(|p| p.to_string_lossy().into_owned())
}

fn text_of(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

enum Kind {
    Blocked,
    YourTurn,
}

fn duration_secs(turn: &Value) -> Option<f64> {
    let stop = crate::clock::parse_f64(&text_of(turn.get("stop_ts"))?)?;
    let prompt = crate::clock::parse_f64(&text_of(turn.get("prompt_ts"))?)?;
    Some(stop - prompt)
}

fn trigger(turn: &Value) -> Option<Kind> {
    match turn.get("state").and_then(Value::as_str) {
        Some("blocked") => Some(Kind::Blocked),
        Some("your_turn") if duration_secs(turn).is_some_and(|d| d >= LONG_TURN) => Some(Kind::YourTurn),
        _ => None,
    }
}

fn duration(turn: &Value) -> String {
    let seconds = duration_secs(turn).unwrap_or(0.0) as i64;
    if seconds < 3600 { format!("{}m {}s", seconds / 60, seconds % 60) } else { format!("{}h {}m", seconds / 3600, seconds % 3600 / 60) }
}

/// A snoozed turn, or one that only handed off to background work.
fn quiet_turn(session: Option<&Session>, item: Option<&WorkItem>, turn: &Value) -> bool {
    let since = text_of(turn.get("since")).and_then(|s| crate::clock::parse_f64(&s));
    if item.is_some_and(|i| i.turn_snoozed(since)) {
        return true;
    }
    let Some(session) = session else { return false };
    crate::chat_attention::ChatAttention::new().turn_for(session).state != "your_turn"
}

pub(crate) fn announce(title: &str, body: &str, group: &str, click: Option<Vec<String>>, sound: bool) {
    let Some(notifier) = find_terminal_notifier() else {
        let sound = if sound { format!(" sound name \"{SOUND}\"") } else { String::new() };
        command_runner::call(&["osascript", "-e", "on run argv", "-e", &format!("display notification (item 2 of argv) with title (item 1 of argv){sound}"), "-e", "end run", title, body]);
        return;
    };
    let mut argv: Vec<String> = vec![notifier, "-title".into(), title.into(), "-message".into(), body.into(), "-group".into(), group.into()];
    if sound {
        argv.extend(["-sound".to_string(), SOUND.to_string()]);
    }
    if let Some(words) = click {
        let me = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "qm".into());
        let command: Vec<String> = std::iter::once(me).chain(words).map(|w| crate::project_config::shell_escape(&w)).collect();
        argv.extend(["-execute".to_string(), command.join(" ")]);
    }
    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    command_runner::call(&refs);
}

fn deliver(notification: &mut Map<String, Value>) {
    let chat = notification.get("slug").filter(|v| !v.is_null()).or_else(|| notification.get("loose")).and_then(Value::as_str).map(String::from);
    let visible;
    {
        let mut st = lock();
        let now = crate::clock::now_f64();
        let page = st.page.as_ref().filter(|p| now - p.at < PAGE_FRESH);
        visible = page.is_some();
        let watching = page.is_some_and(|p| p.focused && chat.is_some() && p.chat == chat);
        notification.insert("page".into(), json!(visible && !watching));
        st.seq += 1;
        notification.insert("id".into(), json!(st.seq));
        st.recent.push(Value::Object(notification.clone()));
        let excess = st.recent.len().saturating_sub(KEEP);
        st.recent.drain(..excess);
    }
    if !visible {
        let group = format!("qm-{}", notification.get("slug").filter(|v| !v.is_null()).or_else(|| notification.get("session")).and_then(Value::as_str).unwrap_or(""));
        announce(
            notification["title"].as_str().unwrap_or(""),
            notification["body"].as_str().unwrap_or(""),
            &group,
            chat.map(|c| vec!["open-chat".to_string(), c]),
            notification["kind"] == "blocked",
        );
    }
}

fn raise_for(session_id: &str, turn: &Value, log: &SessionLog, items: &[WorkItem]) {
    let session = log.session_for(session_id);
    if session.as_ref().is_some_and(|s| s.aside_for().is_some()) {
        return;
    }
    let item = items.iter().find(|i| i.session_ids().iter().any(|s| s == session_id));
    let Some(kind) = trigger(turn) else { return };
    if matches!(kind, Kind::YourTurn) && quiet_turn(session.as_ref(), item, turn) {
        return;
    }
    let title = match item {
        Some(i) => i.title(),
        None => crate::gui_state::unlinked_title(session.as_ref(), "Unlinked session"),
    };
    let body = match kind {
        Kind::Blocked => match turn.get("tool").filter(|v| !matches!(v, Value::Null | Value::Bool(false))) {
            Some(Value::String(t)) => format!("Needs you: {t}"),
            Some(other) => format!("Needs you: {other}"),
            None => "Needs your answer".to_string(),
        },
        Kind::YourTurn => format!("Your turn — after {}", duration(turn)),
    };
    let loose = if item.is_none() { session.as_ref().and_then(|s| s.loose_id()).map(|l| format!("loose--{l}")) } else { None };
    let mut notification = Map::new();
    notification.insert("slug".into(), json!(item.map(|i| i.slug.clone())));
    notification.insert("loose".into(), json!(loose));
    notification.insert("session".into(), json!(session_id));
    notification.insert("kind".into(), json!(if matches!(kind, Kind::Blocked) { "blocked" } else { "your_turn" }));
    notification.insert("title".into(), json!(title));
    notification.insert("body".into(), json!(body));
    deliver(&mut notification);
}

/// The notifications raised by the turns that changed since the last check. The first check only takes
/// stock: what happened before the GUI started is not news.
pub fn check() {
    let turns = crate::event_log::turns();
    let now_seen: Map<String, Value> = turns.iter().map(|(id, t)| (id.clone(), json!([t.get("state").cloned().unwrap_or(Value::Null), t.get("since").cloned().unwrap_or(Value::Null)]))).collect();
    let previous = lock().seen.replace(now_seen.clone());
    let Some(previous) = previous else { return };
    let changed: Vec<(&String, &Value)> = turns.iter().filter(|(id, turn)| previous.get(*id) != now_seen.get(*id) && trigger(turn).is_some()).collect();
    if changed.is_empty() {
        return;
    }
    let log = SessionLog::read();
    let items = match WorkItem::all() {
        Ok(i) => i,
        Err(e) => {
            data_root::log_error("turn-notify", e.class, &e.message);
            return;
        }
    };
    for (id, turn) in changed {
        raise_for(id, turn, &log, &items);
    }
}
