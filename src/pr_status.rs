//! PR state per work-item branch, via `gh`, cached in `pr-cache.json` so repaints don't hit the
//! network on every render. Synchronous: an expired entry blocks on `gh`.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde_json::{Map, Value};

use crate::data_root;
use crate::project_config::ProjectConfig;
use crate::work_item::WorkItem;

const CACHE_TTL_SECONDS: i64 = 6 * 3600;
const OPEN_TTL_SECONDS: i64 = 5 * 60;
const NO_PR_TTL_SECONDS: i64 = 10 * 60;

/// The GUI's mode: an expired entry is served as it is and refetched on a thread, so no page load
/// waits on the network; `refreshing` says an answer may still improve.
static BACKGROUND: AtomicBool = AtomicBool::new(false);
static SERVED_STALE: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT: Mutex<Vec<String>> = Mutex::new(Vec::new());
static CACHE_LOCK: Mutex<()> = Mutex::new(());

pub fn set_background(on: bool) {
    BACKGROUND.store(on, Ordering::SeqCst);
}

/// Forgets which answers were served stale: a state build starts from here.
pub fn start_build() {
    SERVED_STALE.store(false, Ordering::SeqCst);
}

/// True while an answer served since `start_build` may still improve, or a fetch is running.
pub fn refreshing() -> bool {
    SERVED_STALE.load(Ordering::SeqCst) || !IN_FLIGHT.lock().unwrap().is_empty()
}

pub struct PrStatus {
    now: i64,
    config: ProjectConfig,
}

fn cache_path() -> std::path::PathBuf {
    data_root::root().join("pr-cache.json")
}

fn read_cache() -> Map<String, Value> {
    match std::fs::read_to_string(cache_path()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

fn write_cache(cache: &Map<String, Value>) {
    let _ = data_root::ensure_setup();
    let path = cache_path();
    let temp = path.with_file_name(format!(".pr-cache.json.tmp-{}", std::process::id()));
    if std::fs::write(&temp, Value::Object(cache.clone()).to_string()).is_ok() {
        let _ = std::fs::rename(&temp, &path);
    }
}

fn ttl(entry: &Value) -> i64 {
    match entry.get("state") {
        None | Some(Value::Null) => NO_PR_TTL_SECONDS,
        Some(Value::String(s)) if s == "OPEN" => OPEN_TTL_SECONDS,
        _ => CACHE_TTL_SECONDS,
    }
}

impl PrStatus {
    pub fn new(now: i64) -> PrStatus {
        PrStatus { now, config: ProjectConfig::current() }
    }

    pub fn root_for(&self, project: Option<&str>) -> Option<&str> {
        let name = project?;
        self.config.roots().into_iter().find(|(n, _)| *n == name).map(|(_, r)| r)
    }

    /// The branch's PR state ("OPEN", "MERGED", "CLOSED") or None; fetches when the cache has expired.
    pub fn state_for(&self, item: &WorkItem, branch: &str) -> Option<String> {
        let key = format!("{}#{branch}", item.project().unwrap_or_default());
        let cache = read_cache();
        if let Some(entry) = cache.get(&key) {
            if let Some(ts) = entry.get("ts").and_then(Value::as_i64) {
                if self.now - ts < ttl(entry) {
                    return entry.get("state").and_then(Value::as_str).map(String::from);
                }
            }
        }
        if BACKGROUND.load(Ordering::SeqCst) {
            return self.serve_stale(item, branch, &key, cache.get(&key));
        }
        self.store(&key, self.fetch_via_gh(item, branch))
    }

    /// The branch's PR as last fetched, `{number, url}`, or None. Reads the cache only.
    pub fn link_for(&self, item: &WorkItem, branch: &str) -> Option<Map<String, Value>> {
        let cache = read_cache();
        let entry = cache.get(&format!("{}#{branch}", item.project().unwrap_or_default()))?.as_object()?;
        if matches!(entry.get("number"), None | Some(Value::Null) | Some(Value::Bool(false))) {
            return None;
        }
        let mut out = Map::new();
        for k in ["number", "url"] {
            if let Some(v) = entry.get(k) {
                out.insert(k.into(), v.clone());
            }
        }
        Some(out)
    }

    fn serve_stale(&self, item: &WorkItem, branch: &str, key: &str, entry: Option<&Value>) -> Option<String> {
        SERVED_STALE.store(true, Ordering::SeqCst);
        let first = {
            let mut flying = IN_FLIGHT.lock().unwrap();
            if flying.iter().any(|k| k == key) { false } else { flying.push(key.to_string()); true }
        };
        if first {
            let (key, branch, project, now) = (key.to_string(), branch.to_string(), item.project(), self.now);
            std::thread::spawn(move || {
                let me = PrStatus { now, config: ProjectConfig::current() };
                let fetched = me.fetch_in(project.as_deref(), &branch);
                me.store(&key, fetched);
                IN_FLIGHT.lock().unwrap().retain(|k| *k != key);
            });
        }
        entry.and_then(|e| e.get("state")).and_then(Value::as_str).map(String::from)
    }

    fn fetch_via_gh(&self, item: &WorkItem, branch: &str) -> Option<Value> {
        self.fetch_in(item.project().as_deref(), branch)
    }

    fn fetch_in(&self, project: Option<&str>, branch: &str) -> Option<Value> {
        let root = self.root_for(project)?;
        let out = Command::new("gh")
            .args(["pr", "view", branch, "--json", "state,number,url"])
            .current_dir(root)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        serde_json::from_slice(&out.stdout).ok()
    }

    fn store(&self, key: &str, fetched: Option<Value>) -> Option<String> {
        let mut pr = Map::new();
        match &fetched {
            Some(Value::Object(o)) => {
                for k in ["state", "number", "url"] {
                    if let Some(v) = o.get(k) {
                        pr.insert(k.into(), v.clone());
                    }
                }
            }
            Some(other) => {
                pr.insert("state".into(), other.clone());
            }
            None => {
                pr.insert("state".into(), Value::Null);
            }
        }
        pr.insert("ts".into(), Value::from(self.now));
        let state = pr.get("state").and_then(Value::as_str).map(String::from);
        let _guard = CACHE_LOCK.lock().unwrap();
        let mut cache = read_cache();
        cache.insert(key.to_string(), Value::Object(pr));
        write_cache(&cache);
        state
    }
}
