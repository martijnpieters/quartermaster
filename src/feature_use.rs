use std::io::Write;

use crate::clock;
use crate::data_root;

/// One `{v, at, kind, name}` line per use — kind and name only, never arguments. Best effort.
pub fn record(kind: &str, name: &str) {
    let line = serde_json::json!({"v": 1, "at": clock::now_utc_iso(), "kind": kind, "name": name});
    let path = data_root::root().join("feature-use.jsonl");
    let write = || -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        writeln!(f, "{line}")
    };
    if let Err(e) = write() {
        if e.kind() == std::io::ErrorKind::NotFound {
            let _ = std::fs::create_dir_all(data_root::root());
            if write().is_ok() {
                return;
            }
        }
        data_root::log_error("feature-use", "IOError", &e.to_string());
    }
}

/// A `qm` run by its command word; hooks and `QM_SKIP` runs are capture plumbing, not uses.
pub fn command(word: Option<&str>) {
    let Some(word) = word else { return };
    if word.starts_with("hook-") || std::env::var_os("QM_SKIP").is_some() {
        return;
    }
    record("cli", word);
}
