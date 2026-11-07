//! Worktree hygiene, planning only (docs/areas/worktrees.md): which worktrees and local branches of
//! one item could go. The plan is data a checklist renders: one entry per worktree or branch, with
//! a status (ok / warn / blocked), the reasons for it, and whether it starts ticked. Only ok entries
//! start ticked. Planning changes nothing; `cleanup_run` carries a plan out.
//!
//! The sweep (every repo's `.claude/worktrees` worktrees and `worktree-*` branches) is planned here too.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::command_runner;
use crate::data_root::{self, normalize};
use crate::error::{QmError, Res, UNEMITTABLE};
use crate::git_repo::{GitRepo, Worktree};
use crate::issue_drafts;
use crate::pf::{self, Status};
use crate::pr_status::PrStatus;
use crate::project_config::ProjectConfig;
use crate::ruby_text::strip;
use crate::session_log::SessionLog;
use crate::work_item::WorkItem;

const COMMITS_SHOWN: usize = 3;
const SWEEP_BRANCH_PREFIX: &str = "worktree-";
const WORKTREES_DIR: &str = ".claude/worktrees";
const MAINLINE_BRANCHES: [&str; 3] = ["main", "master", "develop"];

#[derive(Clone)]
pub struct Entry {
    /// `worktree:<path>`, `branch:<root>#<branch>`, `issue:<slug>/<file>`.
    pub id: String,
    pub kind: &'static str,
    /// The repo root a worktree or branch belongs to.
    pub repo: Option<String>,
    /// The id of the worktree entry a branch waits on.
    pub requires: Option<String>,
    /// "ok" (the test-DB dry run answered), "unknown", or "n/a".
    pub db_status: &'static str,
    pub path: Option<String>,
    pub branch: Option<String>,
    pub status: &'static str,
    pub reasons: Vec<String>,
    pub notes: Vec<String>,
    pub ticked: bool,
    pub db_names: Option<Vec<String>>,
    /// An issue draft's item and title.
    pub item: Option<String>,
    pub title: Option<String>,
}

impl Entry {
    pub fn is_worktree(&self) -> bool {
        self.kind == "worktree"
    }

    pub fn is_branch(&self) -> bool {
        self.kind == "branch"
    }

    /// A worktree whose directory is already gone.
    pub fn gone(&self) -> bool {
        self.is_worktree() && !self.path.as_deref().is_some_and(|p| std::path::Path::new(p).is_dir())
    }

    pub fn is_issue_draft(&self) -> bool {
        self.kind == "issue-draft"
    }
}

pub struct Plan {
    pub scope: String,
    pub entries: Vec<Entry>,
    pub notes: Vec<String>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// One line for a resolve dialog or a CLI echo: "2 worktrees, 1 branch — 2 ready, 1 needs a look".
    pub fn summary(&self) -> String {
        if self.is_empty() {
            return "nothing to clean up".into();
        }
        let count = |n: usize, noun: &str| {
            (n > 0).then(|| format!("{n} {}", if n == 1 { noun.to_string() } else { format!("{noun}{}", if noun.ends_with('h') { "es" } else { "s" }) }))
        };
        let worktrees = self.entries.iter().filter(|e| e.is_worktree()).count();
        let branches = self.entries.iter().filter(|e| e.kind == "branch").count();
        let issue_drafts = self.entries.iter().filter(|e| e.is_issue_draft()).count();
        let counts = [count(worktrees, "worktree"), count(branches, "branch"), count(issue_drafts, "issue draft")].into_iter().flatten().collect::<Vec<_>>().join(", ");
        let ready = self.entries.iter().filter(|e| e.ticked).count();
        let look = self.entries.iter().filter(|e| e.status == "warn").count();
        let blocked = self.entries.iter().filter(|e| e.status == "blocked").count();
        let mut detail = vec![format!("{ready} ready")];
        if look > 0 {
            detail.push(format!("{look} need{} a look", if look == 1 { "s" } else { "" }));
        }
        if blocked > 0 {
            detail.push(format!("{blocked} blocked"));
        }
        format!("{counts} — {}", detail.join(", "))
    }
}

/// An unpreserved-commit list as the reason a worktree or branch needs a look.
fn lost_reason(commits: Option<Vec<(String, Option<String>)>>) -> Option<String> {
    let commits = match commits {
        None => return Some("couldn't check its commits (git log failed)".into()),
        Some(c) if c.is_empty() => return None,
        Some(c) => c,
    };
    let shown = commits.iter().take(COMMITS_SHOWN).map(|(sha, subject)| format!("{} {}", sha.chars().take(7).collect::<String>(), subject.clone().unwrap_or_default())).collect::<Vec<_>>().join("; ");
    let more = if commits.len() > COMMITS_SHOWN { "; …" } else { "" };
    Some(format!("{} commit{} found nowhere else ({shown}{more})", commits.len(), if commits.len() == 1 { "" } else { "s" }))
}

fn within(path: &str, dir: &str) -> bool {
    let (path, dir) = (normalize(path).to_string_lossy().into_owned(), normalize(dir).to_string_lossy().into_owned());
    path == dir || path.starts_with(&format!("{dir}/"))
}

fn upstream(branch: Option<&str>) -> Vec<String> {
    branch.map(|b| vec![format!("origin/{b}")]).unwrap_or_default()
}

/// What `<test_db> drop PATH --dry-run` would drop; when it is missing or fails the answer is
/// "unknown", not an error.
fn test_databases(test_db: &str, path: &str) -> (Option<Vec<String>>, &'static str, Option<String>) {
    let result = command_runner::call(&[test_db, "drop", path, "--dry-run"]);
    if result.success {
        return (Some(result.out.split('\n').map(|l| strip(l).to_string()).filter(|l| !l.is_empty()).collect()), "ok", None);
    }
    let why = [&result.err, &result.out].iter().map(|t| strip(t)).find(|t| !t.is_empty()).unwrap_or("exit status non-zero").to_string();
    (None, "unknown", Some(format!("test DBs unknown: {}", strip(why.lines().next().unwrap_or("")))))
}

struct Planner {
    sessions: Vec<crate::session_log::Session>,
    config: ProjectConfig,
    pr_status: PrStatus,
    open_items: Vec<WorkItem>,
    served: Option<Option<Status>>,
}

struct Built {
    entry: Entry,
}

#[allow(clippy::too_many_arguments)]
fn build(kind: &'static str, id: String, repo: &str, path: Option<String>, branch: Option<String>, blocked: Vec<String>, warn: Vec<String>, notes: Vec<String>, db_names: Option<Vec<String>>, db_status: &'static str) -> Entry {
    let status = if !blocked.is_empty() { "blocked" } else if !warn.is_empty() { "warn" } else { "ok" };
    Entry { id, repo: Some(repo.to_string()), requires: None, db_status, kind, path, branch, status, reasons: blocked.into_iter().chain(warn).collect(), notes, ticked: status == "ok", db_names, item: None, title: None }
}

impl Planner {
    fn new() -> Planner {
        let _ = data_root::ensure_setup();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        Planner { sessions: SessionLog::read().sessions(), config: ProjectConfig::current(), pr_status: PrStatus::new(now), open_items: vec![], served: None }
    }

    fn served(&mut self, path: &str) -> bool {
        if self.served.is_none() {
            self.served = Some(pf::status(self.config.dev_server()));
        }
        pf::serves(self.served.as_ref().unwrap(), path)
    }

    fn live_sessions_in(&self, path: &str) -> Vec<String> {
        self.sessions.iter().filter(|s| s.live).filter_map(|s| s.cwd().map(|c| (s.id.clone(), c))).filter(|(_, cwd)| within(cwd, path)).map(|(id, _)| id).collect()
    }

    fn owners(&self, items: &[&WorkItem], wt: &Worktree, project: Option<&str>) -> Vec<String> {
        items
            .iter()
            .filter(|i| i.project().as_deref() == project)
            .filter(|i| {
                let own: Vec<String> = i.branches().into_iter().chain(i.temp_branches()).collect();
                wt.branch.as_ref().is_some_and(|b| own.contains(b))
                    || i.session_ids().iter().any(|id| self.sessions.iter().find(|s| s.id == *id).and_then(|s| s.cwd()).is_some_and(|cwd| within(&cwd, &wt.path)))
            })
            .map(|i| i.slug.clone())
            .collect()
    }

    fn branch_owners(items: &[&WorkItem], branch: &str, project: Option<&str>) -> Vec<String> {
        items
            .iter()
            .filter(|i| i.project().as_deref() == project && i.branches().into_iter().chain(i.temp_branches()).any(|b| b == branch))
            .map(|i| i.slug.clone())
            .collect()
    }

    fn worktree_entry(&mut self, repo: &mut GitRepo, wt: &Worktree, bases: &[String], test_db: Option<&str>, warn_owners: &[String]) -> Entry {
        let mut blocked: Vec<String> = vec![];
        let mut warn: Vec<Option<String>> = vec![];
        let mut notes: Vec<String> = vec![];
        if wt.is_locked() {
            let reason = wt.locked.as_deref().unwrap_or("");
            blocked.push(if reason.is_empty() { "locked by git".into() } else { format!("locked: {reason}") });
        }
        if self.served(&wt.path) {
            blocked.push("the dev server serves it".into());
        }
        for id in self.live_sessions_in(&wt.path) {
            blocked.push(format!("live session {} works here", id.chars().take(8).collect::<String>()));
        }
        let mut db_names: Option<Vec<String>> = Some(vec![]);
        let mut db_status = "n/a";
        if wt.prunable || !std::path::Path::new(&wt.path).is_dir() {
            notes.push("directory already gone (git worktree prune)".into());
        } else {
            warn.extend(repo.dirt(&wt.path).into_iter().map(Some));
            if let Some(head) = &wt.head {
                let mut targets = repo.trunk_refs();
                targets.extend(upstream(wt.branch.as_deref()));
                warn.push(lost_reason(repo.unpreserved_commits(head, bases, &targets)));
            }
            if let Some(db) = test_db {
                let (names, status, note) = test_databases(db, &wt.path);
                db_names = names;
                db_status = status;
                if let Some(n) = note {
                    notes.push(n);
                }
            }
        }
        let mut warn: Vec<String> = warn.into_iter().flatten().collect();
        warn.extend(warn_owners.iter().map(|slug| format!("also used by open item '{slug}'")));
        build("worktree", format!("worktree:{}", wt.path), &repo.root, Some(wt.path.clone()), wt.branch.clone(), blocked, warn, notes, db_names, db_status)
    }

    #[allow(clippy::too_many_arguments)]
    fn branch_entry(&mut self, repo: &mut GitRepo, branch: &str, bases: &[String], why: &str, checked_out: &[(String, Worktree)], worktree_entries: &[Entry], warn_owners: &[String]) -> Built {
        let mut blocked: Vec<String> = vec![];
        let mut holder_ticked = None;
        let mut requires = None;
        if let Some((_, wt)) = checked_out.iter().find(|(b, _)| b == branch) {
            match worktree_entries.iter().find(|e| e.path.as_deref() == Some(wt.path.as_str())) {
                Some(holder) => {
                    holder_ticked = Some(holder.ticked);
                    requires = Some(holder.id.clone());
                }
                None => blocked.push(format!("checked out at {}", crate::cli_format::short_path(Some(&wt.path)))),
            }
        }
        let rest: Vec<String> = bases.iter().filter(|b| *b != branch).cloned().collect();
        let mut targets = repo.trunk_refs();
        targets.extend(upstream(Some(branch)));
        let mut warn: Vec<String> = lost_reason(repo.unpreserved_commits(branch, &rest, &targets)).into_iter().collect();
        warn.extend(warn_owners.iter().map(|slug| format!("also linked to open item '{slug}'")));
        let mut entry = build("branch", format!("branch:{}#{branch}", repo.root), &repo.root, None, Some(branch.to_string()), blocked, warn, vec![why.to_string()], None, "n/a");
        entry.requires = requires;
        // deleting it waits on its worktree going first, so it starts ticked only when that worktree does
        entry.ticked = entry.ticked && holder_ticked.unwrap_or(true);
        Built { entry }
    }
}

/// One entry per issue file of the item, broken ones included: deleting takes the file's unsent
/// issue comments along, so those are counted. Reading keeps the revision snapshots (as Ruby's does);
/// files this port can't judge like Ruby come back as an `Unemittable` error, and callers check
/// `issue_drafts::supported` first so that nothing has been written by then.
fn issue_entries(slug: &str) -> Res<Vec<Entry>> {
    let unreadable = || QmError::new(UNEMITTABLE, "issue drafts only Ruby reads exactly");
    if issue_drafts::files(slug).ok_or_else(unreadable)?.is_empty() {
        return Ok(vec![]);
    }
    let loaded = issue_drafts::load(slug).ok_or_else(unreadable)?;
    loaded.record().map_err(|_| unreadable())?;
    let unsent = issue_drafts::unsent_comments(slug).ok_or_else(unreadable)?;
    let mut rows: Vec<(String, String, Option<u64>, Option<String>)> = loaded.issues.iter().map(|i| (i.file.clone(), i.title.clone(), i.number, None)).collect();
    rows.extend(loaded.errors.iter().map(|e| (e.file.clone(), e.file.clone(), None, Some(format!("doesn't parse: {}", e.error)))));
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(rows
        .into_iter()
        .map(|(file, title, number, problem)| {
            let comments = unsent.get(&file).copied().unwrap_or(0);
            let comments_note = (comments > 0).then(|| format!("{comments} unsent comment{}", if comments == 1 { "" } else { "s" }));
            let warn: Vec<String> = if number.is_some() { vec![] } else { [Some(problem.unwrap_or_else(|| "never created on GitHub".into())), comments_note.clone()].into_iter().flatten().collect() };
            let notes: Vec<String> = if let Some(n) = number { [Some(format!("#{n}")), comments_note].into_iter().flatten().collect() } else { vec![] };
            Entry {
                id: format!("issue:{slug}/{file}"),
                repo: None,
                requires: None,
                db_status: "n/a",
                kind: "issue-draft",
                path: Some(issue_drafts::dir(slug).join(&file).to_string_lossy().into_owned()),
                branch: None,
                status: if warn.is_empty() { "ok" } else { "warn" },
                ticked: warn.is_empty(),
                reasons: warn,
                notes,
                db_names: None,
                item: Some(slug.to_string()),
                title: Some(title),
            }
        })
        .collect())
}

/// Listed, never pre-ticked: an unresolved item still claims it.
fn protect(mut entry: Entry, slugs: Vec<String>) -> Entry {
    if !slugs.is_empty() {
        entry.ticked = false;
        entry.notes.extend(slugs.into_iter().map(|slug| format!("belongs to unresolved item '{slug}'")));
    }
    entry
}

/// Slugs with an issue-drafts directory, as the sweep lists them. `None` when a name is not UTF-8.
pub fn swept_slugs() -> Option<Vec<String>> {
    let Ok(entries) = std::fs::read_dir(data_root::root().join("issues")) else { return Some(vec![]) };
    let mut slugs = vec![];
    for e in entries.filter_map(|e| e.ok()) {
        let name = e.file_name().into_string().ok()?;
        if e.path().is_dir() && crate::cleanup_command::valid_slug(&name) {
            slugs.push(name);
        }
    }
    slugs.sort();
    Some(slugs)
}

/// Whether the sweep's issue-draft part is something `issue_drafts::load` can answer for (nothing written).
pub fn sweep_supported() -> bool {
    swept_slugs().is_some_and(|slugs| slugs.iter().all(|s| issue_drafts::supported(s)))
}

/// Every `.claude/worktrees/*` worktree and `worktree-*` branch across the repos qm knows, checked
/// against the trunk and every other local branch. Anything an unresolved item still claims is listed
/// but never pre-ticked.
pub fn plan_sweep() -> Res<Plan> {
    let unreadable = || QmError::new(UNEMITTABLE, "issue drafts only Ruby reads exactly");
    let mut planner = Planner::new();
    planner.open_items = WorkItem::all()?.into_iter().filter(|i| i.state().as_deref() != Some("resolved")).collect();
    let items = std::mem::take(&mut planner.open_items);
    let (mut entries, mut notes): (Vec<Entry>, Vec<String>) = (vec![], vec![]);
    let config = ProjectConfig::current();
    let mut seen: Vec<std::path::PathBuf> = vec![];
    for (project, root) in config.roots() {
        let normal = data_root::normalize(root);
        if seen.contains(&normal) {
            continue;
        }
        seen.push(normal);
        if !std::path::Path::new(root).is_dir() {
            continue;
        }
        let mut repo = GitRepo::new(root);
        let listing = repo.worktrees();
        if listing.is_empty() {
            notes.push(format!("{project}: {} is not a git repo", crate::cli_format::short_path(Some(root))));
            continue;
        }
        let trunk = repo.trunk_refs();
        let locals = repo.local_branches();
        let candidates: Vec<&String> = locals.iter().filter(|b| b.starts_with(SWEEP_BRANCH_PREFIX)).collect();
        let keepers: Vec<String> = locals.iter().filter(|b| !b.starts_with(SWEEP_BRANCH_PREFIX)).cloned().collect();
        let dir = format!("{}/{WORKTREES_DIR}", normalize(&listing[0].path).to_string_lossy());
        let claimers: Vec<&WorkItem> = items.iter().filter(|i| i.project().as_deref() == Some(project)).collect();
        let test_db = planner.config.settings(Some(project)).and_then(|s| s.test_db.clone());

        let mut worktree_entries: Vec<Entry> = vec![];
        for wt in listing.iter().skip(1).filter(|wt| within(&wt.path, &dir)) {
            let mut bases = trunk.clone();
            bases.extend(keepers.iter().filter(|k| Some(*k) != wt.branch.as_ref()).cloned());
            bases.extend(upstream(wt.branch.as_deref()));
            let entry = planner.worktree_entry(&mut repo, wt, &bases, test_db.as_deref(), &[]);
            worktree_entries.push(protect(entry, planner.owners(&claimers, wt, Some(project))));
        }
        let checked_out: Vec<(String, Worktree)> = listing.iter().filter_map(|wt| wt.branch.clone().map(|b| (b, wt.clone()))).collect();
        let mut branch_entries: Vec<Entry> = vec![];
        for b in candidates {
            let mut bases = trunk.clone();
            bases.extend(keepers.iter().cloned());
            let entry = planner.branch_entry(&mut repo, b, &bases, "worktree branch", &checked_out, &worktree_entries, &[]).entry;
            branch_entries.push(protect(entry, Planner::branch_owners(&claimers, b, Some(project))));
        }
        entries.extend(worktree_entries);
        entries.extend(branch_entries);
    }
    let open_slugs: Vec<&str> = items.iter().map(|i| i.slug.as_str()).collect();
    for slug in swept_slugs().ok_or_else(unreadable)? {
        let owners = if open_slugs.contains(&slug.as_str()) { vec![slug.clone()] } else { vec![] };
        for entry in issue_entries(&slug)? {
            entries.push(protect(entry, owners.clone()));
        }
    }
    Ok(Plan { scope: "sweep".into(), entries, notes })
}

/// The item's worktrees — by its branches, its temp branches and the cwds of its sessions — plus its
/// local branches whose PR merged and its temp branches. Never the main checkout.
pub fn plan_item(item: &WorkItem) -> Res<Plan> {
    let issues = issue_entries(&item.slug)?;
    let mut planner = Planner::new();
    let root = crate::project_map::root_for(item.project().as_deref()).filter(|r| std::path::Path::new(r).is_dir());
    let Some(root) = root else {
        return Ok(Plan { scope: item.slug.clone(), entries: issues, notes: vec![format!("no repo root configured for '{}'", item.project().unwrap_or_default())] });
    };
    let mut repo = GitRepo::new(&root);
    let listing = repo.worktrees();
    let trunk = repo.trunk_refs();
    let own: Vec<String> = item.branches().into_iter().chain(item.temp_branches()).collect();
    let cwds: Vec<String> = item
        .session_ids()
        .iter()
        .filter_map(|id| planner.sessions.iter().find(|s| s.id == *id).and_then(|s| s.cwd()))
        .map(|c| normalize(&c).to_string_lossy().into_owned())
        .collect();
    planner.open_items = WorkItem::all()?.into_iter().filter(|i| i.state().as_deref() != Some("resolved")).collect();
    let open = std::mem::take(&mut planner.open_items);
    let others: Vec<&WorkItem> = open.iter().filter(|i| i.slug != item.slug).collect();
    let test_db = planner.config.settings(item.project().as_deref()).and_then(|s| s.test_db.clone());
    let project = item.project();

    let mut worktree_entries: Vec<Entry> = vec![];
    for wt in listing.iter().skip(1) {
        let belongs = wt.branch.as_ref().is_some_and(|b| own.contains(b)) || cwds.iter().any(|cwd| within(cwd, &wt.path));
        if !belongs {
            continue;
        }
        let mut bases: Vec<String> = item.branches().into_iter().filter(|b| Some(b) != wt.branch.as_ref()).collect();
        bases.extend(upstream(wt.branch.as_deref()));
        bases.extend(trunk.iter().cloned());
        let owners = planner.owners(&others, wt, project.as_deref());
        worktree_entries.push(planner.worktree_entry(&mut repo, wt, &bases, test_db.as_deref(), &owners));
    }

    let checked_out: Vec<(String, Worktree)> = listing.iter().filter_map(|wt| wt.branch.clone().map(|b| (b, wt.clone()))).collect();
    let branches = item.branches();
    let temps = item.temp_branches();
    let mut branch_entries: Vec<Entry> = vec![];
    let merged: Vec<String> = branches
        .iter()
        .filter(|b| !MAINLINE_BRANCHES.contains(&b.as_str()))
        .filter(|b| repo.is_branch(b) && planner.pr_status.state_for(item, b).as_deref() == Some("MERGED"))
        .cloned()
        .collect();
    for b in &merged {
        let mut bases = upstream(Some(b));
        bases.extend(branches.iter().filter(|x| *x != b).cloned());
        bases.extend(trunk.iter().cloned());
        let owners = Planner::branch_owners(&others, b, project.as_deref());
        branch_entries.push(planner.branch_entry(&mut repo, b, &bases, "PR merged", &checked_out, &worktree_entries, &owners).entry);
    }
    let temp_only: Vec<String> = temps.iter().filter(|b| !branches.contains(b)).filter(|b| repo.is_branch(b)).cloned().collect();
    for b in &temp_only {
        let mut bases: Vec<String> = branches.iter().flat_map(|ib| std::iter::once(ib.clone()).chain(upstream(Some(ib)))).collect();
        bases.extend(trunk.iter().cloned());
        let owners = Planner::branch_owners(&others, b, project.as_deref());
        branch_entries.push(planner.branch_entry(&mut repo, b, &bases, "temp branch", &checked_out, &worktree_entries, &owners).entry);
    }
    let owned = |mut e: Entry| {
        e.item = Some(item.slug.clone());
        e
    };
    Ok(Plan { scope: item.slug.clone(), entries: worktree_entries.into_iter().map(owned).chain(branch_entries.into_iter().map(owned)).chain(issues).collect(), notes: vec![] })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: &'static str, status: &'static str) -> Entry {
        Entry { id: String::new(), repo: None, requires: None, db_status: "n/a", kind, path: None, branch: None, status, reasons: vec![], notes: vec![], ticked: status == "ok", db_names: None, item: None, title: None }
    }

    fn plan(entries: Vec<Entry>) -> Plan {
        Plan { scope: "s".into(), entries, notes: vec![] }
    }

    #[test]
    fn the_summary_counts_kinds_and_says_how_many_need_a_look() {
        assert_eq!(plan(vec![]).summary(), "nothing to clean up");
        assert_eq!(plan(vec![entry("worktree", "ok")]).summary(), "1 worktree — 1 ready");
        assert_eq!(
            plan(vec![entry("worktree", "ok"), entry("worktree", "warn"), entry("branch", "warn"), entry("branch", "blocked")]).summary(),
            "2 worktrees, 2 branches — 1 ready, 2 need a look, 1 blocked"
        );
        assert_eq!(plan(vec![entry("branch", "warn")]).summary(), "1 branch — 0 ready, 1 needs a look");
    }

    #[test]
    fn lost_commits_name_up_to_three_and_a_failed_check_is_never_nothing() {
        let commit = |i: u32| (format!("{i:040x}"), Some(format!("subject {i}")));
        assert_eq!(lost_reason(Some(vec![])), None);
        assert_eq!(lost_reason(None).as_deref(), Some("couldn't check its commits (git log failed)"));
        assert_eq!(lost_reason(Some(vec![commit(1)])).as_deref(), Some("1 commit found nowhere else (0000000 subject 1)"));
        let four = lost_reason(Some((1..=4).map(commit).collect())).unwrap();
        assert_eq!(four, "4 commits found nowhere else (0000000 subject 1; 0000000 subject 2; 0000000 subject 3; …)");
    }
}
