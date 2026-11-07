//! Output helpers the commands share, as in the Ruby `CliFormat`.

use crate::data_root::home;
use crate::note::Note;

/// `puts`: the text plus a newline unless it already ends with one.
pub fn puts(text: &str) -> String {
    if text.ends_with('\n') { text.to_string() } else { format!("{text}\n") }
}

pub fn paint(text: &str, code: u8) -> String {
    format!("\x1b[{code}m{text}\x1b[0m")
}

pub fn note_line(n: &Note) -> String {
    format!("{} {} {}: {}", paint(&format!("#{}", n.position), 36), paint(&format!("[{}]", n.kind), 90), n.stamp, n.text)
}

/// Ruby's `ljust`, by characters.
pub fn ljust(s: &str, width: usize) -> String {
    let len = s.chars().count();
    format!("{s}{}", " ".repeat(width.saturating_sub(len)))
}

/// `str[0, n]` by characters.
pub fn prefix_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// The path with the first occurrence of the home directory shown as `~` ("?" when unknown).
pub fn short_path(path: Option<&str>) -> String {
    path.unwrap_or("?").replacen(&home(), "~", 1)
}

pub fn format_duration(seconds: Option<i64>) -> String {
    match seconds {
        None => "-".into(),
        Some(s) if s < 60 => format!("{s}s"),
        Some(s) if s < 3600 => format!("{}m", s / 60),
        Some(s) => format!("{}h{:02}m", s / 3600, (s % 3600) / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_use_seconds_minutes_then_hours() {
        assert_eq!(format_duration(None), "-");
        assert_eq!(format_duration(Some(59)), "59s");
        assert_eq!(format_duration(Some(60)), "1m");
        assert_eq!(format_duration(Some(3599)), "59m");
        assert_eq!(format_duration(Some(3600 + 125)), "1h02m");
    }

    #[test]
    fn puts_adds_a_newline_only_when_missing() {
        assert_eq!(puts("a"), "a\n");
        assert_eq!(puts("a\n"), "a\n");
        assert_eq!(puts(""), "\n");
    }

    #[test]
    fn ljust_pads_by_characters() {
        assert_eq!(ljust("é", 3), "é  ");
        assert_eq!(ljust("long", 2), "long");
    }
}
