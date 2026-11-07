//! The item's unsent drafts (`drafts/<slug>.json`, docs/areas/drafts.md): read only here.
//! Writing them — `qm draft add`, the GUI — stays Ruby's.

use serde_json::{Map, Value};

use crate::data_root;

/// `Drafts#entries`: the file's entries, or none when it is missing or unreadable as Ruby would read
/// it. `None` when an entry isn't an object, or holds a number Ruby would print differently.
pub fn entries(slug: &str) -> Option<Vec<Map<String, Value>>> {
    let Ok(text) = std::fs::read_to_string(data_root::root().join("drafts").join(format!("{slug}.json"))) else { return Some(vec![]) };
    let Ok(Value::Object(data)) = serde_json::from_str::<Value>(&text) else { return Some(vec![]) };
    let Some(Value::Array(list)) = data.get("entries") else { return Some(vec![]) };
    let mut out = vec![];
    for entry in list {
        let Value::Object(map) = entry else { return None };
        if has_float(entry) {
            return None;
        }
        out.push(map.clone());
    }
    Some(out)
}

fn has_float(v: &Value) -> bool {
    match v {
        Value::Number(n) => !(n.is_i64() || n.is_u64()),
        Value::Array(items) => items.iter().any(has_float),
        Value::Object(map) => map.values().any(has_float),
        _ => false,
    }
}

/// `JSON.pretty_generate` as the json 2.7 gem writes it: two-space indent, and an empty array or
/// object opens and closes on lines of their own (`[\n\n]`, `{\n}`).
pub fn pretty(v: &Value) -> String {
    let mut out = String::new();
    write(v, 0, &mut out);
    out
}

fn write(v: &Value, depth: usize, out: &mut String) {
    let pad = |n: usize| "  ".repeat(n);
    match v {
        Value::Array(items) => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                out.push_str(&pad(depth + 1));
                write(item, depth + 1, out);
            }
            out.push('\n');
            out.push_str(&pad(depth));
            out.push(']');
        }
        Value::Object(map) if map.is_empty() => {
            out.push_str("{\n");
            out.push_str(&pad(depth));
            out.push('}');
        }
        Value::Object(map) => {
            out.push_str("{\n");
            for (i, (key, value)) in map.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                out.push_str(&pad(depth + 1));
                out.push_str(&serde_json::to_string(key).unwrap());
                out.push_str(": ");
                write(value, depth + 1, out);
            }
            out.push('\n');
            out.push_str(&pad(depth));
            out.push('}');
        }
        scalar => out.push_str(&serde_json::to_string(scalar).unwrap()),
    }
}

/// Whether `forget_issue` can rewrite this item's drafts file exactly as Ruby does.
pub fn writable(slug: &str) -> bool {
    let path = data_root::root().join("drafts").join(format!("{slug}.json"));
    if !path.exists() {
        return true;
    }
    match std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        Some(Value::Object(data)) => match data.get("entries") {
            Some(Value::Array(list)) => list.iter().all(Value::is_object) && !has_float(&Value::Object(data.clone())),
            _ => true, // reads as empty: the file goes
        },
        _ => true,
    }
}

/// `Drafts#forget_issue`: the issue comments on a deleted issue file go with it, under the item's
/// drafts lock; a file left with no drafts and no clipboard load ids is removed.
pub fn forget_issue(slug: &str, file: &str) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let dir = data_root::root().join("drafts");
    let path = dir.join(format!("{slug}.json"));
    if !path.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(&dir)?;
    let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(dir.join(format!(".{slug}.lock")))?;
    unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) };
    let mut data = match std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        Some(Value::Object(d)) if d.get("entries").is_some_and(Value::is_array) => d,
        _ => Map::from_iter([("v".to_string(), Value::from(1)), ("entries".to_string(), Value::Array(vec![]))]),
    };
    if let Some(Value::Array(list)) = data.get_mut("entries") {
        list.retain(|e| !(e.get("kind").and_then(Value::as_str) == Some("issue-comment") && e.get("file").and_then(Value::as_str) == Some(file)));
    }
    let no_entries = matches!(data.get("entries"), Some(Value::Array(l)) if l.is_empty());
    let no_loaded = match data.get("loaded") {
        None | Some(Value::Null) => true,
        Some(Value::Array(l)) => l.is_empty(),
        Some(Value::Object(o)) => o.is_empty(),
        Some(_) => false,
    };
    if no_entries && no_loaded {
        let _ = std::fs::remove_file(&path);
    } else {
        data.insert("v".into(), Value::from(1));
        let temp = format!("{}.{}.{:?}.tmp", path.display(), std::process::id(), std::thread::current().id());
        std::fs::write(&temp, pretty(&Value::Object(data)))?;
        std::fs::rename(&temp, &path)?;
    }
    Ok(())
}

/// `Rational#to_s` of a file's mtime (`Time#to_r`): the nanoseconds over 1e9, reduced.
pub fn mtime_rational(path: &std::path::Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(path).ok()?;
    let ns = m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128;
    let (mut a, mut b) = (ns.abs(), 1_000_000_000i128);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    let g = a.max(1);
    Some(format!("{}/{}", ns / g, 1_000_000_000 / g))
}

/// `Drafts.signature`: moves whenever any item's drafts change; "0" while no item has a drafts file.
pub fn signature() -> String {
    let dir = data_root::root().join("drafts");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .map(|r| r.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json") && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) && p.is_file()).collect())
        .unwrap_or_default();
    files.sort_by_key(|p| p.to_string_lossy().into_owned());
    if files.is_empty() {
        return "0".into();
    }
    let stats: Vec<String> = files
        .iter()
        .map(|p| match (std::fs::metadata(p), mtime_rational(p)) {
            (Ok(m), Some(t)) => format!("{}:{}:{t}", p.file_name().unwrap().to_string_lossy(), m.len()),
            _ => String::new(),
        })
        .collect();
    crate::sha1::hex(stats.join("|").as_bytes())[..12].to_string()
}

const RELOCATED: [&str; 11] = ["path", "side", "line", "code", "range", "start_line", "start_side", "base", "commit", "subject", "pin"];

/// `Drafts#relocate`: moves a GitHub draft to where it posts now, its comment unchanged and its rev
/// kept. A null value drops the field and `failed` always goes. Only while the entry is still at
/// `rev`. Like every `Drafts#update` it rewrites the file under the item's lock whatever it found.
pub fn relocate(slug: &str, id: &str, fields: &Map<String, Value>, rev: Option<&Value>) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let dir = data_root::root().join("drafts");
    let path = dir.join(format!("{slug}.json"));
    std::fs::create_dir_all(&dir)?;
    let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(dir.join(format!(".{slug}.lock")))?;
    unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) };
    let mut data = match std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        Some(Value::Object(d)) if d.get("entries").is_some_and(Value::is_array) => d,
        _ => Map::from_iter([("v".to_string(), Value::from(1)), ("entries".to_string(), Value::Array(vec![]))]),
    };
    if let Some(Value::Array(list)) = data.get_mut("entries") {
        let found = list.iter_mut().find(|e| e.get("id").and_then(Value::as_str) == Some(id) && e.get("kind").and_then(Value::as_str) == Some("github-comment"));
        if let Some(Value::Object(entry)) = found {
            if rev.is_none_or(|r| entry.get("rev") == Some(r)) {
                for k in RELOCATED {
                    if let Some(v) = fields.get(k) {
                        if v.is_null() {
                            entry.shift_remove(k);
                        } else {
                            entry.insert(k.into(), v.clone());
                        }
                    }
                }
                entry.shift_remove("failed");
                entry.insert("updatedAt".into(), Value::String(crate::clock::now_utc_iso()));
            }
        }
    }
    let no_entries = matches!(data.get("entries"), Some(Value::Array(l)) if l.is_empty());
    let no_loaded = match data.get("loaded") {
        None | Some(Value::Null) => true,
        Some(Value::Array(l)) => l.is_empty(),
        Some(Value::Object(o)) => o.is_empty(),
        Some(_) => false,
    };
    if no_entries && no_loaded {
        let _ = std::fs::remove_file(&path);
    } else {
        data.insert("v".into(), Value::from(1));
        let temp = format!("{}.{}.{:?}.tmp", path.display(), std::process::id(), std::thread::current().id());
        std::fs::write(&temp, pretty(&Value::Object(data)))?;
        std::fs::rename(&temp, &path)?;
    }
    Ok(())
}

/// Why a `Drafts#update` block stopped before writing.
pub enum Halt {
    /// Ruby raises this `ArgumentError`.
    Refuse(String),
    /// A value this port does not read as Ruby does; nothing was written.
    Unported,
}

/// `Drafts#update`: the entries read and handed to `f` under the item's lock (the directory and lock
/// file exist even when `f` refuses), then written like Ruby writes them — an item left with no
/// drafts and no clipboard load ids loses its file.
pub fn update<T>(slug: &str, f: impl FnOnce(&mut Vec<Value>) -> Result<T, Halt>) -> Result<T, Halt> {
    update_data(slug, |data| {
        let Some(Value::Array(list)) = data.get_mut("entries") else { return Err(Halt::Unported) };
        f(list)
    })
}

/// `update`, handing the whole data (the entries and the clipboard load ids) to `f`.
pub fn update_data<T>(slug: &str, f: impl FnOnce(&mut Map<String, Value>) -> Result<T, Halt>) -> Result<T, Halt> {
    use std::os::unix::io::AsRawFd;
    if !writable(slug) {
        return Err(Halt::Unported);
    }
    let dir = data_root::root().join("drafts");
    let path = dir.join(format!("{slug}.json"));
    std::fs::create_dir_all(&dir).map_err(|_| Halt::Unported)?;
    let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(dir.join(format!(".{slug}.lock"))).map_err(|_| Halt::Unported)?;
    unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) };
    let mut data = match std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        Some(Value::Object(d)) if d.get("entries").is_some_and(Value::is_array) => d,
        _ => Map::from_iter([("v".to_string(), Value::from(1)), ("entries".to_string(), Value::Array(vec![]))]),
    };
    let result = f(&mut data)?;
    let no_entries = matches!(data.get("entries"), Some(Value::Array(l)) if l.is_empty());
    let no_loaded = match data.get("loaded") {
        None | Some(Value::Null) => true,
        Some(Value::Array(l)) => l.is_empty(),
        Some(Value::Object(o)) => o.is_empty(),
        Some(_) => false,
    };
    if no_entries && no_loaded {
        let _ = std::fs::remove_file(&path);
    } else {
        data.insert("v".into(), Value::from(1));
        let temp = format!("{}.{}.{:?}.tmp", path.display(), std::process::id(), std::thread::current().id());
        std::fs::write(&temp, pretty(&Value::Object(data))).map_err(|_| Halt::Unported)?;
        std::fs::rename(&temp, &path).map_err(|_| Halt::Unported)?;
    }
    Ok(result)
}
