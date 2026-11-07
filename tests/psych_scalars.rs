//! The Rust YAML emitter against the real Psych: every generated string is dumped by Ruby in one
//! run and must come out identical, or be one the emitter declines (left to the Ruby qm).

use quartermaster::psych;
use std::io::Write;
use std::process::{Command, Stdio};

const HEADS: &[&str] = &[
    "", " ", "-", "- ", "?", "? ", ":", ": ", "#", "'", "\"", "&", "*", "!", "|", ">", "%", "@", "`", "[", "]", "{", "}", ",", "~", "0", "1", "+", ".", "a",
    "A", "_", "é", "日", "(", ")", "/", "\\", "$", "=", "<", ";", "y", "n", "Y", "N", "yes", "No", "true", "Null", "on", "OFF", "1.", "0x", "0b", "-1", "2026-01-02",
    "12:", "<<",
];

const BODIES: &[&str] = &[
    "", "x", "x y", "x: y", "x #y", "x:", "x ", " x", "'", "\"", "a\\b", "1", "1.5", "true", "null", "yes", "No", "09", "0x1F", "2026-01-02", "12:30", "é",
    "x\ty", "🎉", "a,b", "a[b]", "a?b", "-", ":", "x  y", "x - y", "x ' y", "x \" y", "...", "---", "\u{a0}", "\u{feff}", "x\u{7f}y", "\u{1f}", "x:y",
    "1_000", "1,000", ".5", "1e3", "-.inf", ".nan", "0o17", "0777", "078", "1:2:3", "1:30.5", "٣", "ǅ", "x\u{301}",
];

fn corpus() -> Vec<String> {
    let mut all: Vec<String> = vec![];
    for h in HEADS {
        for b in BODIES {
            all.push(format!("{h}{b}"));
        }
    }
    let words = ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet", "kilo", "lima"];
    for lead in ["", "[", "'", "\"x", "a: ", "é", "- x "] {
        for len in 60..130 {
            let mut s = String::from(lead);
            let mut i = 0;
            while s.chars().count() < len {
                s.push_str(words[i % words.len()]);
                s.push(' ');
                i += 1;
            }
            s.truncate(s.trim_end().len());
            all.push(s.clone());
            all.push(s.replace(' ', "  "));
            all.push(format!("{s} tail"));
        }
        all.push(format!("{lead}{}", "w".repeat(100)));
        all.push(format!("{lead}{} end", "w".repeat(95)));
    }
    for t in [
        "2026-01-02T10:11:12Z", "2026-01-02 10:11:12", "2026-01-02 10:11:12.5", "2026-01-02 10:11:12 +05:30", "2026-01-02T10:11:12+0530",
        "2026-13-02 10:11:12", "2026-01-02 10:11:12.", "2026-01-02t10:11:12-05", "2026-1-2 1:02:03", "2026-01-02 24:00:00", "2026-01-02 10:11:12 Z",
        "2026-01-02 10:11:12 +25:00", "2026-02-31 10:11:12", "2026-01-02  10:11:12", "2026-01-02 10:11:12Z", "2026-01-02 10:11:12 +5", "2026-01-02 10:11:12.123456",
        "2026-01-02T10:11:12 extra", "2026-01-02 10:11", "-2026-01-02 10:11:12", "2026-01-32 10:11:12", "2026-01-02 10:60:12", "2026-01-02 10:11:60",
    ] {
        all.push(t.to_string());
    }
    all.sort();
    all.dedup();
    all.retain(|s| !s.contains('\n') && !s.contains('\r'));
    all
}

fn ruby_dumps(strings: &[String], key: &str) -> Vec<String> {
    let input = serde_json::to_string(strings).unwrap();
    let script = r#"require "yaml"; require "json"
key = ARGV[0]
puts JSON.generate(JSON.parse($stdin.read).map { |s| Psych.dump({ key => s }) })"#;
    let mut child = Command::new("ruby")
        .args(["-e", script, key])
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("ruby");
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn every_generated_string_dumps_as_psych_dumps_it_or_is_declined() {
    let strings = corpus();
    for key in ["title", "k"] {
        let expected = ruby_dumps(&strings, key);
        let (mut matched, mut declined, mut wrong) = (0, vec![], vec![]);
        for (s, want) in strings.iter().zip(&expected) {
            match psych::pair(key, s) {
                None => declined.push(s.clone()),
                Some(lines) => {
                    let got = format!("---\n{}\n", lines.join("\n"));
                    if &got == want {
                        matched += 1;
                    } else {
                        wrong.push(format!("{s:?}\n   rust: {got:?}\n   ruby: {want:?}"));
                    }
                }
            }
        }
        assert!(wrong.is_empty(), "{} of {} differ for key {key}:\n{}", wrong.len(), strings.len(), wrong.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
        declined.sort();
        assert_eq!(
            declined,
            [
                "2026-01-02 10:11:12 +25:00", "2026-01-02 10:11:12 +5", "2026-01-02 10:11:12.", "2026-01-02 10:11:60", "2026-01-02 10:60:12", "2026-01-02 24:00:00",
                "2026-01-02T10:11:12 extra", "2026-01-32 10:11:12", "2026-13-02 10:11:12", "<<", "\u{a0}", "٣", "\u{feff}", "🎉"
            ],
            "the strings left to Ruby changed");
        assert!(matched * 10 > strings.len() * 9, "only {matched} of {} matched; declined: {:?}", strings.len(), declined.iter().take(20).collect::<Vec<_>>());
        eprintln!("key {key}: {matched} identical, {} declined of {}", declined.len(), strings.len());
    }
}

#[test]
fn list_items_fold_like_psychs_sequences() {
    let strings: Vec<String> = (50..120)
        .map(|n| format!("{}", "word ".repeat(n / 5 + 1).trim_end()))
        .chain((50..120).map(|n| format!("[{}", "word ".repeat(n / 5 + 1).trim_end())))
        .collect();
    let input = serde_json::to_string(&strings).unwrap();
    let script = r#"require "yaml"; require "json"
puts JSON.generate(JSON.parse($stdin.read).map { |s| Psych.dump({ "branches" => [s] }) })"#;
    let mut child = Command::new("ruby").args(["-e", script]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let expected: Vec<String> = serde_json::from_slice(&child.wait_with_output().unwrap().stdout).unwrap();
    for (s, want) in strings.iter().zip(expected) {
        let got = format!("---\n{}\n", psych::list("branches", std::slice::from_ref(s)).unwrap().join("\n"));
        assert_eq!(got, want, "{s:?}");
    }
}
