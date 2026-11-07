//! The tokens one transcript file spent, summed from its assistant entries' `usage` per bucket and
//! read incrementally: the state remembers the byte offset of the last complete line. A message
//! counts once, with the usage of its last entry, so the newest message stays "pending" until the
//! next one starts. The state is plain JSON; `UsageLedger` persists it.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::{json, Map, Value};

use crate::error::QmError;

pub const KINDS: [&str; 5] = ["input", "cache_write_5m", "cache_write_1h", "cache_read", "output"];
const LONG_PROMPT: i64 = 100_000;
const SYNTHETIC_MODEL: &str = "<synthetic>";

pub type State = Map<String, Value>;
pub type Buckets = Map<String, Value>;

/// The class of an error from reading the file itself (the Ruby `SystemCallError`).
pub const SYSTEM_CALL_ERROR: &str = "SystemCallError";

fn sys(e: std::io::Error) -> QmError {
    QmError::new(SYSTEM_CALL_ERROR, e.to_string())
}

pub fn empty_state() -> State {
    let mut s = Map::new();
    s.insert("size".into(), 0.into());
    s.insert("mtime".into(), Value::Null);
    s.insert("offset".into(), 0.into());
    s.insert("buckets".into(), Value::Object(Map::new()));
    s.insert("pending".into(), Value::Null);
    s
}

/// `Time#to_f` of a file's mtime. Ruby converts the integer nanosecond count to a double first and
/// then divides by 1e9, so the result is not always the nearest double to the exact seconds.
pub fn mtime_f64(meta: &std::fs::Metadata) -> Result<f64, QmError> {
    let modified = meta.modified().map_err(sys)?;
    let nanos: i128 = match modified.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i128,
        Err(e) => -(e.duration().as_nanos() as i128),
    };
    Ok(nanos as f64 / 1e9)
}

/// Ruby's `String#to_i`: optional sign, digits with single underscores between them.
fn string_to_i(s: &str) -> i64 {
    let t = s.trim_start_matches([' ', '\t', '\n', '\u{b}', '\u{c}', '\r']);
    let (neg, t) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let mut value: i64 = 0;
    let mut prev_digit = false;
    for c in t.chars() {
        if let Some(d) = c.to_digit(10) {
            value = value.saturating_mul(10).saturating_add(d as i64);
            prev_digit = true;
        } else if c == '_' && prev_digit {
            prev_digit = false;
        } else {
            break;
        }
    }
    if neg { -value } else { value }
}

/// `value.to_i` for a JSON value; a boolean, array or object has no `to_i` in Ruby.
pub fn to_i(v: Option<&Value>) -> Result<i64, QmError> {
    match v {
        None | Some(Value::Null) => Ok(0),
        Some(Value::Number(n)) => Ok(n.as_i64().unwrap_or_else(|| n.as_f64().unwrap_or(0.0).trunc() as i64)),
        Some(Value::String(s)) => Ok(string_to_i(s)),
        Some(_) => Err(QmError::new("NoMethodError", "undefined method 'to_i'")),
    }
}

struct Usage {
    id: Value,
    bucket: String,
    tokens: [i64; 5],
}

/// "claude-haiku-4-5-20251001" → "claude-haiku-4-5"; "claude-opus-4-6[1m]" → "claude-opus-4-6".
pub fn normalize_model(model: &str) -> String {
    let mut m = crate::ruby_text::strip(model);
    if m.ends_with(']') {
        if let Some(open) = m.rfind('[') {
            if !m[open..m.len() - 1].contains(']') {
                m = &m[..open];
            }
        }
    }
    let b = m.as_bytes();
    if b.len() >= 9 && b[b.len() - 9] == b'-' && b[b.len() - 8..].iter().all(u8::is_ascii_digit) {
        m = &m[..m.len() - 9];
    }
    if m.is_empty() { "unknown".into() } else { m.to_string() }
}

fn truthy(v: Option<&Value>) -> Option<&Value> {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => None,
        some => some,
    }
}

/// The 5m/1h cache-write split comes from `usage.cache_creation`; the rest of a write is 5m.
fn kinds(usage: &Map<String, Value>) -> Result<[i64; 5], QmError> {
    let empty = Map::new();
    let creation = usage.get("cache_creation").and_then(Value::as_object).unwrap_or(&empty);
    let write_total = to_i(usage.get("cache_creation_input_tokens"))?;
    let write_1h = to_i(creation.get("ephemeral_1h_input_tokens"))?;
    let write_5m = to_i(creation.get("ephemeral_5m_input_tokens"))?.max(write_total - write_1h);
    Ok([to_i(usage.get("input_tokens"))?, write_5m, write_1h, to_i(usage.get("cache_read_input_tokens"))?, to_i(usage.get("output_tokens"))?])
}

/// None for entries that spent nothing worth counting.
fn entry_usage(entry: &Value) -> Result<Option<Usage>, QmError> {
    let Some(e) = entry.as_object().filter(|e| e.get("type") == Some(&Value::String("assistant".into()))) else { return Ok(None) };
    let Some(message) = e.get("message").and_then(Value::as_object) else { return Ok(None) };
    let Some(usage) = message.get("usage").and_then(Value::as_object) else { return Ok(None) };
    let model = match message.get("model") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    };
    if model == SYNTHETIC_MODEL {
        return Ok(None);
    }
    let tokens = kinds(usage)?;
    if tokens.iter().sum::<i64>() == 0 {
        return Ok(None);
    }
    let prompt = tokens[0] + tokens[3] + tokens[1] + tokens[2];
    let mut bucket = normalize_model(&model);
    if usage.get("speed") == Some(&Value::String("fast".into())) {
        bucket.push_str(":fast");
    }
    if prompt > LONG_PROMPT {
        bucket.push_str(":long");
    }
    let id = truthy(message.get("id")).or_else(|| truthy(e.get("requestId"))).or_else(|| truthy(e.get("uuid"))).or(e.get("uuid")).cloned().unwrap_or(Value::Null);
    Ok(Some(Usage { id, bucket, tokens }))
}

fn tokens_json(tokens: &[i64; 5]) -> Value {
    Value::Object(KINDS.iter().zip(tokens).map(|(k, t)| (k.to_string(), Value::from(*t))).collect())
}

/// Adds `tokens` (a JSON object of kinds) to the bucket `key`, creating it at zero.
pub fn add(buckets: &mut Buckets, key: &str, tokens: &Value) -> Result<(), QmError> {
    let bucket = buckets.entry(key.to_string()).or_insert_with(|| tokens_json(&[0; 5]));
    let obj = bucket.as_object_mut().ok_or_else(|| QmError::new("TypeError", "bucket is not a hash"))?;
    for k in KINDS {
        let current = to_i(obj.get(k))?;
        obj.insert(k.to_string(), Value::from(current + to_i(tokens.get(k))?));
    }
    Ok(())
}

pub fn merge(into: &mut Buckets, buckets: &Buckets) -> Result<(), QmError> {
    for (key, tokens) in buckets {
        add(into, key, tokens)?;
    }
    Ok(())
}

/// The buckets including the pending message: what the file has spent so far.
pub fn totals(state: &State) -> Result<Buckets, QmError> {
    let mut buckets = state.get("buckets").and_then(Value::as_object).cloned().unwrap_or_default();
    if let Some(Value::Object(p)) = state.get("pending") {
        let key = p.get("bucket").and_then(Value::as_str).unwrap_or("");
        add(&mut buckets, key, p.get("tokens").unwrap_or(&Value::Null))?;
    }
    Ok(buckets)
}

fn same_number(a: Option<&Value>, b: f64) -> bool {
    a.and_then(Value::as_f64) == Some(b)
}

/// The state brought up to date with the file: unchanged → as is, grown → continued from its
/// offset, anything else (shrunk, rewritten) → from the start.
pub fn scan(path: &Path, state: Option<&State>) -> Result<State, QmError> {
    let meta = std::fs::metadata(path).map_err(sys)?;
    let (size, mtime) = (meta.len() as i64, mtime_f64(&meta)?);
    if let Some(s) = state {
        if s.get("size").and_then(Value::as_i64) == Some(size) && same_number(s.get("mtime"), mtime) {
            return Ok(s.clone());
        }
    }
    let usable = state.filter(|s| size > to_i(s.get("size")).unwrap_or(0) && to_i(s.get("offset")).unwrap_or(0) <= size);
    let mut next = usable.cloned().unwrap_or_else(empty_state);
    read_from(path, &mut next)?;
    next.insert("size".into(), size.into());
    next.insert("mtime".into(), serde_json::Number::from_f64(mtime).map(Value::Number).unwrap_or(Value::Null));
    Ok(next)
}

fn read_from(path: &Path, state: &mut State) -> Result<(), QmError> {
    let mut file = std::fs::File::open(path).map_err(sys)?;
    let offset = to_i(state.get("offset"))?.max(0) as u64;
    file.seek(SeekFrom::Start(offset)).map_err(sys)?;
    let mut data = vec![];
    file.read_to_end(&mut data).map_err(sys)?;
    let Some(last) = data.iter().rposition(|b| *b == b'\n') else { return Ok(()) };
    let chunk = &data[..=last];
    state.insert("offset".into(), Value::from(offset as i64 + chunk.len() as i64));
    let mut buckets = state.get("buckets").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut pending = state.get("pending").cloned().unwrap_or(Value::Null);
    for line in chunk.split_inclusive(|b| *b == b'\n') {
        let Ok(text) = std::str::from_utf8(line) else { continue };
        let Ok(entry) = serde_json::from_str::<Value>(text) else { continue };
        let Some(usage) = entry_usage(&entry)? else { continue };
        if let Value::Object(p) = &pending {
            if p.get("id") != Some(&usage.id) {
                add(&mut buckets, p.get("bucket").and_then(Value::as_str).unwrap_or(""), p.get("tokens").unwrap_or(&Value::Null))?;
            }
        }
        pending = json!({ "id": usage.id, "bucket": usage.bucket, "tokens": tokens_json(&usage.tokens) });
    }
    state.insert("buckets".into(), Value::Object(buckets));
    state.insert("pending".into(), pending);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_lose_their_date_and_context_suffixes() {
        assert_eq!(normalize_model("claude-haiku-4-5-20251001"), "claude-haiku-4-5");
        assert_eq!(normalize_model("claude-opus-4-6[1m]"), "claude-opus-4-6");
        assert_eq!(normalize_model("claude-opus-4-6[1m]-20251001"), "claude-opus-4-6[1m]");
        assert_eq!(normalize_model("  "), "unknown");
        assert_eq!(normalize_model("x-2025"), "x-2025");
    }

    #[test]
    fn to_i_follows_ruby() {
        assert_eq!(string_to_i(" -1_2x"), -12);
        assert_eq!(string_to_i("_1"), 0);
        assert_eq!(string_to_i("1__2"), 1);
        assert_eq!(string_to_i("0x1A"), 0);
        assert!(to_i(Some(&Value::Bool(true))).is_err());
        assert_eq!(to_i(Some(&json!(2.9))).unwrap(), 2);
    }
}
