//! Which parts of a branch diff (base → working tree) are not committed yet, worked out from two
//! more parsed diffs of the same checkout: `committed` (base → HEAD) and `uncommitted` (HEAD →
//! working tree, untracked files as added). Marks each added or removed line `"uncommitted": true`
//! (the key is left out otherwise) and gives each file a `commitStatus`. A port of `UncommittedMarks`.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

struct Side {
    added: HashSet<i64>,
    removed: HashSet<i64>,
    context: HashMap<i64, i64>,
    hunks: Vec<(i64, i64)>,
}

type Index = HashMap<String, usize>;

pub struct Marks {
    committed: Vec<Value>,
    uncommitted: Vec<Value>,
    committed_by_old: Index,
    committed_by_new: Index,
    uncommitted_by_old: Index,
    uncommitted_by_new: Index,
    sides: HashMap<(bool, usize), Option<std::rc::Rc<Side>>>,
}

fn by(files: &[Value], key: &str, keep: impl Fn(&Value) -> bool) -> Index {
    let mut map = HashMap::new();
    for (i, f) in files.iter().enumerate() {
        if keep(f) {
            map.insert(f[key].as_str().unwrap_or("").to_string(), i);
        }
    }
    map
}

fn status_is(f: &Value, s: &str) -> bool {
    f["status"].as_str() == Some(s)
}

fn flag(f: &Value, k: &str) -> bool {
    f[k].as_bool().unwrap_or(false)
}

fn mark_status(uncommitted: bool, committed: bool) -> &'static str {
    match (uncommitted, committed) {
        (true, true) => "mixed",
        (true, false) => "uncommitted",
        _ => "committed",
    }
}

pub fn apply(files: &mut [Value], committed: Vec<Value>, uncommitted: Vec<Value>) {
    let mut m = Marks {
        committed_by_old: by(&committed, "oldPath", |f| !status_is(f, "added")),
        committed_by_new: by(&committed, "path", |f| !status_is(f, "deleted")),
        uncommitted_by_old: by(&uncommitted, "oldPath", |f| !status_is(f, "added")),
        uncommitted_by_new: by(&uncommitted, "path", |f| !status_is(f, "deleted")),
        committed,
        uncommitted,
        sides: HashMap::new(),
    };
    for file in files.iter_mut() {
        m.mark(file);
    }
}

impl Marks {
    fn mark(&mut self, file: &mut Value) {
        let marks: Vec<Option<bool>> = if flag(file, "truncated") || flag(file, "binary") {
            vec![None]
        } else {
            let mut out = vec![];
            let snapshot = file.clone();
            if let Some(hunks) = file["hunks"].as_array_mut() {
                for hunk in hunks {
                    if let Some(lines) = hunk["lines"].as_array_mut() {
                        for line in lines.iter_mut().filter(|l| l["t"] != " ") {
                            let mark = self.mark_line(&snapshot, line);
                            if mark == Some(true) {
                                line.as_object_mut().unwrap().insert("uncommitted".into(), json!(true));
                            }
                            out.push(mark);
                        }
                    }
                }
            }
            out
        };
        let status = if marks.contains(&Some(true)) && !marks.contains(&None) {
            mark_status(marks.contains(&Some(true)), marks.contains(&Some(false)))
        } else {
            mark_status(marks.contains(&Some(true)) || self.touched_uncommitted(file), marks.contains(&Some(false)) || self.touched_committed(file))
        };
        file.as_object_mut().unwrap().insert("commitStatus".into(), json!(status));
    }

    fn mark_line(&mut self, file: &Value, line: &Value) -> Option<bool> {
        if line["t"] == "+" {
            self.added_uncommitted(file, line["n"].as_i64().unwrap_or(0))
        } else {
            self.removed_uncommitted(file, line["o"].as_i64().unwrap_or(0))
        }
    }

    fn side(&mut self, committed: bool, idx: usize) -> Option<std::rc::Rc<Side>> {
        if let Some(cached) = self.sides.get(&(committed, idx)) {
            return cached.clone();
        }
        let file = if committed { &self.committed[idx] } else { &self.uncommitted[idx] };
        let side = if flag(file, "truncated") || flag(file, "binary") { None } else { Some(std::rc::Rc::new(collect(file))) };
        self.sides.insert((committed, idx), side.clone());
        side
    }

    fn added_uncommitted(&mut self, file: &Value, number: i64) -> Option<bool> {
        let Some(&idx) = self.uncommitted_by_new.get(file["path"].as_str().unwrap_or("")) else { return Some(false) };
        self.side(false, idx).map(|s| s.added.contains(&number))
    }

    fn removed_uncommitted(&mut self, file: &Value, number: i64) -> Option<bool> {
        let (head_path, head_line) = match self.head_position(file, number) {
            Position::Gone => return Some(false),
            Position::Unknown => return None,
            Position::At(path, line) => (path, line),
        };
        let Some(&idx) = self.uncommitted_by_old.get(&head_path) else { return Some(false) };
        self.side(false, idx).map(|s| s.removed.contains(&head_line))
    }

    fn head_position(&mut self, file: &Value, number: i64) -> Position {
        let old_path = file["oldPath"].as_str().unwrap_or("").to_string();
        let Some(&idx) = self.committed_by_old.get(&old_path) else { return Position::At(old_path, number) };
        let (deleted, binary, path) = {
            let c = &self.committed[idx];
            (status_is(c, "deleted"), flag(c, "binary"), c["path"].as_str().unwrap_or("").to_string())
        };
        if deleted && !binary {
            return Position::Gone;
        }
        let Some(side) = self.side(true, idx) else { return Position::Unknown };
        if side.removed.contains(&number) {
            return Position::Gone;
        }
        if let Some(new) = side.context.get(&number) {
            return Position::At(path, *new);
        }
        let shift: i64 = side.hunks.iter().map(|(last_old, delta)| if *last_old < number { *delta } else { 0 }).sum();
        Position::At(path, number + shift)
    }

    fn touched_uncommitted(&self, file: &Value) -> bool {
        let old_path = file["oldPath"].as_str().unwrap_or("");
        let head_path = match self.committed_by_old.get(old_path) {
            Some(&i) => self.committed[i]["path"].as_str().unwrap_or("").to_string(),
            None => old_path.to_string(),
        };
        self.uncommitted_by_new.contains_key(file["path"].as_str().unwrap_or("")) || (!status_is(file, "added") && self.uncommitted_by_old.contains_key(&head_path))
    }

    fn touched_committed(&self, file: &Value) -> bool {
        (!status_is(file, "added") && self.committed_by_old.contains_key(file["oldPath"].as_str().unwrap_or(""))) || self.committed_by_new.contains_key(file["path"].as_str().unwrap_or(""))
    }
}

enum Position {
    Gone,
    Unknown,
    At(String, i64),
}

fn collect(file: &Value) -> Side {
    let mut side = Side { added: HashSet::new(), removed: HashSet::new(), context: HashMap::new(), hunks: vec![] };
    for hunk in file["hunks"].as_array().into_iter().flatten() {
        let (mut old_count, mut new_count) = (0i64, 0i64);
        for line in hunk["lines"].as_array().into_iter().flatten() {
            match line["t"].as_str() {
                Some("+") => {
                    side.added.insert(line["n"].as_i64().unwrap_or(0));
                }
                Some("-") => {
                    side.removed.insert(line["o"].as_i64().unwrap_or(0));
                }
                _ => {
                    side.context.insert(line["o"].as_i64().unwrap_or(0), line["n"].as_i64().unwrap_or(0));
                }
            }
            if line["t"] != "+" {
                old_count += 1;
            }
            if line["t"] != "-" {
                new_count += 1;
            }
        }
        let old_start = hunk["oldStart"].as_i64().unwrap_or(0);
        let last_old = if old_count == 0 { old_start } else { old_start + old_count - 1 };
        side.hunks.push((last_old, new_count - old_count));
    }
    side
}
