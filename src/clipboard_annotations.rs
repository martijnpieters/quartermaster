//! Page annotations a dev server's page overlay copied to the clipboard (`ClipboardAnnotations`): the
//! last fenced ```pf-annotations block of the text, and the source line each owner points at.

use serde_json::{json, Map, Value};

use crate::ruby_text::{inspect_ascii, strip};

const MAX_BYTES: usize = 512 * 1024;
const MAX_COUNT: usize = 100;
const SOURCE_DIR: &str = "app/javascript";
const MAX_FILE_BYTES: u64 = 512 * 1024;
const MAX_CODE: usize = 500;

pub enum Fail {
    Refuse(String),
    Unported,
}

pub struct Payload {
    pub id: String,
    pub url: String,
    pub title: String,
    pub annotations: Vec<Value>,
}

fn refuse<T>(m: impl Into<String>) -> Result<T, Fail> {
    Err(Fail::Refuse(m.into()))
}

fn to_s(v: Option<&Value>) -> Result<String, Fail> {
    match v {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(t)) => Ok(t.clone()),
        Some(Value::Bool(b)) => Ok(b.to_string()),
        Some(Value::Number(n)) if n.is_i64() || n.is_u64() => Ok(n.to_string()),
        _ => Err(Fail::Unported),
    }
}

/// A header line (`^```pf-annotations[ \t]*\r?\n`) starting at byte `i`: where its body begins.
fn header_end(text: &str, i: usize) -> Option<usize> {
    if i != 0 && text.as_bytes()[i - 1] != b'\n' {
        return None;
    }
    let rest = text[i..].strip_prefix("```pf-annotations")?;
    let after = rest.trim_start_matches([' ', '\t']);
    let after = after.strip_prefix('\r').unwrap_or(after);
    let after = after.strip_prefix('\n')?;
    Some(text.len() - after.len())
}

/// A closing fence line (`^```[ \t]*\r?$`) starting at byte `i`.
fn closes(text: &str, i: usize) -> bool {
    let Some(rest) = text[i..].strip_prefix("```") else { return false };
    let rest = rest.trim_start_matches([' ', '\t']);
    let rest = rest.strip_prefix('\r').unwrap_or(rest);
    rest.is_empty() || rest.starts_with('\n')
}

fn last_block(text: &str) -> Option<&str> {
    let start = (0..text.len()).rev().filter(|i| text.is_char_boundary(*i)).find(|i| header_end(text, *i).is_some())?;
    let pos = header_end(text, start)?;
    let mut i = pos;
    loop {
        if closes(text, i) {
            return Some(&text[pos..i]);
        }
        i += text[i..].find('\n')? + 1;
    }
}

/// Ruby's `inspect` of the scalars a `v` can be; anything else is not reproduced.
fn inspect(v: Option<&Value>) -> Result<String, Fail> {
    match v {
        None | Some(Value::Null) => Ok("nil".into()),
        Some(Value::Bool(b)) => Ok(b.to_string()),
        Some(Value::Number(n)) if n.is_i64() || n.is_u64() => Ok(n.to_string()),
        Some(Value::String(t)) => inspect_ascii(t).ok_or(Fail::Unported),
        _ => Err(Fail::Unported),
    }
}

/// The last block's payload; the refusal text is what the page toasts.
pub fn parse(text: &Value) -> Result<Payload, Fail> {
    let text = to_s(Some(text))?;
    if text.len() > MAX_BYTES {
        return refuse("too much on the clipboard to load");
    }
    let Some(json) = last_block(&text) else {
        return refuse("no pf-annotations block on the clipboard — copy them in the page's annotation overlay first");
    };
    let Ok(Value::Object(payload)) = serde_json::from_str::<Value>(json) else {
        return refuse("the pf-annotations block is not valid JSON");
    };
    let version_ok = matches!(payload.get("v"), Some(Value::Number(n)) if n.as_f64() == Some(1.0));
    if !version_ok {
        return refuse(format!("unknown pf-annotations version {} (qm reads 1)", inspect(payload.get("v"))?));
    }
    let annotations = match payload.get("annotations") {
        Some(Value::Array(a)) if !a.is_empty() => a.clone(),
        _ => return refuse("the pf-annotations block holds no annotations"),
    };
    if annotations.len() > MAX_COUNT {
        return refuse(format!("too many annotations (max {MAX_COUNT})"));
    }
    let id = match payload.get("id") {
        Some(Value::String(id)) if !strip(id).is_empty() => strip(id).to_string(),
        _ => return refuse("the pf-annotations block has no id"),
    };
    Ok(Payload { id, url: to_s(payload.get("url"))?, title: to_s(payload.get("title"))?, annotations })
}

fn source_lines(root: Option<&str>, file: &str) -> Option<Vec<String>> {
    let root = root?;
    if !file.starts_with(&format!("{SOURCE_DIR}/")) || file.contains("..") || file.contains('\0') {
        return None;
    }
    let path = std::path::Path::new(root).join(file);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    let mut parts: Vec<Vec<u8>> = bytes.split(|b| *b == b'\n').map(<[u8]>::to_vec).collect();
    while parts.last().is_some_and(Vec::is_empty) {
        parts.pop();
    }
    Some(parts.into_iter().map(|p| String::from_utf8_lossy(&p).into_owned()).collect())
}

fn code_line(line: Option<&String>) -> Option<String> {
    let line = line?;
    let code = line.strip_suffix('\r').unwrap_or(line);
    (code.chars().count() <= MAX_CODE).then(|| code.to_string())
}

/// The annotations with each owner that has a file and a positive line given that line's text from the
/// checkout at `root` as `code`; a `code` on the clipboard is never kept.
pub fn quote(annotations: &[Value], root: Option<&str>) -> Vec<Value> {
    let mut files: Vec<(String, Option<Vec<String>>)> = vec![];
    annotations
        .iter()
        .map(|annotation| {
            let Some(map) = annotation.as_object() else { return annotation.clone() };
            let Some(Value::Array(owners)) = map.get("owners") else { return annotation.clone() };
            let owners: Vec<Value> = owners
                .iter()
                .map(|owner| {
                    let Some(owner) = owner.as_object() else { return owner.clone() };
                    let mut owner: Map<String, Value> = owner.clone();
                    owner.shift_remove("code");
                    let (Some(Value::String(file)), Some(line)) = (owner.get("file").cloned(), owner.get("line").cloned()) else { return Value::Object(owner) };
                    let Some(line) = line.as_i64().filter(|l| *l > 0 && (line.is_i64() || line.is_u64())) else { return Value::Object(owner) };
                    let at = match files.iter().position(|(f, _)| *f == file) {
                        Some(i) => i,
                        None => {
                            files.push((file.clone(), source_lines(root, &file)));
                            files.len() - 1
                        }
                    };
                    if let Some(code) = files[at].1.as_ref().and_then(|lines| code_line(lines.get(line as usize - 1))) {
                        owner.insert("code".into(), json!(code));
                    }
                    Value::Object(owner)
                })
                .collect();
            let mut map = map.clone();
            map.insert("owners".into(), Value::Array(owners));
            Value::Object(map)
        })
        .collect()
}

