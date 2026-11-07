//! The GitHub issues Claude drafts for an item: `issues/<slug>/NN-name.md`, front matter over a body.
//! Claude writes them; qm only reads them, apart from the revision snapshots under `.prev/` that every
//! read keeps (what it saw last and before that) and the pruning of snapshots whose file is gone.
//!
//! Reading is split so a command can decide before it writes anything: `load` only reads, and says
//! `None` when the files hold something this port can't judge exactly as Ruby's Psych does (a YAML
//! error, a date, an alias, a number written with a leading zero, a non-UTF-8 file …) — the command
//! then goes to Ruby. `Loaded::record` is what writes the snapshots, and is safe to repeat.

use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::{json, Value as Json};
use serde_yaml::Value as Yaml;

use crate::clock;
use crate::data_root;
use crate::psych::reads_as_non_string;
use crate::ruby_text::strip;

/// A revision younger than this is still being edited, not a version of its own.
const SETTLE: f64 = 120.0;
const KINDS: [&str; 2] = ["feature", "bug"];
const NO_DEPS: [&str; 3] = ["—", "-", "none"];

pub struct Issue {
    pub file: String,
    pub title: String,
    pub number: Option<u64>,
    /// 12 hex of the SHA-1 of the file, as the page tells revisions apart.
    pub version: String,
    pub data: Data,
}

pub struct Broken {
    pub file: String,
    pub error: String,
}

pub struct Loaded {
    dir: PathBuf,
    names: Vec<String>,
    pub issues: Vec<Issue>,
    pub errors: Vec<Broken>,
    seen: Vec<(String, String)>,
}

/// What `IssueDrafts#parse` makes of a file.
#[derive(Clone)]
pub struct Data {
    pub title: String,
    pub kind: String,
    pub group: String,
    pub parent: String,
    pub deps: Vec<String>,
    pub change: String,
    pub number: Option<u64>,
    pub body: String,
}

enum Parsed {
    Issue(Data),
    Error(String),
    Unsure,
}

pub fn dir(slug: &str) -> PathBuf {
    data_root::root().join("issues").join(slug)
}

/// `IssueDrafts#files`: the item's `*.md` files, dotfiles out, by name. `None` for a name that is not UTF-8.
pub fn files(slug: &str) -> Option<Vec<String>> {
    let Ok(entries) = std::fs::read_dir(dir(slug)) else { return Some(vec![]) };
    let mut names = vec![];
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name().into_string().ok()?;
        if name.ends_with(".md") && !name.starts_with('.') && name.len() > 3 {
            names.push(name);
        }
    }
    names.sort();
    Some(names)
}

/// Whether `load` can answer for this item (a pure check: nothing is written).
pub fn supported(slug: &str) -> bool {
    crate::cleanup_command::valid_slug(slug) && load(slug).is_some()
}

pub fn load(slug: &str) -> Option<Loaded> {
    let names = files(slug)?;
    let dir = dir(slug);
    let (mut issues, mut errors, mut seen) = (vec![], vec![], vec![]);
    for name in &names {
        let path = dir.join(name);
        if !std::fs::metadata(&path).ok()?.is_file() {
            return None;
        }
        let content = String::from_utf8(std::fs::read(&path).ok()?).ok()?;
        match parse(&content) {
            Parsed::Unsure => return None,
            Parsed::Error(error) => errors.push(Broken { file: name.clone(), error }),
            Parsed::Issue(data) => {
                snapshot_is_readable(&dir, name, &content)?;
                let version = crate::sha1::hex(content.as_bytes())[..12].to_string();
                issues.push(Issue { file: name.clone(), title: data.title.clone(), number: data.number, version, data });
                seen.push((name.clone(), content));
            }
        }
    }
    Some(Loaded { dir, names, issues, errors, seen })
}

impl Loaded {
    /// Keeps the snapshots as `IssueDrafts#read` does: what each file holds now, what it held before,
    /// and no snapshot for a file that is gone. Nothing here depends on an earlier call.
    pub fn record(&self) -> std::io::Result<()> {
        for (name, content) in &self.seen {
            remember(&self.dir, name, content)?;
        }
        prune(&self.dir, &self.names);
        Ok(())
    }
}

fn parse(content: &str) -> Parsed {
    let frontmatter = Regex::new(r"(?s)\A---[ \t]*\r?\n(.*?)\r?\n---[ \t]*(?:\r?\n|\z)(.*)\z").unwrap();
    let Some(caps) = frontmatter.captures(content) else {
        return Parsed::Error("no frontmatter (--- title: … ---) at the top".into());
    };
    let front = &caps[1];
    if unsure_text(front) {
        return Parsed::Unsure;
    }
    let value: Yaml = if front.trim().is_empty() {
        Yaml::Null
    } else {
        match serde_yaml::from_str(front) {
            Ok(v) => v,
            Err(_) => return Parsed::Unsure,
        }
    };
    let map = match value {
        Yaml::Mapping(m) => m,
        Yaml::Null | Yaml::Bool(false) => serde_yaml::Mapping::new(),
        Yaml::Tagged(_) => return Parsed::Unsure,
        _ => return Parsed::Error("frontmatter is not a set of fields".into()),
    };
    match fields(&map) {
        Ok(Ok(mut data)) => {
            let body = Regex::new(r"\A(?:[ \t]*\r?\n)+").unwrap().replace(&caps[2], "").into_owned();
            data.body = crate::ruby_text::rstrip(&body).to_string();
            Parsed::Issue(data)
        }
        Ok(Err(message)) => Parsed::Error(message),
        Err(()) => Parsed::Unsure,
    }
}

/// Everything `parse` checks past the YAML, in Ruby's order: `Ok(Err(message))` is the error entry it
/// would raise, `Err(())` a value this port can't read the way Psych does.
fn fields(map: &serde_yaml::Mapping) -> Result<Result<Data, String>, ()> {
    macro_rules! refuse {
        ($e:expr) => {
            match $e? {
                Ok(v) => v,
                Err(m) => return Ok(Err(m)),
            }
        };
    }
    let title = strip(&refuse!(scalar(map, "title"))).to_string();
    if title.is_empty() {
        return Ok(Err("title is missing".into()));
    }
    let kind = strip(&refuse!(scalar(map, "kind"))).to_string();
    if !KINDS.contains(&kind.as_str()) {
        return Ok(Err(format!("kind must be {}, not '{kind}'", KINDS.join(" or "))));
    }
    let number = refuse!(number(map.get("number")));
    let group = strip(&refuse!(scalar(map, "group"))).to_string();
    let parent = strip(&refuse!(scalar(map, "parent"))).to_string();
    let deps = refuse!(deps(map.get("deps")));
    let change = strip(&refuse!(scalar(map, "change"))).to_string();
    Ok(Ok(Data { title, kind, group, parent, deps, change, number, body: String::new() }))
}

/// `value.to_s` as Ruby would give it for what Psych would have loaded; `Err(())` when that isn't
/// something this port can reproduce.
fn text(v: &Yaml) -> Result<String, ()> {
    match v {
        Yaml::Null => Ok(String::new()),
        Yaml::String(s) if s.is_empty() => Ok(String::new()), // quoted, or empty and so null: `to_s` is "" either way
        Yaml::Bool(b) => Ok(b.to_string()),
        Yaml::Number(n) if n.is_i64() || n.is_u64() => Ok(n.to_string()),
        Yaml::String(s) if reads_as_non_string(s) == Some(false) => Ok(s.clone()),
        _ => Err(()),
    }
}

fn scalar(map: &serde_yaml::Mapping, key: &str) -> Result<Result<String, String>, ()> {
    match map.get(key) {
        Some(Yaml::Sequence(_)) | Some(Yaml::Mapping(_)) => Ok(Err(format!("{key} must be text"))),
        Some(v) => text(v).map(Ok),
        None => Ok(Ok(String::new())),
    }
}

fn number(v: Option<&Yaml>) -> Result<Result<Option<u64>, String>, ()> {
    let Some(v) = v else { return Ok(Ok(None)) };
    if matches!(v, Yaml::Sequence(_) | Yaml::Mapping(_)) {
        return Err(());
    }
    let shown = text(v)?;
    if strip(&shown).is_empty() {
        return Ok(Ok(None));
    }
    let digits = strip(&shown).strip_prefix('#').unwrap_or(strip(&shown));
    let integer_or_string = matches!(v, Yaml::Number(_) | Yaml::String(_));
    if integer_or_string && !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        if digits.len() > 18 {
            return Err(());
        }
        let n: u64 = digits.parse().map_err(|_| ())?;
        if n > 0 {
            return Ok(Ok(Some(n)));
        }
    }
    Ok(Err(format!("number must be the issue's number, not '{shown}'")))
}

fn deps(v: Option<&Yaml>) -> Result<Result<Vec<String>, String>, ()> {
    let list: Vec<&Yaml> = match v {
        None | Some(Yaml::Null) => vec![],
        Some(Yaml::Sequence(items)) => items.iter().collect(),
        Some(Yaml::Mapping(_)) => return Ok(Err("deps must be a list".into())),
        Some(other) => vec![other],
    };
    if list.iter().any(|d| matches!(d, Yaml::Sequence(_) | Yaml::Mapping(_))) {
        return Ok(Err("deps must be a list of text".into()));
    }
    let mut out = vec![];
    for dep in list {
        let dep = strip(&text(dep)?).to_string();
        if !dep.is_empty() && !NO_DEPS.contains(&dep.as_str()) {
            out.push(dep);
        }
    }
    Ok(Ok(out))
}

/// Front matter whose reading by Psych's safe_load this port doesn't reproduce: anchors, aliases,
/// tags and merge keys; directives and document markers; and any word shaped like a number that Psych
/// reads differently from the YAML parser here (floats, `1_000`, `010`, `0x1f`, dates, times, `.inf`).
fn unsure_text(front: &str) -> bool {
    let marks = Regex::new(r"(?m)(^|[\s\[{,])([&*!]|<<)|^%|^(---|\.\.\.)").unwrap();
    if marks.is_match(front) {
        return true;
    }
    let canonical = Regex::new(r"^-?(0|[1-9][0-9]{0,17})$").unwrap();
    let numberish = Regex::new(
        r"^(?:[-+]?[0-9_,.:]*[0-9][0-9_,.:]*|[-+]?[0-9_,.]*[eE][-+]?[0-9]+|[-+]?0[xXoObB][0-9a-fA-F_]+|[-+]?\.(?:inf|Inf|INF|nan|NaN|NAN)|\d{4}-\d.*)$",
    )
    .unwrap();
    front
        .split(|c: char| c.is_whitespace() || matches!(c, '[' | ']' | '{' | '}' | ','))
        .filter(|t| t.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '+' | '-' | '.')))
        .any(|t| !canonical.is_match(t) && (numberish.is_match(t) || t.contains(',')))
}

fn snapshot_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(".prev").join(format!("{name}.json"))
}

/// The snapshot as `remember` reads it: a JSON object whose "seen" is text, else none.
fn read_snapshot(dir: &Path, name: &str) -> Option<serde_json::Map<String, Json>> {
    let text = std::fs::read_to_string(snapshot_path(dir, name)).ok()?;
    match serde_json::from_str::<Json>(&text).ok()? {
        Json::Object(m) if m.get("seen").is_some_and(Json::is_string) => Some(m),
        _ => None,
    }
}

/// A snapshot whose time of reading isn't a number would be read by Ruby with `to_f` on whatever it
/// is; only numbers (and nothing) are reproduced.
fn snapshot_is_readable(dir: &Path, name: &str, content: &str) -> Option<()> {
    match read_snapshot(dir, name) {
        Some(m) if m["seen"].as_str() != Some(content) => matches!(m.get("seen_at"), None | Some(Json::Null) | Some(Json::Number(_))).then_some(()),
        _ => Some(()),
    }
}

fn remember(dir: &Path, name: &str, content: &str) -> std::io::Result<serde_json::Map<String, Json>> {
    let old = read_snapshot(dir, name);
    if let Some(m) = old.as_ref().filter(|m| m["seen"].as_str() == Some(content)) {
        return Ok(m.clone());
    }
    let now = clock::now_f64();
    let seen_at = old.as_ref().and_then(|m| m.get("seen_at")).and_then(Json::as_f64).unwrap_or(0.0);
    let settled = old.is_none() || now - seen_at >= SETTLE;
    let previous = match &old {
        None => Json::Null,
        Some(m) if settled => m["seen"].clone(),
        Some(m) => m.get("previous").cloned().unwrap_or(Json::Null),
    };
    let snapshot = json!({"v": 1, "seen": content, "seen_at": now, "previous": previous});
    write_snapshot(&snapshot_path(dir, name), &snapshot)?;
    Ok(snapshot.as_object().cloned().unwrap_or_default())
}

fn write_snapshot(path: &Path, snapshot: &Json) -> std::io::Result<()> {
    std::fs::create_dir_all(path.parent().unwrap())?;
    let temp = format!("{}.{}.{:?}.tmp", path.display(), std::process::id(), std::thread::current().id());
    std::fs::write(&temp, serde_json::to_string(snapshot).unwrap())?;
    std::fs::rename(&temp, path)
}

/// A name `IssueDrafts` takes for an issue file: `\A[^.\/][^\/]*\.md\z`.
pub fn is_issue_file(name: &str) -> bool {
    name.len() >= 4 && !name.starts_with('.') && !name.contains('/') && name.ends_with(".md")
}

/// `IssueDrafts#acknowledge`: the file's current revision stops reading as changed. `Ok(false)` when
/// there is no such file or it doesn't parse; `Err` for what this port can't judge as Ruby does.
pub fn acknowledge(slug: &str, name: &str) -> Result<bool, ()> {
    let dir = dir(slug);
    let path = dir.join(name);
    if !path.is_file() {
        return Ok(false);
    }
    let content = String::from_utf8(std::fs::read(&path).map_err(|_| ())?).map_err(|_| ())?;
    match parse(&content) {
        Parsed::Unsure => return Err(()),
        Parsed::Error(_) => return Ok(false),
        Parsed::Issue(_) => {}
    }
    snapshot_is_readable(&dir, name, &content).ok_or(())?;
    let mut snapshot = remember(&dir, name, &content).map_err(|_| ())?;
    snapshot.insert("previous".into(), Json::String(content));
    snapshot.insert("acked".into(), Json::Bool(true));
    write_snapshot(&snapshot_path(&dir, name), &Json::Object(snapshot)).map_err(|_| ())?;
    Ok(true)
}

/// A snapshot whose file is gone would hand a later file of that name a previous version it never had.
fn prune(dir: &Path, names: &[String]) {
    let Ok(entries) = std::fs::read_dir(dir.join(".prev")) else { return };
    for entry in entries.filter_map(|e| e.ok()) {
        let file = entry.file_name().to_string_lossy().into_owned();
        if let Some(stem) = file.strip_suffix(".json") {
            if !file.starts_with('.') && !names.iter().any(|n| n == stem) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// `Drafts#issue_comment_counts`: unsent comments per issue file, from `drafts/<slug>.json`.
/// `None` when an entry isn't an object (Ruby would raise on it).
pub fn unsent_comments(slug: &str) -> Option<std::collections::HashMap<String, usize>> {
    let mut counts = std::collections::HashMap::new();
    for entry in crate::drafts::entries(slug)? {
        if entry.get("kind").and_then(Json::as_str) == Some("issue-comment") {
            if let Some(file) = entry.get("file").and_then(Json::as_str) {
                *counts.entry(file.to_string()).or_insert(0) += 1;
            }
        }
    }
    Some(counts)
}

/// `IssueDrafts#delete`: removes one issue file and its snapshot, drops its unsent comments, and
/// takes the directory with the last file. True when the file was there.
pub fn delete(slug: &str, name: &str) -> std::io::Result<bool> {
    let dir = dir(slug);
    let path = dir.join(name);
    let existed = path.is_file();
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(snapshot_path(&dir, name));
    crate::drafts::forget_issue(slug, name)?;
    if files(slug).is_some_and(|f| f.is_empty()) {
        let _ = std::fs::remove_dir_all(dir.join(".prev"));
        let _ = std::fs::remove_dir(&dir);
    }
    Ok(existed)
}

/// `IssueDrafts.signature`: every `issues/*/*.md`'s path, size and mtime, so qm's own `.prev`
/// snapshot writes never move it; "0" while there are none.
pub fn signature() -> String {
    let base = data_root::root().join("issues");
    let mut files: Vec<PathBuf> = vec![];
    if let Ok(dirs) = std::fs::read_dir(&base) {
        for d in dirs.filter_map(|e| e.ok()).filter(|e| !e.file_name().to_string_lossy().starts_with('.') && e.path().is_dir()) {
            if let Ok(inner) = std::fs::read_dir(d.path()) {
                files.extend(inner.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "md") && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) && p.is_file()));
            }
        }
    }
    files.sort_by_key(|p| p.to_string_lossy().into_owned());
    if files.is_empty() {
        return "0".into();
    }
    let prefix = base.to_string_lossy().into_owned();
    let stats: Vec<String> = files
        .iter()
        .map(|p| match (std::fs::metadata(p), crate::drafts::mtime_rational(p)) {
            (Ok(m), Some(t)) => format!("{}:{}:{t}", p.to_string_lossy().strip_prefix(&prefix).unwrap_or_default(), m.len()),
            _ => String::new(),
        })
        .collect();
    crate::sha1::hex(stats.join("|").as_bytes())[..12].to_string()
}

/// `IssueDrafts#read`: `{"issues", "errors"}` — the files that parse (not yet created on GitHub first), the
/// ones that don't, each issue with the revision snapshot's verdict. Writes the snapshots as every read
/// does. `None` for what only Ruby reads exactly.
pub fn read(slug: &str) -> Option<Json> {
    let loaded = load(slug)?;
    let mut issues: Vec<(bool, Json)> = vec![];
    for (issue, (name, content)) in loaded.issues.iter().zip(loaded.seen.iter()) {
        let snapshot = remember(&loaded.dir, name, content).ok()?;
        let acknowledged = snapshot.get("acked") == Some(&Json::Bool(true));
        let previous = if acknowledged {
            Json::Null
        } else {
            match snapshot.get("previous") {
                Some(Json::String(text)) => match parse(text) {
                    Parsed::Issue(d) => json!({"title": d.title, "body": d.body}),
                    Parsed::Error(_) => Json::Null,
                    Parsed::Unsure => return None,
                },
                _ => Json::Null,
            }
        };
        let d = &issue.data;
        issues.push((d.number.is_some(), json!({"file": issue.file, "title": d.title, "kind": d.kind, "group": d.group, "parent": d.parent, "deps": d.deps, "change": d.change, "number": d.number, "created": d.number.is_some(), "body": d.body, "acknowledged": acknowledged, "previous": previous, "version": issue.version})));
    }
    prune(&loaded.dir, &loaded.names);
    issues.sort_by_key(|(created, _)| *created as u8);
    let errors: Vec<Json> = loaded.errors.iter().map(|e| json!({"file": e.file, "error": e.error})).collect();
    Some(json!({"issues": issues.into_iter().map(|(_, v)| v).collect::<Vec<_>>(), "errors": errors}))
}
