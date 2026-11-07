//! `qm attention`: chats by whose move it is, then the slower nudges (expiring transcripts, quiet
//! items, merged PRs).

use std::time::SystemTime;

use crate::attention::Attention;
use crate::chat_attention::{ChatAttention, Entry};
use crate::clock;
use crate::cli_format::{format_duration, ljust, paint, prefix_chars, puts};
use crate::outcome::{abort, done, Outcome};
use crate::ruby_text::rstrip;

fn badge(state: &str) -> (&'static str, u8) {
    match state {
        "blocked" => ("🔴 blocked", 31),
        "your_turn" => ("🟠 your turn", 33),
        "working" => ("🔵 working", 34),
        "waiting" => ("🟣 waiting on sub-agent", 35),
        _ => ("⚪ no chat", 90),
    }
}

fn context_label(entry: &Entry, now: f64) -> String {
    let Some(tokens) = entry.context_tokens else { return String::new() };
    let cache = match entry.cache_warm_until {
        Some(until) if until > now => format!("cache warm {}", format_duration(Some((until - now) as i64))),
        _ => "cache cold".to_string(),
    };
    format!("ctx {}k · {cache}", (tokens as f64 / 1000.0).round() as i64)
}

fn tool_text(entry: &Entry) -> Option<String> {
    entry.tool.as_ref().map(|t| match t {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

fn print_chat_attention(chats: &[Entry], now: f64) -> String {
    let width = chats.iter().map(|e| e.slug.chars().count()).max().unwrap_or(0);
    let mut out = String::new();
    for entry in chats {
        let (label, color) = badge(&entry.state);
        let label = if entry.snoozed { format!("{label} (snoozed)") } else { label.to_string() };
        let detail = entry.waiting_on.as_ref().and_then(|w| w.first().cloned().flatten()).or_else(|| tool_text(entry));
        let waiting = match entry.since {
            Some(since) => {
                let mut parts = vec![format_duration(Some((now - since) as i64))];
                parts.extend(detail);
                parts.join(" · ")
            }
            None => String::new(),
        };
        let cells = [paint(&ljust(&label, 21), if entry.snoozed { 90 } else { color }), ljust(&entry.slug, width), ljust(&waiting, 24), context_label(entry, now)];
        out.push_str(&puts(rstrip(&cells.join("  "))));
    }
    out
}

pub fn attention() -> Outcome {
    let mut chat = ChatAttention::new();
    let chats = match chat.entries() {
        Ok(c) => c,
        Err(e) => return abort(&format!("qm: {}", e.message)),
    };
    let entries = match Attention::new(SystemTime::now()).entries() {
        Ok(e) => e,
        Err(e) => return abort(&format!("qm: {}", e.message)),
    };
    if chats.is_empty() && entries.is_empty() {
        return done("Nothing needs attention.\n".into());
    }
    let mut out = print_chat_attention(&chats, clock::now_f64());
    if !chats.is_empty() && !entries.is_empty() {
        out.push('\n');
    }
    let width = entries.iter().map(|e| e.slug.chars().count()).max().unwrap_or(0);
    for e in &entries {
        let suffix = e.session_id.as_ref().map(|id| format!(" (session {})", prefix_chars(id, 8))).unwrap_or_default();
        out.push_str(&puts(&format!("{}  {}{suffix}", ljust(&e.slug, width), e.message)));
    }
    done(out)
}
