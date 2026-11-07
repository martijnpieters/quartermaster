//! Read-only questions cleanup asks of one repo: its worktrees, its local branches, whether a
//! checkout is dirty, and — the one that decides what may be deleted — which commits of a head have
//! no equivalent anywhere else. Nothing here changes the repo.
//!
//! One instance serves one plan: which refs exist, and the touched paths and patch-ids of commits
//! (immutable, keyed by sha), are remembered across the heads it is asked about.

use std::collections::HashMap;

use crate::git::{git, git_bytes, git_with};

/// The trunk refs every safety check compares against, whichever exist.
pub const TRUNK_REFS: [&str; 4] = ["origin/main", "main", "origin/master", "master"];

#[derive(Clone)]
pub struct Worktree {
    pub path: String,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub locked: Option<String>,
    pub prunable: bool,
}

impl Worktree {
    pub fn is_locked(&self) -> bool {
        self.locked.is_some()
    }
}

pub struct GitRepo {
    pub root: String,
    commit_refs: HashMap<String, bool>,
    touched: HashMap<String, String>,
    patch_ids: HashMap<String, Option<String>>,
    trees: HashMap<String, Option<String>>,
}

fn is_object_name(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn lines(s: &str) -> Vec<&str> {
    if s.is_empty() { vec![] } else { s.split('\n').collect() }
}

/// `line.split(" ", 2)`: the first field and the rest (None when there is nothing after it).
fn split_once_awk(line: &str) -> (String, Option<String>) {
    let t = line.trim_start_matches(|c: char| c.is_ascii_whitespace());
    match t.find(|c: char| c.is_ascii_whitespace()) {
        None => (t.to_string(), None),
        Some(i) => {
            let rest = t[i..].trim_start_matches(|c: char| c.is_ascii_whitespace());
            (t[..i].to_string(), if rest.is_empty() { None } else { Some(rest.to_string()) })
        }
    }
}

impl GitRepo {
    pub fn new(root: &str) -> GitRepo {
        GitRepo { root: root.to_string(), commit_refs: HashMap::new(), touched: HashMap::new(), patch_ids: HashMap::new(), trees: HashMap::new() }
    }

    fn git(&self, args: &[&str]) -> Option<String> {
        git(Some(&self.root), args)
    }

    fn git_in(&self, args: &[&str], stdin: &str) -> Option<String> {
        git_with(Some(&self.root), args, stdin)
    }

    /// Every worktree, the main checkout first (as git lists it).
    pub fn worktrees(&self) -> Vec<Worktree> {
        let Some(listing) = self.git(&["worktree", "list", "--porcelain"]) else { return vec![] };
        listing
            .split("\n\n")
            .filter_map(|block| {
                let fields: Vec<&str> = block.lines().collect();
                let find = |prefix: &str| fields.iter().find(|l| l.starts_with(prefix)).map(|l| l[prefix.len()..].to_string());
                let path = find("worktree ")?;
                let lock = fields.iter().find(|l| **l == "locked" || l.starts_with("locked "));
                Some(Worktree {
                    path,
                    branch: find("branch refs/heads/"),
                    head: find("HEAD "),
                    locked: lock.map(|l| crate::ruby_text::strip(&l["locked".len()..]).to_string()),
                    prunable: fields.iter().any(|l| l.starts_with("prunable")),
                })
            })
            .collect()
    }

    pub fn local_branches(&self) -> Vec<String> {
        self.git(&["for-each-ref", "--format=%(refname:short)", "refs/heads"]).map(|o| lines(&o).into_iter().map(String::from).collect()).unwrap_or_default()
    }

    pub fn is_branch(&self, name: &str) -> bool {
        self.git(&["rev-parse", "--verify", "--quiet", &format!("refs/heads/{name}")]).is_some()
    }

    pub fn trunk_refs(&mut self) -> Vec<String> {
        self.existing_refs(&TRUNK_REFS.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    /// Tracked changes and untracked files of one checkout, as reasons.
    pub fn dirt(&self, path: &str) -> Vec<String> {
        let Some(out) = git(Some(path), &["status", "--porcelain"]) else { return vec!["git status failed".into()] };
        let l = lines(&out);
        let mut reasons = vec![];
        if l.iter().any(|x| !x.starts_with("??")) {
            reasons.push("uncommitted changes".to_string());
        }
        if l.iter().any(|x| x.starts_with("??")) {
            reasons.push("untracked files".to_string());
        }
        reasons
    }

    /// The names that resolve to a commit, checked in one call and remembered.
    fn existing_refs(&mut self, names: &[String]) -> Vec<String> {
        let mut missing: Vec<&String> = vec![];
        for n in names {
            if !self.commit_refs.contains_key(n) && !missing.contains(&n) {
                missing.push(n);
            }
        }
        if !missing.is_empty() {
            let stdin = missing.iter().map(|n| format!("{n}^{{commit}}")).collect::<Vec<_>>().join("\n");
            let out = self.git_in(&["cat-file", "--batch-check=%(objectname)"], &stdin);
            let found: Vec<String> = out.map(|o| lines(&o).into_iter().map(String::from).collect()).unwrap_or_default();
            let decided: Vec<(String, bool)> = missing.iter().enumerate().map(|(i, n)| ((*n).clone(), found.get(i).is_some_and(|f| is_object_name(f)))).collect();
            self.commit_refs.extend(decided);
        }
        names.iter().filter(|n| self.commit_refs[*n]).cloned().collect()
    }

    /// Merging head into base would leave base's tree untouched.
    fn content_merged(&mut self, base: &str, head: &str) -> bool {
        let merged = self.git(&["merge-tree", "--write-tree", base, head]);
        if self.trees.get(base).is_none_or(|t| t.is_none()) {
            let tree = self.git(&["rev-parse", &format!("{base}^{{tree}}")]);
            self.trees.insert(base.to_string(), tree);
        }
        match (merged, &self.trees[base]) {
            (Some(m), Some(tree)) => m.lines().next().map(|l| crate::ruby_text::strip(l) == tree).unwrap_or(false),
            _ => false,
        }
    }

    /// The commits of `head` that would be lost if it went away: not contained in any base, with no
    /// patch-equivalent among the commits the bases hold and head doesn't. A head whose combined
    /// change already sits in a `squash_into` ref that is also a base (a squash merge) loses nothing
    /// either. Newest first, as (sha, subject). None when git can't answer — never read that as
    /// "nothing to lose".
    pub fn unpreserved_commits(&mut self, head: &str, bases: &[String], squash_into: &[String]) -> Option<Vec<(String, Option<String>)>> {
        let mut unique: Vec<String> = vec![];
        for b in bases {
            if !unique.contains(b) {
                unique.push(b.clone());
            }
        }
        let bases = self.existing_refs(&unique);
        let out = self.git_in(&["log", "--no-merges", "--format=%H %s", "--stdin"], &revisions(&[head.to_string()], &bases))?;
        if out.is_empty() {
            return Some(vec![]);
        }
        let candidates: Vec<(String, Option<String>)> = out.split('\n').map(split_once_awk).collect();
        if bases.is_empty() {
            return Some(candidates);
        }
        let mut targets: Vec<&String> = vec![];
        for t in squash_into {
            if bases.contains(t) && !targets.contains(&t) {
                targets.push(t);
            }
        }
        for base in targets {
            if self.content_merged(base, head) {
                return Some(vec![]);
            }
        }
        let shas: Vec<String> = candidates.iter().map(|(s, _)| s.clone()).collect();
        let left = self.without_equivalent(&shas, head, &bases)?;
        Some(candidates.into_iter().filter(|(sha, _)| left.contains(sha)).collect())
    }

    /// Of `shas`, those whose patch-id no commit reachable from a base but not from head shares.
    fn without_equivalent(&mut self, shas: &[String], head: &str, bases: &[String]) -> Option<Vec<String>> {
        let others_out = self.git_in(&["rev-list", "--no-merges", "--stdin"], &revisions(bases, &[head.to_string()]))?;
        let others: Vec<String> = lines(&others_out).into_iter().map(String::from).collect();
        let mut all: Vec<String> = shas.to_vec();
        all.extend(others.iter().cloned());
        self.touched_paths(&all)?;
        let mut by_paths: HashMap<String, Vec<String>> = HashMap::new();
        for o in &others {
            by_paths.entry(self.touched[o].clone()).or_default().push(o.clone());
        }
        let suspects: Vec<(&String, Vec<String>)> = shas.iter().map(|s| (s, by_paths.get(&self.touched[s]).cloned().unwrap_or_default())).collect();
        let mut needed: Vec<String> = vec![];
        for (sha, alike) in &suspects {
            if !alike.is_empty() {
                needed.push((*sha).clone());
                needed.extend(alike.iter().cloned());
            }
        }
        self.patch_ids(&needed)?;
        // git patch-id prints nothing for an empty commit, where cherry calls two empty commits
        // equivalent; only a diff touching no path counts as empty
        let id_of = |this: &GitRepo, sha: &str| -> Option<String> {
            if this.touched[sha].is_empty() { Some(":empty".into()) } else { this.patch_ids.get(sha).cloned().flatten() }
        };
        Some(
            suspects
                .iter()
                .filter(|(sha, alike)| match id_of(self, sha) {
                    Some(id) => !alike.iter().any(|o| id_of(self, o).as_deref() == Some(id.as_str())),
                    None => true,
                })
                .map(|(sha, _)| (*sha).clone())
                .collect(),
        )
    }

    /// sha → the sorted paths its diff touches, for every sha given; None when git fails.
    fn touched_paths(&mut self, shas: &[String]) -> Option<()> {
        let mut missing: Vec<&String> = vec![];
        for s in shas {
            if !self.touched.contains_key(s) && !missing.contains(&s) {
                missing.push(s);
            }
        }
        if missing.is_empty() {
            return Some(());
        }
        let stdin = missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n");
        let out = self.git_in(&["log", "--no-walk=unsorted", "--stdin", "--root", "--no-renames", "--name-only", "--format=%x00%H"], &stdin)?;
        for chunk in out.split('\0').filter(|c| !c.is_empty()) {
            let mut it = chunk.split('\n');
            let Some(sha) = it.next() else { continue };
            let mut names: Vec<&str> = it.filter(|n| !n.is_empty()).collect();
            names.sort();
            self.touched.insert(sha.to_string(), names.join("\n"));
        }
        missing.iter().all(|s| self.touched.contains_key(*s)).then_some(())
    }

    /// sha → its patch-id (None for a commit without a diff); None when git fails.
    fn patch_ids(&mut self, shas: &[String]) -> Option<()> {
        let mut missing: Vec<&String> = vec![];
        for s in shas {
            if !self.patch_ids.contains_key(s) && !missing.contains(&s) {
                missing.push(s);
            }
        }
        if missing.is_empty() {
            return Some(());
        }
        let stdin = missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n");
        let diff = git_bytes(
            Some(&self.root),
            &[
                "log", "--no-walk=unsorted", "--stdin", "-p", "--root", "--no-renames", "--full-index", "--binary", "--no-ext-diff", "--no-textconv", "--no-color", "--src-prefix=a/", "--dst-prefix=b/",
                "--format=commit %H",
            ],
            stdin.as_bytes(),
        )?;
        let mut input = diff;
        input.push(b'\n');
        let ids = git_bytes(Some(&self.root), &["patch-id", "--stable"], &input)?;
        for s in &missing {
            self.patch_ids.insert((*s).clone(), None);
        }
        for line in String::from_utf8_lossy(&ids).split('\n') {
            let mut f = line.split_whitespace();
            if let (Some(id), Some(sha)) = (f.next(), f.next()) {
                self.patch_ids.insert(sha.to_string(), Some(id.to_string()));
            }
        }
        Some(())
    }
}

fn revisions(positive: &[String], negative: &[String]) -> String {
    positive.iter().cloned().chain(negative.iter().map(|r| format!("^{r}"))).collect::<Vec<_>>().join("\n")
}
