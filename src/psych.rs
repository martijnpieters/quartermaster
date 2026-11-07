//! Writes YAML scalars the way Ruby's Psych does: Psych picks a style per string, libyaml decides
//! whether that style is allowed and folds long lines at column 80. A string this module cannot
//! reproduce with certainty gives `None`, and the command is left to the Ruby qm.

const BEST_WIDTH: usize = 80;
const INDENT: usize = 2;

#[derive(Clone, Copy, PartialEq)]
enum Style {
    Plain,
    Single,
    Double,
}

/// libyaml's IS_PRINTABLE (4-byte sequences are not; tab is not).
fn printable(c: char) -> bool {
    matches!(c, '\n' | ' '..='~' | '\u{85}' | '\u{a0}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}') && c != '\u{feff}'
}

fn is_break(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

fn blank_or_end(c: Option<&char>) -> bool {
    match c {
        None => true,
        Some(c) => *c == ' ' || *c == '\t' || is_break(*c),
    }
}

/// Psych's `ScalarScanner#tokenize` result: true when the string would read back as a non-string.
/// `None` when the answer depends on details this port does not reproduce.
pub fn reads_as_non_string(s: &str) -> Option<bool> {
    let cs: Vec<char> = s.chars().collect();
    if cs.is_empty() {
        return Some(true);
    }
    let in_class = |c: char| c.is_alphabetic() || c == '_' || c.is_ascii_whitespace() || "!@#$%^&*(){}<>|/\\~;=".contains(c);
    let stringy = in_class(cs[0]) || (cs.len() >= 2 && !matches!(cs[0], '0'..='9' | '.' | ':' | '-') && in_class(cs[1]));
    if stringy {
        if cs.len() > 5 {
            return Some(false);
        }
        let lower = s.to_lowercase();
        let first = lower.chars().next().unwrap();
        if !"ytonf~".contains(first) {
            return Some(false);
        }
        return Some(matches!(lower.as_str(), "~" | "null" | "yes" | "true" | "on" | "no" | "false" | "off"));
    }
    let digits = |c: char| c.is_ascii_digit();
    match time_shape(&cs) {
        TimeShape::No => {}
        TimeShape::Parses => return Some(true),
        TimeShape::Unsure => return None,
    }
    if is_date(&cs) {
        return Some(valid_date(&cs));
    }
    let lower = s.to_ascii_lowercase();
    if matches!(lower.as_str(), ".inf" | "+.inf" | "-.inf" | ".nan") {
        return Some(true);
    }
    if cs[0] == ':' && cs.len() >= 2 {
        return Some(true);
    }
    if is_sexagesimal(&cs) {
        return Some(true);
    }
    let body: &[char] = if matches!(cs[0], '-' | '+') { &cs[1..] } else { &cs[..] };
    // float: [-+]?([0-9][0-9_,]*)?\.[0-9]*([eE][-+][0-9]+)?
    if let Some(dot) = body.iter().position(|&c| c == '.') {
        let (int, frac) = (&body[..dot], &body[dot + 1..]);
        let int_ok = int.is_empty() || (digits(int[0]) && int.iter().all(|&c| digits(c) || c == '_' || c == ','));
        let (frac_digits, exp) = match frac.iter().position(|&c| c == 'e' || c == 'E') {
            Some(e) => (&frac[..e], Some(&frac[e + 1..])),
            None => (frac, None),
        };
        let exp_ok = exp.is_none_or(|x| x.len() >= 2 && matches!(x[0], '-' | '+') && x[1..].iter().all(|&c| digits(c)));
        if int_ok && frac_digits.iter().all(|&c| digits(c)) && exp_ok {
            if int.is_empty() && frac_digits.is_empty() && exp.is_none() {
                return Some(false); // a lone "." (with sign) stays a string
            }
            let raw: String = cs.iter().collect();
            if raw.contains('_') || raw.contains(',') {
                return None;
            }
            return Some(true);
        }
    }
    // integers (legacy regex, commas allowed)
    let b: String = body.iter().collect();
    let int_ok = |digits_ok: fn(char) -> bool, rest: &str| !rest.is_empty() && rest.chars().all(|c| digits_ok(c) || c == '_' || c == ',');
    if let Some(rest) = b.strip_prefix("0b") {
        return int_form(int_ok(|c| matches!(c, '0' | '1'), rest), rest);
    }
    if let Some(rest) = b.strip_prefix("0x") {
        return int_form(int_ok(|c| c.is_ascii_hexdigit(), rest), rest);
    }
    if b.len() > 1 && b.starts_with('0') {
        let rest = &b[1..];
        if int_ok(|c| matches!(c, '0'..='7'), rest) {
            return int_form(true, rest);
        }
    }
    if b == "0" {
        return Some(true);
    }
    if b.starts_with(|c: char| ('1'..='9').contains(&c)) {
        let bc: Vec<char> = b.chars().collect();
        let mut i = 1;
        let mut ok = true;
        while i < bc.len() {
            if bc[i].is_ascii_digit() {
                i += 1;
            } else if (bc[i] == ',' || bc[i] == '_') && bc.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
                i += 2;
            } else {
                ok = false;
                break;
            }
        }
        if ok {
            return Some(true);
        }
    }
    Some(false)
}

/// An int form whose cleaned text Ruby's `Integer()` would reject raises there; leave it to Ruby.
fn int_form(shaped: bool, rest: &str) -> Option<bool> {
    if !shaped {
        return Some(false);
    }
    let cleaned: String = rest.chars().filter(|c| *c != '_' && *c != ',').collect();
    if cleaned.is_empty() {
        None
    } else {
        Some(true)
    }
}

enum TimeShape {
    No,
    /// Matches Psych's TIME pattern and `Time.utc` accepts its fields, so it reads back as a Time.
    Parses,
    /// Matches in part, or in a form whose parsing this port does not reproduce.
    Unsure,
}

/// Psych's `TIME`: `YYYY-M-D[Tt ]H:MM:SS[.frac][ ][Z|±H[H][:][MM]]`.
fn time_shape(cs: &[char]) -> TimeShape {
    let n = cs.len();
    let digits_at = |from: usize| cs[from.min(n)..].iter().take_while(|c| c.is_ascii_digit()).count();
    let num = |from: usize, len: usize| -> u32 { cs[from..from + len].iter().collect::<String>().parse().unwrap_or(0) };
    if n < 14 || cs[0] == '-' || digits_at(0) != 4 || cs[4] != '-' {
        return TimeShape::No;
    }
    let year_ok = true;
    let mut i = 5;
    let m = digits_at(i);
    if !(1..=2).contains(&m) || cs.get(i + m) != Some(&'-') {
        return TimeShape::No;
    }
    let month = num(i, m);
    i += m + 1;
    let d = digits_at(i);
    if !(1..=2).contains(&d) {
        return TimeShape::No;
    }
    let day = num(i, d);
    i += d;
    match cs.get(i) {
        Some('T') | Some('t') => i += 1,
        Some(c) if c.is_ascii_whitespace() => {
            while cs.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
                i += 1;
            }
        }
        _ => return TimeShape::No,
    }
    let h = digits_at(i);
    if !(1..=2).contains(&h) || cs.get(i + h) != Some(&':') {
        return TimeShape::No;
    }
    let hour = num(i, h);
    i += h + 1;
    if digits_at(i) < 2 || cs.get(i + 2) != Some(&':') {
        return TimeShape::No;
    }
    let minute = num(i, 2);
    i += 3;
    if digits_at(i) < 2 {
        return TimeShape::No;
    }
    let second = num(i, 2);
    i += 2;
    let in_range = year_ok && (1..=12).contains(&month) && (1..=31).contains(&day) && hour < 24 && minute < 60 && second < 60;
    if cs.get(i) == Some(&'.') {
        i += 1;
        let f = digits_at(i);
        if f == 0 {
            return TimeShape::Unsure;
        }
        i += f;
    }
    if i == n {
        return if in_range { TimeShape::Parses } else { TimeShape::Unsure };
    }
    while cs.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
        i += 1;
    }
    let rest: String = cs[i..].iter().collect();
    let zone_ok = rest == "Z" || {
        let b = rest.as_bytes();
        let tail = rest.get(3..).unwrap_or("");
        let tail = tail.strip_prefix(':').unwrap_or(tail);
        rest.is_ascii()
            && b.len() >= 3
            && (b[0] == b'+' || b[0] == b'-')
            && b[1..3].iter().all(u8::is_ascii_digit)
            && rest[1..3].parse::<u32>().unwrap_or(99) < 24
            && (tail.is_empty() || (tail.len() == 2 && tail.bytes().all(|c| c.is_ascii_digit()) && tail.parse::<u32>().unwrap_or(99) < 60))
    };
    if zone_ok && in_range { TimeShape::Parses } else { TimeShape::Unsure }
}

fn is_date(cs: &[char]) -> bool {
    let s: String = cs.iter().collect();
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 3
        && parts[0].len() == 4
        && parts[0].chars().all(|c| c.is_ascii_digit())
        && matches!(parts[1].len(), 1 | 2)
        && parts[1].chars().all(|c| c.is_ascii_digit())
        && matches!(parts[2].len(), 1 | 2)
        && parts[2].chars().all(|c| c.is_ascii_digit())
        && {
            let (m, d): (u32, u32) = (parts[1].parse().unwrap(), parts[2].parse().unwrap());
            (1..=12).contains(&m) && (1..=31).contains(&d) && !(parts[1].len() == 2 && m == 0)
        }
}

fn valid_date(cs: &[char]) -> bool {
    let s: String = cs.iter().collect();
    let p: Vec<u32> = s.split('-').map(|x| x.parse().unwrap()).collect();
    chrono::NaiveDate::from_ymd_opt(p[0] as i32, p[1], p[2]).is_some()
}

fn is_sexagesimal(cs: &[char]) -> bool {
    let s: String = cs.iter().collect();
    let body = s.trim_start_matches(['-', '+']);
    let (main, frac) = match body.split_once('.') {
        Some((m, f)) => (m, Some(f)),
        None => (body, None),
    };
    let mut parts = main.split(':');
    let head = parts.next().unwrap_or("");
    let rest: Vec<&str> = parts.collect();
    (1..=2).contains(&rest.len())
        && head.starts_with(|c: char| c.is_ascii_digit())
        && head.chars().all(|c| c.is_ascii_digit() || c == '_')
        && rest.iter().all(|p| (1..=2).contains(&p.len()) && p.chars().all(|c| c.is_ascii_digit()) && p.parse::<u32>().unwrap() <= 59)
        && frac.is_none_or(|f| f.chars().all(|c| c.is_ascii_digit() || c == '_'))
}

struct Analysis {
    block_plain: bool,
    single: bool,
}

/// libyaml's `yaml_emitter_analyze_scalar`, for a block-context value with unicode output on.
fn analyze(cs: &[char]) -> Analysis {
    if cs.is_empty() {
        return Analysis { block_plain: true, single: true };
    }
    let (mut flow_ind, mut block_ind, mut special, mut leading_space, mut trailing_space) = (false, false, false, false, false);
    if cs.len() >= 3 && ((cs[..3] == ['-', '-', '-']) || (cs[..3] == ['.', '.', '.'])) {
        flow_ind = true;
        block_ind = true;
    }
    let mut preceded_by_ws = true;
    let mut followed_by_ws = blank_or_end(cs.get(1));
    for (i, &c) in cs.iter().enumerate() {
        let first = i == 0;
        let last = i == cs.len() - 1;
        if first {
            if "#,[]{}&*!|>'\"%@`".contains(c) {
                flow_ind = true;
                block_ind = true;
            }
            if c == '?' || c == ':' {
                flow_ind = true;
                if followed_by_ws {
                    block_ind = true;
                }
            }
            if c == '-' && followed_by_ws {
                flow_ind = true;
                block_ind = true;
            }
        } else {
            if ",?[]{}".contains(c) {
                flow_ind = true;
            }
            if c == ':' {
                flow_ind = true;
                if followed_by_ws {
                    block_ind = true;
                }
            }
            if c == '#' && preceded_by_ws {
                flow_ind = true;
                block_ind = true;
            }
        }
        if !printable(c) {
            special = true;
        }
        if c == ' ' {
            if first {
                leading_space = true;
            }
            if last {
                trailing_space = true;
            }
        }
        preceded_by_ws = blank_or_end(Some(&c));
        followed_by_ws = blank_or_end(cs.get(i + 2));
    }
    let _ = flow_ind;
    let mut block_plain = true;
    let mut single = true;
    if leading_space || trailing_space {
        block_plain = false;
    }
    if special {
        block_plain = false;
        single = false;
    }
    if block_ind {
        block_plain = false;
    }
    Analysis { block_plain, single }
}

fn requested_style(s: &str, cs: &[char]) -> Option<Style> {
    if matches!(s, "y" | "n") {
        return Some(Style::Double);
    }
    if let Some(&first) = cs.first() {
        if !first.is_ascii() && !first.is_alphabetic() {
            return None; // marks and symbols: Rust and Onigmo disagree on [[:word:]]
        }
        let word = first == '_' || first.is_alphanumeric();
        if !word && !cs[1..].contains(&'"') {
            return Some(Style::Double);
        }
    }
    let non_string = reads_as_non_string(s)?;
    let octal_nine = cs.first() == Some(&'0') && {
        let mut i = 1;
        while i < cs.len() && ('0'..='7').contains(&cs[i]) {
            i += 1;
        }
        matches!(cs.get(i), Some('8') | Some('9'))
    };
    Some(if non_string || octal_nine { Style::Single } else { Style::Plain })
}

fn escape(c: char) -> String {
    match c {
        '\0' => "\\0".into(),
        '\u{7}' => "\\a".into(),
        '\u{8}' => "\\b".into(),
        '\t' => "\\t".into(),
        '\u{b}' => "\\v".into(),
        '\u{c}' => "\\f".into(),
        '\u{1b}' => "\\e".into(),
        '"' => "\\\"".into(),
        '\\' => "\\\\".into(),
        '\u{85}' => "\\N".into(),
        '\u{a0}' => "\\_".into(),
        c if (c as u32) <= 0xff => format!("\\x{:02X}", c as u32),
        c if (c as u32) <= 0xffff => format!("\\u{:04X}", c as u32),
        c => format!("\\U{:08X}", c as u32),
    }
}

fn fold(out: &mut String, col: &mut usize) {
    out.push('\n');
    out.push_str(&" ".repeat(INDENT));
    *col = INDENT;
}

/// The scalar as it follows a `key: ` or `- ` prefix of `prefix_cols` characters; folds carry
/// their own newline and indent. `None` leaves the string to Ruby.
pub fn scalar(prefix_cols: usize, s: &str) -> Option<String> {
    if s.chars().any(is_break) || s == "<<" {
        return None;
    }
    let cs: Vec<char> = s.chars().collect();
    let mut style = requested_style(s, &cs)?;
    let analysis = analyze(&cs);
    if style == Style::Plain && !analysis.block_plain {
        style = Style::Single;
    }
    if style == Style::Single && !analysis.single {
        style = Style::Double;
    }
    let mut out = String::new();
    let mut col = prefix_cols;
    let n = cs.len();
    match style {
        Style::Plain => {
            let mut spaces = false;
            for (i, &c) in cs.iter().enumerate() {
                if c == ' ' {
                    if !spaces && col > BEST_WIDTH && cs.get(i + 1) != Some(&' ') {
                        fold(&mut out, &mut col);
                    } else {
                        out.push(' ');
                        col += 1;
                    }
                    spaces = true;
                } else {
                    out.push(c);
                    col += 1;
                    spaces = false;
                }
            }
        }
        Style::Single => {
            out.push('\'');
            col += 1;
            let mut spaces = false;
            for (i, &c) in cs.iter().enumerate() {
                if c == ' ' {
                    if !spaces && col > BEST_WIDTH && i != 0 && i != n - 1 && cs.get(i + 1) != Some(&' ') {
                        fold(&mut out, &mut col);
                    } else {
                        out.push(' ');
                        col += 1;
                    }
                    spaces = true;
                } else {
                    if c == '\'' {
                        out.push('\'');
                        col += 1;
                    }
                    out.push(c);
                    col += 1;
                    spaces = false;
                }
            }
            out.push('\'');
        }
        Style::Double => {
            out.push('"');
            col += 1;
            let mut spaces = false;
            for (i, &c) in cs.iter().enumerate() {
                if !printable(c) || c == '"' || c == '\\' || c == '\u{feff}' {
                    let e = escape(c);
                    col += e.chars().count();
                    out.push_str(&e);
                    spaces = false;
                } else if c == ' ' {
                    if !spaces && col > BEST_WIDTH && i != 0 && i != n - 1 {
                        fold(&mut out, &mut col);
                        if cs.get(i + 1) == Some(&' ') {
                            out.push('\\');
                            col += 1;
                        }
                    } else {
                        out.push(' ');
                        col += 1;
                    }
                    spaces = true;
                } else {
                    out.push(c);
                    col += 1;
                    spaces = false;
                }
            }
            out.push('"');
        }
    }
    Some(out)
}

/// `key: value` as lines (a long value folds onto continuation lines).
pub fn pair(key: &str, value: &str) -> Option<Vec<String>> {
    let text = scalar(key.chars().count() + 2, value)?;
    Some(format!("{key}: {text}").split('\n').map(String::from).collect())
}

/// `key:` with a block list of strings, or `key: []` when empty.
pub fn list(key: &str, items: &[String]) -> Option<Vec<String>> {
    if items.is_empty() {
        return Some(vec![format!("{key}: []")]);
    }
    let mut lines = vec![format!("{key}:")];
    for item in items {
        let text = scalar(2, item)?;
        lines.extend(format!("- {text}").split('\n').map(String::from));
    }
    Some(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(s: &str) -> String {
        pair("k", s).unwrap().join("\n")
    }

    #[test]
    fn plain_when_nothing_is_ambiguous() {
        assert_eq!(one("Plain title"), "k: Plain title");
        assert_eq!(one("feat/x-1"), "k: feat/x-1");
        assert_eq!(one("café ünï"), "k: café ünï");
    }

    #[test]
    fn strings_that_read_back_as_something_else_are_single_quoted() {
        for s in ["true", "No", "123", "1.5", "2026-01-02", "null", "09", "12:30", "1,000", "0x1F"] {
            assert_eq!(one(s), format!("k: '{s}'"), "{s}");
        }
    }

    #[test]
    fn a_non_word_first_character_gets_double_quotes() {
        for s in ["[WIP] x", "#1", "-5", ".5", ":sym", "/path", "~", "!x"] {
            assert_eq!(one(s), format!("k: \"{s}\""), "{s}");
        }
        assert_eq!(one("y"), "k: \"y\"");
        assert_eq!(one("Y"), "k: Y");
    }

    #[test]
    fn indicators_inside_force_quotes_and_quotes_double_up() {
        assert_eq!(one("a: b"), "k: 'a: b'");
        assert_eq!(one("a #b"), "k: 'a #b'");
        assert_eq!(one("trailing:"), "k: 'trailing:'");
        assert_eq!(one("it's: x"), "k: 'it''s: x'");
        assert_eq!(one("a:b"), "k: a:b");
    }

    #[test]
    fn special_characters_escape_in_double_quotes() {
        assert_eq!(one("tab\there"), "k: \"tab\\there\"");
        assert_eq!(one("party 🎉"), "k: \"party \\U0001F389\"");
    }

    #[test]
    fn what_ruby_would_handle_differently_is_left_to_it() {
        assert!(pair("k", "two\nlines").is_none());
        assert!(pair("k", "<<").is_none());
        assert!(pair("k", "2026-13-02 10:11:12").is_none());
        assert_eq!(one("2026-01-02 10:11:12"), "k: '2026-01-02 10:11:12'");
        assert_eq!(one("2026-11-05T18:00:00Z"), "k: '2026-11-05T18:00:00Z'");
    }

    #[test]
    fn long_values_fold_at_column_80_with_a_two_space_indent() {
        let text = "The quick brown fox jumps over the lazy dog while the five boxing wizards jump quickly past the sphinx of black quartz";
        assert_eq!(
            pair("title", text).unwrap(),
            vec![
                "title: The quick brown fox jumps over the lazy dog while the five boxing wizards jump".to_string(),
                "  quickly past the sphinx of black quartz".to_string()
            ]
        );
    }

    #[test]
    fn lists_are_block_style_and_empty_lists_are_flow() {
        assert_eq!(list("b", &["x".into(), "y z".into()]).unwrap(), vec!["b:", "- x", "- y z"]);
        assert_eq!(list("b", &[]).unwrap(), vec!["b: []"]);
    }
}
