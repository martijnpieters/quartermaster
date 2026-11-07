use std::ffi::OsString;
use std::io::{IsTerminal, Read, Write};

use crate::data_root;
use crate::delegate;
use crate::error::QmError;
use crate::feature_use;
use crate::hook;
use crate::session_log::SessionLog;
use crate::session_rows;
use crate::outcome::Outcome;
use crate::session_commands;
use crate::work_item_commands;

fn out(text: &str) {
    let _ = std::io::stdout().write_all(text.as_bytes());
}

/// `-n N` (or nothing) is the only form handled here; anything else is Ruby's to parse and refuse.
fn limit(args: &[String]) -> Option<usize> {
    match args {
        [] => Some(15),
        [flag, n] if flag == "-n" => n.parse().ok(),
        [joined] => joined.strip_prefix("-n")?.parse().ok(),
        _ => None,
    }
}

/// Hooks must never block or break a session: swallow everything, log, stay silent.
fn guard_hook(context: &str, run: impl FnOnce() -> Result<Option<String>, QmError>) {
    if std::env::var_os("QM_SKIP").is_some() {
        return;
    }
    match run() {
        Ok(Some(context)) => {
            out(&context);
            if !context.ends_with('\n') {
                out("\n");
            }
        }
        Ok(None) => {}
        Err(e) => data_root::log_error(context, e.class, &e.message),
    }
}

fn read_input() -> Result<serde_json::Value, QmError> {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text).map_err(QmError::from)?;
    Ok(serde_json::from_str(&text)?)
}

fn abort(message: &str) -> i32 {
    eprintln!("{message}");
    1
}

pub fn run(raw: Vec<OsString>) -> i32 {
    let Some(args) = raw.iter().map(|a| a.to_str().map(String::from)).collect::<Option<Vec<String>>>() else {
        delegate::exec_ruby(&raw) // arguments that are not UTF-8 are Ruby's to read
    };
    let command = args.first().map(String::as_str);
    let rest = args.get(1..).unwrap_or(&[]);
    if let Some(word) = command {
        if let Some(outcome) = work_item_commands::run(word, rest).or_else(|| session_commands::run(word, rest)) {
            return match outcome {
                Outcome::Delegate => delegate::exec_ruby(&raw),
                Outcome::Done { code, stdout, stderr } => {
                    feature_use::command(command);
                    out(&stdout);
                    let _ = std::io::stderr().write_all(stderr.as_bytes());
                    code
                }
            };
        }
    }
    match command {
        Some("bench-run") => match rest.first() {
            Some(dir) => crate::benchmark_pipeline::main(dir),
            None => abort("usage: qm bench-run RUN_DIR"),
        },
        Some("hook-session-start") => {
            guard_hook("hook-session-start", || hook::session_started(&read_input()?));
            0
        }
        Some("hook-session-end") => {
            guard_hook("hook-session-end", || {
                hook::session_ended(&read_input()?)?;
                Ok(None)
            });
            0
        }
        Some("sessions") if limit(rest).is_some() => {
            feature_use::command(command);
            out(&session_rows::print(SessionLog::read().sessions(), limit(rest).unwrap()));
            0
        }
        Some("inbox") if limit(rest).is_some() => {
            feature_use::command(command);
            match SessionLog::read().inbox() {
                Ok(unlinked) if unlinked.is_empty() => {
                    out("Inbox empty — every captured session is linked to a work item or dismissed.\n");
                    0
                }
                Ok(unlinked) => {
                    out(&session_rows::print(unlinked, limit(rest).unwrap()));
                    0
                }
                Err(e) => abort(&format!("qm: {e}")),
            }
        }
        Some("dismiss") => {
            feature_use::command(command);
            match rest.first() {
                None => abort("usage: qm dismiss SESSION_ID"),
                Some(id) => match SessionLog::dismiss(id) {
                    Ok(()) => 0,
                    Err(e) => abort(&format!("qm: {e}")),
                },
            }
        }
        // bare `qm` lists when there is no terminal; with one it opens the fzf viewer
        None if !std::io::stdout().is_terminal() => match work_item_commands::run("list", &[]) {
            Some(Outcome::Done { code, stdout, stderr }) => {
                out(&stdout);
                let _ = std::io::stderr().write_all(stderr.as_bytes());
                code
            }
            _ => delegate::exec_ruby(&raw),
        },
        _ => delegate::exec_ruby(&raw),
    }
}
