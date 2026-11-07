//! Git's unified diff text as files → hunks → lines, the shape the chat view's diff pane renders.
//! Each line carries its old and new line number. Past `max_lines` the rest of the diff keeps its
//! files and stats but loses its lines (`truncated`). A port of `UnifiedDiff`, key order included.

use regex::Regex;
use serde_json::{json, Map, Value};

pub struct UnifiedDiff {
    budget: i64,
    pub truncated: bool,
}

fn strip_prefix_once<'a>(s: &'a str, prefix: &str) -> &'a str {
    s.strip_prefix(prefix).unwrap_or(s)
}

/// `String#each_line(chomp: true)`: a trailing `\n` or `\r\n` goes.
fn lines_chomped(text: &str) -> impl Iterator<Item = &str> {
    text.split_inclusive('\n').map(|l| l.strip_suffix("\r\n").or_else(|| l.strip_suffix('\n')).unwrap_or(l))
}

fn int(s: &str) -> i64 {
    s.parse().unwrap_or(i64::MAX)
}

fn new_file(line: &str) -> Map<String, Value> {
    let paths = line.strip_prefix("diff --git ").unwrap_or(line);
    let (old_path, new_path) = if paths.starts_with("a/") && paths.contains(" b/") {
        let mut it = paths[2..].splitn(2, " b/");
        (it.next().unwrap_or("").to_string(), it.next().unwrap_or("").to_string())
    } else {
        (paths.to_string(), paths.to_string())
    };
    let mut f = Map::new();
    for (k, v) in [("path", json!(new_path)), ("oldPath", json!(old_path)), ("status", json!("modified")), ("binary", json!(false)), ("additions", json!(0)), ("deletions", json!(0)), ("truncated", json!(false)), ("hunks", json!([]))] {
        f.insert(k.into(), v);
    }
    f
}

fn bump(file: &mut Map<String, Value>, key: &str) {
    let n = file[key].as_i64().unwrap_or(0) + 1;
    file.insert(key.into(), json!(n));
}

fn last_hunk(file: &mut Map<String, Value>) -> &mut Map<String, Value> {
    file.get_mut("hunks").and_then(Value::as_array_mut).and_then(|h| h.last_mut()).and_then(Value::as_object_mut).unwrap()
}

impl UnifiedDiff {
    pub fn new(max_lines: i64) -> UnifiedDiff {
        UnifiedDiff { budget: max_lines, truncated: false }
    }

    fn add_line(&mut self, file: &mut Map<String, Value>, kind: &str, old: Option<i64>, new: Option<i64>, text: &str) {
        if self.budget <= 0 {
            file.insert("truncated".into(), json!(true));
            self.truncated = true;
            return;
        }
        self.budget -= 1;
        let line = json!({"t": kind, "o": old, "n": new, "s": text});
        last_hunk(file).get_mut("lines").and_then(Value::as_array_mut).unwrap().push(line);
    }

    pub fn parse(&mut self, text: &str) -> Vec<Value> {
        let hunk_re = Regex::new(r"\A@@ -([0-9]+)(?:,([0-9]+))? \+([0-9]+)(?:,([0-9]+))? @@ ?(.*)\z").unwrap();
        let mut files: Vec<Map<String, Value>> = vec![];
        let mut have_file = false;
        let mut hunk: Option<(i64, i64)> = None; // next old and new line numbers of the current hunk
        let (mut old_left, mut new_left) = (0i64, 0i64);
        for line in lines_chomped(text) {
            if hunk.is_some() && (old_left > 0 || new_left > 0) {
                let mut file = files.pop().unwrap();
                let (mut old_no, mut new_no) = hunk.unwrap();
                let first = line.chars().next();
                let rest = first.map(|c| &line[c.len_utf8()..]).unwrap_or("");
                let handled = match first {
                    Some('\\') => true,
                    Some('+') => {
                        new_left -= 1;
                        bump(&mut file, "additions");
                        self.add_line(&mut file, "+", None, Some(new_no), rest);
                        new_no += 1;
                        true
                    }
                    Some('-') => {
                        old_left -= 1;
                        bump(&mut file, "deletions");
                        self.add_line(&mut file, "-", Some(old_no), None, rest);
                        old_no += 1;
                        true
                    }
                    Some(' ') | None => {
                        old_left -= 1;
                        new_left -= 1;
                        self.add_line(&mut file, " ", Some(old_no), Some(new_no), rest);
                        old_no += 1;
                        new_no += 1;
                        true
                    }
                    _ => false,
                };
                hunk = Some((old_no, new_no));
                files.push(file);
                if handled {
                    continue;
                }
            }

            if line.starts_with("diff --git ") {
                files.push(new_file(line));
                have_file = true;
                hunk = None;
            } else if !have_file {
                continue;
            } else if let Some(m) = hunk_re.captures(line) {
                old_left = m.get(2).map(|x| int(x.as_str())).unwrap_or(1);
                new_left = m.get(4).map(|x| int(x.as_str())).unwrap_or(1);
                let (old_start, new_start) = (int(&m[1]), int(&m[3]));
                hunk = Some((old_start, new_start));
                let file = files.last_mut().unwrap();
                file.get_mut("hunks").and_then(Value::as_array_mut).unwrap().push(json!({"header": line, "oldStart": old_start, "newStart": new_start, "lines": []}));
            } else if line.starts_with('\\') {
                continue;
            } else if hunk.is_none() {
                header_line(files.last_mut().unwrap(), line);
            }
        }
        files.into_iter().map(Value::Object).collect()
    }
}

fn header_line(file: &mut Map<String, Value>, line: &str) {
    if line.starts_with("new file mode") {
        file.insert("status".into(), json!("added"));
    } else if line.starts_with("deleted file mode") {
        file.insert("status".into(), json!("deleted"));
    } else if let Some(rest) = line.strip_prefix("rename from ") {
        file.insert("oldPath".into(), json!(rest));
    } else if let Some(rest) = line.strip_prefix("rename to ") {
        file.insert("path".into(), json!(rest));
        file.insert("status".into(), json!("renamed"));
    } else if line.starts_with("Binary files ") && line.ends_with(" differ") && Regex::new(r"\ABinary files .* differ\z").unwrap().is_match(line) {
        file.insert("binary".into(), json!(true));
    } else if let Some(rest) = line.strip_prefix("--- ") {
        let path = strip_prefix_once(rest, "a/");
        if path != "/dev/null" {
            file.insert("oldPath".into(), json!(path));
        }
    } else if let Some(rest) = line.strip_prefix("+++ ") {
        let path = strip_prefix_once(rest, "b/");
        if path != "/dev/null" {
            file.insert("path".into(), json!(path));
        }
    }
}
