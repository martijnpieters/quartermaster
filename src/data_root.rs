use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use crate::clock;

const CONFIG_SEED: &str = include_str!("config_seed.yml");

pub fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

/// Ruby's `File.expand_path`: `~` and relative paths resolved, `.` and `..` folded.
pub fn expand_path(path: &str) -> PathBuf {
    let home = home();
    let spelled = if path == "~" {
        home
    } else if let Some(rest) = path.strip_prefix("~/") {
        format!("{home}/{rest}")
    } else {
        path.to_string()
    };
    let p = Path::new(&spelled);
    let absolute = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(p) };
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `File.realpath(File.expand_path(path))`, else the expanded path.
pub fn normalize(path: &str) -> PathBuf {
    let expanded = expand_path(path);
    std::fs::canonicalize(&expanded).unwrap_or(expanded)
}

pub fn root() -> PathBuf {
    match std::env::var("QM_ROOT") {
        Ok(v) => expand_path(&v),
        Err(_) => expand_path("~/.claude/quartermaster"),
    }
}

pub fn transcripts_dir() -> PathBuf {
    match std::env::var("QM_TRANSCRIPTS_DIR") {
        Ok(v) => expand_path(&v),
        Err(_) => expand_path("~/.claude/projects"),
    }
}

pub fn work_items_dir() -> PathBuf {
    root().join("work-items")
}

pub fn config_file() -> PathBuf {
    root().join("config.yml")
}

pub fn ensure_setup() -> std::io::Result<()> {
    fs::create_dir_all(work_items_dir())?;
    let config = config_file();
    if !config.exists() {
        fs::write(config, CONFIG_SEED)?;
    }
    Ok(())
}

pub fn log_error(context: &str, class: &str, message: &str) {
    let _ = ensure_setup();
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(root().join("qm.log")) {
        let _ = writeln!(f, "{} [{}] {}: {}", clock::now_iso(), context, class, message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_path_folds_dots_and_resolves_home() {
        std::env::set_var("HOME", "/home/u");
        assert_eq!(expand_path("~/a/../b/./c"), PathBuf::from("/home/u/b/c"));
        assert_eq!(expand_path("/x//y/.."), PathBuf::from("/x"));
        assert_eq!(expand_path("~"), PathBuf::from("/home/u"));
    }
}
