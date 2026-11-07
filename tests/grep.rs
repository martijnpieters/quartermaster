//! `qm grep`: a transcripts directory with every kind of entry and session, searched with phrases
//! that exercise the matching, the snippets, the grouping and the limits; transcripts of the output
//! generated from Ruby (`rg` does the candidate search in both).

mod common;
use common::*;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

fn user(text: &str, ts: &str) -> String {
    serde_json::json!({"type": "user", "timestamp": ts, "message": {"role": "user", "content": text}}).to_string()
}

fn assistant(blocks: serde_json::Value, ts: &str) -> String {
    serde_json::json!({"type": "assistant", "timestamp": ts, "message": {"role": "assistant", "content": blocks}}).to_string()
}

fn write(dir: &Path, project: &str, id: &str, lines: &[String], age_mins: u64) {
    let path = dir.join(project).join(format!("{id}.jsonl"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, lines.join("\n") + "\n").unwrap();
    fs::OpenOptions::new().write(true).open(&path).unwrap().set_modified(SystemTime::now() - Duration::from_secs(age_mins * 60)).unwrap();
}

fn start(id: &str, ts: &str, extra: &str) -> String {
    format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"/srv/app\",\"pid\":999999,\"ts\":\"{ts}\"{extra}}}\n")
}

fn item(root: &Path, slug: &str, title: &str, state: &str, sessions: &[&str]) {
    let list = sessions.iter().map(|s| format!("- id: {s}\n")).collect::<String>();
    fs::create_dir_all(root.join("work-items")).unwrap();
    fs::write(root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\ntitle: {title}\nstate: {state}\nproject: demo\ncreated: '2026-01-01'\nsessions:\n{list}---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n")).unwrap();
}

fn world(base: &Path) {
    let (root, tr) = (base.join("root"), base.join("transcripts"));
    fs::create_dir_all(&root).unwrap();
    let long_before = "word ".repeat(40);
    let long_after = " tail".repeat(60);
    write(&tr, "p1", "s-user", &[user("Please fix the Login bug in the form", "2026-03-01T10:00:00.000Z"), assistant(serde_json::json!([{"type": "text", "text": "I fixed the LOGIN handler"}]), "2026-03-01T10:01:00.000Z")], 10);
    write(&tr, "p1", "s-tools", &[
        serde_json::json!({"type": "user", "timestamp": "2026-03-02T10:00:00.000Z", "toolUseResult": {"stdout": "login in tool output"}, "message": {"content": [{"type": "tool_result", "content": "login in tool output"}]}}).to_string(),
        assistant(serde_json::json!([{"type": "tool_use", "name": "Bash", "input": {"command": "grep login"}}]), "2026-03-02T10:01:00.000Z"),
        serde_json::json!({"type": "user", "isSidechain": true, "message": {"content": "login in a sidechain"}}).to_string(),
        serde_json::json!({"type": "user", "isCompactSummary": true, "message": {"content": "login in a compact summary"}}).to_string(),
        serde_json::json!({"type": "user", "isMeta": true, "message": {"content": "login meta"}}).to_string(),
        serde_json::json!({"type": "system", "message": {"content": "login system"}}).to_string(),
    ], 20);
    write(&tr, "p1", "s-many", &[user("login one", "2026-03-03T10:00:00.000Z"), assistant(serde_json::json!("login two as a string"), "2026-03-03T10:00:30.000Z"), assistant(serde_json::json!([{"type": "text", "text": "a"}, {"type": "text", "text": "login three"}, {"type": "text", "text": null}]), "2026-03-03T10:01:00.000Z")], 30);
    write(&tr, "p2", "s-long", &[user(&format!("{long_before}NEEDLE{long_after}"), "2026-03-04T10:00:00.000Z")], 40);
    write(&tr, "p2", "s-squash", &[user("first line\n\n\tsecond   line NEEDLE third\r\nfourth", "2026-03-05T10:00:00.000Z")], 50);
    write(&tr, "p2", "s-unicode", &[user("héllo wörld 🙂 ünï — NEEDLE ünï 🙂 after", "2026-03-06T10:00:00.000Z")], 60);
    write(&tr, "p2", "s-escapes", &[user("say \"quoted\" and back\\slash and tab\there", "2026-03-07T10:00:00.000Z")], 70);
    write(&tr, "p2", "s-wide", &[user(&format!("{} NEEDLE and the rest of it", "é".repeat(50)), "2026-03-13T10:00:00.000Z")], 75);
    write(&tr, "p2", "s-nots", &[serde_json::json!({"type": "user", "message": {"content": "needle without any timestamp"}}).to_string()], 80);
    write(&tr, "p3", "s-aside", &[user("needle in an aside", "2026-03-08T10:00:00.000Z")], 90);
    write(&tr, "p3", "s-junk", &["not json needle".to_string(), "[1,2,\"needle\"]".to_string(), user("needle after junk", "2026-03-09T10:00:00.000Z")], 100);
    write(&tr, "p3/deeper", "s-subagent", &[user("needle in a sub-agent run", "2026-03-10T10:00:00.000Z")], 5);
    fs::write(tr.join("p3/notes.txt"), "needle in a text file").unwrap();
    write(&tr, "p3", "s-unlinked", &[user("needle unlinked and unclaimed", "2026-03-11T10:00:00.000Z")], 110);
    write(&tr, "p3", "s-case", &[user("NeEdLe in odd case", "2026-03-12T10:00:00.000Z")], 120);

    let mut log = String::new();
    for (id, ts, extra) in [
        ("s-user", "2026-03-01T10:00:00+00:00", ""),
        ("s-tools", "2026-03-02T10:00:00+00:00", ""),
        ("s-many", "2026-03-03T10:00:00+00:00", ""),
        ("s-long", "2026-03-04T10:00:00+00:00", ""),
        ("s-squash", "2026-03-05T10:00:00+02:00", ""),
        ("s-unicode", "2026-03-06T10:00:00+00:00", ""),
        ("s-escapes", "2026-03-07T10:00:00+00:00", ""),
        ("s-aside", "2026-03-08T10:00:00+00:00", ",\"aside\":\"some-item\""),
        ("s-unlinked", "2026-03-11T10:00:00+00:00", ""),
    ] {
        log += &start(id, ts, extra);
    }
    fs::write(root.join("sessions.jsonl"), log).unwrap();
    item(&root, "login-work", "Login work", "in_progress", &["s-user", "s-many", "s-tools"]);
    item(&root, "later-work", "Later work", "fridge", &["s-long", "s-squash", "s-user"]);
    item(&root, "done-work", "Done work", "resolved", &["s-unicode", "s-escapes"]);
}

fn run_grep(queries: &[&[&str]], prepare: impl Fn(&Path)) -> String {
    let base = tempfile::tempdir().unwrap();
    world(base.path());
    prepare(base.path());
    let qm = Qm::at(&base.path().join("root"));
    let tr = base.path().join("transcripts").display().to_string();
    let env = [("QM_TRANSCRIPTS_DIR", tr.as_str()), ("TZ", "UTC")];
    let steps: Vec<Step> = queries.iter().map(|q| (*q, "")).collect();
    transcript_env(&qm, "none", &steps, &env).replace(&base.path().display().to_string(), "BASE")
}

#[test]
fn phrases_find_only_conversation_text_grouped_by_item() {
    let queries: &[&[&str]] = &[
        &["grep", "login"],
        &["grep", "LOGIN"],
        &["grep", "fix", "the", "login"],
        &["grep", "needle"],
        &["grep", "NEEDLE"],
        &["grep", "quoted"],
        &["grep", "\"quoted\""],
        &["grep", "back\\slash"],
        &["grep", "tab\there"],
        &["grep", "ünï"],
        &["grep", "🙂"],
        &["grep", "no such phrase anywhere"],
        &["grep", "in a sidechain"],
        &["grep", "tool output"],
        &["grep", "compact summary"],
        &["grep"],
        &["grep", "  "],
        &["grep", "--help"],
        &["grep", "-n"],
        &["grep", "a.c*"],
        &["grep", "(login"],
    ];
    golden("grep_phrases.txt", &run_grep(queries, |_| {}));
}

#[test]
fn a_broad_phrase_stops_after_sixty_sessions_and_counts_the_rest() {
    let queries: &[&[&str]] = &[&["grep", "flood"], &["grep", "needle"]];
    let many = |base: &Path| {
        let tr = base.join("transcripts");
        let mut log = fs::read_to_string(base.join("root/sessions.jsonl")).unwrap();
        for i in 0..65u64 {
            let id = format!("f-{i:02}");
            write(&tr, "flood", &id, &[user("flood of sessions", &format!("2026-04-01T10:{:02}:00.000Z", i % 60))], 1000 + i);
            log += &start(&id, &format!("2026-04-{:02}T10:{:02}:00+00:00", 1 + i / 24, (i % 24) * 2), "");
        }
        fs::write(base.join("root/sessions.jsonl"), log).unwrap();
    };
    golden("grep_limit.txt", &run_grep(queries, many));
}

#[test]
fn what_only_ruby_reads_exactly_still_gives_ruby_s_answer() {
    // a transcript that is not UTF-8, a phrase whose case folding spans characters, a `message` that is a string
    let queries: &[&[&str]] = &[&["grep", "needle"], &["grep", "ss"], &["grep", "Strasse"]];
    let odd = |base: &Path| {
        let tr = base.join("transcripts");
        fs::create_dir_all(tr.join("odd")).unwrap();
        fs::write(tr.join("odd/s-bytes.jsonl"), b"{\"type\":\"user\",\"message\":{\"content\":\"needle \xff\"}}\n").unwrap();
    };
    golden("grep_ruby_only.txt", &run_grep(queries, odd));
    let fold = |base: &Path| {
        write(&base.join("transcripts"), "odd", "s-eszett", &[user(&format!("Die Straße first{} then STRASSE last", " filler".repeat(40)), "2026-05-01T10:00:00.000Z")], 3);
    };
    golden("grep_folding.txt", &run_grep(&[&["grep", "strasse"], &["grep", "straße"], &["grep", "ss"]], fold));
    let weird = |base: &Path| {
        write(&base.join("transcripts"), "odd", "s-msg", &[serde_json::json!({"type": "assistant", "message": "needle as text"}).to_string()], 3);
    };
    golden("grep_odd_message.txt", &run_grep(&[&["grep", "needle"]], weird));
}
