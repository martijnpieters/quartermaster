//! The fzf viewer's read-side commands: the rows its lists show and the filter its `^F` cycles.
//! The viewer itself, its pickers (`choose-*`, `attach-session`, `item-from-session`) and the
//! fzf-driven views stay Ruby.

use crate::attention::Attention;
use crate::cli_format::{ljust, paint, prefix_chars, puts, short_path};
use crate::data_root;
use crate::outcome::{abort, done, Outcome};
use crate::session_log::SessionLog;
use crate::work_item::WorkItem;

const FILTER_CYCLE: [&str; 5] = ["open", "in_progress", "fridge", "resolved", "all"];

pub fn run(command: &str, args: &[String]) -> Outcome {
    match command {
        "viewer-rows" => viewer_rows(args),
        "session-rows" => session_rows(args),
        "inbox-rows" => inbox_rows(),
        _ => cycle_filter(),
    }
}

fn failed(e: crate::error::QmError) -> Outcome {
    abort(&format!("qm: {}", e.message))
}

macro_rules! tri {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return failed(e),
        }
    };
}

fn filter_file() -> std::path::PathBuf {
    data_root::root().join("viewer-filter")
}

/// The stored state filter; unreadable or unknown reads as the first.
fn stored_filter() -> &'static str {
    let text = std::fs::read_to_string(filter_file()).map(|t| crate::ruby_text::strip(&t).to_string()).unwrap_or_default();
    FILTER_CYCLE.iter().copied().find(|f| *f == text).unwrap_or(FILTER_CYCLE[0])
}

fn cycle_filter() -> Outcome {
    let at = FILTER_CYCLE.iter().position(|f| *f == stored_filter()).unwrap_or(0);
    let _ = data_root::ensure_setup();
    match std::fs::write(filter_file(), FILTER_CYCLE[(at + 1) % FILTER_CYCLE.len()]) {
        Ok(()) => done(String::new()),
        Err(_) => Outcome::Delegate,
    }
}

fn badge(state: &str) -> (&'static str, u8) {
    match state {
        "in_progress" => ("●", 32),
        "fridge" => ("❄", 36),
        "resolved" => ("✓", 90),
        _ => ("?", 0),
    }
}

fn viewer_rows(a: &[String]) -> Outcome {
    let mut state: Option<String> = None;
    let mut i = 0;
    while i < a.len() {
        match a[i].as_str() {
            "--state" => {
                let Some(v) = a.get(i + 1) else { return Outcome::Delegate };
                state = Some(v.clone());
                i += 2;
            }
            s if s.starts_with('-') && s != "-" => return Outcome::Delegate, // OptionParser reads these its own way
            _ => i += 1,
        }
    }
    let state = state.unwrap_or_else(|| stored_filter().to_string());
    let items: Vec<WorkItem> = tri!(WorkItem::all())
        .into_iter()
        .filter(|i| match state.as_str() {
            "all" => true,
            "open" => i.state().as_deref() != Some("resolved"),
            s => i.state().as_deref() == Some(s),
        })
        .collect();
    let live: Vec<String> = SessionLog::read().sessions().into_iter().filter(|s| s.live).map(|s| s.id).collect();
    let scan = Attention::new(std::time::SystemTime::now());
    let entries = tri!(scan.entries());
    let mut keyed = vec![];
    for item in items {
        let first = if entries.iter().any(|e| e.slug == item.slug) { 0 } else { 1 };
        let activity = tri!(scan.activity_at(&item));
        keyed.push((first, -activity, item));
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)));

    let mut out = String::new();
    for (_, _, item) in keyed {
        let state = item.state().unwrap_or_default();
        let (mark, color) = badge(&state);
        let is_live = item.session_ids().iter().any(|id| live.contains(id));
        let state_col = paint(&ljust(&format!("{mark} {state}{}", if is_live { " ▶" } else { "" }), 15), color);
        let mut reasons: Vec<&str> = vec![];
        for e in entries.iter().filter(|e| e.slug == item.slug) {
            if !reasons.contains(&e.message.as_str()) {
                reasons.push(&e.message);
            }
        }
        let branches = item.branches();
        let cols = [
            item.slug.clone(),
            state_col,
            ljust(&item.project().unwrap_or_default(), 12),
            ljust(&prefix_chars(&item.title(), 44), 44),
            if branches.is_empty() { "-".to_string() } else { branches.join(" ") },
            if reasons.is_empty() { String::new() } else { paint(&format!("⚠ {}", reasons.join("; ")), 33) },
        ];
        out.push_str(&puts(&cols.join("\t")));
    }
    done(out)
}

fn session_rows(a: &[String]) -> Outcome {
    let Some(slug) = a.first() else { return abort("usage: qm session-rows SLUG") };
    if slug.starts_with('-') {
        return Outcome::Delegate;
    }
    let item = tri!(WorkItem::find(slug));
    let rows = tri!(crate::show_command::session_lines(&item, &SessionLog::read()));
    done(rows.iter().map(|r| puts(r)).collect())
}

fn inbox_rows() -> Outcome {
    let mut sessions = tri!(SessionLog::read().inbox());
    sessions.sort_by_key(|s| s.last_ts());
    sessions.reverse();
    let mut out = String::new();
    for s in sessions {
        let spaced: String = {
            let prompt = s.first_prompt().unwrap_or_default();
            regex::Regex::new(r"[ \t\r\n\x0c\x0b]+").unwrap().replace_all(&prompt, " ").into_owned()
        };
        let cols = [s.id.clone(), prefix_chars(&s.last_ts(), 16), prefix_chars(&ljust(&short_path(s.cwd().as_deref()), 34), 34), prefix_chars(&spaced, 60)];
        out.push_str(&puts(&cols.join("\t")));
    }
    done(out)
}
