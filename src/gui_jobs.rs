//! The GUI's long actions, run on server threads and never under the action mutex (`GuiJobs`): one job
//! per key, its label shown as a badge until a state build hands out the result once.

use std::sync::Mutex;

use serde_json::{json, Map, Value};

struct Job {
    key: String,
    label: String,
    pf: bool,
}

#[derive(Default)]
struct Inner {
    running: Vec<Job>,
    finished: Vec<Value>,
}

static JOBS: Mutex<Inner> = Mutex::new(Inner { running: Vec::new(), finished: Vec::new() });

fn lock() -> std::sync::MutexGuard<'static, Inner> {
    JOBS.lock().unwrap_or_else(|p| p.into_inner())
}

/// What a job's block ends with: its result, or the error `ArgumentError` / `StandardError` would raise.
pub enum Failure {
    /// An `ArgumentError`: its message is the user's.
    Argument(String),
    /// Any other error: its message too, and also in `qm.log`.
    Other(String, &'static str),
}

/// Runs `work` on a thread unless the key already has a job, or it needs the dev server while another
/// such job runs. The reply says it started (`background`) or why not.
pub fn background(key: &str, label: &str, pf: bool, work: impl FnOnce() -> Result<Value, Failure> + Send + 'static) -> Value {
    {
        let mut jobs = lock();
        if let Some(job) = jobs.running.iter().find(|j| j.key == key) {
            return json!({"ok": false, "error": format!("{key} is still busy: {}", job.label)});
        }
        if pf {
            if let Some(other) = jobs.running.iter().find(|j| j.pf) {
                return json!({"ok": false, "error": format!("the dev server is busy ({}: {})", other.key, other.label)});
            }
        }
        jobs.running.push(Job { key: key.to_string(), label: label.to_string(), pf });
    }
    let (key, label) = (key.to_string(), label.to_string());
    std::thread::spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
        let result = match outcome {
            Ok(Ok(v)) => v,
            Ok(Err(Failure::Argument(m))) => json!({"ok": false, "error": m}),
            Ok(Err(Failure::Other(m, class))) => {
                crate::data_root::log_error("gui-job", class, &m);
                json!({"ok": false, "error": m})
            }
            Err(_) => {
                crate::data_root::log_error("gui-job", "RuntimeError", "the job panicked");
                json!({"ok": false, "error": "the job panicked"})
            }
        };
        let mut map: Map<String, Value> = match result {
            Value::Object(m) => m,
            other => Map::from_iter([("result".to_string(), other)]),
        };
        map.insert("key".into(), json!(key));
        map.insert("label".into(), json!(label));
        let mut jobs = lock();
        jobs.running.retain(|j| j.key != key);
        jobs.finished.push(Value::Object(map));
    });
    json!({"ok": true, "background": true})
}

/// A running job's badge says what it waits on now.
pub fn relabel(key: &str, label: &str) {
    if let Some(job) = lock().running.iter_mut().find(|j| j.key == key) {
        job.label = label.to_string();
    }
}

/// The running jobs, as the state lists them.
pub fn running() -> Vec<Value> {
    lock().running.iter().map(|j| json!({"key": j.key, "label": j.label, "pf": j.pf})).collect()
}

/// The finished jobs' results, handed out once.
pub fn take_finished() -> Vec<Value> {
    std::mem::take(&mut lock().finished)
}


static PROGRESS: Mutex<Vec<(String, Value)>> = Mutex::new(Vec::new());

/// The newest build-progress estimate per slug (`BuildProgress`), kept for the state builds.
pub fn store_progress(slug: &str, result: Value) {
    let mut all = PROGRESS.lock().unwrap_or_else(|p| p.into_inner());
    match all.iter_mut().find(|(k, _)| k == slug) {
        Some((_, v)) => *v = result,
        None => all.push((slug.to_string(), result)),
    }
}

pub fn progress() -> Vec<(String, Value)> {
    PROGRESS.lock().unwrap_or_else(|p| p.into_inner()).clone()
}


static DISTILLING: Mutex<Vec<(String, usize)>> = Mutex::new(Vec::new());
static DISTILLED: Mutex<Vec<Value>> = Mutex::new(Vec::new());

/// False when the slug is already being distilled.
pub fn start_distill(slug: &str, sessions: usize) -> bool {
    let mut running = DISTILLING.lock().unwrap_or_else(|p| p.into_inner());
    if running.iter().any(|(k, _)| k == slug) {
        return false;
    }
    running.push((slug.to_string(), sessions));
    true
}

pub fn finish_distill(slug: &str, result: Value) {
    DISTILLING.lock().unwrap_or_else(|p| p.into_inner()).retain(|(k, _)| k != slug);
    let mut map = match result {
        Value::Object(m) => m,
        other => Map::from_iter([("result".to_string(), other)]),
    };
    map.insert("slug".into(), json!(slug));
    DISTILLED.lock().unwrap_or_else(|p| p.into_inner()).push(Value::Object(map));
}

pub fn distilling_slugs() -> Vec<String> {
    DISTILLING.lock().unwrap_or_else(|p| p.into_inner()).iter().map(|(k, _)| k.clone()).collect()
}

/// The finished distills, handed out once.
pub fn take_distilled() -> Vec<Value> {
    std::mem::take(&mut *DISTILLED.lock().unwrap_or_else(|p| p.into_inner()))
}
