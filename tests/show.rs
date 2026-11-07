//! `qm show` and `GhStack`: a world with a real git repo, gh-stack stacks, a fake `gh`, transcripts
//! and sessions. The golden is Ruby's output plus every side effect `show` has.

mod common;
use common::*;
use quartermaster::gh_stack;
use quartermaster::work_item::WorkItem;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static ENV: Mutex<()> = Mutex::new(());

fn commit(dir: &Path, file: &str, text: &str) {
    fs::write(dir.join(file), text).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", text]);
}

fn put(path: &Path, text: &str, mtime: SystemTime) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
    fs::OpenOptions::new().write(true).open(path).unwrap().set_modified(mtime).unwrap();
}

fn at(i: u64) -> SystemTime {
    UNIX_EPOCH + Duration::new(1_760_000_000 + i, 250_000_000)
}

fn assistant(id: &str, model: &str, input: u64, output: u64) -> String {
    format!("{{\"type\":\"assistant\",\"message\":{{\"id\":\"{id}\",\"model\":\"{model}\",\"usage\":{{\"input_tokens\":{input},\"output_tokens\":{output}}}}}}}\n")
}

fn show_world(base: &Path, now: SystemTime, pack_refs_last: bool) -> AttentionWorld {
    let w = attention_world(base, now, None);
    let repo = &w.repo;
    let origin = base.join("origin.git");
    fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "-q", "--bare"]);
    git(repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(repo, &["push", "-q", "origin", "main"]);

    // stack S2 first, then pack everything: its branches live only in packed-refs
    git(repo, &["checkout", "-q", "-b", "develop", "main"]);
    commit(repo, "dev.txt", "dev1");
    git(repo, &["checkout", "-q", "-b", "b1", "develop"]);
    commit(repo, "b1.txt", "b1");
    git(repo, &["checkout", "-q", "-b", "b2", "b1"]);
    commit(repo, "b2.txt", "b2");
    if !pack_refs_last {
        git(repo, &["pack-refs", "--all"]);
    }
    // stack S1: a2 needs a restack because a1 moved after it was built; a3 exists only on origin
    git(repo, &["checkout", "-q", "-b", "a1", "main"]);
    commit(repo, "a1.txt", "a1");
    git(repo, &["checkout", "-q", "-b", "a2", "a1"]);
    commit(repo, "a2.txt", "a2");
    git(repo, &["checkout", "-q", "-b", "a3", "a2"]);
    commit(repo, "a3.txt", "a3");
    git(repo, &["push", "-q", "origin", "a3"]);
    git(repo, &["checkout", "-q", "a1"]);
    commit(repo, "a1b.txt", "a1 moves");
    git(repo, &["branch", "-q", "-D", "a3"]);
    git(repo, &["checkout", "-q", "-b", "d1", "main"]);
    commit(repo, "d1.txt", "d1");
    git(repo, &["checkout", "-q", "-b", "feature-x", "main"]);
    commit(repo, "fx.txt", "fx");
    git(repo, &["checkout", "-q", "main"]);

    fs::write(
        repo.join(".git/gh-stack"),
        json!({"stacks": [
            {"trunk": {"branch": "main"}, "branches": [
                {"branch": "a1", "pullRequest": {"number": 1, "url": "https://example.test/pull/1"}},
                {"branch": "a2", "pullRequest": {"number": 2, "url": "https://example.test/pull/2"}},
                {"branch": "a3"}]},
            {"trunk": {"branch": "develop"}, "branches": [{"branch": "b1", "pullRequest": {"number": 11}}, {"branch": "b2"}]},
            {"branches": [{"branch": "d1"}]},
            {"branches": [{"branch": ""}, {"nobranch": 1}, "junk"]},
            "garbage"
        ]})
        .to_string(),
    )
    .unwrap();

    // config: the demo project gets a dev server
    fs::write(w.root.join("config.yml"), format!("projects:\n  demo:\n    root: {}\n    dev_server:\n      command: devsrv\n      modes: [dev, tunnel]\n      default_mode: dev\n", repo.display())).unwrap();
    for (branch, state, n) in [("a1", "MERGED", 21), ("a2", "OPEN", 22), ("b1", "OPEN", 31), ("b2", "MERGED", 32), ("d1", "OPEN", 41)] {
        put(&w.gh_data.join(branch), &format!("{{\"number\":{n},\"state\":\"{state}\",\"url\":\"https://example.test/pull/{n}\"}}"), now);
    }

    // sessions and usage for the usage item
    let mut log = fs::read_to_string(w.root.join("sessions.jsonl")).unwrap();
    let me = std::process::id();
    let transcript = |name: &str, text: &str, age: u64| {
        let p = w.transcripts.join(format!("p/{name}.jsonl"));
        put(&p, text, at(age));
        p
    };
    let main_t = transcript("us-main", &(assistant("m1", "claude-opus-5-5", 1000, 500) + &assistant("m2", "claude-sonnet-5-5", 2_000_000, 40_000)), 1);
    put(&w.transcripts.join("p/us-main/subagents/a.jsonl"), &assistant("s1", "claude-haiku-5-5", 300, 30), at(2));
    put(&w.transcripts.join("p/us-main/subagents/b.jsonl"), &assistant("s2", "claude-haiku-5-5", 400, 40), at(3));
    let live_t = transcript("us-live", &assistant("l1", "claude-opus-5-5", 5, 5), 4);
    let custom_t = transcript("us-custom", &assistant("c1", "mystery-model", 1000, 1000), 5);
    let aside_t = transcript("us-aside", &assistant("as1", "claude-sonnet-5-5", 10, 10), 6);
    let start = |id: &str, ts: &str, pid: u32, path: &Path, extra: &str| {
        format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"/srv/app\",\"pid\":{pid},\"transcript_path\":\"{}\",{extra}\"ts\":\"{ts}\"}}\n", path.display())
    };
    log += &start("us-main", "2026-04-01T10:00:00+00:00", 999999, &main_t, "");
    log += "{\"v\":1,\"event\":\"end\",\"session_id\":\"us-main\",\"reason\":\"exit\",\"duration_s\":60,\"ts\":\"2026-04-01T10:01:00+00:00\"}\n";
    log += &start("us-live", "2026-04-02T10:00:00+00:00", me, &live_t, "");
    log += &start("us-custom", "2026-04-03T10:00:00+00:00", 999999, &custom_t, "");
    log += &start("us-aside", "2026-04-04T10:00:00+00:00", 999999, &aside_t, "\"aside\":\"usage-item\",");
    log += &start("us-expired", "2026-04-05T10:00:00+00:00", 999999, &w.transcripts.join("p/gone.jsonl"), "");
    fs::write(w.root.join("sessions.jsonl"), log).unwrap();

    let item = |slug: &str, extra: &str, sessions: &[&str], notes: &str, age: f64| {
        // acknowledged sessions: their transcripts carry fixed mtimes, and a warning would then depend on today's date
        let list = if sessions.is_empty() { String::new() } else { format!("sessions:\n{}", sessions.iter().map(|s| format!("- id: {s}\n  ack_degrade: true\n")).collect::<String>()) };
        let path = w.root.join(format!("work-items/{slug}.md"));
        put(&path, &format!("---\nv: 1\ntitle: Title of {slug}\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\n{extra}{list}---\n\n## Description\n\nd\n\n## Notes\n{notes}\n## Snapshots\n"), now - Duration::from_secs_f64(age * 86_400.0));
    };
    item("stacked-item", "branches:\n- a2\n", &[], "- 2026-02-01: plain decision\n", 0.1);
    item("two-stacks", "branches:\n- b2\n- a1\n", &[], "", 0.1);
    item("loose-item", "branches:\n- feature-x\n- loose-gone\n- main\n", &[], "- 2026-02-01 [history]: only history\n", 0.1);
    item("d-item", "branches:\n- d1\n", &[], "", 0.1);
    item("issue-item", "branches:\n- issue/123-thing\n- other/issue/45x\npf_mode: tunnel\nsnooze_until: '2099-05-05'\n", &[], "- 2026-02-01 [open]: blocked on review\n- 2026-02-02: later decision\n- 2026-02-03 [history]: mechanics\n- 2026-02-04 [weird]: odd marker\n", 0.1);
    item("usage-item", "", &["us-live", "us-main", "us-custom", "us-expired", "us-uncaptured-id"], "- 2026-02-01 [open]: first open\n- 2026-02-02 [open]: second open\n", 0.1);
    item("bare", "", &[], "", 0.1);
    w
}

fn run_show(w: &AttentionWorld, now_log: &Path) -> String {
    let qm = Qm::at(&w.root);
    let home = w.home.display().to_string();
    let transcripts = w.transcripts.display().to_string();
    let path = format!("{}:{}", w.bin.display(), std::env::var("PATH").unwrap_or_default());
    let (data, log) = (w.gh_data.display().to_string(), now_log.display().to_string());
    let env = [("HOME", home.as_str()), ("QM_TRANSCRIPTS_DIR", transcripts.as_str()), ("PATH", path.as_str()), ("FAKE_GH_DATA", data.as_str()), ("FAKE_GH_LOG", log.as_str())];
    let slugs = [
        "stacked-item", "two-stacks", "loose-item", "d-item", "issue-item", "usage-item", "bare", "degrade", "stale", "snoozed-past", "quiet-sessioned", "pr-one-merged", "pr-stack-merged",
        "pr-merged-plus-gone", "pr-cached", "pr-cached-stale-open", "pr-no-project", "pr-slash-branch", "resolved-old", "ghost",
    ];
    let mut out = String::new();
    for slug in slugs {
        let o = qm.run_with(&["show", slug], "", &env);
        out.push_str(&format!("$ qm show {slug}\nexit {}\n--- stdout\n{}--- stderr\n{}\n", o.code, o.stdout, o.stderr));
    }
    let o = qm.run_with(&["show"], "", &env);
    out.push_str(&format!("$ qm show\nexit {}\n{}", o.code, o.stderr));
    // what the shows left behind
    for slug in ["stacked-item", "two-stacks", "loose-item", "usage-item"] {
        out.push_str(&format!("=== work-items/{slug}.md\n{}", fs::read_to_string(w.root.join(format!("work-items/{slug}.md"))).unwrap().split("\n---\n").next().unwrap()));
        out.push('\n');
    }
    let cache = fs::read_to_string(w.root.join("pr-cache.json")).unwrap_or_default();
    out.push_str(&format!("=== pr-cache.json\n{}\n", regex::Regex::new(r#""ts":\d+"#).unwrap().replace_all(&cache, "\"ts\":T")));
    // mtime floats are covered byte for byte in tests/usage.rs; here the attention world's are relative to now
    let ledger = fs::read_to_string(w.root.join("usage-cache.json")).unwrap_or_default();
    out.push_str(&format!("=== usage-cache.json\n{}\n", regex::Regex::new(r"\b1\d{9}\.\d+\b").unwrap().replace_all(&ledger, "MTIME")));
    out.push_str(&format!("=== gh calls\n{}", fs::read_to_string(now_log).unwrap_or_default()));
    out.replace(&w.root.parent().unwrap().display().to_string(), "BASE")
}

#[test]
fn show_prints_everything_and_leaves_the_same_traces() {
    let base = tempfile::tempdir().unwrap();
    let w = show_world(base.path(), SystemTime::now(), false);
    let text = run_show(&w, &base.path().join("gh.log"));
    golden("show_all.txt", &normalize(&text));
}

#[test]
fn show_is_the_same_with_loose_refs_instead_of_packed_ones() {
    let base = tempfile::tempdir().unwrap();
    let w = show_world(base.path(), SystemTime::now(), true);
    let text = run_show(&w, &base.path().join("gh.log"));
    golden("show_all_loose_refs.txt", &normalize(&text));
}

// ------------------------------------------------------------------ chains, as a library

fn ruby_chains(w: &AttentionWorld, slug: &str, states: &Value, links: &Value) -> Value {
    let script = r#"require File.join(ARGV[0], "lib", "quartermaster")
include Quartermaster
args = JSON.parse(ARGV[2])
puts JSON.generate(GhStack.chains(WorkItem.find(ARGV[1]), pr_states: args["states"], pr_links: args["links"]))"#;
    let out = Command::new("ruby")
        .args(["-e", script, env!("CARGO_MANIFEST_DIR"), slug, &json!({"states": states, "links": links}).to_string()])
        .env_clear()
        .env("QM_ROOT", &w.root)
        .env("HOME", &w.home)
        .env("PATH", std::env::var("PATH").unwrap())
        .env("LANG", "C.UTF-8")
        .env("QM_TMUX_SOCKET", "qm-test-show")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn chains_match_ruby_with_pr_states_and_links_merged_in() {
    let base = tempfile::tempdir().unwrap();
    let w = show_world(base.path(), SystemTime::now(), false);
    let states = json!({"a1": "MERGED", "a2": "OPEN", "b2": "MERGED", "feature-x": "OPEN", "loose-gone": "CLOSED"});
    let links = json!({"a3": {"number": 33, "url": "https://example.test/pull/33"}, "b2": {"number": 12, "url": "u12"}, "feature-x": {"number": 50, "url": "u50"}, "a1": {"number": 99, "url": "ignored"}});
    let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("QM_ROOT", &w.root);
    std::env::set_var("HOME", &w.home);
    for slug in ["stacked-item", "two-stacks", "loose-item", "d-item", "bare", "pr-no-project"] {
        for (s, l) in [(json!({}), json!({})), (states.clone(), links.clone())] {
            let want = ruby_chains(&w, slug, &s, &l);
            let item = WorkItem::find(slug).unwrap();
            let got = Value::Array(gh_stack::chains(&item, s.as_object().unwrap(), l.as_object().unwrap()));
            assert_eq!(got, want, "{slug} with states {s}");
        }
    }
    let stacked = Value::Array(gh_stack::chains(&WorkItem::find("stacked-item").unwrap(), &Map::new(), &Map::new()));
    assert_eq!(stacked[0]["layers"][1]["needsRestack"], true, "a2 must need a restack: {stacked}");
    assert_eq!(stacked[0]["layers"][2]["ahead"], 1, "a3 is only on origin: {stacked}");
}
