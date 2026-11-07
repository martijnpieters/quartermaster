use crate::cli_format::{format_duration, short_path};
use crate::ruby_text::prefix;
use crate::session_log::Session;

/// The `qm sessions` / `qm inbox` table: newest first, at most `limit` rows.
pub fn print(mut sessions: Vec<Session>, limit: usize) -> String {
    sessions.sort_by_key(|s| s.last_ts());
    sessions.reverse();
    let mut out = String::new();
    for s in sessions.iter().take(limit) {
        let status = if s.live { "● live " } else { "  ended" };
        let duration = if s.ended() { format_duration(s.duration_s()) } else { "-".into() };
        let cwd = short_path(s.cwd().as_deref());
        let cols = [
            status.to_string(),
            prefix(&s.id, 8),
            s.item_label().unwrap_or_else(|| "-".into()),
            cwd,
            s.branch().unwrap_or_else(|| "-".into()),
            prefix(&s.started_ts(), 16),
            duration,
        ];
        out.push_str(&cols.join("  "));
        out.push('\n');
    }
    out
}
