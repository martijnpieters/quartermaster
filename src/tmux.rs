//! The tmux server the GUI's chats live in: a dedicated socket (`tmux -L qm`, `QM_TMUX_SOCKET` for
//! tests) keeping it apart from any tmux the user runs. Each chat is its own tmux session holding
//! one window, both named after the item's slug.
//!
//! Looking at a chat, renaming it and starting one (`open`, on a click) are ported. Typing into one,
//! respawning and killing stay in Ruby: nothing types into the user's chat without their click.

use std::process::{Command, Stdio};

const DEFAULT_SOCKET: &str = "qm";

/// Strips Claude Code's own variables before exec: a child that inherits the nested-session marker
/// stops saving its transcript.
const SCRUB: &str = "for v in $(env | sed -n \"s/^\\(CLAUDE[A-Za-z0-9_]*\\)=.*/\\1/p\"); do unset \"$v\"; done; exec \"$@\"";

/// Re-applied on every open: no status bar, wheel scrolls, no escape delay, no prefix key.
const OPTIONS: [[&str; 2]; 7] = [["status", "off"], ["mouse", "on"], ["escape-time", "10"], ["prefix", "None"], ["history-limit", "50000"], ["window-size", "latest"], ["terminal-features[90]", "xterm*:hyperlinks"]];

pub struct Tmux {
    socket: String,
}

/// tmux reserves some characters in target names; slugs never carry them.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphanumeric()) && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

impl Tmux {
    pub fn new() -> Tmux {
        Tmux { socket: std::env::var("QM_TMUX_SOCKET").unwrap_or_else(|_| DEFAULT_SOCKET.to_string()) }
    }

    /// TMUX unset, so a GUI started inside tmux still talks to our server. (success, stdout).
    fn run(&self, args: &[&str]) -> (bool, String) {
        match Command::new("tmux").args(["-L", &self.socket]).args(args).env_remove("TMUX").stdin(Stdio::null()).stderr(Stdio::null()).output() {
            Ok(out) => (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned()),
            Err(_) => (false, String::new()),
        }
    }

    pub fn socket(&self) -> String {
        self.socket.clone()
    }

    pub fn sessions(&self) -> Vec<String> {
        match self.run(&["list-sessions", "-F", "#{session_name}"]) {
            (true, out) => out.split('\n').filter(|l| !l.is_empty()).map(String::from).collect(),
            _ => vec![],
        }
    }

    pub fn live(&self, name: &str) -> bool {
        valid_name(name) && self.run(&["has-session", "-t", &format!("={name}")]).0
    }

    /// Starts `argv` in a new detached session `name` at `cwd` (`Tmux#open`): false when it already
    /// runs, an error text when tmux could not start it.
    pub fn open(&self, name: &str, cwd: &str, argv: &[String], env: &[(String, String)]) -> Result<bool, String> {
        if !valid_name(name) {
            return Err(format!("not a usable tmux name: {}", crate::ruby_text::inspect_ascii(name).ok_or_else(String::new)?));
        }
        if self.live(name) {
            return Ok(false);
        }
        let cwd = if std::path::Path::new(cwd).is_dir() { cwd.to_string() } else { crate::data_root::home() };
        let mut args: Vec<String> = ["new-session", "-d", "-s", name, "-n", name, "-c", &cwd, "-x", "160", "-y", "48", "sh", "-c", SCRUB, "qm-chat", "env"].iter().map(|s| s.to_string()).collect();
        args.extend(env.iter().map(|(k, v)| format!("{k}={v}")));
        args.extend(argv.iter().cloned());
        for pair in OPTIONS {
            args.extend([";", "set-option", "-g", pair[0], pair[1]].map(String::from));
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = Command::new("tmux").args(["-L", &self.socket]).args(&refs).env_remove("TMUX").stdin(Stdio::null()).stderr(Stdio::piped()).stdout(Stdio::piped()).output().map_err(|e| e.to_string())?;
        if out.status.success() {
            return Ok(true);
        }
        let mixed = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        Err(format!("tmux could not start {name}: {}", crate::ruby_text::strip(&mixed)))
    }

    /// Starts `argv` afresh in the session's pane, killing whatever still runs (`Tmux#respawn`).
    pub fn respawn(&self, name: &str, cwd: &str, argv: &[String], env: &[(String, String)]) -> Result<(), String> {
        let cwd = if std::path::Path::new(cwd).is_dir() { cwd.to_string() } else { crate::data_root::home() };
        let mut args: Vec<String> = ["respawn-pane", "-t", &format!("={name}:"), "-k", "-c", &cwd, "sh", "-c", SCRUB, "qm-chat", "env"].iter().map(|s| s.to_string()).collect();
        args.extend(env.iter().map(|(k, v)| format!("{k}={v}")));
        args.extend(argv.iter().cloned());
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        if valid_name(name) && self.run(&refs).0 {
            return Ok(());
        }
        Err(format!("tmux could not restart {name}"))
    }

    /// `remain-on-exit`: the window (and every client attached to it) outlives its program.
    pub fn keep_on_exit(&self, name: &str, keep: bool) -> bool {
        valid_name(name) && self.run(&["set-option", "-w", "-t", &format!("={name}:"), "remain-on-exit", if keep { "on" } else { "off" }]).0
    }

    /// Types text into the session as if from the keyboard (`-l`: no key-name lookup).
    pub fn send_text(&self, name: &str, text: &str) -> bool {
        valid_name(name) && self.run(&["send-keys", "-t", &format!("={name}:"), "-l", text]).0
    }

    /// Pastes text into the session as one block (a bracketed paste through a named buffer), so a
    /// message that spans lines doesn't submit at every newline.
    pub fn paste(&self, name: &str, text: &str) -> bool {
        use std::io::Write;
        if !valid_name(name) {
            return false;
        }
        let buffer = format!("qm-paste-{name}");
        let Ok(mut child) = Command::new("tmux").args(["-L", &self.socket, "load-buffer", "-b", &buffer, "-"]).env_remove("TMUX").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else { return false };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        if !child.wait().is_ok_and(|s| s.success()) {
            return false;
        }
        self.run(&["paste-buffer", "-t", &format!("={name}:"), "-p", "-d", "-b", &buffer]).0
    }

    pub fn send_enter(&self, name: &str) -> bool {
        valid_name(name) && self.run(&["send-keys", "-t", &format!("={name}:"), "Enter"]).0
    }

    /// Whether the pane's program has ended and the window stays (`remain-on-exit`).
    pub fn pane_dead(&self, name: &str) -> bool {
        if !valid_name(name) {
            return false;
        }
        let (ok, out) = self.run(&["display-message", "-p", "-t", &format!("={name}:"), "#{pane_dead}"]);
        ok && crate::ruby_text::strip(&out) == "1"
    }

    /// Ends the session (`Tmux#kill`); false when it was not running.
    pub fn kill(&self, name: &str) -> bool {
        valid_name(name) && self.run(&["kill-session", "-t", &format!("={name}")]).0
    }

    /// The process the session's pane runs (the chat, once its launcher exec'd), or None.
    pub fn pane_pid(&self, name: &str) -> Option<i64> {
        if !valid_name(name) {
            return None;
        }
        let (ok, out) = self.run(&["display-message", "-p", "-t", &format!("={name}:"), "#{pane_pid}"]);
        let pid: i64 = out.trim().parse().unwrap_or(0);
        (ok && pid > 0).then_some(pid)
    }

    /// The directory of the session's pane, or None.
    pub fn pane_path(&self, name: &str) -> Option<String> {
        if !valid_name(name) {
            return None;
        }
        let (ok, out) = self.run(&["display-message", "-p", "-t", &format!("={name}:"), "#{pane_current_path}"]);
        let path = crate::ruby_text::strip(&out).to_string();
        (ok && !path.is_empty()).then_some(path)
    }

    /// Renames session `old` and its window to `new`, so the attach target follows the chat. False
    /// when either name is unusable, `old` isn't running, or `new` already is.
    pub fn rename(&self, old: &str, new: &str) -> bool {
        if !(valid_name(old) && valid_name(new)) || self.live(new) {
            return false;
        }
        self.run(&["rename-session", "-t", &format!("={old}"), new]).0 && self.run(&["rename-window", "-t", &format!("={new}:"), new]).0
    }
}

impl Default for Tmux {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::valid_name;

    #[test]
    fn names_are_slugs() {
        for ok in ["a", "fix-login-bug", "a_b-1", "9lives"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", "-a", "_a", "a b", "a.b", "a:b", "=a", "a\n", "é"] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }
}
