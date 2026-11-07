//! A chat's tmux session is named after its item, so when its session moves to another item the
//! pane must follow: else the new item shows no pane and the old one keeps a chat that is no
//! longer its. Only a pane whose process is that session's claude moves.

use crate::session_log::SessionLog;
use crate::tmux::Tmux;

/// Renames the vacated item's tmux session to `to` when it runs `session_id`. Returns a warning
/// for the frontends when the pane can't follow because `to` already has a chat running.
pub fn follow(session_id: &str, from: &[String], to: &str) -> Option<String> {
    let log = SessionLog::read();
    let session = log.session_for(session_id)?;
    let pid = session.pid()?;
    if session.aside_for().is_some() {
        return None;
    }
    let tmux = Tmux::new();
    for old in from {
        if tmux.pane_pid(old) != Some(pid) {
            continue;
        }
        if tmux.live(to) {
            return Some(format!("{old}'s chat pane stays named {old}: {to} already has a chat running"));
        }
        tmux.rename(old, to);
    }
    None
}
