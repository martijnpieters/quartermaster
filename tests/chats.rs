//! The commands that ask the running GUI to act on a chat (`rotate`, `caffeinate`, `pane-start`,
//! `pane-resume`): a fake GUI on a loopback port records the request each binary sends and answers
//! with a canned reply; Ruby and Rust must send the same request and print the same thing.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

fn rust() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_qm"))
}

fn ruby() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("quartermaster")
}

fn world(root: &Path) {
    std::fs::create_dir_all(root.join("work-items")).unwrap();
    std::fs::write(root.join("work-items/chat-item.md"), "---\nv: 1\ntitle: Chat item\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\nsessions:\n- id: s-live\n---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n").unwrap();
    std::fs::write(root.join("work-items/other.md"), "---\nv: 1\ntitle: Other\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\n---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n").unwrap();
}

/// Starts the fake GUI; it answers one request with `reply` (raw HTTP body) and records it.
fn fake_gui(reply: Option<String>) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(vec![]));
    match reply {
        None => drop(listener),
        Some(body) => {
            let seen = seen.clone();
            std::thread::spawn(move || {
                let Ok((mut stream, _)) = listener.accept() else { return };
                let mut raw = vec![];
                let mut buf = [0u8; 4096];
                loop {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    raw.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&raw).into_owned();
                    if let Some((head, rest)) = text.split_once("\r\n\r\n") {
                        let len = head.to_ascii_lowercase().lines().find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                        if rest.len() >= len || n == 0 {
                            let keep: Vec<&str> = head.lines().filter(|l| l.starts_with("POST") || l.to_ascii_lowercase().starts_with("content-type") || l.to_ascii_lowercase().starts_with("host") || l.to_ascii_lowercase().starts_with("origin")).collect();
                            seen.lock().unwrap().push(format!("{}\nbody: {rest}", keep.join("\n")));
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
            });
        }
    }
    (port, seen)
}

fn attempt(bin: &Path, args: &[&str], reply: Option<&str>, env: &[(&str, &str)], live_pid: bool) -> String {
    attempt_with(bin, args, reply, env, if live_pid { 1 } else { 0 })
}

/// `pid_mode`: 0 a dead pid, 1 the test process (an ancestor of qm), 2 a live process that is not.
fn attempt_with(bin: &Path, args: &[&str], reply: Option<&str>, env: &[(&str, &str)], pid_mode: u8) -> String {
    let mut stranger = if pid_mode == 2 { Some(Command::new("sleep").arg("30").spawn().unwrap()) } else { None };
    let dir = tempfile::tempdir().unwrap();
    world(dir.path());
    let pid = match pid_mode {
        1 => std::process::id(),
        2 => stranger.as_ref().unwrap().id(),
        _ => 999_999,
    };
    std::fs::write(dir.path().join("sessions.jsonl"), format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"s-live\",\"cwd\":\"/srv\",\"pid\":{pid},\"ts\":\"2026-03-01T00:00:00+00:00\"}}\n")).unwrap();
    let (port, seen) = fake_gui(reply.map(String::from));
    let mut cmd = Command::new(bin);
    cmd.args(args).env("QM_ROOT", dir.path()).env("QM_GUI_PORT", port.to_string()).env("LANG", "C.UTF-8").env("LC_ALL", "C.UTF-8").env_remove("QM_ASIDE").env_remove("QM_ITEM").current_dir(dir.path()).stdin(Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    if let Some(c) = stranger.as_mut() {
        let _ = c.kill();
        let _ = c.wait();
    }
    let requests = seen.lock().unwrap().join("\n---\n");
    format!("exit {}\n--- stdout\n{}--- stderr\n{}--- request\n{requests}\n", o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).replace(&port.to_string(), "PORT")
}

fn same(args: &[&str], reply: Option<&str>, env: &[(&str, &str)], live_pid: bool) -> String {
    let ruby_text = attempt(&ruby(), args, reply, env, live_pid);
    let rust_text = attempt(rust(), args, reply, env, live_pid);
    assert_eq!(rust_text, ruby_text, "qm {args:?} differs");
    ruby_text
}

const OK: &str = "{\"ok\":true,\"mode\":\"wrap_up\"}";

#[test]
fn rotate_asks_the_gui_and_says_what_started() {
    let text = [
        same(&["rotate", "chat-item"], Some(OK), &[], false),
        same(&["rotate", "chat-item", "--close"], Some(OK), &[], false),
        same(&["rotate", "--close", "chat-item"], Some(OK), &[], false),
        same(&["rotate", "chat-item"], Some("{\"ok\":true,\"mode\":\"distill\"}"), &[], false),
        same(&["rotate", "--self"], Some(OK), &[], true),
        same(&["rotate", "--self", "--close"], Some(OK), &[], true),
        same(&["rotate", "--self", "ignored-slug"], Some(OK), &[], true),
        same(&["rotate", "--self"], Some(OK), &[("QM_ITEM", "other")], false),
        same(&["rotate", "--self"], Some(OK), &[("QM_ASIDE", "1")], true),
        same(&["rotate", "--self"], Some(OK), &[], false),
        same(&["rotate"], Some(OK), &[], false),
        same(&["rotate", "chat-item"], Some("{\"ok\":false,\"error\":\"the chat is mid-turn\"}"), &[], false),
        same(&["rotate", "chat-item"], Some("{\"ok\":false}"), &[], false),
        same(&["rotate", "chat-item"], Some("not json"), &[], false),
        same(&["rotate", "chat-item"], None, &[], false),
    ]
    .join("=====\n");
    for expected in ["\"close\":false", "\"self\":true,\"session\":\"s-live\"", "\"session\":null", "after this turn ends", "distilling the dead chat", "no qm gui running on 127.0.0.1:PORT", "did not answer like a qm gui", "the chat is mid-turn", "an aside can't rotate", "belongs to no item's chat", "usage: qm rotate SLUG", "Host: 127.0.0.1:PORT"] {
        assert!(text.contains(expected), "should show {expected:?}\n{text}");
    }
}

#[test]
fn a_live_session_that_is_not_an_ancestor_is_not_the_calling_chat() {
    let ruby_text = attempt_with(&ruby(), &["rotate", "--self"], Some(OK), &[], 2);
    let rust_text = attempt_with(rust(), &["rotate", "--self"], Some(OK), &[], 2);
    assert_eq!(rust_text, ruby_text);
    assert!(ruby_text.contains("belongs to no item's chat"), "{ruby_text}");
}

#[test]
fn caffeinate_and_the_pane_commands_report_the_guis_answer() {
    let on = "{\"ok\":true,\"caffeinate\":{\"mode\":\"on\",\"on\":true,\"pid\":4242}}";
    let auto = "{\"ok\":true,\"caffeinate\":{\"mode\":\"auto\",\"on\":false}}";
    let off = "{\"ok\":true,\"caffeinate\":{\"mode\":\"off\",\"on\":false}}";
    let text = [
        same(&["caffeinate", "on"], Some(on), &[], false),
        same(&["caffeinate", "auto"], Some(auto), &[], false),
        same(&["caffeinate", "off"], Some(off), &[], false),
        same(&["caffeinate"], Some(off), &[], false),
        same(&["caffeinate", "bogus"], Some(off), &[], false),
        same(&["caffeinate", "status"], None, &[], false),
        same(&["pane-start", "chat-item"], Some("{\"ok\":true}"), &[], false),
        same(&["pane-start", "chat-item"], Some("{\"ok\":true,\"attached\":true}"), &[], false),
        same(&["pane-start"], Some(OK), &[], false),
        same(&["pane-resume", "chat-item"], Some("{\"ok\":true,\"mode\":\"resume\"}"), &[], false),
        same(&["pane-resume", "chat-item", "--session", "s-9"], Some("{\"ok\":true,\"mode\":\"resume\"}"), &[], false),
        same(&["pane-resume"], Some(OK), &[], false),
        same(&["pane-start", "chat-item"], Some("{\"ok\":false,\"error\":\"no such item\"}"), &[], false),
    ]
    .join("=====\n");
    for expected in ["caffeinate on (pid 4242, until the GUI exits)", "caffeinate auto (not held", "caffeinate off", "usage: qm caffeinate on|off|auto|status", "fresh chat started in its qm gui pane", "already running in the qm gui", "resumed in its qm gui pane", "\"session\":\"s-9\"", "usage: qm pane-resume SLUG [--session ID]"] {
        assert!(text.contains(expected), "should show {expected:?}\n{text}");
    }
}

#[test]
fn shapes_ruby_reads_in_its_own_way_go_to_ruby() {
    for args in [vec!["rotate", "--clos", "chat-item"], vec!["rotate", "--bogus"], vec!["pane-start", "--x", "chat-item"]] {
        same(&args, Some(OK), &[], false);
    }
    // a port Ruby's to_i reads differently
    let a = attempt(&ruby(), &["caffeinate", "status"], Some(OK), &[("QM_GUI_PORT", "12x")], false);
    let b = attempt(rust(), &["caffeinate", "status"], Some(OK), &[("QM_GUI_PORT", "12x")], false);
    assert_eq!(a, b);
}
