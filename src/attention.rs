//! What needs the user's attention: transcripts about to be pruned un-distilled, in-progress items
//! gone quiet, merged PRs on unresolved items.

use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Local};

use crate::clock;
use crate::data_root;
use crate::error::{QmError, Res};
use crate::pr_status::PrStatus;
use crate::session_log::{Session, SessionLog};
use crate::work_item::WorkItem;

const WARN_WINDOW_DAYS: i64 = 7;
const STALE_AFTER_DAYS: i64 = 10;
const DAY: f64 = 86_400.0;

pub struct Entry {
    pub slug: String,
    pub kind: &'static str,
    pub message: String,
    pub session_id: Option<String>,
}

pub struct Attention {
    sessions: Vec<Session>,
    pr_status: PrStatus,
    retention_days: f64,
    now: SystemTime,
}

fn secs(t: SystemTime) -> f64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64(),
        Err(e) => -e.duration().as_secs_f64(),
    }
}

/// How long Claude Code keeps a transcript: settings.json's `cleanupPeriodDays`, else 30.
fn transcript_retention_days() -> f64 {
    let path = std::path::Path::new(&data_root::home()).join(".claude/settings.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("cleanupPeriodDays").and_then(|d| d.as_f64()))
        .unwrap_or(30.0)
}

impl Attention {
    pub fn new(now: SystemTime) -> Attention {
        let _ = data_root::ensure_setup();
        Attention {
            sessions: SessionLog::read().sessions(),
            pr_status: PrStatus::new(secs(now) as i64),
            retention_days: transcript_retention_days(),
            now,
        }
    }

    fn session_for(&self, id: &str) -> Option<&Session> {
        self.sessions.iter().find(|s| s.id == id)
    }

    /// Entries for every unresolved item, in file order.
    pub fn entries(&self) -> Res<Vec<Entry>> {
        let mut out = vec![];
        for item in WorkItem::all()?.iter().filter(|i| i.state().as_deref() != Some("resolved")) {
            out.extend(self.entries_for(item)?);
        }
        Ok(out)
    }

    pub fn entries_for(&self, item: &WorkItem) -> Res<Vec<Entry>> {
        let mut out = self.degradations(item);
        out.extend(self.staleness(item)?);
        out.extend(self.pr_merged(item));
        Ok(out)
    }

    /// Acknowledges the item's degradation warnings, or one session's; returns the ids acked.
    pub fn acknowledge_all(&self, item: &mut WorkItem, session_id: Option<&str>) -> Res<Vec<String>> {
        let ids: Vec<String> = match session_id {
            Some(id) => vec![id.to_string()],
            None => self.degradations(item).into_iter().filter_map(|e| e.session_id).collect(),
        };
        for id in &ids {
            item.ack_degrade(id)?;
        }
        Ok(ids)
    }

    pub fn degradations(&self, item: &WorkItem) -> Vec<Entry> {
        let mut out = vec![];
        for (id, acked) in item.attached_sessions() {
            if acked || item.snapshot_for(&id) {
                continue;
            }
            let Some(session) = self.session_for(&id) else { continue };
            if session.live {
                continue;
            }
            let Some(transcript) = session.transcript() else { continue };
            let Ok(modified) = std::fs::metadata(&transcript).and_then(|m| m.modified()) else { continue };
            let days_left = ((secs(modified) + self.retention_days * DAY - secs(self.now)) / DAY).floor() as i64;
            if days_left > WARN_WINDOW_DAYS {
                continue;
            }
            out.push(Entry { slug: item.slug.clone(), kind: "degrade", message: format!("transcript expires in {days_left}d"), session_id: Some(session.id.clone()) });
        }
        out
    }

    fn staleness(&self, item: &WorkItem) -> Res<Option<Entry>> {
        if item.state().as_deref() != Some("in_progress") {
            return Ok(None);
        }
        let today = DateTime::<Local>::from(self.now).date_naive();
        if item.snoozed_at(today) {
            return Ok(None);
        }
        let days_quiet = ((secs(self.now) - self.activity_at(item)?) / DAY).floor() as i64;
        if days_quiet < STALE_AFTER_DAYS {
            return Ok(None);
        }
        Ok(Some(Entry { slug: item.slug.clone(), kind: "stale", message: format!("no activity for {days_quiet}d"), session_id: None }))
    }

    /// For a stack, a merged bottom PR means nothing yet: nudge only when every linked branch's PR
    /// is merged. A PR-less branch gone locally and from origin (an old name) is ignored.
    fn pr_merged(&self, item: &WorkItem) -> Option<Entry> {
        let branches = item.branches();
        let states: Vec<(String, Option<String>)> = branches.iter().map(|b| (b.clone(), self.pr_status.state_for(item, b))).collect();
        let states: Vec<&(String, Option<String>)> = states.iter().filter(|(b, s)| !(s.is_none() && self.branch_gone(item, b))).collect();
        if states.is_empty() || !states.iter().all(|(_, s)| s.as_deref() == Some("MERGED")) {
            return None;
        }
        let message = if states.len() > 1 { format!("all {} PRs merged — resolve?", states.len()) } else { "PR merged — resolve?".to_string() };
        Some(Entry { slug: item.slug.clone(), kind: "pr_merged", message, session_id: None })
    }

    fn branch_gone(&self, item: &WorkItem, branch: &str) -> bool {
        let Some(root) = self.pr_status.root_for(item.project().as_deref()) else { return false };
        ![format!("refs/heads/{branch}"), format!("refs/remotes/origin/{branch}")].iter().any(|r| {
            Command::new("git")
                .args(["-C", root, "rev-parse", "--verify", "--quiet", r])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
    }

    /// When the item last saw movement: the newest of its file and its sessions' last events.
    pub fn activity_at(&self, item: &WorkItem) -> Res<f64> {
        let mut newest = item.file_mtime().unwrap_or(0.0);
        for (id, _) in item.attached_sessions() {
            if let Some(session) = self.session_for(&id) {
                let ts = session.last_ts();
                if ts.is_empty() {
                    continue;
                }
                let parsed = clock::parse(&ts).ok_or_else(|| QmError::argument(format!("no time information in {ts:?}")))?;
                newest = newest.max(parsed.timestamp_millis() as f64 / 1000.0);
            }
        }
        Ok(newest)
    }
}
