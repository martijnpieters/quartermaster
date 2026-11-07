//! `qm pr-requires`: record, remove and list the earlier PRs or commits a PR waits on to be deployed.
//! Local bookkeeping only; `gh` is asked read-only questions. `qm pr` (it opens a browser and may ask
//! fzf which branch) stays Ruby.

use regex::Regex;
use serde_json::{Map, Value};

use crate::cli_format::puts;
use crate::outcome::{abort, done, Outcome};
use crate::pr_prerequisites::{self as store, Fail, R};

const USAGE: &str = "usage: qm pr-requires PR-OR-SHA [--pr PR] [--note TEXT] | --remove PR-OR-SHA [--pr PR] | --list [--pr PR]";

fn pr_label(url: &str) -> String {
    match Regex::new(r"(?i)\Ahttps?://github\.com/([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)/pull/([0-9]+)/?\z").unwrap().captures(url) {
        Some(c) => format!("{}#{}", &c[1], &c[2]),
        None => url.to_string(),
    }
}

fn prerequisite_label(reference: &str) -> String {
    match Regex::new(r"(?i)\Ahttps?://github\.com/([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)/pull/([0-9]+)/?\z").unwrap().captures(reference) {
        Some(c) => format!("#{}", &c[2]),
        None => reference.chars().take(7).collect(),
    }
}

#[derive(Default)]
struct Options {
    pr: Option<String>,
    remove: bool,
    list: bool,
    session: Option<String>,
    note: Option<String>,
}

pub fn run(args: &[String]) -> Outcome {
    let (mut o, mut rest) = (Options::default(), vec![]);
    let mut i = 0;
    while i < args.len() {
        let value = |i: usize| args.get(i + 1).cloned();
        match args[i].as_str() {
            flag @ ("--pr" | "--session" | "--note") => {
                let Some(v) = value(i) else { return Outcome::Delegate };
                match flag {
                    "--pr" => o.pr = Some(v),
                    "--session" => o.session = Some(v),
                    _ => o.note = Some(v),
                }
                i += 2;
            }
            "--remove" => {
                o.remove = true;
                i += 1;
            }
            "--list" => {
                o.list = true;
                i += 1;
            }
            s if s.starts_with('-') && s != "-" => return Outcome::Delegate, // OptionParser reads these its own way
            s => {
                rest.push(s.to_string());
                i += 1;
            }
        }
    }
    if (o.remove && o.list) || (o.note.is_some() && (o.remove || o.list)) || !(if o.list { rest.is_empty() } else { rest.len() == 1 }) {
        return abort(USAGE);
    }
    let cwd = std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    match perform(&o, &rest, &cwd) {
        Ok(out) => done(out),
        Err(Fail::Arg(message)) => abort(&format!("qm: {message}")),
        Err(Fail::Ruby) => Outcome::Delegate,
    }
}

fn perform(o: &Options, rest: &[String], cwd: &str) -> R<String> {
    if o.list {
        return list(o.pr.as_deref(), cwd);
    }
    let pr = store::resolve_pr(o.pr.as_deref(), cwd)?;
    let repo = store::repo_of_pr_url(&pr)?;
    let mut out = String::new();
    if o.remove {
        let reference = store::stored_ref(&pr, &rest[0], cwd)?;
        store::remove(&pr, &reference)?;
        out.push_str(&puts(&format!("{} no longer requires {}", pr_label(&pr), prerequisite_label(&reference))));
    } else {
        let (kind, reference) = store::resolve_prerequisite(&rest[0], &repo, cwd)?;
        let session = o.session.clone().or_else(|| std::env::var("CLAUDE_CODE_SESSION_ID").ok()).filter(|s| !s.is_empty());
        let added = store::add(&pr, kind, &reference, session.as_deref(), o.note.as_deref())?;
        let label = prerequisite_label(&reference);
        match (&added, &o.note) {
            (Some(_), _) | (None, None) => out.push_str(&puts(&format!("{} {} {label} deployed before merging", pr_label(&pr), if added.is_some() { "now requires" } else { "already requires" }))),
            (None, Some(note)) => match store::set_note(&pr, &reference, note)? {
                None => return Err(Fail::Arg(format!("{} no longer requires {label}", pr_label(&pr)))),
                Some(entry) => out.push_str(&puts(&format!("{}: {}", pr_label(&pr), if entry.contains_key("note") { format!("note on {label} set") } else { format!("note on {label} cleared") }))),
            },
        }
    }
    Ok(out)
}

fn text(entry: &Map<String, Value>, key: &str) -> R<String> {
    match entry.get(key) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(s)) => Ok(s.clone()),
        _ => Err(Fail::Ruby),
    }
}

fn list(spec: Option<&str>, cwd: &str) -> R<String> {
    let pr = match store::resolve_pr(spec, cwd) {
        Ok(url) => Some(url),
        Err(Fail::Arg(message)) if spec.is_some() => return Err(Fail::Arg(message)),
        Err(Fail::Arg(_)) => None,
        Err(Fail::Ruby) => return Err(Fail::Ruby),
    };
    let mut entries: Vec<(String, Vec<Map<String, Value>>)> = match &pr {
        Some(url) => vec![(url.clone(), store::for_pr(url)?)],
        None => store::all()?.into_iter().map(|(url, list)| (url, list.as_array().map(|l| l.iter().filter_map(|e| e.as_object().cloned()).collect()).unwrap_or_default())).collect(),
    };
    entries.retain(|(_, list)| !list.is_empty());
    if entries.is_empty() {
        return Ok(puts(&match &pr {
            Some(url) => format!("{} has no prerequisites", pr_label(url)),
            None => "no PR prerequisites recorded".to_string(),
        }));
    }
    let mut out = String::new();
    for (url, list) in &entries {
        out.push_str(&puts(&format!("{} ({url})", pr_label(url))));
        for entry in list {
            let label = store::verdict_label(entry)?;
            let reference = text(entry, "ref")?;
            out.push_str(&puts(&format!("  needs {} deployed — added {}{}", prerequisite_label(&reference), text(entry, "addedAt")?, label.map(|l| format!(" — {l}")).unwrap_or_default())));
            for (n, line) in text(entry, "note")?.split_inclusive('\n').enumerate() {
                let chomped = line.strip_suffix("\r\n").or_else(|| line.strip_suffix('\n')).or_else(|| line.strip_suffix('\r')).unwrap_or(line);
                out.push_str(&puts(&format!("{}{chomped}", if n == 0 { "    note: " } else { "          " })));
            }
        }
    }
    Ok(out)
}
