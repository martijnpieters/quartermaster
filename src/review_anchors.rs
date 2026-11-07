//! Where a GitHub review comment anchored to one commit of a PR can sit. GitHub validates it against
//! the PR diff base..commit (the merge-base of the PR's base with that commit) — not the commit's own
//! parent..commit — so only lines inside that diff's hunks (3 context lines, hunks one line apart
//! joined) take a comment: RIGHT lines by their number in the commit, LEFT lines by their number in
//! the base; `position` is the line's place in the file's diff after its first `@@` header. A port of
//! `ReviewAnchors`.

use std::collections::HashMap;

use regex::Regex;
use serde_json::Value;

use crate::git::raw;
use crate::unified_diff::UnifiedDiff;

pub const GITHUB_HUNKS: &str = "--inter-hunk-context=1";

/// `Arg` is the message of the `ArgumentError` Ruby raises; `Unsure` a value this port can't word.
pub enum E {
    Arg(String),
    Unsure,
}

type R<T> = Result<T, E>;

#[derive(Clone)]
pub struct Line {
    pub hunk: usize,
    pub position: i64,
    pub text: String,
}

pub struct ReviewAnchors {
    pub base_sha: String,
    pub commit: String,
    repo: String,
    files: HashMap<String, Value>,
    index: std::cell::RefCell<HashMap<(String, &'static str), HashMap<i64, Line>>>,
}

fn g(repo: &str, args: &[&str]) -> Option<String> {
    crate::git::git(Some(repo), args)
}

/// The parsed file's lines are the chunk's +/-/space lines in order, so one walk numbers them.
fn number_positions(chunk: &str, parsed: &mut Value) {
    let mut numbers: Vec<i64> = vec![];
    let mut position: Option<i64> = None;
    for raw_line in chunk.split_inclusive('\n').map(|l| l.strip_suffix("\r\n").or_else(|| l.strip_suffix('\n')).unwrap_or(l)) {
        let Some(p) = position else {
            if raw_line.starts_with("@@ ") {
                position = Some(0);
            }
            continue;
        };
        position = Some(p + 1);
        if raw_line.starts_with("@@ ") || raw_line.starts_with('\\') {
            continue;
        }
        numbers.push(p + 1);
    }
    let mut next = numbers.into_iter();
    for hunk in parsed["hunks"].as_array_mut().into_iter().flatten() {
        for line in hunk["lines"].as_array_mut().into_iter().flatten() {
            if let Some(n) = next.next() {
                line.as_object_mut().unwrap().insert("p".into(), n.into());
            }
        }
    }
}

pub fn new(repo: &str, base: &str, commit: &str) -> Result<ReviewAnchors, String> {
    let Some(commit_sha) = g(repo, &["rev-parse", "--verify", "--quiet", &format!("{commit}^{{commit}}")]) else { return Err(format!("{commit} is not a commit in {repo}")) };
    let Some(base_sha) = g(repo, &["merge-base", base, &commit_sha]) else { return Err(format!("{commit} shares no history with {base}")) };
    let Some(diff) = raw(repo, &["diff", "--no-color", "--no-ext-diff", "-M", GITHUB_HUNKS, &base_sha, &commit_sha, "--"], &[0]) else {
        return Err(format!("git diff {}..{} failed", &base_sha[..7.min(base_sha.len())], &commit_sha[..7.min(commit_sha.len())]));
    };
    let mut files = HashMap::new();
    let mut cuts: Vec<usize> = Regex::new(r"(?m)^diff --git ").unwrap().find_iter(&diff).map(|m| m.start()).collect();
    if cuts.first() != Some(&0) {
        cuts.insert(0, 0);
    }
    cuts.push(diff.len());
    for w in cuts.windows(2) {
        let chunk = &diff[w[0]..w[1]];
        if chunk.is_empty() {
            continue;
        }
        let Some(mut parsed) = UnifiedDiff::new(i64::MAX).parse(chunk).into_iter().next() else { continue };
        number_positions(chunk, &mut parsed);
        let path = parsed["path"].as_str().unwrap_or("").to_string();
        let old = parsed["oldPath"].as_str().unwrap_or("").to_string();
        let deleted = parsed["status"] == "deleted";
        files.entry(path).or_insert_with(|| parsed.clone());
        if deleted {
            files.entry(old).or_insert(parsed);
        }
    }
    Ok(ReviewAnchors { base_sha, commit: commit_sha, repo: repo.to_string(), files, index: Default::default() })
}

/// The diff pane's `old`/`new` are GitHub's LEFT/RIGHT: the key a side reads its line numbers from.
fn side_key(side: &Value) -> R<&'static str> {
    let text = match side {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        _ => return Err(E::Unsure),
    };
    let mapped = match text.as_str() {
        "old" => "LEFT".to_string(),
        "new" => "RIGHT".to_string(),
        other => other.to_uppercase(),
    };
    match mapped.as_str() {
        "LEFT" => Ok("o"),
        "RIGHT" => Ok("n"),
        _ => Err(E::Arg(format!("side must be LEFT or RIGHT, not {}", crate::gui_server::inspect(&mapped)))),
    }
}

fn to_i(v: &Value) -> R<i64> {
    match v {
        Value::Number(n) => n.as_i64().ok_or(E::Unsure),
        Value::String(s) => Ok(s.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0)),
        Value::Null => Ok(0),
        _ => Err(E::Unsure),
    }
}

impl ReviewAnchors {
    /// The file's base..commit diff as `UnifiedDiff` parses it, every line also carrying its diff `p`.
    pub fn file(&self, path: &str) -> Option<&Value> {
        self.files.get(path)
    }

    fn find_line(&self, path: &str, side: &Value, line: &Value) -> R<Option<Line>> {
        let key = side_key(side)?;
        let n = to_i(line)?;
        let mut cache = self.index.borrow_mut();
        let index = cache.entry((path.to_string(), key)).or_insert_with(|| {
            let mut found: HashMap<i64, Line> = HashMap::new();
            for (h, hunk) in self.files.get(path).and_then(|f| f["hunks"].as_array()).into_iter().flatten().enumerate() {
                for l in hunk["lines"].as_array().into_iter().flatten() {
                    if let Some(num) = l[key].as_i64() {
                        found.entry(num).or_insert_with(|| Line { hunk: h, position: l["p"].as_i64().unwrap_or(0), text: l["s"].as_str().unwrap_or("").to_string() });
                    }
                }
            }
            found
        });
        Ok(index.get(&n).cloned())
    }

    /// The numbers a comment on that side can take.
    pub fn lines(&self, path: &str, side: &Value) -> R<Vec<i64>> {
        let key = side_key(side)?;
        Ok(self.files.get(path).and_then(|f| f["hunks"].as_array()).into_iter().flatten().flat_map(|h| h["lines"].as_array().cloned().unwrap_or_default()).filter_map(|l| l[key].as_i64()).collect())
    }

    pub fn text(&self, path: &str, side: &Value, line: i64) -> R<Option<String>> {
        Ok(self.find_line(path, side, &Value::from(line))?.map(|l| l.text))
    }

    pub fn position(&self, path: &str, side: &Value, line: &Value) -> R<Option<i64>> {
        Ok(self.find_line(path, side, line)?.map(|l| l.position))
    }

    /// Whether GitHub takes a comment on this line of a view diffed from `view_base`: inside the hunks
    /// and still this text. A removed line is numbered by `view_base`, so it counts only when that is
    /// GitHub's base too.
    pub fn anchorable(&self, path: &str, side: &Value, line: &Value, text: &str, view_base: &str) -> R<bool> {
        if side_key(side)? == "o" && view_base != self.base_sha {
            return Ok(false);
        }
        Ok(self.find_line(path, side, line)?.is_some_and(|l| l.text == text))
    }

    /// Why GitHub won't take this draft at this commit, or None when it will.
    pub fn refusal(&self, entry: &serde_json::Map<String, Value>, view_base: &str) -> R<Option<String>> {
        let text = |k: &str| crate::draft_command::s(entry.get(k)).ok_or(E::Unsure);
        let path = text("path")?;
        let repo_last = text("repo")?.split('/').collect::<Vec<_>>().into_iter().rev().skip_while(|p| p.is_empty()).next().unwrap_or("").to_string();
        let at = format!("{repo_last}#{} @{}", text("number")?, self.commit.chars().take(7).collect::<String>());
        if self.file(&path).is_none() {
            return Ok(Some(format!("{path} isn't in the diff of {at}")));
        }
        if entry.get("subject").and_then(Value::as_str) == Some("file") {
            return Ok(None);
        }
        let code = text("code")?;
        let texts: Vec<&str> = code.split('\n').collect();
        let truthy = |k: &str| !matches!(entry.get(k), None | Some(Value::Null) | Some(Value::Bool(false)));
        let pick = |a: &str, b: &str| if truthy(a) { entry.get(a).cloned().unwrap_or(Value::Null) } else { entry.get(b).cloned().unwrap_or(Value::Null) };
        let ends = [(pick("start_side", "side"), pick("start_line", "line"), texts[0]), (entry.get("side").cloned().unwrap_or(Value::Null), entry.get("line").cloned().unwrap_or(Value::Null), texts[texts.len() - 1])];
        let mut failing = None;
        for (s, l, t) in &ends {
            if !self.anchorable(&path, s, l, t, view_base)? {
                failing = Some((s.clone(), l.clone()));
                break;
            }
        }
        let Some((side, line)) = failing.filter(|(_, l)| !matches!(l, Value::Null | Value::Bool(false))) else {
            let first = self.find_line(&path, &ends[0].0, &ends[0].1)?.ok_or(E::Unsure)?;
            let last = self.find_line(&path, &ends[1].0, &ends[1].1)?.ok_or(E::Unsure)?;
            if first.hunk == last.hunk {
                return Ok(None);
            }
            let label = crate::draft_command::lines_label(entry).ok_or(E::Unsure)?;
            return Ok(Some(format!("{path}:{} spans two hunks of {at}'s diff — GitHub takes a range of lines only inside one", label.strip_prefix(':').unwrap_or(&label))));
        };
        if side_key(&side)? == "o" && view_base != self.base_sha {
            return Ok(Some(format!("{path}:{} is a removed line of the selected commits, numbered from them, not from the PR's base — GitHub can't take a comment there; select all commits to comment on it", crate::draft_command::s(Some(&line)).ok_or(E::Unsure)?)));
        }
        Ok(Some(format!("GitHub can't take a comment on {path}:{} at {at}: only lines the PR's diff up to that commit shows (changes and 3 lines around them) take one", crate::draft_command::s(Some(&line)).ok_or(E::Unsure)?)))
    }

    pub fn repo(&self) -> &str {
        &self.repo
    }
}

/// `ReviewAnchors.refusal(repo, entry)`: the refusal for a draft as stored — anchored to its commit,
/// against its PR's base, numbered like the view it was made in (a selected run's parent, else the base).
pub fn refusal_stored(repo: &str, entry: &serde_json::Map<String, Value>) -> R<Option<String>> {
    let text = |k: &str| crate::draft_command::s(entry.get(k)).ok_or(E::Unsure);
    let base = text("base")?;
    if !(7..=64).contains(&base.len()) || !base.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(Some("the draft doesn't know its PR's base commit".into()));
    }
    let anchors = new(repo, &base, &text("commit")?).map_err(E::Arg)?;
    let range = text("range")?;
    let mut parts: Vec<&str> = range.split("..").collect();
    while parts.last() == Some(&"") {
        parts.pop();
    }
    let from = parts.first().copied();
    let view_base = match from {
        Some(f) => g(repo, &["rev-parse", "--verify", "--quiet", &format!("{f}^")]).unwrap_or_default(),
        None => anchors.base_sha.clone(),
    };
    anchors.refusal(entry, &view_base)
}
