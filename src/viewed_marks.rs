//! Which files of an item's diff the user marked viewed (`viewed/<slug>.json`): a mark is keyed to
//! (path, digest of that file's diff as shown). Reading and applying only; setting a mark is the
//! GUI action's, in Ruby.

use serde_json::{json, Value};

use crate::data_root;

/// A file's diff as the pane shows it, hashed: its paths, status and every hunk's header and lines.
pub fn digest(file: &Value) -> String {
    let hunks: Vec<Value> = file["hunks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|h| {
            let lines: Vec<String> = h["lines"].as_array().into_iter().flatten().map(|l| format!("{}{}", l["t"].as_str().unwrap_or(""), l["s"].as_str().unwrap_or(""))).collect();
            json!([h["header"], lines])
        })
        .collect();
    let binary = if file["binary"].as_bool().unwrap_or(false) { 1 } else { 0 };
    let shown = json!([file["status"], file["oldPath"], file["path"], binary, hunks]);
    crate::sha256::hex(shown.to_string().as_bytes())[..20].to_string()
}

/// `ViewedMarks#marks`: path → digests, empty for anything unreadable.
fn marks(slug: &str) -> serde_json::Map<String, Value> {
    let path = data_root::root().join("viewed").join(format!("{slug}.json"));
    match std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        Some(Value::Object(d)) => match d.get("marks") {
            Some(Value::Object(m)) => m.clone(),
            _ => Default::default(),
        },
        _ => Default::default(),
    }
}

/// Gives each file of a diff payload its `digest` and `viewed` flag.
pub fn apply(slug: &str, files: &mut [Value]) {
    let known = marks(slug);
    for file in files {
        let d = digest(file);
        let viewed = match known.get(file["path"].as_str().unwrap_or("")) {
            Some(Value::Array(list)) => list.iter().any(|x| x.as_str() == Some(d.as_str())),
            Some(Value::Null) | None => false,
            Some(Value::String(s)) => *s == d,
            Some(_) => false,
        };
        let o = file.as_object_mut().unwrap();
        o.insert("digest".into(), json!(d));
        o.insert("viewed".into(), json!(viewed));
    }
}

/// Why a `set` stopped before writing.
pub enum Halt {
    Refuse(String),
    Unported,
}

fn has_float(v: &Value) -> bool {
    match v {
        Value::Number(n) => !(n.is_i64() || n.is_u64()),
        Value::Array(items) => items.iter().any(has_float),
        Value::Object(map) => map.values().any(has_float),
        _ => false,
    }
}

/// `String#inspect` for what a page might send as a digest; the rest is Ruby's.
fn inspected(v: &Value) -> Option<String> {
    match v {
        Value::Null => Some("nil".into()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) if n.is_i64() || n.is_u64() => Some(n.to_string()),
        Value::String(s) => crate::ruby_text::inspect_ascii(s),
        _ => None,
    }
}

/// `ViewedMarks#set`: marks or unmarks one (path, digest) under the item's lock, keeping a path's
/// newest six digests and the 2000 most recently marked paths. Returns whether it is viewed now.
pub fn set(slug: &str, path: &Value, digest: &Value, viewed: bool) -> Result<bool, Halt> {
    use std::os::unix::io::AsRawFd;
    let Value::String(file_path) = path else { return Err(Halt::Refuse("a viewed mark needs a path".into())) };
    if crate::ruby_text::strip(file_path).is_empty() {
        return Err(Halt::Refuse("a viewed mark needs a path".into()));
    }
    let digest = match digest {
        Value::String(d) if (16..=64).contains(&d.len()) && d.bytes().all(|c| c.is_ascii_hexdigit()) => d.clone(),
        other => return Err(inspected(other).map(|i| Halt::Refuse(format!("{i} is not a diff digest"))).unwrap_or(Halt::Unported)),
    };
    let dir = data_root::root().join("viewed");
    let file = dir.join(format!("{slug}.json"));
    if let Ok(text) = std::fs::read_to_string(&file) {
        if serde_json::from_str::<Value>(&text).is_ok_and(|v| has_float(&v)) {
            return Err(Halt::Unported);
        }
    }
    std::fs::create_dir_all(&dir).map_err(|_| Halt::Unported)?;
    let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(dir.join(format!(".{slug}.lock"))).map_err(|_| Halt::Unported)?;
    unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) };
    let mut data = match std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        Some(Value::Object(d)) if d.get("marks").is_some_and(Value::is_object) => d,
        _ => serde_json::Map::from_iter([("v".to_string(), json!(1)), ("marks".to_string(), json!({}))]),
    };
    let Some(Value::Object(all)) = data.get_mut("marks") else { return Err(Halt::Unported) };
    let mut digests: Vec<Value> = match all.shift_remove(file_path.as_str()) {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(items)) => items,
        Some(Value::String(s)) => vec![Value::String(s)],
        Some(_) => return Err(Halt::Unported),
    };
    digests.retain(|d| *d != Value::String(digest.clone()));
    if viewed {
        digests.push(Value::String(digest));
    }
    if !digests.is_empty() {
        let keep = digests.len().saturating_sub(6);
        all.insert(file_path.clone(), Value::Array(digests.split_off(keep)));
    }
    while all.len() > 2000 {
        let oldest = all.keys().next().cloned().unwrap_or_default();
        all.shift_remove(&oldest);
    }
    if all.is_empty() {
        let _ = std::fs::remove_file(&file);
    } else {
        data.insert("v".into(), json!(1));
        let temp = format!("{}.{}.{:?}.tmp", file.display(), std::process::id(), std::thread::current().id());
        std::fs::write(&temp, Value::Object(data).to_string()).map_err(|_| Halt::Unported)?;
        std::fs::rename(&temp, &file).map_err(|_| Halt::Unported)?;
    }
    Ok(viewed)
}
