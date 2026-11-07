//! `Attention` and `PrStatus`: Ruby and the Rust library run over identical worlds and must agree
//! on the entries, on the `pr-cache.json` they leave behind and on every `gh` call they make.

mod common;
use common::*;
use quartermaster::attention::Attention;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::time::SystemTime;

static ENV: Mutex<()> = Mutex::new(());

struct Run {
    entries: Value,
    cache: Value,
    gh_log: String,
}

fn env_for(w: &AttentionWorld, log: &Path) -> Vec<(&'static str, String)> {
    let path = format!("{}:{}", w.bin.display(), std::env::var("PATH").unwrap_or_default());
    vec![
        ("QM_ROOT", w.root.display().to_string()),
        ("HOME", w.home.display().to_string()),
        ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()),
        ("PATH", path),
        ("FAKE_GH_DATA", w.gh_data.display().to_string()),
        ("FAKE_GH_LOG", log.display().to_string()),
        ("LANG", "C.UTF-8".into()),
        ("LC_ALL", "C.UTF-8".into()),
    ]
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap_or_else(|_| "null".into())).unwrap_or(Value::Null)
}

fn ruby_run(base: &Path, retention: Option<u32>, now: SystemTime) -> Run {
    let w = attention_world(base, now, retention);
    let log = base.join("gh.log");
    let script = r#"require File.join(ARGV[0], "lib", "quartermaster")
puts JSON.generate(Quartermaster::Attention.new.entries.map { |e| { "slug" => e.slug, "kind" => e.kind, "message" => e.message, "session_id" => e.session_id } })"#;
    let mut cmd = Command::new("ruby");
    cmd.args(["-e", script, env!("CARGO_MANIFEST_DIR")]).env_clear().env("QM_TMUX_SOCKET", "qm-test-attention");
    for (k, v) in env_for(&w, &log) {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "ruby failed: {}", String::from_utf8_lossy(&out.stderr));
    Run {
        entries: serde_json::from_slice(&out.stdout).unwrap(),
        cache: read_json(&w.root.join("pr-cache.json")),
        gh_log: std::fs::read_to_string(&log).unwrap_or_default(),
    }
}

fn rust_run(base: &Path, retention: Option<u32>, now: SystemTime) -> Run {
    let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let w = attention_world(base, now, retention);
    let log = base.join("gh.log");
    for (k, v) in env_for(&w, &log) {
        std::env::set_var(k, v);
    }
    let entries: Vec<Value> = Attention::new(SystemTime::now())
        .entries()
        .unwrap()
        .iter()
        .map(|e| json!({"slug": e.slug, "kind": e.kind, "message": e.message, "session_id": e.session_id}))
        .collect();
    Run { entries: Value::Array(entries), cache: read_json(&w.root.join("pr-cache.json")), gh_log: std::fs::read_to_string(&log).unwrap_or_default() }
}

fn assert_same_cache(rust: &Value, ruby: &Value) {
    let (a, b) = (rust.as_object().unwrap(), ruby.as_object().unwrap());
    assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>(), "cache keys or their order differ");
    for (k, v) in a {
        let (mut x, mut y) = (v.clone(), b[k].clone());
        let (tx, ty) = (x["ts"].as_i64().unwrap(), y["ts"].as_i64().unwrap());
        assert!((tx - ty).abs() <= 5, "{k}: ts {tx} vs {ty}");
        x.as_object_mut().unwrap().remove("ts");
        y.as_object_mut().unwrap().remove("ts");
        assert_eq!(x, y, "{k}");
        assert_eq!(v.as_object().unwrap().keys().collect::<Vec<_>>(), b[k].as_object().unwrap().keys().collect::<Vec<_>>(), "{k}: key order");
    }
}

fn compare(retention: Option<u32>) -> Value {
    let now = SystemTime::now();
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let ruby = ruby_run(a.path(), retention, now);
    let rust = rust_run(b.path(), retention, now);
    assert_eq!(rust.entries, ruby.entries, "entries differ");
    assert_same_cache(&rust.cache, &ruby.cache);
    assert_eq!(rust.gh_log, ruby.gh_log, "gh calls differ");
    assert!(ruby.gh_log.lines().count() >= 10, "the world should make the fake gh work: {:?}", ruby.gh_log);
    ruby.entries
}

#[test]
fn rust_and_ruby_agree_on_every_attention_rule() {
    let entries = compare(None);
    let slugs_kinds: Vec<(String, String)> =
        entries.as_array().unwrap().iter().map(|e| (e["slug"].as_str().unwrap().into(), e["kind"].as_str().unwrap().into())).collect();
    for expected in [
        ("degrade", "degrade"), ("stale", "stale"), ("stale-ten-days", "stale"), ("snoozed-past", "stale"), ("snoozed-garbage", "stale"),
        ("pr-one-merged", "pr_merged"), ("pr-stack-merged", "pr_merged"), ("pr-merged-plus-gone", "pr_merged"), ("pr-cached", "pr_merged"),
        ("pr-cached-stale-open", "pr_merged"), ("pr-slash-branch", "pr_merged"),
    ] {
        assert!(slugs_kinds.contains(&(expected.0.into(), expected.1.into())), "the world should raise {expected:?}: {slugs_kinds:?}");
    }
    for absent in ["quiet-sessioned", "snoozed-future", "fridge-old", "resolved-old", "pr-stack-mixed", "pr-none-gone", "pr-none-exists", "pr-no-project", "pr-resolved"] {
        assert!(!slugs_kinds.iter().any(|(s, _)| s == absent), "{absent} must stay quiet");
    }
}

#[test]
fn rust_and_ruby_agree_under_a_shorter_retention() {
    compare(Some(10));
}
