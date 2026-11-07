//! `show-session`, `session-item`, `help` and `install-hooks`, pinned as transcripts generated from
//! Ruby over hand-written session logs and transcript files.

mod common;
use common::*;
use std::path::Path;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn transcript_lines(prompt_entries: &[&str]) -> String {
    let mut lines = vec![r#"{"type":"summary","summary":"x"}"#.to_string()];
    lines.extend(prompt_entries.iter().map(|s| s.to_string()));
    lines.join("\n") + "\n"
}

struct World {
    qm: Qm,
    home: tempfile::TempDir,
    projects: tempfile::TempDir,
}

fn start(id: &str, extra: &str, ts: &str) -> String {
    format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",{extra}\"ts\":\"{ts}\"}}\n")
}

fn end(id: &str, extra: &str, ts: &str) -> String {
    format!("{{\"v\":1,\"event\":\"end\",\"session_id\":\"{id}\",{extra}\"ts\":\"{ts}\"}}\n")
}

fn world() -> World {
    let qm = Qm::new();
    let home = tempfile::tempdir().unwrap();
    let projects = tempfile::tempdir().unwrap();
    let t = |name: &str| projects.path().join(format!("p/{name}.jsonl"));
    let me = std::process::id();

    write(&t("good"), &transcript_lines(&[
        r#"{"type":"user","isMeta":true,"message":{"content":"meta text"}}"#,
        r#"{"type":"user","toolUseResult":{"x":1},"message":{"content":[{"type":"tool_result","content":"r"}]}}"#,
        r#"{"type":"user","message":{"content":"<command-name>/clear</command-name>"}}"#,
        "this line is not json",
        r#"{"type":"user","message":{"content":[{"type":"text","text":"First "},{"type":"image"},{"type":"text","text":"real prompt"}]}}"#,
        r#"{"type":"user","message":{"content":"a later prompt"}}"#,
    ]));
    write(&t("long"), &transcript_lines(&[&format!(r#"{{"type":"user","message":{{"content":"{}"}}}}"#, "word ".repeat(120).trim_end())]));
    write(&t("multi"), &transcript_lines(&[r#"{"type":"user","message":{"content":"  line one\nline two — ünï\n\n  "}}"#]));
    write(&t("nothing"), &transcript_lines(&[r#"{"type":"user","isMeta":true,"message":{"content":"only meta"}}"#, r#"{"type":"assistant","message":{"content":"hi"}}"#]));
    write(&t("shapes"), &transcript_lines(&[
        r#"{"type":"user","message":{"content":{"not":"a list"}}}"#,
        r#"{"type":"user","message":{"content":[]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"text","text":"   "}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"text","text":null},{"type":"text","text":"from the array"}]}}"#,
    ]));
    write(&t("empty"), "");

    let path = |name: &str| t(name).display().to_string();
    let home_s = home.path().display().to_string();
    let mut log = String::new();
    log += &start("s-live", &format!("\"cwd\":\"/srv/app\",\"branch\":\"main\",\"project\":\"demo\",\"pid\":{me},\"transcript_path\":\"{}\",", path("good")), "2026-01-02T03:04:05+00:00");
    log += &start("s-resumable", &format!("\"cwd\":\"{home_s}/proj\",\"branch\":\"feat/x\",\"project\":\"demo\",\"pid\":999999,\"work_item\":\"item\",\"transcript_path\":\"{}\",", path("good")), "2026-01-02T03:04:05+00:00");
    log += &end("s-resumable", "\"reason\":\"clear\",\"duration_s\":125,\"commits\":[\"abc1234 first\",\"def5678 second\"],", "2026-01-02T03:06:10+00:00");
    log += &start("s-expired", "\"cwd\":\"/srv/gone\",\"pid\":999999,\"transcript_path\":\"/nonexistent/x.jsonl\",", "2026-01-03T00:00:00+00:00");
    log += &end("s-expired", "\"reason\":null,\"duration_s\":null,\"commits\":null,\"diff_stat\":\"\",", "2026-01-03T00:00:01+00:00");
    log += &start("s-aside", &format!("\"cwd\":\"/srv/app\",\"branch\":\"feat/x\",\"project\":\"demo\",\"pid\":999999,\"aside\":\"item\",\"transcript_path\":\"{}\",", path("multi")), "2026-01-04T00:00:00+00:00");
    log += &start("s-loose", &format!("\"cwd\":\"/srv/app\",\"pid\":999999,\"loose\":\"L1\",\"transcript_path\":\"{}\",", path("long")), "2026-01-05T00:00:00+00:00");
    log += &start("s-nocwd", &format!("\"cwd\":null,\"pid\":999999,\"transcript_path\":\"{}\",", path("nothing")), "2026-01-06T00:00:00+00:00");
    log += &start("s-shapes", &format!("\"cwd\":\"/x/{home_s}/y\",\"pid\":999999,\"transcript_path\":\"{}\",", path("shapes")), "2026-01-07T00:00:00+00:00");
    log += &start("s-empty", &format!("\"cwd\":\"/srv/app\",\"pid\":999999,\"transcript_path\":\"{}\",", path("empty")), "2026-01-08T00:00:00+00:00");
    log += &start("s-dismissed", &format!("\"cwd\":\"/srv/app\",\"pid\":999999,\"transcript_path\":\"{}\",", path("good")), "2026-01-09T00:00:00+00:00");
    log += &end("s-dismissed", "\"reason\":\"exit\",\"duration_s\":3700,\"commits\":[],", "2026-01-09T01:01:40+00:00");
    log += "{\"v\":1,\"event\":\"dismissed\",\"session_id\":\"s-dismissed\",\"ts\":\"2026-01-09T02:00:00+00:00\"}\n";
    log += &start("s-restarted", "\"cwd\":\"/srv/app\",\"pid\":999999,\"transcript_path\":\"/nonexistent/a\",", "2026-01-10T00:00:00+00:00");
    log += &end("s-restarted", "\"reason\":\"exit\",\"duration_s\":59,", "2026-01-10T00:00:59+00:00");
    log += &start("s-restarted", &format!("\"cwd\":\"/srv/app2\",\"pid\":{me},\"transcript_path\":\"{}\",", path("good")), "2026-01-10T00:05:00+00:00");
    write(&qm.root.path().join("sessions.jsonl"), &log);

    qm.ok(&["new", "Item with ünï title", "--project", "demo"]);
    let _ = std::fs::write(
        qm.root.path().join("work-items/item.md"),
        std::fs::read_to_string(qm.root.path().join("work-items/item-with-n-title.md")).unwrap().replace("sessions: []\n", "").replacen("\n---\n", "\nsessions:\n- id: s-resumable\n- id: s-live\n---\n", 1),
    );
    let _ = std::fs::remove_file(qm.root.path().join("work-items/item-with-n-title.md"));
    World { qm, home, projects }
}

fn show_all(w: &World, command: &str, ids: &[&str]) -> String {
    let env = [("HOME", w.home.path().to_str().unwrap()), ("QM_TRANSCRIPTS_DIR", w.projects.path().to_str().unwrap())];
    let mut out = String::new();
    for id in ids {
        let o = w.qm.run_with(&[command, id], "", &env);
        out.push_str(&format!("$ qm {command} {id}\nexit {}\n--- stdout\n{}--- stderr\n{}\n", o.code, o.stdout, o.stderr));
    }
    let out = out.replace(w.home.path().to_str().unwrap(), "HOME");
    w.qm.norm(&out).replace("SHA", "sha")
}

const IDS: &[&str] = &[
    "s-live", "s-resumable", "s-expired", "s-aside", "s-loose", "s-nocwd", "s-shapes", "s-empty", "s-dismissed", "s-restarted", "s-unknown", "",
];

#[test]
fn show_session_prints_tier_context_ending_and_the_first_prompt() {
    let w = world();
    golden("show_session.txt", &show_all(&w, "show-session", IDS));
}

#[test]
fn session_item_prints_the_linked_items_title() {
    let w = world();
    golden("session_item.txt", &show_all(&w, "session-item", &["s-resumable", "s-live", "s-expired", "s-unknown", ""]));
}

#[test]
fn session_item_with_a_titleless_item_prints_a_blank_line() {
    let qm = Qm::new();
    write(&qm.root.path().join("work-items/bare.md"), "---\nv: 1\nsessions:\n- id: sx\n---\nbody\n");
    let o = qm.run(&["session-item", "sx"]);
    assert_eq!((o.code, o.stdout.as_str()), (0, "\n"));
}

#[test]
fn help_and_its_spellings_print_the_usage() {
    let qm = Qm::new();
    let help = qm.ok(&["help"]);
    golden("help.txt", &help);
    assert_eq!(qm.ok(&["--help"]), help);
    assert_eq!(qm.ok(&["-h"]), help);
    assert_eq!(qm.ok(&["help", "ignored"]), help);
}

#[test]
fn install_hooks_prints_the_snippet_for_this_checkout() {
    let qm = Qm::new();
    let out = qm.ok(&["install-hooks"]);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).display().to_string();
    golden("install_hooks.txt", &out.replace(&repo, "REPO"));
}
