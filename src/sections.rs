//! The markdown body of a work item: `## ` sections, edited the way the Ruby `WorkItem` edits them.

use crate::note::Note;

fn lines(body: &str) -> Vec<&str> {
    body.split_inclusive('\n').collect()
}

fn section_end(lines: &[&str], idx: usize) -> usize {
    (idx + 1..lines.len()).find(|&i| lines[i].starts_with("## ")).unwrap_or(lines.len())
}

/// Adds `entry` at the end of `heading`'s section, before its trailing blank lines.
pub fn append_to_section(body: &str, heading: &str, entry: &str) -> String {
    let mut l = lines(body);
    let Some(idx) = l.iter().position(|x| x.starts_with(heading)) else {
        return format!("{body}\n{heading}\n{entry}");
    };
    let mut at = section_end(&l, idx);
    while at > idx + 1 && crate::ruby_text::strip(l[at - 1]).is_empty() {
        at -= 1;
    }
    l.insert(at, entry);
    l.concat()
}

/// Replaces everything under `heading` with `content`.
pub fn replace_section(body: &str, heading: &str, content: &str) -> String {
    let l = lines(body);
    let Some(idx) = l.iter().position(|x| x.starts_with(heading)) else {
        return format!("{body}\n{heading}\n{content}");
    };
    let end = section_end(&l, idx);
    format!("{}{}{}", l[..=idx].concat(), content, l[end..].concat())
}

/// Renders every stored note line of `## Notes` through `f`; other lines are kept as they are.
pub fn map_notes(body: &str, mut f: impl FnMut(Note) -> String) -> String {
    let mut seen = 0;
    let mut in_notes = false;
    let mut out = String::new();
    for line in lines(body) {
        if line.starts_with("## ") {
            in_notes = line.starts_with("## Notes");
        }
        match if in_notes { Note::parse(line, seen + 1) } else { None } {
            Some(note) => {
                seen += 1;
                out.push_str(&f(note));
                out.push('\n');
            }
            None => out.push_str(line),
        }
    }
    out
}

/// Rewrites every stored note line of `## Notes` through `f`, in the note's normal form.
pub fn rewrite_notes(body: &str, mut f: impl FnMut(Note) -> Note) -> String {
    map_notes(body, |n| f(n).to_line())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = "\n## Description\n\nD\n\n## Notes\n- 2026-01-01: a\n\n\n## Snapshots\n";

    #[test]
    fn append_goes_before_the_sections_trailing_blank_lines() {
        assert_eq!(
            append_to_section(BODY, "## Notes", "- 2026-01-02: b\n"),
            "\n## Description\n\nD\n\n## Notes\n- 2026-01-01: a\n- 2026-01-02: b\n\n\n## Snapshots\n"
        );
    }

    #[test]
    fn a_missing_section_is_added_at_the_end() {
        assert_eq!(append_to_section("text\n", "## Notes", "- x\n"), "text\n\n## Notes\n- x\n");
        assert_eq!(replace_section("text\n", "## Description", "\nD\n\n"), "text\n\n## Description\n\nD\n\n");
    }

    #[test]
    fn replace_keeps_the_heading_and_the_following_sections() {
        assert_eq!(replace_section(BODY, "## Description", "\nNew\n\n"), "\n## Description\n\nNew\n\n## Notes\n- 2026-01-01: a\n\n\n## Snapshots\n");
    }

    #[test]
    fn rewrite_normalises_only_note_lines_inside_the_notes_section() {
        let body = "## Notes\n- 2026-01-01 [weird]: x  \nfree\n## Other\n- 2026-01-01: untouched\n";
        assert_eq!(rewrite_notes(body, |n| n), "## Notes\n- 2026-01-01: x\nfree\n## Other\n- 2026-01-01: untouched\n");
    }
}
