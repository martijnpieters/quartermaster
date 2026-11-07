//! `qm cleanup SLUG` and `qm sweep` — the plan, and with `--yes` carrying it out (`cleanup_run`) — and
//! the resolve hint `state … resolved` prints. A terminal's progress bar is Ruby's: `--yes` on a tty goes there.

use crate::cleanup::{plan_item, plan_sweep, Entry};
use crate::cli_format::{paint, puts, short_path};
use crate::data_root;
use crate::outcome::{abort, done, Outcome};
use crate::work_item::WorkItem;

/// Drafts::SLUG: `\A[a-z0-9][\w.-]*\z`, case-insensitive.
pub fn valid_slug(slug: &str) -> bool {
    let mut chars = slug.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphanumeric()) && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Whether the plan must be Ruby's: a slug IssueDrafts would refuse, or issue-draft files this port
/// can't read exactly as Ruby does (checked before anything is written).
pub fn leaves_to_ruby(slug: &str) -> bool {
    !crate::issue_drafts::supported(slug)
}

/// Resolving is when worktrees go stale, so say what cleanup would do — a hint only: a failing plan
/// must never fail the resolve.
pub fn offer(item: &WorkItem) -> String {
    match plan_item(item) {
        Ok(plan) if !plan.is_empty() => puts(&format!("cleanup: {} — qm cleanup {}", plan.summary(), item.slug)),
        Ok(_) => String::new(),
        Err(e) => {
            data_root::log_error("cleanup-offer", e.class, &e.message);
            String::new()
        }
    }
}

fn line(entry: &Entry, force: &[String]) -> String {
    let (mark, color) = match entry.status {
        "warn" => ("⚠ ", 33),
        "blocked" => ("⛔ ", 31),
        _ => ("", 0),
    };
    let target = if entry.is_worktree() {
        short_path(entry.path.as_deref())
    } else if entry.is_issue_draft() {
        let file = std::path::Path::new(entry.path.as_deref().unwrap_or("")).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        format!("{}/{file}", entry.item.clone().unwrap_or_default())
    } else {
        entry.branch.clone().unwrap_or_default()
    };
    let head = if entry.is_worktree() {
        format!(" ({})", entry.branch.clone().unwrap_or_else(|| "detached".into()))
    } else if entry.is_issue_draft() {
        format!(" “{}”", entry.title.clone().unwrap_or_default())
    } else {
        String::new()
    };
    let dbs = match &entry.db_names {
        Some(names) if !names.is_empty() => format!(" · test DBs: {}", names.join(", ")),
        _ => String::new(),
    };
    let details = entry.reasons.iter().chain(&entry.notes).cloned().collect::<Vec<_>>().join("; ");
    let line = format!("  [{}] {} {target}{head}{dbs}", if entry.ticked || force.contains(&entry.id) { "x" } else { " " }, paint(&format!("{mark}{}", entry.kind), color));
    if details.is_empty() { line } else { format!("{line} — {details}") }
}

struct Options {
    yes: bool,
    force: Vec<String>,
    rest: Vec<String>,
}

/// `--yes` and `--force KEY` anywhere among the arguments; any other flag shape (abbreviated,
/// `=`-joined, `--`, a `~user` key) is read differently by Ruby's `OptionParser`.
fn options(a: &[String]) -> Option<Options> {
    let mut o = Options { yes: false, force: vec![], rest: vec![] };
    let mut i = 0;
    while i < a.len() {
        match a[i].as_str() {
            "--yes" => {
                o.yes = true;
                i += 1;
            }
            "--force" => {
                let key = a.get(i + 1)?;
                if key.starts_with('~') && key != "~" && !key.starts_with("~/") {
                    return None;
                }
                o.force.push(key.clone());
                i += 2;
            }
            s if s.starts_with('-') && s != "-" => return None,
            s => {
                o.rest.push(s.to_string());
                i += 1;
            }
        }
    }
    Some(o)
}

fn tty() -> bool {
    unsafe { libc::isatty(1) == 1 }
}

pub fn cleanup(a: &[String]) -> Outcome {
    let Some(o) = options(a) else { return Outcome::Delegate };
    let Some(slug) = o.rest.first() else { return abort("usage: qm cleanup SLUG [--yes] [--force PATH|BRANCH ...]") };
    if leaves_to_ruby(slug) || (o.yes && (tty() || !crate::drafts::writable(slug))) {
        return Outcome::Delegate;
    }
    let item = match WorkItem::find(slug) {
        Ok(i) => i,
        Err(e) => return abort(&format!("qm: {}", e.message)),
    };
    let plan = match plan_item(&item) {
        Ok(p) => p,
        Err(e) => return abort(&format!("qm: {}", e.message)),
    };
    carry_out(&plan, &o)
}

/// `qm sweep [--yes] [--force KEY]`: the plan over every repo, and with `--yes` carrying it out.
pub fn sweep(a: &[String]) -> Outcome {
    let Some(o) = options(a) else { return Outcome::Delegate };
    let writable = || crate::cleanup::swept_slugs().is_some_and(|s| s.iter().all(|slug| crate::drafts::writable(slug)));
    if !crate::cleanup::sweep_supported() || (o.yes && (tty() || !writable())) {
        return Outcome::Delegate;
    }
    match plan_sweep() {
        Ok(plan) => carry_out(&plan, &o),
        Err(e) if e.class == crate::error::UNEMITTABLE => Outcome::Delegate,
        Err(e) => abort(&format!("qm: {}", e.message)),
    }
}

/// Prints the plan; `--yes` removes what is ticked plus every `--force` entry.
fn carry_out(plan: &crate::cleanup::Plan, o: &Options) -> Outcome {
    let force = match crate::cleanup_run::resolve_ids(plan, &o.force) {
        Ok(f) => f,
        Err(message) => return abort(&format!("qm: {message}")),
    };
    let mut out = print_plan(plan, &force, o.yes);
    if plan.is_empty() || !o.yes {
        return done(out);
    }
    let selected: Vec<String> = plan.entries.iter().filter(|e| e.ticked).map(|e| e.id.clone()).collect();
    let outcomes = crate::cleanup_run::execute(plan, &selected, &force);
    if outcomes.is_empty() {
        out.push_str(&puts(&paint("nothing ticked", 90)));
    }
    for outcome in &outcomes {
        let color = match outcome.result {
            "removed" => 32,
            "skipped" => 33,
            _ => 31,
        };
        let shown = regex::Regex::new(r"\A[A-Za-z0-9_]+:").unwrap().replace(&outcome.id, "").into_owned();
        out.push_str(&puts(&format!("{} {shown}: {}", paint(&crate::cli_format::ljust(outcome.result, 7), color), outcome.message)));
    }
    Outcome::Done { code: if outcomes.iter().any(|o| o.result == "failed") { 1 } else { 0 }, stdout: out, stderr: String::new() }
}

fn print_plan(plan: &crate::cleanup::Plan, force: &[String], yes: bool) -> String {
    let mut out = String::new();
    for note in &plan.notes {
        out.push_str(&puts(&paint(note, 90)));
    }
    out.push_str(&puts(&format!("{}: {}", plan.scope, plan.summary())));
    for e in &plan.entries {
        out.push_str(&puts(&line(e, force)));
    }
    if !plan.is_empty() && !yes {
        out.push_str(&puts(&paint("--yes removes the [x] entries; --force PATH|BRANCH|SLUG/FILE adds a ⚠ one", 90)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::valid_slug;

    #[test]
    fn slugs_are_what_drafts_accept() {
        for ok in ["a", "fix-login-bug", "A1_b.c-d", "9x"] {
            assert!(valid_slug(ok), "{ok}");
        }
        for bad in ["", "-a", ".a", "a b", "a/b", "é"] {
            assert!(!valid_slug(bad), "{bad:?}");
        }
    }
}
