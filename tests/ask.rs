//! `qm ask`: the same headless call as Ruby's `AskSearch`, with a fake `claude` that records the
//! arguments, environment and stdin it is given and answers with canned text. The transcript holds
//! what was sent (the prompt and the item index, byte for byte) and what was printed; generated from Ruby.

mod common;
use common::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn item(root: &Path, slug: &str, front: &str, description: &str, notes: &str) {
    fs::create_dir_all(root.join("work-items")).unwrap();
    fs::write(root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\n{front}---\n\n## Description\n\n{description}\n\n## Notes\n\n{notes}\n## Snapshots\n")).unwrap();
}

fn world(root: &Path) {
    let long = "A long description sentence that goes on.  ".repeat(30);
    let notes: String = (1..=14).map(|i| format!("- 2026-02-{i:02}: note number {i} with some words that take up room {}\n", "padding ".repeat(i * 2))).collect();
    item(root, "resume-broke", "title: Resume broke after worktree move\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\nbranches:\n- feat/resume\n- issue/42-resume-fix\n", &long, &notes);
    item(root, "plain", "title: Plain\nstate: fridge\nproject: other\ncreated: '2026-01-01'\n", "_TODO_", "");
    item(root, "unicode", "title: Ünïcode — title 🙂\nstate: resolved\nproject: demo\ncreated: '2026-01-01'\nbranches:\n- a/issue/x/issue/7\n", "multi\n\n  line\tdescription ünï", "- 2026-03-01 [open]: blocked on ünï review\n- 2026-03-02 [history]: mechanics\n");
    item(root, "no-state", "title: No state\nproject: demo\ncreated: '2026-01-01'\n", "", "");
    // the note budget: six notes clipped at 300 characters use 1,872 of 2,000; what follows must fit exactly
    let big = |day: u32| format!("- 2026-04-{day:02}: {}\n", "w".repeat(400));
    let six: String = (3..=8).map(big).collect();
    item(root, "budget-break", "title: Budget break\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\n", "d", &format!("- 2026-04-01: tiny and old\n{}{six}", big(2)));
    item(root, "budget-edge", "title: Budget edge\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\n", "d", &format!("- 2026-04-01: {}\n{six}", "e".repeat(117)));
    for i in 0..10 {
        item(root, &format!("filler-{i}"), &format!("title: Filler {i}\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\n"), "x", "");
    }
}

fn fake_claude(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    let script = "#!/bin/sh\n{ for a in \"$@\"; do printf '[%s]\\n' \"$a\"; done; printf 'QM_SKIP=%s\\n' \"$QM_SKIP\"; } > \"$FAKE_CLAUDE_DIR/args\"\ncat > \"$FAKE_CLAUDE_DIR/stdin\"\n[ -f \"$FAKE_CLAUDE_DIR/err\" ] && cat \"$FAKE_CLAUDE_DIR/err\" >&2\n[ -f \"$FAKE_CLAUDE_DIR/out\" ] && cat \"$FAKE_CLAUDE_DIR/out\"\n[ -f \"$FAKE_CLAUDE_DIR/fail\" ] && exit 3\nexit 0\n";
    fs::write(dir.join("claude"), script).unwrap();
    fs::set_permissions(dir.join("claude"), fs::Permissions::from_mode(0o755)).unwrap();
}

fn hits(list: serde_json::Value) -> String {
    serde_json::json!({"type": "result", "structured_output": {"hits": list}}).to_string()
}

#[test]
fn ask_sends_the_index_to_claude_and_prints_the_ranked_slugs() {
    let base = tempfile::tempdir().unwrap();
    let (root, bin, rec) = (base.path().join("root"), base.path().join("bin"), base.path().join("rec"));
    world(&root);
    fake_claude(&bin);
    fs::create_dir_all(&rec).unwrap();
    let qm = Qm::at(&root);
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let env = [("PATH", path.as_str()), ("FAKE_CLAUDE_DIR", rec.to_str().unwrap())];
    let many: Vec<serde_json::Value> = (0..10).map(|i| serde_json::json!({"slug": format!("filler-{i}"), "why": format!("reason {i}")})).collect();
    let scenarios: Vec<(&str, Option<String>, Option<&str>, bool, Vec<&str>)> = vec![
        ("ranked", Some(hits(serde_json::json!([{"slug": "unicode", "why": "mentions ünï"}, {"slug": "resume-broke", "why": "the one"}]))), None, false, vec!["ask", "resume", "broke", "after the move"]),
        ("invented and duplicate slugs", Some(hits(serde_json::json!([{"slug": "ghost", "why": "x"}, {"slug": "plain", "why": "first"}, {"slug": "plain", "why": "second"}, "junk", {"slug": 5, "why": "n"}, {"why": "no slug"}]))), None, false, vec!["ask", "  padded  query  "]),
        ("more than eight", Some(hits(serde_json::Value::Array(many))), None, false, vec!["ask", "everything"]),
        ("why in other shapes", Some(hits(serde_json::json!([{"slug": "plain"}, {"slug": "unicode", "why": 7}, {"slug": "no-state", "why": true}, {"slug": "filler-1", "why": null}, {"slug": "filler-2", "why": "multi\nline why\n"}]))), None, false, vec!["ask", "shapes"]),
        ("nothing matches", Some(hits(serde_json::json!([]))), None, false, vec!["ask", "nothing"]),
        ("no structured output", Some("{\"type\":\"result\",\"result\":\"hi\"}".into()), None, false, vec!["ask", "q"]),
        ("hits not a list", Some("{\"structured_output\":{\"hits\":\"nope\"}}".into()), None, false, vec!["ask", "q"]),
        ("unparseable", Some("this is not json at all ".repeat(20)), None, false, vec!["ask", "q"]),
        ("array answer", Some("[1,2]".into()), None, false, vec!["ask", "q"]),
        ("structured output a string", Some("{\"structured_output\":\"x\"}".into()), None, false, vec!["ask", "q"]),
        ("claude fails with stderr", Some("some stdout".into()), Some("boom on stderr\n"), true, vec!["ask", "q"]),
        ("claude fails quietly", Some("only stdout\n".into()), None, true, vec!["ask", "q"]),
        ("percent and quotes in the query", Some(hits(serde_json::json!([]))), None, false, vec!["ask", "100% \"sure\" %<limit>d #{x}"]),
        ("no query", None, None, false, vec!["ask"]),
        ("blank query", None, None, false, vec!["ask", "   "]),
    ];
    let mut out = String::new();
    for (name, answer, err, fail, args) in scenarios {
        for f in ["out", "err", "fail", "args", "stdin"] {
            let _ = fs::remove_file(rec.join(f));
        }
        if let Some(a) = answer {
            fs::write(rec.join("out"), a).unwrap();
        }
        if let Some(e) = err {
            fs::write(rec.join("err"), e).unwrap();
        }
        if fail {
            fs::write(rec.join("fail"), "").unwrap();
        }
        let o = qm.run_with(&args, "", &env);
        out.push_str(&format!("=== {name}: qm {args:?} → {}\n--- stdout\n{}--- stderr\n{}--- claude args\n{}--- claude stdin\n{}\n", o.code, o.stdout, o.stderr, fs::read_to_string(rec.join("args")).unwrap_or_else(|_| "(not run)\n".into()), fs::read_to_string(rec.join("stdin")).unwrap_or_else(|_| "(not run)".into())));
    }
    golden("ask.txt", &out.replace(base.path().to_str().unwrap(), "BASE"));
}

#[test]
fn without_claude_on_the_path_ruby_answers() {
    let base = tempfile::tempdir().unwrap();
    world(&base.path().join("root"));
    let qm = Qm::at(&base.path().join("root"));
    let o = qm.run_with(&["ask", "anything"], "", &[("PATH", "/usr/local/bin:/usr/bin:/bin")]);
    assert_eq!(o.code, 1);
    assert!(!o.stderr.is_empty());
}
