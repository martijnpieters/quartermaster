//! Work-item commands, pinned as full transcripts (exit code, stdout, stderr and the item file
//! after every step) generated from the Ruby implementation.

mod common;
use common::*;

#[test]
fn the_happy_paths_of_every_work_item_command() {
    let qm = Qm::new();
    let steps: Vec<Step> = vec![
        (&["list"], ""),
        (&["new", "Fix login bug", "--project", "demo", "--branch", "feat/a", "--description", "Line one"], ""),
        (&["new", "Second one", "--project", "a much longer project name"], ""),
        (&["note", "fix-login-bug", "first"], ""),
        (&["note", "fix-login-bug", "--kind", "open", "second blocker"], ""),
        (&["note", "fix-login-bug", "third", "--kind", "history"], ""),
        (&["note", "fix-login-bug", "2026-01-02: stamped", "text"], ""),
        (&["note", "fix-login-bug", "colons: and [brackets] stay"], ""),
        (&["retag", "fix-login-bug", "1", "open"], ""),
        (&["retag", "fix-login-bug", "2", "history"], ""),
        (&["state", "fix-login-bug", "fridge"], ""),
        (&["state", "fix-login-bug", "in_progress"], ""),
        (&["link-branch", "fix-login-bug", "feat/b"], ""),
        (&["link-branch", "fix-login-bug", "feat/a"], ""),
        (&["link-branch", "fix-login-bug", "scratch", "--temp"], ""),
        (&["link-branch", "fix-login-bug", "scratch"], ""),
        (&["link-branch", "fix-login-bug", "--temp", "tmp2"], ""),
        (&["link-branch", "fix-login-bug", "tmp2", "--temp"], ""),
        (&["link-branch", "fix-login-bug", "feat/a", "--temp"], ""),
        (&["unlink-branch", "fix-login-bug", "feat/b"], ""),
        (&["unlink-branch", "fix-login-bug", "feat/a"], ""),
        (&["unlink-branch", "fix-login-bug", "scratch"], ""),
        (&["board", "fix-login-bug"], ""),
        (&["board", "fix-login-bug", "https://example.test/b"], ""),
        (&["board", "fix-login-bug"], ""),
        (&["board", "fix-login-bug", "https://example.test/b2", "--answers-dir", "/tmp/answers"], ""),
        (&["board", "fix-login-bug", "https://example.test/b2"], ""),
        (&["board", "fix-login-bug", "https://example.test/b3"], ""),
        (&["board", "fix-login-bug", "--answers-dir", "/srv/a", "https://example.test/b3"], ""),
        (&["board", "fix-login-bug", "clear"], ""),
        (&["move", "fix-login-bug", "other project"], ""),
        (&["move", "fix-login-bug", "/abs/path"], ""),
        (&["retitle", "fix-login-bug", "New: title", "[x]"], ""),
        (&["describe", "fix-login-bug", "Multi", "word", "description"], ""),
        (&["describe", "fix-login-bug", "-"], "from\nstdin\n\nwith a gap\n"),
        (&["describe", "fix-login-bug"], "  piped without dash  \n"),
        (&["state", "fix-login-bug", "resolved"], ""),
        (&["state", "fix-login-bug", "in_progress"], ""),
        (&["list"], ""),
        (&["attach", "second-one", "--session", "s1"], ""),
        (&["detach", "second-one", "--session", "s1"], ""),
        (&["detach", "second-one", "--session", "never-there"], ""),
        (&["list"], ""),
    ];
    golden("work_items_happy.txt", &transcript(&qm, "fix-login-bug", &steps));
}

#[test]
fn usage_errors_unknown_items_and_bad_values() {
    let qm = Qm::new();
    qm.ok(&["new", "Target"]);
    let steps: Vec<Step> = vec![
        (&["new"], ""),
        (&["new", "  "], ""),
        (&["new", "Target"], ""),
        (&["note"], ""),
        (&["note", "target"], ""),
        (&["note", "ghost", "text"], ""),
        (&["note", "target", "text", "--kind", "bogus"], ""),
        (&["retag"], ""),
        (&["retag", "target", "1"], ""),
        (&["retag", "target", "1", "open"], ""),
        (&["retag", "target", "abc", "open"], ""),
        (&["retag", "ghost", "1", "open"], ""),
        (&["state"], ""),
        (&["state", "target"], ""),
        (&["state", "ghost", "fridge"], ""),
        (&["state", "target", "bogus"], ""),
        (&["link-branch"], ""),
        (&["link-branch", "target"], ""),
        (&["link-branch", "ghost", "b"], ""),
        (&["unlink-branch"], ""),
        (&["unlink-branch", "target", "nope"], ""),
        (&["unlink-branch", "ghost", "b"], ""),
        (&["board"], ""),
        (&["board", "ghost"], ""),
        (&["board", "target", "--answers-dir", "/x"], ""),
        (&["board", "target"], ""),
        (&["move"], ""),
        (&["move", "target"], ""),
        (&["move", "ghost", "p"], ""),
        (&["retitle"], ""),
        (&["retitle", "target"], ""),
        (&["retitle", "ghost", "t"], ""),
        (&["describe"], ""),
        (&["describe", "target"], ""),
        (&["describe", "ghost", "text"], ""),
        (&["detach"], ""),
        (&["detach", "target"], ""),
        (&["detach", "ghost", "--session", "s"], ""),
        (&["show", "ghost"], ""),
    ];
    golden("work_items_errors.txt", &transcript(&qm, "target", &steps));
}

#[test]
fn option_spellings_ruby_accepts_still_work() {
    let qm = Qm::new();
    let steps: Vec<Step> = vec![
        (&["new", "Spelled", "--project=demo", "--branch=feat/x", "--description=From equals"], ""),
        (&["new", "Abbreviated", "--proj", "demo", "--desc", "short flags"], ""),
        (&["new", "--project", "demo", "Options first"], ""),
        (&["new", "Dashed", "--", "--literal"], ""),
        (&["note", "spelled", "--kind=open", "equals kind"], ""),
        (&["note", "spelled", "--ki", "history", "abbreviated kind"], ""),
        (&["note", "spelled", "--", "--dashed text"], ""),
        (&["retag", "spelled", "01", "history"], ""),
        (&["retag", "spelled", "0x1", "history"], ""),
        (&["retag", "spelled", " 2 ", "history"], ""),
        (&["board", "spelled", "https://example.test/x", "--answers-dir=/tmp/a"], ""),
        (&["board", "spelled", "--ans", "/tmp/b", "https://example.test/y"], ""),
        (&["link-branch", "spelled", "--tem", "tmp"], ""),
        (&["detach", "spelled", "--session=none"], ""),
        (&["list", "ignored", "extra"], ""),
        (&["state", "spelled", "fridge", "extra"], ""),
        (&["move", "spelled", "p", "extra"], ""),
    ];
    golden("work_items_spellings.txt", &transcript(&qm, "spelled", &steps));
}

#[test]
fn items_with_legacy_and_extra_keys_keep_them_through_every_edit() {
    let qm = Qm::new();
    qm.ok(&["new", "Legacy"]);
    let path = qm.root.path().join("work-items/legacy.md");
    let text = std::fs::read_to_string(&path).unwrap();
    let text = text.replacen(
        "project: No project\n",
        "project: No project\nbranch: feat/old\ntemp_branches:\n- worktree-a\n- worktree-b\npf_mode: dev\n",
        1,
    );
    let text = text.replacen(
        "\n---\n",
        "\nrotations:\n- at: '2026-01-01T00:00:00+00:00'\n  from: s0\n  via: wrap\n  carried:\n  - 1\n  - 2\n  decisions: 1\n  open: 0\nsnoozed_until: '2099-01-01'\n---\n",
        1,
    );
    let text = text.replace("## Notes\n", "## Notes\n- 2026-01-01 [weird]: unknown marker\n- 2026-01-02 [open]:   padded text   \n");
    std::fs::write(&path, text).unwrap();
    let steps: Vec<Step> = vec![
        (&["link-branch", "legacy", "feat/new"], ""),
        (&["link-branch", "legacy", "worktree-a"], ""),
        (&["link-branch", "legacy", "worktree-c", "--temp"], ""),
        (&["unlink-branch", "legacy", "feat/old"], ""),
        (&["unlink-branch", "legacy", "feat/new"], ""),
        (&["unlink-branch", "legacy", "worktree-a"], ""),
        (&["retag", "legacy", "1", "history"], ""),
        (&["note", "legacy", "appended after odd notes"], ""),
        (&["state", "legacy", "fridge"], ""),
        (&["move", "legacy", "demo"], ""),
        (&["board", "legacy", "https://example.test/l"], ""),
        (&["board", "legacy", "clear"], ""),
        (&["describe", "legacy", "now described"], ""),
    ];
    golden("work_items_legacy.txt", &transcript(&qm, "legacy", &steps));
}

#[test]
fn a_body_without_the_expected_sections_still_takes_notes_and_descriptions() {
    let qm = Qm::new();
    std::fs::create_dir_all(qm.root.path().join("work-items")).unwrap();
    std::fs::write(qm.root.path().join("work-items/bare.md"), "---\nv: 1\ntitle: Bare\nstate: in_progress\nproject: No project\ncreated: '2026-01-01'\n---\njust text\n").unwrap();
    std::fs::write(qm.root.path().join("work-items/gaps.md"), "---\nv: 1\ntitle: Gaps\nstate: in_progress\nproject: No project\ncreated: '2026-01-01'\n---\n\n## Notes\n- 2026-01-01: old\n\n\n\n## Snapshots\n").unwrap();
    let mut out = String::new();
    for (slug, steps) in [
        ("bare", vec![(&["note", "bare", "first"][..], ""), (&["describe", "bare", "desc"][..], ""), (&["retag", "bare", "1", "open"][..], "")]),
        ("gaps", vec![(&["note", "gaps", "second"][..], ""), (&["describe", "gaps", "desc"][..], "")]),
    ] {
        out.push_str(&transcript(&qm, slug, &steps));
    }
    golden("work_items_sections.txt", &out);
}

#[test]
fn a_malformed_item_is_reported_not_a_backtrace() {
    let qm = Qm::new();
    std::fs::create_dir_all(qm.root.path().join("work-items")).unwrap();
    std::fs::write(qm.root.path().join("work-items/broken.md"), "no front matter at all\n").unwrap();
    let steps: Vec<Step> = vec![(&["state", "broken", "fridge"], ""), (&["note", "broken", "x"], ""), (&["list"], "")];
    golden("work_items_malformed.txt", &transcript(&qm, "broken", &steps));
}

const TRICKY: &[&str] = &[
    "Plain title", "Fix: colon", "[WIP] bracket", "#123 hash", "dash - inside", "yes please", "No", "y", "Z", "123", "1.5", "2026-01-02",
    "true", "null", "~", "'quoted'", "say \"hi\"", "it's", "trailing colon:", "a #hash", "a:b", "café ünï", "日本語 タイトル",
    "tab\there", "line\nbreak", "09", "0x1F", "1_000", ":sym", "!bang", "&amp", "*star", "@at", "%pct", "`tick`", "a: b: c",
    "{braces}", "?question", "-5", "+5", ".5", "1e3", "12:30", "1,000", "Truthy", "OFF", "(parens) first", "/leading slash", "$dollar",
    "emoji 🎉 party", "ends with space ", "back\\slash", "12:30:45", "2026-01-02 10:11:12", "1 2 3", "0o17", "0b101", "-.inf", ".NaN", "<<", "= equals",
    "The quick brown fox jumps over the lazy dog while the five boxing wizards jump quickly past the sphinx of black quartz",
    "Averyveryveryveryveryveryveryveryveryveryveryveryveryveryveryveryveryveryveryveryverylongwordwithoutanyspaces",
    "Exactly at the edge of the line width so that folding may or may not happen at this point right",
    "[WIP] The quick brown fox jumps over the lazy dog while the five boxing wizards jump quickly past the sphinx",
    "'Quoted start' and the quick brown fox jumps over the lazy dog while the five boxing wizards jump quickly",
    "Tricky: the quick brown fox jumps over the lazy dog while the five boxing wizards jump quickly past it all",
    "Words  with  double  spaces  inside  the  quick  brown  fox  jumps  over  the  lazy  dog  while  the  five  boxing",
];

const PROJECTS: &[&str] = &["demo", "No project", "/srv/x", "a: b", "09", "yes", "[bracketed]", "café", "#tagged", ""];

#[test]
fn titles_and_projects_are_written_the_way_psych_writes_them() {
    let qm = Qm::new();
    let mut out = String::new();
    for (i, title) in TRICKY.iter().enumerate() {
        let project = PROJECTS[i % PROJECTS.len()];
        let o = qm.run(&["new", title, "--project", project, "--branch", title]);
        out.push_str(&format!("=== {title:?} / project {project:?} → exit {}\n", o.code));
        let slug = o.stdout.trim();
        if let Ok(file) = std::fs::read_to_string(qm.root.path().join(format!("work-items/{slug}.md"))) {
            out.push_str(&file.split("\n---\n").next().unwrap().replace('\n', "\n  "));
            out.push('\n');
        }
        if o.code != 0 {
            out.push_str(&o.stderr);
        }
    }
    golden("work_items_psych_titles.txt", &scrub(&qm, &out));
}

#[test]
fn retitling_and_moving_to_tricky_values_matches_psych() {
    let qm = Qm::new();
    qm.ok(&["new", "Anchor"]);
    let mut out = String::new();
    for (i, title) in TRICKY.iter().enumerate() {
        let project = PROJECTS[i % PROJECTS.len()];
        qm.run(&["retitle", "anchor", title]);
        qm.run(&["move", "anchor", project]);
        let file = std::fs::read_to_string(qm.root.path().join("work-items/anchor.md")).unwrap();
        out.push_str(&format!("=== {title:?} / {project:?}\n{}\n", file.split("\n---\n").next().unwrap()));
    }
    golden("work_items_psych_retitle.txt", &scrub(&qm, &out));
}

#[test]
fn bare_qm_without_a_terminal_lists_the_items() {
    let qm = Qm::new();
    assert_eq!(qm.ok(&[]), "No work items yet. Create one with: qm new TITLE\n");
    qm.ok(&["new", "One"]);
    assert_eq!(qm.ok(&[]), qm.ok(&["list"]));
}

#[test]
fn new_pf_mode_is_checked_against_the_project_s_dev_server() {
    let qm = Qm::new();
    qm.ok(&["list"]); // seeds the root
    std::fs::write(
        qm.root.path().join("config.yml"),
        "projects:\n  web:\n    root: /tmp/web\n    dev_server:\n      command: devsrv\n      modes: [dev, tunnel]\n      default_mode: dev\n  plain: /tmp/plain\n  one: {root: /tmp/one, dev_server: solo}\n",
    )
    .unwrap();
    let steps: Vec<Step> = vec![
        (&["new", "Tunnel work", "--project", "web", "--pf-mode", "tunnel"], ""),
        (&["new", "Bad mode", "--project", "web", "--pf-mode", "prod"], ""),
        (&["new", "Tunnel work", "--project", "web", "--pf-mode", "prod"], ""),
        (&["new", "No server", "--project", "plain", "--pf-mode", "dev"], ""),
        (&["new", "Unknown project", "--project", "ghost", "--pf-mode", "dev"], ""),
        (&["new", "Solo", "--project", "one", "--pf-mode", "default"], ""),
        (&["new", "Yes mode", "--branch", "b", "--session", "s1", "--project", "web", "--pf-mode", "dev", "--description", "d"], ""),
        (&["new", "Trailing", "--pf-mode"], ""),
        (&["new", "--pf-mode=dev", "Joined", "--project", "web"], ""),
        (&["show", "tunnel-work"], ""),
    ];
    golden("work_items_pf_mode.txt", &transcript(&qm, "tunnel-work", &steps));
}
