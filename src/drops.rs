//! Files dropped onto a GUI terminal (`Drops`). A browser never tells a page where a dropped file lives,
//! so the page uploads it and types the path this returns into the chat. Kept a week, then pruned on
//! the next drop.

use crate::data_root;

pub const MAX_BYTES: usize = 32 * 1024 * 1024;
const KEEP_SECONDS: u64 = 7 * 24 * 60 * 60;

/// The name reduced to characters that need no shell quoting, so the typed path is one word.
fn safe_name(name: &str) -> String {
    let base = name.rsplit('/').find(|p| !p.is_empty()).unwrap_or("");
    let mut flat = String::new();
    let mut dashing = false;
    for c in base.chars() {
        if c.is_ascii_alphanumeric() || "._-".contains(c) {
            flat.push(c);
            dashing = false;
        } else if !dashing {
            flat.push('-');
            dashing = true;
        }
    }
    let flat = flat.trim_start_matches(['-', '.']).to_string();
    if flat.is_empty() {
        return "drop".into();
    }
    let chars: Vec<char> = flat.chars().collect();
    chars[chars.len().saturating_sub(80)..].iter().collect()
}

/// The saved file's absolute path, or the `ArgumentError` message.
pub fn save(name: &str, bytes: &[u8]) -> Result<String, String> {
    if bytes.is_empty() {
        return Err("dropped file is empty".into());
    }
    if bytes.len() > MAX_BYTES {
        return Err(format!("dropped file is over {} MB", MAX_BYTES / 1024 / 1024));
    }
    let dir = data_root::root().join("drops");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(KEEP_SECONDS);
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.filter_map(|e| e.ok()) {
            if e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < cutoff) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let path = dir.join(format!("{stamp}-{}-{}", crate::gui_draft_actions::random_hex(3), safe_name(name)));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}
