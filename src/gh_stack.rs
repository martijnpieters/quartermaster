//! The stacks the gh-stack extension records in a repo's common git dir (`<repo>/.git/gh-stack`,
//! shared by every worktree). Read-only: gh-stack owns the file. A missing, unreadable or
//! unfamiliar file reads as "no stacks". Unlike the Ruby class there are no in-process caches: they
//! only spare a long-lived GUI process work, and a CLI run reads each file once.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::data_root;
use crate::git::git;
use crate::project_map;
use crate::work_item::WorkItem;

const FILE_NAME: &str = "gh-stack";
const TRUNK_REFS: [&str; 4] = ["origin/main", "main", "origin/master", "master"];
const MAINLINE_BRANCHES: [&str; 3] = ["main", "master", "develop"];

#[derive(Clone)]
pub struct Entry {
    pub branch: String,
    pub pr_number: Value,
    pub pr_url: Value,
}

#[derive(Clone)]
pub struct Stack {
    pub entries: Vec<Entry>,
    pub trunk: Option<String>,
}

pub struct GhStack {
    stacks: Vec<Stack>,
}

fn truthy(v: &Value) -> bool {
    !matches!(v, Value::Null | Value::Bool(false))
}

/// `<common git dir>/gh-stack` of the repo at `root`, if it is one.
pub fn file_for(root: &str) -> Option<PathBuf> {
    let out = git(Some(root), &["rev-parse", "--git-common-dir"])?;
    let common = if Path::new(&out).is_absolute() { data_root::expand_path(&out) } else { data_root::expand_path(&format!("{root}/{out}")) };
    Some(common.join(FILE_NAME))
}

/// Bottom → top, as gh-stack stores them. Anything not shaped like a stack of named branches is skipped.
pub fn parse(json: &str) -> Vec<Stack> {
    let Ok(Value::Object(data)) = serde_json::from_str::<Value>(json) else { return vec![] };
    let Some(Value::Array(stacks)) = data.get("stacks") else { return vec![] };
    stacks
        .iter()
        .filter_map(|stack| {
            let stack = stack.as_object()?;
            let branches = stack.get("branches")?.as_array()?;
            let entries: Vec<Entry> = branches
                .iter()
                .filter_map(|b| {
                    let b = b.as_object()?;
                    let name = b.get("branch")?.as_str().filter(|n| !n.is_empty())?;
                    let empty = Map::new();
                    let pr = b.get("pullRequest").and_then(Value::as_object).unwrap_or(&empty);
                    Some(Entry { branch: name.to_string(), pr_number: pr.get("number").cloned().unwrap_or(Value::Null), pr_url: pr.get("url").cloned().unwrap_or(Value::Null) })
                })
                .collect();
            let trunk = stack.get("trunk").and_then(Value::as_object).and_then(|t| t.get("branch")).and_then(Value::as_str).map(String::from);
            if entries.is_empty() { None } else { Some(Stack { entries, trunk }) }
        })
        .collect()
}

pub fn for_root(root: &str) -> GhStack {
    let Some(path) = file_for(root) else { return GhStack { stacks: vec![] } };
    if !path.exists() {
        return GhStack { stacks: vec![] };
    }
    match std::fs::read_to_string(&path) {
        Ok(text) => GhStack { stacks: parse(&text) },
        Err(e) => {
            data_root::log_error("gh-stack", "IOError", &e.to_string());
            GhStack { stacks: vec![] }
        }
    }
}

fn project_root(item: &WorkItem) -> Option<String> {
    project_map::root_for(item.project().as_deref()).filter(|r| Path::new(r).is_dir())
}

/// What a state build (`qm show`) calls per item: auto-link the item's stack branches. Never fails.
pub fn sync(item: &mut WorkItem) {
    let Some(root) = project_root(item) else { return };
    if let Err(e) = for_root(&root).link_into(item) {
        data_root::log_error("gh-stack-sync", e.class, &e.message);
    }
}

/// The item's branches as chains: every stack holding one of them, newest-linked first, then each
/// other non-mainline branch alone ("loose"). `pr_states` and `pr_links` are keyed by branch.
pub fn chains(item: &WorkItem, pr_states: &Map<String, Value>, pr_links: &Map<String, Value>) -> Vec<Value> {
    let Some(root) = project_root(item) else { return vec![] };
    let refs = file_for(&root).and_then(|p| p.parent().map(|d| RefTips { common: d.to_path_buf(), root: root.clone() }));
    for_root(&root).chains(&item.branches(), &root, refs.as_ref(), pr_states, pr_links)
}

/// Tip shas of local and origin branches, read straight from the common git dir (loose ref files,
/// then packed-refs); a reftable repo has neither and asks git instead.
pub struct RefTips {
    common: PathBuf,
    root: String,
}

fn is_sha(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

impl RefTips {
    /// (ref, sha) for the first of the refs that exists.
    pub fn first(&self, refs: &[&str]) -> Option<(String, String)> {
        refs.iter().find_map(|r| self.sha_for(r).map(|sha| (r.to_string(), sha)))
    }

    fn sha_for(&self, reference: &str) -> Option<String> {
        let full = if reference.starts_with("origin/") { format!("refs/remotes/{reference}") } else { format!("refs/heads/{reference}") };
        if self.common.join("reftable").is_dir() {
            return git(Some(&self.root), &["rev-parse", "--verify", "--quiet", &format!("{full}^{{commit}}")]);
        }
        self.loose(&full).or_else(|| self.packed(&full))
    }

    fn loose(&self, full: &str) -> Option<String> {
        let path = self.common.join(full);
        if !path.is_file() {
            return None;
        }
        let text = std::fs::read_to_string(path).ok()?;
        let sha = crate::ruby_text::strip(&text);
        is_sha(sha).then(|| sha.to_string())
    }

    fn packed(&self, full: &str) -> Option<String> {
        let text = std::fs::read_to_string(self.common.join("packed-refs")).ok()?;
        let mut found = None;
        for line in text.lines() {
            let line = line.trim_start();
            let Some((sha, rest)) = line.split_once(char::is_whitespace) else { continue };
            if is_sha(sha) && crate::ruby_text::strip(rest.trim_start()) == full {
                found = Some(sha.to_string());
            }
        }
        found
    }
}

pub fn default_trunk(refs: Option<&RefTips>) -> String {
    match refs.and_then(|r| r.first(&TRUNK_REFS)) {
        Some((name, _)) => name.strip_prefix("origin/").unwrap_or(&name).to_string(),
        None => "main".to_string(),
    }
}

/// The ref tips of the repo at `root`, read from its common git dir.
pub fn ref_tips(root: &str) -> Option<RefTips> {
    file_for(root).and_then(|p| p.parent().map(|d| RefTips { common: d.to_path_buf(), root: root.to_string() }))
}

impl GhStack {
    /// The branch just below this one in its stack; None for the bottom branch or one in no stack.
    pub fn parent_of(&self, branch: &str) -> Option<String> {
        let stack = self.stacks_for(&[branch.to_string()]).into_iter().next()?;
        let index = stack.entries.iter().position(|e| e.branch == branch)?;
        (index > 0).then(|| stack.entries[index - 1].branch.clone())
    }

    /// Every stack holding any of these branches, in stack-file order.
    pub fn stacks_for(&self, branches: &[String]) -> Vec<&Stack> {
        self.stacks.iter().filter(|s| s.entries.iter().any(|e| branches.contains(&e.branch))).collect()
    }

    /// Links every branch of the item's stacks to it; cheap and write-free when nothing is missing.
    pub fn link_into(&self, item: &mut WorkItem) -> crate::error::Res<Vec<String>> {
        let have = item.branches();
        let mut missing: Vec<String> = vec![];
        for stack in self.stacks_for(&have) {
            for e in &stack.entries {
                if !have.contains(&e.branch) && !missing.contains(&e.branch) {
                    missing.push(e.branch.clone());
                }
            }
        }
        if missing.is_empty() {
            return Ok(vec![]);
        }
        item.link_branches(&missing)
    }

    fn chains(&self, branches: &[String], root: &str, refs: Option<&RefTips>, pr_states: &Map<String, Value>, pr_links: &Map<String, Value>) -> Vec<Value> {
        let own: Vec<String> = branches.iter().filter(|b| !MAINLINE_BRANCHES.contains(&b.as_str())).cloned().collect();
        let newest = |stack: &Stack| stack.entries.iter().map(|e| own.iter().position(|b| *b == e.branch).map(|i| i as i64).unwrap_or(-1)).max().unwrap_or(-1);
        let mut indexed: Vec<(usize, &Stack)> = self.stacks_for(&own).into_iter().enumerate().collect();
        indexed.sort_by_key(|(i, s)| (-newest(s), *i));
        let stacks: Vec<&Stack> = indexed.into_iter().map(|(_, s)| s).collect();
        let stacked: Vec<&String> = stacks.iter().flat_map(|s| s.entries.iter().map(|e| &e.branch)).collect();
        let trunk = default_trunk(refs);
        let mut out: Vec<Value> = stacks.iter().map(|s| stack_chain(s, root, refs, pr_states, pr_links)).collect();
        for branch in own.iter().rev().filter(|b| !stacked.contains(b)) {
            let state = pr_states.get(branch).cloned().unwrap_or(Value::Null);
            let link = pr_links.get(branch).and_then(Value::as_object).cloned().unwrap_or_default();
            out.push(json!({
                "kind": "loose", "trunk": trunk,
                "layers": [{
                    "branch": branch, "pr_number": link.get("number").cloned().unwrap_or(Value::Null),
                    "pr_url": link.get("url").cloned().unwrap_or(Value::Null),
                    "prState": state, "merged": state == "MERGED"
                }]
            }));
        }
        out
    }
}

/// `git rev-list --count from..to`, or None when git fails.
fn ahead(root: &str, from: &str, to: &str) -> Option<i64> {
    git(Some(root), &["rev-list", "--count", &format!("{from}..{to}")]).map(|s| crate::ruby_text::strip(&s).parse().unwrap_or(0))
}

/// git exits 1 for "no" and on errors alike; with shas resolved here an error reads as "no".
fn is_ancestor(root: &str, older: &str, newer: &str) -> bool {
    git(Some(root), &["merge-base", "--is-ancestor", older, newer]).is_some()
}

/// Layer facts: `ahead` counts parent..branch; `needsRestack` (the parent's tip is not in the
/// branch) is checked only above another stack branch; `merged` is the PR's word.
fn stack_chain(stack: &Stack, root: &str, refs: Option<&RefTips>, pr_states: &Map<String, Value>, pr_links: &Map<String, Value>) -> Value {
    let trunk = stack.trunk.clone().unwrap_or_else(|| default_trunk(refs));
    let layers: Vec<Value> = stack
        .entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let parent = if i == 0 { trunk.clone() } else { stack.entries[i - 1].branch.clone() };
            let origin_parent = format!("origin/{parent}");
            let parent_refs: [&str; 2] = if i == 0 { [&origin_parent, &parent] } else { [&parent, &origin_parent] };
            let parent_sha = refs.and_then(|r| r.first(&parent_refs)).map(|(_, sha)| sha);
            let origin_branch = format!("origin/{}", entry.branch);
            let sha = refs.and_then(|r| r.first(&[&entry.branch, &origin_branch])).map(|(_, sha)| sha);
            let state = pr_states.get(&entry.branch).cloned().unwrap_or(Value::Null);
            let link = if truthy(&entry.pr_number) { Map::new() } else { pr_links.get(&entry.branch).and_then(Value::as_object).cloned().unwrap_or_default() };
            let pick = |own: &Value, key: &str| if truthy(own) { own.clone() } else { link.get(key).cloned().unwrap_or(Value::Null) };
            let both = parent_sha.as_deref().zip(sha.as_deref());
            json!({
                "branch": entry.branch,
                "pr_number": pick(&entry.pr_number, "number"),
                "pr_url": pick(&entry.pr_url, "url"),
                "prState": state,
                "parent": parent,
                "ahead": both.and_then(|(p, s)| ahead(root, p, s)),
                "needsRestack": if i != 0 { both.map(|(p, s)| !is_ancestor(root, p, s)).unwrap_or(false) } else { false },
                "merged": state == "MERGED"
            })
        })
        .collect();
    json!({ "kind": "stack", "trunk": trunk, "layers": layers })
}
