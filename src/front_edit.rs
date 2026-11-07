//! Targeted edits of a work item's YAML front matter. Only the touched key's lines change, so
//! every other key (rotations, snoozes, anything a newer qm wrote) survives byte for byte.

use crate::error::{QmError, Res};

/// The lines of `key`'s block: (index of the key line, index just past its last line).
pub fn block(lines: &[String], key: &str) -> Option<(usize, usize)> {
    let at = lines.iter().position(|l| l == &format!("{key}:") || l.starts_with(&format!("{key}: ")))?;
    let mut end = at + 1;
    while end < lines.len() && (lines[end].starts_with("- ") || lines[end] == "-" || lines[end].starts_with(' ')) {
        end += 1;
    }
    Some((at, end))
}

/// Replaces `key`'s lines with `new_lines`, or adds them at the end when the key is absent.
pub fn set(lines: &mut Vec<String>, key: &str, new_lines: Vec<String>) {
    match block(lines, key) {
        Some((at, end)) => {
            lines.splice(at..end, new_lines);
        }
        None => lines.extend(new_lines),
    }
}

/// Removes `key` and its block, if present.
pub fn delete(lines: &mut Vec<String>, key: &str) {
    if let Some((at, end)) = block(lines, key) {
        lines.drain(at..end);
    }
}

/// Adds `entry_lines` as one more item of the block-sequence `key`, creating the key at the end.
pub fn push_item(lines: &mut Vec<String>, key: &str, entry_lines: Vec<String>) -> Res<()> {
    match block(lines, key) {
        None => {
            lines.push(format!("{key}:"));
            lines.extend(entry_lines);
        }
        Some((at, end)) => {
            let key_line = lines[at].clone();
            if key_line == format!("{key}: []") {
                lines[at] = format!("{key}:");
                for (i, l) in entry_lines.into_iter().enumerate() {
                    lines.insert(at + 1 + i, l);
                }
            } else if key_line == format!("{key}:") {
                for (i, l) in entry_lines.into_iter().enumerate() {
                    lines.insert(end + i, l);
                }
            } else {
                return Err(QmError::new("RuntimeError", format!("front matter '{key}' is not a block list; edit it with the Ruby qm")));
            }
        }
    }
    Ok(())
}

/// Splits a block's item lines into items (a `- ` line plus its indented continuation).
pub fn items(lines: &[String], at: usize, end: usize) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let mut i = at + 1;
    while i < end {
        let start = i;
        i += 1;
        while i < end && lines[i].starts_with(' ') {
            i += 1;
        }
        out.push((start, i));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(String::from).collect()
    }

    #[test]
    fn push_item_creates_replaces_and_extends() {
        let mut l = lines("v: 1\ntitle: T");
        push_item(&mut l, "sessions", vec!["- id: a".into()]).unwrap();
        assert_eq!(l.join("\n"), "v: 1\ntitle: T\nsessions:\n- id: a");
        push_item(&mut l, "sessions", vec!["- id: b".into()]).unwrap();
        assert_eq!(l.join("\n"), "v: 1\ntitle: T\nsessions:\n- id: a\n- id: b");
        let mut empty = lines("sessions: []\nstate: x");
        push_item(&mut empty, "sessions", vec!["- id: a".into()]).unwrap();
        assert_eq!(empty.join("\n"), "sessions:\n- id: a\nstate: x");
    }

    #[test]
    fn push_item_keeps_the_keys_after_a_block_and_refuses_flow_lists() {
        let mut l = lines("sessions:\n- id: a\n  extra: 1\nstate: x");
        push_item(&mut l, "sessions", vec!["- id: b".into()]).unwrap();
        assert_eq!(l.join("\n"), "sessions:\n- id: a\n  extra: 1\n- id: b\nstate: x");
        let mut flow = lines("temp_branches: [a]");
        assert!(push_item(&mut flow, "temp_branches", vec!["- b".into()]).is_err());
    }

    #[test]
    fn items_group_continuation_lines() {
        let l = lines("sessions:\n- id: a\n  extra: 1\n- id: b");
        let (at, end) = block(&l, "sessions").unwrap();
        assert_eq!(items(&l, at, end), vec![(1, 3), (3, 4)]);
    }
}

#[cfg(test)]
mod set_tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(String::from).collect()
    }

    #[test]
    fn set_replaces_in_place_or_appends_and_delete_removes_the_block() {
        let mut l = lines("title: Old
  folded on
state: x");
        set(&mut l, "title", vec!["title: New".into()]);
        assert_eq!(l.join("\n"), "title: New\nstate: x");
        set(&mut l, "board", vec!["board: b".into()]);
        assert_eq!(l.join("\n"), "title: New\nstate: x\nboard: b");
        let mut with_list = lines("branches:\n- a\n- b\nstate: x");
        delete(&mut with_list, "branches");
        assert_eq!(with_list.join("\n"), "state: x");
        delete(&mut with_list, "absent");
        assert_eq!(with_list.len(), 1);
    }
}
