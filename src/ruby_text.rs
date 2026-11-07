/// Ruby's `String#strip` (ASCII whitespace, plus NUL at the end).
pub fn strip(s: &str) -> &str {
    s.trim_start_matches([' ', '\t', '\n', '\u{b}', '\u{c}', '\r'])
        .trim_end_matches([' ', '\t', '\n', '\u{b}', '\u{c}', '\r', '\0'])
}

/// `String#inspect` of an ASCII string (UTF-8 encoded); None for anything with other characters.
pub fn inspect_ascii(s: &str) -> Option<String> {
    if !s.is_ascii() {
        return None;
    }
    let mut out = String::from("\"");
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{c}' => out.push_str("\\f"),
            '\u{b}' => out.push_str("\\v"),
            '\u{8}' => out.push_str("\\b"),
            '\u{7}' => out.push_str("\\a"),
            '\u{1b}' => out.push_str("\\e"),
            '#' if matches!(chars.get(i + 1), Some('{' | '$' | '@')) => out.push_str("\\#"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    Some(out)
}

/// Ruby's `String#rstrip`.
pub fn rstrip(s: &str) -> &str {
    s.trim_end_matches([' ', '\t', '\n', '\u{b}', '\u{c}', '\r', '\0'])
}

/// `str[0, n]` by characters.
pub fn prefix(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// `Integer(value, exception: false)` of a JSON value: `Ok(None)` is Ruby's nil, `Err` a value this
/// port does not read exactly (a number past 64 bits).
pub fn kernel_integer(v: &serde_json::Value) -> Result<Option<i64>, ()> {
    use serde_json::Value;
    match v {
        Value::Number(n) => match n.as_i64() {
            Some(i) => Ok(Some(i)),
            None => match n.as_f64() {
                Some(f) if f.is_finite() && f.abs() < 9.0e18 => Ok(Some(f.trunc() as i64)),
                _ => Err(()),
            },
        },
        Value::String(text) => kernel_integer_text(text),
        _ => Ok(None),
    }
}

fn kernel_integer_text(text: &str) -> Result<Option<i64>, ()> {
    let t = text.trim_matches([' ', '\t', '\n', '\u{b}', '\u{c}', '\r']);
    if t.contains('\0') {
        return Ok(None);
    }
    let (neg, rest) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let lower = rest.to_ascii_lowercase();
    let (radix, digits): (u32, &str) = if let Some(d) = lower.strip_prefix("0x") {
        (16, d)
    } else if let Some(d) = lower.strip_prefix("0b") {
        (2, d)
    } else if let Some(d) = lower.strip_prefix("0o") {
        (8, d)
    } else if let Some(d) = lower.strip_prefix("0d") {
        (10, d)
    } else if lower.len() > 1 && lower.starts_with('0') {
        (8, &lower[1..])
    } else {
        (10, lower.as_str())
    };
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") || !digits.chars().all(|c| c == '_' || c.is_digit(radix)) {
        return Ok(None);
    }
    match i64::from_str_radix(&digits.replace('_', ""), radix) {
        Ok(n) => Ok(Some(if neg { -n } else { n })),
        Err(_) => Err(()),
    }
}

/// `String#chomp`: one trailing "\r\n", "\n" or "\r".
pub fn chomp(s: &str) -> &str {
    s.strip_suffix("\r\n").or_else(|| s.strip_suffix('\n')).or_else(|| s.strip_suffix('\r')).unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_and_prefix_follow_ruby() {
        assert_eq!(strip(" \t x y\n\0"), "x y");
        assert_eq!(prefix("héllo wörld", 4), "héll");
        assert_eq!(prefix("ab", 8), "ab");
    }
}
