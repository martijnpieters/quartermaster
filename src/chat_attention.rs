//! Whose move it is in each item's chat — the chat view's left list. Derived from the hook events
//! (`event_log`) plus session liveness, with the transcript filling in what no hook reports (Esc
//! interrupts, context size).
//!
//! blocked: a permission prompt or question is pending. your_turn: the turn ended (Stop or Esc) and
//! stays loud until the user types or snoozes it. working: Claude is mid-turn. waiting: the turn
//! ended with background work still out (watchers don't count). none: no live chat, or a live one
//! that has had no prompt yet.

use std::collections::HashMap;

use serde_json::{Map, Value};

use crate::background_jobs;
use crate::clock;
use crate::error::{QmError, Res};
use crate::event_log;
use crate::session_log::{Session, SessionLog};
use crate::transcript_vitals::{self, Vitals};
use crate::work_item::WorkItem;

pub const STATES: [&str; 5] = ["blocked", "your_turn", "working", "waiting", "none"];

#[derive(Clone)]
pub struct Turn {
    pub state: String,
    pub since: Option<f64>,
    pub tool: Option<Value>,
    pub waiting_on: Option<Vec<Option<String>>>,
}

impl Turn {
    fn none() -> Turn {
        Turn { state: "none".into(), since: None, tool: None, waiting_on: None }
    }
}

pub struct Entry {
    pub slug: String,
    pub session_id: Option<String>,
    pub state: String,
    pub since: Option<f64>,
    pub tool: Option<Value>,
    pub snoozed: bool,
    pub waiting_on: Option<Vec<Option<String>>>,
    pub context_tokens: Option<i64>,
    pub cache_warm_until: Option<f64>,
    pub last_assistant_at: Option<f64>,
    pub activity: Option<f64>,
}

fn state_index(state: &str) -> f64 {
    STATES.iter().position(|s| *s == state).map(|i| i as f64).unwrap_or(f64::NAN)
}

impl Entry {
    /// A snoozed your-turn sinks below working, above chats with no turn at all.
    fn rank(&self) -> f64 {
        if self.snoozed { 2.5 } else { state_index(&self.state) }
    }

    /// Longest-waiting first within a state; no-turn rows by recent activity.
    fn sort_key(&self) -> (f64, f64) {
        (self.rank(), self.since.unwrap_or(-self.activity.unwrap_or(0.0)))
    }
}

/// `Time.parse(ts)` of a turn's stored stamp, nil when absent or unreadable.
fn time(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_str).and_then(clock::parse_f64)
}

fn truthy(v: Option<&Value>) -> Option<&Value> {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => None,
        some => some,
    }
}

pub struct ChatAttention {
    sessions: Vec<Session>,
    turns: Option<Map<String, Value>>,
    vitals: HashMap<String, Option<Vitals>>,
}

impl ChatAttention {
    pub fn new() -> ChatAttention {
        ChatAttention { sessions: SessionLog::read().sessions(), turns: None, vitals: HashMap::new() }
    }

    fn live_sessions(&self, item: &WorkItem) -> Vec<Session> {
        item.session_ids().iter().filter_map(|id| self.sessions.iter().find(|s| s.id == *id)).filter(|s| s.live).cloned().collect()
    }

    /// In-progress items plus anything with a live chat, by attention.
    pub fn entries(&mut self) -> Res<Vec<Entry>> {
        let mut out = vec![];
        for item in WorkItem::all()? {
            let live = self.live_sessions(&item);
            if item.state().as_deref() != Some("in_progress") && live.is_empty() {
                continue;
            }
            out.push(self.entry_for(&item, live)?);
        }
        out.sort_by(|a, b| {
            let (x, y) = (a.sort_key(), b.sort_key());
            x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal).then(x.1.partial_cmp(&y.1).unwrap_or(std::cmp::Ordering::Equal))
        });
        Ok(out)
    }

    /// `entry_for(item).sort_key`, the chat view's attention order.
    pub fn sort_key_for(&mut self, item: &WorkItem) -> Res<(f64, f64)> {
        let live = self.live_sessions(item);
        Ok(self.entry_for(item, live)?.sort_key())
    }

    fn vitals_for(&mut self, session: &Session) -> Option<&Vitals> {
        if !self.vitals.contains_key(&session.id) {
            let v = transcript_vitals::read(session.transcript().as_deref());
            self.vitals.insert(session.id.clone(), v);
        }
        self.vitals[&session.id].as_ref()
    }

    pub fn entry_for(&mut self, item: &WorkItem, live: Vec<Session>) -> Res<Entry> {
        let mut turns: Vec<(Session, Turn)> = vec![];
        for s in live {
            let t = self.turn_for(&s);
            turns.push((s, t));
        }
        let mut best: Option<usize> = None;
        for (i, (_, t)) in turns.iter().enumerate() {
            let key = (state_index(&t.state), t.since.unwrap_or(0.0));
            if best.is_none_or(|b| {
                let bk = (state_index(&turns[b].1.state), turns[b].1.since.unwrap_or(0.0));
                key.0 < bk.0 || (key.0 == bk.0 && key.1 < bk.1)
            }) {
                best = Some(i);
            }
        }
        let (session, turn) = match best {
            Some(i) => {
                let (s, t) = turns.swap_remove(i);
                (Some(s), t)
            }
            None => (self.sessions_of(item)?.into_iter().next(), Turn::none()),
        };
        // a chat handed over without a fresh start (wrap up & close) leaves no context to show
        let vitals = match &session {
            Some(s) if !item.rotated_from(&s.id) => self.vitals_for(s),
            _ => None,
        };
        let (context_tokens, cache_warm_until, last_assistant_at) = vitals.map(|v| (v.context_tokens, v.cache_warm_until, v.last_assistant_at)).unwrap_or((None, None, None));
        let snoozed = turn.state == "your_turn" && item.turn_snoozed(turn.since);
        let activity = match session.as_ref().and_then(|s| s.last_ts_opt()) {
            Some(ts) => Some(clock::parse_f64(&ts).ok_or_else(|| QmError::argument(format!("no time information in {ts:?}")))?),
            None => None,
        };
        Ok(Entry {
            slug: item.slug.clone(),
            session_id: session.map(|s| s.id),
            state: turn.state,
            since: turn.since,
            tool: turn.tool,
            snoozed,
            waiting_on: turn.waiting_on,
            context_tokens,
            cache_warm_until,
            last_assistant_at,
            activity,
        })
    }

    fn sessions_of(&self, item: &WorkItem) -> Res<Vec<Session>> {
        let log = SessionLog::read();
        log.item_sessions(item)
    }

    /// One live session's state. A turn still working or blocked after a transcript interrupt newer
    /// than its last Stop and prompt was ended by Esc, which no hook reports.
    pub fn turn_for(&mut self, session: &Session) -> Turn {
        let turns = self.turns.get_or_insert_with(event_log::turns);
        let Some(raw) = turns.get(&session.id).and_then(Value::as_object).cloned() else { return Turn::none() };
        let Some(state) = truthy(raw.get("state")).and_then(Value::as_str).map(String::from) else { return Turn::none() };
        let turn = Turn { state, since: time(raw.get("since")), tool: truthy(raw.get("tool")).cloned(), waiting_on: None };
        if turn.state == "your_turn" {
            return delegated(session, turn);
        }
        let interrupted = self.vitals_for(session).and_then(|v| v.interrupted_at);
        let settled = [time(raw.get("stop_ts")), time(raw.get("prompt_ts"))].into_iter().flatten().fold(None, |m: Option<f64>, t| Some(m.map_or(t, |x| x.max(t))));
        match interrupted {
            Some(at) if settled.is_none_or(|s| at > s) => Turn { state: "your_turn".into(), since: Some(at), tool: None, waiting_on: None },
            _ => turn,
        }
    }
}

impl Default for ChatAttention {
    fn default() -> Self {
        Self::new()
    }
}

/// A Stop with an agent, workflow or shell still running in the background: the chat isn't done,
/// only answerable. Not after Esc — that was the user's call.
fn delegated(session: &Session, turn: Turn) -> Turn {
    let jobs: Vec<background_jobs::Job> =
        background_jobs::pending(session.transcript().as_deref(), clock::now_f64()).into_iter().filter(|j| !j.watcher()).collect();
    if jobs.is_empty() {
        return turn;
    }
    let names = jobs.iter().map(|j| truthy(Some(&j.description)).map(|_| j.description_text())).collect();
    Turn { state: "waiting".into(), waiting_on: Some(names), ..turn }
}
