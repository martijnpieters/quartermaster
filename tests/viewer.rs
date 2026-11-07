//! The fzf viewer's read-side commands (`viewer-rows`, `session-rows`, `inbox-rows`, `cycle-filter`)
//! over the attention world plus an inbox, as transcripts generated from Ruby.

mod common;
use common::*;
use std::fs;
use std::time::SystemTime;

fn world_steps(steps: &[Step], prepare: impl Fn(&AttentionWorld)) -> String {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), SystemTime::now(), None);
    prepare(&w);
    let qm = Qm::at(&w.root);
    let home = w.home.display().to_string();
    let transcripts = w.transcripts.display().to_string();
    let path = format!("{}:{}", w.bin.display(), std::env::var("PATH").unwrap());
    let (gh_data, gh_log) = (w.gh_data.display().to_string(), base.path().join("gh.log").display().to_string());
    let env = [("HOME", home.as_str()), ("QM_TRANSCRIPTS_DIR", transcripts.as_str()), ("PATH", path.as_str()), ("FAKE_GH_DATA", gh_data.as_str()), ("FAKE_GH_LOG", gh_log.as_str())];
    transcript_env(&qm, "degrade", steps, &env).replace(&base.path().display().to_string(), "BASE")
}

fn unclaimed_sessions(w: &AttentionWorld) {
    let now = chrono::Utc::now();
    let ts = |mins: i64| (now - chrono::Duration::minutes(mins)).to_rfc3339_opts(chrono::SecondsFormat::Secs, false);
    let mut log = fs::read_to_string(w.root.join("sessions.jsonl")).unwrap();
    let prompts: [(&str, &str, i64, Option<&str>); 6] = [
        ("u-plain", "/srv/app", 50, Some("fix the login bug")),
        ("u-multiline", "/srv/other/deeper/than/thirty/four/characters/path", 40, Some("  first line\n\tsecond\t\tline\r\n\n  third  ")),
        ("u-long", "/srv/app", 30, Some("0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789 0123456789")),
        ("u-unicode", "/srv/ünï", 20, Some("héllo wörld — ünïcode 🙂 prompt that keeps going on and on and on and on")),
        ("u-noprompt", "/srv/app", 10, None),
        ("u-dismissed", "/srv/app", 5, Some("dismissed one")),
    ];
    for (id, cwd, mins, prompt) in prompts {
        let path = w.transcripts.join(format!("p/{id}.jsonl"));
        if let Some(p) = prompt {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, format!("{}\n", serde_json::json!({"type": "user", "message": {"content": p}}))).unwrap();
        }
        log += &format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"{cwd}\",\"pid\":999999,\"transcript_path\":\"{}\",\"ts\":\"{}\"}}\n", path.display(), ts(mins));
    }
    log += &format!("{{\"v\":1,\"event\":\"dismissed\",\"session_id\":\"u-dismissed\",\"ts\":\"{}\"}}\n", ts(1));
    fs::write(w.root.join("sessions.jsonl"), log).unwrap();
}

#[test]
fn viewer_rows_rank_items_with_attention_first_then_by_activity() {
    let steps: Vec<Step> = vec![
        (&["viewer-rows"], ""),
        (&["viewer-rows", "--state", "all"], ""),
        (&["viewer-rows", "--state", "in_progress"], ""),
        (&["viewer-rows", "--state", "fridge"], ""),
        (&["viewer-rows", "--state", "resolved"], ""),
        (&["viewer-rows", "--state", "open"], ""),
        (&["viewer-rows", "--state", "bogus"], ""),
        (&["viewer-rows", "ignored", "--state", "fridge", "positional"], ""),
    ];
    // two sessions of one item both nearing expiry: the same warning twice shows once
    let twins = |w: &AttentionWorld| {
        let mut log = fs::read_to_string(w.root.join("sessions.jsonl")).unwrap();
        for id in ["twin-a", "twin-b"] {
            let path = w.transcripts.join(format!("p/{id}.jsonl"));
            fs::write(&path, "{}\n").unwrap();
            let when = SystemTime::now() - std::time::Duration::from_secs_f64(27.0 * 86_400.0);
            fs::OpenOptions::new().write(true).open(&path).unwrap().set_modified(when).unwrap();
            log += &format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"/srv/app\",\"pid\":999999,\"transcript_path\":\"{}\",\"ts\":\"2026-01-01T00:00:00+00:00\"}}\n", path.display());
        }
        fs::write(w.root.join("sessions.jsonl"), log).unwrap();
        fs::write(w.root.join("work-items/twins.md"), "---\nv: 1\ntitle: A title that is a good deal longer than forty-four characters wide ünï\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\nsessions:\n- id: twin-a\n- id: twin-b\n---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n").unwrap();
    };
    golden("viewer_rows.txt", &world_steps(&steps, twins));
}

#[test]
fn the_filter_cycles_through_its_states_and_survives_garbage() {
    let steps: Vec<Step> = vec![
        (&["cycle-filter"], ""),
        (&["viewer-rows"], ""),
        (&["cycle-filter"], ""),
        (&["viewer-rows"], ""),
        (&["cycle-filter"], ""),
        (&["cycle-filter"], ""),
        (&["viewer-rows"], ""),
        (&["cycle-filter"], ""),
        (&["viewer-rows"], ""),
    ];
    let text = world_steps(&steps, |_| {});
    golden("viewer_filter_cycle.txt", &text);
    for garbage in ["nonsense\n", "", "  fridge  \n", "ALL"] {
        let one: Vec<Step> = vec![(&["viewer-rows"], ""), (&["cycle-filter"], ""), (&["viewer-rows"], "")];
        let out = world_steps(&one, |w| fs::write(w.root.join("viewer-filter"), garbage).unwrap());
        golden(&format!("viewer_filter_{}.txt", garbage.trim().to_lowercase().replace(' ', "_").replace("", "")).replace("viewer_filter_.txt", "viewer_filter_empty.txt"), &out);
    }
}

#[test]
fn session_rows_and_inbox_rows_list_what_the_fzf_views_show() {
    let steps: Vec<Step> = vec![
        (&["session-rows", "degrade"], ""),
        (&["session-rows", "stale"], ""),
        (&["session-rows", "quiet-sessioned"], ""),
        (&["session-rows", "ghost"], ""),
        (&["session-rows"], ""),
        (&["inbox-rows"], ""),
        (&["dismiss", "u-plain"], ""),
        (&["inbox-rows"], ""),
    ];
    golden("viewer_session_inbox_rows.txt", &world_steps(&steps, unclaimed_sessions));
}
