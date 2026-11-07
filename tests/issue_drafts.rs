//! Issue drafts, `qm issues-dir`, `qm draft list` and the cleanup plan that lists the drafts: worlds
//! built twice, Ruby run on one copy and Rust on the other, and the printed text and every file left
//! behind (the `.prev` revision snapshots included) must agree. Both binaries are named explicitly, so
//! `QM_BIN` doesn't matter here.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn rust() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_qm"))
}

fn ruby() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("quartermaster")
}

fn put(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn item(root: &Path, slug: &str, project: &str, branches: &[&str]) {
    let list = if branches.is_empty() { String::new() } else { format!("branches:\n{}", branches.iter().map(|b| format!("- {b}\n")).collect::<String>()) };
    put(root, &format!("work-items/{slug}.md"), &format!("---\nv: 1\ntitle: {slug}\nstate: in_progress\nproject: {project}\ncreated: '2026-01-01'\n{list}---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n"));
}

fn issue(root: &Path, slug: &str, file: &str, text: &str) {
    put(root, &format!("issues/{slug}/{file}"), text);
}

fn fm(fields: &str) -> String {
    format!("---\n{fields}---\n\nThe body.\n\n- a list\n")
}

/// One item whose issue files cover every way a file can read: created or not, every error Ruby
/// words, and files that only differ in how the YAML spells a value.
fn drafts_world(root: &Path) {
    item(root, "shared", "elsewhere", &[]);
    item(root, "plain", "elsewhere", &[]);
    item(root, "empty", "elsewhere", &[]);
    let files: &[(&str, String)] = &[
        ("01-open-feature.md", fm("title: Add the thing\nkind: feature\ngroup: Foundations\nparent: \"\"\ndeps: []\nchange: first draft\n")),
        ("02-created.md", fm("title: Already on GitHub\nkind: bug\ndeps: [01-open-feature, 03-hash-number]\nnumber: 40\n")),
        ("03-hash-number.md", fm("title: Hash number\nkind: feature\nnumber: \"#41\"\ndeps:\n  - —\n  - none\n  - 01-open-feature\n")),
        ("04-quoted.md", fm("title: \"Fix: a colon, in quotes\"\nkind: bug\ngroup: 'G 2'\n")),
        ("05-bad-kind.md", fm("title: Odd kind\nkind: epic\n")),
        ("06-no-title.md", fm("kind: bug\n")),
        ("07-no-frontmatter.md", "just a body\n".into()),
        ("08-not-fields.md", fm("just text\n")),
        ("09-title-list.md", fm("title: [a, b]\nkind: bug\n")),
        ("10-bad-number.md", fm("title: Bad number\nkind: bug\nnumber: abc\n")),
        ("11-zero.md", fm("title: Zero\nkind: bug\nnumber: 0\n")),
        ("12-deps-map.md", fm("title: Deps map\nkind: bug\ndeps: {a: b}\n")),
        ("13-deps-nested.md", fm("title: Deps nested\nkind: bug\ndeps: [[a]]\n")),
        ("14-empty-front.md", "---\n\n---\nbody\n".into()),
        ("15-crlf.md", "---\r\ntitle: Windows endings\r\nkind: feature\r\nnumber: 7\r\n---\r\nbody\r\n".into()),
        ("16-lone-dep.md", fm("title: One dep\nkind: bug\ndeps: 02-created\ngroup:\nparent: 5\n")),
        ("17-bool-number.md", fm("title: Bool number\nkind: bug\nnumber: true\n")),
        ("18-comment-only.md", fm("# nothing but a comment\n")),
        ("19-false-front.md", fm("false\n")),
        ("20-unicode-é.md", fm("title: Ünïcode — title\nkind: feature\n")),
        ("21-control\u{1}.md", "---\ntitle: Body with controls\nkind: bug\n---\n\u{1}\u{b}\u{1f}\"quoted\" \\ back\n".into()),
        ("22-title-blank.md", fm("title: \"  \"\nkind: bug\n")),
        ("23-kind-blank.md", fm("title: No kind\n")),
    ];
    for (file, text) in files {
        issue(root, "shared", file, text);
    }
    put(root, "issues/shared/.hidden.md", &fm("title: Hidden\nkind: bug\n"));
    put(root, "issues/shared/notes.txt", "not an issue\n");
    issue(root, "plain", "01-a.md", &fm("title: A plain one\nkind: feature\n"));
    issue(root, "plain", "02-b.md", &fm("title: Already created\nkind: bug\nnumber: 12\n"));
    // snapshots: one stale (its file is gone), one corrupt, one settled and one still settling
    put(root, "issues/shared/.prev/gone.md.json", "{\"v\":1,\"seen\":\"x\",\"seen_at\":1.0,\"previous\":null}");
    put(root, "issues/shared/.prev/04-quoted.md.json", "not json at all");
    put(root, "issues/shared/.prev/01-open-feature.md.json", "{\"v\":1,\"seen\":\"an older text\",\"seen_at\":1000.5,\"previous\":\"even older\",\"acked\":true}");
    let recent = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64() - 5.0;
    put(root, "issues/shared/.prev/02-created.md.json", &format!("{{\"v\":1,\"seen\":\"draft one\",\"seen_at\":{recent},\"previous\":\"kept\"}}"));
    put(root, "issues/shared/.prev/03-hash-number.md.json", "{\"v\":1,\"seen\":\"also different\",\"previous\":7}");
    put(root, "issues/plain/.prev/02-b.md.json", "{\"seen\":\"stale\"}");
    put(
        root,
        "drafts/shared.json",
        "{\"v\":1,\"entries\":[{\"id\":\"a1\",\"kind\":\"issue-comment\",\"comment\":\"c\",\"file\":\"03-hash-number.md\"},{\"id\":\"a2\",\"kind\":\"issue-comment\",\"comment\":\"c\",\"file\":\"03-hash-number.md\"},{\"id\":\"a3\",\"kind\":\"issue-comment\",\"comment\":\"c\",\"file\":\"01-open-feature.md\"},{\"id\":\"a4\",\"kind\":\"diff-comment\",\"comment\":\"d\",\"file\":\"01-open-feature.md\"},{\"id\":\"a5\",\"kind\":\"issue-comment\",\"comment\":\"c\",\"file\":\"07-no-frontmatter.md\"},{\"id\":\"a6\",\"kind\":\"issue-comment\",\"comment\":\"c\",\"file\":\"02-created.md\"}]}",
    );
}

struct Run {
    out: String,
    code: i32,
}

fn run(bin: &Path, root: &Path, args: &[&str], env: &[(&str, &str)]) -> Run {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .env("QM_ROOT", root)
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .env("QM_TMUX_SOCKET", "qm-test-issue-drafts")
        .env_remove("QM_SKIP")
        .current_dir(root)
        .stdin(Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().expect("spawn qm");
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr).lines().map(|l| format!("stderr: {l}\n")).collect::<String>());
    Run { out: text.replace(root.to_str().unwrap(), "ROOT"), code: o.status.code().unwrap_or(-1) }
}

fn tree(dir: &Path, base: &Path, out: &mut Vec<String>) {
    let mut entries: Vec<_> = fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        let rel = path.strip_prefix(base).unwrap().display().to_string();
        if path.is_dir() {
            out.push(format!("{rel}/"));
            tree(&path, base, out);
        } else if !rel.ends_with(".lock") && !rel.starts_with("qm.log") {
            let text = String::from_utf8_lossy(&fs::read(&path).unwrap()).into_owned();
            let text = regex::Regex::new(r#""seen_at":[0-9.e+-]+"#).unwrap().replace_all(&text, "\"seen_at\":T").into_owned();
            out.push(format!("{rel}: {}", normalize(&text)));
        }
    }
}

fn normalize(s: &str) -> String {
    let ts = regex::Regex::new(r"\d{4}-\d{2}-\d{2}(T\d{2}:\d{2}(:\d{2})?(\.\d+)?(Z|[+-]\d{2}:\d{2})?)?").unwrap();
    ts.replace_all(s, "TS").into_owned()
}

fn transcript(bin: &Path, root: &Path, commands: &[&[&str]], env: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for args in commands {
        let r = run(bin, root, args, env);
        out.push_str(&format!("$ qm {} → {}\n{}", args.join(" "), r.code, normalize(&r.out)));
    }
    let mut files = vec![];
    tree(root, root, &mut files);
    out.push_str("=== files\n");
    out.push_str(&files.join("\n"));
    out
}

/// Fails with the first lines that differ, not both transcripts whole.
fn assert_same(rust_text: &str, ruby_text: &str, what: &str) {
    if rust_text == ruby_text {
        return;
    }
    let (r, y): (Vec<&str>, Vec<&str>) = (rust_text.lines().collect(), ruby_text.lines().collect());
    let at = (0..r.len().max(y.len())).find(|i| r.get(*i) != y.get(*i)).unwrap();
    let show = |v: &Vec<&str>| v.iter().skip(at.saturating_sub(2)).take(6).map(|l| l.chars().take(300).collect::<String>()).collect::<Vec<_>>().join("\n  ");
    panic!("{what}: first difference at line {at}\nRust:\n  {}\nRuby:\n  {}", show(&r), show(&y));
}

fn same(build: impl Fn(&Path), commands: &[&[&str]]) -> String {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    build(a.path());
    build(b.path());
    let ruby_text = transcript(&ruby(), a.path(), commands, &[]);
    // a Ruby that isn't there: whatever Rust prints, it printed itself
    let rust_text = transcript(rust(), b.path(), commands, &[("QM_RUBY_LAUNCHER", "/nonexistent/qm")]);
    assert_same(&rust_text, &ruby_text, "Rust and Ruby disagree");
    ruby_text
}

fn plain(s: &str) -> String {
    regex::Regex::new("\x1b\\[[0-9;]*m").unwrap().replace_all(s, "").into_owned()
}

#[test]
fn cleanup_lists_every_issue_draft_and_keeps_the_same_revision_snapshots() {
    let text = plain(&same(
        drafts_world,
        &[&["cleanup", "shared"], &["cleanup", "shared"], &["cleanup", "plain"], &["cleanup", "empty"], &["state", "plain", "resolved"], &["state", "shared", "resolved"]],
    ));
    for expected in [
        "shared: 23 issue drafts — 3 ready, 20 need a look",
        "no repo root configured for 'elsewhere'",
        "never created on GitHub; 1 unsent comment",
        "doesn't parse: kind must be feature or bug, not 'epic'",
        "doesn't parse: title is missing",
        "doesn't parse: no frontmatter (--- title: … ---) at the top",
        "doesn't parse: frontmatter is not a set of fields",
        "doesn't parse: title must be text",
        "doesn't parse: number must be the issue's number, not 'abc'",
        "doesn't parse: number must be the issue's number, not '0'",
        "doesn't parse: number must be the issue's number, not 'true'",
        "doesn't parse: deps must be a list",
        "doesn't parse: deps must be a list of text",
        "#40; 1 unsent comment",
        "#41; 2 unsent comments",
        "shared/03-hash-number.md “Hash number”",
        "plain: 2 issue drafts — 1 ready, 1 needs a look",
        "nothing to clean up",
        "cleanup: 23 issue drafts",
    ] {
        assert!(text.contains(expected), "the world should show {expected:?}\n{text}");
    }
    let shown = text.split("=== files").next().unwrap();
    assert!(!shown.contains("Hidden") && !shown.contains("notes.txt") && !shown.contains(".hidden"), "dotfiles and other extensions are not issues");
}

#[test]
fn revisions_move_the_same_way_when_a_file_changes_between_reads() {
    let build = |root: &Path| {
        item(root, "rev", "elsewhere", &[]);
        issue(root, "rev", "01-a.md", &fm("title: First\nkind: bug\n"));
        issue(root, "rev", "02-b.md", &fm("title: Second\nkind: bug\n"));
        put(root, "issues/rev/.prev/02-b.md.json", "{\"v\":1,\"seen\":\"old\",\"seen_at\":5.0,\"previous\":\"older\"}");
    };
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    build(a.path());
    build(b.path());
    let mut texts = vec![];
    for (bin, root) in [(ruby(), a.path()), (rust().to_path_buf(), b.path())] {
        let mut out = String::new();
        let step = |out: &mut String| {
            let r = run(&bin, root, &["cleanup", "rev"], &[("QM_RUBY_LAUNCHER", "/nonexistent/qm")]);
            let mut files = vec![];
            tree(&root.join("issues"), root, &mut files);
            out.push_str(&format!("{} {}\n{}\n", r.code, normalize(&r.out), files.join("\n")));
        };
        step(&mut out); // first sight of 01, 02 changed since a long-settled snapshot
        issue(root, "rev", "01-a.md", &fm("title: First, revised\nkind: bug\n"));
        step(&mut out); // 01 changed seconds after it was seen: still the same revision
        let snap = root.join("issues/rev/.prev/01-a.md.json");
        let text = fs::read_to_string(&snap).unwrap().replace(regex::Regex::new(r#""seen_at":[0-9.e+-]+"#).unwrap().find(&fs::read_to_string(&snap).unwrap()).unwrap().as_str(), "\"seen_at\":3.0");
        fs::write(&snap, text).unwrap();
        issue(root, "rev", "01-a.md", &fm("title: First, revised again\nkind: bug\n"));
        step(&mut out); // ... and once settled it is a new one
        fs::remove_file(root.join("issues/rev/02-b.md")).unwrap();
        step(&mut out); // a removed file's snapshot goes with it
        texts.push(out);
    }
    assert_same(&texts[1], &texts[0], "snapshots");
    assert!(texts[0].contains("\"previous\":\"---\\ntitle: First, revised\\n"), "a settled snapshot keeps the version before it as previous\n{}", texts[0]);
    assert!(!texts[0].rsplit_once("0 ").unwrap().1.contains("02-b.md.json"));
}

#[test]
fn what_only_ruby_reads_exactly_goes_to_ruby_before_anything_is_written() {
    let cases: [(&str, &str); 14] = [
        ("yes-title", "title: yes\nkind: bug\n"),
        ("date-number", "title: D\nkind: bug\nnumber: 2024-01-01\n"),
        ("comma-title", "title: 1,000 rows\nkind: bug\n"),
        ("float-number", "title: F\nkind: bug\nnumber: 1.5\n"),
        ("octal-dep", "title: O\nkind: bug\ndeps: [010, 08]\n"),
        ("hex-group", "title: H\nkind: bug\ngroup: 0x1F\n"),
        ("underscore", "title: U\nkind: bug\nnumber: 1_000\n"),
        ("anchor", "title: &a A\nkind: bug\ngroup: *a\n"),
        ("tag", "title: !!str 5\nkind: bug\n"),
        ("syntax", "title: a: b\nkind: bug\n"),
        ("duplicate", "title: a\ntitle: b\nkind: bug\n"),
        ("tab", "title: a\n\tkind: bug\n"),
        ("inf", "title: I\nkind: bug\nparent: .inf\n"),
        ("symbol", "title: :sym\nkind: bug\n"),
    ];
    for (name, front) in cases {
        let build = |root: &Path| {
            item(root, "odd", "elsewhere", &[]);
            issue(root, "odd", "01-good.md", &fm("title: Fine\nkind: bug\n"));
            issue(root, "odd", "02-odd.md", &fm(front));
        };
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        build(a.path());
        build(b.path());
        let commands: [&[&str]; 2] = [&["cleanup", "odd"], &["state", "odd", "resolved"]];
        let ruby_text = transcript(&ruby(), a.path(), &commands, &[]);
        let rust_text = transcript(rust(), b.path(), &commands, &[]);
        assert_same(&rust_text, &ruby_text, name);
    }
}

#[test]
fn files_ruby_reports_differently_are_ruby_s() {
    let build = |root: &Path| {
        item(root, "odd", "elsewhere", &[]);
        issue(root, "odd", "01-good.md", &fm("title: Fine\nkind: bug\n"));
        put(root, "issues/odd/02-latin1.md", "---\ntitle: caf\u{e9}\nkind: bug\n---\n");
        fs::write(root.join("issues/odd/03-bytes.md"), b"---\ntitle: x\nkind: bug\n---\n\xff\xfe\n").unwrap();
        fs::create_dir_all(root.join("issues/odd/04-dir.md")).unwrap();
    };
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    build(a.path());
    build(b.path());
    let commands: [&[&str]; 1] = [&["cleanup", "odd"]];
    assert_same(&transcript(rust(), b.path(), &commands, &[]), &transcript(&ruby(), a.path(), &commands, &[]), "odd files");
}

#[test]
fn issues_dir_creates_and_prints_the_directory() {
    let text = same(
        |root| {
            item(root, "has", "elsewhere", &[]);
            item(root, "fresh", "elsewhere", &[]);
            issue(root, "has", "01-a.md", &fm("title: A\nkind: bug\n"));
        },
        &[&["issues-dir", "has"], &["issues-dir", "fresh"], &["issues-dir", "fresh"], &["issues-dir", "nosuch"], &["issues-dir", "bad slug"], &["issues-dir", "../x"], &["issues-dir", ""], &["issues-dir"], &["issues-dir", "fresh", "extra"],],
    );
    assert!(text.contains("$ qm issues-dir fresh → 0\nROOT/issues/fresh\n") && text.contains("no work item 'nosuch'") && text.contains("is not a work item slug") && text.contains("usage: qm issues-dir SLUG"));
}

fn entries_world(root: &Path) {
    item(root, "many", "elsewhere", &[]);
    item(root, "none", "elsewhere", &[]);
    item(root, "broken", "elsewhere", &[]);
    item(root, "weird", "elsewhere", &[]);
    item(root, "floaty", "elsewhere", &[]);
    item(root, "octal", "elsewhere", &[]);
    let entries = serde_json::json!([
        {"id": "g1", "kind": "github-comment", "comment": "Use a\nnewline here", "rev": 1, "path": "src/a.rb", "side": "new", "line": 12, "repo": "acme/widgets", "number": 7, "commit": "abcdef0123456789", "subject": "line", "author": "claude"},
        {"id": "g2", "kind": "github-comment", "comment": "  whole file  ", "path": "src/b.rb", "side": "old", "line": 3, "start_line": 1, "repo": "acme/widgets", "number": 7, "commit": "abc", "subject": "file", "author": "user"},
        {"id": "g3", "kind": "github-comment", "comment": "cross side", "path": "c.rb", "side": "new", "line": 9, "start_line": 4, "start_side": "old", "repo": "acme/widgets/", "number": 8, "commit": "1234567890", "author": "bogus"},
        {"id": "t1", "kind": "thread-reply", "comment": "reply with line", "thread": "PRRT_abc", "repo": "acme/widgets", "number": 7, "path": "d.rb", "line": 5, "author": "claude"},
        {"id": "t2", "kind": "thread-reply", "comment": "reply without line", "thread": "PRRT_def", "repo": "acme/widgets", "number": 7, "path": "d.rb"},
        {"id": "t3", "kind": "thread-reply", "comment": "line zero", "thread": "PRRT_ghi", "repo": "/", "number": 7, "path": "d.rb", "line": 0},
        {"id": "d1", "kind": "diff-comment", "comment": "chat one", "path": "e.rb", "side": "new", "line": "15"},
        {"id": "d2", "kind": "diff-comment", "comment": "chat pr", "path": "f.rb", "side": "new", "line": 2, "start_line": "2", "repo": "o/r", "number": 3, "commit": "abcdef0"},
        {"id": "d3", "kind": "diff-comment", "comment": "no path at all", "line": 4},
        {"id": "p1", "kind": "annotation", "comment": "annotated", "page": "/dashboard", "url": "http://x.test/"},
        {"id": "p2", "kind": "annotation", "comment": "only url", "url": "http://x.test/y"},
        {"id": "p3", "kind": "annotation", "comment": "blank page", "page": "", "url": "http://x.test/z"},
        {"id": "i1", "kind": "issue-comment", "comment": "x".repeat(150) + " end", "file": "01-a.md", "block": "", "title": "T"},
        {"id": "i2", "kind": "issue-comment", "comment": "🤖 ünï ".repeat(40), "file": "02-b.md"},
        {"id": "u1", "kind": "mystery", "comment": "unknown kind"},
        {"kind": "diff-comment", "comment": "no id", "path": "h.rb", "line": true},
        {"id": 42, "kind": "diff-comment", "comment": null, "path": "i.rb", "line": null},
        {"id": "u2", "kind": "diff-comment", "comment": "\n\n  later line\nsecond", "path": "j.rb", "line": 1, "start_line": 1, "side": "old", "start_side": "new"},
        {"id": "u3", "comment": "no kind", "author": 5, "owners": [], "extra": {}, "deep": [[], {}, {"k": []}]},
        {"id": "u4", "kind": "diff-comment", "comment": "ctl \u{1f} del \u{7f} ls \u{2028} slash / quote \" back \\ tab \t é 😀", "path": "k.rb", "line": 8}
    ]);
    put(root, "drafts/many.json", &serde_json::to_string_pretty(&serde_json::json!({"v": 1, "entries": entries, "loaded": ["x"]})).unwrap());
    put(root, "drafts/broken.json", "{not json");
    put(root, "drafts/weird.json", "{\"v\":1,\"entries\":[\"just a string\"]}");
    put(root, "drafts/octal.json", "{\"v\":1,\"entries\":[{\"id\":\"o\",\"kind\":\"diff-comment\",\"comment\":\"c\",\"path\":\"p\",\"line\":\"012\"},{\"id\":\"h\",\"kind\":\"diff-comment\",\"comment\":\"c\",\"path\":\"p\",\"line\":\"0x1f\",\"start_line\":\" 3 \"}]}");
    put(root, "drafts/floaty.json", "{\"v\":1,\"entries\":[{\"id\":\"f\",\"kind\":\"diff-comment\",\"comment\":\"c\",\"path\":\"p\",\"line\":1.5}]}");
}

#[test]
fn draft_list_prints_every_kind_of_draft_and_the_json() {
    let text = plain(&same(
        entries_world,
        &[
            &["draft", "list", "many"],
            &["draft", "list", "many", "--json"],
            &["draft", "list", "--json", "many"],
            &["draft", "list", "none"],
            &["draft", "list", "none", "--json"],
            &["draft", "list", "broken"],
            &["draft", "list", "nosuch"],
            &["draft", "list"],
            &["draft", "list", "many", "extra"],
                        &["draft"],
            &["draft", "bogus"],
        ],
    ));
    for expected in [
        "g1  review comment on widgets#7 src/a.rb:12 @abcdef0  🤖 claude: Use a",
        "g2  file comment on widgets#7 src/b.rb:1-3 (removed) @abc  user: whole file",
        "g3  review comment on widgets#8 c.rb:old 4-new 9 @1234567  user: cross side",
        "t1  reply on widgets#7 to thread PRRT_abc (d.rb:5)  🤖 claude: reply with line",
        "t2  reply on widgets#7 to thread PRRT_def  user: reply without line",
        "d1  chat comment on e.rb:15  user: chat one",
        "d2  chat comment on f.rb:2-2 of r#3  user: chat pr",
        "p2  page annotation on http://x.test/y",
        "i1  issue comment on 01-a.md",
        "u1  mystery",
        "no drafts",
        "usage: qm draft add SLUG --path P --line N",
    ] {
        assert!(text.contains(expected), "the world should show {expected:?}\n{text}");
    }
}

#[test]
fn entries_ruby_prints_in_its_own_way_go_to_ruby() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    entries_world(a.path());
    entries_world(b.path());
    let commands: [&[&str]; 7] = [&["draft", "list", "weird"], &["draft", "list", "floaty"], &["draft", "list", "floaty", "--json"], &["draft", "list", "octal"], &["draft", "list", "octal", "--json"], &["draft", "list", "--jso", "many"], &["draft", "list", "--bogus", "many"]];
    assert_same(&transcript(rust(), b.path(), &commands, &[]), &transcript(&ruby(), a.path(), &commands, &[]), "entries only Ruby prints exactly");
}
