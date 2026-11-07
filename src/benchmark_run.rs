//! One benchmark run of a branch against its project's benchmark database (`BenchmarkRun`,
//! docs/areas/benchmarks.md): its folder under `benchmarks/<run id>`, its meta.json, and starting,
//! cancelling and re-running it. The steps themselves run in a detached pipeline process
//! (`benchmark_pipeline`, `qm bench-run <run dir>`) that outlives the GUI; this module only spawns it.

use std::os::unix::io::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use regex::Regex;
use serde_json::{json, Map, Value};

use crate::data_root;
use crate::ruby_text::strip;
use crate::work_item::WorkItem;

pub const FINAL: [&str; 3] = ["done", "failed", "cancelled"];
const STARTING_GRACE: f64 = 30.0;
const MODELS: [&str; 2] = ["opus", "sonnet"];
const DEFAULT_REQUEST: &str = "work out what this branch's changes make slower or faster from its diff against main";

static START_LOCK: Mutex<()> = Mutex::new(());

const PROMPT: &str = "You plan and write a benchmark for the branch `%{branch}`; qm\nmeasures it after you stop. Read the benchmark skill\n(%{skill}/SKILL.md) and follow its Protocol, Guard and\n\"Progress events\" sections.\n\nStages, compared in this order (name: ref @ sha); every event names a\nstage by exactly this name:\n%{stages}\n\nWhat to benchmark: %{request}\n\nThe run folder is %{dir} (also $QM_BENCH_DIR; the run id is\n$QM_BENCH_RUN = %{id}). Write there:\n1. bench.rb: a copy of %{skill}/bin/example_bench.rb with STAGES,\n   cases and subject picks swapped for this branch. It appends the\n   subject, stage, case, ids and done events to $QM_BENCH_DIR/events.jsonl\n   as the example does, and leaves PLAN_DIR unset (plans go to\n   $QM_BENCH_DIR/plans/). Keep the example's dry mode (BENCH_DRY=1:\n   resolve the subjects, load every stage and build every case, then\n   exit without measuring or emitting) and time with its realtime_ms.%{notes}\n2. One `plan` event line appended to $QM_BENCH_DIR/events.jsonl: the\n   stages and every case with its subject and why.\n3. run.json: {\"cwd\": \"%{checkout}\", \"clone\": false}.\n   cwd is this branch's checkout, so it holds the branch's version of\n   every staged file: never rely on the checkout for main's code.\n   bench.rb loads main like a stage (Protocol step 2): keep the\n   example's MAIN stage, which evals origin/main's version of every\n   file any stage changes before measuring main.\n   Never check out a branch or create a worktree.\n   \"clone\" is true only when a case writes (a job, a listener, a service\n   that saves); qm then runs the script on a clone named exactly\n   %{database}_%{id} and drops it afterwards. Otherwise qm runs it\n   read-only with DATABASE_URL='%{read_only}'.\n\nBefore you stop, load-check bench.rb: in run.json's cwd run\n`BENCH_DRY=1 DATABASE_URL='%{read_only}' bin/rails runner $QM_BENCH_DIR/bench.rb`\nand fix bench.rb until it exits 0. Keep the example's dry mode in\nbench.rb, since qm's measuring run is the same script without it.\n\nDo not measure: beyond that dry run, write bench.rb, the plan event and\nrun.json, then stop.\n";

/// What stopped a call: the `ArgumentError` Ruby raises, or a value only Ruby reads exactly.
pub enum Fail {
    Arg(String),
    Unported,
}

type R<T> = Result<T, Fail>;

fn arg<T>(m: impl Into<String>) -> R<T> {
    Err(Fail::Arg(m.into()))
}

pub fn dir() -> std::path::PathBuf {
    data_root::root().join("benchmarks")
}

pub fn run_id(id: &str) -> bool {
    let re = Regex::new(r"\A[0-9]{8}_[0-9]{6}(_[0-9]+)?\z").unwrap();
    re.is_match(id)
}

pub fn dir_for(id: &str) -> R<std::path::PathBuf> {
    if !run_id(id) {
        return arg(format!("not a benchmark run id: {id}"));
    }
    Ok(dir().join(id))
}

pub fn settings(project: Option<&str>) -> Option<crate::project_config::Benchmark> {
    crate::project_config::ProjectConfig::current().settings(project).and_then(|s| s.benchmark.clone())
}

pub fn enabled(item: &WorkItem) -> bool {
    settings(item.project().as_deref()).is_some()
}

pub fn settings_checked(project: &str) -> R<crate::project_config::Benchmark> {
    match settings(Some(project)) {
        Some(s) => Ok(s),
        None => arg(format!("{project} has no benchmarks: set projects.{project}.benchmark in config.yml")),
    }
}

fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// The template database a run measures on: the one it started with, else its project's.
pub fn database(meta: &Value) -> R<String> {
    match meta.get("database") {
        Some(Value::String(d)) => Ok(d.clone()),
        _ => Ok(settings_checked(&text(meta.get("project")))?.database),
    }
}

pub fn read_only_url(meta: &Value) -> R<String> {
    Ok(crate::project_config::Benchmark { database: database(meta)?, skill: String::new(), notes: None }.read_only_url())
}

pub fn read(id: &str) -> R<Value> {
    let path = dir_for(id)?.join("meta.json");
    match std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        Some(v) => Ok(v),
        None => arg(format!("no benchmark run {id}")),
    }
}

/// Every run's meta, newest first; an unreadable one is skipped.
pub fn all() -> Vec<Value> {
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(dir()).map(|d| d.filter_map(|e| e.ok()).map(|e| e.path().join("meta.json")).filter(|p| p.exists()).collect()).unwrap_or_default();
    paths.sort();
    paths.reverse();
    paths.iter().filter_map(|p| std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok())).collect()
}

/// meta.json written whole: a temp file renamed over it.
pub fn write_meta(dir: &std::path::Path, meta: &Value) -> R<()> {
    let temp = dir.join(format!("meta.json.{}.{:?}.tmp", std::process::id(), std::thread::current().id()));
    std::fs::write(&temp, crate::drafts::pretty(meta)).map_err(|_| Fail::Unported)?;
    std::fs::rename(&temp, dir.join("meta.json")).map_err(|_| Fail::Unported)
}

/// Rewrites the run's meta.json under the run's lock; `f` gets the current meta and returns the new one.
pub fn update(id: &str, f: impl FnOnce(Value) -> R<Value>) -> R<Value> {
    let dir = dir_for(id)?;
    let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(dir.join(".lock")).map_err(|_| Fail::Unported)?;
    unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) };
    let current = std::fs::read_to_string(dir.join("meta.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).ok_or(Fail::Unported)?;
    let meta = f(current)?;
    write_meta(&dir, &meta)?;
    Ok(meta)
}

pub fn merged(meta: &Value, pairs: &[(&str, Value)]) -> Value {
    let mut m = meta.as_object().cloned().unwrap_or_default();
    for (k, v) in pairs {
        m.insert((*k).to_string(), v.clone());
    }
    Value::Object(m)
}

pub fn is_final(meta: &Value) -> bool {
    meta.get("state").and_then(Value::as_str).is_some_and(|s| FINAL.contains(&s))
}

/// The one liveness rule: a non-final run whose pid passes `alive`, or, with no pid yet, one that
/// started less than `STARTING_GRACE` seconds before `now`.
pub fn live(meta: &Value, now: f64, alive: &dyn Fn(i64, &str) -> bool) -> bool {
    if is_final(meta) {
        return false;
    }
    match meta.get("pid") {
        None | Some(Value::Null) => match crate::clock::parse_f64(&text(meta.get("startedAt"))) {
            Some(t) => now - t < STARTING_GRACE,
            None => false,
        },
        Some(Value::Number(n)) => n.as_i64().is_some_and(|pid| alive(pid, &text(meta.get("id")))),
        Some(_) => false,
    }
}

/// Whether a process's command line is run `id`'s pipeline: a word naming exactly that run's folder.
pub fn pipeline(command_line: Option<&str>, id: &str) -> bool {
    command_line.unwrap_or("").split_whitespace().any(|word| std::path::Path::new(word).file_name().is_some_and(|f| f == id))
}

pub fn command_line(pid: i64) -> Option<String> {
    let out = Command::new("ps").args(["-o", "command=", "-p", &pid.to_string()]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| strip(&String::from_utf8_lossy(&out.stdout)).to_string())
}

pub fn alive(pid: i64, id: &str) -> bool {
    pipeline(command_line(pid).as_deref(), id)
}

pub fn live_now(meta: &Value) -> bool {
    live(meta, crate::clock::now_f64(), &alive)
}

/// The state a reader shows: a non-final run whose pipeline is gone has failed.
pub fn effective(meta: &Value) -> Value {
    if is_final(meta) || live_now(meta) {
        return meta.clone();
    }
    let error = match meta.get("error") {
        Some(v) if !matches!(v, Value::Null | Value::Bool(false)) => v.clone(),
        _ => json!("pipeline died"),
    };
    merged(meta, &[("state", json!("failed")), ("error", error)])
}

/// The one live run among `metas`, as `{id, slug, branch, state}`.
pub fn busy(metas: &[Value]) -> Value {
    match metas.iter().find(|m| live_now(m)) {
        Some(m) => json!({"id": m.get("id").cloned().unwrap_or(Value::Null), "slug": m.get("slug").cloned().unwrap_or(Value::Null), "branch": m.get("branch").cloned().unwrap_or(Value::Null), "state": m.get("state").cloned().unwrap_or(Value::Null)}),
        None => Value::Null,
    }
}

/// Per branch of `slug`, its newest run that finished done, as `{id, finishedAt}`.
pub fn finished_by_branch(metas: &[Value], slug: &str) -> Value {
    let mut done = Map::new();
    for meta in metas {
        if meta.get("slug").and_then(Value::as_str) != Some(slug) || meta.get("state").and_then(Value::as_str) != Some("done") {
            continue;
        }
        let branch = text(meta.get("branch"));
        if !done.contains_key(&branch) {
            done.insert(branch, json!({"id": meta.get("id").cloned().unwrap_or(Value::Null), "finishedAt": meta.get("finishedAt").cloned().unwrap_or(Value::Null)}));
        }
    }
    Value::Object(done)
}

/// `BenchmarkPipeline.cause_line`: the line of a failed command's stderr that names its cause.
pub fn cause_line(stderr: &str) -> Option<String> {
    let lines: Vec<&str> = stderr.split('\n').map(strip).filter(|l| !l.is_empty()).collect();
    let exception = Regex::new(r"\b\w*(Error|Exception)\b|:in '.*': ").unwrap();
    let warning = Regex::new(r"(?i)\bwarning:").unwrap();
    lines.iter().find(|l| exception.is_match(l) && !warning.is_match(l)).or(lines.last()).map(|l| l.to_string())
}

pub fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() > limit { format!("{}…", crate::ruby_text::rstrip(&text.chars().take(limit).collect::<String>())) } else { text.to_string() }
}

pub fn clone_name(meta: &Value) -> R<String> {
    Ok(format!("{}_{}", database(meta)?, text(meta.get("id"))))
}

/// Records on meta.json's warning that the run's clone survived a failed dropdb.
pub fn clone_left(id: &str, stderr: &str) -> R<Value> {
    let name = clone_name(&read(id)?)?;
    let cause = clip(&cause_line(stderr).unwrap_or_else(|| "it printed nothing".to_string()), 200);
    let note = format!("dropdb failed: {cause} — clone {name} is still on disk; drop it with: dropdb {name} — see pipeline.log");
    update(id, |meta| {
        if text(meta.get("warning")).contains(&note) {
            return Ok(meta);
        }
        let earlier = text(meta.get("warning"));
        let joined = [earlier, note.clone()].into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ");
        Ok(merged(&meta, &[("warning", json!(joined))]))
    })
}

fn stage_sha(root: &str, reference: &str) -> Option<String> {
    crate::git::git(Some(root), &["rev-parse", "--verify", "--quiet", &format!("{reference}^{{commit}}")])
}

/// main, then one stage per gh-stack layer from the bottom up to and including the branch, else the
/// branch alone; each `{name, ref, sha}`.
fn stages_for(tip: &Value) -> R<Vec<Value>> {
    let root = text(tip.get("root"));
    let branch = text(tip.get("branch"));
    let Some(trunk) = crate::git_repo::GitRepo::new(&root).trunk_refs().into_iter().next() else {
        return arg(format!("no main to compare against: none of {} exists", crate::git_repo::TRUNK_REFS.join(", ")));
    };
    let stacks = crate::gh_stack::for_root(&root);
    let found = stacks.stacks_for(&[branch.clone()]);
    let entries: Vec<(String, Value)> = match found.first() {
        Some(stack) => stack.entries.iter().map(|e| (e.branch.clone(), e.pr_number.clone())).collect(),
        None => vec![(branch.clone(), Value::Null)],
    };
    let upto = entries.iter().position(|(b, _)| *b == branch).ok_or(Fail::Unported)?;
    let mut stages = vec![json!({"name": "main", "ref": trunk, "sha": stage_sha(&root, &trunk)})];
    for (name, pr) in entries.into_iter().take(upto + 1) {
        let candidates = [name.clone(), format!("origin/{name}")];
        let Some(reference) = candidates.iter().find(|r| stage_sha(&root, r).is_some()) else { return arg(format!("stack layer {name} has no ref")) };
        let label = match &pr {
            Value::Null | Value::Bool(false) => name.clone(),
            other => format!("#{}", text(Some(other))),
        };
        stages.push(json!({"name": label, "ref": reference, "sha": stage_sha(&root, reference)}));
    }
    Ok(stages)
}

fn prompt(meta: &Value, id: &str, settings: &crate::project_config::Benchmark) -> R<String> {
    let stages = meta["stages"].as_array().map(|l| l.iter().map(|s| format!("- {}: {} @ {}", text(s.get("name")), text(s.get("ref")), text(s.get("sha")))).collect::<Vec<_>>().join("\n")).unwrap_or_default();
    let request = if text(meta.get("request")).is_empty() { DEFAULT_REQUEST.to_string() } else { text(meta.get("request")) };
    let notes = match &settings.notes {
        Some(n) => format!("\n{}", n.split('\n').map(|l| format!("   {l}")).collect::<Vec<_>>().join("\n")),
        None => String::new(),
    };
    let pairs = [
        ("branch", text(meta.get("branch"))),
        ("stages", stages),
        ("request", request),
        ("dir", dir_for(id)?.to_string_lossy().into_owned()),
        ("id", id.to_string()),
        ("checkout", text(meta.get("checkout"))),
        ("skill", settings.skill.clone()),
        ("database", settings.database.clone()),
        ("read_only", settings.read_only_url()),
        ("notes", notes),
    ];
    // one pass over the template, so a value that holds "%{…}" is never substituted again
    let re = Regex::new(r"%\{([a-z_]+)\}").unwrap();
    Ok(re.replace_all(PROMPT, |c: &regex::Captures| pairs.iter().find(|(k, _)| *k == &c[1]).map(|(_, v)| v.clone()).unwrap_or_default()).into_owned())
}

/// The detached pipeline: its own process group, stdio to pipeline.log, so it outlives the GUI.
fn spawn_pipeline(dir: &std::path::Path) -> Result<i64, String> {
    let log = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("pipeline.log")).map_err(|e| e.to_string())?;
    let err = log.try_clone().map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = Command::new(exe);
    command.arg("bench-run").arg(dir).current_dir(dir).stdin(Stdio::null()).stdout(log).stderr(err);
    unsafe {
        command.pre_exec(|| {
            libc::setpgid(0, 0);
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let pid = child.id() as i64;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(pid)
}

/// The folder and meta.json of a new run, then its pipeline. `copy_from`: the run whose script a re-run
/// measures again.
fn launch(copy_from: Option<&std::path::Path>, body: impl FnOnce(&str) -> R<Map<String, Value>>) -> R<Value> {
    let _lock = START_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let metas = all();
    if let Value::Object(other) = busy(&metas) {
        return arg(format!("a benchmark is already running: {} ({}, run {})", text(other.get("branch")), text(other.get("slug")), text(other.get("id"))));
    }
    let (id, run_dir) = claim_dir()?;
    let mut meta = Map::new();
    meta.insert("v".into(), json!(1));
    meta.insert("id".into(), json!(id));
    meta.extend(body(&id)?);
    meta.insert("startedAt".into(), json!(crate::clock::now_utc_iso()));
    meta.insert("finishedAt".into(), Value::Null);
    meta.insert("state".into(), json!(if copy_from.is_some() { "measuring" } else { "planning" }));
    meta.insert("error".into(), Value::Null);
    meta.insert("pid".into(), Value::Null);
    meta.entry("rerunOf".to_string()).or_insert(Value::Null);
    if let Some(source) = copy_from {
        copy_script(source, &run_dir)?;
    }
    let meta = Value::Object(meta);
    write_meta(&run_dir, &meta)?;
    let pid = match spawn_pipeline(&run_dir) {
        Ok(p) => p,
        Err(message) => {
            update(&id, |current| Ok(merged(&current, &[("state", json!("failed")), ("error", json!(format!("could not start: {message}")))])))?;
            return arg(format!("the benchmark pipeline did not start: {message}"));
        }
    };
    update(&id, |current| Ok(merged(&current, &[("pid", json!(pid))])))
}

/// run id: the UTC second, `_2`, `_3`… on a clash; database-name safe.
fn claim_dir() -> R<(String, std::path::PathBuf)> {
    std::fs::create_dir_all(dir()).map_err(|_| Fail::Unported)?;
    let stamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
    for n in 1.. {
        let id = if n == 1 { stamp.clone() } else { format!("{stamp}_{n}") };
        let path = dir().join(&id);
        match std::fs::create_dir(&path) {
            Ok(()) => {
                std::fs::create_dir_all(path.join("plans")).map_err(|_| Fail::Unported)?;
                return Ok((id, path));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(Fail::Unported),
        }
    }
    Err(Fail::Unported)
}

fn copy_script(source: &std::path::Path, dir: &std::path::Path) -> R<()> {
    for file in ["bench.rb", "run.json"] {
        std::fs::copy(source.join(file), dir.join(file)).map_err(|_| Fail::Unported)?;
    }
    if let Some(plan) = crate::benchmark_pipeline::events(source).into_iter().find(|e| e.get("t") == Some(&json!("plan"))) {
        std::fs::write(dir.join("events.jsonl"), format!("{plan}\n")).map_err(|_| Fail::Unported)?;
    }
    Ok(())
}

/// Starts a run of the item's branch: folder, meta.json, the detached pipeline.
pub fn start(item: &mut WorkItem, branch: &str, request: &str, model: &str) -> R<Value> {
    if !MODELS.contains(&model) {
        return arg(format!("model must be one of: {}", MODELS.join(", ")));
    }
    let project = item.project().unwrap_or_default();
    let settings = settings_checked(&project)?;
    let tip = crate::branch_diff::tip(item, if branch.is_empty() { None } else { Some(branch) }).map_err(|f| match f {
        crate::branch_diff::Failure::Arg(m) => Fail::Arg(m),
        crate::branch_diff::Failure::Unported => Fail::Unported,
    })?;
    let stages = stages_for(&tip)?;
    launch(None, |id| {
        let mut m = Map::new();
        m.insert("slug".into(), json!(item.slug));
        m.insert("project".into(), json!(project));
        m.insert("database".into(), json!(settings.database));
        m.insert("root".into(), tip["root"].clone());
        m.insert("checkout".into(), tip["path"].clone());
        m.insert("branch".into(), tip["branch"].clone());
        m.insert("request".into(), json!(strip(request)));
        m.insert("model".into(), json!(model));
        m.insert("stages".into(), Value::Array(stages));
        m.insert("head".into(), tip["head"].clone());
        let text = prompt(&Value::Object(m.clone()), id, &settings)?;
        m.insert("prompt".into(), json!(text));
        Ok(m)
    })
}

/// A new run of an earlier run's script (no agent), measured afresh; refused once any stage ref
/// resolves to another sha than the original run recorded.
pub fn rerun(id: &str) -> R<Value> {
    let original = read(id)?;
    let source = dir_for(id)?;
    for file in ["bench.rb", "run.json"] {
        if !source.join(file).exists() {
            return arg(format!("run {id} has no {file} to re-run"));
        }
    }
    let root = text(original.get("root"));
    let stages = original["stages"].as_array().ok_or(Fail::Unported)?;
    let moved: Vec<String> = stages.iter().filter(|s| stage_sha(&root, &text(s.get("ref"))).as_deref() != s.get("sha").and_then(Value::as_str)).map(|s| text(s.get("ref"))).collect();
    if !moved.is_empty() {
        return arg(format!("can't re-run {id}: {} moved since it ran — start a new benchmark", moved.join(", ")));
    }
    let database = database(&original)?;
    launch(Some(&source), |_| {
        let mut m = Map::new();
        for k in ["slug", "project", "root", "checkout", "branch", "request", "model", "stages", "head"] {
            if let Some(v) = original.get(k) {
                m.insert(k.into(), v.clone());
            }
        }
        m.insert("database".into(), json!(database));
        m.insert("rerunOf".into(), json!(id));
        Ok(m)
    })
}

/// Stops a run: state cancelled first (so the dying pipeline writes no other), then its process group
/// killed, then its clone dropped by exactly its own name. Measured cases stay.
pub fn cancel(id: &str) -> R<Value> {
    let mut before = Value::Null;
    let meta = update(id, |current| {
        before = current.clone();
        if is_final(&current) {
            return arg(format!("run {id} already {}", text(current.get("state"))));
        }
        Ok(merged(&current, &[("state", json!("cancelled")), ("finishedAt", json!(crate::clock::now_utc_iso()))]))
    })?;
    if let Some(pid) = before.get("pid").and_then(Value::as_i64) {
        if live_now(&before) {
            unsafe { libc::kill(-(pid as i32), libc::SIGTERM) };
        }
    }
    let Ok(clone) = clone_name(&before) else { return Ok(meta) };
    let dropped = crate::command_runner::call(&["dropdb", "--if-exists", "--force", &clone]);
    if dropped.success || before.get("state").and_then(Value::as_str) != Some("measuring") {
        return Ok(meta);
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir_for(id)?.join("pipeline.log")) {
        let _ = write!(f, "{} cancel: dropdb failed\n{}\n", crate::clock::now_utc_iso(), strip(&dropped.err));
    }
    clone_left(id, &dropped.err)
}
