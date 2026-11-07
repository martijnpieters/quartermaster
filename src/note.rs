pub const NOTE_KINDS: [&str; 3] = ["open", "decision", "history"];
pub const DEFAULT_NOTE_KIND: &str = "decision";

/// One note line of a work item's `## Notes` section.
#[derive(Clone)]
pub struct Note {
    pub stamp: String,
    pub kind: String,
    pub text: String,
    pub position: usize,
}

impl Note {
    /// The stored form; the default kind is left unmarked.
    pub fn to_line(&self) -> String {
        let marker = if self.kind == DEFAULT_NOTE_KIND { String::new() } else { format!(" [{}]", self.kind) };
        format!("- {}{}: {}", self.stamp, marker, self.text)
    }

    pub fn with_kind(&self, kind: &str) -> Note {
        Note { kind: kind.to_string(), ..self.clone() }
    }

    pub fn is_open(&self) -> bool {
        self.kind == "open"
    }
    pub fn is_history(&self) -> bool {
        self.kind == "history"
    }

    /// `- YYYY-MM-DD[ [kind]]: text`; an unknown kind marker falls back to the default.
    pub fn parse(line: &str, position: usize) -> Option<Note> {
        let line = line.strip_suffix('\n').unwrap_or(line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        let rest = line.strip_prefix("- ")?;
        let stamp = rest.get(..10)?;
        let b = stamp.as_bytes();
        let date_shaped = b.iter().enumerate().all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() });
        if !date_shaped {
            return None;
        }
        let mut tail = &rest[10..];
        let mut marker = None;
        if let Some(after) = tail.strip_prefix(" [") {
            if let Some(end) = after.find(']') {
                let name = &after[..end];
                if !name.is_empty() && name.bytes().all(|c| c.is_ascii_lowercase()) && after[end + 1..].starts_with(": ") {
                    marker = Some(name);
                    tail = &after[end + 1..];
                }
            }
        }
        let text = tail.strip_prefix(": ")?;
        let kind = match marker {
            Some(k) if NOTE_KINDS.contains(&k) => k,
            _ => DEFAULT_NOTE_KIND,
        };
        Some(Note { stamp: stamp.to_string(), kind: kind.to_string(), text: crate::ruby_text::strip(text).to_string(), position })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stamp_kind_and_text() {
        let n = Note::parse("- 2026-01-02 [open]: waiting on review\n", 3).unwrap();
        assert_eq!((n.stamp.as_str(), n.kind.as_str(), n.text.as_str(), n.position), ("2026-01-02", "open", "waiting on review", 3));
    }

    #[test]
    fn an_unmarked_note_is_a_decision_and_an_unknown_marker_falls_back() {
        assert_eq!(Note::parse("- 2026-01-02: plain", 1).unwrap().kind, "decision");
        assert_eq!(Note::parse("- 2026-01-02 [weird]: x", 1).unwrap().kind, "decision");
    }

    #[test]
    fn lines_that_are_not_notes_do_not_parse() {
        for line in ["", "- not a date: x", "2026-01-02: x", "- 2026-01-02 x", "- 2026-01-02[open]: x", "## Notes"] {
            assert!(Note::parse(line, 1).is_none(), "{line:?}");
        }
    }

    #[test]
    fn text_keeps_inner_colons_and_is_stripped() {
        assert_eq!(Note::parse("- 2026-01-02:   a: b  ", 1).unwrap().text, "a: b");
    }
}
