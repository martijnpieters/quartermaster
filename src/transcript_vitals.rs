//! What a session's transcript says about its chat right now, read from the tail only (transcripts
//! reach many megabytes): the context size, how long the prompt cache stays warm, and the last
//! interrupt (Esc fires no hook, so this is the only trace one leaves).

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

use crate::clock;
use crate::transcript_usage::to_i;

const TAIL_BYTES: u64 = 256 * 1024;
const CACHE_TTL_1H: f64 = 3600.0;
const CACHE_TTL_5M: f64 = 300.0;
pub(crate) const INTERRUPT_MARK: &str = "[Request interrupted by user";

#[derive(Default)]
pub struct Vitals {
    /// The last assistant message's input + cache_read + cache_creation tokens: what the next turn sends.
    pub context_tokens: Option<i64>,
    pub last_assistant_at: Option<f64>,
    pub cache_warm_until: Option<f64>,
    pub interrupted_at: Option<f64>,
}

/// `Time.parse` of an entry's timestamp: None when absent or unreadable.
fn time(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_str).and_then(clock::parse_f64)
}

fn truthy(v: Option<&Value>) -> bool {
    !matches!(v, None | Some(Value::Null) | Some(Value::Bool(false)))
}

/// `entry.dig("message", key)`: None when absent; Err where Ruby's `dig` would raise.
fn dig_message<'a>(entry: &'a Value, key: &str) -> Result<Option<&'a Value>, ()> {
    match entry.get("message") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(m)) => Ok(m.get(key)),
        Some(_) => Err(()),
    }
}

/// nil when this message created no cache entries — the tier then comes from the newest that did.
fn cache_tier(creation: Option<&Value>) -> Result<Option<&'static str>, ()> {
    let Some(Value::Object(c)) = creation else { return Ok(None) };
    if to_i(c.get("ephemeral_1h_input_tokens")).map_err(|_| ())? > 0 {
        return Ok(Some("1h"));
    }
    Ok((to_i(c.get("ephemeral_5m_input_tokens")).map_err(|_| ())? > 0).then_some("5m"))
}

fn interrupt(entry: &Value) -> Result<bool, ()> {
    let texts: Vec<&str> = match dig_message(entry, "content")? {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(parts)) => parts.iter().filter_map(|c| c.as_object().and_then(|o| o.get("text")).and_then(Value::as_str)).collect(),
        _ => vec![],
    };
    Ok(texts.iter().any(|t| t.starts_with(INTERRUPT_MARK)))
}

fn read_tail(path: &Path, bytes: u64, size: u64) -> Option<Vec<String>> {
    let mut f = std::fs::File::open(path).ok()?;
    f.seek(SeekFrom::Start(size.saturating_sub(bytes))).ok()?;
    let mut data = vec![];
    f.read_to_end(&mut data).ok()?;
    let text = String::from_utf8_lossy(&data).into_owned();
    let mut lines: Vec<String> = text.split_inclusive('\n').map(String::from).collect();
    if bytes < size && !lines.is_empty() {
        lines.remove(0); // a partial first line
    }
    Some(lines)
}

fn scan_lines(v: &mut Vitals, lines: &[String]) -> Result<(), ()> {
    let (mut usage_seen, mut tier): (bool, Option<&'static str>) = (false, None);
    for line in lines.iter().rev() {
        let entry = match serde_json::from_str::<Value>(line) {
            Ok(Value::Null) | Ok(Value::Bool(false)) | Err(_) => continue,
            Ok(Value::Object(o)) => Value::Object(o),
            Ok(Value::String(_)) => continue,
            Ok(_) => return Err(()),
        };
        if truthy(entry.get("isSidechain")) {
            continue;
        }
        match entry.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                let Some(usage) = dig_message(&entry, "usage")?.and_then(Value::as_object) else { continue };
                if dig_message(&entry, "model")? == Some(&Value::String("<synthetic>".into())) {
                    continue;
                }
                if !usage_seen {
                    let mut total = 0;
                    for k in ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"] {
                        total += to_i(usage.get(k)).map_err(|_| ())?;
                    }
                    if total == 0 {
                        continue;
                    }
                    usage_seen = true;
                    v.context_tokens = Some(total);
                    v.last_assistant_at = time(entry.get("timestamp"));
                }
                if tier.is_none() {
                    tier = cache_tier(usage.get("cache_creation"))?;
                }
            }
            Some("user") => {
                if v.interrupted_at.is_none() && interrupt(&entry)? {
                    v.interrupted_at = time(entry.get("timestamp"));
                }
            }
            _ => {}
        }
        if usage_seen && tier.is_some() && v.interrupted_at.is_some() {
            break;
        }
    }
    if let Some(at) = v.last_assistant_at {
        v.cache_warm_until = Some(at + if tier == Some("1h") { CACHE_TTL_1H } else { CACHE_TTL_5M });
    }
    Ok(())
}

/// None without a transcript, or when it cannot be read as Ruby reads it.
pub fn read(path: Option<&Path>) -> Option<Vitals> {
    let path = path.filter(|p| p.exists())?;
    let size = std::fs::metadata(path).ok()?.len();
    let mut vitals = Vitals::default();
    let mut tail = TAIL_BYTES;
    loop {
        let lines = read_tail(path, tail.min(size), size)?;
        scan_lines(&mut vitals, &lines).ok()?;
        if vitals.context_tokens.is_some() || tail >= size {
            break;
        }
        tail *= 4;
    }
    Some(vitals)
}
