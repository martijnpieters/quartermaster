//! Carrying out a cleanup plan: the only Rust code that removes anything. It trusts the plan it is
//! given exactly as Ruby's `Cleanup#execute` does (a caller re-plans right before): a blocked entry
//! never runs, a warn entry only when forced by id, worktrees go first (their test DBs, then
//! `git worktree remove`) so a branch checked out there can follow, and a branch is deleted only
//! once it is provably checked out nowhere. Remotes are never touched.

use crate::cleanup::{Entry, Plan};
use crate::cli_format::short_path;
use crate::command_runner::{self, CmdResult};
use crate::data_root;
use crate::git_repo::GitRepo;
use crate::issue_drafts;
use crate::ruby_text::strip;
use crate::work_item::WorkItem;

pub struct Outcome {
    pub id: String,
    pub kind: &'static str,
    pub result: &'static str,
    pub message: String,
}

impl Outcome {
    fn new(entry: &Entry, result: &'static str, message: impl Into<String>) -> Outcome {
        Outcome { id: entry.id.clone(), kind: entry.kind, result, message: message.into() }
    }
    fn removed(&self) -> bool {
        self.result == "removed"
    }
}

/// Entry ids for what a user typed: an id, a worktree path, a branch name, or an issue draft's path
/// or SLUG/FILE. `Err` carries the message of the `ArgumentError` Ruby raises.
pub fn resolve_ids(plan: &Plan, keys: &[String]) -> Result<Vec<String>, String> {
    let mut ids = vec![];
    for key in keys {
        let matches: Vec<&Entry> = plan
            .entries
            .iter()
            .filter(|e| {
                e.id == *key
                    || (e.is_worktree() && same_path(e.path.as_deref(), key))
                    || (e.is_branch() && e.branch.as_deref() == Some(key))
                    || (e.is_issue_draft() && (same_path(e.path.as_deref(), key) || format!("{}/{}", e.item.clone().unwrap_or_default(), base(e.path.as_deref())) == *key))
            })
            .collect();
        if matches.is_empty() {
            return Err(format!("no cleanup entry matches '{key}'"));
        }
        ids.extend(matches.iter().map(|e| e.id.clone()));
    }
    Ok(ids)
}

fn base(path: Option<&str>) -> String {
    path.and_then(|p| std::path::Path::new(p).file_name()).map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
}

fn same_path(left: Option<&str>, right: &str) -> bool {
    let Some(left) = left else { return false };
    if data_root::expand_path(left) == data_root::expand_path(right) {
        return true;
    }
    let (l, r) = (std::path::Path::new(left), std::path::Path::new(right));
    l.exists() && r.exists() && matches!((std::fs::canonicalize(l), std::fs::canonicalize(r)), (Ok(a), Ok(b)) if a == b)
}

fn message(result: &CmdResult) -> String {
    [&result.err, &result.out].iter().map(|t| strip(t)).find(|t| !t.is_empty()).unwrap_or("exit status non-zero").to_string()
}

fn run(entry: &Entry, argv: &[&str], chdir: &str, done: String) -> Outcome {
    let result = command_runner::call_in(argv, chdir);
    if !result.success {
        return Outcome::new(entry, "failed", format!("{} failed: {}", argv.iter().take(3).copied().collect::<Vec<_>>().join(" "), message(&result)));
    }
    Outcome::new(entry, "removed", done)
}

fn test_db_of(root: &str) -> Option<String> {
    let project = crate::project_map::project_for(Some(root));
    crate::project_config::ProjectConfig::current().settings(Some(&project)).and_then(|s| s.test_db.clone())
}

fn remove_worktree(entry: &Entry, forced: bool) -> Outcome {
    let root = entry.repo.clone().unwrap_or_default();
    let path = entry.path.clone().unwrap_or_default();
    if entry.gone() {
        return run(entry, &["git", "worktree", "prune"], &root, "pruned".into());
    }
    let names = entry.db_names.clone().unwrap_or_default();
    if entry.db_status == "ok" && !names.is_empty() {
        if let Some(test_db) = test_db_of(&root) {
            let result = command_runner::call(&[&test_db, "drop", &path]);
            // The worktree stays, so a retry finds the same entry again.
            if !result.success {
                return Outcome::new(entry, "failed", format!("dropping test DBs failed: {}", message(&result)));
            }
        }
    }
    let mut argv = vec!["git", "worktree", "remove"];
    if forced {
        argv.push("--force");
    }
    argv.push(&path);
    let dbs = if names.is_empty() { String::new() } else { format!(" and test DBs {}", names.join(", ")) };
    run(entry, &argv, &root, format!("removed{dbs}"))
}

/// `git branch -D` only once the branch provably isn't checked out anywhere any more.
fn delete_branch(entry: &Entry, outcomes: &[Outcome]) -> Outcome {
    if let Some(required) = &entry.requires {
        if !outcomes.iter().find(|o| &o.id == required).is_some_and(Outcome::removed) {
            return Outcome::new(entry, "skipped", "its worktree stays, so the branch stays checked out");
        }
    }
    let root = entry.repo.clone().unwrap_or_default();
    let branch = entry.branch.clone().unwrap_or_default();
    if let Some(holder) = GitRepo::new(&root).worktrees().into_iter().find(|wt| wt.branch.as_deref() == Some(branch.as_str())) {
        return Outcome::new(entry, "skipped", format!("checked out at {}", short_path(Some(&holder.path))));
    }
    let outcome = run(entry, &["git", "branch", "-D", &branch], &root, "deleted".into());
    if outcome.removed() {
        forget_temp_branch(&root, &branch);
    }
    outcome
}

/// A deleted temp branch has nothing left to point at.
fn forget_temp_branch(root: &str, branch: &str) {
    let config = crate::project_config::ProjectConfig::current();
    let Some(project) = config.roots().into_iter().find(|(_, r)| *r == root).map(|(p, _)| p.to_string()) else { return };
    for mut item in WorkItem::all().unwrap_or_default() {
        if item.project().as_deref() == Some(project.as_str()) && item.temp_branches().iter().any(|b| b == branch) {
            let _ = item.unlink_temp_branch(branch);
        }
    }
}

fn delete_issue_draft(entry: &Entry) -> Outcome {
    let slug = entry.item.clone().unwrap_or_default();
    match issue_drafts::delete(&slug, &base(entry.path.as_deref())) {
        Ok(existed) => Outcome::new(entry, "removed", if existed { "deleted" } else { "already gone" }),
        Err(e) => Outcome::new(entry, "failed", format!("deleting it failed: {e}")),
    }
}

/// Removes the selected entries (ids) of a plan; a warn entry also needs its id in `force`, which
/// implies selecting it. One outcome per entry acted on, in the order they ran.
pub fn execute(plan: &Plan, selected: &[String], force: &[String]) -> Vec<Outcome> {
    execute_with(plan, selected, force, &mut |_, _| {})
}

/// `execute`, telling `on_start(index, total)` (1-based over the entries acted on) before each one.
pub fn execute_with(plan: &Plan, selected: &[String], force: &[String], on_start: &mut dyn FnMut(usize, usize)) -> Vec<Outcome> {
    let wanted: Vec<&String> = selected.iter().chain(force).collect();
    let ordered = plan.entries.iter().filter(|e| e.is_worktree()).chain(plan.entries.iter().filter(|e| e.is_branch())).chain(plan.entries.iter().filter(|e| e.is_issue_draft()));
    let todo: Vec<&Entry> = ordered.filter(|e| wanted.contains(&&e.id)).collect();
    let mut outcomes: Vec<Outcome> = vec![];
    for (index, entry) in todo.iter().copied().enumerate() {
        on_start(index + 1, todo.len());
        let forced = force.contains(&entry.id);
        let outcome = if entry.status == "blocked" {
            Outcome::new(entry, "skipped", format!("blocked: {}", entry.reasons.join("; ")))
        } else if entry.status == "warn" && !forced {
            Outcome::new(entry, "skipped", format!("needs force: {}", entry.reasons.join("; ")))
        } else if entry.is_worktree() {
            remove_worktree(entry, forced)
        } else if entry.is_branch() {
            delete_branch(entry, &outcomes)
        } else {
            delete_issue_draft(entry)
        };
        outcomes.retain(|o| o.id != outcome.id); // a repeated id keeps its last outcome, as Ruby's hash does
        outcomes.push(outcome);
    }
    outcomes
}
