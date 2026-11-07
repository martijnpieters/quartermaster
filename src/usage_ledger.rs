//! Token totals per session — its transcript plus its sub-agent transcripts — kept permanently in
//! `usage-cache.json`. Claude Code prunes transcripts after a few weeks; the ledger is what still
//! knows what an expired session spent. It is a record, not a cache: nothing here prunes or rebuilds it.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::data_root;
use crate::error::QmError;
use crate::session_log::Session;
use crate::transcript_usage::{self as tu, Buckets, SYSTEM_CALL_ERROR};

pub struct SessionUsage {
    pub buckets: Buckets,
    pub agents: i64,
    pub known: bool,
}

pub struct UsageLedger {
    path: PathBuf,
    sessions: Option<Map<String, Value>>,
    dirty: bool,
}

/// The `sessions` of the ledger file; a missing, unreadable or malformed file reads as empty.
fn read_sessions(path: &Path) -> Map<String, Value> {
    match std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        Some(Value::Object(mut m)) => match m.remove("sessions") {
            Some(Value::Object(s)) => s,
            _ => Map::new(),
        },
        _ => Map::new(),
    }
}

/// `<transcript dir>/<session id>/subagents/**/*.jsonl`, sorted; hidden entries and symlinked
/// directories are skipped, as `Dir.glob` does.
fn subagent_transcripts(transcript: &Path, session_id: &str) -> Vec<PathBuf> {
    let mut found = vec![];
    let root = transcript.parent().unwrap_or(Path::new("")).join(session_id).join("subagents");
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.filter_map(|e| e.ok()) {
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = e.path();
            let is_real_dir = std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir());
            if is_real_dir {
                stack.push(path);
            } else if path.extension().is_some_and(|x| x == "jsonl") && path.is_file() {
                found.push(path);
            }
        }
    }
    found.sort_by_key(|p| p.to_string_lossy().into_owned());
    found
}

impl UsageLedger {
    pub fn new() -> UsageLedger {
        UsageLedger { path: data_root::root().join("usage-cache.json"), sessions: None, dirty: false }
    }

    fn sessions(&mut self) -> &mut Map<String, Value> {
        let path = &self.path;
        self.sessions.get_or_insert_with(|| read_sessions(path))
    }

    /// The session's totals; known is false only for an expired session nobody tallied.
    pub fn session_usage(&mut self, session: &Session) -> Result<SessionUsage, QmError> {
        let entry = self.sessions().get(&session.id).cloned();
        let entry = match session.transcript() {
            Some(transcript) => self.refresh(&session.id, &transcript, entry)?,
            None => entry,
        };
        let Some(entry) = entry else { return Ok(SessionUsage { buckets: Map::new(), agents: 0, known: session.live }) };
        let files = entry.get("files").and_then(Value::as_object).ok_or_else(|| QmError::new("NoMethodError", "undefined method 'values'"))?;
        let mut buckets = Map::new();
        for state in files.values() {
            let state = state.as_object().ok_or_else(|| QmError::new("TypeError", "file state is not a hash"))?;
            tu::merge(&mut buckets, &tu::totals(state)?)?;
        }
        Ok(SessionUsage { buckets, agents: files.len() as i64 - 1, known: true })
    }

    fn refresh(&mut self, session_id: &str, transcript: &Path, entry: Option<Value>) -> Result<Option<Value>, QmError> {
        let Ok(meta) = std::fs::metadata(transcript) else { return Ok(entry) };
        let signature = json!([transcript.to_string_lossy(), meta.len() as i64, tu::mtime_f64(&meta)?]);
        if let Some(e) = &entry {
            if e.get("main") == Some(&signature) {
                return Ok(entry);
            }
        }
        let mut files = entry.as_ref().and_then(|e| e.get("files")).and_then(Value::as_object).cloned().unwrap_or_default();
        let mut paths = vec![transcript.to_path_buf()];
        paths.extend(subagent_transcripts(transcript, session_id));
        for path in paths {
            let key = path.to_string_lossy().into_owned();
            let state = files.get(&key).and_then(Value::as_object);
            match tu::scan(&path, state) {
                Ok(next) => {
                    files.insert(key, Value::Object(next));
                }
                Err(e) if e.class == SYSTEM_CALL_ERROR => continue, // vanished between the glob and the read
                Err(e) => return Err(e),
            }
        }
        self.dirty = true;
        let fresh = json!({ "main": signature, "files": Value::Object(files) });
        self.sessions().insert(session_id.to_string(), fresh.clone());
        Ok(Some(fresh))
    }

    /// Writes the ledger when a lookup changed it: atomic, and merged over what another qm process
    /// saved meanwhile (its sessions aren't dropped).
    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        let mut merged = read_sessions(&self.path);
        for (k, v) in self.sessions().clone() {
            merged.insert(k, v);
        }
        let payload = json!({ "v": 1, "sessions": Value::Object(merged) });
        let write = || -> std::io::Result<()> {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = PathBuf::from(format!("{}.{}.tmp", self.path.display(), std::process::id()));
            std::fs::write(&tmp, payload.to_string())?;
            std::fs::rename(&tmp, &self.path)
        };
        match write() {
            Ok(()) => self.dirty = false,
            Err(e) => data_root::log_error("usage-ledger", "IOError", &e.to_string()),
        }
    }
}

impl Default for UsageLedger {
    fn default() -> Self {
        Self::new()
    }
}
