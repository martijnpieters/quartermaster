//! `qm grep PHRASE`: the sessions whose conversation mentions a phrase — what the user typed or
//! Claude answered, never tool calls or their output. `rg` picks the candidate transcripts (through
//! `command_runner`), then each candidate's lines are parsed to keep only real conversation hits.
//! Read-only, so anything this port can't judge exactly as Ruby does (a transcript that isn't UTF-8,
//! a phrase Onigmo's multi-character case folding could treat differently, a `message` that isn't an
//! object) hands the whole command to Ruby.

use std::collections::HashMap;

use regex::Regex;
use serde_json::{Map, Value};

use crate::command_runner;
use crate::data_root;
use crate::session_log::SessionLog;
use crate::work_item::WorkItem;

const MAX_SESSIONS: usize = 60;
const SNIPPET_BEFORE: usize = 60;
const SNIPPET_AFTER: usize = 140;
const RG_FALLBACK_DIRS: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];

pub struct Hit {
    pub id: String,
    /// The first matching entry's own `timestamp`, as the transcript wrote it.
    pub ts: Value,
    pub transcript: String,
    pub cwd: Option<String>,
    pub date: Option<String>,
    pub hits: usize,
    pub role: &'static str,
    pub before: String,
    pub matched: String,
    pub after: String,
}

pub struct Group {
    pub slug: Option<String>,
    pub title: String,
    pub state: Option<String>,
    pub sessions: Vec<Hit>,
}

pub struct Found {
    pub query: String,
    pub groups: Vec<Group>,
    pub more: usize,
}

impl Found {
    /// The GUI's answer: `TranscriptSearch#call`'s hash.
    pub fn to_json(&self) -> Value {
        let groups: Vec<Value> = self
            .groups
            .iter()
            .map(|g| {
                let sessions: Vec<Value> = g
                    .sessions
                    .iter()
                    .map(|h| serde_json::json!({"id": h.id, "date": h.date, "hits": h.hits, "transcript": h.transcript, "cwd": h.cwd, "role": h.role, "ts": h.ts, "before": h.before, "match": h.matched, "after": h.after}))
                    .collect();
                serde_json::json!({"slug": g.slug, "title": g.title, "state": g.state, "sessions": sessions})
            })
            .collect();
        serde_json::json!({"query": self.query, "groups": groups, "more": self.more})
    }
}

pub enum Failure {
    /// Ruby's `ArgumentError` message, printed bare.
    Abort(String),
    /// Something only Ruby reads exactly.
    Ruby,
}

fn truthy(v: Option<&Value>) -> bool {
    !matches!(v, None | Some(Value::Null) | Some(Value::Bool(false)))
}

fn rg() -> String {
    let mut dirs: Vec<String> = std::env::var("PATH").unwrap_or_default().split(':').map(String::from).collect();
    dirs.extend(RG_FALLBACK_DIRS.iter().map(|d| d.to_string()));
    dirs.iter()
        .map(|d| std::path::Path::new(d).join("rg"))
        .find(|p| {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "rg".into())
}

/// Characters whose full case folding is more than one character: Onigmo matches `ss` against `ß`.
fn multi_fold(c: char) -> bool {
    matches!(c, 'ß' | 'ẞ' | 'ŉ' | 'ǰ' | 'ΐ' | 'ΰ' | 'և' | 'İ') || ('\u{1e96}'..='\u{1e9b}').contains(&c) || ('\u{1f50}'..='\u{1fff}').contains(&c) || ('\u{fb00}'..='\u{fb06}').contains(&c) || ('\u{fb13}'..='\u{fb17}').contains(&c)
}

/// Whether a line with a multi-fold character could match the phrase in Ruby but not here.
fn fold_sensitive(query: &str) -> bool {
    if !query.is_ascii() {
        return true;
    }
    let lower = query.to_ascii_lowercase();
    ["ss", "ff", "fi", "fl", "st"].iter().any(|s| lower.contains(s))
}

/// The text of a conversation entry: `("user" | "assistant", text)`; `Err` where Ruby would raise.
fn conversation_text(entry: &Map<String, Value>) -> Result<Option<(&'static str, String)>, ()> {
    match entry.get("type").and_then(Value::as_str) {
        Some("user") => {
            if truthy(entry.get("isCompactSummary")) {
                return Ok(None);
            }
            if let Some(Value::Array(parts)) = entry.get("message").and_then(|m| m.get("content")) {
                let odd = parts.iter().any(|c| matches!(c, Value::Object(o) if o.get("type") == Some(&Value::String("text".into())) && !matches!(o.get("text"), None | Some(Value::Null) | Some(Value::Bool(false)) | Some(Value::String(_)))));
                if odd {
                    return Err(());
                }
            }
            Ok(crate::transcripts::prompt_text(entry)?.map(|t| ("user", t)))
        }
        Some("assistant") => {
            let content = match entry.get("message") {
                None | Some(Value::Null) => None,
                Some(Value::Object(m)) => m.get("content"),
                Some(_) => return Err(()),
            };
            let text = match content {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Array(parts)) => {
                    let mut texts = vec![];
                    for b in parts {
                        let Value::Object(o) = b else { continue };
                        if o.get("type") != Some(&Value::String("text".into())) {
                            continue;
                        }
                        match o.get("text") {
                            None | Some(Value::Null) | Some(Value::Bool(false)) => {}
                            Some(Value::String(s)) => texts.push(s.clone()),
                            Some(_) => return Err(()),
                        }
                    }
                    texts.join("\n")
                }
                _ => String::new(),
            };
            Ok(if text.is_empty() { None } else { Some(("assistant", text)) })
        }
        _ => Ok(None),
    }
}

fn squash(text: &str) -> String {
    Regex::new(r"[ \t\r\n\x0c\x0b]+").unwrap().replace_all(text, " ").into_owned()
}

struct Matchers {
    text: Regex,
    line: Regex,
    sensitive: bool,
}

fn scan(path: &str, m: &Matchers, log: &SessionLog) -> Result<Option<Hit>, Failure> {
    let Ok(bytes) = std::fs::read(path) else { return Ok(None) };
    let content = String::from_utf8(bytes).map_err(|_| Failure::Ruby)?;
    let id = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let timestamp = Regex::new(r#""timestamp":"([^"]+)""#).unwrap();
    let (mut first_ts, mut found, mut count): (Option<String>, Option<(&'static str, String, String, String)>, usize) = (None, None, 0);
    let mut found_ts = Value::Null;
    for line in content.split_inclusive('\n') {
        if first_ts.is_none() {
            first_ts = timestamp.captures(line).map(|c| c[1].to_string());
        }
        if m.sensitive && line.chars().any(multi_fold) {
            return Err(Failure::Ruby);
        }
        if !m.line.is_match(line) {
            continue;
        }
        let Ok(Value::Object(entry)) = serde_json::from_str::<Value>(line) else { continue };
        if truthy(entry.get("isSidechain")) {
            continue;
        }
        let Some((role, text)) = conversation_text(&entry).map_err(|_| Failure::Ruby)? else { continue };
        let Some(found_at) = m.text.find(&text) else { continue };
        count += 1;
        if found.is_none() {
            found_ts = entry.get("timestamp").cloned().unwrap_or(Value::Null);
            let chars: Vec<char> = text.chars().collect();
            let at = text[..found_at.start()].chars().count();
            let len = found_at.as_str().chars().count();
            let from = at.saturating_sub(SNIPPET_BEFORE);
            let to = (at + len + SNIPPET_AFTER).min(chars.len());
            let seg = |a: usize, b: usize| chars[a..b].iter().collect::<String>();
            found = Some((
                role,
                format!("{}{}", if from > 0 { "…" } else { "" }, squash(&seg(from, at))),
                seg(at, at + len),
                format!("{}{}", squash(&seg(at + len, to)), if to < chars.len() { "…" } else { "" }),
            ));
        }
    }
    let Some((role, before, matched, after)) = found else { return Ok(None) };
    let started = log.session_for(&id).map(|s| s.started_ts()).filter(|t| !t.is_empty());
    let cwd = log.session_for(&id).and_then(|s| s.cwd());
    Ok(Some(Hit { id, ts: found_ts, transcript: path.to_string(), cwd, date: started.or(first_ts), hits: count, role, before, matched, after }))
}

pub fn call(query: &str) -> Result<Found, Failure> {
    let query = crate::ruby_text::strip(query);
    if query.is_empty() {
        return Err(Failure::Abort("query is empty".into()));
    }
    let items = match WorkItem::all() {
        Ok(i) => i,
        Err(e) if e.class == "ArgumentError" => return Err(Failure::Abort(e.message)),
        Err(_) => return Err(Failure::Ruby),
    };
    let log = SessionLog::read();
    let escaped = serde_json::to_string(query).unwrap();
    let escaped = &escaped[1..escaped.len() - 1];
    let matchers = Matchers {
        text: Regex::new(&format!("(?i){}", regex::escape(query))).map_err(|_| Failure::Ruby)?,
        line: Regex::new(&format!("(?i){}", regex::escape(escaped))).map_err(|_| Failure::Ruby)?,
        sensitive: fold_sensitive(query),
    };
    if !query.is_ascii() && query.chars().any(multi_fold) {
        return Err(Failure::Ruby);
    }
    let asides: Vec<String> = log.sessions().iter().filter(|s| s.aside_for().is_some()).map(|s| s.id.clone()).collect();

    let dir = data_root::transcripts_dir();
    let mut candidates: Vec<String> = vec![];
    if dir.is_dir() {
        let dir_text = dir.to_string_lossy().into_owned();
        let result = command_runner::call(&[&rg(), "--files-with-matches", "--ignore-case", "--fixed-strings", "--max-depth", "2", "--glob", "*.jsonl", "--", escaped, &dir_text]);
        if result.err.contains("(os error") && result.out.is_empty() {
            return Err(Failure::Ruby); // rg is missing or won't run: Ruby words that error its own way
        }
        let failed = !result.success && !crate::ruby_text::strip(&result.err).is_empty() && crate::ruby_text::strip(&result.out).is_empty();
        if failed {
            return Err(Failure::Abort(format!("rg failed: {}", crate::ruby_text::strip(&result.err))));
        }
        let mut paths: Vec<(f64, String)> = result
            .out
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| (std::fs::metadata(l).and_then(|m| m.modified()).map(|t| crate::clock::epoch_f64(t)).unwrap_or(0.0), l.to_string()))
            .collect();
        paths.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        candidates = paths.into_iter().map(|(_, p)| p).filter(|p| !asides.contains(&std::path::Path::new(p).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())).collect();
    }
    let (mut hits, mut read) = (vec![], 0);
    for path in &candidates {
        if hits.len() >= MAX_SESSIONS {
            break;
        }
        read += 1;
        if let Some(hit) = scan(path, &matchers, &log)? {
            hits.push(hit);
        }
    }
    Ok(Found { query: query.to_string(), groups: group(hits, &items), more: candidates.len() - read })
}

fn group(hits: Vec<Hit>, items: &[WorkItem]) -> Vec<Group> {
    let mut owners: HashMap<String, &WorkItem> = HashMap::new();
    for item in items {
        for id in item.session_ids() {
            owners.entry(id).or_insert(item);
        }
    }
    let mut order: Vec<Option<String>> = vec![];
    let mut by_item: HashMap<Option<String>, Vec<Hit>> = HashMap::new();
    for hit in hits {
        let key = owners.get(&hit.id).map(|i| i.slug.clone());
        if !order.contains(&key) {
            order.push(key.clone());
        }
        by_item.entry(key).or_default().push(hit);
    }
    let mut groups: Vec<Group> = order
        .into_iter()
        .map(|key| {
            let mut sessions = by_item.remove(&key).unwrap();
            sessions.sort_by(|a, b| a.date.clone().unwrap_or_default().cmp(&b.date.clone().unwrap_or_default()));
            sessions.reverse();
            let item = key.as_ref().and_then(|slug| items.iter().find(|i| &i.slug == slug));
            Group { slug: key, title: item.map(|i| i.title()).unwrap_or_else(|| "unlinked".into()), state: item.and_then(|i| i.state()), sessions }
        })
        .collect();
    let first_date = |g: &Group| g.sessions[0].date.clone().unwrap_or_default();
    let mut linked: Vec<Group> = vec![];
    let mut unlinked: Vec<Group> = vec![];
    for g in groups.drain(..) {
        if g.slug.is_some() { linked.push(g) } else { unlinked.push(g) }
    }
    linked.sort_by_key(|g| first_date(g));
    linked.reverse();
    linked.extend(unlinked);
    linked
}
