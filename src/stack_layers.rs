//! The layers of the gh-stack a diffed branch sits in, for the diff pane's layered commit list and
//! file circles: per layer its commits (parent..branch, newest first), its own +/− and files, and for
//! each commit the lower layer — or the trunk — that already holds a `git cherry` equivalent of it.
//! A port of `StackLayers`; Ruby caches git's answers per tip pair, which only spares a long-lived GUI.

use regex::Regex;
use serde_json::{json, Map, Value};

use crate::gh_stack;
use crate::git;

const MAX_COMMITS: usize = 200;

fn count_of(stat: &str, word: &str) -> i64 {
    Regex::new(&format!(r"([0-9]+) {word}")).unwrap().captures(stat).and_then(|c| c[1].parse().ok()).unwrap_or(0)
}

pub struct Layers {
    pub trunk: String,
    pub layers: Vec<Value>,
    pub file_layers: Map<String, Value>,
}

/// `{trunk, layers, fileLayers}` for the stack holding `branch`, else None.
pub fn call(root: &str, branch: Option<&str>) -> Option<Layers> {
    let stacks = gh_stack::for_root(root);
    let branch = branch?;
    let found = stacks.stacks_for(&[branch.to_string()]);
    let stack = found.first()?;
    let refs = gh_stack::ref_tips(root);
    let trunk = stack.trunk.clone().unwrap_or_else(|| gh_stack::default_trunk(refs.as_ref()));
    let origin_trunk = format!("origin/{trunk}");
    let trunk_sha = refs.as_ref().and_then(|r| r.first(&[&origin_trunk, &trunk])).map(|(_, sha)| sha);
    let tips: Vec<Option<String>> = stack
        .entries
        .iter()
        .map(|e| {
            let origin = format!("origin/{}", e.branch);
            refs.as_ref().and_then(|r| r.first(&[&e.branch, &origin])).map(|(_, sha)| sha)
        })
        .collect();
    let mut upstreams: Vec<(String, Option<String>)> = vec![(trunk.clone(), trunk_sha.clone())];
    upstreams.extend(stack.entries.iter().zip(&tips).map(|(e, t)| (e.branch.clone(), t.clone())));
    let mut file_layers: Map<String, Value> = Map::new();
    let mut layers = vec![];
    for (i, entry) in stack.entries.iter().enumerate() {
        let parent_sha = tips[..i].iter().flatten().last().cloned().or_else(|| trunk_sha.clone());
        let (mut layer, files) = layer_for(root, &entry.branch, parent_sha.as_deref(), tips[i].as_deref(), &upstreams[..=i]);
        layer.remove("files");
        for file in files {
            file_layers.entry(file).or_insert_with(|| json!([])).as_array_mut().unwrap().push(json!(i));
        }
        layers.push(Value::Object(layer));
    }
    Some(Layers { trunk, layers, file_layers })
}

fn layer_for(root: &str, branch: &str, parent_sha: Option<&str>, sha: Option<&str>, upstreams: &[(String, Option<String>)]) -> (Map<String, Value>, Vec<String>) {
    let (Some(parent_sha), Some(sha)) = (parent_sha, sha) else {
        let layer = json!({"branch": branch, "n": 0, "add": 0, "del": 0, "commits": [], "files": []});
        return (layer.as_object().unwrap().clone(), vec![]);
    };
    let (count, commits) = commits(root, parent_sha, sha);
    let (add, del, files) = numstat(root, parent_sha, sha);
    let also = also_in(root, parent_sha, sha, upstreams);
    let commits: Vec<Value> = commits
        .into_iter()
        .map(|mut c| {
            let name = also.iter().find(|(s, _)| Some(s.as_str()) == c.get("sha").and_then(Value::as_str)).map(|(_, n)| json!(n)).unwrap_or(Value::Null);
            c.insert("alsoIn".into(), name);
            Value::Object(c)
        })
        .collect();
    let layer = json!({"branch": branch, "n": count, "add": add, "del": del, "commits": commits, "files": files});
    (layer.as_object().unwrap().clone(), files)
}

/// parent..sha newest first, the newest MAX_COMMITS of them, and how many there are in all.
fn commits(root: &str, parent_sha: &str, sha: &str) -> (usize, Vec<Map<String, Value>>) {
    let out = git::raw(root, &["log", "--shortstat", "--format=%x1e%H%x1f%h%x1f%s", &format!("{parent_sha}..{sha}")], &[0]).unwrap_or_default();
    let all: Vec<Map<String, Value>> = out
        .split('\x1e')
        .filter_map(|record| {
            let (line, stat) = record.split_once('\n').unwrap_or((record, ""));
            let mut f = crate::ruby_text::strip(line).splitn(3, '\x1f');
            let full = f.next().filter(|s| !s.is_empty())?;
            let (short, subject) = (f.next().unwrap_or(""), f.next().unwrap_or(""));
            let c = json!({"sha": full, "short": short, "subject": subject, "add": count_of(stat, "insertion"), "del": count_of(stat, "deletion")});
            c.as_object().cloned()
        })
        .collect();
    (all.len(), all.into_iter().take(MAX_COMMITS).collect())
}

fn numstat(root: &str, parent_sha: &str, sha: &str) -> (i64, i64, Vec<String>) {
    let out = git::raw(root, &["diff", "--numstat", "-z", "--no-renames", &format!("{parent_sha}...{sha}"), "--"], &[0]).unwrap_or_default();
    let (mut add, mut del, mut files) = (0, 0, vec![]);
    for record in out.split('\0') {
        let mut f = record.splitn(3, '\t');
        let (a, d, file) = (f.next(), f.next(), f.next());
        let Some(file) = file else { continue };
        add += lenient_int(a.unwrap_or(""));
        del += lenient_int(d.unwrap_or(""));
        files.push(file.to_string());
    }
    (add, del, files)
}

/// `String#to_i`: leading digits, else 0.
fn lenient_int(s: &str) -> i64 {
    s.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0)
}

/// sha → the first upstream (the trunk, then the lowest layer up) whose own commits hold a
/// patch-equivalent of it.
fn also_in(root: &str, parent_sha: &str, sha: &str, upstreams: &[(String, Option<String>)]) -> Vec<(String, String)> {
    let mut also: Vec<(String, String)> = vec![];
    for (name, upstream) in upstreams.iter().rev() {
        let Some(upstream) = upstream else { continue };
        let out = git::raw(root, &["cherry", upstream, sha, parent_sha], &[0]).unwrap_or_default();
        for line in out.lines() {
            if let Some(c) = Regex::new(r"\A- ([0-9a-fA-F]+)").unwrap().captures(line) {
                let commit = c[1].to_string();
                match also.iter_mut().find(|(s, _)| *s == commit) {
                    Some(slot) => slot.1 = name.clone(),
                    None => also.push((commit, name.clone())),
                }
            }
        }
    }
    also
}
