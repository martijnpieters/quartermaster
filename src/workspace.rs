//! "Work on this item": point the dev server and the IDE at the item's target worktree
//! (`Workspace` and `TargetWorktree`). The actions the GUI's jobs run; failures are the tool's own
//! message as an `Err`.

use std::path::Path;

use serde_json::{json, Value};

use crate::branch_diff::branch_worktrees;
use crate::cli_format::short_path;
use crate::command_runner;
use crate::data_root;
use crate::git::git;
use crate::project_config::{Ide, ProjectConfig};
use crate::ruby_text::strip;
use crate::work_item::WorkItem;

const MAINLINE_BRANCHES: [&str; 3] = ["main", "master", "develop"];

pub struct Target {
    pub path: String,
    pub branch: Option<String>,
    pub create: bool,
    pub warning: Option<String>,
    /// The item's other branches with a worktree, offered instead.
    pub alternatives: Vec<(String, String)>,
}

pub struct Report {
    pub target: Target,
    pub mode: Option<String>,
    pub steps: Vec<String>,
}

pub type Failed = String;

/// `TargetWorktree.slug_for`: one flat directory segment for a branch name.
pub fn slug_for(branch: &str) -> String {
    let mut flat = String::new();
    let mut dashing = false;
    for c in branch.chars() {
        if c.is_ascii_alphanumeric() || "._-".contains(c) {
            flat.push(c);
            dashing = false;
        } else if !dashing {
            flat.push('-');
            dashing = true;
        }
    }
    flat.trim_matches('-').to_string()
}

fn new_worktree_path(root: &str, branch: &str) -> String {
    format!("{root}/.claude/worktrees/{}", slug_for(branch))
}

fn root_for(project: Option<&str>) -> Result<String, Failed> {
    crate::project_map::root_for(project).ok_or_else(|| format!("no repo root configured for project '{}'", project.unwrap_or("")))
}

/// `TargetWorktree.require_linked`.
pub fn require_linked(item: &WorkItem, branch: &str) -> Result<(), Failed> {
    if item.branches().iter().any(|b| b == branch) {
        Ok(())
    } else {
        Err(format!("branch {branch} is not linked to '{}'", item.slug))
    }
}

fn checked_out(root: &str) -> Vec<(String, String)> {
    branch_worktrees(root).into_iter().filter(|(_, p)| Path::new(p).is_dir()).collect()
}

fn lookup<'a>(list: &'a [(String, String)], branch: &str) -> Option<&'a String> {
    list.iter().find(|(b, _)| b == branch).map(|(_, p)| p)
}

/// Where "work on this item" means, and whether that worktree must be created first (nothing is
/// created here, but the item's stack branches are linked).
pub fn resolve(item: &mut WorkItem, worktree: Option<&str>, branch: Option<&str>) -> Result<Target, Failed> {
    if let Some(w) = worktree {
        let path = data_root::expand_path(w).to_string_lossy().into_owned();
        if !Path::new(&path).is_dir() {
            return Err(format!("no such worktree: {path}"));
        }
        return Ok(Target { path, branch: None, create: false, warning: None, alternatives: vec![] });
    }
    let project = item.project();
    let root = root_for(project.as_deref())?;
    let stacks = crate::gh_stack::for_root(&root);
    stacks.link_into(item).map_err(|e| e.message)?;
    let out = checked_out(&root);
    let branches = item.branches();
    let with_worktree: Vec<(String, String)> = branches.iter().rev().filter_map(|b| lookup(&out, b).map(|p| (b.clone(), p.clone()))).collect();
    if let Some(branch) = branch {
        require_linked(item, branch)?;
        let alternatives: Vec<(String, String)> = with_worktree.iter().filter(|(b, _)| b != branch).cloned().collect();
        return Ok(match lookup(&out, branch) {
            Some(path) => Target { path: path.clone(), branch: Some(branch.to_string()), create: false, warning: None, alternatives: alternatives.clone() },
            None => Target { path: new_worktree_path(&root, branch), branch: Some(branch.to_string()), create: true, warning: None, alternatives },
        });
    }
    // the stack holding the most recently linked stacked branch
    let stack = branches.iter().rev().find_map(|b| stacks.stacks_for(std::slice::from_ref(b)).into_iter().next());
    if let Some(stack) = stack {
        let top = stack.entries.last().map(|e| e.branch.clone()).unwrap_or_default();
        let alternatives: Vec<(String, String)> = with_worktree.iter().filter(|(b, _)| *b != top).cloned().collect();
        if let Some(path) = lookup(&out, &top) {
            return Ok(Target { path: path.clone(), branch: Some(top), create: false, warning: None, alternatives });
        }
        let siblings: Vec<String> = stack.entries.iter().filter_map(|e| lookup(&out, &e.branch).map(|p| format!("{} ({})", e.branch, short_path(Some(p))))).collect();
        let warning = (!siblings.is_empty()).then(|| format!("stack top {top} has no worktree; only {} does — a new worktree is created for the top instead of checking it out there", siblings.join(", ")));
        return Ok(Target { path: new_worktree_path(&root, &top), branch: Some(top), create: true, warning, alternatives });
    }
    if let Some((branch, path)) = with_worktree.first() {
        return Ok(Target { path: path.clone(), branch: Some(branch.clone()), create: false, warning: None, alternatives: with_worktree[1..].to_vec() });
    }
    if let Some(branch) = branches.iter().rev().find(|b| !MAINLINE_BRANCHES.contains(&b.as_str())) {
        return Ok(Target { path: new_worktree_path(&root, branch), branch: Some(branch.clone()), create: true, warning: None, alternatives: vec![] });
    }
    Ok(Target { path: root, branch: None, create: false, warning: None, alternatives: vec![] })
}

fn run_checked(argv: &[&str], label: &str, chdir: &str) -> Result<(), Failed> {
    let result = command_runner::call_in(argv, chdir);
    if result.success {
        return Ok(());
    }
    let message = [result.err.as_str(), result.out.as_str()].iter().map(|t| strip(t)).find(|t| !t.is_empty()).unwrap_or("exit status non-zero").to_string();
    Err(format!("{label} failed: {message}"))
}

fn add_worktree(root: &str, branch: &str, path: &str, base: Option<&str>) -> Result<(), Failed> {
    if git(Some(root), &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).is_some() {
        return run_checked(&["git", "worktree", "add", path, branch], "git worktree add", root);
    }
    if let Some(base) = base {
        return run_checked(&["git", "worktree", "add", "--no-track", "-b", branch, path, base], "git worktree add", root);
    }
    if git(Some(root), &["rev-parse", "--verify", "--quiet", &format!("refs/remotes/origin/{branch}")]).is_none() {
        run_checked(&["git", "fetch", "origin", branch], "git fetch", root)?;
    }
    run_checked(&["git", "worktree", "add", "--track", "-b", branch, path, &format!("origin/{branch}")], "git worktree add", root)
}

/// A retry finds the worktree already there and skips this step, so the failure says it exists.
fn prepare_test_db(project: Option<&str>, path: &str) -> Result<(), Failed> {
    let config = ProjectConfig::current();
    let Some(command) = config.settings(project).and_then(|s| s.test_db.clone()) else { return Ok(()) };
    run_checked(&[command.as_str(), "prepare"], &format!("{command} prepare"), path).map_err(|e| format!("created worktree {}, then {e}; run `{command} prepare` there before using it", short_path(Some(path))))
}

/// `TargetWorktree#materialize`: resolve, creating the worktree when the target has none yet.
pub fn materialize(item: &mut WorkItem, worktree: Option<&str>, branch: Option<&str>) -> Result<Target, Failed> {
    let target = resolve(item, worktree, branch)?;
    if target.create {
        let root = root_for(item.project().as_deref())?;
        add_worktree(&root, target.branch.as_deref().unwrap_or(""), &target.path, None)?;
        prepare_test_db(item.project().as_deref(), &target.path)?;
    }
    Ok(target)
}

fn report(target: Target, mode: Option<String>, steps: impl FnOnce(&mut Vec<String>, &Target) -> Result<(), Failed>) -> Result<Report, Failed> {
    let mut list = vec![];
    if target.create {
        list.push(format!("created worktree {} for {}", short_path(Some(&target.path)), target.branch.as_deref().unwrap_or("")));
    }
    if let Err(e) = steps(&mut list, &target) {
        // steps that already ran must not vanish behind the later failure, or a retry is blind to them
        return Err(if list.is_empty() { e } else { format!("{}; then {e}", list.join("; ")) });
    }
    Ok(Report { target, mode, steps: list })
}

fn pf_step(server: &crate::project_config::DevServer, target: &Target, mode: &str) -> Result<String, Failed> {
    crate::pf::switch(server, &target.path, mode)?;
    Ok(format!("{} {mode} @ {}", server.command, short_path(Some(&target.path))))
}

fn ide_step(ide: &Ide, target: &Target) -> Result<String, Failed> {
    let argv = ide.argv(&target.path);
    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let result = command_runner::call(&refs);
    if !result.success {
        let text = if strip(&result.err).is_empty() { &result.out } else { &result.err };
        return Err(format!("opening {} failed: {}", ide.label, strip(text)));
    }
    Ok(format!("{} opened {}", ide.label, short_path(Some(&target.path))))
}

fn check_mode(server: &crate::project_config::DevServer, mode: &str) -> Result<String, Failed> {
    if server.modes.iter().any(|m| m == mode) {
        Ok(mode.to_string())
    } else {
        Err(format!("{} mode must be one of: {}", server.command, server.modes.join(", ")))
    }
}

fn no_ide() -> Failed {
    "no IDE configured: set ide_command (and ide_label) in config.yml".into()
}

fn dev_server_of(config: &ProjectConfig, project: Option<&str>) -> Result<crate::project_config::DevServer, Failed> {
    config.settings(project).and_then(|s| s.dev_server.clone()).ok_or_else(|| format!("{0} has no dev server: set projects.{0}.dev_server in config.yml", project.unwrap_or("")))
}

/// `Workspace#switch_pf`.
pub fn switch_pf(item: &mut WorkItem, mode: Option<&str>, worktree: Option<&str>, branch: Option<&str>) -> Result<Report, Failed> {
    let config = ProjectConfig::current();
    let server = dev_server_of(&config, item.project().as_deref())?;
    let wanted = mode.map(String::from).or_else(|| item.pf_mode()).unwrap_or_default();
    let mode = check_mode(&server, &wanted)?;
    let target = materialize(item, worktree, branch)?;
    let shown = mode.clone();
    report(target, Some(mode), |steps, target| {
        steps.push(pf_step(&server, target, &shown)?);
        Ok(())
    })
}

/// `Workspace#open_ide`.
pub fn open_ide(item: &mut WorkItem, worktree: Option<&str>, branch: Option<&str>) -> Result<Report, Failed> {
    let config = ProjectConfig::current();
    let ide = config.ide(item.project().as_deref()).ok_or_else(no_ide)?;
    let target = materialize(item, worktree, branch)?;
    report(target, None, |steps, target| {
        steps.push(ide_step(ide, target)?);
        Ok(())
    })
}

/// A new tab in Warp's active window (`WarpTab#open`): a tab config file, then `open warp://…`.
pub fn warp_tab(cwd: &str, title: &str, command: Option<&str>) -> Result<String, Failed> {
    let dir = data_root::expand_path(&std::env::var("QM_WARP_TAB_CONFIGS").unwrap_or_else(|_| "~/.warp/tab_configs".into()));
    let toml = |text: &str| format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("qm-launch.toml"), format!("name = {}\n\n[[panes]]\nid = \"main\"\ntype = \"terminal\"\ndirectory = {}\n{}\n", toml(title), toml(cwd), command.map(|c| format!("commands = [{}]", toml(c))).unwrap_or_default())).map_err(|e| e.to_string())?;
    if !command_runner::call(&["open", "warp://tab_config/qm-launch"]).success {
        return Err("could not open a Warp tab (is Warp installed?)".into());
    }
    Ok(format!("Opened Warp tab: {title}"))
}

/// `Workspace#open_terminal`.
pub fn open_terminal(item: &mut WorkItem, worktree: Option<&str>, branch: Option<&str>) -> Result<Report, Failed> {
    let slug = item.slug.clone();
    let target = materialize(item, worktree, branch)?;
    report(target, None, |steps, target| {
        let name = target.branch.clone().unwrap_or_else(|| Path::new(&target.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        steps.push(warp_tab(&target.path, &format!("{slug} · {name}"), None)?);
        Ok(())
    })
}

/// `Workspace#work_on`: whichever of the dev server and the IDE are configured; refused with neither.
pub fn work_on(item: &mut WorkItem, mode: Option<&str>, worktree: Option<&str>, branch: Option<&str>) -> Result<Report, Failed> {
    let config = ProjectConfig::current();
    let project = item.project();
    let server = config.settings(project.as_deref()).and_then(|s| s.dev_server.clone());
    let ide = if server.is_some() { config.ide(project.as_deref()) } else { Some(config.ide(project.as_deref()).ok_or_else(no_ide)?) };
    let mode = match &server {
        Some(s) => Some(check_mode(s, &mode.map(String::from).or_else(|| item.pf_mode()).unwrap_or_default())?),
        None => None,
    };
    let target = materialize(item, worktree, branch)?;
    let name = project.clone().unwrap_or_default();
    report(target, mode.clone(), |steps, target| {
        steps.push(match (&server, &mode) {
            (Some(s), Some(m)) => pf_step(s, target, m)?,
            _ => format!("dev server skipped: {name} has none configured"),
        });
        steps.push(match ide {
            Some(ide) => ide_step(ide, target)?,
            None => "IDE skipped: no ide_command in config.yml".to_string(),
        });
        Ok(())
    })
}

/// `Workspace#stop_pf`: stops the dev server, whichever checkout it serves.
pub fn stop_pf() -> Result<String, Failed> {
    let config = ProjectConfig::current();
    let server = config.dev_server().ok_or_else(|| "no dev server configured in config.yml".to_string())?;
    crate::pf::stop(server)?;
    Ok(format!("{} stopped", server.command))
}

/// The page's reply for a finished workspace job.
pub fn job_reply(slug: &str, report: &Report) -> Value {
    json!({"ok": true, "message": format!("{slug}: {}", report.steps.join("; ")), "warning": report.target.warning, "switched": report.mode.is_some(), "path": report.target.path})
}

/// `TargetWorktree#materialize_branch`: a PR's branch rather than an item's — its existing worktree, else a
/// new one (the same never-switch-a-worktree rule as for items).
pub fn materialize_branch(project: &str, branch: &str, base: Option<&str>) -> Result<Target, Failed> {
    let root = root_for(Some(project))?;
    if let Some(path) = lookup(&checked_out(&root), branch) {
        return Ok(Target { path: path.clone(), branch: Some(branch.to_string()), create: false, warning: None, alternatives: vec![] });
    }
    let target = Target { path: new_worktree_path(&root, branch), branch: Some(branch.to_string()), create: true, warning: None, alternatives: vec![] };
    add_worktree(&root, branch, &target.path, base)?;
    prepare_test_db(Some(project), &target.path)?;
    Ok(target)
}

/// `Workspace#switch_pf_to_branch`: the PRs view's button; `mode` None means the project's default mode.
pub fn switch_pf_to_branch(project: &str, branch: &str, mode: Option<&str>) -> Result<Report, Failed> {
    let config = ProjectConfig::current();
    let server = dev_server_of(&config, Some(project))?;
    let mode = check_mode(&server, mode.unwrap_or(&server.default_mode))?;
    let target = materialize_branch(project, branch, None)?;
    let shown = mode.clone();
    report(target, Some(mode), |steps, target| {
        steps.push(pf_step(&server, target, &shown)?);
        Ok(())
    })
}

/// `Workspace#open_ide_on_branch`.
pub fn open_ide_on_branch(project: &str, branch: &str) -> Result<Report, Failed> {
    let config = ProjectConfig::current();
    let ide = config.ide(Some(project)).ok_or_else(no_ide)?;
    let target = materialize_branch(project, branch, None)?;
    report(target, None, |steps, target| {
        steps.push(ide_step(ide, target)?);
        Ok(())
    })
}
