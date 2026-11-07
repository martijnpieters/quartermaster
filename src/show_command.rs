//! `qm show SLUG`: the item with its stacks, headline note, warnings, sessions, usage and body.
//! Like the Ruby command it has side effects: it auto-links the item's stack branches, refreshes
//! the PR cache and saves the usage ledger.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::attention::Attention;
use crate::cli_format::{ljust, note_line, paint, prefix_chars, puts, short_path};
use crate::gh_stack;
use crate::item_usage::{dollars, human_tokens, ItemUsage};
use crate::outcome::{abort, done, Outcome};
use crate::pr_status::PrStatus;
use crate::price_table::PriceTable;
use crate::project_config::ProjectConfig;
use crate::session_log::SessionLog;
use crate::usage_ledger::UsageLedger;
use crate::work_item::WorkItem;

macro_rules! tri {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return abort(&format!("qm: {}", e.message)),
        }
    };
}

/// One gh-stack, trunk first: "stack: main → a (#1) → b (#2) ⟳".
fn stack_line(chain: &Value) -> String {
    let layers: Vec<String> = chain["layers"]
        .as_array()
        .map(|ls| ls.as_slice())
        .unwrap_or(&[])
        .iter()
        .map(|layer| {
            let mut parts = vec![layer["branch"].as_str().unwrap_or("").to_string()];
            match &layer["pr_number"] {
                Value::Null | Value::Bool(false) => {}
                Value::String(s) => parts.push(format!("(#{s})")),
                other => parts.push(format!("(#{other})")),
            }
            if layer["needsRestack"] == Value::Bool(true) {
                parts.push("⟳".into());
            }
            parts.join(" ")
        })
        .collect();
    let mut all = vec![chain["trunk"].as_str().unwrap_or("").to_string()];
    all.extend(layers);
    format!("stack: {}", all.join(" → "))
}

fn tier_badge(tier: &str) -> (&'static str, u8) {
    match tier {
        "live" => ("● live", 33),
        "resumable" => ("↻ resumable", 32),
        _ => ("✗ expired", 90),
    }
}

/// Newest first, matching what plain resume picks; sessions the log never captured sink to the bottom.
/// The rows the viewer's ^X list shows: tab-separated columns.
pub fn session_lines(item: &WorkItem, log: &SessionLog) -> Result<Vec<String>, crate::error::QmError> {
    Ok(log
        .item_sessions(item)?
        .iter()
        .map(|s| {
            if s.tier() == "uncaptured" {
                format!("{}\t{}\t-\t-", s.id, paint(&ljust("? uncaptured", 12), 90))
            } else {
                let (label, color) = tier_badge(s.tier());
                let when = s.last_ts_opt().map(|t| prefix_chars(&t, 16)).unwrap_or_default();
                format!("{}\t{}\t{}\t{}", s.id, paint(&ljust(label, 12), color), when, short_path(s.cwd().as_deref()))
            }
        })
        .collect())
}

/// Tokens and API-equivalent cost: the total, then a line per session. Nothing for an item without sessions.
fn usage_lines(item: &WorkItem, log: &SessionLog) -> Result<Vec<String>, crate::error::QmError> {
    let mut ledger = UsageLedger::new();
    let usage = ItemUsage::for_item(item, log, &mut ledger, &PriceTable::load())?;
    ledger.save();
    if usage.is_empty() {
        return Ok(vec![]);
    }
    let mut lines = vec![String::new(), format!("{}  {}", paint("Usage", 1), usage.line())];
    if usage.partial() {
        lines.push(paint(&format!("  partial: no price for {}", usage.unpriced().join(", ")), 33));
    }
    for row in &usage.rows {
        let mut bits = vec![prefix_chars(&row.id, 8)];
        if row.aside {
            bits.push("aside".into());
        }
        if row.known {
            bits.push(format!("{} tok", human_tokens(row.tokens)));
            bits.push(format!("~{}", dollars(row.cost)));
        } else {
            bits.push("expired, never tallied".into());
        }
        if row.agents > 0 {
            bits.push(format!("{} sub-agent{}", row.agents, if row.agents == 1 { "" } else { "s" }));
        }
        lines.push(format!("  {}", bits.join(" · ")));
    }
    Ok(lines)
}

pub fn show(a: &[String]) -> Outcome {
    let Some(slug) = a.first() else { return abort("usage: qm show SLUG") };
    let mut item = tri!(WorkItem::find(slug));
    gh_stack::sync(&mut item);
    let chains = gh_stack::chains(&item, &Map::new(), &Map::new());
    let stacks: Vec<&Value> = chains.iter().filter(|c| c["kind"] == "stack").collect();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let pr_status = PrStatus::new(now);
    let branch_bits: Vec<String> = item
        .branches()
        .iter()
        .map(|b| match pr_status.state_for(&item, b) {
            Some(state) => format!("{b} (PR {state})"),
            None => b.clone(),
        })
        .collect();
    let mut meta: Vec<String> = item.state().into_iter().chain(item.project()).collect();
    meta.extend(branch_bits);
    meta.extend(item.issue_number().map(|n| format!("issue #{n}")));
    meta.extend(item.snooze_until_raw().map(|s| format!("snoozed until {s}")));
    let config = ProjectConfig::current();
    if let Some(server) = config.settings(item.project().as_deref()).and_then(|s| s.dev_server.as_ref()) {
        let mode = item.pf_mode_override().unwrap_or_else(|| server.default_mode.clone());
        meta.push(format!("{} {mode}", server.command));
    }
    meta.push(format!("created {}", item.created().unwrap_or_default()));

    let mut out = String::new();
    out.push_str(&puts(&paint(&item.title(), 1)));
    out.push_str(&puts(&meta.join(" · ")));
    for chain in stacks {
        out.push_str(&puts(&stack_line(chain)));
    }
    if let Some(headline) = item.headline_note() {
        out.push_str(&puts(&note_line(&headline)));
    }
    for e in tri!(Attention::new(SystemTime::now()).entries_for(&item)) {
        out.push_str(&puts(&paint(&format!("⚠ {}", e.message), 33)));
    }
    let log = SessionLog::read();
    out.push('\n');
    out.push_str(&puts(&paint("Sessions", 1)));
    let lines = tri!(session_lines(&item, &log));
    if lines.is_empty() {
        out.push_str("(none)\n");
    } else {
        for l in lines {
            out.push_str(&puts(&l.replace('\t', " ")));
        }
    }
    for l in tri!(usage_lines(&item, &log)) {
        out.push_str(&puts(&l));
    }
    out.push('\n');
    out.push_str(&puts(crate::ruby_text::strip(&item.body_with_notes(|n| note_line(&n)))));
    done(out)
}
