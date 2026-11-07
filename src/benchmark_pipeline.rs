//! The steps of one benchmark run (`BenchmarkPipeline`), run by the detached `qm bench-run <run dir>`
//! process `benchmark_run` spawns: plan & write (`claude -p` in the branch's checkout), measure
//! (`bin/rails runner bench.rb`, read-only or on a clone dropped afterwards), verdict (a sonnet call over
//! the table). Each state lands in meta.json before its step starts; a final state (a cancel's) is never
//! overwritten and stops the pipeline at its next step.

use std::io::{Read, Seek, SeekFrom, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use regex::Regex;
use serde_json::{json, Value};

use crate::benchmark_run::{self as run, merged};
use crate::ruby_text::strip;

const AGENT_TOOLS: &str = "Read,Grep,Glob,Write,Edit,Bash";
const MIN_FREE_BYTES: u64 = 20 * 1024 * 1024 * 1024;
const ERROR_MAX: usize = 300;
const STDERR_TAIL: usize = 4000;
const VERDICT_PROMPT: &str = "Below is a benchmark of a branch against main on the project's benchmark\ndatabase: why each case was picked, then the measured table (milliseconds\nper stage; \"timeout\" means more than 60 s; \"same ids\" checks a list case\nreturns what the earlier stage returned). Write two or three plain\nsentences: what got faster or slower, by roughly how much, and whether\nany case returns different ids. Timings move 20-40% between runs, even\nmain against main, so call a difference under about 40% noise rather\nthan a change. Answer with the sentences only.\n";

static TERMINATED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_term(_: libc::c_int) {
    TERMINATED.store(true, Ordering::SeqCst);
}

/// Why a step stopped the pipeline.
enum Stop {
    /// The run reached a final state elsewhere (a cancel).
    Stopped,
    /// A step failed: the message is meta.json's error, `stderr` the tail pipeline.log keeps.
    Step { message: String, stderr: Option<String> },
    /// An `ArgumentError`.
    Arg(String),
    /// Any other error, by Ruby class name.
    Other(&'static str, String),
}

type R<T> = Result<T, Stop>;

fn arg<T>(m: impl Into<String>) -> R<T> {
    Err(Stop::Arg(m.into()))
}

fn from_run(f: run::Fail) -> Stop {
    match f {
        run::Fail::Arg(m) => Stop::Arg(m),
        run::Fail::Unported => Stop::Other("RuntimeError", "the run folder could not be read or written".into()),
    }
}

pub fn events(dir: &std::path::Path) -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(dir.join("events.jsonl")) else { return vec![] };
    text.split_inclusive('\n').filter_map(|l| serde_json::from_str::<Value>(l).ok()).collect()
}

struct Pipeline {
    id: String,
    dir: std::path::PathBuf,
    cloned: Option<String>,
}

fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// `qm bench-run RUN_DIR`: the pipeline of one run; its exit code.
pub fn main(dir: &str) -> i32 {
    unsafe { libc::signal(libc::SIGTERM, on_term as extern "C" fn(libc::c_int) as libc::sighandler_t) };
    let id = std::path::Path::new(&std::fs::canonicalize(dir).unwrap_or_else(|_| dir.into())).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let Ok(run_dir) = run::dir_for(&id) else {
        eprintln!("not a benchmark run id: {id}");
        return 1;
    };
    let mut p = Pipeline { id, dir: run_dir, cloned: None };
    p.call();
    0
}

impl Pipeline {
    fn path(&self, file: &str) -> std::path::PathBuf {
        self.dir.join(file)
    }

    fn log(&self, message: &str) {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(self.path("pipeline.log")) {
            let _ = writeln!(f, "{} {message}", crate::clock::now_utc_iso());
        }
    }

    fn call(&mut self) {
        let outcome = (|| -> R<()> {
            let meta = run::update(&self.id, |current| Ok(merged(&current, &[("pid", json!(std::process::id()))]))).map_err(from_run)?;
            if !matches!(meta.get("rerunOf"), None | Some(Value::Null) | Some(Value::Bool(false))) {
                // a re-run copied the script
            } else {
                self.plan(&meta)?;
            }
            self.measure(&meta)?;
            self.verdict()?;
            self.finish("done", None)
        })();
        match outcome {
            Ok(()) | Err(Stop::Stopped) => {}
            Err(stop) => {
                let (class, message, stderr) = match stop {
                    Stop::Step { message, stderr } => ("Quartermaster::BenchmarkPipeline::StepFailed", message, stderr),
                    Stop::Arg(m) => ("ArgumentError", m, None),
                    Stop::Other(c, m) => (c, m, None),
                    Stop::Stopped => unreachable!(),
                };
                self.log(&format!("{class}: {message}"));
                if let Some(err) = &stderr {
                    let tail: String = if err.chars().count() > STDERR_TAIL { err.chars().skip(err.chars().count() - STDERR_TAIL).collect() } else { err.clone() };
                    self.log(&format!("stderr:\n{tail}"));
                }
                let _ = self.finish("failed", Some(&message));
            }
        }
    }

    fn plan(&mut self, meta: &Value) -> R<()> {
        self.transition("planning", &[])?;
        let (dir, id) = (self.dir.to_string_lossy().into_owned(), self.id.clone());
        let argv: Vec<String> = ["env", "QM_SKIP=1", &format!("QM_BENCH_DIR={dir}"), &format!("QM_BENCH_RUN={id}"), "claude", "-p", &text(meta.get("prompt")), "--model", &text(meta.get("model")), "--allowedTools", AGENT_TOOLS, "--add-dir", &dir, "--output-format", "stream-json", "--verbose", "--strict-mcp-config"].iter().map(|s| s.to_string()).collect();
        let (ok, stderr) = self.streamed(&argv, &text(meta.get("checkout")), "agent.jsonl", "pipeline.log")?;
        if !ok {
            return Err(self.failure("the planning agent failed", &stderr, "pipeline.log", "agent.jsonl", false));
        }
        self.run_spec().map(|_| ())
    }

    /// run.json as the agent wrote it, checked: `{cwd, clone}`.
    fn run_spec(&self) -> R<Value> {
        if !self.path("bench.rb").exists() {
            return arg("the planning agent wrote no bench.rb");
        }
        let Ok(raw) = std::fs::read_to_string(self.path("run.json")) else { return arg("the planning agent wrote no run.json") };
        let spec: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => {
                // json 2.7 names the text from where the failing value starts: the whole of what the agent wrote
                let rest = raw.trim_start_matches([' ', '\t', '\n', '\r']);
                let message = format!("unexpected token at '{rest}'");
                return arg(format!("run.json does not parse: {}", message.chars().take(200).collect::<String>()));
            }
        };
        let cwd = spec.get("cwd");
        match cwd {
            Some(Value::String(c)) if std::path::Path::new(c).is_dir() => Ok(spec),
            _ => {
                let shown = match cwd {
                    Some(Value::String(c)) => crate::ruby_text::inspect_ascii(c).ok_or_else(|| Stop::Other("RuntimeError", "an unprintable cwd".into()))?,
                    None | Some(Value::Null) => "nil".to_string(),
                    Some(other) => other.to_string(),
                };
                arg(format!("run.json's cwd {shown} is no directory"))
            }
        }
    }

    fn measure(&mut self, meta: &Value) -> R<()> {
        let result = (|| -> R<()> {
            let spec = self.run_spec()?;
            self.transition("measuring", &[])?;
            let url = if matches!(spec.get("clone"), Some(v) if !matches!(v, Value::Null | Value::Bool(false))) { self.clone_db(meta)? } else { run::read_only_url(meta).map_err(from_run)? };
            let (dir, id) = (self.dir.to_string_lossy().into_owned(), self.id.clone());
            let bench = self.path("bench.rb").to_string_lossy().into_owned();
            let argv: Vec<String> = ["env", "QM_SKIP=1", &format!("QM_BENCH_DIR={dir}"), &format!("QM_BENCH_RUN={id}"), &format!("DATABASE_URL={url}"), "bin/rails", "runner", &bench].iter().map(|s| s.to_string()).collect();
            let (ok, stderr) = self.streamed(&argv, &text(spec.get("cwd")), "bench.out", "bench.log")?;
            if !ok {
                return Err(self.failure("bin/rails runner failed", &stderr, "bench.log", "bench.out", true));
            }
            Ok(())
        })();
        if self.cloned.is_some() {
            self.drop_clone();
        }
        result
    }

    /// The clone's URL, after checking there is room for it.
    fn clone_db(&mut self, meta: &Value) -> R<String> {
        let free = free_bytes();
        if free < MIN_FREE_BYTES {
            return arg(format!("only {} GB free; a clone of the benchmark database needs {} GB", free / 1024u64.pow(3), MIN_FREE_BYTES / 1024u64.pow(3)));
        }
        let name = run::clone_name(meta).map_err(from_run)?;
        let template = run::database(meta).map_err(from_run)?;
        let result = crate::command_runner::call(&["createdb", "-T", &template, &name]);
        if !result.success {
            return Err(self.failure(&format!("createdb {name} failed"), &result.err, "pipeline.log", "pipeline.log", true));
        }
        self.cloned = Some(name.clone());
        Ok(format!("postgres:///{name}"))
    }

    /// A failed drop is logged and lands on meta.json's warning, never raised.
    fn drop_clone(&self) {
        let Some(name) = &self.cloned else { return };
        let result = crate::command_runner::call(&["dropdb", "--if-exists", "--force", name]);
        if result.success {
            return;
        }
        let message = match self.failure("dropdb failed", &result.err, "pipeline.log", "pipeline.log", false) {
            Stop::Step { message, .. } => message,
            _ => String::new(),
        };
        self.log(&format!("{message}\n{}", strip(&result.err)));
        let _ = run::clone_left(&self.id, &result.err);
    }

    fn verdict(&self) -> R<()> {
        let table = std::fs::read_to_string(self.path("bench.out")).map(|t| strip(&t).to_string()).unwrap_or_default();
        if table.is_empty() {
            return arg("bench.rb printed no table");
        }
        self.transition("verdict", &[])?;
        let prompt = format!("{VERDICT_PROMPT}\n{}\n\n{table}\n", self.whys());
        let result = crate::command_runner::call(&["env", "QM_SKIP=1", "claude", "-p", &prompt, "--model", "sonnet", "--tools", "", "--strict-mcp-config", "--no-session-persistence"]);
        let out = strip(&result.out).to_string();
        if !(result.success && !out.is_empty()) {
            let stderr = if strip(&result.err).is_empty() { out } else { result.err.clone() };
            return Err(self.failure("the verdict call failed", &stderr, "pipeline.log", "pipeline.log", true));
        }
        std::fs::write(self.path("verdict.md"), format!("{out}\n")).map_err(|e| Stop::Other("Errno::EIO", e.to_string()))
    }

    fn whys(&self) -> String {
        let plan = events(&self.dir).into_iter().find(|e| e.get("t") == Some(&json!("plan")));
        let cases: Vec<Value> = plan.and_then(|p| p.get("cases").and_then(Value::as_array).cloned()).unwrap_or_default();
        let lines: Vec<String> = cases.iter().map(|c| format!("- {} / {}: {}", text(c.get("subject")), text(c.get("case")), text(c.get("why")))).collect();
        format!("Why each case:\n{}", if lines.is_empty() { "(no plan event)".to_string() } else { lines.join("\n") })
    }

    /// Moves the run to `state` unless it is already final; `Stopped` when it is.
    fn transition(&self, state: &str, extra: &[(&str, Value)]) -> R<()> {
        let mut moved = false;
        run::update(&self.id, |meta| {
            if run::is_final(&meta) {
                return Ok(meta);
            }
            moved = true;
            let mut next = merged(&meta, &[("state", json!(state))]);
            next = merged(&next, extra);
            Ok(next)
        })
        .map_err(from_run)?;
        if moved { Ok(()) } else { Err(Stop::Stopped) }
    }

    fn finish(&self, state: &str, error: Option<&str>) -> R<()> {
        match self.transition(state, &[("finishedAt", json!(crate::clock::now_utc_iso())), ("error", error.map(|e| json!(e)).unwrap_or(Value::Null))]) {
            Err(Stop::Stopped) => return Ok(()),
            other => other?,
        }
        let meta = run::read(&self.id).map_err(from_run)?;
        let summary = if state == "done" {
            one_line(&std::fs::read_to_string(self.path("verdict.md")).map_err(|e| Stop::Other("Errno::ENOENT", e.to_string()))?)
        } else {
            format!("failed: {}", one_line(error.unwrap_or("")))
        };
        self.announce(&meta, state, &summary);
        self.record(&meta, &summary);
        Ok(())
    }

    fn announce(&self, meta: &Value, state: &str, summary: &str) {
        crate::turn_notifier::announce(&format!("Benchmark {state} · {}", text(meta.get("branch"))), summary, &format!("qm-bench-{}", self.id), Some(vec!["open-chat".into(), text(meta.get("slug")), "--benchmark".into(), text(meta.get("branch"))]), false);
    }

    fn record(&self, meta: &Value, summary: &str) {
        let note = format!("Benchmark {}: {summary} (run {})", text(meta.get("branch")), self.id);
        match crate::work_item::WorkItem::find(&text(meta.get("slug"))) {
            Ok(mut item) => {
                if let Err(e) = item.add_note(&note, "history") {
                    self.log(&format!("history note: {}", e.message));
                }
            }
            Err(e) => self.log(&format!("history note: {}", e.message)),
        }
    }

    /// Runs argv streaming to the run folder's out and err files; `(success, the stderr this call appended)`.
    fn streamed(&self, argv: &[String], chdir: &str, out: &str, err: &str) -> R<(bool, String)> {
        let err_path = self.path(err);
        let from = std::fs::metadata(&err_path).map(|m| m.len()).unwrap_or(0);
        let open = |p: &std::path::Path| std::fs::OpenOptions::new().create(true).append(true).open(p);
        let (stdout, stderr) = (open(&self.path(out)).map_err(|e| Stop::Other("Errno::EACCES", e.to_string()))?, open(&err_path).map_err(|e| Stop::Other("Errno::EACCES", e.to_string()))?);
        let program = crate::command_runner::resolve_program(&argv[0]);
        let mut command = Command::new(&program);
        command.args(&argv[1..]).current_dir(chdir).stdin(Stdio::null()).stdout(stdout).stderr(stderr);
        let success = match command.spawn() {
            Ok(mut child) => child.wait().map(|s| s.success()).unwrap_or(false),
            Err(_) => false,
        };
        if TERMINATED.load(Ordering::SeqCst) {
            return Err(Stop::Other("SignalException", "SIGTERM".into()));
        }
        let mut appended = vec![];
        if let Ok(mut f) = std::fs::File::open(&err_path) {
            let _ = f.seek(SeekFrom::Start(from));
            let _ = f.read_to_end(&mut appended);
        }
        Ok((success, String::from_utf8_lossy(&appended).into_owned()))
    }

    /// "<step>: <cause line> — see <log>" in at most `ERROR_MAX` chars, the cause losing its front at a
    /// path or word boundary.
    fn failure(&self, step: &str, stderr: &str, log: &str, silent: &str, keep: bool) -> Stop {
        let Some(cause) = run::cause_line(stderr) else { return Stop::Step { message: format!("{step}: it printed nothing — see {silent}"), stderr: None } };
        let suffix = format!(" — see {log}");
        let room = ERROR_MAX as i64 - step.chars().count() as i64 - 2 - suffix.chars().count() as i64;
        Stop::Step { message: format!("{step}: {}{suffix}", front_clipped(&cause, room.max(0) as usize)), stderr: keep.then(|| strip(stderr).to_string()) }
    }
}

fn front_clipped(text: &str, room: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= room {
        return text.to_string();
    }
    let tail: Vec<char> = chars[chars.len() - (room.max(1) - 1)..].to_vec();
    let tail_text: String = tail.iter().collect();
    match Regex::new(r"[ \t\r\n\x0c\x0b/]").unwrap().find(&tail_text) {
        Some(m) => format!("…{}", &tail_text[m.end()..]),
        None => format!("…{tail_text}"),
    }
}

fn one_line(text: &str) -> String {
    let mut squished = String::new();
    let mut space = false;
    for c in text.chars() {
        if matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r') {
            space = true;
        } else {
            if space {
                squished.push(' ');
            }
            space = false;
            squished.push(c);
        }
    }
    if space {
        squished.push(' ');
    }
    run::clip(strip(&squished), 240)
}

/// Bytes free in the home volume, where Postgres.app keeps its data.
fn free_bytes() -> u64 {
    let home = crate::data_root::home();
    let Ok(out) = Command::new("df").args(["-Pk", &home]).output() else { return 0 };
    if !out.status.success() {
        return 0;
    }
    String::from_utf8_lossy(&out.stdout).lines().last().and_then(|l| l.split_whitespace().nth(3)).and_then(|n| n.parse::<u64>().ok()).map(|k| k * 1024).unwrap_or(0)
}

