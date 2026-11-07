//! `qm grep PHRASE`, printed as the Ruby command prints it.

use chrono::{DateTime, Local};

use crate::cli_format::puts;
use crate::outcome::{abort, done, Outcome};
use crate::transcript_search::{self, Failure};

pub fn grep(args: &[String]) -> Outcome {
    let query = args.join(" ");
    if crate::ruby_text::strip(&query).is_empty() {
        return abort("usage: qm grep PHRASE");
    }
    let found = match transcript_search::call(&query) {
        Ok(f) => f,
        Err(Failure::Abort(message)) => return abort(&message),
        Err(Failure::Ruby) => return Outcome::Delegate,
    };
    if found.groups.is_empty() {
        return done("No transcript mentions that.\n".into());
    }
    let tty = unsafe { libc::isatty(1) } == 1;
    let bold = |s: &str| if tty { format!("\x1b[1m{s}\x1b[0m") } else { s.to_string() };
    let mut out = String::new();
    for group in &found.groups {
        match &group.slug {
            Some(slug) => out.push_str(&puts(&format!("{slug}  {}  ({})", group.title, group.state.clone().unwrap_or_default()))),
            None => out.push_str("unlinked\n"),
        }
        for hit in &group.sessions {
            let date = match &hit.date {
                Some(d) => match DateTime::parse_from_rfc3339(d) {
                    Ok(t) => t.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string(),
                    Err(_) => return Outcome::Delegate, // Time.parse reads more shapes than RFC 3339
                },
                None => "????-??-?? --:--".to_string(),
            };
            let who = if hit.role == "user" { "you" } else { "claude" };
            let hits = if hit.hits > 1 { format!(" ({} hits)", hit.hits) } else { String::new() };
            out.push_str(&puts(&format!("  {date}  {}{hits}", hit.id)));
            out.push_str(&puts(&format!("    {who}: {}{}{}", hit.before, bold(&hit.matched), hit.after)));
        }
    }
    if found.more > 0 {
        out.push_str(&puts(&format!("… {} more transcript(s) not read; narrow the phrase", found.more)));
    }
    out.push_str("Resume: qm resume SLUG --session ID (unlinked: claude --resume ID from its project dir)\n");
    done(out)
}
