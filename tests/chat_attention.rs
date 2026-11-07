//! `qm attention` and the folds under it (`EventLog`, `TranscriptVitals`, `BackgroundJobs`): one
//! world is built twice with the same clock anchor, Ruby runs on one copy and Rust on the other,
//! and the printed chats and the persisted `events-fold.json` must agree.

mod common;
use quartermaster::background_jobs;
use quartermaster::transcript_vitals;
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn rust() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_qm"))
}

fn ruby() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("quartermaster")
}

fn ago(now: SystemTime, secs: u64) -> SystemTime {
    now - Duration::from_secs(secs)
}

/// Event stamps, as the hook writes them: whole seconds, UTC.
fn stamp(now: SystemTime, secs: u64) -> String {
    let t: chrono::DateTime<chrono::Utc> = ago(now, secs).into();
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Transcript stamps carry milliseconds.
fn stamp_ms(now: SystemTime, secs: u64) -> String {
    let t: chrono::DateTime<chrono::Utc> = ago(now, secs).into();
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn event(now: SystemTime, secs: u64, name: &str, session: &str, tool: Option<&str>, agent: Option<&str>) -> String {
    let mut e = json!({"v": 1, "ts": stamp(now, secs), "event": name, "session_id": session});
    if let Some(t) = tool {
        e["tool"] = json!(t);
    }
    if let Some(a) = agent {
        e["agent"] = json!(a);
    }
    format!("{e}\n")
}

fn assistant(now: SystemTime, secs: u64, input: u64, read: u64, creation: u64, one_hour: Option<u64>) -> String {
    let mut usage = json!({"input_tokens": input, "cache_read_input_tokens": read, "cache_creation_input_tokens": creation});
    if let Some(h) = one_hour {
        usage["cache_creation"] = json!({"ephemeral_1h_input_tokens": h, "ephemeral_5m_input_tokens": 0});
    }
    format!("{}\n", json!({"type": "assistant", "timestamp": stamp_ms(now, secs), "message": {"model": "claude-opus-5-5", "usage": usage}}))
}

fn interrupt(now: SystemTime, secs: u64) -> String {
    format!("{}\n", json!({"type": "user", "timestamp": stamp_ms(now, secs), "message": {"content": [{"type": "text", "text": "[Request interrupted by user]"}]}}))
}

fn launch(now: SystemTime, secs: u64, id: &str, description: &str) -> String {
    format!("{}\n", json!({"type": "user", "timestamp": stamp_ms(now, secs), "toolUseResult": {"status": "async_launched", "agentId": id, "description": description}}))
}

fn notification(now: SystemTime, secs: u64, id: &str, interim: bool) -> String {
    let note = if interim { "<note>the result below may be interim</note>" } else { "" };
    format!("{}\n", json!({"type": "user", "timestamp": stamp_ms(now, secs), "message": {"content": format!("<task-notification><task-id>{id}</task-id><status>completed</status>{note}</task-notification>")}}))
}

struct World {
    base: PathBuf,
    root: PathBuf,
    home: PathBuf,
    transcripts: PathBuf,
}

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn world(base: &Path, now: SystemTime) -> World {
    let w = World { base: base.to_path_buf(), root: base.join("root"), home: base.join("home"), transcripts: base.join("transcripts") };
    fs::create_dir_all(&w.home).unwrap();
    let me = std::process::id();
    let mut events = String::new();
    let mut log = String::new();
    let mut items: Vec<(String, String, String, Vec<String>)> = vec![]; // slug, state, extra front, sessions

    let mut chat = |slug: &str, state: &str, extra: &str, sessions: &[(&str, bool, Option<String>)], log: &mut String| {
        for (i, (id, live, transcript)) in sessions.iter().enumerate() {
            let path = w.transcripts.join(format!("p/{id}.jsonl"));
            if let Some(text) = transcript {
                put(&path, text);
            }
            log.push_str(&format!(
                "{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"/srv/app\",\"pid\":{},\"transcript_path\":\"{}\",\"ts\":\"{}\"}}\n",
                if *live { me } else { 999999 },
                path.display(),
                stamp(now, 3600 + i as u64 * 60)
            ));
        }
        items.push((slug.to_string(), state.to_string(), extra.to_string(), sessions.iter().map(|(id, ..)| id.to_string()).collect()));
    };
    let warm = |secs: u64| Some(assistant(now, secs, 1000, 181_000, 300, None));
    let e = |secs: u64, name: &str, id: &str| event(now, secs, name, id, None, None);

    chat("blocked-chat", "in_progress", "", &[("c-blocked", true, warm(90))], &mut log);
    events += &e(630, "UserPromptSubmit", "c-blocked");
    events += &event(now, 510, "PermissionRequest", "c-blocked", Some("Bash"), None);

    chat("your-turn", "in_progress", "", &[("c-yt", true, Some(assistant(now, 1230, 500, 60_000, 0, Some(100))))], &mut log);
    events += &e(1830, "UserPromptSubmit", "c-yt");
    events += &e(1230, "Stop", "c-yt");

    chat("working", "in_progress", "", &[("c-work", true, Some(assistant(now, 7230, 10, 20, 30, None)))], &mut log);
    events += &event(now, 210, "UserPromptSubmit", "c-work", Some("Edit"), None);

    chat("agent-resolves-block", "in_progress", "", &[("c-ab", true, None)], &mut log);
    events += &e(900, "UserPromptSubmit", "c-ab");
    events += &event(now, 800, "PermissionRequest", "c-ab", Some("Bash"), None);
    events += &event(now, 700, "PostToolUse", "c-ab", Some("Read"), Some("sub-1"));

    chat("agent-after-stop", "in_progress", "", &[("c-as", true, None)], &mut log);
    events += &e(1000, "UserPromptSubmit", "c-as");
    events += &e(950, "Stop", "c-as");
    events += &event(now, 600, "PostToolUse", "c-as", Some("Read"), Some("sub-2"));

    // a background agent asks for permission after the main turn's Stop; its next tool call resolves
    // that, back to the state before the block: still the user's turn
    chat("agent-block-after-stop", "in_progress", "", &[("c-bs", true, None)], &mut log);
    events += &e(1500, "UserPromptSubmit", "c-bs");
    events += &e(1400, "Stop", "c-bs");
    events += &event(now, 1300, "PermissionRequest", "c-bs", Some("Bash"), None);
    events += &event(now, 1200, "PostToolUse", "c-bs", Some("Read"), Some("sub-3"));

    // sessions long quiet: the fold forgets the older of these, keeps the newer
    events += &e(31 * 86400, "UserPromptSubmit", "old-31d");
    events += &e(29 * 86400, "UserPromptSubmit", "old-29d");

    chat("failure-resolves-block", "in_progress", "", &[("c-fv1", true, None)], &mut log);
    events += &e(1100, "PermissionRequest", "c-fv1");
    events += &e(1050, "PostToolUseFailure", "c-fv1");
    chat("stop-failure", "in_progress", "", &[("c-fv2", true, None)], &mut log);
    events += &e(1150, "UserPromptSubmit", "c-fv2");
    events += &e(1120, "StopFailure", "c-fv2");

    chat("esc-interrupted", "in_progress", "", &[("c-int", true, Some(interrupt(now, 2100) + &assistant(now, 2000, 5, 5, 5, None)))], &mut log);
    events += &e(2430, "UserPromptSubmit", "c-int");
    chat("interrupt-older", "in_progress", "", &[("c-io", true, Some(interrupt(now, 3000)))], &mut log);
    events += &e(2430, "UserPromptSubmit", "c-io");

    chat("waiting-job", "in_progress", "", &[("c-wait", true, Some(launch(now, 2000, "ag1", "Index the repo") + &assistant(now, 1900, 5, 5, 5, None)))], &mut log);
    events += &e(2430, "UserPromptSubmit", "c-wait");
    events += &e(1830, "Stop", "c-wait");
    chat("watcher-only", "in_progress", "", &[("c-wo", true, Some(launch(now, 2000, "ag2", "Watch CI for the PR")))], &mut log);
    events += &e(1830, "Stop", "c-wo");
    chat("stale-job", "in_progress", "", &[("c-sj", true, Some(launch(now, 13 * 3600, "ag3", "Ancient work")))], &mut log);
    events += &e(1830, "Stop", "c-sj");
    chat("finished-job", "in_progress", "", &[("c-fj", true, Some(launch(now, 2000, "ag4", "Short task") + &notification(now, 1900, "ag4", false)))], &mut log);
    events += &e(1830, "Stop", "c-fj");
    chat("interim-job", "in_progress", "", &[("c-ij", true, Some(launch(now, 11 * 3600, "ag5", "Long task") + &notification(now, 600, "ag5", true)))], &mut log);
    events += &e(1830, "Stop", "c-ij");
    let stop = json!({"type": "user", "timestamp": stamp_ms(now, 1500), "toolUseResult": {"task_id": "ag6", "message": "Successfully stopped task: ag6 (x)"}});
    chat("stopped-job", "in_progress", "", &[("c-st", true, Some(launch(now, 2000, "ag6", "Doomed task") + &format!("{stop}\n")))], &mut log);
    events += &e(1830, "Stop", "c-st");
    let bash_use = json!({"type": "assistant", "timestamp": stamp_ms(now, 2000), "message": {"content": [{"type": "tool_use", "id": "toolu_77", "name": "Bash", "input": {"command": "npm run build\nsecond line", "description": "  "}}]}});
    let bash_res = json!({"type": "user", "timestamp": stamp_ms(now, 1990), "message": {"content": [{"type": "tool_result", "tool_use_id": "toolu_77"}]}, "toolUseResult": {"backgroundTaskId": "bg1"}});
    chat("bash-background", "in_progress", "", &[("c-bg", true, Some(format!("{bash_use}\n{bash_res}\n")))], &mut log);
    events += &e(1830, "Stop", "c-bg");
    let resumed = json!({"type": "user", "timestamp": stamp_ms(now, 800), "message": {"content": "<task-notification><task-id>ag8</task-id><summary>Agent \"Reviver\" was resumed by the user</summary></task-notification>"}});
    chat("user-resumed", "in_progress", "", &[("c-ur", true, Some(format!("{resumed}\n")))], &mut log);
    events += &e(1830, "Stop", "c-ur");

    chat("snoozed-turn", "in_progress", &format!("turn_snoozed_at: '{}'\n", stamp(now, 10)), &[("c-sn", true, None)], &mut log);
    events += &e(1230, "Stop", "c-sn");
    chat("snooze-expired", "in_progress", &format!("turn_snoozed_at: '{}'\n", stamp(now, 5000)), &[("c-se", true, None)], &mut log);
    events += &e(1330, "Stop", "c-se");

    chat("multi-live", "in_progress", "", &[("m-1", true, None), ("m-2", true, warm(60))], &mut log);
    events += &e(1900, "Stop", "m-1");
    events += &e(300, "PermissionRequest", "m-2");
    chat("no-live-chat", "in_progress", "", &[("c-ended", false, warm(4000))], &mut log);
    chat("fridge-live", "fridge", "", &[("c-fl", true, None)], &mut log);
    chat("resolved-live", "resolved", "", &[("c-rl", true, None)], &mut log);
    chat("fridge-quiet", "fridge", "", &[("c-fq", false, None)], &mut log);
    chat("rotated", "in_progress", "rotations:\n- at: '2026-01-01T00:00:00+00:00'\n  from: c-rot\n  via: wrap\n", &[("c-rot", true, warm(100))], &mut log);
    events += &e(700, "UserPromptSubmit", "c-rot");
    chat("cache-cold", "in_progress", "", &[("c-cold", true, Some(assistant(now, 7200, 1, 2, 3, None)))], &mut log);
    events += &e(7300, "UserPromptSubmit", "c-cold");
    chat("no-transcript", "in_progress", "", &[("c-nt", true, None)], &mut log);
    events += &e(400, "UserPromptSubmit", "c-nt");
    chat("no-events", "in_progress", "", &[("c-ne", true, warm(30))], &mut log);
    chat("no-session-at-all", "in_progress", "", &[], &mut log);
    chat("unknown-event", "in_progress", "", &[("c-ue", true, None)], &mut log);
    events += &e(300, "Notification", "c-ue");
    events += "this line is not json\n";

    put(&w.root.join("sessions.jsonl"), &log);
    put(&w.root.join("events.jsonl"), &events);
    for (slug, state, extra, sessions) in items {
        let list = if sessions.is_empty() { String::new() } else { format!("sessions:\n{}", sessions.iter().map(|s| format!("- id: {s}\n")).collect::<String>()) };
        put(&w.root.join(format!("work-items/{slug}.md")), &format!("---\nv: 1\ntitle: {slug}\nstate: {state}\nproject: No project\ncreated: '2026-01-01'\n{extra}{list}---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n"));
    }
    w
}

fn run(bin: &Path, w: &World, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(bin)
        .args(args)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .env("QM_ROOT", &w.root)
        .env("HOME", &w.home)
        .env("QM_TRANSCRIPTS_DIR", &w.transcripts)
        .env("QM_TMUX_SOCKET", "qm-test-chat-attention")
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .current_dir(&w.base)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

fn fold(w: &World) -> String {
    let text = fs::read_to_string(w.root.join("events-fold.json")).unwrap_or_default();
    regex::Regex::new(r#""inode":\d+"#).unwrap().replace(&text, "\"inode\":N").into_owned()
}

fn pair(now: SystemTime) -> (tempfile::TempDir, tempfile::TempDir, World, World) {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (wa, wb) = (world(a.path(), now), world(b.path(), now));
    (a, b, wa, wb)
}

#[test]
fn attention_prints_the_same_chats_and_leaves_the_same_fold() {
    let now = SystemTime::now();
    let (_a, _b, ruby_world, rust_world) = pair(now);
    let (rc, rout, rerr) = run(&ruby(), &ruby_world, &["attention"]);
    let (xc, xout, xerr) = run(rust(), &rust_world, &["attention"]);
    assert_eq!((xc, &xout, &xerr), (rc, &rout, &rerr));
    assert_eq!(fold(&rust_world), fold(&ruby_world), "events-fold.json differs");
    let plain = regex::Regex::new("\x1b\\[[0-9;]*m").unwrap().replace_all(&xout, "").into_owned();
    for expected in [
        "🔴 blocked", "🟠 your turn", "🔵 working", "🟣 waiting on sub-agent", "⚪ no chat", "(snoozed)", "Index the repo", "npm run build", "Reviver", "ctx 182k", "cache warm", "cache cold",
    ] {
        assert!(plain.contains(expected), "the world should show {expected:?}:\n{plain}");
    }
    for absent in ["fridge-quiet", "Watch CI", "Ancient work", "Short task", "Doomed task"] {
        assert!(!plain.contains(absent), "{absent} must not show:\n{plain}");
    }
}

#[test]
fn a_second_run_reads_only_what_was_appended_and_a_truncated_log_refolds() {
    let now = SystemTime::now();
    let (_a, _b, ruby_world, rust_world) = pair(now);
    for w in [&ruby_world, &rust_world] {
        let _ = (w, run(&ruby(), w, &["attention"])); // both worlds start from a fold the Ruby side wrote
    }
    let extra = event(now, 100, "PermissionRequest", "c-work", Some("Bash"), None) + &event(now, 50, "Stop", "c-yt", None, None) + "{\"partial\": ";
    for w in [&ruby_world, &rust_world] {
        fs::OpenOptions::new().append(true).open(w.root.join("events.jsonl")).unwrap().write_all(extra.as_bytes()).unwrap();
    }
    let (rc, rout, _) = run(&ruby(), &ruby_world, &["attention"]);
    let (xc, xout, _) = run(rust(), &rust_world, &["attention"]);
    assert_eq!((xc, &xout), (rc, &rout));
    assert_eq!(fold(&rust_world), fold(&ruby_world), "the incremental fold differs");
    // truncate: the file shrank below the fold's offset
    for w in [&ruby_world, &rust_world] {
        let kept = event(now, 60, "UserPromptSubmit", "c-work", None, None);
        fs::write(w.root.join("events.jsonl"), kept).unwrap();
    }
    let (rc, rout, _) = run(&ruby(), &ruby_world, &["attention"]);
    let (xc, xout, _) = run(rust(), &rust_world, &["attention"]);
    assert_eq!((xc, &xout), (rc, &rout));
    assert_eq!(fold(&rust_world), fold(&ruby_world), "the refold differs");
    // no events file at all
    for w in [&ruby_world, &rust_world] {
        fs::remove_file(w.root.join("events.jsonl")).unwrap();
    }
    let (rc, rout, _) = run(&ruby(), &ruby_world, &["attention"]);
    let (xc, xout, _) = run(rust(), &rust_world, &["attention"]);
    assert_eq!((xc, &xout), (rc, &rout));
}

#[test]
fn nothing_needs_attention_in_an_empty_root() {
    let base = tempfile::tempdir().unwrap();
    let w = World { base: base.path().to_path_buf(), root: base.path().join("root"), home: base.path().join("home"), transcripts: base.path().join("t") };
    fs::create_dir_all(&w.home).unwrap();
    let (rc, rout, _) = run(&ruby(), &w, &["attention"]);
    let (xc, xout, _) = run(rust(), &w, &["attention"]);
    assert_eq!((xc, &xout), (rc, &rout));
    assert_eq!(xout, "Nothing needs attention.\n");
}

// ------------------------------------------------------------- the transcript parsers, directly

fn ruby_json(script: &str, input: &Value) -> Value {
    let mut child = Command::new("ruby")
        .args(["-e", &format!("require File.join(ARGV[0], 'lib', 'quartermaster'); include Quartermaster; puts JSON.generate(({script}).call(JSON.parse($stdin.read)))"), env!("CARGO_MANIFEST_DIR")])
        .env("LANG", "C.UTF-8")
        .env("QM_ROOT", tempfile::tempdir().unwrap().keep())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input.to_string().as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    serde_json::from_slice(&out.stdout).unwrap()
}

fn job_json(j: &background_jobs::Job) -> Value {
    json!({"id": j.id, "description": j.description, "workflow": j.workflow, "launched_at": j.launched_at, "command": j.command, "seen_at": j.seen_at})
}

#[test]
fn background_jobs_and_vitals_read_transcripts_like_ruby() {
    let now = SystemTime::now();
    let dir = tempfile::tempdir().unwrap();
    let nowf = now.duration_since(UNIX_EPOCH).unwrap().as_secs_f64();
    let ts = |s: u64| stamp_ms(now, s);
    let u = |s: u64, extra: Value| {
        let mut e = json!({"type": "user", "timestamp": ts(s)});
        e.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        format!("{e}\n")
    };
    let launch_r = |s: u64, r: Value| u(s, json!({"toolUseResult": r}));
    let note = |s: u64, body: &str| u(s, json!({"message": {"content": body}}));
    let shell_use = |s: u64, id: &str, cmd: &str, desc: Value| format!("{}\n", json!({"type": "assistant", "timestamp": ts(s), "message": {"content": [{"type": "tool_use", "id": id, "name": "Bash", "input": {"command": cmd, "description": desc}}]}}));
    let shell_res = |s: u64, id: &str, bg: Value| u(s, json!({"message": {"content": [{"type": "tool_result", "tool_use_id": id}]}, "toolUseResult": {"backgroundTaskId": bg}}));
    let corpus: Vec<(&str, String)> = vec![
        ("launch", launch_r(100, json!({"status": "async_launched", "agentId": "a1", "description": "D1"}))),
        ("workflow", launch_r(100, json!({"status": "async_launched", "taskId": "w1", "workflowName": "Flow", "taskType": "local_workflow"}))),
        ("sidechain", u(100, json!({"isSidechain": true, "toolUseResult": {"status": "async_launched", "agentId": "a1"}}))),
        ("ended", launch_r(300, json!({"status": "async_launched", "agentId": "a1", "description": "D1"})) + &note(200, "<task-notification><task-id>a1</task-id><status>done</status></task-notification>")),
        ("ended-by-queue", launch_r(300, json!({"status": "async_launched", "agentId": "a1", "description": "D1"})) + &u(200, json!({"type": "queue-operation", "content": "  <task-notification><task-id>a1</task-id></task-notification>"}))),
        ("interim", launch_r(40000, json!({"status": "async_launched", "agentId": "a1", "description": "D1"})) + &note(100, "<task-notification><task-id>a1</task-id><note>the result below may be interim</note></task-notification>")),
        ("interim-of-unknown", note(100, "<task-notification><task-id>zz</task-id><note>the result below may be interim</note></task-notification>")),
        ("too-old", launch_r(13 * 3600, json!({"status": "async_launched", "agentId": "a1", "description": "D1"}))),
        ("stop", launch_r(300, json!({"status": "async_launched", "agentId": "a1", "description": "D1"})) + &u(200, json!({"toolUseResult": {"task_id": "a1", "message": "Successfully stopped task: a1 (cmd)"}}))),
        ("stop-wrong-message", launch_r(300, json!({"status": "async_launched", "agentId": "a1", "description": "D1"})) + &u(200, json!({"toolUseResult": {"task_id": "a1", "message": "Failed to stop"}}))),
        ("resume", launch_r(900, json!({"status": "async_launched", "agentId": "a1", "description": "Orig"})) + &note(800, "<task-notification><task-id>a1</task-id></task-notification>") + &u(300, json!({"toolUseResult": {"resumedAgentId": "a1", "message": "revived"}}))),
        ("resume-unknown", u(300, json!({"toolUseResult": {"resumedAgentId": "a9", "message": "revived"}}))),
        ("user-resumed", note(300, "<task-notification><task-id>a7</task-id><summary>Agent \"Rev\" was resumed by the user</summary></task-notification>")),
        ("user-resumed-with-status", note(300, "<task-notification><task-id>a7</task-id><status>x</status><summary>Agent \"Rev\" was resumed by the user</summary></task-notification>")),
        ("shell-bg", shell_use(400, "t1", "make all\nmore", json!("Build it")) + &shell_res(390, "t1", json!("bg1"))),
        ("shell-no-description", shell_use(400, "t1", "  make all  \nmore", json!(null)) + &shell_res(390, "t1", json!(7))),
        ("shell-foreground", shell_use(400, "t1", "ls", json!("List")) + &shell_res(390, "t1", json!(null))),
        ("shell-watch", shell_use(400, "t1", "gh pr checks 5 --watch", json!("Run")) + &shell_res(390, "t1", json!("bg2")) + &shell_use(380, "t2", "gh run watch 9", json!("Other")) + &shell_res(370, "t2", json!("bg3"))),
        ("shell-attachment-mention", shell_use(400, "toolu_x", "sleep 9", json!("Nap")) + &format!("{}\n", json!({"type": "attachment", "timestamp": ts(395), "note": "toolu_x mentioned"})) + &shell_res(390, "toolu_x", json!("bg4"))),
        ("shell-then-notification", shell_use(400, "t1", "make", json!("Build")) + &shell_res(390, "t1", json!("bg1")) + &note(100, "<task-notification><task-id>bg1</task-id></task-notification>")),
        ("bad-lines", "not json\n{\"async_launched\": \n\n".to_string() + &launch_r(100, json!({"status": "async_launched", "agentId": "a1", "description": "D1"}))),
        ("partial-last-line", launch_r(100, json!({"status": "async_launched", "agentId": "a1", "description": "D1"})) + "{\"toolUseResult\": {\"status\": \"async_launched\", \"agentId\": \"a2\""),
        ("non-object-entry", "[1, 2, \"async_launched\"]\n".to_string() + &launch_r(100, json!({"status": "async_launched", "agentId": "a1", "description": "D1"}))),
        ("bad-timestamp", format!("{}\n", json!({"type": "user", "timestamp": "not a time", "toolUseResult": {"status": "async_launched", "agentId": "a1"}})) + &launch_r(100, json!({"status": "async_launched", "agentId": "a2", "description": "D2"}))),
        ("order", launch_r(100, json!({"status": "async_launched", "agentId": "late", "description": "Late"})) + &launch_r(500, json!({"status": "async_launched", "agentId": "early", "description": "Early"}))),
        ("numeric-ids", launch_r(100, json!({"status": "async_launched", "agentId": 5, "description": "Five"})) + &launch_r(90, json!({"status": "async_launched", "agentId": "5", "description": "Five str"}))),
        ("vitals-basic", assistant(now, 100, 1000, 181_000, 300, None) + &assistant(now, 200, 7, 7, 7, None)),
        ("vitals-1h", assistant(now, 100, 1, 2, 3, Some(5)) + &assistant(now, 200, 1, 2, 3, None)),
        ("vitals-tier-older", assistant(now, 300, 1, 2, 3, Some(5)) + &assistant(now, 200, 1, 2, 3, None) + &assistant(now, 100, 1, 2, 3, None)),
        ("vitals-interrupt", interrupt(now, 150) + &assistant(now, 100, 1, 2, 3, None)),
        ("vitals-interrupt-first", assistant(now, 100, 1, 2, 3, None) + &interrupt(now, 50)),
        ("vitals-synthetic", format!("{}\n", json!({"type": "assistant", "timestamp": ts(100), "message": {"model": "<synthetic>", "usage": {"input_tokens": 99}}})) + &assistant(now, 200, 4, 5, 6, None)),
        ("vitals-zero", assistant(now, 100, 0, 0, 0, None) + &assistant(now, 200, 4, 0, 0, None)),
        ("vitals-sidechain", format!("{}\n", json!({"type": "assistant", "isSidechain": true, "timestamp": ts(100), "message": {"usage": {"input_tokens": 99}}})) + &assistant(now, 200, 4, 5, 6, None)),
        ("vitals-string-tokens", format!("{}\n", json!({"type": "assistant", "timestamp": ts(100), "message": {"usage": {"input_tokens": "12", "cache_read_input_tokens": null}}}))),
        ("vitals-bad-time", format!("{}\n", json!({"type": "assistant", "timestamp": "nope", "message": {"usage": {"input_tokens": 12}}}))),
        ("vitals-no-time", format!("{}\n", json!({"type": "assistant", "message": {"usage": {"input_tokens": 12}}}))),
        ("vitals-interrupt-text", format!("{}\n", json!({"type": "user", "timestamp": ts(100), "message": {"content": "[Request interrupted by user for tool use]"}})) + &assistant(now, 200, 1, 1, 1, None)),
        ("vitals-message-string", "{\"type\":\"assistant\",\"message\":\"text\"}\n".to_string() + &assistant(now, 200, 1, 1, 1, None)),
        ("vitals-array-line", "[1]\n".to_string() + &assistant(now, 200, 1, 1, 1, None)),
        ("vitals-empty", String::new()),
        ("vitals-big-tail", {
            let mut t = assistant(now, 900, 77, 0, 0, None);
            let filler = format!("{}\n", json!({"type": "user", "message": {"content": "x".repeat(2000)}}));
            for _ in 0..400 {
                t += &filler;
            }
            t
        }),
        ("vitals-big-tail-interrupt", {
            let mut t = interrupt(now, 950) + &assistant(now, 900, 77, 0, 0, None);
            let filler = format!("{}\n", json!({"type": "user", "message": {"content": "x".repeat(2000)}}));
            for _ in 0..400 {
                t += &filler;
            }
            t
        }),
    ];
    let paths: Vec<String> = corpus
        .iter()
        .map(|(name, text)| {
            let p = dir.path().join(format!("{name}.jsonl"));
            fs::write(&p, text).unwrap();
            p.display().to_string()
        })
        .collect();
    let want = ruby_json(
        "->(i) { i['paths'].map { |p| now = Time.at(i['now']); jobs = BackgroundJobs.pending(p, now: now).map { |j| { 'id' => j.id, 'description' => j.description, 'workflow' => j.workflow, 'launched_at' => j.launched_at&.to_f, 'command' => j.command, 'seen_at' => j.seen_at&.to_f } }; v = TranscriptVitals.read(p); { 'jobs' => jobs, 'vitals' => v && { 'context' => v.context_tokens, 'last' => v.last_assistant_at&.to_f, 'warm' => v.cache_warm_until&.to_f, 'interrupted' => v.interrupted_at&.to_f } } } }",
        &json!({"paths": paths, "now": nowf}),
    );
    let with_jobs = want.as_array().unwrap().iter().filter(|w| !w["jobs"].as_array().unwrap().is_empty()).count();
    let with_vitals = want.as_array().unwrap().iter().filter(|w| !w["vitals"].is_null() && w["vitals"]["context"].is_number()).count();
    assert!(with_jobs >= 14 && with_vitals >= 12, "the corpus should produce results ({with_jobs} with jobs, {with_vitals} with vitals)");
    for (((name, _), path), want) in corpus.iter().zip(&paths).zip(want.as_array().unwrap()) {
        let jobs: Vec<Value> = background_jobs::pending(Some(Path::new(path)), nowf).iter().map(job_json).collect();
        assert_eq!(Value::Array(jobs), want["jobs"], "{name}: jobs");
        let vitals = transcript_vitals::read(Some(Path::new(path))).map(|v| json!({"context": v.context_tokens, "last": v.last_assistant_at, "warm": v.cache_warm_until, "interrupted": v.interrupted_at}));
        assert_eq!(vitals.unwrap_or(Value::Null), want["vitals"], "{name}: vitals");
    }
}

#[test]
fn watchers_are_told_apart_like_ruby_does() {
    let descriptions = [
        "Watch CI", "watching deploy", "monitor the queue", "Babysit PR 5", "stopwatch", "watch_x", "re-watch", "watché", "日 watch", "日watch", "éwatch", "WATCHER", "unwatch", "Monitoring: x", "", "no match here",
        "watch\u{a0}me", "foo-watch", "foo/watch", "_watch", "9watch",
    ];
    let commands: Vec<Option<&str>> = vec![None, Some("gh pr checks 5 --watch"), Some("gh  pr  checks 5 -q --watch"), Some("gh pr checks 5 --watching"), Some("gh run watch 1"), Some("gh\u{a0}run watch 1"), Some("xgh run watch 1"), Some("gh pr checks 5;--watch"), Some("echo gh run watch")];
    let mut cases = vec![];
    for d in descriptions {
        for workflow in [false, true] {
            cases.push(json!({"d": d, "c": null, "w": workflow}));
        }
    }
    for c in &commands {
        cases.push(json!({"d": "x", "c": c, "w": false}));
        cases.push(json!({"d": "x", "c": c, "w": true}));
    }
    let want = ruby_json("->(cases) { cases.map { |c| BackgroundJobs::Job.new(id: 'i', description: c['d'], workflow: c['w'], command: c['c']).watcher? } }", &Value::Array(cases.clone()));
    for (case, want) in cases.iter().zip(want.as_array().unwrap()) {
        let job = background_jobs::Job {
            id: json!("i"),
            description: case["d"].clone(),
            workflow: case["w"].as_bool().unwrap(),
            launched_at: None,
            command: case["c"].as_str().map(String::from),
            seen_at: None,
        };
        assert_eq!(job.watcher(), want.as_bool().unwrap(), "{case}");
    }
}
