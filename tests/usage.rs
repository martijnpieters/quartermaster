//! The usage ledger and everything that feeds it: Ruby and the Rust library run over identical
//! worlds, pass after pass, and must agree on every row, every line and the ledger file's bytes.

mod common;
use quartermaster::item_usage::{dollars, human_tokens, ItemUsage};
use quartermaster::price_table::PriceTable;
use quartermaster::ruby_format::format_f;
use quartermaster::ruby_num::Num;
use quartermaster::session_log::SessionLog;
use quartermaster::transcript_usage::mtime_f64;
use quartermaster::usage_ledger::UsageLedger;
use quartermaster::work_item::WorkItem;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static ENV: Mutex<()> = Mutex::new(());

fn at(i: u64) -> SystemTime {
    UNIX_EPOCH + Duration::new(1_760_000_000 + i, 123_456_789 + (i as u32) * 7)
}

fn put(path: &Path, text: &str, mtime: SystemTime) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
    fs::OpenOptions::new().write(true).open(path).unwrap().set_modified(mtime).unwrap();
}

fn append(path: &Path, text: &str, mtime: SystemTime) {
    use std::io::Write;
    fs::OpenOptions::new().append(true).open(path).unwrap().write_all(text.as_bytes()).unwrap();
    fs::OpenOptions::new().write(true).open(path).unwrap().set_modified(mtime).unwrap();
}

fn assistant(id: &str, model: &str, usage: &str) -> String {
    format!("{{\"type\":\"assistant\",\"message\":{{\"id\":\"{id}\",\"model\":\"{model}\",\"usage\":{usage}}}}}\n")
}

struct World {
    base: PathBuf,
    root: PathBuf,
    transcripts: PathBuf,
}

impl World {
    fn t(&self, name: &str) -> PathBuf {
        self.transcripts.join(format!("p/{name}.jsonl"))
    }
}

fn basic_transcript() -> String {
    let mut t = String::new();
    t += "not json at all\n\n5\n\"a string\"\n";
    t += &assistant("m1", "claude-opus-5-5", r#"{"input_tokens":10,"output_tokens":5}"#);
    t += &assistant("m1", "claude-opus-5-5", r#"{"input_tokens":10,"output_tokens":50}"#);
    t += &assistant("m1", "claude-opus-5-5", r#"{"input_tokens":10,"output_tokens":500}"#);
    t += &assistant("m2", "claude-sonnet-4-5-20250929", r#"{"input_tokens":1,"cache_creation_input_tokens":3000,"cache_creation":{"ephemeral_1h_input_tokens":1000,"ephemeral_5m_input_tokens":500},"cache_read_input_tokens":7,"output_tokens":9}"#);
    t += &assistant("m3", "claude-opus-5-5", r#"{"input_tokens":100,"output_tokens":200,"speed":"fast"}"#);
    t += &assistant("m4", "claude-haiku-5-5", r#"{"input_tokens":90000,"cache_read_input_tokens":20000,"output_tokens":4}"#);
    t += &assistant("m5", "<synthetic>", r#"{"input_tokens":999,"output_tokens":999}"#);
    t += "{\"type\":\"assistant\",\"message\":{\"id\":\"m6\",\"model\":\"claude-opus-5-5\"}}\n";
    t += &assistant("m7", "claude-opus-5-5", r#"{"input_tokens":0,"output_tokens":0}"#);
    t += &assistant("m9", "claude-opus-4-6[1m]", r#"{"input_tokens":4,"output_tokens":4}"#);
    t += &assistant("m10", "my-custom-model", r#"{"input_tokens":1000,"output_tokens":2000}"#);
    t += "{\"type\":\"assistant\",\"message\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":3}},\"requestId\":\"req-1\"}\n";
    t += "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-opus-5-5\",\"usage\":{\"input_tokens\":4,\"output_tokens\":4}}}\n";
    t += "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-opus-5-5\",\"usage\":{\"input_tokens\":5,\"output_tokens\":5}}}\n";
    t += &assistant("m11", "claude-opus-5-5", r#"{"input_tokens":"12","output_tokens":3.7,"cache_read_input_tokens":null}"#);
    t += "{\"type\":\"user\",\"message\":{\"content\":\"hi\"}}\n";
    t += "{\"type\":\"assistant\",\"message\":\"not a hash\"}\n";
    t += "{\"type\":\"assistant\",\"message\":{\"id\":\"m12\",\"model\":\"claude-opus-5-5\",\"usage\":{\"input_tokens\":8,\"output_tokens\":8}}"; // no newline: not read yet
    t
}

fn world(base: &Path) -> World {
    let w = World { base: base.to_path_buf(), root: base.join("root"), transcripts: base.join("transcripts") };
    let me = std::process::id();
    let usage = |i: u64| format!(r#"{{"input_tokens":{},"output_tokens":{}}}"#, 100 * i, 10 * i);

    put(&w.t("u-basic"), &basic_transcript(), at(1));
    put(&w.t("u-sub"), &assistant("s1", "claude-sonnet-5-5", &usage(2)), at(2));
    let sub = w.transcripts.join("p/u-sub/subagents");
    put(&sub.join("a.jsonl"), &(assistant("a1", "claude-haiku-5-5", &usage(3)) + &assistant("a2", "claude-haiku-5-5", &usage(4))), at(3));
    put(&sub.join("nested/deeper/b.jsonl"), &assistant("b1", "claude-opus-4-8", &usage(5)), at(4));
    put(&sub.join(".hidden/c.jsonl"), &assistant("c1", "claude-opus-4-8", &usage(6)), at(5));
    put(&sub.join("d.txt"), &assistant("d1", "claude-opus-4-8", &usage(7)), at(6));
    put(&sub.join(".dot.jsonl"), &assistant("e1", "claude-opus-4-8", &usage(8)), at(7));
    std::os::unix::fs::symlink(sub.join("a.jsonl"), sub.join("link.jsonl")).unwrap();
    put(&w.transcripts.join("elsewhere/real/z.jsonl"), &assistant("z1", "claude-opus-4-8", &usage(9)), at(8));
    std::os::unix::fs::symlink(w.transcripts.join("elsewhere/real"), sub.join("linkdir")).unwrap();
    put(&w.t("u-live"), &assistant("l1", "claude-opus-5", &usage(10)), at(9));
    put(&w.t("u-uncaptured"), &assistant("n1", "claude-sonnet-5", &usage(11)), at(10));
    put(&w.t("u-expired-tallied"), &assistant("e1", "claude-opus-5-5", &usage(12)), at(11));
    put(&w.t("a-1"), &assistant("as1", "claude-sonnet-5-5", &usage(13)), at(12));
    put(&w.t("u-priced"), &assistant("p1", "claude-sonnet-5-5", &usage(14)), at(13));
    put(&w.t("u-custom"), &assistant("c1", "mystery-model-9", &usage(15)), at(14));
    put(&w.t("u-dup"), &assistant("d1", "claude-opus-5-5", &usage(16)), at(15));

    let mut log = String::new();
    let mut start = |id: &str, n: u64, pid: u32, extra: &str| {
        log += &format!(
            "{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"/srv/app\",\"pid\":{pid},\"transcript_path\":\"{}\",{extra}\"ts\":\"2026-03-{n:02}T00:00:00+00:00\"}}\n",
            w.t(id).display()
        );
    };
    start("u-basic", 1, 999999, "");
    start("u-sub", 2, 999999, "");
    start("u-live", 3, me, "");
    start("u-expired-tallied", 4, 999999, "");
    start("a-1", 5, 999999, "\"aside\":\"basic\",");
    start("a-2", 6, 999999, "\"aside\":\"basic\",");
    start("x-expired-1", 7, 999999, "");
    start("x-expired-2", 8, 999999, "");
    start("x-expired-3", 9, 999999, "");
    start("u-priced", 10, 999999, "");
    start("u-custom", 11, 999999, "");
    start("u-dup", 12, 999999, "");
    fs::create_dir_all(&w.root).unwrap();
    fs::write(w.root.join("sessions.jsonl"), log).unwrap();

    fs::write(
        w.root.join("config.yml"),
        "prices:\n  claude-opus-5-5: { input: 4.5, output: 21 }\n  my-custom-model: { input: 1, cache_write_5m: 1, cache_write_1h: 1, cache_read: 1, output: 2.5 }\n  half-priced: { input: 1 }\n  claude-sonnet-5-5: { cache_read: 'x', input: 3 }\n  mystery-model-9: not a hash\n",
    )
    .unwrap();
    let item = |slug: &str, sessions: &[&str]| {
        let list = if sessions.is_empty() { String::new() } else { format!("sessions:\n{}", sessions.iter().map(|s| format!("- id: {s}\n")).collect::<String>()) };
        put(&w.root.join(format!("work-items/{slug}.md")), &format!("---\nv: 1\ntitle: {slug}\nstate: in_progress\nproject: No project\ncreated: '2026-01-01'\n{list}---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n"), at(20));
    };
    item("basic", &["u-basic", "u-sub", "u-live", "u-uncaptured", "u-expired-tallied"]);
    item("unknown-only", &["x-expired-1", "x-expired-2"]);
    item("lower", &["u-priced", "x-expired-3"]);
    item("partial", &["u-custom"]);
    item("dups", &["u-dup", "u-dup"]);
    item("empty", &[]);
    w
}

/// The same changes in both worlds, with explicit mtimes.
fn mutate(w: &World, pass: u64) {
    match pass {
        1 => {
            // complete the partial last line, then add a message
            append(&w.t("u-basic"), "}\n", at(30));
            append(&w.t("u-basic"), &assistant("m13", "claude-opus-5-5", r#"{"input_tokens":77,"output_tokens":7}"#), at(31));
            // shrink a sub-agent file, add another
            let sub = w.transcripts.join("p/u-sub/subagents");
            put(&sub.join("a.jsonl"), &assistant("a1", "claude-haiku-5-5", r#"{"input_tokens":1,"output_tokens":1}"#), at(32));
            put(&sub.join("new.jsonl"), &assistant("w1", "claude-opus-4-8", r#"{"input_tokens":2,"output_tokens":2}"#), at(33));
            // same size, newer mtime
            let live = w.t("u-live");
            let text = fs::read_to_string(&live).unwrap();
            put(&live, &text, at(34));
            // grow the live one
            append(&w.t("u-live"), &assistant("l2", "claude-opus-5", r#"{"input_tokens":5,"output_tokens":5}"#), at(35));
            // the tallied session's transcript is pruned
            fs::remove_file(w.t("u-expired-tallied")).unwrap();
        }
        2 => {
            // a record from another qm process, and top-level keys the writer drops
            let path = w.root.join("usage-cache.json");
            let mut data: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            data["sessions"]["other-process"] = json!({"main": ["/gone", 1, 1.5], "files": {"/gone": {"size": 1, "mtime": 1.5, "offset": 1, "buckets": {"claude-opus-5-5": {"input": 1, "cache_write_5m": 0, "cache_write_1h": 0, "cache_read": 0, "output": 1}}, "pending": null}}});
            data["extra"] = json!(1);
            fs::write(&path, data.to_string()).unwrap();
            append(&w.t("u-priced"), &assistant("p2", "claude-sonnet-5-5", r#"{"input_tokens":1,"output_tokens":1}"#), at(40));
        }
        _ => {}
    }
}

struct Pass {
    items: Value,
    ledger: String,
}

fn rust_pass(w: &World) -> Pass {
    let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("QM_ROOT", &w.root);
    std::env::set_var("QM_TRANSCRIPTS_DIR", &w.transcripts);
    std::env::set_var("HOME", w.base.join("home"));
    let (log, prices, mut ledger) = (SessionLog::read(), PriceTable::load(), UsageLedger::new());
    let mut out = serde_json::Map::new();
    for item in WorkItem::all().unwrap() {
        let u = ItemUsage::for_item(&item, &log, &mut ledger, &prices).unwrap();
        let rows: Vec<Value> = u
            .rows
            .iter()
            .map(|r| {
                json!({"id": r.id, "aside": r.aside, "when": r.when, "known": r.known, "agents": r.agents, "tokens": r.tokens, "cost": r.cost.json(),
                       "unpriced": r.unpriced, "models": r.models.iter().map(|(k, v)| (k.clone(), Value::from(*v))).collect::<serde_json::Map<String, Value>>()})
            })
            .collect();
        out.insert(item.slug.clone(), json!({"rows": rows, "line": u.line(), "tokens": u.tokens(), "cost": u.cost().json(), "lower_bound": u.lower_bound(), "partial": u.partial(), "unpriced": u.unpriced()}));
    }
    ledger.save();
    Pass { items: Value::Object(out), ledger: fs::read_to_string(w.root.join("usage-cache.json")).unwrap_or_default().replace(w.base.to_str().unwrap(), "BASE") }
}

fn ruby_pass(w: &World) -> Pass {
    let script = r#"require File.join(ARGV[0], "lib", "quartermaster")
include Quartermaster
ledger = UsageLedger.new; log = SessionLog.new; prices = PriceTable.load; out = {}
WorkItem.all.each do |item|
  u = ItemUsage.for(item, session_log: log, ledger: ledger, prices: prices)
  out[item.slug] = { "rows" => u.rows, "line" => u.line, "tokens" => u.tokens, "cost" => u.cost, "lower_bound" => u.lower_bound?, "partial" => u.partial?, "unpriced" => u.unpriced }
end
ledger.save
puts JSON.generate(out)"#;
    let out = Command::new("ruby")
        .args(["-e", script, env!("CARGO_MANIFEST_DIR")])
        .env_clear()
        .env("QM_ROOT", &w.root)
        .env("QM_TRANSCRIPTS_DIR", &w.transcripts)
        .env("HOME", w.base.join("home"))
        .env("PATH", std::env::var("PATH").unwrap())
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .env("QM_TMUX_SOCKET", "qm-test-usage")
        .output()
        .unwrap();
    assert!(out.status.success(), "ruby failed: {}", String::from_utf8_lossy(&out.stderr));
    Pass { items: serde_json::from_slice(&out.stdout).unwrap(), ledger: fs::read_to_string(w.root.join("usage-cache.json")).unwrap_or_default().replace(w.base.to_str().unwrap(), "BASE") }
}

#[test]
fn rust_and_ruby_keep_the_same_ledger_pass_after_pass() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (ruby, rust) = (world(a.path()), world(b.path()));
    let mut first_ledger: Option<Value> = None;
    for pass in 0..3 {
        let (r, x) = (ruby_pass(&ruby), rust_pass(&rust));
        assert_eq!(x.items, r.items, "pass {pass}: rows differ");
        assert_eq!(x.ledger, r.ledger, "pass {pass}: the ledger file differs");
        if pass == 0 {
            first_ledger = Some(serde_json::from_str(&r.ledger).unwrap());
            let basic = &r.items["basic"];
            assert!(basic["line"].as_str().unwrap().contains("tok"), "{basic}");
            assert_eq!(r.items["unknown-only"]["line"], "usage unknown · sessions expired");
            assert!(r.items["lower"]["line"].as_str().unwrap().starts_with("≥ "));
            assert_eq!(r.items["partial"]["partial"], true);
        }
        mutate(&ruby, pass + 1);
        mutate(&rust, pass + 1);
    }
    // a record: the pruned transcript's tally is still there, unchanged, after later passes
    let last: Value = serde_json::from_str(&fs::read_to_string(rust.root.join("usage-cache.json")).unwrap().replace(rust.base.to_str().unwrap(), "BASE")).unwrap();
    assert_eq!(last["sessions"]["u-expired-tallied"], first_ledger.unwrap()["sessions"]["u-expired-tallied"]);
    assert!(last["sessions"].get("other-process").is_some(), "another process's record must survive a save");
    assert!(last.get("extra").is_none() && last["v"] == 1);
}

// ----------------------------------------------------------------------------- fuzz and floats

fn ruby_json(script: &str, input: &Value) -> Value {
    use std::io::Write;
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

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[test]
fn costs_sum_like_rubys_kahan_babuska_over_mixed_rates() {
    let mut rng = Lcg(42);
    let models = ["claude-opus-5-5", "claude-opus-5-5:fast", "claude-haiku-5-5:long", "claude-fable-5-1", "claude-sonnet-4-5", "claude-3-5-haiku", "claude-opus-4-1", "nope", "claude-opus-5-5:fast:long"];
    let cases: Vec<Value> = (0..400)
        .map(|_| {
            let mut buckets = serde_json::Map::new();
            for _ in 0..1 + rng.below(6) {
                let model = models[rng.below(models.len() as u64) as usize];
                let scale = [1, 10, 1000, 1_000_000, 300_000_000][rng.below(5) as usize];
                buckets.insert(model.into(), json!({"input": rng.below(scale), "cache_write_5m": rng.below(scale), "cache_write_1h": rng.below(scale), "cache_read": rng.below(scale), "output": rng.below(scale)}));
            }
            Value::Object(buckets)
        })
        .collect();
    let want = ruby_json("->(cases) { prices = PriceTable.new({}); cases.map { |b| c, u = prices.cost(b); [c, u] } }", &Value::Array(cases.clone()));
    let prices = PriceTable::new(&[]);
    for (case, want) in cases.iter().zip(want.as_array().unwrap()) {
        let (cost, unpriced) = prices.cost(case.as_object().unwrap());
        assert_eq!(json!([cost.json(), unpriced]), *want, "{case}");
    }
}

#[test]
fn token_and_dollar_labels_round_like_ruby() {
    let mut counts: Vec<i64> = vec![0, 1, 999, 1000, 1499, 1500, 1501, 2500, 3500, 999_499, 999_500, 999_949_999, 999_950_000, 1_250_000, 2_500_000, 1_250_000_000, 123_456_789_012];
    let mut rng = Lcg(7);
    counts.extend((0..300).map(|_| rng.below(5_000_000_000) as i64));
    let mut amounts: Vec<Value> = vec![json!(0), json!(0.125), json!(0.375), json!(1.005), json!(9.995), json!(9.994999), json!(10), json!(10.5), json!(11.5), json!(99.5), json!(1234.5), json!(0.004), json!(0.005)];
    amounts.extend((0..300).map(|_| json!((rng.below(2_000_000) as f64) / 997.0)));
    let want = ruby_json("->(i) { [i['counts'].map { |c| ItemUsage.human_tokens(c) }, i['amounts'].map { |a| ItemUsage.dollars(a) }] }", &json!({"counts": counts, "amounts": amounts}));
    for (c, w) in counts.iter().zip(want[0].as_array().unwrap()) {
        assert_eq!(human_tokens(*c), w.as_str().unwrap(), "{c}");
    }
    for (a, w) in amounts.iter().zip(want[1].as_array().unwrap()) {
        let num = match a.as_i64() {
            Some(i) => Num::Int(i as i128),
            None => Num::Float(a.as_f64().unwrap()),
        };
        assert_eq!(dollars(num), w.as_str().unwrap(), "{a}");
    }
}

#[test]
fn mtimes_become_the_same_floats_as_rubys_to_f() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = Lcg(99);
    let mut paths = vec![];
    for i in 0..400 {
        let p = dir.path().join(format!("f{i}"));
        let when = UNIX_EPOCH + Duration::new(1_500_000_000 + rng.below(600_000_000), rng.below(1_000_000_000) as u32);
        put(&p, "x", when);
        paths.push(p);
    }
    let want = ruby_json("->(paths) { paths.map { |p| File.mtime(p).to_f } }", &json!(paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>()));
    for (p, w) in paths.iter().zip(want.as_array().unwrap()) {
        let got = mtime_f64(&fs::metadata(p).unwrap()).unwrap();
        assert_eq!(got.to_bits(), w.as_f64().unwrap().to_bits(), "{p:?}: {got} vs {w}");
        assert_eq!(Value::from(got).to_string(), w.to_string(), "{p:?}");
    }
}

#[test]
fn format_f_matches_rubys_sprintf_including_its_tie_handling() {
    let mut rng = Lcg(2024);
    let mut inputs: Vec<(f64, usize)> = vec![];
    for prec in 0..=3usize {
        let scale = 10f64.powi(prec as i32);
        // values on and beside every tie, at several magnitudes
        for _ in 0..4000 {
            let k = rng.below(2_000_000) as f64;
            let base = (k + 0.5) / scale;
            inputs.push((base, prec));
            inputs.push((base + f64::EPSILON * base, prec));
            inputs.push((base - f64::EPSILON * base, prec));
            inputs.push((k / scale, prec));
        }
        for _ in 0..3000 {
            inputs.push((rng.below(10_000_000_000) as f64 / 997.0 / (1 + rng.below(1000)) as f64, prec));
        }
        // the calls the code makes: counts over 1e3, 1e6 and 1e9, and cost sums
        for _ in 0..2000 {
            let c = rng.below(5_000_000_000) as f64;
            inputs.push((c / 1e3, prec));
            inputs.push((c / 1e6, prec));
            inputs.push((c / 1e9, prec));
        }
    }
    inputs.extend([(0.0, 2), (0.004, 2), (0.005, 2), (0.0049999999, 2), (0.00500001, 2), (0.05, 1), (0.95, 1), (99.95, 1), (999.95, 1), (9.5, 0), (10.5, 0), (0.5, 0), (1.5, 0)]);
    let want = ruby_json(
        "->(rows) { rows.map { |x, p| format(\"%.#{p}f\", x) } }",
        &Value::Array(inputs.iter().map(|(x, p)| json!([x, p])).collect()),
    );
    let mut wrong = vec![];
    for ((x, p), w) in inputs.iter().zip(want.as_array().unwrap()) {
        let got = format_f(*x, *p);
        if got != w.as_str().unwrap() {
            wrong.push(format!("{x:?} .{p}f: rust {got}, ruby {w}"));
        }
    }
    assert!(wrong.is_empty(), "{} of {} differ, e.g.\n{}", wrong.len(), inputs.len(), wrong.iter().take(15).cloned().collect::<Vec<_>>().join("\n"));
}

/// Two qm processes: A loads the ledger, B saves a session meanwhile, then A saves. B's record is a
/// record too — the merge over what is on disk keeps it, in disk-first order.
#[test]
fn a_save_keeps_what_another_process_saved_in_between() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (ruby, rust) = (world(a.path()), world(b.path()));

    let script = r#"require File.join(ARGV[0], "lib", "quartermaster")
include Quartermaster
log = SessionLog.new
first = UsageLedger.new
first.session_usage(log.session_for("u-priced"))
second = UsageLedger.new
second.session_usage(log.session_for("u-custom"))
second.save
first.save"#;
    let out = Command::new("ruby")
        .args(["-e", script, env!("CARGO_MANIFEST_DIR")])
        .env_clear()
        .env("QM_ROOT", &ruby.root)
        .env("QM_TRANSCRIPTS_DIR", &ruby.transcripts)
        .env("HOME", ruby.base.join("home"))
        .env("PATH", std::env::var("PATH").unwrap())
        .env("LANG", "C.UTF-8")
        .env("QM_TMUX_SOCKET", "qm-test-usage")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let ruby_file = fs::read_to_string(ruby.root.join("usage-cache.json")).unwrap().replace(ruby.base.to_str().unwrap(), "BASE");

    {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("QM_ROOT", &rust.root);
        std::env::set_var("QM_TRANSCRIPTS_DIR", &rust.transcripts);
        std::env::set_var("HOME", rust.base.join("home"));
        let sessions = SessionLog::read().sessions();
        let find = |id: &str| sessions.iter().find(|s| s.id == id).unwrap();
        let mut first = UsageLedger::new();
        first.session_usage(find("u-priced")).unwrap();
        let mut second = UsageLedger::new();
        second.session_usage(find("u-custom")).unwrap();
        second.save();
        first.save();
    }
    let rust_file = fs::read_to_string(rust.root.join("usage-cache.json")).unwrap().replace(rust.base.to_str().unwrap(), "BASE");
    assert_eq!(rust_file, ruby_file);
    let data: Value = serde_json::from_str(&rust_file).unwrap();
    let ids: Vec<&String> = data["sessions"].as_object().unwrap().keys().collect();
    assert_eq!(ids, ["u-custom", "u-priced"], "both processes' records, the one saved first on disk first");
}
