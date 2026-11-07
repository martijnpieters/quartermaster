//! Black-box characterization of the `qm` executable.
//!
//! The suite drives whichever executable `QM_BIN` names (default: the Ruby
//! launcher `./quartermaster`) against a throwaway `QM_ROOT`, so the same
//! assertions pin the Ruby behavior today and the Rust port tomorrow.

mod common;
use common::*;
use regex::Regex;

// ---------------------------------------------------------------- work items

#[test]
fn new_prints_the_slug_and_writes_a_versioned_item() {
    let qm = Qm::new();
    assert_eq!(qm.ok(&["new", "Fix login bug", "--description", "desc here"]), "fix-login-bug\n");
    assert_eq!(
        qm.read("work-items/fix-login-bug.md"),
        "---\nv: 1\ntitle: Fix login bug\nstate: in_progress\nproject: No project\ncreated: 'DATE'\n---\n\n## Description\n\ndesc here\n\n## Notes\n\n## Snapshots\n"
    );
}

#[test]
fn new_seeds_the_data_root() {
    let qm = Qm::new();
    qm.ok(&["new", "Seed"]);
    assert!(qm.exists("config.yml"));
    assert!(qm.exists("work-items"));
}

#[test]
fn new_with_session_project_and_branch_records_them() {
    let qm = Qm::new();
    qm.ok(&["new", "Linked", "--project", "demo", "--session", "s9", "--branch", "feat/x"]);
    let item = qm.read("work-items/linked.md");
    assert!(item.contains("project: demo"), "{item}");
    assert!(item.contains("- id: s9"), "{item}");
    assert!(item.contains("feat/x"), "{item}");
}

#[test]
fn a_repeated_title_is_refused() {
    let qm = Qm::new();
    assert_eq!(qm.ok(&["new", "Same"]), "same\n");
    let again = qm.run(&["new", "Same"]);
    assert_eq!((again.code, again.stdout.as_str()), (1, ""));
    assert_eq!(again.stderr, "qm: work item 'same' already exists\n");
}

#[test]
fn note_appends_with_a_kind_and_prints_the_numbered_notes() {
    let qm = Qm::new();
    qm.ok(&["new", "Fix login bug"]);
    qm.ok(&["note", "fix-login-bug", "first note", "--kind", "decision"]);
    let out = qm.ok(&["note", "fix-login-bug", "hist", "--kind", "history"]);
    assert_eq!(normalize(&strip_ansi(&out)), "#2 [history] DATE: hist\n");
    let item = qm.read("work-items/fix-login-bug.md");
    assert!(item.contains("- DATE: first note\n- DATE [history]: hist\n"), "{item}");
}

#[test]
fn retag_changes_a_notes_kind() {
    let qm = Qm::new();
    qm.ok(&["new", "T"]);
    qm.ok(&["note", "t", "n"]);
    qm.ok(&["retag", "t", "1", "open"]);
    assert!(qm.read("work-items/t.md").contains("[open]"));
}

#[test]
fn state_changes_and_rejects_unknown_states() {
    let qm = Qm::new();
    qm.ok(&["new", "T"]);
    qm.ok(&["state", "t", "fridge"]);
    assert!(qm.read("work-items/t.md").contains("state: fridge"));
    let bad = qm.run(&["state", "t", "bogus"]);
    assert_eq!(bad.code, 1);
    assert_eq!(bad.stderr, "qm: state must be one of: in_progress, fridge, resolved\n");
}

#[test]
fn unknown_slug_is_an_error_not_a_backtrace() {
    let qm = Qm::new();
    let o = qm.run(&["show", "nope"]);
    assert_eq!(o.code, 1);
    assert_eq!(o.stderr, "qm: no work item 'nope'\n");
    assert_eq!(o.stdout, "");
}

#[test]
fn retitle_keeps_the_slug_and_move_changes_the_project() {
    let qm = Qm::new();
    qm.ok(&["new", "Fix login bug"]);
    assert_eq!(qm.ok(&["retitle", "fix-login-bug", "Fix login"]), "fix-login-bug: Fix login\n");
    qm.ok(&["move", "fix-login-bug", "other"]);
    let item = qm.read("work-items/fix-login-bug.md");
    assert!(item.contains("title: Fix login\n") && item.contains("project: other\n"), "{item}");
}

#[test]
fn describe_replaces_the_description_and_reads_stdin() {
    let qm = Qm::new();
    qm.ok(&["new", "T", "--description", "old"]);
    qm.run_with(&["describe", "t", "-"], "from stdin\n", &[]);
    let item = qm.read("work-items/t.md");
    assert!(item.contains("## Description\n\nfrom stdin\n"), "{item}");
    assert!(!item.contains("old"), "{item}");
}

#[test]
fn attach_moves_a_session_between_items_and_detach_unlinks_it() {
    let qm = Qm::new();
    qm.ok(&["new", "A", "--session", "s1"]);
    qm.ok(&["new", "B"]);
    qm.ok(&["attach", "b", "--session", "s1"]);
    assert!(qm.read("work-items/a.md").contains("sessions: []"));
    assert!(qm.read("work-items/b.md").contains("- id: s1"));
    qm.ok(&["detach", "b", "--session", "s1"]);
    assert!(qm.read("work-items/b.md").contains("sessions: []"));
}

#[test]
fn link_and_unlink_branch() {
    let qm = Qm::new();
    qm.ok(&["new", "T"]);
    qm.ok(&["link-branch", "t", "feat/a"]);
    assert!(qm.read("work-items/t.md").contains("feat/a"));
    qm.ok(&["unlink-branch", "t", "feat/a"]);
    assert!(!qm.read("work-items/t.md").contains("feat/a"));
}

#[test]
fn board_prints_only_the_url_and_fails_when_unset() {
    let qm = Qm::new();
    qm.ok(&["new", "T"]);
    let unset = qm.run(&["board", "t"]);
    assert_ne!(unset.code, 0);
    assert_eq!(unset.stdout, "");
    qm.ok(&["board", "t", "https://example.test/board"]);
    assert_eq!(qm.ok(&["board", "t"]), "https://example.test/board\n");
    qm.ok(&["board", "t", "clear"]);
    assert_ne!(qm.run(&["board", "t"]).code, 0);
}

#[test]
fn list_is_a_table_of_state_project_slug_and_counts() {
    let qm = Qm::new();
    qm.ok(&["new", "Fix login bug"]);
    qm.ok(&["note", "fix-login-bug", "n"]);
    qm.ok(&["state", "fix-login-bug", "fridge"]);
    qm.ok(&["attach", "fix-login-bug", "--session", "s1"]);
    assert_eq!(qm.ok(&["list"]), "fridge  No project  fix-login-bug  1  -\n");
}

#[test]
fn show_prints_title_notes_sessions_usage_and_the_item_body() {
    let qm = Qm::new();
    qm.ok(&["new", "Fix login", "--description", "desc here"]);
    qm.ok(&["note", "fix-login", "first note"]);
    qm.ok(&["attach", "fix-login", "--session", "s1"]);
    let out = normalize(&strip_ansi(&qm.ok(&["show", "fix-login"])));
    assert!(out.starts_with("Fix login\nin_progress · No project · created DATE\n#1 [decision] DATE: first note\n"), "{out}");
    assert!(out.contains("Sessions\ns1 ? uncaptured"), "{out}");
    assert!(out.contains("## Description\n\ndesc here\n"), "{out}");
}

// ------------------------------------------------------------- capture hooks

fn start_payload(id: &str) -> String {
    format!(r#"{{"session_id":"{id}","cwd":"/tmp","source":"startup","transcript_path":"/nonexistent/x.jsonl"}}"#)
}

#[test]
fn session_start_logs_an_event_and_prints_the_linking_protocol() {
    let qm = Qm::new();
    qm.ok(&["new", "T"]);
    let o = qm.run_with(&["hook-session-start"], &start_payload("abc"), &[]);
    assert_eq!(o.code, 0);
    assert!(o.stdout.starts_with("[qm] This Claude Code session (abc) is not linked to a work item. Project baseline from cwd: No project.\n\nOpen work items:\n- t — T (No project, in_progress)\n"), "{}", o.stdout);
    assert!(o.stdout.contains("qm attach <slug> --session abc"));
    let log = qm.read("sessions.jsonl");
    assert_eq!(
        log,
        "{\"v\":1,\"event\":\"start\",\"session_id\":\"abc\",\"source\":\"startup\",\"cwd\":\"/tmp\",\"transcript_path\":\"/nonexistent/x.jsonl\",\"branch\":null,\"head\":null,\"pid\":PID,\"project\":\"No project\",\"work_item\":null,\"ts\":\"TS\"}\n"
    );
}

#[test]
fn session_start_with_qm_item_attaches_the_session() {
    let qm = Qm::new();
    qm.ok(&["new", "T"]);
    let o = qm.run_with(&["hook-session-start"], &start_payload("abc"), &[("QM_ITEM", "t")]);
    assert_eq!(o.code, 0);
    assert!(qm.read("work-items/t.md").contains("- id: abc"));
    assert!(qm.read("sessions.jsonl").contains("\"work_item\":\"t\""));
}

#[test]
fn an_aside_is_logged_but_never_attached() {
    let qm = Qm::new();
    qm.ok(&["new", "T"]);
    let o = qm.run_with(&["hook-session-start"], &start_payload("as1"), &[("QM_ASIDE", "t"), ("QM_ITEM", "t")]);
    assert_eq!(o.code, 0);
    assert!(o.stdout.contains("is an aside about work item 't'"), "{}", o.stdout);
    assert!(!qm.read("work-items/t.md").contains("as1"));
    assert!(qm.read("sessions.jsonl").contains("\"aside\":\"t\""));
}

#[test]
fn a_loose_chat_is_logged_with_its_id_and_told_not_to_link() {
    let qm = Qm::new();
    let o = qm.run_with(&["hook-session-start"], &start_payload("l1"), &[("QM_LOOSE", "abc123")]);
    assert_eq!(o.code, 0);
    assert!(o.stdout.contains("is a loose chat, by intent"), "{}", o.stdout);
    assert!(qm.read("sessions.jsonl").contains("\"loose\":\"abc123\""));
}

#[test]
fn session_end_of_an_unused_session_dismisses_it() {
    let qm = Qm::new();
    qm.run_with(&["hook-session-start"], &start_payload("abc"), &[]);
    let o = qm.run_with(&["hook-session-end"], r#"{"session_id":"abc","cwd":"/tmp","reason":"exit"}"#, &[]);
    assert_eq!((o.code, o.stdout.as_str()), (0, ""));
    let log = qm.read("sessions.jsonl");
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 3, "{log}");
    assert!(lines[1].starts_with(r#"{"v":1,"event":"end","session_id":"abc","reason":"exit","duration_s":N,"commits":null,"diff_stat":null,"ts":"TS","empty":true}"#), "{}", lines[1]);
    assert_eq!(lines[2], r#"{"v":1,"event":"dismissed","session_id":"abc","ts":"TS"}"#);
}

#[test]
fn hooks_swallow_bad_input_and_honor_qm_skip() {
    let qm = Qm::new();
    let bad = qm.run_with(&["hook-session-start"], r#"{"session_id":"x""#, &[]);
    assert_eq!((bad.code, bad.stdout.as_str()), (0, ""));
    let skipped = qm.run_with(&["hook-session-start"], &start_payload("abc"), &[("QM_SKIP", "1")]);
    assert_eq!((skipped.code, skipped.stdout.as_str()), (0, ""));
    assert!(!qm.exists("sessions.jsonl"));
}

#[test]
fn sessions_and_inbox_reflect_the_log() {
    let qm = Qm::new();
    assert_eq!(qm.ok(&["inbox"]), "Inbox empty — every captured session is linked to a work item or dismissed.\n");
    qm.run_with(&["hook-session-start"], &start_payload("abc"), &[]);
    let inbox = normalize(&qm.ok(&["inbox"]));
    assert!(inbox.contains("abc") && inbox.contains("/tmp"), "{inbox}");
    qm.ok(&["dismiss", "abc"]);
    assert!(qm.ok(&["inbox"]).starts_with("Inbox empty"));
}

#[test]
fn session_end_after_a_start_marks_the_session_ended() {
    let qm = Qm::new();
    qm.run_with(&["hook-session-start"], &start_payload("abc"), &[]);
    qm.run_with(&["hook-session-end"], r#"{"session_id":"abc","cwd":"/tmp","reason":"exit"}"#, &[]);
    let out = Regex::new(r"  \d+s\n$").unwrap().replace(&normalize(&qm.ok(&["sessions"])), "  Ns\n").into_owned();
    assert_eq!(out, "  ended  abc  -  /tmp  -  TS  Ns\n");
}

// ------------------------------------------------------------------- the CLI

#[test]
fn unknown_command_prints_an_error_and_the_usage_and_exits_1() {
    let qm = Qm::new();
    let o = qm.run(&["bogus"]);
    assert_eq!(o.code, 1);
    assert_eq!(o.stderr, "qm: unknown command 'bogus'\n");
    assert!(o.stdout.starts_with("qm — Claude Code session & work-item tracker\n\nViewer:\n"), "{}", o.stdout);
}

#[test]
fn help_lists_every_documented_command() {
    let qm = Qm::new();
    let help = qm.ok(&["help"]);
    for word in [
        "qm new TITLE", "qm attach SLUG", "qm detach SLUG", "qm note SLUG", "qm retag SLUG", "qm state SLUG",
        "qm link-branch SLUG", "qm board SLUG", "qm move SLUG", "qm retitle SLUG", "qm describe SLUG",
        "qm show SLUG", "qm list", "qm attention", "qm inbox", "qm hook-session-start", "qm install-hooks",
    ] {
        assert!(help.contains(word), "help lacks {word}");
    }
}

#[test]
fn install_hooks_prints_a_settings_snippet_and_writes_nothing() {
    let qm = Qm::new();
    let home = tempfile::tempdir().unwrap();
    let o = qm.run_with(&["install-hooks"], "", &[("HOME", home.path().to_str().unwrap())]);
    assert_eq!(o.code, 0);
    assert!(o.stdout.contains("hook-session-start") && o.stdout.contains("hook-session-end"), "{}", o.stdout);
    assert!(!home.path().join(".claude").join("settings.json").exists());
}

#[test]
fn every_qm_command_appends_to_the_feature_use_log_without_arguments() {
    let qm = Qm::new();
    qm.ok(&["new", "Secret title"]);
    qm.ok(&["list"]);
    let log = qm.read("feature-use.jsonl");
    assert!(log.contains("\"kind\":\"cli\"") || log.contains("\"kind\": \"cli\""), "{log}");
    assert!(!log.contains("Secret") && !log.contains("secret-title"), "{log}");
}

// ------------------------------------------- hook context (golden, from Ruby)

fn busy_item(qm: &Qm) {
    qm.ok(&["new", "Busy item", "--project", "demo", "--description", "The current truth."]);
    qm.ok(&["link-branch", "busy-item", "feat/busy"]);
    qm.ok(&["board", "busy-item", "https://example.test/b"]);
    for (i, kind) in ["open", "decision", "history", "decision", "open", "history", "decision", "decision", "open", "decision"].iter().enumerate() {
        qm.ok(&["note", "busy-item", &format!("note number {i}"), "--kind", kind]);
    }
    qm.ok(&["attach", "busy-item", "--session", "old1"]);
}

#[test]
fn linked_context_digests_the_item_and_windows_its_notes() {
    let qm = Qm::new();
    busy_item(&qm);
    let o = qm.run_with(&["hook-session-start"], &start_payload("new1"), &[("QM_ITEM", "busy-item")]);
    assert_eq!(o.code, 0);
    golden("linked_context.txt", &qm.norm(&o.stdout));
    assert!(qm.read("work-items/busy-item.md").contains("- id: new1"));
}

#[test]
fn linked_context_names_the_dev_server_ide_and_pr_prerequisites_when_configured() {
    let qm = Qm::new();
    std::fs::write(
        qm.root.path().join("config.yml"),
        "projects:\n  demo:\n    root: /nonexistent/demo\n    dev_server:\n      command: devsrv\n      modes: [dev, tunnel]\n      default_mode: tunnel\n    pr_prerequisites: true\n    ide_command: [code, PATH]\n    ide_label: Code\n",
    )
    .unwrap();
    qm.ok(&["new", "Configured", "--project", "demo"]);
    let o = qm.run_with(&["hook-session-start"], &start_payload("c1"), &[("QM_ITEM", "configured")]);
    golden("linked_context_configured.txt", &qm.norm(&o.stdout));
}

#[test]
fn unlinked_context_lists_open_items_with_project_branches_and_state() {
    let qm = Qm::new();
    qm.ok(&["new", "Alpha", "--project", "demo", "--branch", "feat/a"]);
    qm.ok(&["new", "Beta"]);
    qm.ok(&["state", "beta", "fridge"]);
    qm.ok(&["new", "Gamma"]);
    qm.ok(&["state", "gamma", "resolved"]);
    let o = qm.run_with(&["hook-session-start"], &start_payload("u1"), &[]);
    golden("unlinked_context.txt", &qm.norm(&o.stdout));
}

#[test]
fn aside_context_carries_the_items_digest() {
    let qm = Qm::new();
    busy_item(&qm);
    let o = qm.run_with(&["hook-session-start"], &start_payload("as1"), &[("QM_ASIDE", "busy-item")]);
    golden("aside_context.txt", &qm.norm(&o.stdout));
    let unknown = qm.run_with(&["hook-session-start"], &start_payload("as2"), &[("QM_ASIDE", "missing")]);
    assert_eq!((unknown.code, unknown.stdout.as_str()), (0, ""));
    assert!(qm.read("sessions.jsonl").contains("\"session_id\":\"as2\""));
}

#[test]
fn a_promoted_loose_chat_follows_the_item_it_became() {
    let qm = Qm::new();
    qm.run_with(&["hook-session-start"], &start_payload("l1"), &[("QM_LOOSE", "L")]);
    qm.ok(&["new", "Promoted", "--session", "l1"]);
    let o = qm.run_with(&["hook-session-start"], &start_payload("l2"), &[("QM_LOOSE", "L")]);
    assert!(o.stdout.contains("linked to work item 'promoted'"), "{}", o.stdout);
    assert!(qm.read("work-items/promoted.md").contains("- id: l2"));
}

#[test]
fn an_already_attached_item_beats_qm_item_and_an_unknown_qm_item_falls_through() {
    let qm = Qm::new();
    qm.ok(&["new", "One", "--session", "s1"]);
    qm.ok(&["new", "Two"]);
    let o = qm.run_with(&["hook-session-start"], &start_payload("s1"), &[("QM_ITEM", "two")]);
    assert!(o.stdout.contains("linked to work item 'one'"), "{}", o.stdout);
    let o = qm.run_with(&["hook-session-start"], &start_payload("s2"), &[("QM_ITEM", "ghost")]);
    assert!(o.stdout.contains("is not linked to a work item"), "{}", o.stdout);
}

#[test]
fn a_session_on_a_linked_branch_in_a_configured_project_attaches_by_branch() {
    let qm = Qm::new();
    let work = tempfile::tempdir().unwrap();
    let dir = work.path().join("demo");
    repo(&dir, "feat/x");
    std::fs::write(qm.root.path().join("config.yml"), format!("projects:\n  demo: {}\n", dir.display())).unwrap();
    qm.ok(&["new", "Branchy", "--project", "demo", "--branch", "feat/x"]);
    qm.ok(&["new", "Other project", "--project", "elsewhere", "--branch", "feat/x"]);
    let o = qm.run_with(&["hook-session-start"], &payload("b1", &dir), &[]);
    assert!(o.stdout.contains("linked to work item 'branchy'"), "{}", o.stdout);
    let log = qm.norm(&qm.read("sessions.jsonl")).replace(&dir.display().to_string(), "DIR");
    assert!(log.contains("\"branch\":\"feat/x\",\"head\":\"SHA\""), "{log}");
    assert!(log.contains("\"project\":\"demo\",\"work_item\":\"branchy\""), "{log}");
}

#[test]
fn a_mainline_branch_links_nothing_and_a_worktree_branch_is_recorded_as_temporary() {
    let qm = Qm::new();
    let work = tempfile::tempdir().unwrap();
    let main = work.path().join("m");
    repo(&main, "main");
    qm.ok(&["new", "Item"]);
    let o = qm.run_with(&["hook-session-start"], &payload("m1", &main), &[]);
    assert!(o.stdout.contains("is not linked"), "{}", o.stdout);

    let wt = work.path().join("proj/.claude/worktrees/w1");
    repo(&wt, "worktree-w1");
    qm.run_with(&["hook-session-start"], &payload("w1", &wt), &[("QM_ITEM", "item")]);
    assert!(qm.read("work-items/item.md").contains("temp_branches:\n- worktree-w1\n"));
    let main_wt = work.path().join("proj2/.claude/worktrees/w2");
    repo(&main_wt, "main");
    qm.run_with(&["hook-session-start"], &payload("w2", &main_wt), &[("QM_ITEM", "item")]);
    assert!(!qm.read("work-items/item.md").contains("- main"));
}

#[test]
fn session_end_records_commits_and_the_diff_stat_and_keeps_a_used_session() {
    let qm = Qm::new();
    let work = tempfile::tempdir().unwrap();
    let dir = work.path().join("r");
    repo(&dir, "main");
    qm.ok(&["new", "Item", "--session", "e1"]);
    qm.run_with(&["hook-session-start"], &payload("e1", &dir), &[]);
    std::fs::write(dir.join("a.txt"), "a\nb\n").unwrap();
    git(&dir, &["commit", "-qam", "second change"]);
    std::fs::write(dir.join("a.txt"), "a\nb\nc\n").unwrap();
    let transcript = work.path().join("t.jsonl");
    std::fs::write(&transcript, "{}\n").unwrap();
    let end = format!(r#"{{"session_id":"e1","cwd":"{}","reason":"clear","transcript_path":"{}"}}"#, dir.display(), transcript.display());
    let o = qm.run_with(&["hook-session-end"], &end, &[]);
    assert_eq!((o.code, o.stdout.as_str()), (0, ""));
    let log = qm.norm(&qm.read("sessions.jsonl"));
    let end_line = log.lines().nth(1).unwrap();
    assert!(end_line.contains("\"reason\":\"clear\",\"duration_s\":N,\"commits\":[\"SHA second change\"],\"diff_stat\":\"1 file changed, 1 insertion(+)\""), "{end_line}");
    assert!(!end_line.contains("empty"));
    assert_eq!(log.lines().count(), 2, "a used session is not dismissed");
    assert!(qm.read("work-items/item.md").contains("- id: e1"));
}

#[test]
fn session_end_finds_the_transcript_by_session_id_under_the_transcripts_dir() {
    let qm = Qm::new();
    let projects = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(projects.path().join("proj")).unwrap();
    std::fs::write(projects.path().join("proj/t9.jsonl"), "{}\n").unwrap();
    let env = [("QM_TRANSCRIPTS_DIR", projects.path().to_str().unwrap())];
    qm.run_with(&["hook-session-start"], &start_payload("t9"), &env);
    qm.run_with(&["hook-session-end"], r#"{"session_id":"t9","cwd":"/tmp","reason":"exit"}"#, &env);
    assert!(!qm.read("sessions.jsonl").contains("empty"));
}

#[test]
fn an_empty_session_end_detaches_it_from_its_item() {
    let qm = Qm::new();
    qm.ok(&["new", "Item"]);
    qm.run_with(&["hook-session-start"], &start_payload("z1"), &[("QM_ITEM", "item")]);
    assert!(qm.read("work-items/item.md").contains("- id: z1"));
    qm.run_with(&["hook-session-end"], r#"{"session_id":"z1","cwd":"/tmp","reason":"exit"}"#, &[]);
    assert!(!qm.read("work-items/item.md").contains("z1"));
}

#[test]
fn a_hook_that_fails_logs_to_qm_log_and_stays_silent() {
    let qm = Qm::new();
    let o = qm.run_with(&["hook-session-start"], "not json", &[]);
    assert_eq!((o.code, o.stdout.as_str(), o.stderr.as_str()), (0, "", ""));
    assert!(qm.read("qm.log").contains("[hook-session-start] JSON::ParserError"), "{}", qm.read("qm.log"));
}

#[test]
fn sessions_list_newest_first_truncate_ids_and_honor_n() {
    let qm = Qm::new();
    qm.ok(&["new", "Item"]);
    qm.run_with(&["hook-session-start"], &start_payload("sess-one-1"), &[("QM_ITEM", "item")]);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    qm.run_with(&["hook-session-start"], &start_payload("sess-two-2"), &[("QM_LOOSE", "L9")]);
    let all = qm.norm(&qm.ok(&["sessions"]));
    assert_eq!(all, "● live   sess-two  loose:L9  /tmp  -  TS  -\n● live   sess-one  item  /tmp  -  TS  -\n");
    let one = qm.ok(&["sessions", "-n", "1"]);
    assert_eq!(one.lines().count(), 1);
    assert!(one.contains("sess-two"));
    // launched, loose-aside and dismissed sessions stay out of the inbox; a loose chat does not
    let inbox = qm.norm(&qm.ok(&["inbox"]));
    assert_eq!(inbox, "● live   sess-two  loose:L9  /tmp  -  TS  -\n");
}

#[test]
fn dismiss_needs_a_session_id() {
    let qm = Qm::new();
    let o = qm.run(&["dismiss"]);
    assert_eq!((o.code, o.stderr.as_str()), (1, "usage: qm dismiss SESSION_ID\n"));
    qm.ok(&["dismiss", "zzz"]);
    assert_eq!(qm.read("sessions.jsonl"), "{\"v\":1,\"event\":\"dismissed\",\"session_id\":\"zzz\",\"ts\":\"TS\"}\n");
}

#[test]
fn sessions_on_a_dead_pid_are_not_live() {
    let qm = Qm::new();
    std::fs::write(
        qm.root.path().join("sessions.jsonl"),
        "{\"v\":1,\"event\":\"start\",\"session_id\":\"dead1234\",\"cwd\":\"/tmp\",\"pid\":999999,\"branch\":\"b\",\"ts\":\"2026-01-02T03:04:05+00:00\"}\nbroken line\n",
    )
    .unwrap();
    assert_eq!(qm.ok(&["sessions"]), "  ended  dead1234  -  /tmp  b  2026-01-02T03:04  -\n");
}

#[test]
fn sessions_and_inbox_do_not_log_hooks_but_do_log_themselves_as_feature_use() {
    let qm = Qm::new();
    qm.run_with(&["hook-session-start"], &start_payload("f1"), &[]);
    assert!(!qm.exists("feature-use.jsonl"));
    qm.ok(&["sessions"]);
    qm.ok(&["inbox"]);
    let log = qm.read("feature-use.jsonl");
    assert!(log.contains("\"name\":\"sessions\"") && log.contains("\"name\":\"inbox\""), "{log}");
}

#[test]
fn the_seeded_config_matches_the_fixture() {
    let qm = Qm::new();
    qm.ok(&["new", "Seed the config"]);
    let seeded = qm.read("config.yml");
    assert!(seeded.starts_with("# qm configuration"), "{seeded:?}");
    golden("config_seed.yml", &seeded);
}
