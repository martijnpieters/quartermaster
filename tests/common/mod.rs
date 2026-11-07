#![allow(dead_code)]
//! Helpers shared by the black-box test files.

use regex::Regex;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::TempDir;

/// The data root: a temp dir the helper owns, or an existing directory it only points at.
pub struct Root {
    _owned: Option<TempDir>,
    path: PathBuf,
}

impl Root {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub struct Qm {
    pub root: Root,
}

pub struct Out {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

pub fn bin() -> PathBuf {
    match std::env::var("QM_BIN") {
        Ok(p) => PathBuf::from(p),
        Err(_) => Path::new(env!("CARGO_MANIFEST_DIR")).join("quartermaster"),
    }
}

/// Dates, timestamps and pids vary per run; everything else is compared verbatim.
pub fn normalize(s: &str) -> String {
    let ts = Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2})?(\.\d+)?(Z|[+-]\d{2}:\d{2})?").unwrap();
    let date = Regex::new(r"\d{4}-\d{2}-\d{2}").unwrap();
    let pid = Regex::new(r#""pid":\d+"#).unwrap();
    let dur = Regex::new(r#""duration_s":\d+"#).unwrap();
    let s = ts.replace_all(s, "TS");
    let s = date.replace_all(&s, "DATE");
    let s = pid.replace_all(&s, r#""pid":PID"#);
    dur.replace_all(&s, r#""duration_s":N"#).into_owned()
}

impl Qm {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        Qm { root: Root { path: dir.path().to_path_buf(), _owned: Some(dir) } }
    }

    /// Points at an existing data root without owning it.
    pub fn at(path: &Path) -> Self {
        Qm { root: Root { _owned: None, path: path.to_path_buf() } }
    }

    pub fn run_with(&self, args: &[&str], stdin: &str, env: &[(&str, &str)]) -> Out {
        let mut cmd = Command::new(bin());
        cmd.args(args)
            .env("QM_ROOT", self.root.path())
            .env("LANG", "C.UTF-8")
            .env("LC_ALL", "C.UTF-8")
            .env("QM_TMUX_SOCKET", "qm-test-characterization")
            .env_remove("QM_SKIP")
            .env_remove("QM_ASIDE")
            .env_remove("QM_LOOSE")
            .env_remove("QM_ITEM")
            .current_dir(self.root.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("spawn qm");
        let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes()); // a hook that exits unread closes the pipe
        let o = child.wait_with_output().unwrap();
        Out {
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            code: o.status.code().unwrap_or(-1),
        }
    }

    pub fn run(&self, args: &[&str]) -> Out {
        self.run_with(args, "", &[])
    }

    pub fn ok(&self, args: &[&str]) -> String {
        let o = self.run(args);
        assert_eq!(o.code, 0, "qm {args:?} failed: {}", o.stderr);
        o.stdout
    }

    pub fn read(&self, rel: &str) -> String {
        normalize(&std::fs::read_to_string(self.root.path().join(rel)).unwrap_or_default())
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.root.path().join(rel).exists()
    }
}

impl Qm {
    /// Output with the data root, git hashes and the per-run temp dirs replaced by stable markers.
    pub fn norm(&self, s: &str) -> String {
        let root = self.root.path().to_string_lossy().into_owned();
        let sha = Regex::new(r"\b[0-9a-f]{7,40}\b").unwrap();
        let s = normalize(&s.replace(&root, "ROOT"));
        sha.replace_all(&s, "SHA").into_owned()
    }
}

/// Compares against `tests/fixtures/golden/<name>`; `QM_UPDATE_GOLDEN=1` rewrites it.
pub fn golden(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/golden").join(name);
    if std::env::var("QM_UPDATE_GOLDEN").is_ok_and(|v| !v.is_empty()) {
        std::fs::write(&path, actual).unwrap();
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing golden {name}"));
    // a Ruby backtrace names the launcher's line, which moves whenever the launcher does
    let frame = Regex::new(r"/quartermaster:\d+:in `<main>'").unwrap();
    let (actual, expected) = (frame.replace_all(actual, "/quartermaster:LINE:in `<main>'"), frame.replace_all(&expected, "/quartermaster:LINE:in `<main>'"));
    assert_eq!(actual, expected, "golden {name} differs");
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "user.name=t", "-c", "user.email=t@t.test"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

pub fn repo(dir: &Path, branch: &str) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", branch]);
    std::fs::write(dir.join("a.txt"), "a\n").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "first"]);
}

pub fn payload(id: &str, cwd: &Path) -> String {
    format!(r#"{{"session_id":"{id}","cwd":"{}","source":"startup","transcript_path":"/nonexistent/x.jsonl"}}"#, cwd.display())
}

pub fn strip_ansi(s: &str) -> String {
    Regex::new("\x1b\\[[0-9;]*m").unwrap().replace_all(s, "").into_owned()
}

pub type Step<'a> = (&'a [&'a str], &'a str);

pub fn scrub(qm: &Qm, s: &str) -> String {
    normalize(&s.replace(qm.root.path().to_string_lossy().as_ref(), "ROOT"))
}

pub fn transcript(qm: &Qm, watch: &str, steps: &[Step]) -> String {
    transcript_env(qm, watch, steps, &[])
}

pub fn transcript_env(qm: &Qm, watch: &str, steps: &[Step], env: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (args, stdin) in steps {
        let o = qm.run_with(args, stdin, env);
        let shown: Vec<String> = args.iter().map(|a| format!("{a:?}")).collect();
        out.push_str(&format!("$ qm {}\nexit {}\n", shown.join(" "), o.code));
        for (label, text) in [("stdout", &o.stdout), ("stderr", &o.stderr)] {
            if !text.is_empty() {
                out.push_str(&format!("--- {label}\n{text}"));
                if !text.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
        if let Ok(file) = std::fs::read_to_string(qm.root.path().join(format!("work-items/{watch}.md"))) {
            out.push_str(&format!("--- {watch}.md\n{file}"));
        }
        out.push('\n');
    }
    scrub(qm, &out)
}


// ------------------------------------------------------------ the attention world

pub struct AttentionWorld {
    pub root: PathBuf,
    pub home: PathBuf,
    pub transcripts: PathBuf,
    pub bin: PathBuf,
    pub gh_data: PathBuf,
    pub repo: PathBuf,
}

fn days_ago(now: std::time::SystemTime, days: f64) -> std::time::SystemTime {
    now - std::time::Duration::from_secs_f64(days * 86_400.0)
}

fn iso(now: std::time::SystemTime, days: f64) -> String {
    let t: chrono::DateTime<chrono::Utc> = days_ago(now, days).into();
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

fn touch(path: &Path, when: std::time::SystemTime) {
    let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.set_modified(when).unwrap();
}

fn put(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A data root exercising every attention rule: expiring transcripts, quiet items, snoozes and PR
/// states behind a fake `gh`. `base` must be an existing empty directory; `now` anchors every time.
pub fn attention_world(base: &Path, now: std::time::SystemTime, retention_days: Option<u32>) -> AttentionWorld {
    let w = AttentionWorld {
        root: base.join("root"),
        home: base.join("home"),
        transcripts: base.join("transcripts"),
        bin: base.join("bin"),
        gh_data: base.join("gh-data"),
        repo: base.join("repo"),
    };
    repo(&w.repo, "main");
    git(&w.repo, &["branch", "nopr-exists"]);
    git(&w.repo, &["branch", "merged-b"]);
    put(&w.bin.join("gh"), "#!/bin/sh\necho \"$3\" >> \"$FAKE_GH_LOG\"\nf=\"$FAKE_GH_DATA/$(echo \"$3\" | tr '/' '_')\"\nif [ -f \"$f\" ]; then cat \"$f\"; exit 0; fi\nexit 1\n");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(w.bin.join("gh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    for (branch, state, n) in [("b-merged", "MERGED", 1), ("s1", "MERGED", 2), ("s2", "MERGED", 3), ("m1", "MERGED", 4), ("m2", "OPEN", 5), ("merged-b", "MERGED", 6), ("c-open-stale", "MERGED", 8), ("feat_x", "MERGED", 9)] {
        put(&w.gh_data.join(branch), &format!("{{\"number\":{n},\"state\":\"{state}\",\"url\":\"https://example.test/pull/{n}\"}}"));
    }
    if let Some(days) = retention_days {
        put(&w.home.join(".claude/settings.json"), &format!("{{\"cleanupPeriodDays\":{days}}}"));
    } else {
        std::fs::create_dir_all(&w.home).unwrap();
    }
    put(&w.root.join("config.yml"), &format!("projects:\n  demo: {}\n", w.repo.display()));
    let me = std::process::id();

    // sessions.jsonl: each session, with the transcript it points at
    let mut log = String::new();
    let mut session = |id: &str, age_days: f64, pid: u32, transcript_age: Option<f64>| {
        let path = w.transcripts.join(format!("p/{id}.jsonl"));
        if let Some(age) = transcript_age {
            put(&path, "{}\n");
            touch(&path, days_ago(now, age));
        }
        log += &format!(
            "{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"/srv/app\",\"pid\":{pid},\"transcript_path\":\"{}\",\"ts\":\"{}\"}}\n",
            path.display(),
            iso(now, age_days)
        );
    };
    session("d-soon", 27.0, 999999, Some(26.5));
    session("d-negative", 33.0, 999999, Some(32.5));
    session("d-fresh", 0.5, 999999, Some(0.5));
    session("d-acked", 28.0, 999999, Some(27.0));
    session("d-snap", 28.0, 999999, Some(27.0));
    session("d-live", 28.0, me, Some(27.0));
    session("d-notranscript", 28.0, 999999, None);
    session("q-recent", 2.0, 999999, Some(1.0));
    session("t-ten", 15.0, 999999, Some(15.0));
    put(&w.root.join("sessions.jsonl"), &log);

    let item = |slug: &str, state: &str, project: &str, extra: &str, sessions: &[&str], body_extra: &str, age_days: f64| {
        let list = if sessions.is_empty() { String::new() } else { format!("sessions:\n{}", sessions.iter().map(|s| if *s == "d-acked" { format!("- id: {s}\n  ack_degrade: true\n") } else { format!("- id: {s}\n") }).collect::<String>()) };
        let path = w.root.join(format!("work-items/{slug}.md"));
        put(&path, &format!("---\nv: 1\ntitle: {slug}\nstate: {state}\nproject: {project}\ncreated: '2026-01-01'\n{extra}{list}---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n{body_extra}"));
        touch(&path, days_ago(now, age_days));
    };
    item("degrade", "in_progress", "demo", "", &["d-soon", "d-negative", "d-fresh", "d-acked", "d-snap", "d-live", "d-notranscript", "d-nolog"], "\n### 2026-01-02 — session d-snap\n\ntext\n", 0.1);
    item("stale", "in_progress", "demo", "", &[], "", 11.5);
    item("stale-ten-days", "in_progress", "demo", "", &["t-ten"], "", 30.0);
    item("quiet-sessioned", "in_progress", "demo", "", &["q-recent"], "", 20.0);
    item("snoozed-future", "in_progress", "demo", "snooze_until: '2099-01-01'\n", &[], "", 30.0);
    item("snoozed-past", "in_progress", "demo", "snooze_until: '2020-01-01'\n", &[], "", 30.0);
    item("snoozed-garbage", "in_progress", "demo", "snooze_until: not a date\n", &[], "", 30.0);
    item("fridge-old", "fridge", "demo", "", &[], "", 40.0);
    item("resolved-old", "resolved", "demo", "", &[], "", 40.0);
    item("pr-one-merged", "in_progress", "demo", "branches:\n- b-merged\n", &[], "", 0.1);
    item("pr-stack-merged", "in_progress", "demo", "branches:\n- s1\n- s2\n", &[], "", 0.1);
    item("pr-stack-mixed", "in_progress", "demo", "branches:\n- m1\n- m2\n", &[], "", 0.1);
    item("pr-none-gone", "in_progress", "demo", "branches:\n- nopr\n", &[], "", 0.1);
    item("pr-none-exists", "in_progress", "demo", "branches:\n- nopr-exists\n", &[], "", 0.1);
    item("pr-merged-plus-gone", "in_progress", "demo", "branches:\n- merged-b\n- renamed-old\n", &[], "", 0.1);
    item("pr-cached", "in_progress", "demo", "branches:\n- c-merged\n", &[], "", 0.1);
    item("pr-cached-stale-open", "in_progress", "demo", "branches:\n- c-open-stale\n", &[], "", 0.1);
    item("pr-no-project", "in_progress", "elsewhere", "branches:\n- b-merged\n", &[], "", 0.1);
    item("pr-slash-branch", "in_progress", "demo", "branches:\n- feat/x\n", &[], "", 0.1);
    item("pr-resolved", "resolved", "demo", "branches:\n- b-merged\n", &[], "", 0.1);
    let now_secs = now.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    put(
        &w.root.join("pr-cache.json"),
        &format!(
            "{{\"demo#c-merged\":{{\"state\":\"MERGED\",\"number\":7,\"url\":\"https://example.test/pull/7\",\"ts\":{now_secs}}},\"demo#c-open-stale\":{{\"state\":\"OPEN\",\"number\":8,\"url\":\"u8\",\"ts\":{}}}}}",
            now_secs - 400
        ),
    );
    w
}
