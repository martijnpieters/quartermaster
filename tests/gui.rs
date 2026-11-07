//! The Rust GUI server (`QM_RUST_GUI=1 qm gui`) next to Ruby's, both on one data root: the same
//! request goes to each and the status, content type and body must agree, route by route. Only what
//! is ported is compared; the rest answers 501 here.

mod common;
use common::{attention_world, git};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

struct Server {
    child: Child,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn start(rust: bool, root: &Path, home: &Path) -> Server {
    start_env(rust, root, home, &[])
}

fn start_env(rust: bool, root: &Path, home: &Path, extra: &[(&str, String)]) -> Server {
    let port = free_port();
    let bin: PathBuf = if rust { Path::new(env!("CARGO_BIN_EXE_qm")).into() } else { Path::new(env!("CARGO_MANIFEST_DIR")).join("quartermaster") };
    let mut cmd = Command::new(bin);
    cmd.arg("gui").env("QM_ROOT", root).env("HOME", home).env("QM_GUI_PORT", port.to_string()).env("QM_GUI_NO_OPEN", "1").env("LANG", "C.UTF-8").env("LC_ALL", "C.UTF-8").env("QM_TMUX_SOCKET", "qm-test-gui").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    if rust {
        cmd.env("QM_RUST_GUI", "1");
    }
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let child = cmd.spawn().unwrap();
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Server { child, port };
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("the GUI did not come up");
}

struct Answer {
    status: String,
    content_type: String,
    body: Vec<u8>,
}

fn request(port: u16, method: &str, path: &str, headers: &[(&str, String)], body: &str, own_host: bool) -> Answer {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\n");
    if own_host {
        req += &format!("Host: 127.0.0.1:{port}\r\n");
    }
    for (k, v) in headers {
        req += &format!("{k}: {}\r\n", v.replace("PORT", &port.to_string()));
    }
    req += &format!("Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    stream.write_all(req.as_bytes()).unwrap();
    let mut raw = vec![];
    stream.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let status = head.lines().next().unwrap().trim_start_matches("HTTP/1.1 ").to_string();
    let content_type = head.lines().find_map(|l| l.strip_prefix("Content-Type: ")).unwrap_or("").to_string();
    Answer { status, content_type, body: raw[split + 4..].to_vec() }
}

fn shown(a: &Answer, port: u16) -> String {
    let body = String::from_utf8_lossy(&a.body).replace(&format!(":{port}"), ":PORT");
    let body = regex::Regex::new(r"let terminalToken = \x22[0-9a-f]{48}\x22").unwrap().replace_all(&body, "let terminalToken = TOKEN").into_owned();
    format!("{} | {} | {body}", a.status, a.content_type)
}

fn world(base: &Path) -> (PathBuf, PathBuf, String) {
    let (root, home, repo) = (base.join("root"), base.join("home"), base.join("repo"));
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    fs::write(repo.join("a.txt"), "one\ntwo\r\nthree\n\nlast without newline").unwrap();
    fs::write(repo.join("bytes.bin"), b"ok \xff\xfe bad\nnext \xe2\x82 line\n").unwrap();
    fs::write(repo.join("big.txt"), "x".repeat(70) + "\n").unwrap();
    let many: String = (0..25_000).map(|i| format!("line {i}\n")).collect();
    fs::write(repo.join("many.txt"), many).unwrap();
    let wide: String = (0..30_000).map(|i| format!("{i:030}\n")).collect();
    fs::write(repo.join("wide.txt"), wide).unwrap();
    fs::write(repo.join("empty.txt"), "").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["-c", "commit.gpgsign=false", "-c", "user.name=t", "-c", "user.email=t@t.test", "commit", "-q", "-m", "one"]);
    let sha = git(&repo, &["rev-parse", "HEAD"]);
    fs::create_dir_all(root.join("work-items")).unwrap();
    fs::write(root.join("config.yml"), format!("projects:\n  demo: {}\n", repo.display())).unwrap();
    for (slug, project) in [("demo-item", "demo"), ("orphan", "elsewhere")] {
        fs::write(root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\ntitle: {slug}\nstate: in_progress\nproject: {project}\ncreated: '2026-01-01'\n---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n")).unwrap();
    }
    fs::write(root.join("sessions.jsonl"), "{\"v\":1,\"event\":\"start\",\"session_id\":\"s1\",\"ts\":\"2026-03-01T00:00:00+00:00\"}\n").unwrap();
    fs::write(root.join("events.jsonl"), "{\"v\":1}\n").unwrap();
    fs::create_dir_all(root.join("drafts")).unwrap();
    fs::write(root.join("drafts/demo-item.json"), "{\"v\":1,\"entries\":[]}").unwrap();
    fs::write(root.join("drafts/.demo-item.lock"), "").unwrap();
    fs::create_dir_all(root.join("issues/demo-item/.prev")).unwrap();
    fs::write(root.join("issues/demo-item/01-a.md"), "---\ntitle: A\nkind: bug\n---\n").unwrap();
    fs::write(root.join("issues/demo-item/.prev/01-a.md.json"), "{}").unwrap();
    fs::create_dir_all(root.join("issues/a-b")).unwrap();
    fs::write(root.join("issues/a-b/x.md"), "x").unwrap();
    fs::create_dir_all(root.join("issues/a")).unwrap();
    fs::write(root.join("issues/a/x.md"), "x").unwrap();
    (root, home, sha)
}

#[test]
fn the_rust_server_answers_the_ported_routes_as_ruby_does() {
    let base = tempfile::tempdir().unwrap();
    let (root, home, sha) = world(base.path());
    let ruby = start(false, &root, &home);
    let rust = start(true, &root, &home);
    let mut log = String::new();
    let mut check = |label: &str, method: &str, path: &str, headers: &[(&str, String)], body: &str, own_host: bool| {
        let (a, b) = (request(ruby.port, method, path, headers, body, own_host), request(rust.port, method, path, headers, body, own_host));
        let (ra, rb) = (shown(&a, ruby.port), shown(&b, rust.port));
        assert_eq!(rb, ra, "{label}: {method} {path}");
        log += &format!("{label}: {}\n", ra.chars().take(160).collect::<String>().replace('\n', "↵"));
    };
    check("page", "GET", "/", &[], "", true);
    for asset in ["css/base.css", "js/core.js", "js/events.js", "css/diff.css"] {
        check("asset", "GET", &format!("/assets/{asset}"), &[], "", true);
    }
    for bad in ["/assets/css/nope.css", "/assets/js/../../page.html", "/assets/css/Base.css", "/assets/js/core.js.map", "/assets/"] {
        check("bad asset", "GET", bad, &[], "", true);
    }
    check("pulse", "GET", "/api/pulse", &[], "", true);
    check("pulse with query", "GET", "/api/pulse?visible=1&focused=1&chat=demo-item", &[], "", true);
    fs::write(root.join("events.jsonl"), "{\"v\":1}\n{\"v\":2}\n").unwrap();
    fs::write(root.join("drafts/other.json"), "{\"v\":1,\"entries\":[{\"id\":\"x\"}]}").unwrap();
    fs::create_dir_all(root.join("issues/zz")).unwrap();
    fs::write(root.join("issues/zz/y.md"), "y").unwrap();
    check("pulse after changes", "GET", "/api/pulse", &[], "", true);
    let slug = "demo-item";
    for (label, query) in [
        ("file", format!("rev={sha}&path=a.txt")),
        ("short sha", format!("rev={}&path=a.txt", &sha[..7])),
        ("upper sha", format!("rev={}&path=a.txt", sha.to_uppercase())),
        ("bytes", format!("rev={sha}&path=bytes.bin")),
        ("empty file", format!("rev={sha}&path=empty.txt")),
        ("many lines", format!("rev={sha}&path=many.txt")),
        ("many bytes", format!("rev={sha}&path=wide.txt")),
        ("missing", format!("rev={sha}&path=nope.txt")),
        ("tree not blob", format!("rev={sha}&path=")),
        ("no path", format!("rev={sha}")),
        ("no rev", "path=a.txt".to_string()),
        ("bad rev", "rev=zzzz&path=a.txt".to_string()),
        ("quoted rev", "rev=a%22b%0A%23%7Bx%7D&path=a.txt".to_string()),
        ("too short", "rev=abc&path=a.txt".to_string()),
        ("plus path", format!("rev={sha}&path=a%20.txt")),
        ("nul path", format!("rev={sha}&path=a.txt%00x")),
        ("option-like path", format!("rev={sha}&path=--help")),
        ("repeated key", format!("rev=zzz&rev={sha}&path=a.txt")),
    ] {
        check(label, "GET", &format!("/api/diff-file/{slug}?{query}"), &[], "", true);
    }
    check("unknown item", "GET", &format!("/api/diff-file/nosuch?rev={sha}&path=a.txt"), &[], "", true);
    check("no project root", "GET", &format!("/api/diff-file/orphan?rev={sha}&path=a.txt"), &[], "", true);
    check("encoded slug", "GET", &format!("/api/diff-file/demo%2Ditem?rev={sha}&path=a.txt"), &[], "", true);
    check("bad host", "GET", "/api/pulse", &[("Host", "evil.test".into())], "", false);
    check("no host", "GET", "/api/pulse", &[], "", false);
    check("localhost host", "GET", "/api/pulse", &[("Host", "localhost:PORT".into())], "", false);
    check("foreign origin", "POST", "/api/action", &[("Origin", "http://evil.test".into())], "{}", true);
    check("no origin, not local", "POST", "/api/action", &[], "{\"cmd\":\"state\"}", true);
    check("no origin, junk", "POST", "/api/action", &[], "not json", true);
    check("no origin, other route", "POST", "/api/grep", &[], "{}", true);
    // the use log: what each server recorded, by kind and name only
    let uses = fs::read_to_string(root.join("feature-use.jsonl")).unwrap_or_default();
    let names: Vec<String> = uses.lines().map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()).map(|v| format!("{} {}", v["kind"], v["name"])).collect();
    let mut counts = std::collections::BTreeMap::new();
    for n in names.iter().filter(|n| !n.starts_with("\"cli\"")) {
        *counts.entry(n.clone()).or_insert(0) += 1;
    }
    assert!(counts.contains_key("\"get\" \"/api/diff-file\"") && counts.contains_key("\"action\" \"state\"") == false);
    for (name, n) in &counts {
        assert_eq!(n % 2, 0, "{name} is recorded once by each server: {counts:?}");
    }
    assert!(log.lines().count() > 40, "{log}");
}

fn sh(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "user.name=t", "-c", "user.email=t@t.test", "-c", "init.defaultBranch=main", "-c", "protocol.file.allow=always"])
        .args(args)
        .env("GIT_AUTHOR_DATE", "2026-02-01T10:00:00+00:00")
        .env("GIT_COMMITTER_DATE", "2026-02-01T10:00:00+00:00")
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn lines(prefix: &str, n: usize) -> String {
    (1..=n).map(|i| format!("{prefix} {i}\n")).collect()
}

fn commit(dir: &Path, msg: &str) {
    sh(dir, &["add", "-A"]);
    sh(dir, &["commit", "-q", "-m", msg]);
}

/// A repo with a checked-out feature branch (committed, staged-and-not and untracked changes, a rename,
/// a deletion, a binary), an issue branch, a gh-stack of two (the top has no worktree), a ref-only
/// branch and an origin-only one.
fn diff_world(base: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let (root, home, repo, origin) = (base.join("root"), base.join("home"), base.join("repo"), base.join("origin.git"));
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&origin).unwrap();
    sh(&origin, &["init", "-q", "--bare"]);
    sh(&repo, &["init", "-q", "-b", "main"]);
    fs::write(repo.join("a.txt"), lines("alpha", 20)).unwrap();
    fs::write(repo.join("b.txt"), lines("beta", 12)).unwrap();
    fs::write(repo.join("old_name.txt"), lines("rename me", 15)).unwrap();
    fs::write(repo.join("del.txt"), lines("delete me", 5)).unwrap();
    fs::write(repo.join("bin.dat"), b"\x00\x01\x02binary\xff").unwrap();
    fs::write(repo.join("héllo wörld.txt"), lines("unicode", 4)).unwrap();
    fs::write(repo.join("crlf.txt"), (1..=8).map(|i| format!("crlf {i}\r\n")).collect::<String>()).unwrap();
    fs::write(repo.join("nonl.txt"), "first\nsecond\nno newline at the end").unwrap();
    fs::write(repo.join("shift.txt"), lines("shift", 40)).unwrap();
    commit(&repo, "base");
    sh(&repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
    sh(&repo, &["push", "-q", "origin", "main"]);

    let wt = |name: &str, branch: &str, from: &str| -> PathBuf {
        let p = base.join(name);
        sh(&repo, &["worktree", "add", "-q", "-b", branch, p.to_str().unwrap(), from]);
        p
    };
    let feat = wt("wt-feat", "feat", "main");
    let mut a = lines("alpha", 20);
    a = a.replace("alpha 3\n", "alpha three changed\n");
    fs::write(feat.join("a.txt"), a).unwrap();
    fs::write(feat.join("d.txt"), lines("new file", 6)).unwrap();
    fs::write(feat.join("crlf.txt"), (1..=8).map(|i| if i == 4 { "crlf four changed\r\n".to_string() } else { format!("crlf {i}\r\n") }).collect::<String>()).unwrap();
    fs::write(feat.join("nonl.txt"), "first\nsecond\nchanged without newline").unwrap();
    fs::write(feat.join("shift.txt"), lines("top", 3) + &lines("shift", 40)).unwrap();
    commit(&feat, "feat one");
    fs::rename(feat.join("old_name.txt"), feat.join("new_name.txt")).unwrap();
    fs::remove_file(feat.join("del.txt")).unwrap();
    fs::write(feat.join("bin.dat"), b"\x00\x09changed\xff\xfe").unwrap();
    fs::write(feat.join("héllo wörld.txt"), lines("unicode", 4) + "more\n").unwrap();
    commit(&feat, "feat two: rename, delete, binary");
    let a2 = fs::read_to_string(feat.join("a.txt")).unwrap().replace("alpha 8\n", "alpha eight uncommitted\n").replace("alpha 15\n", "");
    fs::write(feat.join("a.txt"), a2).unwrap();
    fs::write(feat.join("b.txt"), lines("beta", 12).replace("beta 6\n", "beta six uncommitted\nextra line\n")).unwrap();
    fs::write(feat.join("new_name.txt"), lines("rename me", 15) + "appended later\n").unwrap();
    let shifted = fs::read_to_string(feat.join("shift.txt")).unwrap().replace("shift 25\n", "");
    fs::write(feat.join("shift.txt"), shifted).unwrap();
    fs::write(feat.join("untracked.txt"), "brand new\nfile\n").unwrap();
    fs::write(feat.join("big_untracked.txt"), lines("big", 25_000)).unwrap();
    fs::write(feat.join("too_big_untracked.bin"), vec![b'x'; 1_100_000]).unwrap();

    let issue = wt("wt-issue", "issue/12-x", "main");
    fs::write(issue.join("issue.txt"), lines("issue work", 3)).unwrap();
    commit(&issue, "issue one");
    let issue3 = wt("wt-issue3", "issue/3-y", "main");
    fs::write(issue3.join("issue3.txt"), lines("issue three", 2)).unwrap();
    commit(&issue3, "issue three one");
    let sa = wt("wt-sa", "stack-a", "main");
    fs::rename(sa.join("old_name.txt"), sa.join("sa_renamed.txt")).unwrap();
    fs::write(sa.join("sa.txt"), lines("stack a", 4)).unwrap();
    fs::write(sa.join("a.txt"), lines("alpha", 20).replace("alpha 1\n", "alpha one from stack a\n")).unwrap();
    commit(&sa, "stack a one");
    sh(&repo, &["branch", "stack-b", "stack-a"]);
    sh(&repo, &["branch", "refonly", "main"]);
    sh(&repo, &["checkout", "-q", "refonly"]);
    fs::write(repo.join("ro.txt"), lines("ref only", 3)).unwrap();
    commit(&repo, "ref only one");
    sh(&repo, &["checkout", "-q", "-b", "remote-only", "main"]);
    fs::write(repo.join("rem.txt"), lines("remote only", 2)).unwrap();
    commit(&repo, "remote only one");
    sh(&repo, &["push", "-q", "origin", "remote-only"]);
    sh(&repo, &["checkout", "-q", "main"]);
    sh(&repo, &["branch", "-q", "-D", "remote-only"]);
    // stack-b gets its own commit through a temporary worktree, then the worktree goes
    let tmp = wt("wt-tmp", "stack-b-tmp", "stack-b");
    fs::write(tmp.join("sb.txt"), lines("stack b", 2)).unwrap();
    commit(&tmp, "stack b one");
    let tip = sh(&tmp, &["rev-parse", "HEAD"]);
    sh(&repo, &["worktree", "remove", "--force", tmp.to_str().unwrap()]);
    sh(&repo, &["branch", "-q", "-D", "stack-b-tmp"]);
    sh(&repo, &["branch", "-f", "stack-b", &tip]);
    // the trunk takes a copy of stack-b's commit (a squash-merge leftover): `git cherry` calls it equivalent
    sh(&repo, &["cherry-pick", &tip]);
    sh(&repo, &["push", "-q", "origin", "main"]);
    sh(&repo, &["checkout", "-q", "-b", "many", "main"]);
    for i in 0..205 {
        sh(&repo, &["commit", "-q", "--allow-empty", "-m", &format!("empty {i}")]);
    }
    sh(&repo, &["checkout", "-q", "main"]);
    sh(&repo, &["remote", "set-url", "origin", "git@github.com:acme/widgets.git"]);
    fs::write(repo.join(".git/gh-stack"), r#"{"stacks":[{"trunk":{"branch":"main"},"branches":[{"branch":"stack-a"},{"branch":"stack-b","pullRequest":{"number":2,"url":"https://github.com/acme/widgets/pull/2"}}]},{"trunk":{"branch":"main"},"branches":[{"branch":"many"}]}]}"#).unwrap();

    fs::create_dir_all(root.join("work-items")).unwrap();
    fs::write(root.join("config.yml"), format!("projects:\n  demo: {}\n", repo.display())).unwrap();
    let item = |slug: &str, branches: &[&str], title: &str, desc: &str| {
        let list: String = branches.iter().map(|b| format!("- {b}\n")).collect();
        let list = if list.is_empty() { String::new() } else { format!("branches:\n{list}") };
        fs::write(root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\ntitle: {title}\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\n{list}---\n\n## Description\n\n{desc}\n\n## Notes\n\n## Snapshots\n")).unwrap();
    };
    item("diff-item", &["feat", "refonly", "remote-only", "stack-a", "issue/12-x", "issue/3-y", "many"], "Diff item", "d");
    item("stack-item-r", &["stack-a"], "Stack item r", "d");
    item("stack-item", &["stack-a"], "Stack item", "d");
    item("nobranch", &[], "No branch", "d");
    item("review-item", &["feat"], "Review acme/widgets#9: x", "Review of pull request https://github.com/acme/widgets/pull/9");
    fs::write(root.join("pr-inbox.json"), r#"{"prs":[{"repo":"Acme/Widgets","head":"feat","number":5,"url":"https://github.com/acme/widgets/pull/5"}],"ts":1}"#).unwrap();
    fs::create_dir_all(root.join("viewed")).unwrap();
    (root, home, feat)
}

#[test]
fn the_diff_payload_matches_ruby_scope_by_scope() {
    let base = tempfile::tempdir().unwrap();
    let (root, home, _feat) = diff_world(base.path());
    let ruby = start(false, &root, &home);
    let rust = start(true, &root, &home);
    let mut compared = 0;
    let mut seen = String::new();
    let mut check = |path: &str| {
        let (a, b) = (request(ruby.port, "GET", path, &[], "", true), request(rust.port, "GET", path, &[], "", true));
        if a.status.starts_with("501") {
            return;
        }
        assert_eq!(b.status, a.status, "{path}");
        let (sa, sb) = (shown(&a, ruby.port), shown(&b, rust.port));
        if sa != sb {
            let at = sa.bytes().zip(sb.bytes()).position(|(x, y)| x != y).unwrap_or(sa.len().min(sb.len()));
            let from = at.saturating_sub(200);
            let cut = |s: &str| s.get(from..(from + 500).min(s.len())).unwrap_or("").to_string();
            panic!("{path}: first difference at byte {at}\nRuby: …{}\nRust: …{}", cut(&sa), cut(&sb));
        }
        compared += 1;
        seen.push_str(&sa);
    };
    for (slug, query) in [
        ("diff-item", ""),
        ("diff-item", "?scope=branch"),
        ("diff-item", "?scope=branch&branch=feat"),
        ("diff-item", "?scope=commits&branch=feat"),
        ("diff-item", "?scope=uncommitted&branch=feat"),
        ("diff-item", "?scope=branch&branch=issue%2F12-x"),
        ("diff-item", "?scope=commits&branch=stack-a"),
        ("diff-item", "?scope=branch&branch=stack-a"),
        ("diff-item", "?scope=branch&branch=stack-b"),
        ("diff-item", "?scope=commits&branch=stack-b"),
        ("diff-item", "?scope=uncommitted&branch=stack-b"),
        ("diff-item", "?scope=branch&branch=stack-b&base=main"),
        ("diff-item", "?scope=commits&branch=stack-b&base=stack-a"),
        ("diff-item", "?scope=branch&branch=stack-b&base=nope"),
        ("diff-item", "?scope=branch&branch=refonly"),
        ("diff-item", "?scope=commits&branch=refonly"),
        ("diff-item", "?scope=uncommitted&branch=refonly"),
        ("diff-item", "?scope=branch&branch=remote-only"),
        ("diff-item", "?scope=commits&branch=remote-only"),
        ("diff-item", "?scope=branch&branch=unlinked"),
        ("diff-item", "?scope=bogus"),
        ("diff-item", "?scope="),
        ("stack-item", ""),
        ("stack-item", "?scope=commits"),
        ("diff-item", "?scope=commits&branch=many"),
        ("diff-item", "?scope=branch&branch=many"),
        ("diff-item", "?scope=branch&branch=issue%2F3-y"),
        ("nobranch", ""),
        ("nosuch", ""),
    ] {
        check(&format!("/api/diff/{slug}{query}"));
    }
    // the first load of a stack's item links its other stack branches: Rust must do that too
    let (b, a) = (request(rust.port, "GET", "/api/diff/stack-item-r?scope=branch", &[], "", true), request(ruby.port, "GET", "/api/diff/stack-item-r?scope=branch", &[], "", true));
    assert_eq!(shown(&b, rust.port), shown(&a, ruby.port));
    assert!(fs::read_to_string(root.join("work-items/stack-item-r.md")).unwrap().contains("- stack-b"), "the item should now link the whole stack");
    // a selected run of commits
    let feat_commits = sh(&base.path().join("repo"), &["log", "--format=%H", "main..feat"]);
    let shas: Vec<&str> = feat_commits.lines().collect();
    let (newest, oldest) = (shas[0], shas[1]);
    for query in [
        format!("?scope=commits&branch=feat&from={oldest}&to={oldest}"),
        format!("?scope=commits&branch=feat&from={oldest}&to={newest}"),
        format!("?scope=commits&branch=feat&from={}", &newest[..8]),
        format!("?scope=commits&branch=feat&to={}", &oldest[..7]),
        format!("?scope=commits&branch=feat&from={newest}&to={oldest}"),
        "?scope=commits&branch=feat&from=zzzz".to_string(),
        "?scope=commits&branch=feat&from=abcd1234".to_string(),
        format!("?scope=commits&branch=feat&from={}", sh(&base.path().join("repo"), &["rev-parse", "main"])),
    ] {
        check(&format!("/api/diff/diff-item{query}"));
    }
    // viewed marks ride along
    let first = request(ruby.port, "GET", "/api/diff/diff-item?scope=branch&branch=feat", &[], "", true);
    let payload: serde_json::Value = serde_json::from_slice(&first.body).unwrap();
    let file = &payload["files"][0];
    fs::write(root.join("viewed/diff-item.json"), serde_json::json!({"v": 1, "marks": {file["path"].as_str().unwrap(): [file["digest"]]}}).to_string()).unwrap();
    check("/api/diff/diff-item?scope=branch&branch=feat");
    // without a pr-inbox cache
    fs::remove_file(root.join("pr-inbox.json")).unwrap();
    check("/api/diff/diff-item?scope=branch&branch=feat");
    // the chat's own pane picks the checkout
    let socket = "qm-test-gui";
    let tmux = |args: &[&str]| Command::new("tmux").args(["-L", socket]).args(args).output();
    let _ = tmux(&["kill-server"]);
    if tmux(&["new-session", "-d", "-s", "diff-item", "-c", base.path().join("wt-sa").to_str().unwrap(), "sleep 60"]).is_ok_and(|o| o.status.success()) {
        check("/api/diff/diff-item");
        check("/api/diff/diff-item?scope=commits");
        let _ = tmux(&["kill-server"]);
    }
    assert!(compared > 35, "compared {compared}");
    for expected in ["\"ok\":true", "\"ok\":false", "\"commitStatus\":\"mixed\"", "\"commitStatus\":\"uncommitted\"", "\"commitStatus\":\"committed\"", "\"uncommitted\":true", "\"layers\":[", "\"alsoIn\":\"", "\"fileLayers\"", "\"kind\":\"stack\"", "\"kind\":\"stack-span\"", "\"kind\":\"trunk\"", "\"checkout\":false", "\"truncated\":true", "\"binary\":true", "\"status\":\"renamed\"", "\"status\":\"deleted\"", "\"prAvailable\":true", "\"viewed\":true", "\"chat\":true", "\"range\":{", "\"messages\":[{", "\"commitCount\":205", "\"n\":205"] {
        assert!(seen.contains(expected), "the payloads should contain {expected}");
    }
}

fn git_at(dir: &Path, when: &str, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "user.name=t", "-c", "user.email=t@t.test", "-c", "init.defaultBranch=main"])
        .args(args)
        .env("GIT_AUTHOR_DATE", when)
        .env("GIT_COMMITTER_DATE", when)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Records the worktree as the UserPromptSubmit hook does, then dates the line `at`.
fn snapshot(root: &Path, wt: &Path, session: &str, at: &str) {
    let mut child = Command::new("sh")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("hooks/snapshot-worktree.sh"))
        .env("QM_ROOT", root)
        .env_remove("QM_SKIP")
        .env_remove("QM_ASIDE")
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(format!("{{\"session_id\":\"{session}\",\"cwd\":\"{}\"}}", wt.display()).as_bytes()).unwrap();
    child.wait().unwrap();
    let file = root.join("prompt-snapshots.jsonl");
    let text = fs::read_to_string(&file).unwrap();
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let last = lines.pop().unwrap();
    lines.push(regex::Regex::new(r#""at":"[^"]+""#).unwrap().replace(&last, format!("\"at\":\"{at}\"")).into_owned());
    fs::write(file, lines.join("\n") + "\n").unwrap();
}

fn prompt_world(base: &Path) -> (PathBuf, PathBuf) {
    let (root, home, repo) = (base.join("root"), base.join("home"), base.join("repo"));
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(root.join("work-items")).unwrap();
    let d = |t: &str| format!("2026-04-01T{t}+00:00");
    git_at(&repo, &d("09:00:00"), &["init", "-q", "-b", "main"]);
    fs::write(repo.join("a.txt"), lines("alpha", 10)).unwrap();
    fs::write(repo.join("c.txt"), lines("gamma", 6)).unwrap();
    fs::write(repo.join("d.txt"), lines("delta", 6)).unwrap();
    fs::write(repo.join("gone.txt"), "going away\n").unwrap();
    git_at(&repo, &d("09:00:00"), &["add", "-A"]);
    git_at(&repo, &d("09:00:00"), &["commit", "-q", "-m", "base"]);
    let wt = base.join("wt-chat");
    git_at(&repo, &d("09:01:00"), &["worktree", "add", "-q", "-b", "feat", wt.to_str().unwrap(), "main"]);
    git_at(&repo, &d("09:02:00"), &["branch", "refonly", "main"]);
    git_at(&repo, &d("10:05:00"), &["branch", "before-only", "main"]);
    snapshot(&root, &wt, "s1", "2026-04-01T10:00:00Z");
    fs::write(wt.join("a.txt"), lines("alpha", 10).replace("alpha 3\n", "alpha three changed\n")).unwrap();
    fs::write(wt.join("new.txt"), "a new file\n").unwrap();
    fs::remove_file(wt.join("gone.txt")).unwrap();
    snapshot(&root, &wt, "s1", "2026-04-01T10:10:00Z");
    git_at(&wt, &d("10:15:00"), &["add", "-A"]);
    git_at(&wt, &d("10:15:00"), &["commit", "-q", "-m", "feat one"]);
    fs::write(wt.join("a.txt"), fs::read_to_string(wt.join("a.txt")).unwrap().replace("alpha 7\n", "alpha seven uncommitted\n")).unwrap();
    snapshot(&root, &wt, "s1", "2026-04-01T10:20:00Z");
    fs::write(wt.join("c.txt"), lines("gamma", 6).replace("gamma 2\n", "gamma two mine\n")).unwrap();
    snapshot(&root, &wt, "s1", "2026-04-01T10:30:00Z");
    // work elsewhere: branches that moved since the prompts
    let tree = git_at(&repo, &d("10:31:00"), &["rev-parse", "main^{tree}"]);
    let parent = git_at(&repo, &d("10:31:00"), &["rev-parse", "main"]);
    let other = git_at(&repo, &d("10:32:00"), &["commit-tree", &tree, "-p", &parent, "-m", "other work"]);
    git_at(&repo, &d("10:32:00"), &["update-ref", "-m", "other moves", "refs/heads/other", &other]);
    git_at(&repo, &d("10:33:00"), &["update-ref", "-m", "branch: Created", "refs/heads/created-later", &other]);
    let before_only = git_at(&repo, &d("10:05:00"), &["commit-tree", &tree, "-p", &parent, "-m", "before only"]);
    git_at(&repo, &d("10:06:00"), &["update-ref", "-m", "moved early", "refs/heads/before-only", &before_only]);
    let (mut tip, mut n) = (parent.clone(), 0);
    for (when, name) in [("10:20:00", "a"), ("10:30:00", "b"), ("10:34:00", "c")] {
        n += 1;
        tip = git_at(&repo, &d(when), &["commit-tree", &tree, "-p", &tip, "-m", &format!("at prompt {name}{n}")]);
        git_at(&repo, &d(when), &["update-ref", "-m", "step", "refs/heads/at-prompt", &tip]);
    }
    git_at(&repo, &d("10:30:00"), &["update-ref", "-m", "exactly at the prompt", "refs/heads/at-exact", &parent]);
    // the trunk moves on, and the chat's branch is rebased onto it
    fs::write(repo.join("d.txt"), lines("delta", 6).replace("delta 5\n", "delta five upstream\n")).unwrap();
    git_at(&repo, &d("10:35:00"), &["commit", "-q", "-a", "-m", "m1"]);
    fs::write(repo.join("c.txt"), lines("gamma", 6).replace("gamma 2\n", "gamma two theirs\n")).unwrap();
    git_at(&repo, &d("10:36:00"), &["commit", "-q", "-a", "-m", "m2"]);
    git_at(&wt, &d("10:39:00"), &["stash", "-q"]);
    git_at(&wt, &d("10:40:00"), &["rebase", "-q", "main"]);
    git_at(&wt, &d("10:41:00"), &["stash", "drop", "-q"]);
    fs::write(wt.join("a.txt"), fs::read_to_string(wt.join("a.txt")).unwrap().replace("alpha 9\n", "alpha nine after\n")).unwrap();
    fs::write(wt.join("after.txt"), "after the rebase\n").unwrap();

    // the transcript the prompts come from, and the log of sessions
    let transcript = base.join("s1.jsonl");
    let user = |ts: &str, text: &str| serde_json::json!({"type": "user", "timestamp": ts, "message": {"content": text}}).to_string();
    let long = format!("{}\nsecond line of the prompt", "A very long first line of a prompt that goes on and on past the eighty character limit for labels ");
    fs::write(
        &transcript,
        [
            serde_json::json!({"type": "user", "timestamp": "2026-04-01T10:00:00.500Z", "isSidechain": true, "message": {"content": "a sidechain prompt that comes first"}}).to_string(),
            user("2026-04-01T10:00:01.000Z", "first prompt"),
            serde_json::json!({"type": "user", "timestamp": "2026-04-01T10:05:00.000Z", "isSidechain": true, "message": {"content": "a sidechain prompt"}}).to_string(),
            user("2026-04-01T10:10:02.500Z", &long),
            user("2026-04-01T10:20:01.000Z", "[Request interrupted by user]"),
            serde_json::json!({"type": "user", "timestamp": "2026-04-01T10:21:00.000Z", "isCompactSummary": true, "message": {"content": "summary"}}).to_string(),
            serde_json::json!({"type": "user", "timestamp": "2026-04-01T10:22:00.000Z", "toolUseResult": {"x": 1}, "message": {"content": "tool output"}}).to_string(),
            user("2026-04-01T10:29:57.500Z", "fourth prompt, a couple of seconds before its snapshot"),
            "not json at all".to_string(),
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();
    let me = std::process::id();
    let start = |id: &str, pid: u32, cwd: &Path, ts: &str, transcript: &str| format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"{}\",\"pid\":{pid},\"transcript_path\":\"{transcript}\",\"ts\":\"{ts}\"}}\n", cwd.display());
    fs::write(
        root.join("sessions.jsonl"),
        start("s1", me, &wt, "2026-04-01T10:00:00+00:00", transcript.to_str().unwrap())
            + &start("s0", 999_999, &wt, "2026-03-01T10:00:00+00:00", "/nonexistent.jsonl")
            + &start("s2", 999_999, &wt, "2026-03-02T10:00:00+00:00", "/nonexistent.jsonl")
            + &start("s3", 999_999, &wt, "2026-03-03T10:00:00+00:00", "/nonexistent.jsonl")
            + &start("s4", 999_999, &wt, "2026-03-04T10:00:00+00:00", "/nonexistent.jsonl")
            + &start("s5", me, &wt, "2026-03-10T10:00:00+00:00", "/nonexistent.jsonl")
            + &start("s6", 999_999, &wt, "2026-03-11T10:00:00+00:00", "/nonexistent.jsonl"),
    )
    .unwrap();
    let mut snaps = fs::read_to_string(root.join("prompt-snapshots.jsonl")).unwrap();
    let t0 = git_at(&wt, &d("11:00:00"), &["rev-parse", "HEAD^{tree}"]);
    snaps += &format!("{{\"v\":1,\"session_id\":\"s0\",\"cwd\":\"{w}\",\"toplevel\":\"{w}\",\"tree\":\"{t0}\",\"at\":\"2026-03-01T10:00:00Z\"}}\n", w = wt.display());
    for s in ["s5", "s6"] {
        snaps += &format!("{{\"v\":1,\"session_id\":\"{s}\",\"cwd\":\"{w}\",\"toplevel\":\"{w}\",\"tree\":\"{t0}\",\"at\":\"2026-03-12T10:00:00Z\"}}\n", w = wt.display());
    }
    snaps += &format!("{{\"v\":1,\"session_id\":\"s3\",\"cwd\":\"/nowhere/at/all\",\"toplevel\":\"/nowhere/at/all\",\"tree\":\"{t0}\",\"at\":\"2026-03-03T10:00:00Z\"}}\n");
    snaps += &format!("{{\"v\":1,\"session_id\":\"s4\",\"cwd\":\"{w}\",\"toplevel\":\"{w}\",\"tree\":\"{}\",\"at\":\"2026-03-04T10:00:00Z\"}}\n", "ab".repeat(20), w = wt.display());
    snaps += &format!("{{\"v\":1,\"session_id\":\"s1\",\"toplevel\":\"{w}\",\"tree\":\"not a tree\",\"at\":\"2026-04-01T10:40:00Z\"}}\n{{\"v\":1,\"session_id\":\"s1\",\"tree\":\"{t0}\",\"at\":\"nonsense\"}}\n{{\"v\":1,\"session_id\":\"s1\",\"toplevel\":\"{w}\",\"tre", w = wt.display());
    fs::write(root.join("prompt-snapshots.jsonl"), snaps).unwrap();
    fs::write(root.join("config.yml"), format!("projects:\n  demo: {}\n", repo.display())).unwrap();
    let item = |slug: &str, sessions: &[&str], branches: &[&str]| {
        let s: String = sessions.iter().map(|s| format!("- id: {s}\n")).collect();
        let s = if s.is_empty() { String::new() } else { format!("sessions:\n{s}") };
        let b: String = branches.iter().map(|b| format!("- {b}\n")).collect();
        let b = if b.is_empty() { String::new() } else { format!("branches:\n{b}") };
        fs::write(root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\ntitle: {slug}\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\n{b}{s}---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n")).unwrap();
    };
    item("chat-item", &["s1", "s0"], &["feat", "refonly"]);
    item("nosession", &[], &["feat"]);
    item("nosnap", &["s2"], &[]);
    item("gone-item", &["s3"], &[]);
    item("pruned-item", &["s4"], &[]);
    item("uncaptured-item", &["never-logged"], &[]);
    item("live-older", &["s6", "s5"], &[]);
    (root, home)
}

#[test]
fn the_prompt_scope_matches_ruby() {
    let base = tempfile::tempdir().unwrap();
    let (root, home) = prompt_world(base.path());
    let ruby = start(false, &root, &home);
    let rust = start(true, &root, &home);
    let mut seen = String::new();
    let mut compared = 0;
    let mut check = |path: &str| {
        let (a, b) = (request(ruby.port, "GET", path, &[], "", true), request(rust.port, "GET", path, &[], "", true));
        assert!(!b.status.starts_with("501"), "{path} should be served: {}", String::from_utf8_lossy(&b.body));
        let (sa, sb) = (shown(&a, ruby.port), shown(&b, rust.port));
        if sa != sb {
            let at = sa.bytes().zip(sb.bytes()).position(|(x, y)| x != y).unwrap_or(sa.len().min(sb.len()));
            let from = at.saturating_sub(200);
            let cut = |s: &str| s.get(from..(from + 500).min(s.len())).unwrap_or("").to_string();
            panic!("{path}: first difference at byte {at}\nRuby: …{}\nRust: …{}", cut(&sa), cut(&sb));
        }
        compared += 1;
        seen.push_str(&sa);
    };
    let q = "/api/diff/chat-item?scope=prompt";
    for extra in [
        "", "&prompt=s1:1", "&prompt=s1:2", "&prompt=s1:3", "&prompt=s1:4", "&prompt=s1:5", "&prompt=s1:0", "&prompt=s1:%2B2", "&prompt=s1:0x2", "&prompt=s1:_2", "&prompt=s1:2_", "&prompt=s1:2_0", "&prompt=s9:1", "&prompt=nonsense",
        "&prompt=s1:1&rebase=show", "&prompt=s1:2&rebase=show", "&prompt=s1:4&rebase=show", "&rebase=show",
        "&prompt=s1:1&moved=other", "&prompt=s1:2&moved=other", "&prompt=s1:2&moved=created-later", "&prompt=s1:1&moved=before-only", "&prompt=s1:2&moved=before-only", "&prompt=s1:2&moved=feat", "&prompt=s1:2&moved=nope",
        "&prompt=s1:2&moved=at-prompt", "&prompt=s1:4&moved=at-prompt", "&prompt=s1:2&moved=other&rebase=show", "&prompt=s1:2&moved=main", "&branch=feat", "&branch=refonly", "&branch=unlinked",
    ] {
        check(&format!("{q}{extra}"));
    }
    for slug in ["nosession", "nosnap", "gone-item", "pruned-item", "uncaptured-item", "live-older", "nosuch"] {
        check(&format!("/api/diff/{slug}?scope=prompt"));
    }
    assert!(compared > 30);
    // git 2.43 refuses a tree where `merge-tree --write-tree` wants a commit, so replaying the prompt's
    // tree onto the new base fails here, in Ruby too ("hidden": true and the conflict notice need a
    // git that accepts it); the failure path and the unhidden (`rebase=show`) one are what is compared.
    for expected in ["\"kind\":\"prompt\"", "\"kind\":\"prompt-branch\"", "\"rebase\":{\"onto\":", "\"hidden\":false", "\"rebaseNotice\":\"Couldn't hide the rebase: git merge-tree failed.\"", "\"movedBranches\":[{\"branch\":\"feat\",\"commits\":3},{\"branch\":\"main\"", "\"branch\":\"created-later\",\"commits\":0", "\"prompt\":null", "before “first prompt”", "before the prompt at 2026-04-01T10:20:00Z", "…\"", "\"notice\":\"This item has no captured chat session", "\"notice\":\"No prompt snapshots", "is gone — nothing to diff.", "snapshot is gone: git pruned it", "\"oldRev\":\"", "is not checked out anywhere"] {
        assert!(seen.contains(expected), "the payloads should contain {expected}");
    }
}

/// A repo whose origin is `<base>/acme/widgets.git` (so its GitHub name is acme/widgets and `git
/// fetch origin` works), with PR heads published as `refs/pull/<n>/head`.
fn pr_world(base: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let (root, home, repo) = (base.join("root"), base.join("home"), base.join("repo"));
    let origin = base.join("acme/widgets.git");
    for d in [&home, &repo, &origin, &base.join("bin"), &base.join("ghdata")] {
        fs::create_dir_all(d).unwrap();
    }
    sh(&origin, &["init", "-q", "--bare"]);
    sh(&repo, &["init", "-q", "-b", "main"]);
    fs::write(repo.join("a.txt"), lines("alpha", 30)).unwrap();
    fs::write(repo.join("b.txt"), lines("beta", 10)).unwrap();
    fs::write(repo.join("gone.txt"), lines("gone", 4)).unwrap();
    fs::write(repo.join("far.txt"), lines("far", 40)).unwrap();
    commit(&repo, "base");
    sh(&repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
    sh(&repo, &["push", "-q", "origin", "main"]);
    let feat = base.join("wt-feat");
    sh(&repo, &["worktree", "add", "-q", "-b", "feat", feat.to_str().unwrap(), "main"]);
    fs::write(feat.join("a.txt"), lines("alpha", 30).replace("alpha 5\n", "alpha five\n").replace("alpha 25\n", "alpha twenty-five\n").replace("alpha 12\n", "alpha twelve\n").replace("alpha 20\n", "alpha twenty\n")).unwrap();
    commit(&feat, "feat one: two edits far apart");
    fs::write(feat.join("b.txt"), lines("beta", 10).replace("beta 3\n", "")).unwrap();
    fs::remove_file(feat.join("gone.txt")).unwrap();
    fs::write(feat.join("new.txt"), lines("new", 3)).unwrap();
    fs::write(feat.join("dup.txt"), "dup\ndup\ndup\n").unwrap();
    fs::write(feat.join("far.txt"), lines("far", 40).replace("far 2\n", "far two\n").replace("far 38\n", "far thirty-eight\n")).unwrap();
    commit(&feat, "feat two: removals and a new file");
    fs::write(feat.join("a.txt"), fs::read_to_string(feat.join("a.txt")).unwrap().replace("alpha 6\n", "alpha 6\nalpha 6b\n").replace("alpha five\n", "")).unwrap();
    commit(&feat, "feat three");
    sh(&repo, &["push", "-q", "origin", "feat:refs/pull/5/head"]);
    // a second PR onto another base, found through gh
    sh(&repo, &["checkout", "-q", "-b", "release", "main"]);
    fs::write(repo.join("rel.txt"), lines("release", 2)).unwrap();
    commit(&repo, "release one");
    sh(&repo, &["push", "-q", "origin", "release"]);
    sh(&repo, &["checkout", "-q", "-b", "feat2", "release"]);
    fs::write(repo.join("two.txt"), lines("two", 5)).unwrap();
    commit(&repo, "feat2 one");
    sh(&repo, &["push", "-q", "origin", "feat2:refs/pull/6/head"]);
    sh(&repo, &["checkout", "-q", "main"]);
    sh(&repo, &["branch", "nopr", "main"]);
    sh(&repo, &["branch", "gone-pr", "main"]);
    sh(&repo, &["branch", "wrong-repo", "main"]);
    sh(&repo, &["branch", "bad-base", "main"]);
    sh(&repo, &["branch", "gh-fails", "main"]);
    sh(&repo, &["branch", "closed-br", "main"]);
    sh(&repo, &["branch", "dots-base", "main"]);
    // gh answers from files; every call is logged to the server's own log
    let gh = "#!/bin/sh\necho \"$*\" >> \"$FAKE_GH_LOG\"\nsel=\"$3\"\nf=\"$FAKE_GH_DATA/$(echo \"$sel\" | tr '/:' '__').json\"\nif [ -f \"$f\" ]; then cat \"$f\"; exit 0; fi\nif [ -f \"$FAKE_GH_DATA/$sel.fail\" ]; then echo 'HTTP 502: bad gateway' >&2; exit 1; fi\necho \"no pull requests found for branch $sel\" >&2\nexit 1\n";
    fs::write(base.join("bin/gh"), gh).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(base.join("bin/gh"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    let pr = |n: u32, head: &str, base_ref: &str| format!("{{\"number\":{n},\"url\":\"https://github.com/acme/widgets/pull/{n}\",\"title\":\"PR {n}\",\"headRefName\":\"{head}\",\"baseRefName\":\"{base_ref}\",\"state\":\"OPEN\"}}");
    fs::write(base.join("ghdata/feat2.json"), pr(6, "feat2", "release")).unwrap();
    fs::write(base.join("ghdata/https___github.com_acme_widgets_pull_6.json"), pr(6, "feat2", "release")).unwrap();
    fs::write(base.join("ghdata/gh-fails.fail"), "").unwrap();
    fs::create_dir_all(root.join("work-items")).unwrap();
    fs::write(root.join("config.yml"), format!("projects:\n  demo: {}\n", repo.display())).unwrap();
    let item = |slug: &str, title: &str, branches: &[&str], desc: &str| {
        let list: String = branches.iter().map(|b| format!("- {b}\n")).collect();
        let list = if list.is_empty() { String::new() } else { format!("branches:\n{list}") };
        fs::write(root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\ntitle: \"{title}\"\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\n{list}---\n\n## Description\n\n{desc}\n\n## Notes\n\n## Snapshots\n")).unwrap();
    };
    item("pr-item", "PR item", &["feat", "feat2", "nopr", "gone-pr", "wrong-repo", "bad-base", "gh-fails", "closed-br", "dots-base"], "d");
    item("review-cached", "Review acme/widgets#5: x", &[], "Review of pull request https://github.com/acme/widgets/pull/5");
    item("review-gh", "Review acme/widgets#6: y", &[], "Review of pull request https://github.com/acme/widgets/pull/6");
    item("review-branch", "Review acme/widgets#5: z", &["feat"], "Review of pull request https://github.com/acme/widgets/pull/5");
    item("review-unknown", "Review acme/widgets#77: w", &[], "Review of pull request https://github.com/acme/widgets/pull/77");
    item("draft-item", "Draft item", &["feat"], "d");
    item("review-other", "Review other/repo#3: v", &[], "Review of pull request https://github.com/other/repo/pull/3");
    let real_git = String::from_utf8_lossy(&Command::new("sh").args(["-c", "command -v git"]).output().unwrap().stdout).trim().to_string();
    fs::write(base.join("bin/git"), format!("#!/bin/sh\ncase \" $* \" in *\" fetch \"*) echo \"$*\" | sed 's#-C [^ ]* ##' >> \"$FAKE_GIT_LOG\" ;; esac\nexec {real_git} \"$@\"\n")).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(base.join("bin/git"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(base.join("ghdata/closed-br.json"), pr(8, "closed-br", "main").replace("OPEN", "MERGED")).unwrap();
    let prs = serde_json::json!([
        {"repo": "Acme/Widgets", "number": 5, "url": "https://github.com/acme/widgets/pull/5", "title": "PR 5", "head": "feat", "base": "main"},
        {"repo": "acme/widgets", "number": 99, "url": "https://github.com/acme/widgets/pull/99", "title": "PR 99", "head": "gone-pr", "base": "main"},
        {"repo": "other/repo", "number": 3, "url": "https://github.com/other/repo/pull/3", "title": "PR 3", "head": "wrong-repo", "base": "main"},
        {"repo": "acme/widgets", "number": 4, "url": "https://github.com/acme/widgets/pull/4", "title": "PR 4", "head": "bad-base", "base": "-evil"},
        {"repo": "acme/widgets", "number": 7, "url": "https://github.com/acme/widgets/pull/7", "title": "PR 7", "head": "dots-base", "base": "a..b"}
    ]);
    fs::write(root.join("pr-inbox.json"), serde_json::json!({"prs": prs, "ts": 1}).to_string()).unwrap();
    (root, home, base.to_path_buf())
}

#[test]
fn the_pr_scope_matches_ruby() {
    let base = tempfile::tempdir().unwrap();
    let (root, home, b) = pr_world(base.path());
    let env = |tag: &str| -> Vec<(&'static str, String)> {
        vec![("PATH", format!("{}:{}", b.join("bin").display(), std::env::var("PATH").unwrap())), ("FAKE_GH_DATA", b.join("ghdata").display().to_string()), ("FAKE_GH_LOG", b.join(format!("gh-{tag}.log")).display().to_string()), ("FAKE_GIT_LOG", b.join(format!("git-{tag}.log")).display().to_string())]
    };
    let ruby = start_env(false, &root, &home, &env("ruby"));
    let rust = start_env(true, &root, &home, &env("rust"));
    let mut seen = String::new();
    let mut compared = 0;
    let mut check = |path: &str| {
        let (a, c) = (request(ruby.port, "GET", path, &[], "", true), request(rust.port, "GET", path, &[], "", true));
        assert!(!c.status.starts_with("501"), "{path} should be served: {}", String::from_utf8_lossy(&c.body));
        let (sa, sc) = (shown(&a, ruby.port), shown(&c, rust.port));
        if sa != sc {
            let at = sa.bytes().zip(sc.bytes()).position(|(x, y)| x != y).unwrap_or(sa.len().min(sc.len()));
            let from = at.saturating_sub(200);
            let cut = |s: &str| s.get(from..(from + 500).min(s.len())).unwrap_or("").to_string();
            panic!("{path}: first difference at byte {at}\nRuby: …{}\nRust: …{}", cut(&sa), cut(&sc));
        }
        compared += 1;
        seen.push_str(&sa);
    };
    let repo = base.path().join("repo");
    let shas = sh(&repo, &["log", "--format=%H", "main..feat"]);
    let shas: Vec<&str> = shas.lines().collect(); // newest first
    let (c3, c2, c1) = (shas[0], shas[1], shas[2]);
    for query in [
        "?scope=pr&branch=feat".to_string(),
        "?scope=pr&branch=feat&fresh=1".to_string(),
        "?scope=pr&branch=feat".to_string(),
        format!("?scope=pr&branch=feat&from={c1}&to={c1}"),
        format!("?scope=pr&branch=feat&from={c2}&to={c2}"),
        format!("?scope=pr&branch=feat&from={c1}&to={c3}"),
        format!("?scope=pr&branch=feat&from={c3}&to={c1}"),
        "?scope=pr&branch=feat&from=zzzz".to_string(),
        "?scope=pr&branch=feat2".to_string(),
        "?scope=pr&branch=nopr".to_string(),
        "?scope=pr&branch=gone-pr".to_string(),
        "?scope=pr&branch=wrong-repo".to_string(),
        "?scope=pr&branch=bad-base".to_string(),
        "?scope=pr&branch=gh-fails".to_string(),
        "?scope=pr&branch=closed-br".to_string(),
        "?scope=pr&branch=dots-base".to_string(),
        "?scope=pr&branch=unlinked".to_string(),
        "?scope=pr".to_string(),
        "?scope=branch&branch=feat".to_string(),
    ] {
        check(&format!("/api/diff/pr-item{query}"));
    }
    for slug in ["review-cached", "review-gh", "review-branch", "review-unknown", "review-other"] {
        for query in ["", "?scope=pr", "?scope=branch", "?scope=commits", "?scope=uncommitted"] {
            check(&format!("/api/diff/{slug}{query}"));
        }
    }
    // the same questions asked of gh and of git, by each server, with the memo in between
    let calls = |tag: &str| fs::read_to_string(b.join(format!("gh-{tag}.log"))).unwrap_or_default();
    assert_eq!(calls("rust"), calls("ruby"), "gh must be asked the same things");
    assert!(!calls("ruby").is_empty());
    let fetches = |tag: &str| fs::read_to_string(b.join(format!("git-{tag}.log"))).unwrap_or_default();
    assert_eq!(fetches("rust"), fetches("ruby"), "git must be asked to fetch the same things");
    assert!(fetches("ruby").contains("fetch --quiet --no-tags origin +refs/pull/5/head:refs/quartermaster/pr/5 +refs/heads/main:refs/remotes/origin/main"));
    assert!(compared > 40);
    for expected in ["\"scope\":\"pr\"", "\"kind\":\"pr\"", "\"reviewRoutes\":{}", "\"anchor\":\"", "\"gh\":false", "\"messages\":[{", "has no open pull request", "is not in this project's repo", "has no usable base branch", "could not fetch", "gh pr view gh-fails failed: HTTP 502", "\"prAvailable\":true", "\"checkout\":false"] {
        assert!(seen.contains(expected), "the payloads should contain {expected}");
    }
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap().filter_map(|e| e.ok()) {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            copy_dir(&src, &dst)
        } else {
            fs::copy(&src, &dst).unwrap();
            let when = fs::metadata(&src).unwrap().modified().unwrap();
            fs::OpenOptions::new().write(true).open(&dst).unwrap().set_modified(when).unwrap();
        }
    }
}

#[test]
fn github_drafts_are_re_anchored_like_ruby() {
    let base = tempfile::tempdir().unwrap();
    let (root, home, b) = pr_world(base.path());
    let repo = base.path().join("repo");
    let shas = sh(&repo, &["log", "--format=%H", "main..feat"]);
    let shas: Vec<&str> = shas.lines().collect();
    let (c3, c2, c1) = (shas[0].to_string(), shas[1].to_string(), shas[2].to_string());
    let base_sha = sh(&repo, &["merge-base", "main", "feat"]);
    // a commit outside the PR with the patch of its first commit (a rebase that kept the changes)
    sh(&repo, &["checkout", "-q", "-b", "side", "main"]);
    sh(&repo, &["cherry-pick", "-x", &c1]);
    let twin = sh(&repo, &["rev-parse", "HEAD"]);
    sh(&repo, &["checkout", "-q", "main"]);
    let entry = |id: &str, extra: serde_json::Value| {
        let mut e = serde_json::json!({"id": id, "kind": "github-comment", "comment": format!("comment {id}"), "rev": 1, "path": "a.txt", "side": "new", "line": 12, "code": "alpha twelve", "repo": "acme/widgets", "number": 5, "commit": c3, "base": base_sha, "subject": "line", "author": "user", "createdAt": "2026-04-01T00:00:00Z", "updatedAt": "2026-04-01T00:00:00Z"});
        for (k, v) in extra.as_object().unwrap() {
            if v.is_null() { e.as_object_mut().unwrap().remove(k); } else { e[k] = v.clone(); }
        }
        e
    };
    let entries = serde_json::json!([
        entry("head-same", serde_json::json!({})),
        entry("moved-by-push", serde_json::json!({"line": 99, "code": "alpha twenty", "commit": c1})),
        entry("moved-range", serde_json::json!({"line": 99, "start_line": 98, "code": "alpha 11\nalpha twelve", "commit": c1})),
        entry("moved-range-sides", serde_json::json!({"line": 99, "start_line": 98, "start_side": "old", "code": "alpha 11\nalpha twelve", "commit": c1})),
        entry("moved-range-bad-start", serde_json::json!({"line": 99, "start_line": 98, "code": "wrong first\nalpha twelve", "commit": c1})),
        entry("ambiguous", serde_json::json!({"path": "dup.txt", "line": 99, "code": "dup", "commit": c1})),
        entry("head-old-commit", serde_json::json!({"commit": c1, "base": "fffffff"})),
        entry("outdated-range-context", serde_json::json!({"start_line": 10, "code": "alpha 10\nalpha 11\nalpha 12 as it was", "commit": c1})),
        entry("moved-failed", serde_json::json!({"line": 98, "code": "alpha twelve", "commit": c1, "failed": "GitHub said no"})),
        entry("on-commit", serde_json::json!({"line": 5, "code": "alpha five", "commit": c1, "range": "aaa..bbb"})),
        entry("on-twin", serde_json::json!({"line": 5, "code": "alpha five", "commit": twin, "range": "aaa..bbb"})),
        entry("on-unknown", serde_json::json!({"commit": "1234567890abcdef1234567890abcdef12345678", "range": "aaa..bbb"})),
        entry("pinned", serde_json::json!({"line": 12, "commit": c1, "pin": "commit"})),
        entry("pinned-gone", serde_json::json!({"line": 77, "commit": c1, "pin": "commit"})),
        entry("file-in-pr", serde_json::json!({"subject": "file", "line": 1})),
        entry("file-deleted", serde_json::json!({"subject": "file", "path": "gone.txt"})),
        entry("file-not-in-pr", serde_json::json!({"subject": "file", "path": "nowhere.txt"})),
        entry("outdated-text", serde_json::json!({"line": 99, "code": "text that is nowhere", "commit": c2})),
        entry("outdated-context", serde_json::json!({"line": 12, "code": "alpha 12 as it was", "commit": c1})),
        entry("outdated-old-commit", serde_json::json!({"line": 5, "code": "alpha five", "commit": c1})),
        entry("outdated-file-gone", serde_json::json!({"path": "nowhere.txt", "line": 3, "code": "x", "commit": c1})),
        entry("range-same-hunk", serde_json::json!({"start_line": 10, "code": "alpha 10\nalpha 11\nalpha twelve"})),
        entry("range-two-hunks", serde_json::json!({"path": "far.txt", "line": 38, "start_line": 2, "code": "far two\nfar thirty-eight"})),
        entry("old-side", serde_json::json!({"path": "b.txt", "side": "old", "line": 3, "code": "beta 3"})),
        entry("old-side-moved", serde_json::json!({"path": "b.txt", "side": "old", "line": 8, "code": "beta 3", "commit": c1})),
        entry("string-line", serde_json::json!({"line": "12"})),
        entry("other-pr", serde_json::json!({"number": 6})),
        entry("other-repo", serde_json::json!({"repo": "other/repo"})),
        {"id": "a-chat-draft", "kind": "diff-comment", "comment": "x", "rev": 1, "path": "a.txt", "side": "new", "line": 1}
    ]);
    let write = |dir: &Path, slug: &str, entries: &serde_json::Value| {
        fs::create_dir_all(dir.join("drafts")).unwrap();
        fs::write(dir.join(format!("drafts/{slug}.json")), serde_json::to_string_pretty(&serde_json::json!({"v": 1, "entries": entries})).unwrap()).unwrap();
    };
    let bad = serde_json::json!([entry("bad-side", serde_json::json!({"side": "sideways", "line": 99, "commit": c1}))]);
    let root2 = base.path().join("root2");
    for r in [&root] {
        write(r, "draft-item", &entries);
        write(r, "draft-bad", &bad);
        let item = fs::read_to_string(r.join("work-items/draft-item.md")).unwrap().replace("title: \"Draft item\"", "title: \"Draft bad\"");
        fs::write(r.join("work-items/draft-bad.md"), item).unwrap();
    }
    copy_dir(&root, &root2);
    let env = |tag: &str| -> Vec<(&'static str, String)> {
        vec![("PATH", format!("{}:{}", b.join("bin").display(), std::env::var("PATH").unwrap())), ("FAKE_GH_DATA", b.join("ghdata").display().to_string()), ("FAKE_GH_LOG", b.join(format!("gh-{tag}.log")).display().to_string()), ("FAKE_GIT_LOG", b.join(format!("git-{tag}.log")).display().to_string())]
    };
    let ruby = start_env(false, &root, &home, &env("ruby"));
    let rust = start_env(true, &root2, &home, &env("rust"));
    let mut seen = String::new();
    for path in ["/api/diff/draft-item?scope=pr&branch=feat", "/api/diff/draft-item?scope=pr&branch=feat", "/api/diff/draft-item?scope=pr&branch=feat&fresh=1", &format!("/api/diff/draft-item?scope=pr&branch=feat&from={c1}&to={c2}"), "/api/diff/draft-bad?scope=pr&branch=feat"] {
        let (a, c) = (request(ruby.port, "GET", path, &[], "", true), request(rust.port, "GET", path, &[], "", true));
        assert!(!c.status.starts_with("501"), "{path} should be served: {}", String::from_utf8_lossy(&c.body));
        let (sa, sc) = (shown(&a, ruby.port), shown(&c, rust.port));
        if sa != sc {
            let at = sa.bytes().zip(sc.bytes()).position(|(x, y)| x != y).unwrap_or(sa.len().min(sc.len()));
            let from = at.saturating_sub(300);
            let cut = |s: &str| s.get(from..(from + 700).min(s.len())).unwrap_or("").to_string();
            panic!("{path}: first difference at byte {at}\nRuby: …{}\nRust: …{}", cut(&sa), cut(&sc));
        }
        seen.push_str(&sa);
    }
    // the drafts each server left behind
    let left = |r: &Path| {
        let text = fs::read_to_string(r.join("drafts/draft-item.json")).unwrap();
        regex::Regex::new(r#""updatedAt": "[^"]+""#).unwrap().replace_all(&text, "\"updatedAt\": \"T\"").into_owned()
    };
    assert_eq!(left(&root2), left(&root), "the re-anchored drafts differ");
    assert_ne!(left(&root), regex::Regex::new(r#""updatedAt": "[^"]+""#).unwrap().replace_all(&serde_json::to_string_pretty(&serde_json::json!({"v": 1, "entries": entries})).unwrap(), "\"updatedAt\": \"T\"").into_owned(), "drafts should have moved");
    for expected in ["\"route\":\"head\"", "\"route\":\"commit\"", "\"route\":\"outdated\"", "\"moved\":true", "\"fileGone\":true", "\"oldCommit\":\"", "\"position\":", "no longer in the pull request", "its commit's line can't take a comment any more", "since the push its line isn't in the PR's diff once, unchanged", "\"at\":true", "side must be LEFT or RIGHT, not \\\"SIDEWAYS\\\""] {
        assert!(seen.contains(expected), "the payloads should contain {expected}");
    }
}

/// What a state build is compared on: everything but the terminal token.
fn compare_state(ruby: &Server, rust: &Server, label: &str) -> String {
    let (a, b) = (request(ruby.port, "GET", "/api/state", &[], "", true), request(rust.port, "GET", "/api/state", &[], "", true));
    assert!(!b.status.starts_with("501"), "{label}: the state should be served: {}", String::from_utf8_lossy(&b.body));
    let (sa, sb) = (shown(&a, ruby.port), shown(&b, rust.port));
    let (sa, sb) = (regex::Regex::new(r#""terminalToken":"[0-9a-f]{48}""#).unwrap().replace_all(&sa, "\"terminalToken\":\"T\"").into_owned(), regex::Regex::new(r#""terminalToken":"[0-9a-f]{48}""#).unwrap().replace_all(&sb, "\"terminalToken\":\"T\"").into_owned());
    // a job's estimate is stamped when it ran: seconds apart between the two servers
    let (sa, sb) = (regex::Regex::new(r#""taken_at":"[^"]*""#).unwrap().replace_all(&sa, "\"taken_at\":\"T\"").into_owned(), regex::Regex::new(r#""taken_at":"[^"]*""#).unwrap().replace_all(&sb, "\"taken_at\":\"T\"").into_owned());
    let sb = sb.replace("/root2", "/root");
    if sa != sb {
        let at = sa.bytes().zip(sb.bytes()).position(|(x, y)| x != y).unwrap_or(sa.len().min(sb.len()));
        let from = at.saturating_sub(300);
        let cut = |s: &str| s.get(from..(from + 700).min(s.len())).unwrap_or("").to_string();
        panic!("{label}: first difference at byte {at}\nRuby: …{}\nRust: …{}", cut(&sa), cut(&sb));
    }
    sa
}

#[test]
fn the_state_matches_ruby() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let env: Vec<(&str, String)> = vec![
        ("HOME", w.home.display().to_string()),
        ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()),
        ("PATH", format!("{}:{}", w.bin.display(), std::env::var("PATH").unwrap())),
        ("FAKE_GH_DATA", w.gh_data.display().to_string()),
        ("FAKE_GH_LOG", base.path().join("gh.log").display().to_string()),
    ];
    // each server gets its own copy of the data root: a build rewrites caches and links branches
    let root2 = base.path().join("root2");
    copy_dir(&w.root, &root2);
    let mut env_rust = env.clone();
    env_rust[4].1 = base.path().join("gh-rust.log").display().to_string();
    let ruby = start_env(false, &w.root, &w.home, &env);
    let rust = start_env(true, &root2, &w.home, &env_rust);
    // the first build starts the background PR lookups (a race of its own); compare once they landed
    for s in [&ruby, &rust] {
        request(s.port, "GET", "/api/state", &[], "", true);
    }
    std::thread::sleep(std::time::Duration::from_millis(800));
    let first = compare_state(&ruby, &rust, "first build");
    assert!(first.contains("\"items\":[{"));
    compare_state(&ruby, &rust, "second build");
}

/// Everything the attention world leaves out: usage transcripts, events, notes of every kind,
/// rotations, an aside, inbox sessions, drafts, issues, rate limits and a linked worktree.
fn richer_state_world(w: &common::AttentionWorld, base: &Path) {
    let now = std::time::SystemTime::now();
    let stamp = |secs: u64| -> String {
        let t: chrono::DateTime<chrono::Utc> = (now - std::time::Duration::from_secs(secs)).into();
        t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    };
    let me = std::process::id();
    let put = |path: PathBuf, text: String| {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    let user = |secs: u64, text: &str| format!("{}\n", serde_json::json!({"type": "user", "timestamp": stamp(secs), "message": {"role": "user", "content": text}}));
    let assistant = |secs: u64, model: &str, input: u64, out: u64, read: u64| {
        format!("{}\n", serde_json::json!({"type": "assistant", "timestamp": stamp(secs), "message": {"model": model, "usage": {"input_tokens": input, "output_tokens": out, "cache_read_input_tokens": read, "cache_creation_input_tokens": 50}}}))
    };
    let tpath = |id: &str| w.transcripts.join(format!("p/{id}.jsonl"));
    // usage: a priced model, an unpriced one, and a very large session
    put(tpath("d-fresh"), user(300, "first prompt of the fresh session") + &assistant(280, "claude-opus-5-5", 1000, 2000, 30_000) + &assistant(200, "claude-opus-5-5", 1500, 900, 40_000));
    put(tpath("d-live"), user(250, "live prompt") + &assistant(240, "mystery-model-9", 10, 20, 30) + &assistant(100, "claude-sonnet-5-5", 700_000, 400_000, 0));
    put(tpath("q-recent"), user(86_000, "quiet one") + &assistant(85_000, "claude-haiku-5-5", 5, 6, 7));
    // asides and inbox sessions, an extra live session for events
    let mut log = fs::read_to_string(w.root.join("sessions.jsonl")).unwrap();
    let start = |id: &str, extra: &str, pid: u32, secs: u64| format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"/srv/app\",\"pid\":{pid},\"transcript_path\":\"{}\"{extra},\"ts\":\"{}\"}}\n", tpath(id).display(), stamp(secs).replace(".000", ""));
    log += &start("a-one", ",\"aside\":\"degrade\"", 999999, 5000);
    log += &start("a-two", ",\"aside\":\"degrade\"", 999999, 4000);
    log += &start("a-stale", ",\"aside\":\"stale\"", 999999, 4000);
    log += &start("i-one", "", 999999, 900);
    log += &start("i-two", "", 999999, 7200);
    log += &start("i-none", "", 999999, 90_000);
    log += &start("w-live", "", me, 600);
    // sessions whose last activity sits in each relative-time bucket
    for (id, age) in [("r-sec", 20u64), ("r-min", 600), ("r-hour", 18_000), ("r-edge", 86_200), ("r-day", 3 * 86_400), ("r-ten", 10 * 86_400 + 43_200), ("r-eleven", 11 * 86_400 + 43_200), ("r-old", 12 * 86_400)] {
        log += &start(id, "", 999999, age + 5);
        put(tpath(id), user(age, "relative"));
        let t = now - std::time::Duration::from_secs(age);
        fs::File::options().write(true).open(tpath(id)).unwrap().set_modified(t).unwrap();
    }
    log += &start("s-snz", "", me, 800);
    put(tpath("s-snz"), user(790, "snoozed turn") + &assistant(700, "claude-opus-5-5", 5, 5, 5));
    put(w.root.join("sessions.jsonl"), log);
    put(tpath("a-one"), user(4900, "aside question") + &assistant(4800, "claude-opus-5-5", 3, 4, 5));
    put(tpath("a-two"), user(3900, "second aside") + &assistant(3800, "claude-opus-5-5", 3, 4, 5));
    put(tpath("a-stale"), user(3900, "stale aside"));
    put(tpath("i-one"), user(800, "inbox prompt one\nwith a second line"));
    put(tpath("i-two"), user(7100, "x".repeat(300).as_str()));
    put(tpath("w-live"), user(500, "working prompt") + &assistant(450, "claude-opus-5-5", 12345, 6789, 4321));
    // events: a working chat and a blocked one
    let ev = |secs: u64, name: &str, id: &str, tool: &str| format!("{{\"v\":1,\"ts\":\"{}\",\"event\":\"{name}\",\"session_id\":\"{id}\",\"tool\":\"{tool}\"}}\n", stamp(secs).replace(".000", ""));
    let snoozed_at = stamp(100).replace(".000", "").replace('Z', "+00:00");
    put(w.root.join("work-items/snoozed-chat.md"), format!("---\nv: 1\ntitle: Snoozed chat\nstate: in_progress\nproject: demo\ncreated: '2026-02-03'\nsnooze_until: '2099-01-01'\nturn_snoozed_at: '{}'\nsessions:\n- id: s-snz\n---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n", snoozed_at));
    put(w.root.join("events.jsonl"), ev(600, "Stop", "s-snz", "") + &ev(400, "UserPromptSubmit", "w-live", "Edit") + &ev(120, "PermissionRequest", "d-live", "Bash") + &ev(60, "UserPromptSubmit", "d-fresh", "Read"));
    // an item with every note kind, rotations, snapshots and a session of each sort
    let item = format!(
        "---\nv: 1\ntitle: Rich item\nstate: in_progress\nproject: demo\ncreated: '2026-02-03'\nrotations:\n- at: '2026-01-01T00:00:00+00:00'\n  from: s0\n  carried: [1, 2]\n- at: '2026-01-05T00:00:00+00:00'\n  from: s1\nsessions:\n- id: w-live\n- id: i-none\n- id: r-sec\n- id: r-min\n- id: r-hour\n- id: r-edge\n- id: r-day\n- id: r-ten\n- id: r-eleven\n- id: r-old\n---\n\n## Description\n\nrich\n\n## Notes\n\n- 2026-01-02 [history]: opened\n- 2026-01-03: a decision\n- 2026-01-04 [open]: what about this?\n- 2026-01-05 [open]: and that?\n- 2026-01-06 [bogus]: odd marker\n\n## Snapshots\n\n### 2026-01-02 — session s0\n\ntext\n"
    );
    put(w.root.join("work-items/rich-item.md"), item);
    // a linked worktree with a checked-out branch
    git(&w.repo, &["branch", "wt-branch"]);
    let wt = base.join("wt-checkout");
    git(&w.repo, &["worktree", "add", wt.to_str().unwrap(), "wt-branch"]);
    let wt_item = "---\nv: 1\ntitle: In a worktree\nstate: in_progress\nproject: demo\ncreated: '2026-02-03'\nbranches:\n- wt-branch\n- main\n---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n";
    put(w.root.join("work-items/in-worktree.md"), wt_item.to_string());
    // drafts and issues
    let entries = serde_json::json!([
        {"id": "d1", "kind": "diff-comment", "comment": "first", "rev": 1, "path": "a.txt", "side": "new", "line": 3},
        {"id": "d2", "kind": "page-annotation", "comment": "second", "rev": 2}
    ]);
    put(w.root.join("drafts/rich-item.json"), serde_json::to_string_pretty(&serde_json::json!({"v": 1, "entries": entries})).unwrap());
    put(w.root.join("drafts/degrade.json"), "{\"v\":1,\"entries\":[]}".to_string());
    put(w.root.join("issues/rich-item/01-bug.md"), "---\nv: 1\ntitle: A bug\nkind: bug\n---\n\nbody\n".to_string());
    put(w.root.join("issues/rich-item/02-feature.md"), "---\nv: 1\ntitle: A feature\nkind: feature\n---\n\nbody two\n".to_string());
    put(w.root.join("issues/stale/.hidden.md"), "---\nv: 1\ntitle: Hidden\nkind: bug\n---\n".to_string());
    // the status line's snapshot
    put(w.root.join("rate-limits.json"), format!("{{\"captured_at\":{},\"five_hour\":{{\"used_percentage\":41.5,\"resets_at\":{}}},\"seven_day\":{{\"used_percentage\":12,\"resets_at\":\"2026-12-01T00:00:00Z\"}}}}", now.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() - 30, now.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 3600));
}

#[test]
fn the_state_matches_ruby_over_a_richer_world() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    richer_state_world(&w, base.path());
    let config = fs::read_to_string(w.root.join("config.yml")).unwrap();
    fs::write(w.root.join("config.yml"), config + "loose_starters:\n  dev:\n    label: Dev docs\n    prompt: Load the dev docs.\n  plain:\n    prompt: Read it.\n  emptylabel:\n    label: \"\"\n    prompt: P\n  skipped:\n    prompt: \"\"\n").unwrap();
    // loose chats: two open in tmux (one with a session, one without), one closed and left in the inbox
    let socket = "qm-test-state-loose";
    tmux_on(socket, &["kill-server"]);
    let (stamp, me) = ((chrono::Utc::now() - chrono::Duration::seconds(90)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true), std::process::id());
    let long_prompt = "a long first prompt that goes past sixty characters so the title is cut short with an ellipsis";
    let mut log = fs::read_to_string(w.root.join("sessions.jsonl")).unwrap();
    let later = (chrono::Utc::now() - chrono::Duration::seconds(30)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let (p61, p60) = ("x".repeat(61), "y".repeat(60));
    for (id, loose, pid, prompt, cwd, ts) in [
        ("s-loose-a", "ab12", me, long_prompt, w.home.join("proj").display().to_string(), stamp.clone()),
        ("s-loose-b", "ab12", 999999, "a newer session of the same chat, no longer running", "/srv/app".to_string(), later.clone()),
        ("s-loose-closed", "zz99", 999999, "a chat that was closed", "/srv/app".to_string(), stamp.clone()),
        ("s-loose-61", "ef56", 999999, p61.as_str(), "/srv/app".to_string(), stamp.clone()),
        ("s-loose-60", "gh78", 999999, p60.as_str(), "/srv/app".to_string(), stamp.clone()),
    ] {
        let path = w.transcripts.join(format!("p/{id}.jsonl"));
        fs::write(&path, format!("{}\n", serde_json::json!({"type": "user", "message": {"role": "user", "content": prompt}}))).unwrap();
        log += &format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"{cwd}\",\"pid\":{pid},\"project\":\"demo\",\"loose\":\"{loose}\",\"transcript_path\":\"{}\",\"ts\":\"{ts}\"}}\n", path.display());
    }
    fs::write(w.root.join("sessions.jsonl"), log).unwrap();
    let events = fs::read_to_string(w.root.join("events.jsonl")).unwrap_or_default();
    fs::write(w.root.join("events.jsonl"), events + &format!("{{\"v\":1,\"ts\":\"{stamp}\",\"event\":\"UserPromptSubmit\",\"session_id\":\"s-loose-a\",\"tool\":\"Edit\"}}\n")).unwrap();
    for name in ["loose--ab12", "loose--cd34", "loose--ef56", "loose--gh78"] {
        tmux_on(socket, &["new-session", "-d", "-s", name, "sleep 300"]);
    }
    let env: Vec<(&str, String)> = vec![("QM_TMUX_SOCKET", socket.to_string()),
        ("HOME", w.home.display().to_string()),
        ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()),
        ("PATH", format!("{}:{}", w.bin.display(), std::env::var("PATH").unwrap())),
        ("FAKE_GH_DATA", w.gh_data.display().to_string()),
        ("FAKE_GH_LOG", base.path().join("gh.log").display().to_string()),
    ];
    let root2 = base.path().join("root2");
    copy_dir(&w.root, &root2);
    let mut env_rust = env.clone();
    env_rust[5].1 = base.path().join("gh-rust.log").display().to_string();
    let ruby = start_env(false, &w.root, &w.home, &env);
    let rust = start_env(true, &root2, &w.home, &env_rust);
    for s in [&ruby, &rust] {
        request(s.port, "GET", "/api/state", &[], "", true);
    }
    std::thread::sleep(std::time::Duration::from_millis(800));
    let first = compare_state(&ruby, &rust, "first build");
    for needle in ["\"rich-item\"", "\"asideCount\":2", "\"rateLimits\":{", "\"wt-branch\"", "\"claude-haiku-5-5\":68", "inbox prompt one", "\"state\":\"working\"", "\"lowerBound\":true", "\"snoozed\":true", "\"looseStarters\":[{\"key\":\"dev\",\"label\":\"Dev docs\"},{\"key\":\"plain\",\"label\":\"plain\"},{\"key\":\"emptylabel\",\"label\":\"emptylabel\"}]", "\"name\":\"loose--ab12\"", "a long first prompt that goes past sixty characters so the …", "\"title\":\"New loose chat\"", "a chat that was closed", "\"cwd\":\"~/proj\"", "23h ago", "10d ago", "3d ago", "just now", "10m ago", "5h ago"] {
        assert!(first.contains(needle), "the world should reach {needle}");
    }
    compare_state(&ruby, &rust, "second build");
    tmux_on(socket, &["kill-server"]);
}

/// What the data actions leave behind: the item files and the error log, without its stamps.
fn action_traces(root: &Path, other: &Path) -> String {
    let mut out = String::new();
    let mut names: Vec<_> = fs::read_dir(root.join("work-items")).unwrap().map(|e| e.unwrap().path()).collect();
    names.sort();
    for p in names {
        out += &format!("== {}\n{}\n", p.file_name().unwrap().to_string_lossy(), fs::read_to_string(&p).unwrap());
    }
    let log = fs::read_to_string(root.join("qm.log")).unwrap_or_default();
    let log = regex::Regex::new(r"(?m)^\S+ \[").unwrap().replace_all(&log, "[").into_owned();
    out += &format!("== qm.log\n{}", log.replace(&other.display().to_string(), &root.display().to_string()));
    out
}

#[test]
fn the_data_actions_match_ruby() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let env: Vec<(&str, String)> = vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string())];
    let root2 = base.path().join("root2");
    copy_dir(&w.root, &root2);
    let ruby = start_env(false, &w.root, &w.home, &env);
    let rust = start_env(true, &root2, &w.home, &env);
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let mut step = |label: &str, body: &str| {
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, body, true), request(rust.port, "POST", "/api/action", &origin, body, true));
        assert_eq!(shown(&a, ruby.port), shown(&b, rust.port), "{label}: reply to {body}");
        assert_eq!(action_traces(&w.root, &root2), action_traces(&root2, &w.root), "{label}: files after {body}");
        shown(&a, ruby.port)
    };
    let act = |cmd: &str, slug: &str, args: &str| format!("{{\"cmd\":\"{cmd}\",\"slug\":\"{slug}\",\"args\":{args}}}");
    let cases: Vec<(&str, String)> = vec![
        ("note", act("note", "stale", r#"{"text":"first note"}"#)),
        ("note kind", act("note", "stale", r#"{"text":"an open question","kind":"open"}"#)),
        ("note empty kind", act("note", "stale", r#"{"text":"  padded  ","kind":""}"#)),
        ("note history", act("note", "stale", r#"{"text":"it happened","kind":"history"}"#)),
        ("note stamped", act("note", "stale", r#"{"text":"2026-01-01 stamped text"}"#)),
        ("note numeric", act("note", "stale", r#"{"text":5}"#)),
        ("note multi-line", act("note", "stale", r#"{"text":"line one\nline two: colon"}"#)),
        ("note bad kind", act("note", "stale", r#"{"text":"x","kind":"bogus"}"#)),
        ("note blank", act("note", "stale", r#"{"text":"   "}"#)),
        ("note blank missing item", act("note", "nowhere", r#"{"text":" "}"#)),
        ("retitle blank missing item", act("retitle", "nowhere", r#"{"title":" "}"#)),
        ("note no args", r#"{"cmd":"note","slug":"stale"}"#.into()),
        ("note missing item", act("note", "nowhere", r#"{"text":"x"}"#)),
        ("note null slug", r#"{"cmd":"note","slug":null,"args":{"text":"x"}}"#.into()),
        ("note traversal", act("note", "../x", r#"{"text":"x"}"#)),
        ("retag", act("retag", "stale", r#"{"position":2,"kind":"decision"}"#)),
        ("retag string", act("retag", "stale", r#"{"position":"3","kind":"history"}"#)),
        ("retag past the end", act("retag", "stale", r#"{"position":99,"kind":"open"}"#)),
        ("retag null position", act("retag", "stale", r#"{"position":null,"kind":"open"}"#)),
        ("retag zero", act("retag", "stale", r#"{"position":0,"kind":"open"}"#)),
        ("retag bad kind", act("retag", "stale", r#"{"position":1,"kind":"nope"}"#)),
        ("retag missing item", act("retag", "nowhere", r#"{"position":1,"kind":"open"}"#)),
        ("describe", act("describe", "stale", r#"{"text":"  A new description.\n\nTwo paragraphs.  "}"#)),
        ("describe empty", act("describe", "stale", r#"{"text":""}"#)),
        ("describe numeric", act("describe", "stale", r#"{"text":42}"#)),
        ("describe missing", act("describe", "nowhere", r#"{"text":"x"}"#)),
        ("retitle", act("retitle", "stale", r#"{"title":"  A better title  "}"#)),
        ("retitle colon", act("retitle", "stale", r#"{"title":"Fix: the thing # not a comment"}"#)),
        ("retitle empty", act("retitle", "stale", r#"{"title":"  "}"#)),
        ("retitle missing", act("retitle", "nowhere", r#"{"title":"x"}"#)),
        ("move", act("move", "stale", r#"{"project":"elsewhere"}"#)),
        ("move quoted", act("move", "stale", r#"{"project":"a: b"}"#)),
        ("move back", act("move", "stale", r#"{"project":"demo"}"#)),
        ("move missing", act("move", "nowhere", r#"{"project":"demo"}"#)),
        ("state fridge", act("state", "stale", r#"{"state":"fridge"}"#)),
        ("state back", act("state", "stale", r#"{"state":"in_progress"}"#)),
        ("state bogus", act("state", "stale", r#"{"state":"bogus"}"#)),
        ("state number", act("state", "stale", r#"{"state":5}"#)),
        ("state missing", act("state", "nowhere", r#"{"state":"fridge"}"#)),
        ("snooze days", act("snooze", "stale", r#"{"duration":"3d"}"#)),
        ("snooze zero", act("snooze", "stale", r#"{"duration":"0d"}"#)),
        ("snooze turn", act("snooze", "stale", r#"{"duration":"turn"}"#)),
        ("snooze clear", act("snooze", "stale", r#"{"duration":"clear"}"#)),
        ("snooze bogus", act("snooze", "stale", r#"{"duration":"soon"}"#)),
        ("snooze number", act("snooze", "stale", r#"{"duration":5}"#)),
        ("snooze null", act("snooze", "stale", r#"{"duration":null}"#)),
        ("snooze missing", act("snooze", "nowhere", r#"{"duration":"1d"}"#)),
        ("ack", act("ack", "degrade", "{}")),
        ("ack again", act("ack", "degrade", "{}")),
        ("ack none", act("ack", "stale", "{}")),
        ("ack missing", act("ack", "nowhere", "{}")),
        ("detach", act("detach", "degrade", r#"{"session":"d-soon"}"#)),
        ("detach again", act("detach", "degrade", r#"{"session":"d-soon"}"#)),
        ("detach null", act("detach", "degrade", r#"{"session":null}"#)),
        ("detach unattached", act("detach", "degrade", r#"{"session":"zzz"}"#)),
        ("detach missing", act("detach", "nowhere", r#"{"session":"d-soon"}"#)),
        ("unknown", act("frobnicate", "stale", "{}")),
        ("unknown empty", act("", "stale", "{}")),
        ("no cmd", r#"{"slug":"stale"}"#.into()),
        ("number cmd", r#"{"cmd":5}"#.into()),
        ("null cmd", r#"{"cmd":null}"#.into()),
        ("string payload", r#""note""#.into()),
        ("array payload", "[1]".into()),
        ("empty array payload", "[]".into()),
        ("number payload", "7".into()),
        ("junk payload", "not json".into()),
        ("empty payload", "".into()),
    ];
    for (label, body) in &cases {
        step(label, body);
    }
}

/// Drafts and item files as the draft actions leave them, with the random parts named: each
/// server's own new ids become `NEWID`, stamps become `STAMP`.
fn draft_traces(root: &Path, other: &Path, ids: &[String]) -> String {
    let mut out = String::new();
    let mut names: Vec<_> = fs::read_dir(root.join("drafts")).map(|r| r.map(|e| e.unwrap().path()).collect()).unwrap_or_default();
    names.sort();
    for p in names {
        out += &format!("== {}\n{}\n", p.file_name().unwrap().to_string_lossy(), fs::read_to_string(&p).unwrap_or_default());
    }
    let log = fs::read_to_string(root.join("qm.log")).unwrap_or_default();
    out += &format!("== qm.log\n{}", regex::Regex::new(r"(?m)^\S+ \[").unwrap().replace_all(&log, "[").replace(&other.display().to_string(), &root.display().to_string()));
    normalize_drafts(&out, ids)
}

fn normalize_drafts(text: &str, ids: &[String]) -> String {
    let mut text = regex::Regex::new(r#""(createdAt|updatedAt)":\s*"[^"]*""#).unwrap().replace_all(text, "\"$1\":\"STAMP\"").into_owned();
    for id in ids {
        text = text.replace(id.as_str(), "NEWID");
    }
    text
}

#[test]
fn the_draft_actions_match_ruby() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let entries = serde_json::json!([
        {"id": "d1", "kind": "diff-comment", "comment": "first", "rev": 2, "createdAt": "2026-04-01T00:00:00Z", "updatedAt": "2026-04-01T00:00:00Z", "path": "a.txt", "side": "new", "line": 3},
        {"id": "g1", "kind": "github-comment", "comment": "review one", "rev": 1, "author": "claude", "failed": "nope", "path": "a.txt", "side": "new", "line": 9, "repo": "acme/widgets", "number": 5, "commit": "abcdef0", "subject": "line"},
        {"id": "g2", "kind": "github-comment", "comment": "no rev", "path": "b.txt", "side": "old", "line": 1, "repo": "acme/widgets", "number": 5, "commit": "abcdef0", "author": "user"},
        {"id": "g3", "kind": "github-comment", "comment": "never edited, no rev", "path": "b.txt", "side": "new", "line": 2, "repo": "acme/widgets", "number": 5, "commit": "abcdef0", "author": "user"},
        {"id": "g4", "kind": "github-comment", "comment": "failed to post", "rev": 4, "failed": "422", "path": "b.txt", "side": "new", "line": 3, "repo": "acme/widgets", "number": 5, "commit": "abcdef0", "author": "user", "subject": "file"},
        {"id": "d2", "kind": "diff-comment", "comment": "pinned", "rev": 1, "path": "e.txt", "side": "new", "line": 1, "pin": "abcdef0", "failed": "old failure"},
        {"id": "a1", "kind": "annotation", "comment": "annotated", "rev": 1, "url": "https://example.test/"},
        {"id": "i1", "kind": "issue-comment", "comment": "about an issue", "rev": 3, "file": "01.md", "block": "x"}
    ]);
    fs::create_dir_all(w.root.join("drafts")).unwrap();
    fs::write(w.root.join("drafts/stale.json"), serde_json::to_string_pretty(&serde_json::json!({"v": 1, "entries": entries, "loaded": ["page-1"]})).unwrap()).unwrap();
    fs::write(w.root.join("drafts/degrade.json"), serde_json::to_string_pretty(&serde_json::json!({"v": 1, "entries": [{"id": "d9", "kind": "diff-comment", "comment": "only one", "rev": 1, "path": "z.txt", "side": "new", "line": 1}]})).unwrap()).unwrap();
    let env: Vec<(&str, String)> = vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string())];
    let root2 = base.path().join("root2");
    copy_dir(&w.root, &root2);
    let ruby = start_env(false, &w.root, &w.home, &env);
    let rust = start_env(true, &root2, &w.home, &env);
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let (mut ruby_ids, mut rust_ids): (Vec<String>, Vec<String>) = (vec![], vec![]);
    let id_pattern = regex::Regex::new(r#""id":"([0-9a-f]{12})""#).unwrap();
    let mut step = |label: &str, body: &str| {
        let last = |ids: &Vec<String>| ids.last().cloned().unwrap_or_default();
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, &body.replace("{ID}", &last(&ruby_ids)), true), request(rust.port, "POST", "/api/action", &origin, &body.replace("{ID}", &last(&rust_ids)), true));
        for (ids, answer) in [(&mut ruby_ids, &a), (&mut rust_ids, &b)] {
            if let Some(c) = id_pattern.captures(&String::from_utf8_lossy(&answer.body)) {
                ids.push(c[1].to_string());
            }
        }
        assert_eq!(normalize_drafts(&shown(&a, ruby.port), &ruby_ids), normalize_drafts(&shown(&b, rust.port), &rust_ids), "{label}: reply to {body}");
        assert_eq!(draft_traces(&w.root, &root2, &ruby_ids), draft_traces(&root2, &w.root, &rust_ids), "{label}: files after {body}");
    };
    let act = |cmd: &str, slug: &str, args: &str| format!("{{\"cmd\":\"{cmd}\",\"slug\":\"{slug}\",\"args\":{args}}}");
    let cases: Vec<(&str, String)> = vec![
        ("edit", act("draft-edit", "stale", r#"{"id":"d1","comment":"changed"}"#)),
        ("edit same", act("draft-edit", "stale", r#"{"id":"d1","comment":"changed"}"#)),
        ("edit blank", act("draft-edit", "stale", r#"{"id":"d1","comment":"  "}"#)),
        ("edit null", act("draft-edit", "stale", r#"{"id":"d1","comment":null}"#)),
        ("edit missing id", act("draft-edit", "stale", r#"{"id":"zz","comment":"x"}"#)),
        ("edit null id", act("draft-edit", "stale", r#"{"id":null,"comment":"x"}"#)),
        ("edit claude's draft", act("draft-edit", "stale", r#"{"id":"g1","comment":"mine now"}"#)),
        ("edit without rev", act("draft-edit", "stale", r#"{"id":"g2","comment":"has a rev"}"#)),
        ("edit number", act("draft-edit", "stale", r#"{"id":"i1","comment":5}"#)),
        ("edit missing item", act("draft-edit", "nowhere", r#"{"id":"d1","comment":"x"}"#)),
        ("put", act("draft-put", "stale", r#"{"path":"a.txt","side":"new","line":10,"code":"ten","comment":"hello"}"#)),
        ("put replaces", act("draft-put", "stale", r#"{"path":"a.txt","side":"new","line":10,"code":"ten","comment":"hello again"}"#)),
        ("put identical", act("draft-put", "stale", r#"{"path":"a.txt","side":"new","line":10,"code":"ten","comment":"hello again"}"#)),
        ("put new code", act("draft-put", "stale", r#"{"path":"a.txt","side":"new","line":10,"code":"changed","comment":"hello again"}"#)),
        ("put a range", act("draft-put", "stale", r#"{"path":"a.txt","side":"new","line":12,"range":"10-12","start_line":10,"start_side":"new","comment":"a range"}"#)),
        ("put on d1's line", act("draft-put", "stale", r#"{"path":"a.txt","side":"new","line":"3","comment":"over d1"}"#)),
        ("put branch", act("draft-put", "stale", r#"{"path":"c.txt","side":"new","line":1,"branch":"feature/x","committed":true,"comment":"on a branch"}"#)),
        ("put committed false", act("draft-put", "stale", r#"{"path":"c.txt","side":"new","line":2,"committed":false,"comment":"not committed"}"#)),
        ("put blanks dropped", act("draft-put", "stale", r#"{"path":"c.txt","side":"","line":null,"code":"","comment":"blank fields"}"#)),
        ("put no path", act("draft-put", "stale", r#"{"side":"new","line":1,"comment":"x"}"#)),
        ("put blank path", act("draft-put", "stale", r#"{"path":"  ","comment":"x"}"#)),
        ("put drops pin and failed", act("draft-put", "stale", r#"{"path":"e.txt","side":"new","line":1,"comment":"unpinned"}"#)),
        ("put backslash branch", act("draft-put", "stale", r#"{"path":"a.txt","branch":"a\\b","comment":"x"}"#)),
        ("put bracket branch", act("draft-put", "stale", r#"{"path":"a.txt","branch":"a[b","comment":"x"}"#)),
        ("put control branch", act("draft-put", "stale", r#"{"path":"a.txt","branch":"a\u0001b","comment":"x"}"#)),
        ("put long branch", act("draft-put", "stale", &format!(r#"{{"path":"a.txt","branch":"{}","comment":"x"}}"#, "b".repeat(256)))),
        ("put longest branch", act("draft-put", "stale", &format!(r#"{{"path":"l.txt","line":1,"branch":"{}","comment":"x"}}"#, "b".repeat(255)))),
        ("put bad branch", act("draft-put", "stale", r#"{"path":"a.txt","branch":"a b","comment":"x"}"#)),
        ("put number branch", act("draft-put", "stale", r#"{"path":"a.txt","branch":5,"comment":"x"}"#)),
        ("put kind annotation", act("draft-put", "stale", r#"{"path":"a.txt","kind":"annotation","comment":"x"}"#)),
        ("put kind null", act("draft-put", "stale", r#"{"path":"d.txt","kind":null,"line":1,"comment":"null kind"}"#)),
        ("put comment null", act("draft-put", "stale", r#"{"path":"never.txt","comment":null}"#)),
        ("put removes", act("draft-put", "stale", r#"{"path":"a.txt","side":"new","line":10,"code":"changed","comment":" "}"#)),
        ("put removes nothing", act("draft-put", "stale", r#"{"path":"a.txt","side":"new","line":99,"comment":""}"#)),
        ("put missing item", act("draft-put", "nowhere", r#"{"path":"a.txt","comment":"x"}"#)),
        ("move to chat", act("draft-move", "stale", r#"{"id":"g1","to":"chat"}"#)),
        ("move a failed draft", act("draft-move", "stale", r#"{"id":"g4","to":"chat"}"#)),
        ("move again", act("draft-move", "stale", r#"{"id":"g1","to":"chat"}"#)),
        ("move a chat draft", act("draft-move", "stale", r#"{"id":"d1","to":"chat"}"#)),
        ("move an annotation", act("draft-move", "stale", r#"{"id":"a1","to":"chat"}"#)),
        ("move an issue comment", act("draft-move", "stale", r#"{"id":"i1","to":"chat"}"#)),
        ("move missing id", act("draft-move", "stale", r#"{"id":"zz","to":"chat"}"#)),
        ("move bogus", act("draft-move", "stale", r#"{"id":"d1","to":"bogus"}"#)),
        ("move null", act("draft-move", "stale", r#"{"id":"d1","to":null}"#)),
        ("move number", act("draft-move", "stale", r#"{"id":"d1","to":5}"#)),
        ("move quoted", act("draft-move", "stale", r#"{"id":"d1","to":"a\"b\\c"}"#)),
        ("move missing item", act("draft-move", "nowhere", r#"{"id":"d1","to":"chat"}"#)),
        ("delete", act("draft-delete", "stale", r#"{"id":"d1"}"#)),
        ("delete again", act("draft-delete", "stale", r#"{"id":"d1"}"#)),
        ("delete null", act("draft-delete", "stale", r#"{"id":null}"#)),
        ("delete missing item", act("draft-delete", "nowhere", r#"{"id":"d1"}"#)),
        ("issue comment new", act("issue-comment-put", "stale", r#"{"file":"02.md","block":"  a block ","title":" T ","comment":"new one"}"#)),
        ("issue comment replace", act("issue-comment-put", "stale", r#"{"file":"02.md","block":"a block","title":"T2","comment":"changed"}"#)),
        ("issue comment same", act("issue-comment-put", "stale", r#"{"file":"02.md","block":"a block","comment":"changed"}"#)),
        ("issue comment keeps title", act("issue-comment-put", "stale", r#"{"file":"02.md","block":"a block","title":"","comment":"changed twice"}"#)),
        ("issue comment whole issue", act("issue-comment-put", "stale", r#"{"file":"02.md","comment":"whole"}"#)),
        ("issue comment on i1", act("issue-comment-put", "stale", r#"{"file":"01.md","block":"x","comment":"i1 again","title":"new title"}"#)),
        ("issue comment blank removes", act("issue-comment-put", "stale", r#"{"file":"02.md","block":"a block","comment":" "}"#)),
        ("issue comment blank nothing", act("issue-comment-put", "stale", r#"{"file":"03.md","comment":""}"#)),
        ("issue comment bad file", act("issue-comment-put", "stale", r#"{"file":"../x.md","comment":"x"}"#)),
        ("issue comment dot file", act("issue-comment-put", "stale", r#"{"file":".md","comment":"x"}"#)),
        ("issue comment nil file", act("issue-comment-put", "stale", r#"{"comment":"x"}"#)),
        ("issue comment number file", act("issue-comment-put", "stale", r#"{"file":5,"comment":"x"}"#)),
        ("issue comment number comment", act("issue-comment-put", "stale", r#"{"file":"04.md","comment":7}"#)),
        ("issue comment missing item", act("issue-comment-put", "nowhere", r#"{"file":"01.md","comment":"x"}"#)),
        ("reply new", act("thread-reply-put", "stale", r#"{"thread":"PRRT_abc-1","repo":"acme/widgets","number":5,"url":"https://github.com/acme/widgets/pull/5","path":"a.rs","line":4,"comment":"a reply"}"#)),
        ("reply replace", act("thread-reply-put", "stale", r#"{"thread":"PRRT_abc-1","repo":"acme/widgets","number":"5x","comment":"edited","path":"","line":null}"#)),
        ("reply same", act("thread-reply-put", "stale", r#"{"thread":"PRRT_abc-1","repo":"acme/widgets","number":5,"comment":"edited","author":"claude"}"#)),
        ("reply other thread", act("thread-reply-put", "stale", r#"{"thread":"T2","repo":"acme/widgets","number":6.9,"comment":"two","extra":1}"#)),
        ("reply blank removes", act("thread-reply-put", "stale", r#"{"thread":"T2","repo":"acme/widgets","number":5,"comment":""}"#)),
        ("reply blank nothing", act("thread-reply-put", "stale", r#"{"thread":"T9","repo":"acme/widgets","number":5,"comment":""}"#)),
        ("reply bad thread", act("thread-reply-put", "stale", r#"{"thread":"a b","repo":"acme/widgets","number":5,"comment":"x"}"#)),
        ("reply nil thread", act("thread-reply-put", "stale", r#"{"repo":"acme/widgets","number":5,"comment":"x"}"#)),
        ("reply bad repo", act("thread-reply-put", "stale", r#"{"thread":"T3","repo":"nope","number":5,"comment":"x"}"#)),
        ("reply zero number", act("thread-reply-put", "stale", r#"{"thread":"T3","repo":"acme/widgets","number":0,"comment":"x"}"#)),
        ("reply no number", act("thread-reply-put", "stale", r#"{"thread":"T3","repo":"acme/widgets","comment":"x"}"#)),
        ("reply long thread", act("thread-reply-put", "stale", &format!("{{\"thread\":\"{}\",\"repo\":\"acme/widgets\",\"number\":5,\"comment\":\"x\"}}", "t".repeat(201)))),
        ("reply missing item", act("thread-reply-put", "nowhere", r#"{"thread":"T3","repo":"acme/widgets","number":5,"comment":"x"}"#)),
        ("discard some", act("drafts-discard", "stale", r#"{"ids":["a1","nonexistent"]}"#)),
        ("discard none", act("drafts-discard", "stale", r#"{"ids":[]}"#)),
        ("discard without rev", act("drafts-discard", "stale", r#"{"ids":["g2"]}"#)),
        ("discard missing item", act("drafts-discard", "nowhere", r#"{"ids":["d1"]}"#)),
        ("discard last of an item", act("drafts-discard", "degrade", r#"{"ids":["d9"]}"#)),
        ("discard on an item without drafts", act("drafts-discard", "fridge-old", r#"{"ids":["d9"]}"#)),
        ("discard everything", act("drafts-discard", "stale", r#"{"ids":null}"#)),
    ];
    for (label, body) in &cases {
        step(label, body);
    }
    // the same, once more with a file Ruby reads as empty
    fs::write(w.root.join("drafts/stale.json"), "not json").unwrap();
    fs::write(root2.join("drafts/stale.json"), "not json").unwrap();
    step("put over junk", &act("draft-put", "stale", r#"{"path":"a.txt","line":1,"comment":"fresh"}"#));
    step("delete it", &act("draft-delete", "stale", r#"{"id":"{ID}"}"#));
    // GitHub drafts, PR addresses and moves to the review are checked with git (the review test covers a real PR)
    step("github put without a repo", &act("draft-put", "stale", r#"{"path":"a.txt","kind":"github-comment","comment":"x"}"#));
    step("PR address put", &act("draft-put", "stale", r#"{"path":"a.txt","number":5,"comment":"x"}"#));
    step("PR address put, complete", &act("draft-put", "stale", r#"{"path":"a.txt","number":5,"repo":"acme/widgets","commit":"abcdef0","comment":"x"}"#));
    step("move to the review", &act("draft-move", "stale", r#"{"id":"g1","to":"github"}"#));
}

fn viewed_traces(root: &Path, other: &Path) -> String {
    let mut out = String::new();
    let mut names: Vec<_> = fs::read_dir(root.join("viewed")).map(|r| r.map(|e| e.unwrap().path()).collect()).unwrap_or_default();
    names.sort();
    for p in names {
        out += &format!("== {}\n{}\n", p.file_name().unwrap().to_string_lossy(), fs::read_to_string(&p).unwrap_or_default());
    }
    out + &action_traces(root, other)
}

#[test]
fn diff_viewed_and_pf_mode_match_ruby() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    fs::write(w.root.join("config.yml"), format!("projects:\n  demo:\n    root: {}\n    dev_server:\n      command: devsrv\n      modes: [dev, tunnel]\n      default_mode: dev\n", w.repo.display())).unwrap();
    fs::write(w.root.join("work-items/no-project.md"), "---\nv: 1\ntitle: no project\nstate: in_progress\ncreated: '2026-01-01'\n---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n").unwrap();
    // a viewed file already at the 2000-path limit, with odd values among the marks
    let mut marks = serde_json::Map::new();
    marks.insert("odd-string".into(), serde_json::json!("0123456789abcdef0123"));
    marks.insert("odd-null".into(), serde_json::Value::Null);
    for i in 0..1998 {
        marks.insert(format!("old/{i:04}.txt"), serde_json::json!([format!("{i:020x}")]));
    }
    fs::create_dir_all(w.root.join("viewed")).unwrap();
    fs::write(w.root.join("viewed/degrade.json"), serde_json::json!({"v": 1, "marks": marks, "extra": [1]}).to_string()).unwrap();
    fs::write(w.root.join("viewed/pr-one-merged.json"), "not json").unwrap();
    let env: Vec<(&str, String)> = vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string())];
    let root2 = base.path().join("root2");
    copy_dir(&w.root, &root2);
    let ruby = start_env(false, &w.root, &w.home, &env);
    let rust = start_env(true, &root2, &w.home, &env);
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let step = |label: &str, body: &str| {
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, body, true), request(rust.port, "POST", "/api/action", &origin, body, true));
        assert_eq!(shown(&a, ruby.port), shown(&b, rust.port), "{label}: reply to {body}");
        assert_eq!(viewed_traces(&w.root, &root2), viewed_traces(&root2, &w.root), "{label}: files after {body}");
    };
    let act = |cmd: &str, slug: &str, args: &str| format!("{{\"cmd\":\"{cmd}\",\"slug\":\"{slug}\",\"args\":{args}}}");
    let d = |n: u32| format!("{n:020x}");
    let mark = |slug: &str, path: &str, digest: &str, viewed: &str| act("diff-viewed", slug, &format!("{{\"path\":\"{path}\",\"digest\":{digest},\"viewed\":{viewed}}}"));
    let mut cases: Vec<(String, String)> = vec![];
    let mut add = |label: &str, body: String| cases.push((label.to_string(), body));
    add("mark", mark("stale", "a.txt", &format!("\"{}\"", d(1)), "true"));
    add("mark again", mark("stale", "a.txt", &format!("\"{}\"", d(1)), "true"));
    for n in 2..=8 {
        add("mark more", mark("stale", "a.txt", &format!("\"{}\"", d(n)), "true"));
    }
    add("mark another path", mark("stale", "b.txt", &format!("\"{}\"", d(1)), "true"));
    add("mark the first path again", mark("stale", "a.txt", &format!("\"{}\"", d(8)), "true"));
    add("unmark", mark("stale", "a.txt", &format!("\"{}\"", d(8)), "false"));
    add("unmark absent", mark("stale", "a.txt", &format!("\"{}\"", d(99)), "false"));
    add("viewed not boolean", mark("stale", "a.txt", &format!("\"{}\"", d(5)), "\"true\""));
    add("viewed missing", act("diff-viewed", "stale", &format!("{{\"path\":\"a.txt\",\"digest\":\"{}\"}}", d(6))));
    add("uppercase hex", mark("stale", "c.txt", "\"ABCDEF0123456789\"", "true"));
    add("64 hex", mark("stale", "c.txt", &format!("\"{}\"", "a".repeat(64)), "true"));
    add("unicode path", mark("stale", "dir/ü and \\\\ \\\"q\\\".txt", &format!("\"{}\"", d(1)), "true"));
    for (label, digest) in [("short", "\"abc\""), ("15 hex", "\"0123456789abcde\""), ("65 hex", &format!("\"{}\"", "a".repeat(65))), ("not hex", "\"zzzzzzzzzzzzzzzzzzzz\""), ("null", "null"), ("number", "5"), ("true", "true"), ("quoted", "\"a\\\"b\\\\c\""), ("hash sign", "\"a#{x}\""), ("newline", "\"a\\nb\""), ("control", "\"a\\u0001\\u007f\\u001b\\t\""), ("dollar", "\"a#$b#@c#d\"")] {
        add(&format!("digest {label}"), mark("stale", "a.txt", digest, "true"));
    }
    add("path null", mark("stale", "x", "null", "true").replace("\"path\":\"x\"", "\"path\":null"));
    add("path blank", mark("stale", "  ", &format!("\"{}\"", d(1)), "true"));
    add("path empty", mark("stale", "", "null", "true"));
    add("path number", act("diff-viewed", "stale", &format!("{{\"path\":5,\"digest\":\"{}\",\"viewed\":true}}", d(1))));
    add("no args", act("diff-viewed", "stale", "{}"));
    add("missing item", mark("nowhere", "a.txt", &format!("\"{}\"", d(1)), "true"));
    add("unmark the last", mark("stale", "b.txt", &format!("\"{}\"", d(1)), "false"));
    add("odd string kept", mark("degrade", "odd-string", "\"fedcba9876543210ffff\"", "true"));
    add("odd string value", mark("degrade", "odd-string", "\"0123456789abcdef0123\"", "false"));
    add("odd null", mark("degrade", "odd-null", &format!("\"{}\"", d(3)), "true"));
    add("beyond the limit", mark("degrade", "new.txt", &format!("\"{}\"", d(7)), "true"));
    add("over junk", mark("pr-one-merged", "a.txt", &format!("\"{}\"", d(1)), "true"));
    add("unmark the only mark", mark("pr-one-merged", "a.txt", &format!("\"{}\"", d(1)), "false"));
    for (label, mode) in [("dev", "\"dev\""), ("tunnel", "\"tunnel\""), ("again", "\"tunnel\""), ("bogus", "\"bogus\""), ("empty", "\"\""), ("number", "5"), ("null", "null"), ("true", "true")] {
        add(&format!("pf-mode {label}"), act("pf-mode", "stale", &format!("{{\"mode\":{mode}}}")));
    }
    add("pf-mode no args", act("pf-mode", "stale", "{}"));
    add("pf-mode no dev server", act("pf-mode", "pr-no-project", r#"{"mode":"dev"}"#));
    add("pf-mode no project", act("pf-mode", "no-project", r#"{"mode":"dev"}"#));
    add("pf-mode missing item", act("pf-mode", "nowhere", r#"{"mode":"dev"}"#));
    add("pf-mode quoted", act("pf-mode", "stale", r#"{"mode":"a: b"}"#));
    for (label, body) in &cases {
        step(label, body);
    }
}

fn tree_text(dir: &Path, prefix: &str, out: &mut String) {
    let mut names: Vec<_> = fs::read_dir(dir).map(|r| r.map(|e| e.unwrap().path()).collect()).unwrap_or_default();
    names.sort();
    for p in names {
        let shown = format!("{prefix}/{}", p.file_name().unwrap().to_string_lossy());
        if p.is_dir() {
            *out += &format!("== {shown}/\n");
            tree_text(&p, &shown, out);
        } else {
            *out += &format!("== {shown}\n{}\n", fs::read_to_string(&p).unwrap_or_default());
        }
    }
}

fn issue_traces(root: &Path, other: &Path) -> String {
    let mut out = String::new();
    tree_text(&root.join("issues"), "issues", &mut out);
    tree_text(&root.join("drafts"), "drafts", &mut out);
    let log = fs::read_to_string(root.join("qm.log")).unwrap_or_default();
    out += &format!("== qm.log\n{}", regex::Regex::new(r"(?m)^\S+ \[").unwrap().replace_all(&log, "[").replace(&other.display().to_string(), &root.display().to_string()));
    regex::Regex::new(r#""seen_at":[0-9.e+-]+"#).unwrap().replace_all(&out, "\"seen_at\":T").into_owned()
}

#[test]
fn the_issue_draft_actions_match_ruby() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let issue = |slug: &str, name: &str, front: &str, body: &str| {
        let p = w.root.join(format!("issues/{slug}/{name}"));
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, format!("---\n{front}---\n\n{body}\n")).unwrap();
    };
    issue("stale", "01-bug.md", "title: A bug\nkind: bug\nnumber: 12\n", "created already");
    issue("stale", "02-feature.md", "title: A feature\nkind: feature\n", "still to create");
    issue("stale", "03-created.md", "title: Created too\nkind: feature\nnumber: '#7'\ndeps: [one, none]\n", "body three");
    issue("stale", "04-broken.md", "title: Broken\nkind: epic\n", "bad kind");
    issue("stale", "05-notitle.md", "kind: bug\n", "no title");
    fs::write(w.root.join("issues/stale/06-nofront.md"), "just text\n").unwrap();
    fs::write(w.root.join("issues/stale/.hidden.md"), "---\ntitle: h\nkind: bug\n---\n").unwrap();
    fs::write(w.root.join("issues/stale/notes.txt"), "not an issue\n").unwrap();
    issue("degrade", "01-done.md", "title: Done\nkind: bug\nnumber: 3\n", "only created");
    issue("pr-stack-merged", "01-done.md", "title: Done\nkind: bug\nnumber: 4\n", "drafts hold a float");
    fs::create_dir_all(w.root.join("drafts")).unwrap();
    fs::write(w.root.join("drafts/pr-stack-merged.json"), "{\"v\":1,\"entries\":[{\"id\":\"f\",\"kind\":\"issue-comment\",\"file\":\"01-done.md\",\"rev\":1.5}]}").unwrap();
    issue("fridge-old", "01-date.md", "title: Dated\nkind: bug\nwhen: 2026-01-01\n", "a date Psych reads as one");
    fs::create_dir_all(w.root.join("issues/stale/.prev")).unwrap();
    fs::write(w.root.join("issues/stale/.prev/02-feature.md.json"), serde_json::json!({"v": 1, "seen": "---\ntitle: Older\nkind: feature\n---\n\nolder body\n", "seen_at": 1.0, "previous": null}).to_string()).unwrap();
    fs::write(w.root.join("issues/stale/.prev/orphan.md.json"), "{\"v\":1,\"seen\":\"x\",\"seen_at\":1.0,\"previous\":null}").unwrap();
    fs::create_dir_all(w.root.join("drafts")).unwrap();
    fs::write(
        w.root.join("drafts/stale.json"),
        serde_json::to_string_pretty(&serde_json::json!({"v": 1, "entries": [
            {"id": "c1", "kind": "issue-comment", "comment": "on the bug", "rev": 1, "file": "01-bug.md", "block": "x"},
            {"id": "c2", "kind": "issue-comment", "comment": "on the feature", "rev": 1, "file": "02-feature.md", "block": "y"},
            {"id": "d1", "kind": "diff-comment", "comment": "unrelated", "rev": 1, "path": "a.txt", "side": "new", "line": 1}
        ]}))
        .unwrap(),
    )
    .unwrap();
    let env: Vec<(&str, String)> = vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string())];
    let root2 = base.path().join("root2");
    copy_dir(&w.root, &root2);
    let ruby = start_env(false, &w.root, &w.home, &env);
    let rust = start_env(true, &root2, &w.home, &env);
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let step = |label: &str, body: &str| {
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, body, true), request(rust.port, "POST", "/api/action", &origin, body, true));
        assert_eq!(shown(&a, ruby.port), shown(&b, rust.port), "{label}: reply to {body}");
        assert_eq!(issue_traces(&w.root, &root2), issue_traces(&root2, &w.root), "{label}: files after {body}");
    };
    let act = |cmd: &str, slug: &str, args: &str| format!("{{\"cmd\":\"{cmd}\",\"slug\":\"{slug}\",\"args\":{args}}}");
    let ack = |slug: &str, file: &str| act("issue-acknowledge", slug, &format!("{{\"file\":{file}}}"));
    step("ack", &ack("stale", "\"02-feature.md\""));
    step("ack again", &ack("stale", "\"02-feature.md\""));
    step("ack a created one", &ack("stale", "\"01-bug.md\""));
    step("ack broken", &ack("stale", "\"04-broken.md\""));
    step("ack no title", &ack("stale", "\"05-notitle.md\""));
    step("ack no front matter", &ack("stale", "\"06-nofront.md\""));
    step("ack missing file", &ack("stale", "\"nope.md\""));
    step("ack not a file name", &ack("stale", "\"notes.txt\""));
    step("ack hidden", &ack("stale", "\".hidden.md\""));
    step("ack with a slash", &ack("stale", "\"a/b.md\""));
    step("ack too short", &ack("stale", "\".md\""));
    step("ack null", &ack("stale", "null"));
    step("ack number", &ack("stale", "5"));
    step("ack bool", &ack("stale", "true"));
    step("ack no args", &act("issue-acknowledge", "stale", "{}"));
    step("ack no issues", &ack("fridge-quiet-none", "\"x.md\""));
    step("ack missing item", &ack("nowhere", "\"02-feature.md\""));
    // the file changes, is read again, and the new revision is acknowledged
    for root in [&w.root, &root2] {
        fs::write(root.join("issues/stale/02-feature.md"), "---\ntitle: A feature, edited\nkind: feature\n---\n\nnew body\n").unwrap();
    }
    step("ack a new revision", &ack("stale", "\"02-feature.md\""));
    for root in [&w.root, &root2] {
        fs::write(root.join("issues/stale/03-created.md"), "---\ntitle: Created, edited\nkind: feature\nnumber: 7\n---\n\nchanged\n").unwrap();
    }
    step("ack another", &ack("stale", "\"03-created.md\""));
    step("clear created", &act("issues-clear-created", "stale", "{}"));
    step("clear again", &act("issues-clear-created", "stale", "{}"));
    step("clear the last", &act("issues-clear-created", "degrade", "{}"));
    step("clear without issues", &act("issues-clear-created", "pr-one-merged", "{}"));
    step("clear missing item", &act("issues-clear-created", "nowhere", "{}"));
    // a file Psych would read as a date: Ruby's, nothing written
    let before = issue_traces(&root2, &w.root);
    for body in [act("issues-clear-created", "fridge-old", "{}"), act("issues-clear-created", "pr-stack-merged", "{}"), ack("fridge-old", "\"01-date.md\"")] {
        let a = request(rust.port, "POST", "/api/action", &origin, &body, true);
        assert!(a.status.starts_with("501"), "{body}: {}", a.status);
    }
    assert_eq!(before, issue_traces(&root2, &w.root));
}

#[test]
fn resolving_offers_the_cleanup_plan_as_ruby_does() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let repo = &w.repo;
    // worktrees in several states: merged and clean, dirty, with a commit found nowhere else
    git(repo, &["worktree", "add", base.path().join("wt-merged").to_str().unwrap(), "merged-b"]);
    let dirty = base.path().join("wt-dirty");
    git(repo, &["worktree", "add", "-b", "dirty-b", dirty.to_str().unwrap()]);
    fs::write(dirty.join("loose.txt"), "uncommitted\n").unwrap();
    let unpushed = base.path().join("wt-unpushed");
    git(repo, &["worktree", "add", "-b", "unpushed-b", unpushed.to_str().unwrap()]);
    fs::write(unpushed.join("work.txt"), "work\n").unwrap();
    git(&unpushed, &["add", "."]);
    git(&unpushed, &["commit", "-q", "-m", "work nobody has"]);
    let item = |slug: &str, extra: &str| {
        fs::write(w.root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\ntitle: {slug}\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\n{extra}---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n")).unwrap();
    };
    item("res-merged", "branches:\n- merged-b\n");
    item("res-dirty", "branches:\n- dirty-b\n");
    item("res-unpushed", "branches:\n- unpushed-b\n- gone-branch\n");
    item("res-issues", "");
    item("res-nothing", "");
    item("res-twice", "branches:\n- nopr-exists\n");
    item("res-unsure", "");
    fs::create_dir_all(w.root.join("issues/res-unsure")).unwrap();
    fs::write(w.root.join("issues/res-unsure/01-date.md"), "---\ntitle: Dated\nkind: bug\nwhen: 2026-01-01\n---\n\nbody\n").unwrap();
    let issue = |name: &str, front: &str| {
        let p = w.root.join(format!("issues/res-issues/{name}"));
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, format!("---\n{front}---\n\nbody\n")).unwrap();
    };
    issue("01-made.md", "title: Made\nkind: bug\nnumber: 9\n");
    issue("02-open.md", "title: Open\nkind: feature\n");
    // fresh PR answers, so no plan waits on a background lookup
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let entry = |state: &str, n: u32| format!("{{\"state\":{state},\"number\":{n},\"url\":\"https://example.test/pull/{n}\",\"ts\":{now}}}");
    fs::write(
        w.root.join("pr-cache.json"),
        format!("{{\"demo#merged-b\":{},\"demo#dirty-b\":{},\"demo#unpushed-b\":{{\"state\":null,\"ts\":{now}}},\"demo#nopr-exists\":{{\"state\":null,\"ts\":{now}}},\"demo#gone-branch\":{{\"state\":null,\"ts\":{now}}}}}", entry("\"MERGED\"", 6), entry("\"OPEN\"", 7)),
    )
    .unwrap();
    fs::create_dir_all(w.root.join("viewed")).unwrap();
    fs::write(w.root.join("viewed/res-merged.json"), "{\"v\":1,\"marks\":{\"a.txt\":[\"0123456789abcdef0123\"]}}").unwrap();
    let env: Vec<(&str, String)> = vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("PATH", format!("{}:{}", w.bin.display(), std::env::var("PATH").unwrap())), ("FAKE_GH_DATA", w.gh_data.display().to_string()), ("FAKE_GH_LOG", base.path().join("gh.log").display().to_string())];
    let root2 = base.path().join("root2");
    copy_dir(&w.root, &root2);
    let ruby = start_env(false, &w.root, &w.home, &env);
    let rust = start_env(true, &root2, &w.home, &env);
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let step = |label: &str, body: &str| {
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, body, true), request(rust.port, "POST", "/api/action", &origin, body, true));
        let (sa, sb) = (shown(&a, ruby.port), shown(&b, rust.port).replace("/root2/", "/root/"));
        assert_eq!(sa, sb, "{label}: reply to {body}");
        assert_eq!(viewed_traces(&w.root, &root2), viewed_traces(&root2, &w.root), "{label}: files after {body}");
        sa
    };
    let resolve = |slug: &str| format!("{{\"cmd\":\"state\",\"slug\":\"{slug}\",\"args\":{{\"state\":\"resolved\"}}}}");
    let mut plans = String::new();
    for slug in ["res-merged", "res-dirty", "res-unpushed", "res-issues", "res-nothing", "res-twice", "res-twice", "pr-no-project", "nowhere"] {
        plans += &step(slug, &resolve(slug));
    }
    for slug in ["res-merged", "res-dirty"] {
        step("reopened", &format!("{{\"cmd\":\"state\",\"slug\":\"{slug}\",\"args\":{{\"state\":\"fridge\"}}}}"));
    }
    // issue files Psych might read differently are Ruby's: nothing written, not even the state
    let before = viewed_traces(&root2, &w.root);
    let a = request(rust.port, "POST", "/api/action", &origin, &resolve("res-unsure"), true);
    assert!(a.status.starts_with("501"), "{}", a.status);
    assert_eq!(before, viewed_traces(&root2, &w.root));
    for needle in ["worktree:", "branch:", "issue:", "need", "untracked files", "found nowhere else", "PR merged", "never created on GitHub"] {
        assert!(plans.contains(needle), "the world should reach {needle}: {plans}");
    }
}

fn tmux_on(socket: &str, args: &[&str]) -> String {
    let out = Command::new("tmux").args(["-L", socket]).args(args).env_remove("TMUX").output().unwrap();
    if args == ["kill-server"] {
        // the server goes asynchronously: a session made right away could land on the dying one
        for _ in 0..60 {
            let alive = Command::new("tmux").args(["-L", socket, "list-sessions"]).env_remove("TMUX").output().map(|o| o.status.success()).unwrap_or(false);
            if !alive {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// What a pane opened by an action left in tmux: sessions, what each runs and how big, the server options.
fn tmux_picture(socket: &str) -> String {
    tmux_picture_with(socket, &|s| s)
}

/// The same, with names and commands passed through `norm` before sorting (random ids made stable).
fn tmux_picture_with(socket: &str, norm: &dyn Fn(String) -> String) -> String {
    let mut sessions: Vec<String> = tmux_on(socket, &["list-sessions", "-F", "#{session_name}"]).lines().map(|l| norm(l.to_string())).collect();
    sessions.sort();
    let mut panes: Vec<String> = tmux_on(socket, &["list-panes", "-a", "-F", "#{session_name}|#{window_name}|#{pane_start_command}|#{window_width}x#{window_height}"]).lines().map(|l| norm(l.to_string())).collect();
    panes.sort();
    let options: String = ["status", "mouse", "escape-time", "prefix", "history-limit", "window-size"].iter().map(|o| tmux_on(socket, &["show-options", "-g", o])).collect();
    format!("{sessions:?}\n{}\n{options}{}", panes.join("\n"), tmux_on(socket, &["show-options", "-g", "terminal-features"]))
}

fn records(dir: &Path) -> Vec<String> {
    let mut all: Vec<String> = fs::read_dir(dir).map(|r| r.map(|e| e.unwrap().path()).filter(|p| !p.file_name().unwrap().to_string_lossy().starts_with('.')).map(|p| fs::read_to_string(p).unwrap_or_default()).collect()).unwrap_or_default();
    all.sort();
    all
}

#[test]
fn the_pane_actions_open_the_same_chats_as_ruby() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let me = std::process::id();
    let stamp = |secs: i64| (chrono::Utc::now() - chrono::Duration::seconds(secs)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let work = base.path().join("work-a");
    fs::create_dir_all(&work).unwrap();
    git(&w.repo, &["worktree", "add", "-b", "wt-b", base.path().join("wt-b").to_str().unwrap()]);
    let dashed = |p: &Path| -> String { p.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect() };
    let mut log = String::new();
    let mut session = |id: &str, cwd: &Path, transcript: Option<PathBuf>, pid: u32, age: i64| {
        let path = transcript.unwrap_or_else(|| w.transcripts.join(format!("p/{id}.jsonl")));
        if !path.to_string_lossy().contains("MISSING") {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "{}\n").unwrap();
        }
        log += &format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"{}\",\"pid\":{pid},\"transcript_path\":\"{}\",\"ts\":\"{}\"}}\n", cwd.display(), path.display(), stamp(age));
    };
    session("r-one", &work, None, 999999, 500);
    let gone = w.repo.join(".claude/worktrees/gone");
    session("r-anc", &gone, Some(w.transcripts.join(dashed(&w.repo)).join("r-anc.jsonl")), 999999, 400);
    session("r-exp", &work, Some(w.transcripts.join("p/MISSING.jsonl")), 999999, 300);
    session("r-live", &work, None, me as u32, 200);
    session("r-old", &work, None, 999999, 900);
    session("r-new", &gone, Some(w.transcripts.join(dashed(&w.repo)).join("r-new.jsonl")), 999999, 100);
    session("r-np", &work, None, 999999, 150);
    fs::write(w.root.join("sessions.jsonl"), log).unwrap();
    let item = |slug: &str, project: &str, extra: &str| {
        fs::write(w.root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\ntitle: {slug}\nstate: in_progress\nproject: {project}\ncreated: '2026-01-01'\n{extra}---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n")).unwrap();
    };
    item("p-fresh", "demo", "");
    item("p-wt", "demo", "branches:\n- wt-b\n");
    item("p-resume", "demo", "sessions:\n- id: r-one\n");
    item("p-anc", "demo", "sessions:\n- id: r-anc\n");
    item("p-exp", "demo", "sessions:\n- id: r-exp\n");
    item("p-live", "demo", "sessions:\n- id: r-live\n- id: r-one\n");
    item("p-two-a", "demo", "sessions:\n- id: r-old\n- id: r-new\n");
    item("p-two-b", "demo", "sessions:\n- id: r-old\n- id: r-new\n");
    item("p-two-c", "demo", "sessions:\n- id: r-old\n- id: r-new\n");
    item("p-two-d", "demo", "sessions:\n- id: r-old\n- id: r-new\n");
    item("p-two-e", "demo", "sessions:\n- id: r-old\n- id: r-new\n");
    item("p-noproject", "No project", "sessions:\n- id: r-np\n");
    item("p-nothing", "No project", "");
    item("p-restart", "demo", "");
    let chat = base.path().join("fake-chat");
    fs::write(&chat, "#!/bin/sh\n{ echo \"cwd: $(pwd)\"; echo \"args: $*\"; echo \"QM_ITEM=${QM_ITEM-unset}\"; echo \"claude vars: $(env | grep -c '^CLAUDE')\"; } > \"$FAKE_REC/.tmp-$$\"\nmv \"$FAKE_REC/.tmp-$$\" \"$FAKE_REC/rec-$$\"\nexec sleep 300\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&chat, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let (rec_ruby, rec_rust) = (base.path().join("rec-ruby"), base.path().join("rec-rust"));
    for d in [&rec_ruby, &rec_rust] {
        fs::create_dir_all(d).unwrap();
    }
    let (sock_ruby, sock_rust) = ("qm-test-pane-ruby", "qm-test-pane-rust");
    for s in [sock_ruby, sock_rust] {
        tmux_on(s, &["kill-server"]);
    }
    let env_for = |sock: &str, rec: &Path| -> Vec<(&'static str, String)> {
        vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("QM_TMUX_SOCKET", sock.to_string()), ("QM_CHAT_COMMAND", format!("{} --flag value", chat.display())), ("FAKE_REC", rec.display().to_string()), ("CLAUDECODE", "1".to_string()), ("CLAUDE_CODE_ENTRYPOINT", "cli".to_string())]
    };
    let root2 = base.path().join("root2");
    copy_dir(&w.root, &root2);
    let ruby = start_env(false, &w.root, &w.home, &env_for(sock_ruby, &rec_ruby));
    let rust = start_env(true, &root2, &w.home, &env_for(sock_rust, &rec_rust));
    let origin = [("Content-Type", "application/json".to_string())];
    let mut opened = 0usize;
    let mut step = |label: &str, body: &str| {
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, body, true), request(rust.port, "POST", "/api/action", &origin, body, true));
        assert_eq!(shown(&a, ruby.port), shown(&b, rust.port), "{label}: reply to {body}");
        if String::from_utf8_lossy(&a.body).contains("\"mode\"") {
            opened += 1;
        }
        for _ in 0..100 {
            if records(&rec_ruby).len() >= opened && records(&rec_rust).len() >= opened {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(records(&rec_ruby), records(&rec_rust), "{label}: what the chats were started with, after {body}");
        assert_eq!(tmux_picture(sock_ruby), tmux_picture(sock_rust), "{label}: tmux after {body}");
    };
    let act = |cmd: &str, slug: &str, args: &str| format!("{{\"cmd\":\"{cmd}\",\"slug\":\"{slug}\",\"args\":{args}}}");
    let cases: Vec<(&str, String)> = vec![
        ("start", act("pane-start", "p-fresh", "{}")),
        ("start again", act("pane-start", "p-fresh", "{}")),
        ("start in the worktree", act("pane-start", "p-wt", "{}")),
        ("resume", act("pane-resume", "p-resume", "{}")),
        ("resume where the worktree is gone", act("pane-resume", "p-anc", "{}")),
        ("resume an expired transcript", act("pane-resume", "p-exp", "{}")),
        ("start refuses a live session", act("pane-start", "p-live", "{}")),
        ("resume refuses a live session", act("pane-resume", "p-live", r#"{"session":"r-one"}"#)),
        ("resume the newest", act("pane-resume", "p-two-a", "{}")),
        ("resume the chosen older one", act("pane-resume", "p-two-b", r#"{"session":"r-old"}"#)),
        ("resume an unknown session", act("pane-resume", "p-two-c", r#"{"session":"nope"}"#)),
        ("resume a blank session", act("pane-resume", "p-two-d", r#"{"session":"  "}"#)),
        ("resume a numeric session", act("pane-resume", "p-two-e", r#"{"session":5}"#)),
        ("start ignores the session", act("pane-start", "p-restart", r#"{"session":"r-old"}"#)),
        ("no project, a session's cwd", act("pane-start", "p-noproject", "{}")),
        ("no project, no session", act("pane-start", "p-nothing", "{}")),
        ("missing item", act("pane-start", "nowhere", "{}")),
        ("resume missing item", act("pane-resume", "nowhere", "{}")),
        ("start null slug", r#"{"cmd":"pane-start","slug":null}"#.to_string()),
    ];
    for (label, body) in &cases {
        step(label, body);
    }
    let seen = records(&rec_rust).join("\n");
    for needle in ["--flag value", "--resume r-one", "QM_ITEM=p-fresh", "claude vars: 0", "wt-b"] {
        assert!(seen.contains(needle), "the world should reach {needle}: {seen}");
    }
    for s in [sock_ruby, sock_rust] {
        tmux_on(s, &["kill-server"]);
    }
}

fn session_traces(root: &Path, other: &Path) -> String {
    let log = fs::read_to_string(root.join("sessions.jsonl")).unwrap_or_default();
    let log = regex::Regex::new(r#""(ts|pid|cwd)":("[^"]*"|[0-9]+)"#).unwrap().replace_all(&log, "\"$1\":X").into_owned();
    format!("{}== sessions\n{log}", action_traces(root, other))
}

#[test]
fn the_item_and_session_actions_match_ruby() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let stamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut log = fs::read_to_string(w.root.join("sessions.jsonl")).unwrap();
    for (id, extra) in [("ifs-feature", ",\"project\":\"demo\",\"branch\":\"feat/x\""), ("ifs-main", ",\"project\":\"demo\",\"branch\":\"main\""), ("ifs-master", ",\"project\":\"demo\",\"branch\":\"master\""), ("ifs-develop", ",\"project\":\"demo\",\"branch\":\"develop\""), ("ifs-none", ""), ("ifs-claimed", ",\"project\":\"demo\""), ("ifs-odd", ",\"project\":\"demo\",\"branch\":\"a: b #c\"")] {
        log += &format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"/srv/app\",\"pid\":999999,\"ts\":\"{stamp}\"{extra}}}\n");
    }
    fs::write(w.root.join("sessions.jsonl"), log).unwrap();
    let item = |slug: &str, extra: &str| {
        fs::write(w.root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\ntitle: {slug}\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\n{extra}---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n")).unwrap();
    };
    item("owner", "sessions:\n- id: ifs-claimed\n");
    item("pf-a", "sessions:\n- id: sess-pf-a\n");
    item("pf-w", "sessions:\n- id: sess-pf-w\n");
    item("pf-b", "");
    item("pf-x", "");
    let chat = base.path().join("fake-chat");
    fs::write(&chat, "#!/bin/sh\nprintf '{\"v\":1,\"event\":\"start\",\"session_id\":\"sess-%s\",\"cwd\":\"%s\",\"pid\":%s,\"ts\":\"%s\"}\\n' \"$QM_ITEM\" \"$(pwd)\" \"$$\" \"$(date -u +%FT%TZ)\" >> \"$QM_ROOT/sessions.jsonl\"\ntouch \"$FAKE_REC/up-$QM_ITEM\"\nexec sleep 300\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&chat, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let (rec_ruby, rec_rust) = (base.path().join("rec-ruby"), base.path().join("rec-rust"));
    for d in [&rec_ruby, &rec_rust] {
        fs::create_dir_all(d).unwrap();
    }
    let (sock_ruby, sock_rust) = ("qm-test-item-ruby", "qm-test-item-rust");
    for s in [sock_ruby, sock_rust] {
        tmux_on(s, &["kill-server"]);
    }
    let env_for = |sock: &str, rec: &Path| -> Vec<(&'static str, String)> {
        vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("QM_TMUX_SOCKET", sock.to_string()), ("QM_CHAT_COMMAND", chat.display().to_string()), ("FAKE_REC", rec.display().to_string())]
    };
    let root2 = base.path().join("root2");
    copy_dir(&w.root, &root2);
    let ruby = start_env(false, &w.root, &w.home, &env_for(sock_ruby, &rec_ruby));
    let rust = start_env(true, &root2, &w.home, &env_for(sock_rust, &rec_rust));
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let step = |label: &str, body: &str, wait_for: Option<&str>| {
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, body, true), request(rust.port, "POST", "/api/action", &origin, body, true));
        assert_eq!(shown(&a, ruby.port), shown(&b, rust.port), "{label}: reply to {body}");
        if let Some(item) = wait_for {
            for _ in 0..100 {
                if rec_ruby.join(format!("up-{item}")).exists() && rec_rust.join(format!("up-{item}")).exists() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        assert_eq!(session_traces(&w.root, &root2), session_traces(&root2, &w.root), "{label}: files after {body}");
        let sessions = |s: &str| {
            let mut v: Vec<String> = tmux_on(s, &["list-sessions", "-F", "#{session_name}"]).lines().map(String::from).collect();
            v.sort();
            v
        };
        assert_eq!(sessions(sock_ruby), sessions(sock_rust), "{label}: tmux after {body}");
        (shown(&a, ruby.port), sessions(sock_rust))
    };
    let act = |cmd: &str, slug: &str, args: &str| format!("{{\"cmd\":\"{cmd}\",\"slug\":\"{slug}\",\"args\":{args}}}");
    for (label, body) in [
        ("new", act("new", "", r#"{"title":"Brand new"}"#)),
        ("new again", act("new", "", r#"{"title":"Brand new"}"#)),
        ("new with a description", act("new", "", r#"{"title":"  Described  ","description":"  line one\nline two  ","project":"demo"}"#)),
        ("new with symbols", act("new", "", r#"{"title":"He said: \"hi\" — #1, ok?"}"#)),
        ("new null project", act("new", "", r#"{"title":"Null project","project":null}"#)),
        ("new false project", act("new", "", r#"{"title":"False project","project":false}"#)),
        ("new quoted project", act("new", "", r#"{"title":"Quoted project","project":"a: b"}"#)),
        ("new blank", act("new", "", r#"{"title":"   "}"#)),
        ("new number title", act("new", "", r#"{"title":42}"#)),
        ("new no args", r#"{"cmd":"new"}"#.to_string()),
        ("attach", act("attach", "pf-b", r#"{"session":"ifs-feature"}"#)),
        ("attach again", act("attach", "pf-b", r#"{"session":"ifs-feature"}"#)),
        ("attach moves it", act("attach", "pf-x", r#"{"session":"ifs-claimed"}"#)),
        ("attach unknown", act("attach", "pf-b", r#"{"session":"nobody"}"#)),
        ("attach blank", act("attach", "pf-b", r#"{"session":""}"#)),
        ("attach missing item", act("attach", "nowhere", r#"{"session":"ifs-none"}"#)),
        ("item from a session", act("item-from-session", "", r#"{"session":"ifs-feature","title":"From feature"}"#)),
        ("item from main", act("item-from-session", "", r#"{"session":"ifs-main","title":"From main"}"#)),
        ("item from master", act("item-from-session", "", r#"{"session":"ifs-master","title":"From master"}"#)),
        ("item from develop", act("item-from-session", "", r#"{"session":"ifs-develop","title":"From develop"}"#)),
        ("item from nothing", act("item-from-session", "", r#"{"session":"ifs-none","title":"From nothing"}"#)),
        ("item from an odd branch", act("item-from-session", "", r#"{"session":"ifs-odd","title":"From odd"}"#)),
        ("item from a claimed session", act("item-from-session", "", r#"{"session":"ifs-claimed","title":"From claimed"}"#)),
        ("item from unknown", act("item-from-session", "", r#"{"session":"nope","title":"Never"}"#)),
        ("item from, no title", act("item-from-session", "", r#"{"session":"ifs-none","title":" "}"#)),
        ("item from, taken title", act("item-from-session", "", r#"{"session":"ifs-none","title":"Brand new"}"#)),
        ("dismiss", act("dismiss", "", r#"{"session":"ifs-none"}"#)),
        ("dismiss again", act("dismiss", "", r#"{"session":"ifs-none"}"#)),
        ("dismiss an unknown id", act("dismiss", "", r#"{"session":"who"}"#)),
    ] {
        step(label, &body, None);
    }
    // a chat pane follows its session to the item it moves to
    step("open a pane", &act("pane-start", "pf-a", "{}"), Some("pf-a"));
    let (_, panes) = step("move the session", &act("attach", "pf-b", r#"{"session":"sess-pf-a"}"#), None);
    assert!(panes.contains(&"pf-b".to_string()) && !panes.contains(&"pf-a".to_string()), "the pane should follow: {panes:?}");
    step("open the other pane", &act("pane-start", "pf-w", "{}"), Some("pf-w"));
    step("open the target's pane", &act("pane-start", "pf-x", "{}"), Some("pf-x"));
    let (reply, _) = step("move to an item with a chat", &act("attach", "pf-x", r#"{"session":"sess-pf-w"}"#), None);
    assert!(reply.contains("\"warning\":\"pf-w's chat pane stays named pf-w: pf-x already has a chat running\""), "{reply}");
    for s in [sock_ruby, sock_rust] {
        tmux_on(s, &["kill-server"]);
    }
    // what is not read as Ruby reads it answers 501 with nothing written
    let before = session_traces(&root2, &w.root);
    for body in [act("new", "", r#"{"title":"Odd","project":5}"#), act("attach", "pf-b", r#"{"session":null}"#), act("dismiss", "", r#"{"session":5}"#), act("item-from-session", "", r#"{"session":null,"title":"x"}"#)] {
        let a = request(rust.port, "POST", "/api/action", &origin, &body, true);
        assert!(a.status.starts_with("501"), "{body}: {}", a.status);
    }
    assert_eq!(before, session_traces(&root2, &w.root));
}

#[test]
fn the_aside_actions_match_ruby() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let me = std::process::id();
    let stamp = |secs: i64| (chrono::Utc::now() - chrono::Duration::seconds(secs)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let work = base.path().join("work-a");
    fs::create_dir_all(&work).unwrap();
    let assistant = |id: &str, model: &str, blocks: &[&str]| -> String {
        let content: Vec<serde_json::Value> = blocks.iter().map(|t| serde_json::json!({"type": "text", "text": t})).collect();
        format!("{}\n", serde_json::json!({"type": "assistant", "message": {"id": id, "model": model, "content": content}}))
    };
    let mut log = String::new();
    let mut aside = |id: &str, slug: &str, transcript: Option<String>, pid: u32, age: i64| {
        let path = w.transcripts.join(format!("p/{id}.jsonl"));
        match transcript {
            Some(text) => {
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, text).unwrap();
            }
            None => {}
        }
        log += &format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"{}\",\"pid\":{pid},\"aside\":\"{slug}\",\"transcript_path\":\"{}\",\"ts\":\"{}\"}}\n", work.display(), path.display(), stamp(age));
    };
    aside("a-opus", "as-one", Some(format!("junk line\n{}{}{}", assistant("m1", "claude-opus-5-5", &["first answer", "  with\n a   second block "]), assistant("m2", "<synthetic>", &["ignored model"]), "{\"type\":\"user\",\"message\":{\"content\":\"hi\"}}\n")), 999999, 900);
    aside("a-multi", "as-one", Some(format!("{}{}{}", assistant("m1", "claude-haiku-5-5", &["part one"]), assistant("m1", "claude-haiku-5-5", &["part two"]), assistant("m2", "claude-haiku-5-5", &["final", "answer"]))), 999999, 800);
    aside("a-join", "as-one", Some(format!("{}{}{}", assistant("m0", "claude-opus-5-5", &["earlier"]), assistant("m1", "claude-opus-5-5", &["part one"]), assistant("m1", "claude-opus-5-5", &["part two"]))), 999999, 650);
    aside("a-switch", "as-one", Some(format!("{}{}", assistant("m1", "claude-opus-5-5", &["on opus"]), assistant("m2", "claude-sonnet-5-5", &["on sonnet"]))), 999999, 640);
    aside("a-blankmsg", "as-one", Some(format!("{}{}", assistant("m1", "claude-opus-5-5", &["the real answer"]), "{\"type\":\"assistant\",\"message\":{\"id\":\"m2\",\"content\":\"   \"}}\n")), 999999, 630);
    aside("a-synth", "as-one", Some(assistant("m1", "<synthetic>", &["only synthetic"])), 999999, 700);
    aside("a-empty", "as-one", Some("{\"type\":\"user\"}\n".to_string()), 999999, 600);
    aside("a-gone", "as-one", None, 999999, 500);
    aside("a-other", "as-two", Some(assistant("m1", "claude-opus-5-5", &["not yours"])), 999999, 400);
    aside("a-live-old", "as-live", Some(assistant("m1", "claude-opus-5-5", &["old live answer"])), me as u32, 300);
    aside("a-live-new", "as-live", Some(assistant("m1", "claude-opus-5-5", &["new live answer"])), me as u32, 100);
    aside("a-live-none", "as-nolive", Some(assistant("m1", "claude-opus-5-5", &["ended answer"])), 999999, 100);
    fs::write(w.root.join("sessions.jsonl"), fs::read_to_string(w.root.join("sessions.jsonl")).unwrap() + &log).unwrap();
    let item = |slug: &str| {
        fs::write(w.root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\ntitle: {slug}\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\n---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n")).unwrap();
    };
    for slug in ["as-one", "as-two", "as-live", "as-nolive", "as-fresh", "as-dotted.x"] {
        item(slug);
    }
    let chat = base.path().join("fake-chat");
    fs::write(&chat, "#!/bin/sh\n{ echo \"cwd: $(pwd)\"; echo \"args: $*\"; echo \"QM_ASIDE=${QM_ASIDE-unset}\"; echo \"QM_ITEM=[${QM_ITEM-unset}]\"; } > \"$FAKE_REC/.tmp-$$\"\nmv \"$FAKE_REC/.tmp-$$\" \"$FAKE_REC/rec-$$\"\nexec sleep 300\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&chat, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let (rec_ruby, rec_rust) = (base.path().join("rec-ruby"), base.path().join("rec-rust"));
    for d in [&rec_ruby, &rec_rust] {
        fs::create_dir_all(d).unwrap();
    }
    let (sock_ruby, sock_rust) = ("qm-test-aside-ruby", "qm-test-aside-rust");
    for s in [sock_ruby, sock_rust] {
        tmux_on(s, &["kill-server"]);
    }
    let env_for = |sock: &str, rec: &Path| -> Vec<(&'static str, String)> {
        vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("QM_TMUX_SOCKET", sock.to_string()), ("QM_CHAT_COMMAND", format!("{} --flag", chat.display())), ("FAKE_REC", rec.display().to_string())]
    };
    let root2 = base.path().join("root2");
    copy_dir(&w.root, &root2);
    let ruby = start_env(false, &w.root, &w.home, &env_for(sock_ruby, &rec_ruby));
    let rust = start_env(true, &root2, &w.home, &env_for(sock_rust, &rec_rust));
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let mut opened = 0usize;
    let mut step = |label: &str, body: &str| -> String {
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, body, true), request(rust.port, "POST", "/api/action", &origin, body, true));
        assert_eq!(shown(&a, ruby.port), shown(&b, rust.port), "{label}: reply to {body}");
        let text = String::from_utf8_lossy(&a.body).into_owned();
        if text.contains("\"session\"") && !text.contains("\"attached\":true") {
            opened += 1;
        }
        for _ in 0..100 {
            if records(&rec_ruby).len() >= opened && records(&rec_rust).len() >= opened {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(records(&rec_ruby), records(&rec_rust), "{label}: what the asides were started with, after {body}");
        assert_eq!(tmux_picture(sock_ruby), tmux_picture(sock_rust), "{label}: tmux after {body}");
        assert_eq!(action_traces(&w.root, &root2), action_traces(&root2, &w.root), "{label}: files after {body}");
        text
    };
    let act = |cmd: &str, slug: &str, args: &str| format!("{{\"cmd\":\"{cmd}\",\"slug\":\"{slug}\",\"args\":{args}}}");
    step("open", &act("aside-open", "as-fresh", "{}"));
    step("open again", &act("aside-open", "as-fresh", r#"{"model":"opus"}"#));
    step("close", &act("aside-close", "as-fresh", "{}"));
    step("close again", &act("aside-close", "as-fresh", "{}"));
    step("open a model", &act("aside-open", "as-fresh", r#"{"model":"haiku"}"#));
    step("close it", &act("aside-close", "as-fresh", "{}"));
    step("open an unknown model", &act("aside-open", "as-fresh", r#"{"model":"gpt"}"#));
    step("open a quoted model", &act("aside-open", "as-fresh", r#"{"model":"a \"b\" \\c #{d}"}"#));
    step("open a numeric model", &act("aside-open", "as-fresh", r#"{"model":5}"#));
    step("open an unusable name", &act("aside-open", "as-dotted.x", "{}"));
    step("open for a missing item", &act("aside-open", "nowhere", "{}"));
    step("close a missing item", &act("aside-close", "nowhere", "{}"));
    step("reopen the model it ran", &act("aside-reopen", "as-one", r#"{"session":"a-opus"}"#));
    step("reopen while one is open", &act("aside-reopen", "as-one", r#"{"session":"a-multi"}"#));
    step("close for the next", &act("aside-close", "as-one", "{}"));
    step("close before the model switch", &act("aside-close", "as-one", "{}"));
    step("reopen a switched model", &act("aside-reopen", "as-one", r#"{"session":"a-switch"}"#));
    step("close after the switch", &act("aside-close", "as-one", "{}"));
    step("reopen under a synthetic model", &act("aside-reopen", "as-one", r#"{"session":"a-synth"}"#));
    step("close again", &act("aside-close", "as-one", "{}"));
    step("reopen another", &act("aside-reopen", "as-one", r#"{"session":"a-multi"}"#));
    step("close once more", &act("aside-close", "as-one", "{}"));
    step("reopen without a transcript", &act("aside-reopen", "as-one", r#"{"session":"a-gone"}"#));
    step("reopen another item's aside", &act("aside-reopen", "as-one", r#"{"session":"a-other"}"#));
    step("reopen an unknown session", &act("aside-reopen", "as-one", r#"{"session":"a-very-long-unknown-id"}"#));
    step("reopen nothing", &act("aside-reopen", "as-one", "{}"));
    step("reopen a number", &act("aside-reopen", "as-one", r#"{"session":123456789012}"#));
    step("reopen a missing item", &act("aside-reopen", "nowhere", r#"{"session":"a-opus"}"#));
    let seen = records(&rust_dir(&rec_rust)).join("\n");
    assert!(seen.contains("--model haiku") && seen.contains("--model claude-opus-5-5 --resume a-opus") && seen.contains("QM_ASIDE=as-one") && seen.contains("QM_ITEM=[]"), "{seen}");
    let mut note = |label: &str, slug: &str, args: &str| step(label, &act("aside-promote", slug, args));
    let promoted = note("promote a session", "as-one", r#"{"session":"a-opus"}"#);
    assert!(promoted.contains("aside: ignored model"), "the last assistant message with text is the answer: {promoted}");
    let joined = note("promote joined blocks", "as-one", r#"{"session":"a-multi"}"#);
    assert!(joined.contains("aside: final answer"), "{joined}");
    let spliced = note("promote blocks of one message", "as-one", r#"{"session":"a-join"}"#);
    assert!(spliced.contains("aside: part one part two"), "{spliced}");
    let blank = note("promote past a blank message", "as-one", r#"{"session":"a-blankmsg"}"#);
    assert!(blank.contains("aside: the real answer"), "{blank}");
    note("promote the same again", "as-one", r#"{"session":"a-multi"}"#);
    note("promote without an answer", "as-one", r#"{"session":"a-empty"}"#);
    note("promote without a transcript", "as-one", r#"{"session":"a-gone"}"#);
    note("promote another item's", "as-one", r#"{"session":"a-other"}"#);
    note("promote an unknown", "as-one", r#"{"session":"zzz"}"#);
    note("promote a blank session", "as-one", r#"{"session":"  "}"#);
    note("promote the running one", "as-live", "{}");
    note("promote with none running", "as-nolive", "{}");
    note("promote for a missing item", "nowhere", "{}");
    let live = note("promote the running one once more", "as-live", r#"{"session":null}"#);
    assert!(live.contains("aside: new live answer"), "{live}");
    for s in [sock_ruby, sock_rust] {
        tmux_on(s, &["kill-server"]);
    }
}

fn rust_dir(p: &Path) -> PathBuf {
    p.to_path_buf()
}

#[test]
fn the_loose_chat_actions_match_ruby() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let config = fs::read_to_string(w.root.join("config.yml")).unwrap();
    fs::write(w.root.join("config.yml"), config + "  gone: /nonexistent/project/root\nloose_starters:\n  dev:\n    label: Dev docs\n    prompt: \"  Load the dev docs.  \"\n  nolabel:\n    prompt: Read the guide.\n  emptylabel:\n    label: \"\"\n    prompt: Read x.\n  empty:\n    prompt: \"  \"\n  broken: notamap\n  5:\n    prompt: numeric key\n").unwrap();
    let chat = base.path().join("fake-chat");
    // the stand-in chat records how it was started and registers a session, as the SessionStart hook would
    fs::write(&chat, "#!/bin/sh\n{ echo \"cwd: $(pwd)\"; echo \"args: $*\"; echo \"QM_LOOSE=${QM_LOOSE-unset}\"; echo \"QM_ITEM=[${QM_ITEM-unset}]\"; } > \"$FAKE_REC/.tmp-$$\"\nmv \"$FAKE_REC/.tmp-$$\" \"$FAKE_REC/rec-$$\"\nprintf '{\"v\":1,\"event\":\"start\",\"session_id\":\"sess-%s\",\"cwd\":\"%s\",\"pid\":%s,\"project\":\"demo\",\"loose\":\"%s\",\"ts\":\"%s\"}\\n' \"$QM_LOOSE\" \"$(pwd)\" \"$$\" \"$QM_LOOSE\" \"$(date -u +%FT%TZ)\" >> \"$QM_ROOT/sessions.jsonl\"\nexec sleep 300\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&chat, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let (rec_ruby, rec_rust) = (base.path().join("rec-ruby"), base.path().join("rec-rust"));
    for d in [&rec_ruby, &rec_rust] {
        fs::create_dir_all(d).unwrap();
    }
    let (sock_ruby, sock_rust) = ("qm-test-loose-ruby", "qm-test-loose-rust");
    for (s, root, rec) in [(sock_ruby, &w.root, &rec_ruby), (sock_rust, &base.path().join("root2"), &rec_rust)] {
        tmux_on(s, &["kill-server"]);
        // the tmux server keeps the environment it was started with, and the chats inherit it: start it as the GUI would
        let start = |args: &[&str]| Command::new("tmux").args(["-L", s]).args(args).env_remove("TMUX").env("QM_ROOT", root).env("FAKE_REC", rec).output().unwrap();
        // a server that was just killed may still be winding down: ask until the session is there
        for _ in 0..50 {
            start(&["new-session", "-d", "-s", "warn-me", "-n", "warn-me", "sleep 300"]);
            if tmux_on(s, &["list-sessions", "-F", "#{session_name}"]).contains("warn-me") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        start(&["new-session", "-d", "-s", "warn-two", "-n", "warn-two", "sleep 300"]);
    }
    let env_for = |sock: &str, rec: &Path| -> Vec<(&'static str, String)> {
        vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("QM_TMUX_SOCKET", sock.to_string()), ("QM_CHAT_COMMAND", format!("{} --flag", chat.display())), ("FAKE_REC", rec.display().to_string())]
    };
    let root2 = base.path().join("root2");
    copy_dir(&w.root, &root2);
    let ruby = start_env(false, &w.root, &w.home, &env_for(sock_ruby, &rec_ruby));
    let rust = start_env(true, &root2, &w.home, &env_for(sock_rust, &rec_rust));
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let (mut ruby_ids, mut rust_ids): (Vec<String>, Vec<String>) = (vec![], vec![]);
    let id_pattern = regex::Regex::new(r#""id":"([0-9a-f]{8})""#).unwrap();
    let mut step = |label: &str, body: &str| -> String {
        let fill = |ids: &Vec<String>| (0..8).fold(body.to_string(), |b, i| b.replace(&format!("{{ID{i}}}"), ids.get(i).map(String::as_str).unwrap_or("")));
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, &fill(&ruby_ids), true), request(rust.port, "POST", "/api/action", &origin, &fill(&rust_ids), true));
        let norm = |text: String, ids: &Vec<String>| ids.iter().enumerate().fold(text, |t, (i, id)| t.replace(id.as_str(), &format!("ID{i}")));
        let (ta, tb) = (String::from_utf8_lossy(&a.body).into_owned(), String::from_utf8_lossy(&b.body).into_owned());
        for (ids, text) in [(&mut ruby_ids, &ta), (&mut rust_ids, &tb)] {
            if let Some(c) = id_pattern.captures(text) {
                ids.push(c[1].to_string());
            }
        }
        assert_eq!(norm(shown(&a, ruby.port), &ruby_ids), norm(shown(&b, rust.port), &rust_ids), "{label}: reply to {body}");
        let started = ruby_ids.len();
        // the chats register themselves; wait for the newest one in both roots
        for _ in 0..100 {
            let have = |root: &Path, ids: &Vec<String>| ids.last().is_none_or(|id| fs::read_to_string(root.join("sessions.jsonl")).unwrap_or_default().contains(&format!("sess-{id}")));
            if have(&w.root, &ruby_ids) && have(&root2, &rust_ids) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let recs = |dir: &Path, ids: &Vec<String>| {
            let mut all: Vec<String> = records(dir).into_iter().map(|r| norm(r, ids)).collect();
            all.sort();
            all
        };
        assert_eq!(recs(&rec_ruby, &ruby_ids), recs(&rec_rust, &rust_ids), "{label}: what the chats were started with, after {body} ({started} started)");
        let picture = |sock: &str, ids: &Vec<String>| tmux_picture_with(sock, &|t| norm(t, ids));
        assert_eq!(picture(sock_ruby, &ruby_ids), picture(sock_rust, &rust_ids), "{label}: tmux after {body}");
        assert_eq!(norm(action_traces(&w.root, &root2), &ruby_ids), norm(action_traces(&root2, &w.root), &rust_ids), "{label}: files after {body}");
        norm(shown(&a, ruby.port), &ruby_ids)
    };
    let act = |cmd: &str, args: &str| format!("{{\"cmd\":\"{cmd}\",\"slug\":null,\"args\":{args}}}");
    step("start", &act("loose-start", "{}"));
    step("start in a project", &act("loose-start", r#"{"project":"demo","model":"opus","prompt":"  what is this?  "}"#));
    step("start for a project that is not there", &act("loose-start", r#"{"project":"ghost","model":" ","prompt":"  "}"#));
    step("start for a project whose root is gone", &act("loose-start", r#"{"project":"gone"}"#));
    step("start on haiku", &act("loose-start", r#"{"model":"haiku"}"#));
    step("start on sonnet", &act("loose-start", r#"{"model":"sonnet"}"#));
    step("start with a bad model", &act("loose-start", r#"{"model":"gpt"}"#));
    step("start with a quoted model", &act("loose-start", r#"{"model":"a \"b\" \\c"}"#));
    step("start with a numeric project", &act("loose-start", r#"{"project":5,"prompt":7}"#));
    step("start blank", &act("loose-start", r#"{"project":"  ","starter":"  ","prompt":null}"#));
    step("start with a starter", &act("loose-start", r#"{"starter":"dev"}"#));
    step("start with a starter and a question", &act("loose-start", r#"{"starter":" nolabel ","prompt":"  why?  ","project":"demo"}"#));
    step("start with a starter and a model", &act("loose-start", r#"{"starter":"emptylabel","model":"opus"}"#));
    step("start with a numeric starter", &act("loose-start", r#"{"starter":"5"}"#));
    step("start with an unknown starter", &act("loose-start", r#"{"starter":"nope"}"#));
    step("start with a starter that has no prompt", &act("loose-start", r#"{"starter":"empty"}"#));
    step("start with a starter that is no map", &act("loose-start", r#"{"starter":"broken"}"#));
    step("start with a quoted starter", &act("loose-start", r#"{"starter":"a \"b\""}"#));
    step("start with a bad model and an unknown starter", &act("loose-start", r#"{"starter":"nope","model":"gpt"}"#));
    let promoted = step("promote", &act("loose-promote", r#"{"id":"{ID1}","title":"  Promoted chat  "}"#));
    assert!(promoted.contains("\"slug\":\"promoted-chat\"") && !promoted.contains("warning"), "{promoted}");
    step("promote the same title", &act("loose-promote", r#"{"id":"{ID2}","title":"Promoted chat"}"#));
    let warned = step("promote onto a running chat", &act("loose-promote", r#"{"id":"{ID2}","title":"Warn me"}"#));
    assert!(warned.contains("\"warning\":\"warn-me is created, but its chat stays under loose--ID2: a chat named warn-me already runs\""), "{warned}");
    step("promote without a title", &act("loose-promote", r#"{"id":"{ID0}","title":"   "}"#));
    step("promote an unknown chat", &act("loose-promote", r#"{"id":"deadbeef","title":"Never"}"#));
    step("promote nothing", &act("loose-promote", "{}"));
    step("promote an old id", &act("loose-promote", r#"{"id":"{ID1}","title":"Again"}"#));
    let told = step("promote and tell the chat", &act("loose-promote", r#"{"id":"{ID3}","title":"Told chat","tell":true}"#));
    assert!(told.contains("\"told\":true"), "{told}");
    std::thread::sleep(std::time::Duration::from_millis(300));
    let pane = |sock: &str| tmux_on(sock, &["capture-pane", "-p", "-t", "=told-chat:"]);
    assert_eq!(pane(sock_ruby), pane(sock_rust), "what the chat was told");
    assert!(pane(sock_rust).contains("This chat is now work item told-chat; run qm show told-chat"), "{}", pane(sock_rust));
    let warned_told = step("promote and tell onto a running chat", &act("loose-promote", r#"{"id":"{ID4}","title":"Warn two","tell":true}"#));
    assert!(warned_told.contains("\"told\":false"), "{warned_told}");
    step("promote and tell nothing", &act("loose-promote", r#"{"id":"{ID5}","title":"Plain again","tell":false}"#));
    step("promote and tell with a title missing", &act("loose-promote", r#"{"id":"{ID6}","title":" ","tell":true}"#));
    for s in [sock_ruby, sock_rust] {
        tmux_on(s, &["kill-server"]);
    }
    // what is not read as Ruby does, and typing into the chat, stay Ruby's: nothing is written
    let before = action_traces(&root2, &w.root);
    for body in [act("loose-start", r#"{"model":"é"}"#)] {
        let a = request(rust.port, "POST", "/api/action", &origin, &body, true);
        assert!(a.status.starts_with("501"), "{body}: {}", a.status);
    }
    assert_eq!(before, action_traces(&root2, &w.root));
}

#[test]
fn closing_a_loose_chat_runs_as_a_job_like_ruby() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let chat = base.path().join("fake-chat");
    fs::write(&chat, "#!/bin/sh\nprintf '{\"v\":1,\"event\":\"start\",\"session_id\":\"sess-%s\",\"cwd\":\"%s\",\"pid\":%s,\"project\":\"demo\",\"loose\":\"%s\",\"ts\":\"%s\"}\\n' \"$QM_LOOSE\" \"$(pwd)\" \"$$\" \"$QM_LOOSE\" \"$(date -u +%FT%TZ)\" >> \"$QM_ROOT/sessions.jsonl\"\nread -r line\necho \"$line\" > \"$QM_ROOT/typed-$QM_LOOSE\"\nexec sleep 300\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&chat, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let (sock_ruby, sock_rust) = ("qm-test-close-ruby", "qm-test-close-rust");
    for s in [sock_ruby, sock_rust] {
        tmux_on(s, &["kill-server"]);
    }
    let env_for = |sock: &str| -> Vec<(&'static str, String)> { vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("QM_TMUX_SOCKET", sock.to_string()), ("QM_CHAT_COMMAND", chat.display().to_string())] };
    let root2 = base.path().join("root2");
    copy_dir(&w.root, &root2);
    let ruby = start_env(false, &w.root, &w.home, &env_for(sock_ruby));
    let rust = start_env(true, &root2, &w.home, &env_for(sock_rust));
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let post = |server: &Server, body: &str| String::from_utf8_lossy(&request(server.port, "POST", "/api/action", &origin, body, true).body).into_owned();
    let state = |server: &Server| -> serde_json::Value { serde_json::from_slice(&request(server.port, "GET", "/api/state", &[], "", true).body).expect("a state") };
    let id_of = |reply: &str| serde_json::from_str::<serde_json::Value>(reply).unwrap()["id"].as_str().unwrap().to_string();
    let act = |cmd: &str, args: &str| format!("{{\"cmd\":\"{cmd}\",\"slug\":null,\"args\":{args}}}");
    let mut ids: Vec<(String, String)> = vec![];
    for _ in 0..3 {
        let (a, b) = (post(&ruby, &act("loose-start", "{}")), post(&rust, &act("loose-start", "{}")));
        ids.push((id_of(&a), id_of(&b)));
    }
    for _ in 0..100 {
        let seen = |root: &Path, pick: &dyn Fn(&(String, String)) -> String| ids.iter().all(|i| fs::read_to_string(root.join("sessions.jsonl")).unwrap_or_default().contains(&format!("sess-{}", pick(i))));
        if seen(&w.root, &|i| i.0.clone()) && seen(&root2, &|i| i.1.clone()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let norm = |text: String, side: usize| ids.iter().enumerate().fold(text, |t, (n, i)| t.replace(if side == 0 { &i.0 } else { &i.1 }, &format!("ID{n}")));
    let both = |label: &str, fields: &[&str]| -> String {
        let (a, b) = (state(&ruby), state(&rust));
        let pick = |s: &serde_json::Value, side: usize| norm(fields.iter().map(|f| format!("{f}: {}", s[*f])).collect::<Vec<_>>().join("\n"), side);
        assert_eq!(pick(&a, 0), pick(&b, 1), "{label}");
        pick(&a, 0)
    };
    let close = |n: usize, side_body: &dyn Fn(&str) -> String| -> (String, String) { (post(&ruby, &side_body(&ids[n].0)), post(&rust, &side_body(&ids[n].1))) };
    let closing = |id: &str| act("loose-close", &format!("{{\"id\":\"{id}\"}}"));
    // two chats closing at once: running jobs show as badges, the same chat again is busy
    let (a, b) = close(0, &closing);
    assert_eq!((a.as_str(), b.as_str()), ("{\"ok\":true,\"background\":true}", "{\"ok\":true,\"background\":true}"));
    let (a, b) = close(1, &closing);
    assert_eq!(a, b);
    let (a, b) = close(0, &closing);
    assert_eq!(norm(a, 0), norm(b, 1));
    let running = both("while closing", &["jobs"]);
    assert!(running.contains("closing…") && running.contains("ID0") && running.contains("ID1"), "{running}");
    // wait for both jobs to finish; every state build hands out what finished since the last one, so keep it
    let (mut got_ruby, mut got_rust): (Vec<String>, Vec<String>) = (vec![], vec![]);
    for _ in 0..200 {
        let (a, b) = (state(&ruby), state(&rust));
        got_ruby.extend(a["finished"].as_array().unwrap().iter().map(|v| norm(v.to_string(), 0)));
        got_rust.extend(b["finished"].as_array().unwrap().iter().map(|v| norm(v.to_string(), 1)));
        if a["jobs"].as_array().is_some_and(|j| j.is_empty()) && b["jobs"].as_array().is_some_and(|j| j.is_empty()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    got_ruby.sort();
    got_rust.sort();
    assert_eq!(got_ruby, got_rust);
    let fa = got_ruby.join("\n");
    assert!(got_ruby.len() == 2 && fa.contains("loose chat closed") && fa.contains("\"key\":\"loose--ID0\"") && fa.contains("\"key\":\"loose--ID1\"") && fa.contains("\"label\":\"closing…\""), "{fa}");
    let after = both("after closing", &["finished", "jobs", "loose", "inbox"]);
    assert!(after.contains("finished: []") && after.contains("ID2") && !after.contains("loose--ID0"), "{after}");
    assert_eq!(tmux_picture_with(sock_ruby, &|t| norm(t, 0)).lines().next(), tmux_picture_with(sock_rust, &|t| norm(t, 1)).lines().next());
    // a chat that is not running closes at once, and still dismisses its sessions
    let (a, b) = close(0, &closing);
    assert_eq!(a, b);
    std::thread::sleep(std::time::Duration::from_millis(600));
    let (fa, fb) = (norm(state(&ruby)["finished"].to_string(), 0), norm(state(&rust)["finished"].to_string(), 1));
    assert_eq!(fa, fb);
    assert!(fa.contains("\"key\":\"loose--ID0\""), "{fa}");
    for body in [act("loose-close", r#"{"id":""}"#), act("loose-close", "{}"), act("loose-close", r#"{"id":null}"#)] {
        assert_eq!(post(&ruby, &body), post(&rust, &body), "{body}");
    }
    let events = |root: &Path, side: usize| {
        let stamp = regex::Regex::new(r#""ts":"[^"]*""#).unwrap();
        let lines: std::collections::BTreeSet<String> = fs::read_to_string(root.join("sessions.jsonl")).unwrap().lines().filter(|l| l.contains("dismissed")).map(|l| norm(stamp.replace_all(l, "\"ts\":X").into_owned(), side)).collect();
        lines.into_iter().collect::<Vec<_>>().join("\n")
    };
    assert_eq!(events(&w.root, 0), events(&root2, 1));
    assert!(events(&root2, 1).contains("sess-ID0") && events(&root2, 1).contains("sess-ID1"));
    // /exit and Enter were typed into the chats that were closed, and only those
    for (n, (a, b)) in ids.iter().enumerate() {
        let typed = |root: &Path, id: &str| fs::read_to_string(root.join(format!("typed-{id}"))).ok();
        assert_eq!(typed(&w.root, a), typed(&root2, b), "chat {n}");
        assert_eq!(typed(&root2, b), (n < 2).then(|| "/exit\n".to_string()), "chat {n}");
    }
    for s in [sock_ruby, sock_rust] {
        tmux_on(s, &["kill-server"]);
    }
}

/// The world the resolving and cleanup tests share: worktrees merged, dirty and with unpreserved
/// commits, items on them, issue drafts, and fresh PR answers so no plan waits on a lookup.
fn cleanup_world(base: &Path) -> common::AttentionWorld {
    let w = attention_world(base, std::time::SystemTime::now(), None);
    let repo = &w.repo;
    git(repo, &["worktree", "add", base.join("wt-merged").to_str().unwrap(), "merged-b"]);
    let dirty = base.join("wt-dirty");
    git(repo, &["worktree", "add", "-b", "dirty-b", dirty.to_str().unwrap()]);
    fs::write(dirty.join("loose.txt"), "uncommitted\n").unwrap();
    let unpushed = base.join("wt-unpushed");
    git(repo, &["worktree", "add", "-b", "unpushed-b", unpushed.to_str().unwrap()]);
    fs::write(unpushed.join("work.txt"), "work\n").unwrap();
    git(&unpushed, &["add", "."]);
    git(&unpushed, &["commit", "-q", "-m", "work nobody has"]);
    git(repo, &["branch", "spare-b"]);
    let item = |slug: &str, extra: &str, state: &str| {
        fs::write(w.root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\ntitle: {slug}\nstate: {state}\nproject: demo\ncreated: '2026-01-01'\n{extra}---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n")).unwrap();
    };
    item("res-merged", "branches:\n- merged-b\n", "resolved");
    item("res-dirty", "branches:\n- dirty-b\n", "resolved");
    item("res-unpushed", "branches:\n- unpushed-b\n", "resolved");
    item("res-spare", "branches:\n- spare-b\n", "resolved");
    item("res-issues", "", "resolved");
    item("res-nothing", "", "resolved");
    for (name, front) in [("01-made.md", "title: Made\nkind: bug\nnumber: 9\n"), ("02-open.md", "title: Open\nkind: feature\n")] {
        let p = w.root.join(format!("issues/res-issues/{name}"));
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, format!("---\n{front}---\n\nbody\n")).unwrap();
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let entry = |state: &str, n: u32| format!("{{\"state\":{state},\"number\":{n},\"url\":\"https://example.test/pull/{n}\",\"ts\":{now}}}");
    fs::write(
        w.root.join("pr-cache.json"),
        format!("{{\"demo#merged-b\":{},\"demo#dirty-b\":{},\"demo#unpushed-b\":{{\"state\":null,\"ts\":{now}}},\"demo#spare-b\":{},\"demo#nopr-exists\":{{\"state\":null,\"ts\":{now}}}}}", entry("\"MERGED\"", 6), entry("\"OPEN\"", 7), entry("\"MERGED\"", 8)),
    )
    .unwrap();
    w
}

#[test]
fn the_cleanup_dialog_and_job_match_ruby() {
    let base = tempfile::tempdir().unwrap();
    let (base_a, base_b) = (base.path().join("a"), base.path().join("b"));
    let (wa, wb) = (cleanup_world(&base_a), cleanup_world(&base_b));
    let env_for = |w: &common::AttentionWorld, tag: &str| -> Vec<(&'static str, String)> {
        vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("PATH", format!("{}:{}", w.bin.display(), std::env::var("PATH").unwrap())), ("FAKE_GH_DATA", w.gh_data.display().to_string()), ("FAKE_GH_LOG", base.path().join(format!("gh-{tag}.log")).display().to_string()), ("QM_TMUX_SOCKET", format!("qm-test-clean-{tag}"))]
    };
    let ruby = start_env(false, &wa.root, &wa.home, &env_for(&wa, "ruby"));
    let rust = start_env(true, &wb.root, &wb.home, &env_for(&wb, "rust"));
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let sha = regex::Regex::new(r"\b[0-9a-f]{7,40}\b").unwrap();
    let norm = |text: String, side: usize| {
        let text = text.replace(&(if side == 0 { &base_a } else { &base_b }).display().to_string(), "BASE");
        sha.replace_all(&text, "SHA").into_owned()
    };
    let post = |server: &Server, path: &str, body: &str| String::from_utf8_lossy(&request(server.port, "POST", path, &origin, body, true).body).into_owned();
    let state = |server: &Server| -> serde_json::Value { serde_json::from_slice(&request(server.port, "GET", "/api/state", &[], "", true).body).expect("a state") };
    // the plan the dialog shows, and the ids in it (they carry each world's paths)
    let plan = |scope: &str| -> (String, String, serde_json::Value, serde_json::Value) {
        let body = format!("{{\"scope\":\"{scope}\"}}");
        let (a, b) = (post(&ruby, "/api/cleanup-plan", &body), post(&rust, "/api/cleanup-plan", &body));
        assert_eq!(norm(a.clone(), 0), norm(b.clone(), 1), "plan for {scope}");
        (norm(a.clone(), 0), scope.to_string(), serde_json::from_str(&a).unwrap(), serde_json::from_str(&b).unwrap())
    };
    let ids = |v: &serde_json::Value, suffix: &str| -> Vec<String> { v["plan"]["entries"].as_array().map(|e| e.iter().filter_map(|x| x["id"].as_str()).filter(|i| i.ends_with(suffix)).map(String::from).collect()).unwrap_or_default() };
    let (mut got_ruby, mut got_rust): (Vec<String>, Vec<String>) = (vec![], vec![]);
    let mut run = |label: &str, scope: &str, select: &[&str], force: &[&str]| {
        let (_, _, pa, pb) = plan(scope);
        let body = |p: &serde_json::Value| {
            let pick = |suffixes: &[&str]| suffixes.iter().flat_map(|s| match s.strip_prefix('=') { Some(literal) => vec![literal.to_string()], None => ids(p, s) }).map(serde_json::Value::String).collect::<Vec<_>>();
            serde_json::json!({"cmd": "cleanup", "slug": null, "args": {"scope": scope, "selected": pick(select), "force": pick(force)}}).to_string()
        };
        let (a, b) = (post(&ruby, "/api/action", &body(&pa)), post(&rust, "/api/action", &body(&pb)));
        assert_eq!(norm(a.clone(), 0), norm(b.clone(), 1), "{label}: reply");
        if a.contains("background") {
            for _ in 0..300 {
                let (sa, sb) = (state(&ruby), state(&rust));
                got_ruby.extend(sa["finished"].as_array().unwrap().iter().map(|v| norm(v.to_string(), 0)));
                got_rust.extend(sb["finished"].as_array().unwrap().iter().map(|v| norm(v.to_string(), 1)));
                if sa["jobs"].as_array().is_some_and(|j| j.is_empty()) && sb["jobs"].as_array().is_some_and(|j| j.is_empty()) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        assert_eq!(got_ruby, got_rust, "{label}: what the job reported");
        let left = |w: &common::AttentionWorld, side: usize| {
            let mut tree = String::new();
            tree_text(&w.root.join("issues"), "issues", &mut tree);
            let tree = regex::Regex::new(r#""seen_at":[0-9.e+-]+"#).unwrap().replace_all(&tree, "\"seen_at\":T").into_owned();
            norm(format!("{}\n{}\n{tree}", git(&w.repo, &["worktree", "list", "--porcelain"]), git(&w.repo, &["branch", "--list", "--format=%(refname:short)"])), side)
        };
        assert_eq!(left(&wa, 0), left(&wb, 1), "{label}: what is left");
    };
    run("plan only: nothing selected", "res-merged", &[], &[]);
    run("a warned entry without force", "res-merged", &["wt-merged", "#merged-b"], &[]);
    run("forced", "res-merged", &[], &["wt-merged", "#merged-b"]);
    run("dirty, unforced", "res-dirty", &["wt-dirty"], &[]);
    run("dirty, forced", "res-dirty", &[], &["wt-dirty"]);
    run("unpreserved commits, forced", "res-unpushed", &[], &["wt-unpushed"]);
    run("a merged spare branch", "res-spare", &["#spare-b"], &[]);
    run("issue drafts", "res-issues", &["01-made.md"], &[]);
    run("an uncreated issue draft, forced", "res-issues", &[], &["02-open.md"]);
    run("nothing matches", "res-nothing", &["=worktree:/nowhere"], &[]);
    run("an item that is not there", "nowhere", &["x"], &[]);
    run("the sweep, nothing selected", "sweep", &[], &[]);
    let seen = got_rust.join("\n");
    for needle in ["needs force", "\"result\":\"removed\"", "cleaning up", "\"ok\":true", "nothing to do"] {
        assert!(seen.contains(needle), "the world should reach {needle}: {seen}");
    }
    let a = plan("sweep");
    assert!(a.0.contains("\"ok\":true"));
    let missing = plan("nowhere");
    assert!(missing.0.contains("no work item"), "{}", missing.0);
}

fn executable(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A repo with an origin, branches in every state "work on this item" meets (local without a worktree,
/// with one, a slash in the name, only on origin, a gh-stack whose top has none), and fake tools that
/// log what they are asked: the dev server, the test-DB command, the IDE and `open` for Warp.
fn workspace_world(base: &Path, live: bool) -> common::AttentionWorld {
    let mut w = attention_world(base, std::time::SystemTime::now(), None);
    if live {
        // the live data root of a sandbox home, so the dev server is not fenced off
        let root = w.home.join(".claude/quartermaster");
        fs::create_dir_all(root.parent().unwrap()).unwrap();
        fs::rename(&w.root, &root).unwrap();
        w.root = root;
    }
    let repo = &w.repo;
    let origin = base.join("origin.git");
    fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "-q", "--bare", "-b", "main"]);
    git(repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(repo, &["push", "-q", "origin", "main"]);
    git(repo, &["push", "-q", "origin", "HEAD:refs/heads/origin-only"]);
    for b in ["feat-a", "feat/slash", "feat-w", "st-1", "st-2", "fresh-b"] {
        git(repo, &["branch", b]);
    }
    git(repo, &["worktree", "add", base.join("wt-w").to_str().unwrap(), "feat-w"]);
    git(repo, &["worktree", "add", base.join("wt-st1").to_str().unwrap(), "st-1"]);
    // st-1 moves after st-2 was built on it: st-2 needs a restack
    fs::write(base.join("wt-st1/moved.txt"), "moved\n").unwrap();
    git(&base.join("wt-st1"), &["add", "."]);
    git(&base.join("wt-st1"), &["commit", "-q", "-m", "st-1 moved"]);
    fs::write(repo.join(".git/gh-stack"), serde_json::json!({"stacks": [{"trunk": {"branch": "main"}, "branches": [{"branch": "st-1"}, {"branch": "st-2"}]}]}).to_string()).unwrap();
    let tools = base.join("tools");
    let log = "\"${FAKE_TOOL_LOG}\"";
    executable(&tools.join("devsrv"), &format!("#!/bin/sh\necho \"devsrv $*\" >> {log}\ncase \"$1\" in status) exit 1;; esac\n[ -f \"$FAKE_FAIL/devsrv\" ] && {{ echo \"devsrv exploded\" >&2; exit 3; }}\nexit 0\n"));
    executable(&tools.join("tdb"), &format!("#!/bin/sh\necho \"tdb $* @ $(pwd)\" >> {log}\n[ -f \"$FAKE_FAIL/tdb\" ] && {{ echo \"no database\" >&2; exit 4; }}\nexit 0\n"));
    executable(&tools.join("fake-ide"), &format!("#!/bin/sh\necho \"ide $*\" >> {log}\n[ -f \"$FAKE_FAIL/ide\" ] && {{ echo \"ide refused\" >&2; exit 5; }}\nexit 0\n"));
    executable(&tools.join("open"), &format!("#!/bin/sh\necho \"open $*\" >> {log}\n[ -f \"$FAKE_FAIL/open\" ] && exit 1\nexit 0\n"));
    fs::write(
        w.root.join("config.yml"),
        format!("ide_command: [{ide}, PATH]\nide_label: FakeIDE\nprojects:\n  demo:\n    root: {repo}\n    test_db: {tdb}\n    dev_server:\n      command: {dev}\n      modes: [dev, tunnel]\n      default_mode: dev\n  plain: {repo}\n", ide = tools.join("fake-ide").display(), repo = repo.display(), tdb = tools.join("tdb").display(), dev = tools.join("devsrv").display()),
    )
    .unwrap();
    let item = |slug: &str, project: &str, extra: &str| {
        fs::write(w.root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\ntitle: {slug}\nstate: in_progress\nproject: {project}\ncreated: '2026-01-01'\n{extra}---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n")).unwrap();
    };
    item("w-exist", "demo", "branches:\n- feat-w\n");
    item("w-feat", "demo", "branches:\n- feat-a\n");
    item("w-slash", "demo", "branches:\n- feat/slash\n");
    item("w-origin", "demo", "branches:\n- origin-only\n");
    item("w-stack", "demo", "branches:\n- st-1\n");
    item("w-main", "demo", "branches:\n- main\n");
    item("w-two", "demo", "branches:\n- feat-w\n- feat-a\n");
    item("w-plain", "plain", "branches:\n- feat-w\n");
    item("w-fresh", "demo", "branches:\n- fresh-b\n");
    item("w-none", "nowhere", "");
    w
}

#[test]
fn the_workspace_jobs_match_ruby() {
    let base = tempfile::tempdir().unwrap();
    let (base_a, base_b) = (base.path().join("a"), base.path().join("b"));
    let (wa, wb) = (workspace_world(&base_a, true), workspace_world(&base_b, true));
    let fail_dirs = [base_a.join("fail"), base_b.join("fail")];
    let logs = [base_a.join("tool.log"), base_b.join("tool.log")];
    let env_for = |w: &common::AttentionWorld, side: usize| -> Vec<(&'static str, String)> {
        let base = if side == 0 { &base_a } else { &base_b };
        vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("PATH", format!("{}:{}:{}", base.join("tools").display(), w.bin.display(), std::env::var("PATH").unwrap())), ("FAKE_FAIL", fail_dirs[side].display().to_string()), ("FAKE_TOOL_LOG", logs[side].display().to_string()), ("QM_WARP_TAB_CONFIGS", base.join("warp").display().to_string()), ("QM_TMUX_SOCKET", format!("qm-test-ws-{side}"))]
    };
    for d in &fail_dirs {
        fs::create_dir_all(d).unwrap();
    }
    let ruby = start_env(false, &wa.root, &wa.home, &env_for(&wa, 0));
    let rust = start_env(true, &wb.root, &wb.home, &env_for(&wb, 1));
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let norm = |text: String, side: usize| text.replace(&(if side == 0 { &base_a } else { &base_b }).display().to_string(), "BASE");
    let post = |server: &Server, body: &str| String::from_utf8_lossy(&request(server.port, "POST", "/api/action", &origin, body, true).body).into_owned();
    let state = |server: &Server| -> serde_json::Value { serde_json::from_slice(&request(server.port, "GET", "/api/state", &[], "", true).body).expect("a state") };
    let (mut got_ruby, mut got_rust): (Vec<String>, Vec<String>) = (vec![], vec![]);
    let act = |cmd: &str, slug: &str, args: &str| format!("{{\"cmd\":\"{cmd}\",\"slug\":\"{slug}\",\"args\":{args}}}");
    let mut run = |label: &str, body: String| {
        let (a, b) = (post(&ruby, &body), post(&rust, &body));
        assert_eq!(norm(a.clone(), 0), norm(b.clone(), 1), "{label}: reply to {body}");
        if a.contains("background") {
            for _ in 0..300 {
                let (sa, sb) = (state(&ruby), state(&rust));
                got_ruby.extend(sa["finished"].as_array().unwrap().iter().map(|v| norm(v.to_string(), 0)));
                got_rust.extend(sb["finished"].as_array().unwrap().iter().map(|v| norm(v.to_string(), 1)));
                if sa["jobs"].as_array().is_some_and(|j| j.is_empty()) && sb["jobs"].as_array().is_some_and(|j| j.is_empty()) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        for (i, (x, y)) in got_ruby.iter().zip(got_rust.iter()).enumerate() {
            assert_eq!(x, y, "{label}: job report #{i}");
        }
        assert_eq!(got_ruby.len(), got_rust.len(), "{label}: how many jobs reported");
        let left = |w: &common::AttentionWorld, side: usize| {
            let base = if side == 0 { &base_a } else { &base_b };
            // the state builds the test polls with ask the dev server for its status whenever they run
            let tools: String = fs::read_to_string(&logs[side]).unwrap_or_default().lines().filter(|l| !l.starts_with("devsrv status")).map(|l| format!("{l}\n")).collect();
            let warp = fs::read_to_string(base.join("warp/qm-launch.toml")).unwrap_or_default();
            let mut items = String::new();
            tree_text(&w.root.join("work-items"), "items", &mut items);
            norm(format!("{}\n{}\n== tools\n{tools}== warp\n{warp}== items\n{items}", git(&w.repo, &["worktree", "list", "--porcelain"]), git(&w.repo, &["branch", "--list", "--format=%(refname:short) %(upstream:short)"])), side)
        };
        let (la, lb) = (left(&wa, 0), left(&wb, 1));
        let sha = regex::Regex::new(r"\bHEAD [0-9a-f]{40}\b").unwrap();
        assert_eq!(sha.replace_all(&la, "HEAD SHA"), sha.replace_all(&lb, "HEAD SHA"), "{label}: what is left");
    };
    let fail = |tool: &str, on: bool| {
        for d in &fail_dirs {
            if on {
                fs::write(d.join(tool), "").unwrap();
            } else {
                let _ = fs::remove_file(d.join(tool));
            }
        }
    };
    run("work on an item with a worktree", act("work-on", "w-exist", "{}"));
    run("work on, a worktree to create", act("work-on", "w-feat", "{}"));
    run("work on again", act("work-on", "w-feat", "{}"));
    run("switch the dev server, a slash branch", act("pf-switch", "w-slash", r#"{"mode":"tunnel"}"#));
    run("switch to a mode that is not there", act("pf-switch", "w-two", r#"{"mode":"bogus"}"#));
    run("switch to the item's default mode", act("pf-switch", "w-two", "{}"));
    run("open the IDE on a branch only origin has", act("ide", "w-origin", "{}"));
    run("a terminal on a stack whose top has no worktree", act("terminal", "w-stack", "{}"));
    run("work on a stack's existing worktree", act("work-on", "w-stack", r#"{"branch":"st-1"}"#));
    run("work on the main checkout", act("work-on", "w-main", "{}"));
    run("an explicit worktree", act("ide", "w-exist", &format!("{{\"worktree\":\"{}/wt-w\"}}", "BASEPLACEHOLDER")).replace("BASEPLACEHOLDER", "/nonexistent"));
    run("a branch that is not linked", act("work-on", "w-exist", r#"{"branch":"nope"}"#));
    run("a project without a dev server", act("work-on", "w-plain", "{}"));
    run("switching where there is no dev server", act("pf-switch", "w-plain", "{}"));
    run("an item without a project root", act("work-on", "w-none", "{}"));
    run("a missing item", act("ide", "nowhere", "{}"));
    fail("devsrv", true);
    run("the dev server fails", act("pf-switch", "w-exist", "{}"));
    run("the dev server fails after a new worktree", act("work-on", "w-slash", "{}"));
    fail("devsrv", false);
    fail("ide", true);
    run("the IDE refuses", act("ide", "w-exist", "{}"));
    fail("ide", false);
    fail("tdb", true);
    run("the test database fails", act("work-on", "w-fresh", "{}"));
    fail("tdb", false);
    fail("open", true);
    run("Warp is not there", act("terminal", "w-exist", "{}"));
    fail("open", false);
    run("stop the dev server", act("pf-stop", "", "{}"));
    fail("devsrv", true);
    run("the dev server will not stop", act("pf-stop", "", "{}"));
    fail("devsrv", false);
    let seen = got_rust.join("\n");
    for needle in ["devsrv tunnel @", "created worktree", "FakeIDE opened", "Opened Warp tab", "mode must be one of", "devsrv exploded", "ide refused", "no database", "could not open a Warp tab", "devsrv stopped", "tdb prepare", "worktrees/origin-only for origin-only"] {
        let found = seen.contains(needle) || fs::read_to_string(&logs[1]).unwrap_or_default().contains(needle);
        assert!(found, "the world should reach {needle}: {seen}");
    }
    tmux_on("qm-test-ws-0", &["kill-server"]);
    tmux_on("qm-test-ws-1", &["kill-server"]);
}

#[test]
fn a_gui_on_a_throwaway_root_is_fenced_off_the_dev_server_and_runs_updates() {
    let base = tempfile::tempdir().unwrap();
    let (base_a, base_b) = (base.path().join("a"), base.path().join("b"));
    let (wa, wb) = (workspace_world(&base_a, false), workspace_world(&base_b, false));
    for b in [&base_a, &base_b] {
        executable(&b.join("proj/bin/update"), "#!/bin/sh\necho updating\n");
        fs::create_dir_all(b.join("proj-plain")).unwrap();
        fs::create_dir_all(b.join("fail")).unwrap();
    }
    let env_for = |w: &common::AttentionWorld, side: usize| -> Vec<(&'static str, String)> {
        let base = if side == 0 { &base_a } else { &base_b };
        vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("PATH", format!("{}:{}:{}", base.join("tools").display(), w.bin.display(), std::env::var("PATH").unwrap())), ("FAKE_FAIL", base.join("fail").display().to_string()), ("FAKE_TOOL_LOG", base.join("tool.log").display().to_string()), ("QM_WARP_TAB_CONFIGS", base.join("warp").display().to_string()), ("QM_TMUX_SOCKET", format!("qm-test-fence-{side}"))]
    };
    for side in 0..2 {
        tmux_on(&format!("qm-test-fence-{side}"), &["kill-server"]);
    }
    let ruby = start_env(false, &wa.root, &wa.home, &env_for(&wa, 0));
    let rust = start_env(true, &wb.root, &wb.home, &env_for(&wb, 1));
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let norm = |text: String, side: usize| text.replace(&(if side == 0 { &base_a } else { &base_b }).display().to_string(), "BASE");
    let post = |server: &Server, body: &str| String::from_utf8_lossy(&request(server.port, "POST", "/api/action", &origin, body, true).body).into_owned();
    let state = |server: &Server| -> serde_json::Value { serde_json::from_slice(&request(server.port, "GET", "/api/state", &[], "", true).body).expect("a state") };
    let act = |cmd: &str, slug: &str, args: &str| format!("{{\"cmd\":\"{cmd}\",\"slug\":\"{slug}\",\"args\":{args}}}");
    let mut finished = |label: &str| {
        let (mut a, mut b): (Vec<String>, Vec<String>) = (vec![], vec![]);
        for _ in 0..200 {
            let (sa, sb) = (state(&ruby), state(&rust));
            a.extend(sa["finished"].as_array().unwrap().iter().map(|v| norm(v.to_string(), 0)));
            b.extend(sb["finished"].as_array().unwrap().iter().map(|v| norm(v.to_string(), 1)));
            if sa["jobs"].as_array().is_some_and(|j| j.is_empty()) && sb["jobs"].as_array().is_some_and(|j| j.is_empty()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(a, b, "{label}");
        a.join("\n")
    };
    let mut seen = String::new();
    for (label, body) in [("switch", act("pf-switch", "w-exist", "{}")), ("work on", act("work-on", "w-exist", "{}")), ("stop", act("pf-stop", "", "{}")), ("a worktree to create first", act("work-on", "w-feat", "{}"))] {
        let (a, b) = (post(&ruby, &body), post(&rust, &body));
        assert_eq!(norm(a, 0), norm(b, 1), "{label}");
        seen += &finished(label);
    }
    assert!(seen.matches("the dev server is off in this GUI: BASE/root is not the live qm data root").count() >= 4 && seen.contains("created worktree"), "{seen}");
    // the fence reaches the page's dev-server badge, and nothing reached the dev server but status asks
    let (sa, sb) = (state(&ruby), state(&rust));
    assert_eq!(norm(sa["pf"].to_string(), 0), norm(sb["pf"].to_string(), 1));
    assert!(sa["pf"]["off"].is_string());
    for base in [&base_a, &base_b] {
        let log = fs::read_to_string(base.join("tool.log")).unwrap_or_default();
        assert!(log.lines().all(|l| l.starts_with("devsrv status") || !l.starts_with("devsrv")), "{log}");
    }
    // bin/update runs in a tmux session of its own
    for (label, args) in [("update", format!("{{\"path\":\"{}/proj\"}}", "BASEX")), ("again", format!("{{\"path\":\"{}/proj\"}}", "BASEX")), ("no bin/update", format!("{{\"path\":\"{}/proj-plain\"}}", "BASEX")), ("no such path", "{\"path\":\"/nonexistent/x\"}".to_string())] {
        let (a, b) = (post(&ruby, &act("run-update", "", &args.replace("BASEX", &base_a.display().to_string()))), post(&rust, &act("run-update", "", &args.replace("BASEX", &base_b.display().to_string()))));
        assert_eq!(norm(a, 0), norm(b, 1), "{label}");
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    let panes = |side: usize| tmux_on(&format!("qm-test-fence-{side}"), &["list-sessions", "-F", "#{session_name}"]);
    assert_eq!(panes(0), panes(1));
    assert!(panes(1).contains("bin-update-proj"), "{}", panes(1));
    for side in 0..2 {
        tmux_on(&format!("qm-test-fence-{side}"), &["kill-server"]);
    }
}

#[test]
fn adopt_and_restack_start_or_type_into_chats_like_ruby() {
    let base = tempfile::tempdir().unwrap();
    let (base_a, base_b) = (base.path().join("a"), base.path().join("b"));
    let (wa, wb) = (workspace_world(&base_a, false), workspace_world(&base_b, false));
    for w in [&wa, &wb] {
        for slug in ["w-two", "w-exist", "w-feat", "w-stack"] {
            let p = w.root.join(format!("work-items/{slug}.md"));
            let text = fs::read_to_string(&p).unwrap().replacen("---\n\n## Description", &format!("sessions:\n- id: sess-{slug}\n---\n\n## Description"), 1);
            fs::write(&p, text).unwrap();
        }
        fs::write(w.root.join("work-items/w-stack2.md"), "---\nv: 1\ntitle: w-stack2\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\nbranches:\n- st-1\nsessions:\n- id: sess-w-stack2\n---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n").unwrap();
        // a chat's transcript with a background agent still out
        let transcript = w.transcripts.join("p/sess-w-stack2.jsonl");
        fs::create_dir_all(transcript.parent().unwrap()).unwrap();
        fs::write(&transcript, format!("{}\n", serde_json::json!({"type": "user", "timestamp": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true), "toolUseResult": {"status": "async_launched", "agentId": "bg-1", "description": "the long job"}}))).unwrap();
    }
    let chat = base.path().join("fake-chat");
    executable(&chat, "#!/bin/sh\nprintf '{\"v\":1,\"event\":\"start\",\"session_id\":\"sess-%s\",\"cwd\":\"%s\",\"pid\":%s,\"transcript_path\":\"%s/sess-%s.jsonl\",\"ts\":\"%s\"}\\n' \"$QM_ITEM\" \"$(pwd)\" \"$$\" \"$QM_TRANSCRIPTS_DIR/p\" \"$QM_ITEM\" \"$(date -u +%FT%TZ)\" >> \"$QM_ROOT/sessions.jsonl\"\necho \"started in $(pwd) with: $*\" > \"$FAKE_REC/start-$QM_ITEM\"\nwhile read -r line; do echo \"$line\" >> \"$FAKE_REC/typed-$QM_ITEM\"; done\n");
    let recs = [base_a.join("rec"), base_b.join("rec")];
    for r in &recs {
        fs::create_dir_all(r).unwrap();
    }
    let env_for = |w: &common::AttentionWorld, side: usize| -> Vec<(&'static str, String)> {
        let base = if side == 0 { &base_a } else { &base_b };
        vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("PATH", format!("{}:{}", w.bin.display(), std::env::var("PATH").unwrap())), ("QM_TMUX_SOCKET", format!("qm-test-adopt-{side}")), ("QM_CHAT_COMMAND", chat.display().to_string()), ("FAKE_REC", recs[side].display().to_string()), ("QM_ROOT", w.root.display().to_string()), ("FAKE_BASE", base.display().to_string())]
    };
    for side in 0..2 {
        tmux_on(&format!("qm-test-adopt-{side}"), &["kill-server"]);
    }
    let ruby = start_env(false, &wa.root, &wa.home, &env_for(&wa, 0));
    let rust = start_env(true, &wb.root, &wb.home, &env_for(&wb, 1));
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let norm = |text: String, side: usize| text.replace(&(if side == 0 { &base_a } else { &base_b }).display().to_string(), "BASE");
    let post = |server: &Server, body: &str| String::from_utf8_lossy(&request(server.port, "POST", "/api/action", &origin, body, true).body).into_owned();
    let act = |cmd: &str, slug: &str, args: &str| format!("{{\"cmd\":\"{cmd}\",\"slug\":\"{slug}\",\"args\":{args}}}");
    let mut step = |label: &str, make: &dyn Fn(&Path) -> String, wait_for: Option<&str>| -> String {
        let (a, b) = (post(&ruby, &make(&base_a)), post(&rust, &make(&base_b)));
        assert_eq!(norm(a.clone(), 0), norm(b.clone(), 1), "{label}: reply");
        if let Some(file) = wait_for {
            for _ in 0..100 {
                if recs[0].join(file).exists() && recs[1].join(file).exists() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            std::thread::sleep(std::time::Duration::from_millis(700));
        }
        let record = |side: usize| {
            let mut all: Vec<String> = fs::read_dir(&recs[side]).unwrap().map(|e| {
                let p = e.unwrap().path();
                format!("{}: {}", p.file_name().unwrap().to_string_lossy(), fs::read_to_string(&p).unwrap_or_default())
            }).collect();
            all.sort();
            norm(all.join(""), side)
        };
        // what a chat writes lands a moment after the action answers
        for _ in 0..60 {
            if record(0) == record(1) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(record(0), record(1), "{label}: what the chats saw");
        let panes = |side: usize| tmux_on(&format!("qm-test-adopt-{side}"), &["list-sessions", "-F", "#{session_name}"]);
        assert_eq!(panes(0), panes(1), "{label}: tmux");
        norm(a, 0)
    };
    let wt = |base: &Path| format!("{}/wt-w", base.display());
    step("adopt starts a chat", &|b| act("adopt", "w-exist", &format!("{{\"worktree\":\"{}\"}}", wt(b))), Some("start-w-exist"));
    step("adopt the main checkout", &|b| act("adopt", "w-main", &format!("{{\"worktree\":\"{}/repo\"}}", b.display())), None);
    step("adopt what is no worktree", &|b| act("adopt", "w-main", &format!("{{\"worktree\":\"{}\"}}", b.join("tools").display())), None);
    step("adopt without a root", &|b| act("adopt", "w-none", &format!("{{\"worktree\":\"{}\"}}", wt(b))), None);
    step("adopt for a missing item", &|b| act("adopt", "nowhere", &format!("{{\"worktree\":\"{}\"}}", wt(b))), None);
    step("restack starts a chat", &|_| act("restack", "w-stack", "{}"), Some("start-w-stack"));
    step("restack names a branch", &|_| act("restack", "w-stack", r#"{"branch":"st-2"}"#), None);
    step("restack a branch in no stack", &|_| act("restack", "w-stack", r#"{"branch":"zzz"}"#), None);
    step("restack without a stack", &|_| act("restack", "w-main", "{}"), None);
    // a chat that is running: typed into once it is the user's turn
    step("open a chat", &|_| act("pane-start", "w-two", "{}"), Some("start-w-two"));
    let events = |side: usize, text: &str| {
        let root = if side == 0 { &wa.root } else { &wb.root };
        let mut f = fs::OpenOptions::new().append(true).create(true).open(root.join("events.jsonl")).unwrap();
        use std::io::Write;
        f.write_all(text.as_bytes()).unwrap();
    };
    let event = |name: &str, session: &str| format!("{{\"v\":1,\"ts\":\"{}\",\"event\":\"{name}\",\"session_id\":\"{session}\"}}\n", chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    for side in 0..2 {
        events(side, &event("UserPromptSubmit", "sess-w-two"));
    }
    step("restack while the chat works", &|_| act("restack", "w-two", "{}"), None);
    step("adopt while the chat works", &|b| act("adopt", "w-two", &format!("{{\"worktree\":\"{}\"}}", wt(b))), None);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    for side in 0..2 {
        events(side, &event("Stop", "sess-w-two"));
    }
    step("adopt types into the chat", &|b| act("adopt", "w-two", &format!("{{\"worktree\":\"{}\"}}", wt(b))), Some("typed-w-two"));
    let typed = fs::read_to_string(recs[1].join("typed-w-two")).unwrap_or_default();
    assert!(typed.contains("/adopt-worktree "), "{typed}");
    for side in 0..2 {
        events(side, &event("Stop", "sess-w-stack"));
    }
    step("restack types into the chat", &|_| act("restack", "w-stack", "{}"), Some("typed-w-stack"));
    let typed = fs::read_to_string(recs[1].join("typed-w-stack")).unwrap_or_default();
    assert!(typed.contains("Restack the gh-stack main ← st-1 ← st-2") && typed.contains("behind their parent: st-2"), "{typed}");
    // its report would land on top of the prompt while background work is out
    step("open a chat with work out", &|_| act("pane-start", "w-stack2", "{}"), Some("start-w-stack2"));
    for side in 0..2 {
        events(side, &event("Stop", "sess-w-stack2"));
    }
    let waiting = step("restack while work is out", &|_| act("restack", "w-stack2", "{}"), None);
    assert!(waiting.contains("waiting on background work (the long job)"), "{waiting}");
    step("adopt while work is out", &|b| act("adopt", "w-stack2", &format!("{{\"worktree\":\"{}\"}}", wt(b))), Some("typed-w-stack2"));
    // a chat that waits on the user's permission is mid-turn too
    for side in 0..2 {
        events(side, &event("PermissionRequest", "sess-w-exist"));
    }
    step("adopt while the chat is blocked", &|b| act("adopt", "w-exist", &format!("{{\"worktree\":\"{}\"}}", wt(b))), None);
    for side in 0..2 {
        tmux_on(&format!("qm-test-adopt-{side}"), &["kill-server"]);
    }
}

#[test]
fn caffeinate_answers_as_ruby_does_without_the_program() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let env: Vec<(&str, String)> = vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string())];
    let root2 = base.path().join("root2");
    copy_dir(&w.root, &root2);
    let ruby = start_env(false, &w.root, &w.home, &env);
    let rust = start_env(true, &root2, &w.home, &env);
    let local = [("Content-Type", "application/json".to_string())];
    let state = |s: &Server| -> serde_json::Value { serde_json::from_slice(&request(s.port, "GET", "/api/state", &[], "", true).body).unwrap() };
    let log = |root: &Path| fs::read_to_string(root.join("qm.log")).unwrap_or_default().lines().map(|l| l.splitn(2, ' ').nth(1).unwrap_or("").to_string()).collect::<Vec<_>>();
    for (label, args) in [("status", r#"{"mode":"status"}"#), ("no mode", "{}"), ("off", r#"{"mode":"off"}"#), ("a bad mode", r#"{"mode":"sometimes"}"#), ("a number", r#"{"mode":5}"#), ("null", r#"{"mode":null}"#), ("auto with nothing busy", r#"{"mode":"auto"}"#), ("on without the program", r#"{"mode":"on"}"#), ("status after", r#"{"mode":"status"}"#), ("auto again", r#"{"mode":"auto"}"#), ("off again", r#"{"mode":"off"}"#)] {
        let body = format!("{{\"cmd\":\"caffeinate\",\"slug\":null,\"args\":{args}}}");
        let (a, b) = (request(ruby.port, "POST", "/api/action", &local, &body, true), request(rust.port, "POST", "/api/action", &local, &body, true));
        assert_eq!(shown(&a, ruby.port), shown(&b, rust.port), "{label}");
        assert_eq!(state(&ruby)["caffeinate"], state(&rust)["caffeinate"], "{label}: the state's caffeinate");
        assert_eq!(log(&w.root), log(&root2), "{label}: qm.log");
    }
    assert!(log(&root2).iter().any(|l| l.contains("No such file or directory - /usr/bin/caffeinate")), "{:?}", log(&root2));
}

#[test]
fn caffeinate_holds_one_child_of_the_gui_and_ends_it() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let fake = base.path().join("fake-caffeinate");
    executable(&fake, &format!("#!/bin/sh\necho \"$*\" > \"{}/args\"\nexec sleep 300\n", base.path().display()));
    let env: Vec<(&str, String)> = vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("QM_CAFFEINATE_BINARY", fake.display().to_string())];
    let rust = start_env(true, &w.root, &w.home, &env);
    let local = [("Content-Type", "application/json".to_string())];
    let mode = |m: &str| -> serde_json::Value { serde_json::from_slice(&request(rust.port, "POST", "/api/action", &local, &format!("{{\"cmd\":\"caffeinate\",\"slug\":null,\"args\":{{\"mode\":\"{m}\"}}}}"), true).body).unwrap() };
    let alive = |pid: i64| Command::new("kill").args(["-0", &pid.to_string()]).output().unwrap().status.success() && !fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default().contains(") Z");
    let first = mode("on");
    let pid = first["caffeinate"]["pid"].as_i64().expect("a pid");
    assert_eq!(first["caffeinate"]["mode"], "on");
    assert!(alive(pid));
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(fs::read_to_string(base.path().join("args")).unwrap().trim(), format!("-i -w {}", rust.child.id()));
    assert_eq!(mode("on")["caffeinate"]["pid"].as_i64(), Some(pid), "a second on keeps the child");
    let state: serde_json::Value = serde_json::from_slice(&request(rust.port, "GET", "/api/state", &[], "", true).body).unwrap();
    assert_eq!(state["caffeinate"]["pid"].as_i64(), Some(pid));
    assert!(state["pulse"].as_str().unwrap().split(':').any(|p| p == "1"));
    let off = mode("off");
    assert_eq!((off["caffeinate"]["on"].as_bool(), off["caffeinate"]["mode"].as_str()), (Some(false), Some("off")));
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(!alive(pid), "off ends the child");
    // a child killed from outside reads as off
    let pid = mode("on")["caffeinate"]["pid"].as_i64().unwrap();
    Command::new("kill").args(["-9", &pid.to_string()]).output().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert_eq!(mode("status")["caffeinate"]["on"], false);
    // auto holds only while a session is busy: nothing is, so it holds nothing
    let auto = mode("auto");
    assert_eq!((auto["caffeinate"]["on"].as_bool(), auto["caffeinate"]["mode"].as_str()), (Some(false), Some("auto")));
    mode("off");
}

#[test]
fn the_warp_tab_actions_match_ruby() {
    let base = tempfile::tempdir().unwrap();
    let (base_a, base_b) = (base.path().join("a"), base.path().join("b"));
    let (wa, wb) = (workspace_world(&base_a, false), workspace_world(&base_b, false));
    let me = std::process::id();
    let stamp = |secs: i64| (chrono::Utc::now() - chrono::Duration::seconds(secs)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    for (w, b) in [(&wa, &base_a), (&wb, &base_b)] {
        let work = b.join("work-a");
        fs::create_dir_all(&work).unwrap();
        let mut log = fs::read_to_string(w.root.join("sessions.jsonl")).unwrap_or_default();
        for (id, pid, transcript, age) in [("r-one", 999999u32, true, 500), ("r-old", 999999, true, 900), ("r-live", me, true, 200), ("r-gone", 999999, false, 300), ("r-loose-extra-long", 999999, true, 400)] {
            let path = w.transcripts.join(format!("p/{id}.jsonl"));
            if transcript {
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, "{}\n").unwrap();
            }
            log += &format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"{}\",\"pid\":{pid},\"transcript_path\":\"{}\",\"ts\":\"{}\"}}\n", work.display(), path.display(), stamp(age));
        }
        fs::write(w.root.join("sessions.jsonl"), log).unwrap();
        for (slug, sessions) in [("k-one", "- id: r-one\n- id: r-old\n"), ("k-live", "- id: r-live\n- id: r-one\n"), ("k-gone", "- id: r-gone\n"), ("k-empty", "")] {
            let list = if sessions.is_empty() { String::new() } else { format!("sessions:\n{sessions}") };
            fs::write(w.root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\ntitle: {slug}\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\n{list}---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n")).unwrap();
        }
    }
    let chat = base.path().join("fake-chat");
    executable(&chat, "#!/bin/sh\nexec sleep 300\n");
    let fail_dirs = [base_a.join("fail"), base_b.join("fail")];
    let logs = [base_a.join("tool.log"), base_b.join("tool.log")];
    for d in &fail_dirs {
        fs::create_dir_all(d).unwrap();
    }
    let env_for = |w: &common::AttentionWorld, side: usize| -> Vec<(&'static str, String)> {
        let base = if side == 0 { &base_a } else { &base_b };
        vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("PATH", format!("{}:{}:{}", base.join("tools").display(), w.bin.display(), std::env::var("PATH").unwrap())), ("FAKE_FAIL", fail_dirs[side].display().to_string()), ("FAKE_TOOL_LOG", logs[side].display().to_string()), ("QM_WARP_TAB_CONFIGS", base.join("warp").display().to_string()), ("QM_TMUX_SOCKET", format!("qm-test-warp-{side}")), ("QM_CHAT_COMMAND", chat.display().to_string())]
    };
    for side in 0..2 {
        tmux_on(&format!("qm-test-warp-{side}"), &["kill-server"]);
    }
    let ruby = start_env(false, &wa.root, &wa.home, &env_for(&wa, 0));
    let rust = start_env(true, &wb.root, &wb.home, &env_for(&wb, 1));
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let norm = |text: String, side: usize| text.replace(&(if side == 0 { &base_a } else { &base_b }).display().to_string(), "BASE").replace(&format!("qm-test-warp-{side}"), "SOCK");
    let post = |server: &Server, body: &str| request(server.port, "POST", "/api/action", &origin, body, true);
    let act = |cmd: &str, slug: &str, args: &str| format!("{{\"cmd\":\"{cmd}\",\"slug\":\"{slug}\",\"args\":{args}}}");
    let step = |label: &str, body: &str| -> String {
        let (a, b) = (post(&ruby, body), post(&rust, body));
        let (ta, tb) = (norm(String::from_utf8_lossy(&a.body).into_owned(), 0), norm(String::from_utf8_lossy(&b.body).into_owned(), 1));
        assert_eq!((a.status.as_str(), &ta), (b.status.as_str(), &tb), "{label}: reply to {body}");
        let left = |side: usize| {
            let base = if side == 0 { &base_a } else { &base_b };
            norm(format!("{}\n== tools\n{}", fs::read_to_string(base.join("warp/qm-launch.toml")).unwrap_or_default(), fs::read_to_string(&logs[side]).unwrap_or_default()), side)
        };
        assert_eq!(left(0), left(1), "{label}: the tab asked for");
        ta
    };
    step("start", &act("start", "k-empty", "{}"));
    step("start in a worktree", &act("start", "w-exist", "{}"));
    step("start for a missing item", &act("start", "nowhere", "{}"));
    step("resume", &act("resume", "k-one", "{}"));
    step("resume a chosen session", &act("resume", "k-one", r#"{"session":"r-old"}"#));
    step("resume an unknown session", &act("resume", "k-one", r#"{"session":"nope"}"#));
    step("resume with an empty session", &act("resume", "k-one", r#"{"session":""}"#));
    step("resume with a live session", &act("resume", "k-live", "{}"));
    step("resume rehydrates", &act("resume", "k-gone", "{}"));
    step("resume without sessions", &act("resume", "k-empty", "{}"));
    step("resume a missing item", &act("resume", "nowhere", "{}"));
    step("resume a session of nobody", &act("resume-session", "", r#"{"session":"r-loose-extra-long"}"#));
    step("resume-session, live", &act("resume-session", "", r#"{"session":"r-live"}"#));
    step("resume-session, pruned", &act("resume-session", "", r#"{"session":"r-gone"}"#));
    step("resume-session, unknown", &act("resume-session", "", r#"{"session":"zzz"}"#));
    step("a pane's Warp tab, no pane", &act("pane-warp", "k-empty", "{}"));
    step("open a pane", &act("pane-start", "k-empty", "{}"));
    let tab = step("a pane's Warp tab", &act("pane-warp", "k-empty", "{}"));
    assert!(tab.contains("\"ok\":true"), "{tab}");
    assert!(fs::read_to_string(base_b.join("warp/qm-launch.toml")).unwrap().contains("attach-session -t \\\\=k-empty"), "{}", fs::read_to_string(base_b.join("warp/qm-launch.toml")).unwrap());
    for d in &fail_dirs {
        fs::write(d.join("open"), "").unwrap();
    }
    let aborted = step("Warp is not there", &act("pane-warp", "k-empty", "{}"));
    assert!(aborted.contains("operation aborted"), "{aborted}");
    step("start without Warp", &act("start", "k-empty", "{}"));
    for side in 0..2 {
        tmux_on(&format!("qm-test-warp-{side}"), &["kill-server"]);
    }
    let a = post(&rust, &act("resume", "k-one", r#"{"session":5}"#));
    assert!(a.status.starts_with("501"), "{}", a.status);
}

#[test]
fn the_transcript_search_route_matches_ruby() {
    let base = tempfile::tempdir().unwrap();
    let (root, home, tr) = (base.path().join("root"), base.path().join("home"), base.path().join("transcripts"));
    fs::create_dir_all(root.join("work-items")).unwrap();
    fs::create_dir_all(&home).unwrap();
    let user = |text: &str, ts: &str| serde_json::json!({"type": "user", "timestamp": ts, "message": {"role": "user", "content": text}}).to_string();
    let assistant = |blocks: serde_json::Value, ts: &str| serde_json::json!({"type": "assistant", "timestamp": ts, "message": {"role": "assistant", "content": blocks}}).to_string();
    let write = |project: &str, id: &str, lines: &[String], age_mins: u64| {
        let path = tr.join(project).join(format!("{id}.jsonl"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, lines.join("\n") + "\n").unwrap();
        fs::OpenOptions::new().write(true).open(&path).unwrap().set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(age_mins * 60)).unwrap();
    };
    write("p1", "s-user", &[user("Please fix the Login bug in the form", "2026-03-01T10:00:00.000Z"), assistant(serde_json::json!([{"type": "text", "text": "I fixed the LOGIN handler"}]), "2026-03-01T10:01:00.000Z")], 10);
    write("p1", "s-tools", &[serde_json::json!({"type": "user", "timestamp": "2026-03-02T10:00:00.000Z", "toolUseResult": {"stdout": "login in tool output"}, "message": {"content": [{"type": "tool_result", "content": "login in tool output"}]}}).to_string(), serde_json::json!({"type": "user", "isSidechain": true, "message": {"content": "login in a sidechain"}}).to_string()], 20);
    write("p1", "s-many", &[user("login one", "2026-03-03T10:00:00.000Z"), assistant(serde_json::json!("login two as a string"), "2026-03-03T10:00:30.000Z")], 30);
    write("p2", "s-long", &[user(&format!("{}NEEDLE{}", "word ".repeat(40), " tail".repeat(60)), "2026-03-04T10:00:00.000Z")], 40);
    write("p2", "s-squash", &[user("first line\n\n\tsecond   line NEEDLE third\r\nfourth", "2026-03-05T10:00:00.000Z")], 50);
    write("p2", "s-unicode", &[user("héllo wörld 🙂 ünï — NEEDLE ünï 🙂 after", "2026-03-06T10:00:00.000Z")], 60);
    write("p2", "s-escapes", &[user("say \"quoted\" and back\\slash and tab\there", "2026-03-07T10:00:00.000Z")], 70);
    write("p2", "s-nots", &[serde_json::json!({"type": "user", "message": {"content": "needle without any timestamp"}}).to_string()], 80);
    write("p3", "s-aside", &[user("needle in an aside", "2026-03-08T10:00:00.000Z")], 90);
    write("p3", "s-junk", &["not json needle".to_string(), "[1,2,\"needle\"]".to_string(), user("needle after junk", "2026-03-09T10:00:00.000Z")], 100);
    write("p3/deeper", "s-subagent", &[user("needle in a sub-agent run", "2026-03-10T10:00:00.000Z")], 5);
    let start = |id: &str, ts: &str, extra: &str| format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"/srv/app\",\"pid\":999999,\"ts\":\"{ts}\"{extra}}}\n");
    let log = [start("s-user", "2026-03-01T09:59:00Z", ""), start("s-many", "2026-03-03T09:59:00Z", ""), start("s-long", "2026-03-04T09:59:00Z", ""), start("s-aside", "2026-03-08T09:59:00Z", ",\"aside\":\"alpha\""), start("s-junk", "2026-03-09T09:59:00Z", "")].concat();
    fs::write(root.join("sessions.jsonl"), log).unwrap();
    for (slug, title, state, sessions) in [("alpha", "Alpha work", "in_progress", "- id: s-user\n- id: s-many\n"), ("beta", "Beta work", "fridge", "- id: s-long\n")] {
        fs::write(root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\ntitle: {title}\nstate: {state}\nproject: demo\ncreated: '2026-01-01'\nsessions:\n{sessions}---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n")).unwrap();
    }
    let env: Vec<(&str, String)> = vec![("QM_TRANSCRIPTS_DIR", tr.display().to_string())];
    let ruby = start_env(false, &root, &home, &env);
    let rust = start_env(true, &root, &home, &env);
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let mut seen = String::new();
    for query in ["login", "NEEDLE", "needle", "héllo", "quoted", "back\\\\slash", "é", "nomatch", "", "   ", " login "] {
        let body = serde_json::json!({"query": query}).to_string();
        let (a, b) = (request(ruby.port, "POST", "/api/grep", &origin, &body, true), request(rust.port, "POST", "/api/grep", &origin, &body, true));
        assert_eq!(shown(&a, ruby.port), shown(&b, rust.port), "query {query:?}");
        seen += &String::from_utf8_lossy(&a.body);
    }
    for body in ["{}", "{\"query\":null}", "{\"query\":5}", "not json"] {
        let (a, b) = (request(ruby.port, "POST", "/api/grep", &origin, body, true), request(rust.port, "POST", "/api/grep", &origin, body, true));
        assert_eq!(shown(&a, ruby.port), shown(&b, rust.port), "body {body}");
    }
    for needle in ["\"slug\":\"alpha\"", "\"state\":\"fridge\"", "\"slug\":null", "\"transcript\":", "\"more\":0", "query is empty", "\"title\":\"unlinked\""] {
        assert!(seen.contains(needle), "the world should reach {needle}");
    }
}

#[test]
fn the_ask_route_matches_ruby() {
    let base = tempfile::tempdir().unwrap();
    let (root, home, bin, rec) = (base.path().join("root"), base.path().join("home"), base.path().join("bin"), base.path().join("rec"));
    fs::create_dir_all(root.join("work-items")).unwrap();
    for d in [&home, &rec] {
        fs::create_dir_all(d).unwrap();
    }
    for i in 0..12 {
        fs::write(root.join(format!("work-items/item-{i}.md")), format!("---\nv: 1\ntitle: Item {i}\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\n---\n\n## Description\n\nthing number {i}\n\n## Notes\n\n- 2026-02-01: note {i}\n\n## Snapshots\n")).unwrap();
    }
    executable(&bin.join("claude"), "#!/bin/sh\ncat > /dev/null\n[ -f \"$FAKE_CLAUDE_DIR/err\" ] && cat \"$FAKE_CLAUDE_DIR/err\" >&2\n[ -f \"$FAKE_CLAUDE_DIR/out\" ] && cat \"$FAKE_CLAUDE_DIR/out\"\n[ -f \"$FAKE_CLAUDE_DIR/fail\" ] && exit 3\nexit 0\n");
    let env: Vec<(&str, String)> = vec![("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap())), ("FAKE_CLAUDE_DIR", rec.display().to_string())];
    let ruby = start_env(false, &root, &home, &env);
    let rust = start_env(true, &root, &home, &env);
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let answer = |v: serde_json::Value| serde_json::json!({"type": "result", "structured_output": v}).to_string();
    let many: Vec<serde_json::Value> = (0..11).map(|i| serde_json::json!({"slug": format!("item-{i}"), "why": format!("reason {i}")})).collect();
    let cases: Vec<(&str, Option<String>, Option<&str>, bool, serde_json::Value)> = vec![
        ("ranked", Some(answer(serde_json::json!({"hits": [{"slug": "item-3", "why": "the one"}, {"slug": "ghost", "why": "invented"}, {"slug": "item-3", "why": "again"}, "junk", {"slug": 5, "why": "n"}, {"slug": "item-1"}, {"slug": "item-2", "why": 7}]}))), None, false, serde_json::json!({"query": "the one with the thing"})),
        ("more than eight", Some(answer(serde_json::json!({"hits": many}))), None, false, serde_json::json!({"query": "everything"})),
        ("nothing matches", Some(answer(serde_json::json!({"hits": []}))), None, false, serde_json::json!({"query": "nothing"})),
        ("stderr says why it failed", None, Some("rate limited\n"), true, serde_json::json!({"query": "x"})),
        ("stdout says why it failed", Some("quota exceeded\n".into()), None, true, serde_json::json!({"query": "x"})),
        ("an unparseable answer", Some("not json at all, and long enough to be cut".into()), None, false, serde_json::json!({"query": "x"})),
        ("no structured output", Some("{\"type\":\"result\"}".into()), None, false, serde_json::json!({"query": "x"})),
        ("hits that are no list", Some(answer(serde_json::json!({"hits": "none"}))), None, false, serde_json::json!({"query": "x"})),
        ("an empty query", Some(answer(serde_json::json!({"hits": []}))), None, false, serde_json::json!({"query": "   "})),
        ("no query", Some(answer(serde_json::json!({"hits": []}))), None, false, serde_json::json!({})),
        ("a numeric query", Some(answer(serde_json::json!({"hits": [{"slug": "item-0", "why": "w"}]}))), None, false, serde_json::json!({"query": 42})),
    ];
    let mut seen = String::new();
    for (label, out, err, fail, body) in cases {
        for f in ["out", "err", "fail"] {
            let _ = fs::remove_file(rec.join(f));
        }
        if let Some(o) = out {
            fs::write(rec.join("out"), o).unwrap();
        }
        if let Some(e) = err {
            fs::write(rec.join("err"), e).unwrap();
        }
        if fail {
            fs::write(rec.join("fail"), "").unwrap();
        }
        let text = body.to_string();
        let (a, b) = (request(ruby.port, "POST", "/api/ask", &origin, &text, true), request(rust.port, "POST", "/api/ask", &origin, &text, true));
        assert_eq!(shown(&a, ruby.port), shown(&b, rust.port), "{label}");
        seen += &String::from_utf8_lossy(&a.body);
    }
    for needle in ["\"slug\":\"item-3\"", "claude -p failed: rate limited", "claude -p failed: quota exceeded", "unparseable answer from claude", "unexpected answer from claude", "query is empty", "\"why\":\"7\"", "\"why\":\"\""] {
        assert!(seen.contains(needle), "the world should reach {needle}: {seen}");
    }
    assert!(!seen.contains("\"slug\":\"ghost\"") && !seen.contains("item-9"), "invented slugs and a ninth hit are dropped");
}

#[test]
fn the_asides_route_matches_ruby() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let stamp = |secs: i64| (chrono::Utc::now() - chrono::Duration::seconds(secs)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let assistant = |id: &str, model: &str, blocks: &[&str]| format!("{}\n", serde_json::json!({"type": "assistant", "message": {"id": id, "model": model, "content": blocks.iter().map(|t| serde_json::json!({"type": "text", "text": t})).collect::<Vec<_>>()}}));
    let user = |text: &str| format!("{}\n", serde_json::json!({"type": "user", "message": {"role": "user", "content": text}}));
    let mut log = fs::read_to_string(w.root.join("sessions.jsonl")).unwrap();
    for (id, slug, text, age, ts) in [
        ("h-old", "as-one", Some(user("first question") + &assistant("m1", "claude-opus-5-5", &["an answer", "in two blocks"])), 90_000, true),
        ("h-new", "as-one", Some(user("second question") + &assistant("m1", "claude-sonnet-5-5", &["already a note"])), 3_000, true),
        ("h-none", "as-one", None, 2_000, true),
        ("h-empty", "as-one", Some(user("nothing back")), 1_000, true),
        ("h-nots", "as-one", Some(assistant("m1", "<synthetic>", &["no ts"])), 500, false),
        ("h-other", "as-two", Some(assistant("m1", "claude-opus-5-5", &["not yours"])), 100, true),
    ] {
        let path = w.transcripts.join(format!("p/{id}.jsonl"));
        if let Some(t) = text {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, t).unwrap();
        }
        let when = if ts { format!(",\"ts\":\"{}\"", stamp(age)) } else { String::new() };
        log += &format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"/srv/app\",\"pid\":999999,\"aside\":\"{slug}\",\"transcript_path\":\"{}\"{when}}}\n", path.display());
    }
    fs::write(w.root.join("sessions.jsonl"), log).unwrap();
    for (slug, notes) in [("as-one", "- 2026-01-02: aside: already a note\n"), ("as-two", ""), ("as-bare", "")] {
        fs::write(w.root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\ntitle: {slug}\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\n---\n\n## Description\n\nd\n\n## Notes\n\n{notes}\n## Snapshots\n")).unwrap();
    }
    let env: Vec<(&str, String)> = vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string())];
    let root2 = base.path().join("root2");
    copy_dir(&w.root, &root2);
    let ruby = start_env(false, &w.root, &w.home, &env);
    let rust = start_env(true, &root2, &w.home, &env);
    let mut seen = String::new();
    for slug in ["as-one", "as-two", "as-bare", "nowhere"] {
        let path = format!("/api/asides/{slug}");
        let (a, b) = (request(ruby.port, "GET", &path, &[], "", true), request(rust.port, "GET", &path, &[], "", true));
        assert_eq!(shown(&a, ruby.port), shown(&b, rust.port), "{slug}");
        seen += &String::from_utf8_lossy(&a.body);
    }
    let log = |root: &Path| fs::read_to_string(root.join("qm.log")).unwrap_or_default().lines().map(|l| l.splitn(2, ' ').nth(1).unwrap_or("").to_string()).collect::<Vec<_>>();
    assert_eq!(log(&w.root), log(&root2));
    for needle in ["\"promoted\":true", "\"promoted\":false", "\"model\":\"claude-opus-5-5\"", "\"answer\":null", "\"tier\":\"expired\"", "\"at\":null", "ago", "no work item"] {
        assert!(seen.contains(needle), "the world should reach {needle}: {seen}");
    }
}

#[test]
fn the_session_route_matches_ruby() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let j = |v: serde_json::Value| format!("{v}\n");
    let user = |text: &str, ts: &str| j(serde_json::json!({"type": "user", "timestamp": ts, "message": {"role": "user", "content": text}}));
    let asst = |model: &str, blocks: serde_json::Value| j(serde_json::json!({"type": "assistant", "message": {"model": model, "content": blocks}}));
    let tool = |name: &str, file: Option<&str>| serde_json::json!({"type": "tool_use", "name": name, "input": file.map(|f| serde_json::json!({"file_path": f})).unwrap_or(serde_json::json!({}))});
    let long = "r".repeat(500);
    let long_prompt = "p".repeat(900);
    let mut transcripts: Vec<(&str, String)> = vec![];
    transcripts.push(("t-full", [
        j(serde_json::json!({"type": "ai-title", "aiTitle": "A fine title"})),
        asst("claude-opus-5-5", serde_json::json!([{"type": "text", "text": "before any prompt"}])),
        user("first prompt", "2026-03-01T10:00:00.000Z"),
        j(serde_json::json!({"type": "assistant", "gitBranch": "feat/x", "message": {"model": "claude-opus-5-5", "content": [tool("Read", Some("/a/b/one.rs")), tool("Read", Some("/a/two.rs")), tool("Edit", Some("/a/b/one.rs")), tool("Bash", None), {"type": "text", "text": "  the reply  "}]}})),
        user(&long_prompt, "2026-03-01T10:05:00.000Z"),
        asst("claude-sonnet-5-5", serde_json::json!([tool("Grep", None), {"type": "text", "text": long}, {"type": "text", "text": "second text wins"}, 5, "str"])),
        j(serde_json::json!({"type": "user", "isMeta": true, "message": {"content": "meta"}})),
        j(serde_json::json!({"type": "user", "toolUseResult": {"x": 1}, "message": {"content": "tool result"}})),
        user("<command-name>x</command-name>", "2026-03-01T10:06:00.000Z"),
        j(serde_json::json!({"type": "user", "isSidechain": true, "message": {"content": "side"}})),
        j(serde_json::json!({"type": "assistant", "isSidechain": true, "gitBranch": "side", "message": {"model": "side-model", "content": []}})),
        "not json\n".to_string(),
        "\"just a string\"\n".to_string(),
        "\n".to_string(),
        user("third prompt", "2026-03-01T10:10:00.000Z"),
        asst("<synthetic>", serde_json::json!("a string content")),
        j(serde_json::json!({"type": "assistant", "message": {"content": [tool("Read", Some("/f/1")), tool("Read", Some("/f/2")), tool("Read", Some("/f/3")), tool("Read", Some("/f/4")), tool("Read", Some("/f/5")), tool("Read", Some("/f/6")), tool("Read", Some("/f/7")), serde_json::json!({"type": "tool_use", "input": null})]}})),
    ].concat()));
    transcripts.push(("t-edges", [
        j(serde_json::json!({"type": "user", "gitBranch": "kept", "timestamp": "2026-03-04T10:00:00.000Z", "message": {"content": format!("{} bcccc", "a".repeat(699))}})),
        j(serde_json::json!({"type": "assistant", "gitBranch": "", "message": {"content": [{"type": "text", "text": "r".repeat(361)}]}})),
        user("second", "2026-03-04T10:01:00.000Z"),
        j(serde_json::json!({"type": "assistant", "message": {"content": [{"type": "text", "text": format!("{} tail words", "s".repeat(359))}]}})),
    ].concat()));
    transcripts.push(("t-array", format!("{}[1,2]\n", user("a prompt", "2026-03-02T10:00:00.000Z"))));
    transcripts.push(("t-number", format!("{}5\n", user("a prompt", "2026-03-02T10:00:00.000Z"))));
    transcripts.push(("t-null", format!("{}null\n", user("a prompt", "2026-03-02T10:00:00.000Z"))));
    transcripts.push(("t-badmsg", format!("{}{}", user("a prompt", "2026-03-02T10:00:00.000Z"), j(serde_json::json!({"type": "assistant", "message": "text"})))));
    transcripts.push(("t-baduser", j(serde_json::json!({"type": "user", "message": "oops"}))));
    transcripts.push(("t-badinput", format!("{}{}", user("a prompt", "2026-03-02T10:00:00.000Z"), asst("m", serde_json::json!([{"type": "tool_use", "name": "X", "input": "text"}])))));
    transcripts.push(("t-empty", String::new()));
    transcripts.push(("t-noreply", user("only a prompt", "2026-03-03T10:00:00.000Z")));
    let mut log = fs::read_to_string(w.root.join("sessions.jsonl")).unwrap();
    for (id, text) in &transcripts {
        let path = w.transcripts.join(format!("p/{id}.jsonl"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        log += &format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"{}/proj\",\"pid\":999999,\"transcript_path\":\"{}\",\"ts\":\"2026-03-01T09:00:00Z\"}}\n", w.home.display(), path.display());
    }
    log += "{\"v\":1,\"event\":\"start\",\"session_id\":\"t-pruned\",\"cwd\":\"/srv/app\",\"pid\":999999,\"transcript_path\":\"/nonexistent/x.jsonl\",\"ts\":\"2026-03-01T09:00:00Z\"}\n";
    fs::write(w.root.join("sessions.jsonl"), log).unwrap();
    let env: Vec<(&str, String)> = vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string())];
    let ruby = start_env(false, &w.root, &w.home, &env);
    let rust = start_env(true, &w.root, &w.home, &env);
    let mut seen = String::new();
    let ids: Vec<&str> = transcripts.iter().map(|(id, _)| *id).chain(["t-pruned", "d-live", "never-heard-of", "x/y"]).collect();
    for id in ids {
        let path = format!("/api/session/{id}");
        let (a, b) = (request(ruby.port, "GET", &path, &[], "", true), request(rust.port, "GET", &path, &[], "", true));
        assert_eq!(shown(&a, ruby.port), shown(&b, rust.port), "{id}");
        seen += &String::from_utf8_lossy(&a.body);
    }
    for needle in ["\"title\":\"A fine title\"", "\"branch\":\"feat/x\"", "\"model\":\"<synthetic>\"", "\"name\":\"Read\",\"count\":2", "one.rs", "\"turns\":null", "\"tier\":\"uncaptured\"", "…", "\"reply\":\"the reply\"", "\"reply\":null"] {
        assert!(seen.contains(needle), "the world should reach {needle}");
    }
}

#[test]
fn the_issues_routes_match_ruby() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    git(&w.repo, &["remote", "add", "origin", "git@github.com:acme/widgets.git"]);
    let issue = |slug: &str, name: &str, text: &str| {
        let p = w.root.join(format!("issues/{slug}/{name}"));
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    };
    let item = |slug: &str, state: &str, project: &str| {
        fs::write(w.root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\ntitle: Title of {slug}\nstate: {state}\nproject: {project}\ncreated: '2026-01-01'\n---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n")).unwrap();
    };
    for (slug, state, project) in [("is-a", "in_progress", "demo"), ("is-b", "fridge", "demo"), ("is-c", "resolved", "demo"), ("is-d", "in_progress", "elsewhere"), ("is-e", "in_progress", "demo"), ("is-f", "fridge", "demo"), ("is-g", "fridge", "demo"), ("is-weird", "weird", "demo"), ("is-empty", "in_progress", "demo")] {
        item(slug, state, project);
    }
    issue("is-a", "01-bug.md", "---\ntitle: A bug\nkind: bug\ngroup: '  auth  '\nparent: epic-1\ndeps: [one, none, 2]\nchange: tweak\n---\n\n\n   \nthe body\n\nwith  paragraphs  \n\n");
    issue("is-a", "02-made.md", "---\ntitle: Made\nkind: feature\nnumber: '#12'\ndeps: just-one\n---\n\nbody two\n");
    issue("is-a", "03-broken.md", "---\ntitle: Broken\nkind: epic\n---\n\nbad kind\n");
    issue("is-a", "04-nofront.md", "no front matter here\n");
    issue("is-a", "05-acked.md", "---\ntitle: Acked now\nkind: bug\n---\n\nnew words\n");
    issue("is-a", "06-changed.md", "---\ntitle: Changed title\nkind: bug\n---\n\nnow longer\n");
    issue("is-a", "07-plain.md", "---\ntitle: Plain\nkind: feature\nnumber: 5\n---\n\n");
    fs::create_dir_all(w.root.join("issues/is-a/.prev")).unwrap();
    for (name, seen, extra) in [("05-acked.md", "---\ntitle: Acked now\nkind: bug\n---\n\nnew words\n", ",\"acked\":true,\"previous\":\"---\\ntitle: Old\\nkind: bug\\n---\\n\\nold\\n\""), ("06-changed.md", "---\ntitle: Changed title\nkind: bug\n---\n\nnow longer\n", ",\"previous\":\"---\\ntitle: Before\\nkind: bug\\n---\\n\\nshort\\n\"")] {
        fs::write(w.root.join(format!("issues/is-a/.prev/{name}.json")), format!("{{\"v\":1,\"seen\":{},\"seen_at\":1.0{extra}}}", serde_json::to_string(seen).unwrap())).unwrap();
    }
    issue("is-b", "01-only.md", "---\ntitle: Only\nkind: bug\n---\n\nb\n");
    issue("is-c", "01-done.md", "---\ntitle: Done\nkind: bug\nnumber: 3\n---\n\nc\n");
    issue("is-d", "01-x.md", "---\ntitle: X\nkind: bug\n---\n\nd\n");
    issue("is-e", "01-broken.md", "---\nkind: bug\n---\n\nno title\n");
    issue("is-f", "01-f.md", "---\ntitle: F\nkind: bug\n---\n\nf\n");
    issue("is-g", "01-g.md", "---\ntitle: G\nkind: bug\n---\n\ng\n");
    issue("is-weird", "01-w.md", "---\ntitle: W\nkind: bug\n---\n\nw\n");
    issue("no-item", "01-orphan.md", "---\ntitle: Orphan\nkind: bug\n---\n\no\n");
    fs::create_dir_all(w.root.join("issues/is-empty")).unwrap();
    fs::create_dir_all(w.root.join("issues/.hidden")).unwrap();
    let now = std::time::SystemTime::now();
    for (slug, name, age) in [("is-a", "01-bug.md", 1000), ("is-b", "01-only.md", 5000), ("is-c", "01-done.md", 100), ("is-d", "01-x.md", 3000), ("is-e", "01-broken.md", 2000), ("is-f", "01-f.md", 4000), ("is-g", "01-g.md", 4000), ("is-weird", "01-w.md", 6000)] {
        fs::File::options().write(true).open(w.root.join(format!("issues/{slug}/{name}"))).unwrap().set_modified(now - std::time::Duration::from_secs(age)).unwrap();
    }
    let env: Vec<(&str, String)> = vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string())];
    let root2 = base.path().join("root2");
    copy_dir(&w.root, &root2);
    let ruby = start_env(false, &w.root, &w.home, &env);
    let rust = start_env(true, &root2, &w.home, &env);
    let mut seen = String::new();
    let mut paths = vec!["/api/issues".to_string()];
    for slug in ["is-a", "is-b", "is-c", "is-d", "is-e", "no-item", "is-empty", "nowhere", "bad%20slug", "../x"] {
        paths.push(format!("/api/issues/{slug}"));
    }
    paths.push("/api/issues".to_string());
    paths.push("/api/issues/is-a".to_string());
    for path in &paths {
        let (a, b) = (request(ruby.port, "GET", path, &[], "", true), request(rust.port, "GET", path, &[], "", true));
        assert_eq!(shown(&a, ruby.port), shown(&b, rust.port), "{path}");
        assert_eq!(issue_traces(&w.root, &root2), issue_traces(&root2, &w.root), "files after {path}");
        seen += &String::from_utf8_lossy(&a.body);
    }
    for needle in ["\"repo\":\"acme/widgets\"", "\"repo\":null", "\"acknowledged\":true", "\"previous\":{\"title\":\"Before\"", "\"deps\":[\"one\",\"2\"]", "\"group\":\"auth\"", "kind must be feature or bug", "no frontmatter", "title is missing", "\"created\":true", "no work item", "\"state\":\"resolved\""] {
        assert!(seen.contains(needle), "the world should reach {needle}: {seen}");
    }
}

#[test]
fn the_workspace_route_matches_ruby() {
    let base = tempfile::tempdir().unwrap();
    let (base_a, base_b) = (base.path().join("a"), base.path().join("b"));
    let (wa, wb) = (workspace_world(&base_a, false), workspace_world(&base_b, false));
    for w in [&wa, &wb] {
        fs::write(w.root.join("work-items/w-nochat.md"), "---\nv: 1\ntitle: w-nochat\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\nbranches:\n- st-1\n---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n").unwrap();
    }
    let socks = ["qm-test-wsr-0", "qm-test-wsr-1"];
    for (i, b) in [&base_a, &base_b].iter().enumerate() {
        tmux_on(socks[i], &["kill-server"]);
        // a chat running in a worktree, one in the stack's bottom worktree
        for (name, dir) in [("w-exist", "wt-w"), ("w-stack", "wt-st1"), ("w-nochat", "repo")] {
            for _ in 0..50 {
                tmux_on(socks[i], &["new-session", "-d", "-s", name, "-c", b.join(dir).to_str().unwrap(), "sleep 300"]);
                if tmux_on(socks[i], &["list-sessions", "-F", "#{session_name}"]).contains(name) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
    let env_for = |w: &common::AttentionWorld, side: usize| -> Vec<(&'static str, String)> {
        let base = if side == 0 { &base_a } else { &base_b };
        vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("PATH", format!("{}:{}:{}", base.join("tools").display(), w.bin.display(), std::env::var("PATH").unwrap())), ("QM_TMUX_SOCKET", socks[side].to_string())]
    };
    let ruby = start_env(false, &wa.root, &wa.home, &env_for(&wa, 0));
    let rust = start_env(true, &wb.root, &wb.home, &env_for(&wb, 1));
    let norm = |text: String, side: usize| text.replace(&(if side == 0 { &base_a } else { &base_b }).display().to_string(), "BASE");
    let mut seen = String::new();
    let mut paths: Vec<String> = ["w-exist", "w-feat", "w-slash", "w-origin", "w-stack", "w-main", "w-two", "w-plain", "w-none", "w-nochat", "nowhere"].iter().map(|s| format!("/api/workspace/{s}")).collect();
    for (slug, branch) in [("w-two", "feat-a"), ("w-two", "feat-w"), ("w-two", "main"), ("w-stack", "st-2"), ("w-stack", "st-1"), ("w-exist", ""), ("w-exist", "nope")] {
        paths.push(format!("/api/workspace/{slug}?branch={branch}"));
    }
    paths.push("/api/workspace/w-stack".to_string());
    for path in &paths {
        let (a, b) = (request(ruby.port, "GET", path, &[], "", true), request(rust.port, "GET", path, &[], "", true));
        assert_eq!(norm(shown(&a, ruby.port), 0), norm(shown(&b, rust.port), 1), "{path}");
        seen += &norm(String::from_utf8_lossy(&a.body).into_owned(), 0);
    }
    let items = |w: &common::AttentionWorld, side: usize| {
        let mut t = String::new();
        tree_text(&w.root.join("work-items"), "items", &mut t);
        norm(t, side)
    };
    assert_eq!(items(&wa, 0), items(&wb, 1), "the stack's branches are linked as Ruby links them");
    for needle in ["\"devServer\":{", "\"ide\":{\"label\":\"FakeIDE\"", "\"create\":true", "\"alternatives\":[{", "\"chatBranch\":\"feat-w\"", "\"defaultBranch\":\"st-1\"", "\"defaultBranch\":\"st-2\"", "not linked to", "\"devServer\":null", "no work item"] {
        assert!(seen.contains(needle), "the world should reach {needle}: {seen}");
    }
    for s in socks {
        tmux_on(s, &["kill-server"]);
    }
}

#[test]
fn the_history_route_matches_ruby() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let me = std::process::id();
    let j = |v: serde_json::Value| format!("{v}\n");
    let user = |text: &str, ts: &str| j(serde_json::json!({"type": "user", "timestamp": ts, "message": {"role": "user", "content": text}}));
    let asst = |id: Option<&str>, ts: &str, blocks: serde_json::Value| j(serde_json::json!({"type": "assistant", "timestamp": ts, "message": {"id": id, "content": blocks}}));
    let home = w.home.display().to_string();
    let mut shapes = String::new();
    shapes += &user("first", "2026-03-01T10:00:00.000Z");
    shapes += &asst(Some("m1"), "2026-03-01T10:00:05.000Z", serde_json::json!([{"type": "text", "text": "  part one  "}, {"type": "tool_use", "name": "Bash", "input": {"command": "git   status\n--short", "description": "ignored"}}, {"type": "text", "text": "after the tool"}]));
    shapes += &asst(Some("m1"), "2026-03-01T10:00:06.000Z", serde_json::json!([{"type": "text", "text": "same message, next block"}]));
    shapes += &asst(Some("m2"), "2026-03-01T10:00:07.000Z", serde_json::json!([{"type": "tool_use", "name": "Read", "input": {"file_path": format!("{home}/proj/file.rs")}}, {"type": "tool_use", "name": "Odd", "input": {"zzz": "  only value  ", "n": 5}}, {"type": "tool_use", "input": {}}, {"type": "tool_use", "name": "Grep", "input": {"pattern": "p".repeat(300)}}, {"type": "tool_use", "name": "Nothing", "input": "text"}]));
    shapes += &asst(Some("m3"), "2026-03-01T10:00:08.000Z", serde_json::json!("content as a string"));
    shapes += &asst(None, "2026-03-01T10:00:09.000Z", serde_json::json!([{"type": "text", "text": "no id"}]));
    shapes += &asst(None, "2026-03-01T10:00:10.000Z", serde_json::json!([{"type": "text", "text": "no id again"}, {"type": "text", "text": ""}, {"type": "text", "text": null}, {"type": "thinking", "text": "hmm"}]));
    shapes += &user("[Request interrupted by user for tool use]", "2026-03-01T10:01:00.000Z");
    shapes += &j(serde_json::json!({"type": "user", "isCompactSummary": true, "message": {"content": "compact"}}));
    shapes += &j(serde_json::json!({"type": "user", "isSidechain": true, "message": {"content": "side"}}));
    shapes += &j(serde_json::json!({"type": "user", "isMeta": true, "message": {"content": "meta"}}));
    shapes += &j(serde_json::json!({"type": "user", "toolUseResult": {}, "message": {"content": "result"}}));
    shapes += &user("<local-command>x</local-command>", "2026-03-01T10:02:00.000Z");
    shapes += "junk\n[1]\n\"s\"\n";
    shapes += &user("last", "2026-03-01T10:03:00.000Z");
    let mut long = String::new();
    for i in 0..130 {
        long += &user(&format!("prompt {i}"), &format!("2026-03-02T10:{:02}:00.000Z", i % 60));
    }
    let mut log = fs::read_to_string(w.root.join("sessions.jsonl")).unwrap();
    let mut session = |id: &str, text: Option<String>, pid: u32, started: &str, last: Option<&str>| {
        let path = w.transcripts.join(format!("p/{id}.jsonl"));
        if let Some(t) = text {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, t).unwrap();
        }
        log += &format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"/srv/app\",\"pid\":{pid},\"transcript_path\":\"{}\",\"ts\":\"{started}\"}}\n", path.display());
        if let Some(l) = last {
            log += &format!("{{\"v\":1,\"event\":\"end\",\"session_id\":\"{id}\",\"ts\":\"{l}\"}}\n");
        }
    };
    session("h-shapes", Some(shapes), 999999, "2026-03-01T09:59:00+00:00", Some("2026-03-01T10:05:00+00:00"));
    session("h-long", Some(long), 999999, "2026-03-02T09:59:00+01:00", Some("2026-03-02T12:05:00+01:00"));
    session("h-snap", None, 999999, "2026-02-20T09:00:00+00:00", Some("2026-02-20T10:00:00+00:00"));
    session("h-gone", None, 999999, "2026-02-10T09:00:00+00:00", Some("2026-02-10T10:00:00+00:00"));
    session("h-live", Some(user("running", "2026-03-05T10:00:00.000Z")), me as u32, "2026-03-05T09:59:00+00:00", None);
    fs::write(w.root.join("sessions.jsonl"), log).unwrap();
    fs::write(
        w.root.join("work-items/hist.md"),
        "---\nv: 1\ntitle: hist\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\nrotations:\n- at: '2026-03-01T11:00:00+00:00'\n  from: h-shapes\n  via: wrap\n  carried: [1, 3, 99]\n  decisions: 2\n  open: 1\n- at: '2026-02-25T00:00:00Z'\n  from: h-snap\n- at: '2026-03-04T00:00:00+00:00'\n  from: h-long\n  via: clear\n  carried: []\nsessions:\n- id: h-shapes\n- id: h-long\n- id: h-snap\n- id: h-gone\n- id: h-live\n- id: h-unknown\n---\n\n## Description\n\nd\n\n## Notes\n\n- 2026-02-01: first decision\n- 2026-02-02 [open]: a question\n- 2026-02-03 [history]: mechanics\n\n## Snapshots\n\n### 2026-02-21 — session h-snap\n\nthe distilled text\n"
    ).unwrap();
    fs::write(w.root.join("work-items/hist-bare.md"), "---\nv: 1\ntitle: hist-bare\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\n---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n").unwrap();
    let env: Vec<(&str, String)> = vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string())];
    let ruby = start_env(false, &w.root, &w.home, &env);
    let rust = start_env(true, &w.root, &w.home, &env);
    let mut seen = String::new();
    let mut paths: Vec<String> = ["hist", "hist-bare", "nowhere", ""].iter().map(|s| format!("/api/history/{s}")).collect();
    for (session, query) in [("h-shapes", ""), ("h-long", ""), ("h-long", "?before=70"), ("h-long", "?before=10"), ("h-long", "?before=0"), ("h-long", "?before=-5"), ("h-long", "?before=1000"), ("h-long", "?before=abc"), ("h-long", "?before="), ("h-snap", ""), ("h-gone", ""), ("h-live", ""), ("h-unknown", ""), ("nope", ""), ("", "")] {
        paths.push(format!("/api/history/hist/{session}{query}"));
    }
    paths.push("/api/history/hist-bare/x".to_string());
    for path in &paths {
        let (a, b) = (request(ruby.port, "GET", path, &[], "", true), request(rust.port, "GET", path, &[], "", true));
        assert_eq!(shown(&a, ruby.port), shown(&b, rust.port), "{path}");
        seen += &String::from_utf8_lossy(&a.body);
    }
    for needle in ["\"type\":\"divider\"", "\"source\":\"snapshot\"", "\"source\":\"none\"", "\"source\":\"transcript\"", "part one", "after the tool\\n\\nsame message, next block", "Bash  git status --short", "Read  ~/proj/file.rs", "Odd  only value", "\"kind\":\"event\"", "\"kind\":\"snapshot\"", "nothing of this session survives", "\"before\":70", "\"total\":130", "not an earlier session", "\"decisions\":2", "first decision"] {
        assert!(seen.contains(needle), "the world should reach {needle}");
    }
}

#[test]
fn turn_notifications_match_ruby() {
    let base = tempfile::tempdir().unwrap();
    let (base_a, base_b) = (base.path().join("a"), base.path().join("b"));
    let (wa, wb) = (attention_world(&base_a, std::time::SystemTime::now(), None), attention_world(&base_b, std::time::SystemTime::now(), None));
    let now = chrono::Utc::now();
    let at = |secs: i64| (now - chrono::Duration::seconds(secs)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    for w in [&wa, &wb] {
        let mut log = fs::read_to_string(w.root.join("sessions.jsonl")).unwrap();
        for (id, extra, prompt) in [("n-linked", "", None), ("n-blank", "", None), ("n-long", "", None), ("n-short", "", None), ("n-hour", "", None), ("n-snooze", "", None), ("n-aside", ",\"aside\":\"nlink\"", None), ("n-loose", ",\"loose\":\"abcd1234\"", Some("a question that was asked in a loose chat")), ("n-unlinked", "", Some("an unlinked session\nwith two lines")), ("n-bare", "", None), ("n-wait", "", None)] {
            let path = w.transcripts.join(format!("p/{id}.jsonl"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut text = prompt.map(|p| format!("{}\n", serde_json::json!({"type": "user", "message": {"role": "user", "content": p}}))).unwrap_or_default();
            if id == "n-wait" {
                text += &format!("{}\n", serde_json::json!({"type": "user", "timestamp": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true), "toolUseResult": {"status": "async_launched", "agentId": "bg-1", "description": "a job"}}));
            }
            fs::write(&path, text).unwrap();
            log += &format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"/srv/app\",\"pid\":999999,\"transcript_path\":\"{}\",\"ts\":\"{}\"{extra}}}\n", path.display(), at(3600));
        }
        fs::write(w.root.join("sessions.jsonl"), log).unwrap();
        for (slug, sessions, extra) in [("nlink", "- id: n-linked\n- id: n-long\n- id: n-short\n- id: n-hour\n- id: n-wait\n", ""), ("nsnooze", "- id: n-snooze\n", "turn_snoozed_at: '2099-01-01T00:00:00+00:00'\n")] {
            fs::write(w.root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\ntitle: Title of {slug}\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\n{extra}sessions:\n{sessions}---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n")).unwrap();
        }
        executable(&w.bin.join("terminal-notifier"), "#!/bin/sh\necho \"$@\" >> \"$FAKE_NOTIFY_LOG\"\n");
    }
    // what happened before the GUI started is not news
    for w in [&wa, &wb] {
        fs::write(w.root.join("events.jsonl"), format!("{}\n{}\n", serde_json::json!({"v": 1, "ts": at(500), "event": "UserPromptSubmit", "session_id": "n-old"}), serde_json::json!({"v": 1, "ts": at(490), "event": "PermissionRequest", "session_id": "n-old", "tool": "Bash"}))).unwrap();
    }
    let logs = [base_a.join("notify.log"), base_b.join("notify.log")];
    let env_for = |w: &common::AttentionWorld, side: usize| -> Vec<(&'static str, String)> { vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("PATH", format!("{}:{}", w.bin.display(), std::env::var("PATH").unwrap())), ("FAKE_NOTIFY_LOG", logs[side].display().to_string())] };
    let ruby = start_env(false, &wa.root, &wa.home, &env_for(&wa, 0));
    let rust = start_env(true, &wb.root, &wb.home, &env_for(&wb, 1));
    std::thread::sleep(std::time::Duration::from_millis(2000)); // the first check only takes stock
    let state = |s: &Server| -> serde_json::Value { serde_json::from_slice(&request(s.port, "GET", "/api/state", &[], "", true).body).unwrap() };
    let event = |name: &str, id: &str, secs: i64, tool: Option<&str>| format!("{}\n", serde_json::json!({"v": 1, "ts": at(secs), "event": name, "session_id": id, "tool": tool}));
    let emit = |text: &str| {
        for w in [&wa, &wb] {
            use std::io::Write;
            fs::OpenOptions::new().append(true).create(true).open(w.root.join("events.jsonl")).unwrap().write_all(text.as_bytes()).unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(3400));
    };
    let notes = |s: &Server| state(s)["notifications"].to_string();
    let notified = |side: usize| fs::read_to_string(&logs[side]).unwrap_or_default().lines().map(|l| regex::Regex::new(r"-execute .*? (open-chat [^ ]*)$").unwrap().replace(l, "-execute <qm> $1").into_owned()).map(|l| regex::Regex::new(r"-execute \S+ ").unwrap().replace(&l, "-execute ").into_owned()).collect::<Vec<_>>();
    let step = |label: &str| {
        assert_eq!(notes(&ruby), notes(&rust), "{label}: the page's notifications");
        let (a, b) = (notified(0), notified(1));
        assert_eq!(a.len(), b.len(), "{label}: macOS notifications: {a:?} vs {b:?}");
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.split("-execute").next(), y.split("-execute").next(), "{label}");
            assert_eq!(x.rsplit(' ').next(), y.rsplit(' ').next(), "{label}: the click opens the same chat");
        }
        assert_eq!(state(&ruby)["pulse"].as_str().unwrap().rsplit(':').next(), state(&rust)["pulse"].as_str().unwrap().rsplit(':').next(), "{label}: the pulse moves with each notification");
    };
    emit(&[event("UserPromptSubmit", "n-linked", 200, Some("Edit")), event("PermissionRequest", "n-linked", 190, Some("Bash"))].concat());
    step("blocked on a tool");
    emit(&[event("UserPromptSubmit", "n-bare", 180, None), event("PermissionRequest", "n-bare", 170, None)].concat());
    step("blocked, no tool, no item");
    emit(&[event("UserPromptSubmit", "n-long", 300, None), event("Stop", "n-long", 255, None)].concat());
    step("a long turn");
    emit(&[event("UserPromptSubmit", "n-short", 100, None), event("Stop", "n-short", 90, None)].concat());
    step("a short turn is no news");
    emit(&[event("UserPromptSubmit", "n-hour", 7000, None), event("Stop", "n-hour", 3250, None)].concat());
    step("an hour-long turn");
    emit(&[event("UserPromptSubmit", "n-loose", 60, None), event("PermissionRequest", "n-loose", 50, Some("Write"))].concat());
    step("a loose chat");
    emit(&[event("UserPromptSubmit", "n-unlinked", 60, None), event("PermissionRequest", "n-unlinked", 50, Some("Edit"))].concat());
    step("an unlinked session, titled by its prompt");
    emit(&[event("UserPromptSubmit", "n-aside", 60, None), event("PermissionRequest", "n-aside", 50, Some("Edit"))].concat());
    step("an aside is never news");
    emit(&[event("UserPromptSubmit", "n-snooze", 300, None), event("Stop", "n-snooze", 200, None)].concat());
    step("a snoozed turn");
    emit(&[event("UserPromptSubmit", "n-wait", 300, None), event("Stop", "n-wait", 200, None)].concat());
    step("a turn that handed off to background work");
    // the page's pulse chooses the channel: a visible tab shows the notification itself, and says nothing to
    // macOS; one focused on that very chat says nothing at all
    let keep_seen = |chat: &'static str| {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let handles: Vec<_> = [ruby.port, rust.port]
            .into_iter()
            .map(|port| {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                        request(port, "GET", &format!("/api/pulse?visible=1&focused=1&chat={chat}"), &[], "", true);
                        std::thread::sleep(std::time::Duration::from_millis(300));
                    }
                })
            })
            .collect();
        (stop, handles)
    };
    let release = |held: (std::sync::Arc<std::sync::atomic::AtomicBool>, Vec<std::thread::JoinHandle<()>>)| {
        held.0.store(true, std::sync::atomic::Ordering::SeqCst);
        for h in held.1 {
            h.join().unwrap();
        }
    };
    let before = (notified(0).len(), notified(1).len());
    let held = keep_seen("nlink");
    emit(&[event("UserPromptSubmit", "n-linked", 40, Some("Edit")), event("PermissionRequest", "n-linked", 30, Some("Read"))].concat());
    release(held);
    step("watching that very chat");
    assert_eq!(before, (notified(0).len(), notified(1).len()), "a tab on that chat is told, macOS is not");
    let held = keep_seen("elsewhere");
    emit(&[event("UserPromptSubmit", "n-linked", 20, Some("Edit")), event("PermissionRequest", "n-linked", 10, Some("Grep"))].concat());
    release(held);
    step("visible, on another chat");
    assert_eq!(before, (notified(0).len(), notified(1).len()), "a visible tab shows it itself");
    let (a, b) = (request(ruby.port, "GET", "/api/attention", &[], "", true), request(rust.port, "GET", "/api/attention", &[], "", true));
    assert_eq!(shown(&a, ruby.port), shown(&b, rust.port), "the attention payload");
    assert!(String::from_utf8_lossy(&a.body).contains("\"notifications\":[{"));
    let seen = notes(&rust);
    for needle in ["Needs you: Bash", "Needs your answer", "Your turn — after 0m 45s", "Your turn — after 1h 2m", "\"loose\":\"loose--abcd1234\"", "a question that was asked in a loose chat", "an unlinked session\\nwith two lines", "\"title\":\"Title of nlink\"", "\"page\":false"] {
        assert!(seen.contains(needle) || notes(&ruby).contains(needle), "the world should reach {needle}: {seen}");
    }
    let log = notified(1).join("\n");
    assert!(log.contains("-sound Glass") && log.contains("open-chat nlink") && log.contains("open-chat loose--abcd1234"), "{log}");
    assert!(!seen.contains("n-aside") && !seen.contains("n-short") && !seen.contains("n-snooze") && !seen.contains("n-wait"), "{seen}");
}

#[test]
fn dropped_files_are_saved_as_ruby_saves_them() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let root2 = base.path().join("root2");
    copy_dir(&w.root, &root2);
    let old = |root: &Path| {
        fs::create_dir_all(root.join("drops")).unwrap();
        let p = root.join("drops/old-file.png");
        fs::write(&p, "old").unwrap();
        fs::File::options().write(true).open(&p).unwrap().set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 86_400)).unwrap();
        let fresh = root.join("drops/fresh-file.png");
        fs::write(&fresh, "fresh").unwrap();
    };
    old(&w.root);
    old(&root2);
    let env: Vec<(&str, String)> = vec![("HOME", w.home.display().to_string())];
    let ruby = start_env(false, &w.root, &w.home, &env);
    let rust = start_env(true, &root2, &w.home, &env);
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/octet-stream".to_string())];
    let token = |s: &Server| -> String { serde_json::from_slice::<serde_json::Value>(&request(s.port, "GET", "/api/state", &[], "", true).body).unwrap()["terminalToken"].as_str().unwrap().to_string() };
    let (ta, tb) = (token(&ruby), token(&rust));
    let long = "n".repeat(120);
    let mut seen = String::new();
    let stamp = regex::Regex::new(r"/drops/\d{8}-\d{6}-[0-9a-f]{6}-").unwrap();
    let cases: Vec<(&str, String, Option<&str>, &str)> = vec![
        ("a plain drop", "name=shot.png".into(), None, "bytes of a screenshot"),
        ("a path as a name", "name=..%2F..%2Fetc%2Fpass%20wd.png".into(), None, "x"),
        ("a name of dots", "name=...".into(), None, "x"),
        ("no name", String::new(), None, "x"),
        ("a unicode name", "name=r%C3%A9sum%C3%A9%20%F0%9F%99%82.pdf".into(), None, "x"),
        ("a long name", format!("name={long}.png"), None, "x"),
        ("an empty file", "name=empty.png".into(), None, ""),
        ("a bad token", "name=a.png".into(), Some("wrong"), "x"),
        ("no token", "name=a.png".into(), Some(""), "x"),
    ];
    for (label, query, token_override, body) in cases {
        let go = |s: &Server, tok: &str| {
            let t = token_override.unwrap_or(tok);
            let q = if t.is_empty() { query.clone() } else { format!("{query}&token={t}").trim_start_matches('&').to_string() };
            request(s.port, "POST", &format!("/api/drop?{q}"), &origin, body, true)
        };
        let (a, b) = (go(&ruby, &ta), go(&rust, &tb));
        let (sa, sb) = (shown(&a, ruby.port), shown(&b, rust.port));
        let (sa, sb) = (sa.replace(&w.root.display().to_string(), "ROOT"), sb.replace(&root2.display().to_string(), "ROOT"));
        assert_eq!((stamp.replace_all(&sa, "/drops/STAMP-"), a.status.clone()), (stamp.replace_all(&sb, "/drops/STAMP-"), b.status.clone()), "{label}");
        seen += &sa;
    }
    let listing = |root: &Path| {
        let mut names: Vec<String> = fs::read_dir(root.join("drops")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).map(|n| regex::Regex::new(r"^\d{8}-\d{6}-[0-9a-f]{6}-").unwrap().replace(&n, "STAMP-").into_owned()).collect();
        names.sort();
        names
    };
    assert_eq!(listing(&w.root), listing(&root2));
    assert!(!listing(&root2).contains(&"old-file.png".to_string()) && listing(&root2).contains(&"fresh-file.png".to_string()), "{:?}", listing(&root2));
    for needle in ["\"ok\":true", "dropped file is empty", "bad terminal token", "STAMP-shot.png"] {
        assert!(seen.replace(|c: char| c.is_ascii_digit(), "").contains(&needle.replace(|c: char| c.is_ascii_digit(), "")) || listing(&root2).iter().any(|n| n.contains("shot.png")), "{needle}");
    }
    // a body past the cap is refused unread, whatever the route
    for s in [&ruby, &rust] {
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", s.port)).unwrap();
        use std::io::{Read, Write};
        write!(stream, "POST /api/drop?name=big.bin HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nOrigin: http://127.0.0.1:{}\r\nContent-Length: 33554433\r\nConnection: close\r\n\r\n", s.port, s.port).unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 413 Content Too Large") && reply.contains("dropped file is over 32 MB"), "{reply}");
    }
}

fn ws_handshake(port: u16, route_query: &str, host: Option<String>, origin: Option<String>, key: Option<&str>) -> (std::net::TcpStream, String) {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
    let mut req = format!("GET {route_query} HTTP/1.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n");
    if let Some(h) = host {
        req += &format!("Host: {h}\r\n");
    }
    if let Some(o) = origin {
        req += &format!("Origin: {o}\r\n");
    }
    if let Some(k) = key {
        req += &format!("Sec-WebSocket-Key: {k}\r\nSec-WebSocket-Version: 13\r\n");
    }
    req += "\r\n";
    stream.write_all(req.as_bytes()).unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).unwrap_or(0) == 0 {
            break;
        }
        head.push(byte[0]);
    }
    let mut text = String::from_utf8_lossy(&head).into_owned();
    if text.starts_with("HTTP/1.1 403") {
        let mut rest = String::new();
        let _ = stream.read_to_string(&mut rest);
        text += &rest;
    }
    (stream, text)
}

fn ws_send(stream: &mut std::net::TcpStream, opcode: u8, payload: &[u8], masked: bool) {
    use std::io::Write;
    let mut frame = vec![0x80 | opcode];
    let mask = [0x12u8, 0x34, 0x56, 0x78];
    let flag = if masked { 0x80 } else { 0 };
    if payload.len() < 126 {
        frame.push(flag | payload.len() as u8);
    } else {
        frame.push(flag | 126);
        frame.extend((payload.len() as u16).to_be_bytes());
    }
    if masked {
        frame.extend(mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    } else {
        frame.extend(payload);
    }
    stream.write_all(&frame).unwrap();
}

/// What the server sent within `wait` milliseconds: (opcode, payload) frames.
fn ws_read(stream: &mut std::net::TcpStream, wait: u64) -> Vec<(u8, Vec<u8>)> {
    use std::io::Read;
    stream.set_read_timeout(Some(std::time::Duration::from_millis(wait))).unwrap();
    let mut frames = vec![];
    loop {
        let mut head = [0u8; 2];
        if stream.read_exact(&mut head).is_err() {
            return frames;
        }
        let mut len = (head[1] & 0x7f) as usize;
        if len == 126 {
            let mut b = [0u8; 2];
            stream.read_exact(&mut b).unwrap();
            len = u16::from_be_bytes(b) as usize;
        } else if len == 127 {
            let mut b = [0u8; 8];
            stream.read_exact(&mut b).unwrap();
            len = u64::from_be_bytes(b) as usize;
        }
        let mut data = vec![0u8; len];
        stream.read_exact(&mut data).unwrap();
        frames.push((head[0] & 0x0f, data));
    }
}

fn plain_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes).into_owned();
    regex::Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b[()][0-9A-Za-z]|\x1b[=>]|\x1b\][^\x07]*\x07|\r").unwrap().replace_all(&text, "").into_owned()
}

#[test]
fn the_terminal_websocket_matches_ruby() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let socks = ["qm-test-ws-term-0", "qm-test-ws-term-1"];
    let root2 = base.path().join("root2");
    copy_dir(&w.root, &root2);
    let env_for = |sock: &str| -> Vec<(&'static str, String)> { vec![("HOME", w.home.display().to_string()), ("QM_TMUX_SOCKET", sock.to_string())] };
    for s in socks {
        tmux_on(s, &["kill-server"]);
        for _ in 0..50 {
            tmux_on(s, &["new-session", "-d", "-s", "chat", "-x", "80", "-y", "24", "sh -c 'printf \"ready to type\\n\"; exec cat'"]);
            if tmux_on(s, &["list-sessions", "-F", "#{session_name}"]).contains("chat") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        tmux_on(s, &["set-option", "-g", "status", "off"]);
    }
    let ruby = start_env(false, &w.root, &w.home, &env_for(socks[0]));
    let rust = start_env(true, &root2, &w.home, &env_for(socks[1]));
    let token = |s: &Server| -> String { serde_json::from_slice::<serde_json::Value>(&request(s.port, "GET", "/api/state", &[], "", true).body).unwrap()["terminalToken"].as_str().unwrap().to_string() };
    let (ta, tb) = (token(&ruby), token(&rust));
    let servers = [(&ruby, ta), (&rust, tb)];
    let own = |s: &Server| (Some(format!("127.0.0.1:{}", s.port)), Some(format!("http://127.0.0.1:{}", s.port)));
    // what is refused, and how
    let key = "dGhlIHNhbXBsZSBub25jZQ==";
    let refused = |s: &Server, tok: &str, route: &str, host: Option<String>, origin: Option<String>, k: Option<&str>, session: &str| -> String {
        let (mut stream, text) = ws_handshake(s.port, &format!("{route}?token={tok}&session={session}&cols=80&rows=24"), host, origin, k);
        let _ = stream.shutdown(std::net::Shutdown::Both);
        text.replace(&s.port.to_string(), "PORT")
    };
    let mut seen = String::new();
    for (label, route, tokmod, host_ok, origin, k, session) in [
        ("another route", "/ws/other", "", true, "own", Some(key), "chat"),
        ("a wrong host", "/ws/terminal", "", false, "own", Some(key), "chat"),
        ("no origin", "/ws/terminal", "", true, "none", Some(key), "chat"),
        ("a foreign origin", "/ws/terminal", "", true, "foreign", Some(key), "chat"),
        ("a bad token", "/ws/terminal", "bad", true, "own", Some(key), "chat"),
        ("a prefix of the token", "/ws/terminal", "prefix", true, "own", Some(key), "chat"),
        ("a wrong host and no origin", "/ws/terminal", "", false, "none", Some(key), "chat"),
        ("a token of the right length", "/ws/terminal", "same-length", true, "own", Some(key), "chat"),
        ("no such chat", "/ws/terminal", "", true, "own", Some(key), "nowhere"),
        ("no key", "/ws/terminal", "", true, "own", None, "chat"),
    ] {
        let mut answers = vec![];
        for (s, tok) in &servers {
            let tok = match tokmod {
                "bad" => "x".to_string(),
                "same-length" => "y".repeat(tok.len()),
                "prefix" => tok[..10].to_string(),
                _ => tok.clone(),
            };
            let (host, origin_value) = own(s);
            let host = if host_ok { host } else { Some("evil.test".to_string()) };
            let origin_value = match origin {
                "none" => None,
                "foreign" => Some("http://evil.test".to_string()),
                _ => origin_value,
            };
            answers.push(refused(s, &tok, route, host, origin_value, k, session));
        }
        assert_eq!(answers[0], answers[1], "{label}");
        seen += &answers[0];
    }
    assert!(["not found", "bad host", "bad origin", "bad token", "no such chat", "missing websocket key"].iter().all(|n| seen.contains(n)), "{seen}");
    // a served connection: the handshake, what tmux draws, what is typed, a resize, a ping
    let connect = |s: &Server, tok: &str| ws_handshake(s.port, &format!("/ws/terminal?token={tok}&session=chat&cols=100&rows=30"), own(s).0, own(s).1, Some(key));
    let mut conns = vec![];
    for (s, tok) in &servers {
        let (stream, head) = connect(s, tok);
        assert!(head.starts_with("HTTP/1.1 101 Switching Protocols") && head.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo="), "{head}");
        conns.push(stream);
    }
    let drawn: Vec<String> = conns.iter_mut().map(|c| ws_read(c, 1500).iter().filter(|(op, _)| *op == 2).map(|(_, d)| plain_text(d)).collect::<String>()).collect();
    assert!(drawn[0].contains("ready to type") && drawn[1].contains("ready to type"), "{drawn:?}");
    for c in conns.iter_mut() {
        ws_send(c, 2, b"hello there\r", true);
    }
    let typed: Vec<String> = conns.iter_mut().map(|c| ws_read(c, 1500).iter().map(|(_, d)| plain_text(d)).collect::<String>()).collect();
    assert!(typed[0].contains("hello there") && typed[1].contains("hello there"), "{typed:?}");
    for c in conns.iter_mut() {
        ws_send(c, 1, b"{\"cols\":120,\"rows\":40}", true);
    }
    std::thread::sleep(std::time::Duration::from_millis(800));
    let clients = |sock: &str| tmux_on(sock, &["list-clients", "-F", "#{client_width}x#{client_height}"]).trim().to_string();
    assert_eq!(clients(socks[0]), clients(socks[1]), "the resize reaches tmux");
    assert!(clients(socks[1]).contains("120x40"), "{}", clients(socks[1]));
    for c in conns.iter_mut() {
        ws_send(c, 9, b"ping!", true);
    }
    let pongs: Vec<Vec<(u8, Vec<u8>)>> = conns.iter_mut().map(|c| ws_read(c, 1000).into_iter().filter(|(op, _)| *op == 10).collect()).collect();
    assert_eq!(pongs[0], pongs[1]);
    assert_eq!(pongs[0], vec![(10u8, b"ping!".to_vec())]);
    // an unmasked frame is a protocol error: the connection ends, and it is logged
    for c in conns.iter_mut() {
        ws_send(c, 2, b"x", false);
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    let log = |root: &Path| fs::read_to_string(root.join("qm.log")).unwrap_or_default().lines().map(|l| l.splitn(2, ' ').nth(1).unwrap_or("").to_string()).collect::<Vec<_>>();
    assert_eq!(log(&w.root), log(&root2));
    assert!(log(&root2).iter().any(|l| l.contains("[gui-terminal]") && l.contains("unmasked client frame")), "{:?}", log(&root2));
    // a close frame is answered and ends the connection; the chat lives on
    let mut closers = vec![];
    for (s, tok) in &servers {
        let (mut stream, _) = connect(s, tok);
        ws_read(&mut stream, 800);
        ws_send(&mut stream, 8, &1000u16.to_be_bytes(), true);
        closers.push(ws_read(&mut stream, 1000).into_iter().filter(|(op, _)| *op == 8).collect::<Vec<_>>());
    }
    assert_eq!(closers[0], closers[1]);
    assert_eq!(closers[0].len(), 1);
    for s in socks {
        assert!(tmux_on(s, &["list-sessions", "-F", "#{session_name}"]).contains("chat"), "the chat outlives its client");
        tmux_on(s, &["kill-server"]);
    }
}

fn pr_node(number: u32, repo: &str, head: &str, updated: &str, decision: Option<&str>, reviews: &[&str], contexts: serde_json::Value, draft: bool) -> serde_json::Value {
    serde_json::json!({
        "number": number, "title": format!("PR {number} title"), "url": format!("https://github.com/{repo}/pull/{number}"), "isDraft": draft, "updatedAt": updated,
        "headRefName": head, "baseRefName": "main", "reviewDecision": decision,
        "latestReviews": {"nodes": reviews.iter().map(|s| serde_json::json!({"state": s})).collect::<Vec<_>>()},
        "author": {"login": "octo"}, "repository": {"nameWithOwner": repo},
        "commits": {"nodes": [{"commit": {"statusCheckRollup": {"contexts": {"nodes": contexts}}}}]}
    })
}

fn run_node(name: &str, conclusion: &str, started: &str, suite_id: u64, suite_at: &str, slug: &str, workflow: &str) -> serde_json::Value {
    serde_json::json!({"name": name, "status": "COMPLETED", "conclusion": conclusion, "startedAt": started, "checkSuite": {"databaseId": suite_id, "createdAt": suite_at, "app": {"slug": slug}, "workflowRun": {"workflow": {"name": workflow}}}})
}

#[test]
fn the_prs_view_and_its_actions_match_ruby() {
    let base = tempfile::tempdir().unwrap();
    let (base_a, base_b) = (base.path().join("a"), base.path().join("b"));
    let (wa, wb) = (workspace_world(&base_a, false), workspace_world(&base_b, false));
    let now = chrono::Utc::now().timestamp();
    let nodes = |side: &Path| {
        let ok_suite = vec![
            run_node("build", "FAILURE", "2026-03-01T10:00:01Z", 1, "2026-03-01T10:00:00Z", "github-actions", "CI"),
            run_node("build", "CANCELLED", "2026-03-01T11:00:01Z", 2, "2026-03-01T11:00:00Z", "github-actions", "CI"),
            run_node("build", "SUCCESS", "2026-03-01T11:05:00Z", 2, "2026-03-01T11:00:00Z", "github-actions", "CI"),
            run_node("lint", "NEUTRAL", "2026-03-01T11:00:02Z", 2, "2026-03-01T11:00:00Z", "github-actions", "CI"),
            serde_json::json!({"context": "deploy/preview", "state": "PENDING", "createdAt": "2026-03-01T11:00:03Z"}),
        ];
        let _ = side;
        let a = pr_node(5, "acme/widgets", "feat-w", "2026-03-05T10:00:00Z", Some("APPROVED"), &[], serde_json::json!(ok_suite), false);
        let b = pr_node(6, "acme/widgets", "feat-nowhere", "2026-03-04T10:00:00Z", None, &["APPROVED", "CHANGES_REQUESTED"], serde_json::json!([]), true);
        let c = pr_node(7, "acme/other", "other-branch", "2026-03-06T10:00:00Z", None, &["APPROVED"], serde_json::json!([run_node("t", "TIMED_OUT", "2026-03-01T12:00:00Z", 9, "2026-03-01T12:00:00Z", "x", "Y"), serde_json::Value::Null]), false);
        let d = pr_node(8, "acme/unknown", "u", "2026-03-03T10:00:00Z", None, &[], serde_json::json!([run_node("t", "SUCCESS", "2026-03-01T12:00:00Z", 9, "2026-03-01T12:00:00Z", "x", "Y"), run_node("u", "SKIPPED", "2026-03-01T12:00:01Z", 9, "2026-03-01T12:00:00Z", "x", "Y")]), false);
        (a, b, c, d)
    };
    let gh_dirs = [base_a.join("gh"), base_b.join("gh")];
    for (i, (w, b)) in [(&wa, &base_a), (&wb, &base_b)].iter().enumerate() {
        git(&w.repo, &["remote", "set-url", "origin", "git@github.com:acme/widgets.git"]);
        let other = b.join("other-repo");
        common::repo(&other, "main");
        git(&other, &["remote", "add", "origin", "https://github.com/acme/other.git"]);
        let config = fs::read_to_string(w.root.join("config.yml")).unwrap();
        fs::write(w.root.join("config.yml"), config.replace("  plain:", &format!("  other: {}\n  plain:", other.display()))).unwrap();
        fs::create_dir_all(&gh_dirs[i]).unwrap();
        executable(&w.bin.join("gh"), "#!/bin/sh\necho \"$*\" >> \"$FAKE_GH_LOG\"\ncase \"$1\" in\n  api)\n    case \"$2\" in\n      graphql) [ -f \"$FAKE_GH/graphql-fail\" ] && { echo \"graphql is down\" >&2; exit 1; }; cat \"$FAKE_GH/graphql.json\";;\n      */releases/latest) f=\"$FAKE_GH/release-$(echo \"${2#repos/}\" | cut -d/ -f1,2 | tr / _)\"; [ -f \"$f\" ] && cat \"$f\" || { echo \"no release\" >&2; exit 1; };;\n      */compare/*) f=\"$FAKE_GH/compare-${2##*...}\"; [ -f \"$f\" ] && cat \"$f\" || { echo \"no compare\" >&2; exit 1; };;\n    esac;;\n  pr) f=\"$FAKE_GH/pull-$(echo \"$3\" | tr '/:' '__')\"; [ -f \"$f\" ] && cat \"$f\" || { echo \"no pull\" >&2; exit 1; };;\nesac\n");
        let (n_a, n_b, n_c, n_d) = nodes(b);
        let graphql = serde_json::json!({"data": {"author": {"nodes": [n_a, n_b, serde_json::Value::Null, serde_json::json!({"title": "no url"})]}, "review": {"nodes": [n_c.clone()]}, "assignee": {"nodes": [n_c, n_d]}}});
        fs::write(gh_dirs[i].join("graphql.json"), graphql.to_string()).unwrap();
        // the cache the page starts from: fresh, with two of the PRs
        let (c_a, c_b, ..) = nodes(b);
        let merged = quartermaster::pr_inbox::merge(&[("author", vec![c_a, c_b])]);
        fs::write(w.root.join("pr-inbox.json"), serde_json::json!({"prs": merged, "ts": now - 30, "error": null}).to_string()).unwrap();
        // two items: one holds the first PR's branch, one is its review; prerequisites and their deploy cache
        fs::write(w.root.join("work-items/review-widgets-6-old-title.md"), "---\nv: 1\ntitle: 'Review widgets#6: old title'\nstate: resolved\nproject: demo\ncreated: '2026-01-01'\n---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n").unwrap();
        fs::write(w.root.join("pr-prerequisites.json"), serde_json::json!({"v": 1, "prs": {"https://github.com/acme/widgets/pull/5": [
            {"kind": "pr", "ref": "https://github.com/acme/widgets/pull/9", "repo": "acme/widgets"},
            {"kind": "commit", "ref": "abc1234", "repo": "acme/widgets"},
            {"kind": "pr", "ref": "https://github.com/acme/widgets/pull/10", "repo": "acme/widgets"}
        ]}}).to_string()).unwrap();
        fs::write(w.root.join("pr-deployment.json"), serde_json::json!({"v": 1, "releases": {"acme/widgets": {"tag": "v1.2", "ts": now - 10}}, "pulls": {
            "https://github.com/acme/widgets/pull/5": {"state": "OPEN", "mergeCommit": null, "ts": now - 10},
            "https://github.com/acme/widgets/pull/9": {"state": "MERGED", "mergeCommit": "def5678", "ts": now - 10},
            "https://github.com/acme/widgets/pull/10": {"state": "OPEN", "mergeCommit": null, "ts": now - 10}
        }, "verdicts": {"def5678": {"tag": "v1.2", "deployed": true, "ts": now - 10}, "abc1234": {"tag": "v1.1", "deployed": false, "ts": now - 10}}}).to_string()).unwrap();
        let _ = b;
    }
    let logs = [base_a.join("gh.log"), base_b.join("gh.log")];
    let chat = base.path().join("fake-chat");
    executable(&chat, "#!/bin/sh\nexec sleep 300\n");
    let env_for = |w: &common::AttentionWorld, side: usize| -> Vec<(&'static str, String)> {
        let base = if side == 0 { &base_a } else { &base_b };
        vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("PATH", format!("{}:{}:{}", w.bin.display(), base.join("tools").display(), std::env::var("PATH").unwrap())), ("FAKE_GH", gh_dirs[side].display().to_string()), ("FAKE_GH_LOG", logs[side].display().to_string()), ("FAKE_FAIL", base.join("fail").display().to_string()), ("FAKE_TOOL_LOG", base.join("tool.log").display().to_string()), ("QM_TMUX_SOCKET", format!("qm-test-prs-{side}")), ("QM_CHAT_COMMAND", chat.display().to_string()), ("QM_WARP_TAB_CONFIGS", base.join("warp").display().to_string())]
    };
    for side in 0..2 {
        tmux_on(&format!("qm-test-prs-{side}"), &["kill-server"]);
        fs::create_dir_all(if side == 0 { base_a.join("fail") } else { base_b.join("fail") }).unwrap();
    }
    for w in [&wa, &wb] {
        // an item on the first PR's branch, with a chat pane
        fs::write(w.root.join("work-items/w-pr.md"), "---\nv: 1\ntitle: w-pr\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\nbranches:\n- feat-w\n---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n").unwrap();
    }
    let ruby = start_env(false, &wa.root, &wa.home, &env_for(&wa, 0));
    let rust = start_env(true, &wb.root, &wb.home, &env_for(&wb, 1));
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let norm = |text: String, side: usize| regex::Regex::new(r#""fetchedAt":"[^"]*""#).unwrap().replace_all(&text, "\"fetchedAt\":\"T\"").into_owned().replace("\"prerequisitesRefreshing\":true", "\"prerequisitesRefreshing\":false").replace(&(if side == 0 { &base_a } else { &base_b }).display().to_string(), "BASE");
    let get = |path: &str| -> (String, String) {
        let (a, b) = (request(ruby.port, "GET", path, &[], "", true), request(rust.port, "GET", path, &[], "", true));
        (norm(shown(&a, ruby.port), 0), norm(shown(&b, rust.port), 1))
    };
    let settle = || {
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let (a, b) = get("/api/prs");
            if [&a, &b].iter().all(|t| !t.contains("\"refreshing\":true") && !t.contains("\"prerequisitesRefreshing\":true")) {
                return;
            }
        }
    };
    let (a, b) = get("/api/prs");
    assert_eq!(a, b, "the cached list");
    for needle in ["\"label\":\"widgets#5\"", "\"project\":\"plain\"", "\"item\":\"w-plain\"", "\"reviewItem\":\"review-widgets-6-old-title\"", "\"state\":\"deployed\"", "\"state\":\"unmerged\"", "\"state\":\"pending\"", "\"reviewSkill\":\"/code-review\"", "\"worktree\":\"BASE/wt-w\"", "\"ci\":{\"state\":\"PENDING\",\"ok\":2,\"total\":3}", "\"reviewDecision\":\"CHANGES_REQUESTED\"", "\"ci\":null"] {
        assert!(a.contains(needle), "the world should reach {needle}: {a}");
    }
    // a forced refresh answers at once and the list follows; the roles merge; the checks are counted as GitHub counts them
    let (a, b) = get("/api/prs?refresh=1");
    assert_eq!(a, b, "a forced refresh");
    assert!(a.contains("\"refreshing\":true"), "{a}");
    settle();
    let (a, b) = get("/api/prs");
    assert_eq!(a, b, "after the refresh");
    for needle in ["\"roles\":[\"review\",\"assignee\"]", "\"widgets#5\"", "\"project\":null", "\"state\":\"FAILURE\"", "\"state\":\"SUCCESS\",\"ok\":2,\"total\":2", "\"refreshing\":false", "\"error\":null"] {
        assert!(a.contains(needle), "the refreshed list should show {needle}: {a}");
    }
    let cache = |w: &common::AttentionWorld| regex::Regex::new(r#""ts":\d+"#).unwrap().replace_all(&fs::read_to_string(w.root.join("pr-inbox.json")).unwrap(), "\"ts\":T").into_owned();
    assert_eq!(cache(&wa), cache(&wb));
    // a failing refresh keeps the last list and says why
    for d in &gh_dirs {
        fs::write(d.join("graphql-fail"), "").unwrap();
    }
    get("/api/prs?refresh=1");
    settle();
    let (a, b) = get("/api/prs");
    assert_eq!(a, b, "after a failed refresh");
    assert!(a.contains("gh api graphql failed: graphql is down") && a.contains("\"widgets#5\""), "{a}");
    for d in &gh_dirs {
        fs::remove_file(d.join("graphql-fail")).unwrap();
        fs::write(d.join("graphql.json"), "{\"errors\":[{\"message\":\"timed out\"},{\"message\":\"again\"}],\"data\":null}").unwrap();
    }
    get("/api/prs?refresh=1");
    settle();
    let (a, b) = get("/api/prs");
    assert_eq!(a, b, "after a GraphQL error");
    assert!(a.contains("gh api graphql: timed out; again"), "{a}");
    // the prerequisites refresh asks GitHub read-only and prunes what it confirms merged
    let (a, b) = get("/api/prs");
    assert_eq!(a, b);
    settle();
    let (a, b) = get("/api/prs");
    assert_eq!(a, b, "after the deployment refresh");
    let deploy = |w: &common::AttentionWorld| regex::Regex::new(r#""ts": \d+"#).unwrap().replace_all(&fs::read_to_string(w.root.join("pr-deployment.json")).unwrap(), "\"ts\": T").into_owned();
    assert_eq!(deploy(&wa), deploy(&wb));
    let ghlog = |side: usize| { let mut l: Vec<String> = fs::read_to_string(&logs[side]).unwrap_or_default().lines().map(String::from).collect(); l.sort(); l };
    assert_eq!(ghlog(0), ghlog(1), "what each server asked GitHub");
    // the actions on a PR, by its URL
    for d in &gh_dirs {
        fs::write(d.join("graphql.json"), serde_json::json!({"data": {"author": {"nodes": [pr_node(5, "acme/widgets", "feat-w", "2026-03-05T10:00:00Z", None, &[], serde_json::json!([]), false), pr_node(6, "acme/widgets", "feat-nowhere", "2026-03-04T10:00:00Z", None, &[], serde_json::json!([]), false), pr_node(7, "acme/other", "other-branch", "2026-03-03T10:00:00Z", None, &[], serde_json::json!([]), false), pr_node(8, "acme/unknown", "u", "2026-03-02T10:00:00Z", None, &[], serde_json::json!([]), false)]}}}).to_string()).unwrap();
    }
    get("/api/prs?refresh=1");
    settle();
    let post = |body: &str| -> (String, String) {
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, body, true), request(rust.port, "POST", "/api/action", &origin, body, true));
        (norm(shown(&a, ruby.port), 0), norm(shown(&b, rust.port), 1))
    };
    let act = |cmd: &str, url: &str, extra: &str| format!("{{\"cmd\":\"{cmd}\",\"slug\":null,\"args\":{{\"url\":\"{url}\"{extra}}}}}");
    let pr = |n: u32, repo: &str| format!("https://github.com/{repo}/pull/{n}");
    let mut finished = |label: &str| {
        let (mut x, mut y): (Vec<String>, Vec<String>) = (vec![], vec![]);
        for _ in 0..200 {
            let state = |s: &Server| -> serde_json::Value { serde_json::from_slice(&request(s.port, "GET", "/api/state", &[], "", true).body).unwrap() };
            let (sa, sb) = (state(&ruby), state(&rust));
            x.extend(sa["finished"].as_array().unwrap().iter().map(|v| norm(v.to_string(), 0)));
            y.extend(sb["finished"].as_array().unwrap().iter().map(|v| norm(v.to_string(), 1)));
            if sa["jobs"].as_array().is_some_and(|j| j.is_empty()) && sb["jobs"].as_array().is_some_and(|j| j.is_empty()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(x, y, "{label}");
        x.join("\n")
    };
    for (label, body) in [
        ("an unknown PR", act("pr-chat", "https://github.com/acme/widgets/pull/99", "")),
        ("a PR's chat", act("pr-chat", &pr(5, "acme/widgets"), "")),
        ("a PR's chat again", act("pr-chat", &pr(5, "acme/widgets"), "")),
        ("a chat for a PR with no item", act("pr-chat", &pr(6, "acme/widgets"), "")),
        ("a chat for a PR of no project", act("pr-chat", &pr(8, "acme/unknown"), "")),
        ("a review", act("pr-review", &pr(7, "acme/other"), "")),
        ("the review again", act("pr-review", &pr(7, "acme/other"), "")),
        ("a review whose item was resolved", act("pr-review", &pr(6, "acme/widgets"), "")),
        ("a PR that is no project", act("pr-pf", &pr(8, "acme/unknown"), "")),
        ("an IDE for a PR that is no project", act("pr-ide", &pr(8, "acme/unknown"), "")),
        ("an unknown prerequisite", act("pr-unrequire", &pr(5, "acme/widgets"), ",\"ref\":\"nothing\"")),
        ("a prerequisite", act("pr-unrequire", &pr(5, "acme/widgets"), ",\"ref\":\"abc1234\"")),
    ] {
        let (a, b) = post(&body);
        assert_eq!(a, b, "{label}: reply to {body}");
    }
    for (label, body) in [("the IDE on a PR's branch", act("pr-ide", &pr(5, "acme/widgets"), "")), ("a worktree to create for a PR", act("pr-ide", &pr(6, "acme/widgets"), ""))] {
        let (a, b) = post(&body);
        assert_eq!(a, b, "{label}");
        finished(label);
    }
    let items = |w: &common::AttentionWorld, side: usize| {
        let mut t = String::new();
        tree_text(&w.root.join("work-items"), "items", &mut t);
        tree_text(&w.root.join("pr-prerequisites.json").parent().unwrap().join("."), "x", &mut String::new());
        norm(format!("{t}{}", fs::read_to_string(w.root.join("pr-prerequisites.json")).unwrap_or_default()), side)
    };
    assert_eq!(items(&wa, 0), items(&wb, 1), "the items and prerequisites the actions left");
    let panes = |side: usize| tmux_on(&format!("qm-test-prs-{side}"), &["list-sessions", "-F", "#{session_name}"]);
    assert_eq!(panes(0), panes(1));
    for side in 0..2 {
        tmux_on(&format!("qm-test-prs-{side}"), &["kill-server"]);
    }
}

#[test]
fn the_pr_threads_route_matches_ruby() {
    let base = tempfile::tempdir().unwrap();
    let dirs = [base.path().join("a"), base.path().join("b")];
    let page1 = serde_json::json!({"data": {"viewer": {"login": "me"}, "repository": {"pullRequest": {"id": "PR_1",
        "reviews": {"nodes": [null, {"id": "R_other", "viewerDidAuthor": false}, {"id": "R_mine", "viewerDidAuthor": true}]},
        "reviewThreads": {"pageInfo": {"hasNextPage": true, "endCursor": "CUR1"}, "nodes": [
            {"id": "T1", "path": "a.rs", "line": 5, "originalLine": 5, "startLine": 3, "diffSide": "RIGHT", "subjectType": "LINE", "isResolved": false, "isOutdated": false,
             "viewerCanResolve": true, "viewerCanUnresolve": false, "viewerCanReply": true, "resolvedBy": null,
             "comments": {"nodes": [{"id": "C1", "body": "first", "createdAt": "2026-03-01T10:00:00Z", "url": "u1", "state": "SUBMITTED", "diffHunk": "@@ -1 +1 @@\n-old\n+new line\n\n", "author": {"login": "octo"}}, null,
                                    {"id": "C2", "body": null, "createdAt": "2026-03-01T11:00:00Z", "url": "u2", "state": "PENDING", "diffHunk": "", "author": null}]}},
            null,
            {"id": "T2", "path": "b.rs", "line": null, "originalLine": 9, "startLine": null, "diffSide": "LEFT", "subjectType": "FILE", "isResolved": true, "isOutdated": true,
             "viewerCanResolve": false, "viewerCanUnresolve": true, "viewerCanReply": false, "resolvedBy": {"login": "boss"}, "comments": {"nodes": []}},
            {"id": "T3", "path": "c.rs", "diffSide": null, "isResolved": "yes", "comments": null}
        ]}}}}});
    let page2 = serde_json::json!({"data": {"viewer": {"login": "me"}, "repository": {"pullRequest": {"id": "PR_1", "reviews": {"nodes": []},
        "reviewThreads": {"pageInfo": {"hasNextPage": false, "endCursor": "CUR2"}, "nodes": [{"id": "T4", "path": "d.rs", "line": 1, "diffSide": "RIGHT", "comments": {"nodes": [{"id": "C4", "body": "late", "diffHunk": "x\ny", "author": {"login": "z"}}]}}]}}}}});
    let mut env = vec![];
    for (i, d) in dirs.iter().enumerate() {
        fs::create_dir_all(d.join("bin")).unwrap();
        fs::create_dir_all(d.join("gh")).unwrap();
        fs::create_dir_all(d.join("home")).unwrap();
        fs::write(d.join("gh/page1.json"), page1.to_string()).unwrap();
        fs::write(d.join("gh/page2.json"), page2.to_string()).unwrap();
        fs::write(d.join("gh/notfound.json"), "{\"data\":{\"viewer\":{\"login\":\"me\"},\"repository\":{\"pullRequest\":null}}}").unwrap();
        fs::write(d.join("gh/errors.json"), "{\"errors\":[{\"message\":\"bad one\"},{\"nope\":1},{\"message\":\"bad two\"}],\"data\":null}").unwrap();
        fs::write(d.join("gh/junk.json"), "this is not json\n").unwrap();
        executable(&d.join("bin/gh"), "#!/bin/sh\necho \"$*\" | sed 's/query=.*//' >> \"$FAKE_GH_LOG\"\ncase \"$*\" in\n  *number=404*) cat \"$FAKE_GH/notfound.json\";;\n  *number=405*) cat \"$FAKE_GH/errors.json\";;\n  *number=406*) cat \"$FAKE_GH/junk.json\";;\n  *number=407*) echo \"boom on stderr\" >&2; exit 1;;\n  *number=408*) echo \"only stdout\"; exit 1;;\n  *cursor=CUR1*) cat \"$FAKE_GH/page2.json\";;\n  *) cat \"$FAKE_GH/page1.json\";;\nesac\n");
        env.push(vec![("HOME", d.join("home").display().to_string()), ("PATH", format!("{}:{}", d.join("bin").display(), std::env::var("PATH").unwrap())), ("FAKE_GH", d.join("gh").display().to_string()), ("FAKE_GH_LOG", d.join("gh.log").display().to_string()), ("QM_TMUX_SOCKET", format!("qm-test-threads-{i}"))]);
    }
    let roots = [dirs[0].join("root"), dirs[1].join("root")];
    for r in &roots {
        fs::create_dir_all(r.join("work-items")).unwrap();
    }
    let ruby = start_env(false, &roots[0], &dirs[0].join("home"), &env[0]);
    let rust = start_env(true, &roots[1], &dirs[1].join("home"), &env[1]);
    let paths = [
        "/api/pr-threads", "/api/pr-threads?repo=nope&number=1", "/api/pr-threads?repo=acme/widgets", "/api/pr-threads?repo=acme/widgets&number=x",
        "/api/pr-threads?repo=acme/widgets&number=0", "/api/pr-threads?repo=acme/widgets&number=-3", "/api/pr-threads?repo=a/b/c&number=1", "/api/pr-threads?repo=a/b%0A&number=1",
        "/api/pr-threads?repo=acme/widgets&number=7&head=h1", "/api/pr-threads?repo=acme/widgets&number=7&head=h1", "/api/pr-threads?repo=ACME/Widgets&number=7&head=h1",
        "/api/pr-threads?repo=acme/widgets&number=%207%20&head=h2", "/api/pr-threads?repo=acme/widgets&number=+7&head=h3", "/api/pr-threads?repo=acme/widgets&number=0_7&head=h4",
        "/api/pr-threads?repo=acme/widgets&number=7&head=h1&fresh=1", "/api/pr-threads?repo=acme/widgets&number=7&head=h1&fresh=0",
        "/api/pr-threads?repo=acme/widgets&number=404", "/api/pr-threads?repo=acme/widgets&number=405", "/api/pr-threads?repo=acme/widgets&number=406",
        "/api/pr-threads?repo=acme/widgets&number=407", "/api/pr-threads?repo=acme/widgets&number=408", "/api/pr-threads?repo=acme/widgets&number=1_&head=q", "/api/pr-threads?repo=ac9me/w9&number=1__0", "/api/pr-threads?repo=ac9me/w9&number=_1", "/api/pr-threads?repo=ac9me/w9&number=7&head=d",
    ];
    for path in paths {
        let (a, b) = (request(ruby.port, "GET", path, &[], "", true), request(rust.port, "GET", path, &[], "", true));
        let (a, b) = (shown(&a, ruby.port), shown(&b, rust.port));
        assert_eq!(a, b, "{path}");
    }
    let (a, b) = (request(ruby.port, "GET", "/api/pr-threads?repo=acme/widgets&number=7&head=h1&fresh=1", &[], "", true), request(rust.port, "GET", "/api/pr-threads?repo=acme/widgets&number=7&head=h1&fresh=1", &[], "", true));
    let body = String::from_utf8_lossy(&b.body).to_string();
    assert_eq!(a.body, b.body);
    for needle in ["\"ghost\"", "\"side\":\"old\"", "\"file\":true", "\"quote\":\"+new line\"", "\"quote\":null", "\"resolvedBy\":\"boss\"", "\"id\":\"T4\"", "\"quote\":\"y\"", "\"pendingReview\":null"] {
        assert!(body.contains(needle), "{needle} in {body}");
    }
    assert_eq!(fs::read_to_string(dirs[0].join("gh.log")).unwrap(), fs::read_to_string(dirs[1].join("gh.log")).unwrap(), "the gh calls");
}

#[test]
fn drafts_send_and_issues_create_paste_the_same_messages_as_ruby() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let issue = |slug: &str, name: &str, front: &str, body: &str| {
        let p = w.root.join(format!("issues/{slug}/{name}"));
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, format!("---\n{front}---\n\n{body}\n")).unwrap();
    };
    issue("stale", "01-bug.md", "title: A bug\nkind: bug\nnumber: 12\n", "first block\n\nsecond block\nthird line");
    issue("stale", "02-feature.md", "title: A feature\nkind: feature\n", "feature body with a block of text");
    issue("stale", "03-created.md", "title: Created too\nkind: feature\nnumber: 7\n", "body three");
    issue("stale", "04-broken.md", "title: Broken\nkind: epic\n", "bad kind");
    fs::write(w.root.join("issues/stale/05-nofront.md"), "just text\n").unwrap();
    let root2 = base.path().join("root2");
    copy_dir(&w.root, &root2);
    let sockets = ["qm-test-send-0", "qm-test-send-1"];
    let outs = [base.path().join("out0"), base.path().join("out1")];
    let roots = [w.root.clone(), root2.clone()];
    for i in 0..2 {
        tmux_on(sockets[i], &["kill-server"]);
        fs::write(&outs[i], "").unwrap();
        tmux_on(sockets[i], &["new-session", "-d", "-s", "stale", "-x", "100", "-y", "30", &format!("cat >> {}", outs[i].display())]);
    }
    let env = |i: usize| -> Vec<(&'static str, String)> { vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("QM_TMUX_SOCKET", sockets[i].to_string())] };
    let ruby = start_env(false, &roots[0], &w.home, &env(0));
    let rust = start_env(true, &roots[1], &w.home, &env(1));
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let long_html = "<div class=\"x\">".to_string() + &"y".repeat(700);
    let drafts = serde_json::json!([
        {"id": "da", "kind": "diff-comment", "comment": " A  ", "rev": 1, "path": "b.rs", "side": "new", "line": 10, "code": "x\ny\n", "branch": "feat"},
        {"id": "db", "kind": "diff-comment", "comment": "B", "rev": 1, "path": "a.rs", "side": "new", "line": "3", "start_line": 1, "start_side": "old", "range": "abc1234..def5678"},
        {"id": "dc", "kind": "diff-comment", "comment": "C", "rev": 2, "path": "a.rs", "side": "old", "line": 2, "committed": true, "code": "old line"},
        {"id": "dd", "kind": "diff-comment", "comment": "D", "rev": 1, "path": "c.rs", "side": "new", "line": 7, "repo": "acme/widgets", "number": 5, "commit": "abcdef0123", "code": "\n\n"},
        {"id": "de", "kind": "diff-comment", "comment": "E", "rev": 1, "path": "a.rs", "side": "new", "line": "0x10", "code": "  indented \r\n", "range": "abcd", "start_line": 12, "start_side": "new", "branch": "other"},
        {"id": "df", "kind": "diff-comment", "comment": "F", "rev": 1, "path": "a.rs", "side": "new", "line": 1, "branch": "feat", "committed": false},
        {"id": "an1", "kind": "annotation", "comment": "move this", "rev": 1, "page": "/other", "url": "http://h:3000/x?y=1#f", "title": "A  title", "selector": "div > a", "owners": [{"name": "Card", "as": "Tile", "file": "app/javascript/card.jsx", "line": 12, "via": "definition"}, {"name": "Page", "file": "app/javascript/page.jsx", "line": 0}, {"name": ""}, "junk", {"name": "Lone"}], "context": "ctx   text", "regions": "header"},
        {"id": "an2", "kind": "annotation", "comment": "resize", "rev": 1, "page": "", "url": "http://h:3000/x?y=1#f", "title": "A  title", "components": ["Card", " Tile ", 5, ""], "text": "t".repeat(250), "html": long_html},
        {"id": "an3", "kind": "annotation", "comment": "plain", "rev": 1, "url": "http://h:3000/x?y=1#f", "title": "", "component": "Lone  Comp"},
        {"id": "an4", "kind": "annotation", "comment": "elsewhere", "rev": 1, "page": "/third", "url": "http://h", "title": "T"},
        {"id": "an5", "kind": "annotation", "comment": "bad url", "rev": 1, "page": "/p", "url": "not a url", "title": "T"},
        {"id": "an6", "kind": "annotation", "comment": "no title", "rev": 1, "url": "", "title": ""},
        {"id": "an7", "kind": "annotation", "comment": "limits", "rev": 1, "page": "p".repeat(301), "url": "http://h:3000/x", "title": "T", "owners": [{"name": "n".repeat(81), "as": "a".repeat(81), "file": "f".repeat(201), "line": 3}], "context": "c".repeat(301), "regions": "r".repeat(201)},
        {"id": "an8", "kind": "annotation", "comment": "chain", "rev": 1, "page": "p".repeat(300), "url": "http://h:3000/x", "title": "T", "components": ["k".repeat(81), "m".repeat(80), "z".repeat(80), "y".repeat(80)], "text": "t".repeat(200), "html": "h".repeat(601)},
        {"id": "an9", "kind": "annotation", "comment": "exact", "rev": 1, "page": "p".repeat(300), "url": "http://h:3000/x", "title": "T", "components": ["k".repeat(80)], "text": "t".repeat(200), "html": "h".repeat(600), "context": "c".repeat(300), "regions": "r".repeat(200), "selector": "s"},
        {"id": "i1", "kind": "issue-comment", "comment": "on first", "rev": 1, "file": "01-bug.md", "block": "second block\nthird line", "title": "A bug"},
        {"id": "i2", "kind": "issue-comment", "comment": "on a changed block", "rev": 1, "file": "01-bug.md", "block": "gone\n\ntext"},
        {"id": "i3", "kind": "issue-comment", "comment": "whole", "rev": 1, "file": "01-bug.md", "block": ""},
        {"id": "i4", "kind": "issue-comment", "comment": "on a gone file", "rev": 1, "file": "99-gone.md", "block": "b", "title": "Old title"},
        {"id": "i5", "kind": "issue-comment", "comment": "on a broken one", "rev": 1, "file": "04-broken.md", "block": "bad"},
        {"id": "i6", "kind": "issue-comment", "comment": "first block", "rev": 1, "file": "01-bug.md", "block": "first block"},
        {"id": "g1", "kind": "github-comment", "comment": "review", "rev": 1, "path": "a.rs", "line": 1, "repo": "acme/widgets", "number": 5, "commit": "abcdef0"},
        {"id": "t1", "kind": "thread-reply", "comment": "reply", "rev": 1, "thread": "T1", "repo": "acme/widgets", "number": 5}
    ]);
    let reset = |ids: &[&str]| {
        let entries: Vec<serde_json::Value> = drafts.as_array().unwrap().iter().filter(|e| ids.is_empty() || ids.contains(&e["id"].as_str().unwrap())).cloned().collect();
        for r in &roots {
            fs::create_dir_all(r.join("drafts")).unwrap();
            fs::write(r.join("drafts/stale.json"), serde_json::to_string_pretty(&serde_json::json!({"v": 1, "entries": entries})).unwrap()).unwrap();
        }
    };
    let act = |cmd: &str, slug: &str, args: &str| format!("{{\"cmd\":\"{cmd}\",\"slug\":\"{slug}\",\"args\":{args}}}");
    let seen = |i: usize| -> String { fs::read_to_string(&outs[i]).unwrap_or_default().replace(&roots[i].display().to_string(), "ROOT") };
    let mut step = |label: &str, body: &str| {
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, body, true), request(rust.port, "POST", "/api/action", &origin, body, true));
        std::thread::sleep(std::time::Duration::from_millis(150));
        let (sa, sb) = (shown(&a, ruby.port).replace(&roots[0].display().to_string(), "ROOT"), shown(&b, rust.port).replace(&roots[1].display().to_string(), "ROOT"));
        assert_eq!(sa, sb, "{label}: reply to {body}");
        assert_eq!(seen(0), seen(1), "{label}: what the chat got after {body}");
        let files = |r: &Path| fs::read_to_string(r.join("drafts/stale.json")).unwrap_or_default();
        assert_eq!(files(&roots[0]), files(&roots[1]), "{label}: the drafts after {body}");
        let log = |r: &Path| regex::Regex::new(r"(?m)^\S+ \[").unwrap().replace_all(&fs::read_to_string(r.join("qm.log")).unwrap_or_default(), "[").replace(&r.display().to_string(), "ROOT");
        assert_eq!(log(&roots[0]), log(&roots[1]), "{label}: qm.log after {body}");
    };
    reset(&[]);
    step("missing item", &act("drafts-send", "nowhere", "{}"));
    step("no chat open", &act("drafts-send", "degrade", "{}"));
    step("empty ids", &act("drafts-send", "stale", r#"{"ids":[]}"#));
    step("only review drafts", &act("drafts-send", "stale", r#"{"ids":["g1","t1"]}"#));
    for (label, ids, extra) in [
        ("one comment on a branch", r#"["da"]"#, r#","branch":"feat","base":"main""#),
        ("two comments, ranges", r#"["db","dc"]"#, r#","branch":"feat","base":"main""#),
        ("a PR comment and a hex line", r#"["dd","de","df"]"#, r#","branch":"feat""#),
        ("same branch for all", r#"["da","df"]"#, r#","branch":" feat ","base":" main ""#),
        ("no branch", r#"["dc"]"#, ""),
        ("odd branch type", r#"["dc"]"#, r#","branch":7,"base":null"#),
        ("annotations on other pages", r#"["an1","an2"]"#, ""),
        ("one annotation", r#"["an3"]"#, ""),
        ("limits", r#"["an7"]"#, ""),
        ("a long chain", r#"["an8","an9"]"#, ""),
        ("a bare url", r#"["an4"]"#, ""),
        ("an invalid url", r#"["an5"]"#, ""),
        ("no url", r#"["an6"]"#, ""),
        ("issue comments", r#"["i1","i2","i3","i4","i5","i6"]"#, ""),
        ("everything", "null", r#","branch":"feat","base":"main""#),
    ] {
        reset(&[]);
        step(label, &act("drafts-send", "stale", &format!("{{\"ids\":{ids}{extra}}}")));
    }
    reset(&["da", "an3", "i1"]);
    step("mixed kinds", &act("drafts-send", "stale", r#"{"branch":"b"}"#));
    step("nothing left", &act("drafts-send", "stale", "{}"));
    reset(&[]);
    for (label, files) in [
        ("create all", "null"),
        ("create chosen", r#"["02-feature.md"]"#),
        ("create repeated", r#"["02-feature.md","02-feature.md"]"#),
        ("create none chosen", "[]"),
        ("create a created one", r#"["01-bug.md"]"#),
        ("create a broken one", r#"["04-broken.md"]"#),
        ("create a missing one", r#"["zzz.md"]"#),
        ("create a number", "[5]"),
        ("create from a string", r#""02-feature.md""#),
    ] {
        step(label, &act("issues-create", "stale", &format!("{{\"files\":{files}}}")));
    }
    step("create without a chat", &act("issues-create", "degrade", r#"{"files":null}"#));
    step("create on a missing item", &act("issues-create", "nowhere", r#"{"files":null}"#));
    tmux_on(sockets[0], &["kill-server"]);
    tmux_on(sockets[1], &["kill-server"]);
}

#[test]
fn page_annotations_and_clipboard_loads_match_ruby() {
    let base = tempfile::tempdir().unwrap();
    let bases = [base.path().join("a"), base.path().join("b")];
    let worlds = [workspace_world(&bases[0], false), workspace_world(&bases[1], false)];
    let sockets = ["qm-test-annot-0", "qm-test-annot-1"];
    let js = "app/javascript";
    for (i, w) in worlds.iter().enumerate() {
        let wt = bases[i].join("wt-w");
        fs::create_dir_all(wt.join(js)).unwrap();
        fs::write(wt.join(js).join("card.jsx"), "line one\r\nline two\n  <Card a=\"b\" />\nbad \u{ff}\u{fe} bytes\n").unwrap();
        let mut raw = b"ok \xff\xfe bad\n".to_vec();
        raw.extend_from_slice(b"cut \xe2\x82 end\n");
        raw.extend_from_slice("x".repeat(501).as_bytes());
        raw.extend_from_slice(b"\n");
        raw.extend_from_slice("y".repeat(500).as_bytes());
        raw.extend_from_slice(b"\nlast no newline");
        fs::write(wt.join(js).join("odd.jsx"), raw).unwrap();
        fs::write(wt.join(js).join("big.jsx"), format!("short\n{}", "z".repeat(512 * 1024 - 6 + 1))).unwrap();
        fs::write(wt.join(js).join("edge.jsx"), format!("short\n{}", "z".repeat(512 * 1024 - 6))).unwrap();
        fs::write(wt.join(js).join("wide.jsx"), format!("{}\nnext\n", "é".repeat(300))).unwrap();
        fs::write(wt.join("app/outside.jsx"), "secret\n").unwrap();
        std::os::unix::fs::symlink(wt.join("app/outside.jsx"), wt.join(js).join("link.jsx")).unwrap();
        fs::create_dir_all(wt.join(js).join("dir.jsx")).unwrap();
        tmux_on(sockets[i], &["kill-server"]);
        tmux_on(sockets[i], &["new-session", "-d", "-s", "w-exist", "-c", wt.to_str().unwrap(), "sleep 300"]);
        let _ = w;
    }
    let env = |i: usize| -> Vec<(&'static str, String)> { vec![("HOME", worlds[i].home.display().to_string()), ("QM_TRANSCRIPTS_DIR", worlds[i].transcripts.display().to_string()), ("QM_TMUX_SOCKET", sockets[i].to_string())] };
    let ruby = start_env(false, &worlds[0].root, &worlds[0].home, &env(0));
    let rust = start_env(true, &worlds[1].root, &worlds[1].home, &env(1));
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let norm = |text: String, i: usize| -> String {
        let t = text.replace(&bases[i].display().to_string(), "BASE");
        let t = regex::Regex::new(r#""(createdAt|updatedAt)":\s*"[^"]*""#).unwrap().replace_all(&t, "\"$1\":\"STAMP\"").into_owned();
        regex::Regex::new(r#"\b[0-9a-f]{12}\b"#).unwrap().replace_all(&t, "ID").into_owned()
    };
    let state = |i: usize| -> String {
        let mut out = String::new();
        tree_text(&worlds[i].root.join("drafts"), "drafts", &mut out);
        let log = fs::read_to_string(worlds[i].root.join("qm.log")).unwrap_or_default();
        out += &regex::Regex::new(r"(?m)^\S+ \[").unwrap().replace_all(&log, "[");
        norm(out, i)
    };
    let step = |label: &str, body: &str| {
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, body, true), request(rust.port, "POST", "/api/action", &origin, body, true));
        let (sa, sb) = (norm(shown(&a, ruby.port), 0), norm(shown(&b, rust.port), 1));
        assert_eq!(sa, sb, "{label}: reply to {body}");
        assert_eq!(state(0), state(1), "{label}: files after {body}");
    };
    let act = |cmd: &str, slug: &str, args: &serde_json::Value| serde_json::json!({"cmd": cmd, "slug": slug, "args": args}).to_string();
    let wt = |i: usize| bases[i].join("wt-w").display().to_string();
    let wt_a = wt(0);
    let _ = wt_a;
    let ann = |comment: &str| serde_json::json!({"comment": comment, "selector": "div", "text": "t", "owners": [{"name": "Card", "file": "app/javascript/card.jsx", "line": 3, "extra": 1}, "junk", {"name": "B", "code": "keep?"}], "stray": 1, "url": "http://other", "title": "other"});
    // page annotations relayed from a dev server, by the checkout it serves
    for (label, root_of, annotations) in [
        ("one annotation", Some("wt"), serde_json::json!([ann("move it")])),
        ("two, one blank", Some("wt"), serde_json::json!([ann("a"), ann("  "), "junk", {"selector": "x"}, ann("b")])),
        ("owners that aren't a list", Some("wt"), serde_json::json!([{"comment": "c", "owners": "x"}, {"comment": "d", "owners": null}])),
        ("trailing slash", Some("wt/"), serde_json::json!([ann("slashed")])),
        ("a dotted path", Some("wt/../wt-w"), serde_json::json!([ann("dotted")])),
        ("the stacked worktree", Some("st"), serde_json::json!([ann("stack")])),
        ("the main checkout, several items", Some("repo"), serde_json::json!([ann("main")])),
        ("no annotations", Some("wt"), serde_json::json!([])),
        ("all blank", Some("wt"), serde_json::json!([ann(" ")])),
        ("not a list", Some("wt"), serde_json::json!({"a": 1})),
        ("nowhere", Some("nowhere"), serde_json::json!([ann("x")])),
        ("blank root", Some("  "), serde_json::json!([ann("x")])),
        ("no root", None, serde_json::json!([ann("x")])),
    ] {
        for (i, w) in worlds.iter().enumerate() {
            let _ = (i, w);
        }
        let build = |i: usize| -> String {
            let root = match root_of {
                Some("wt") => serde_json::json!(wt(i)),
                Some("wt/") => serde_json::json!(format!("{}/", wt(i))),
                Some("wt/../wt-w") => serde_json::json!(format!("{}/../wt-w", wt(i))),
                Some("st") => serde_json::json!(bases[i].join("wt-st1").display().to_string()),
                Some("repo") => serde_json::json!(worlds[i].repo.display().to_string()),
                Some(other) => serde_json::json!(other),
                None => serde_json::Value::Null,
            };
            act("page-annotations", "", &serde_json::json!({"root": root, "annotations": annotations, "url": "http://dev:3000/p?q=1", "title": "A page"}))
        };
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, &build(0), true), request(rust.port, "POST", "/api/action", &origin, &build(1), true));
        assert_eq!(norm(shown(&a, ruby.port), 0), norm(shown(&b, rust.port), 1), "{label}");
        assert_eq!(state(0), state(1), "{label}: files");
    }
    // what a page overlay copied to the clipboard
    let block = |v: serde_json::Value| format!("```pf-annotations\n{v}\n```\n");
    let payload = |id: &str, annotations: serde_json::Value| serde_json::json!({"v": 1, "id": id, "url": "http://dev/p", "title": "T", "annotations": annotations});
    let owners = |file: &str, line: serde_json::Value| serde_json::json!([{"name": "C", "file": file, "line": line, "code": "ignored"}]);
    let load = |text: String| act("drafts-load", "w-exist", &serde_json::json!({"text": text}));
    step("a valid load", &load(format!("Some message\n\n{}", block(payload("p1", serde_json::json!([{"comment": "one", "owners": owners("app/javascript/card.jsx", serde_json::json!(1))}, {"comment": "two", "owners": owners("app/javascript/card.jsx", serde_json::json!(2))}, {"comment": "three", "owners": owners("app/javascript/card.jsx", serde_json::json!(3))}, {"comment": "four", "owners": owners("app/javascript/card.jsx", serde_json::json!(4))}, {"comment": "five", "owners": owners("app/javascript/card.jsx", serde_json::json!(5))}]))))));
    step("the same again", &load(block(payload("p1", serde_json::json!([{"comment": "one"}])))));
    step("odd sources", &load(block(payload("p2", serde_json::json!([{"comment": "a", "owners": owners("app/javascript/odd.jsx", serde_json::json!(1))}, {"comment": "b", "owners": owners("app/javascript/odd.jsx", serde_json::json!(2))}, {"comment": "c", "owners": owners("app/javascript/odd.jsx", serde_json::json!(3))}, {"comment": "d", "owners": owners("app/javascript/odd.jsx", serde_json::json!(4))}, {"comment": "e", "owners": owners("app/javascript/odd.jsx", serde_json::json!(5))}, {"comment": "f", "owners": owners("app/javascript/odd.jsx", serde_json::json!(6))}, {"comment": "g", "owners": owners("app/javascript/odd.jsx", serde_json::json!(0))}, {"comment": "h", "owners": owners("app/javascript/odd.jsx", serde_json::json!(-1))}])))));
    step("refused sources", &load(block(payload("p3", serde_json::json!([{"comment": "a", "owners": owners("app/javascript/big.jsx", serde_json::json!(1))}, {"comment": "b", "owners": owners("app/javascript/link.jsx", serde_json::json!(1))}, {"comment": "c", "owners": owners("app/javascript/dir.jsx", serde_json::json!(1))}, {"comment": "d", "owners": owners("app/javascript/../outside.jsx", serde_json::json!(1))}, {"comment": "d2", "owners": owners("app/javascript/edge.jsx", serde_json::json!(1))}, {"comment": "d3", "owners": owners("app/javascript/wide.jsx", serde_json::json!(1))}, {"comment": "e", "owners": owners("other/card.jsx", serde_json::json!(1))}, {"comment": "f", "owners": owners("app/javascript/missing.jsx", serde_json::json!(1))}, {"comment": "g", "owners": [{"name": "x", "file": 5, "line": 1}, "junk", {"name": "y", "file": "app/javascript/card.jsx", "line": "1"}]}, {"comment": "h", "owners": "nope"}])))));
    for n in 0..52 {
        step(&format!("load {n}"), &load(block(payload(&format!("many{n}"), serde_json::json!([{"comment": format!("n{n}")}])))));
    }
    step("the evicted id again", &load(block(payload("many0", serde_json::json!([{"comment": "back"}])))));
    step("a recent id again", &load(block(payload("many51", serde_json::json!([{"comment": "no"}])))));
    step("the last block counts", &load(format!("{}\nquoted older copy above\n{}", block(payload("p4", serde_json::json!([{"comment": "old"}]))), block(payload("p5", serde_json::json!([{"comment": "new"}]))))));
    step("crlf fences", &load(format!("```pf-annotations  \r\n{}\r\n```  \r\n", payload("p6", serde_json::json!([{"comment": "crlf"}])))));
    step("no closing fence", &load(format!("```pf-annotations\n{}\n", payload("p7", serde_json::json!([{"comment": "open"}])))));
    step("a header in the text above", &load(format!("```pf-annotations\nnot json\n\n{}", block(payload("p8", serde_json::json!([{"comment": "ok"}]))))));
    step("no block", &load("nothing here".into()));
    step("a block without a fence line", &load("```pf-annotations x\n{}\n```\n".into()));
    step("not json", &load("```pf-annotations\n{oops\n```\n".into()));
    step("json but not an object", &load("```pf-annotations\n[1]\n```\n".into()));
    for v in ["2", "\"x\"", "null", "true", "1.0", "\"a\\nb\"", "7"] {
        step(&format!("version {v}"), &load(format!("```pf-annotations\n{{\"v\":{v},\"id\":\"vv{}\",\"annotations\":[{{\"comment\":\"c\"}}]}}\n```\n", v.len())));
    }
    step("no annotations", &load(block(payload("q1", serde_json::json!([])))));
    step("annotations not a list", &load(block(payload("q2", serde_json::json!({"a": 1})))));
    step("too many", &load(block(payload("q3", serde_json::json!((0..101).map(|i| serde_json::json!({"comment": format!("c{i}")})).collect::<Vec<_>>())))));
    step("a hundred", &load(block(payload("q4", serde_json::json!((0..100).map(|i| serde_json::json!({"comment": format!("c{i}")})).collect::<Vec<_>>())))));
    step("no id", &load(block(serde_json::json!({"v": 1, "annotations": [{"comment": "c"}]}))));
    step("blank id", &load(block(payload("  ", serde_json::json!([{"comment": "c"}])))));
    step("number id", &load(block(serde_json::json!({"v": 1, "id": 5, "annotations": [{"comment": "c"}]}))));
    step("all blank comments", &load(block(payload("q5", serde_json::json!([{"comment": " "}, "x"])))));
    step("an id with spaces", &load(block(payload("  q6  ", serde_json::json!([{"comment": "c"}])))));
    step("the same id, trimmed", &load(block(payload("q6", serde_json::json!([{"comment": "again"}])))));
    step("too much text", &load(format!("{}{}", "x".repeat(600 * 1024), block(payload("q7", serde_json::json!([{"comment": "c"}]))))));
    step("a number as text", &act("drafts-load", "w-exist", &serde_json::json!({"text": 5})));
    step("no text", &act("drafts-load", "w-exist", &serde_json::json!({})));
    step("a url that is a number", &load(block(serde_json::json!({"v": 1, "id": "q8", "url": 5, "title": null, "annotations": [{"comment": "c"}]}))));
    step("on an item without a chat", &act("drafts-load", "w-main", &serde_json::json!({"text": block(payload("r1", serde_json::json!([{"comment": "c", "owners": owners("app/javascript/card.jsx", serde_json::json!(1))}])))})));
    step("on a missing item", &act("drafts-load", "nowhere", &serde_json::json!({"text": block(payload("r2", serde_json::json!([{"comment": "c"}])))})));
    for s in sockets {
        tmux_on(s, &["kill-server"]);
    }
}

struct ReviewWorld {
    base: PathBuf,
    root: PathBuf,
    home: PathBuf,
    repo: PathBuf,
    transcripts: PathBuf,
    base_sha: String,
    c1: String,
    head: String,
}

fn review_world(base: &Path) -> ReviewWorld {
    let w = attention_world(base, std::time::SystemTime::now(), None);
    let repo = base.join("rev-repo");
    let origin = base.join("rev-origin.git");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "-q", "--bare", "-b", "main"]);
    git(&repo, &["init", "-q", "-b", "main"]);
    let lines = |name: &str, n: usize| (1..=n).map(|i| format!("{name}{i}\n")).collect::<String>();
    fs::write(repo.join("a.rs"), lines("a", 10)).unwrap();
    fs::write(repo.join("bad.rs"), lines("b", 3)).unwrap();
    fs::write(repo.join("err.rs"), lines("e", 3)).unwrap();
    fs::write(repo.join("cf.rs"), lines("f", 5)).unwrap();
    fs::write(repo.join("long.rs"), lines("l", 3)).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "base"]);
    git(&repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(&repo, &["push", "-q", "origin", "main"]);
    let base_sha = git(&repo, &["rev-parse", "HEAD"]);
    let edit = |file: &str, from: &str, to: &str| {
        let text = fs::read_to_string(repo.join(file)).unwrap().replace(from, to);
        fs::write(repo.join(file), text).unwrap();
    };
    git(&repo, &["checkout", "-q", "-b", "pr-branch"]);
    edit("a.rs", "a3\n", "a3 c1\n");
    edit("bad.rs", "b2\n", "b2 new\n");
    edit("err.rs", "e2\n", "e2 new\n");
    edit("cf.rs", "f2\n", "f2 c1\n");
    edit("long.rs", "l2\n", "l2 new\n");
    git(&repo, &["commit", "-q", "-am", "c1"]);
    let c1 = git(&repo, &["rev-parse", "HEAD"]);
    edit("a.rs", "a3 c1\n", "a3 c2\n");
    edit("a.rs", "a8\n", "a8 c2\n");
    edit("cf.rs", "f2 c1\n", "f2 c2\n");
    git(&repo, &["commit", "-q", "-am", "c2"]);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    git(&repo, &["push", "-q", "origin", "HEAD:refs/pull/5/head"]);
    git(&repo, &["checkout", "-q", "main"]);
    fs::write(w.root.join("config.yml"), format!("projects:\n  rev: {}\n", repo.display())).unwrap();
    fs::write(w.root.join("work-items/rev-item.md"), "---\nv: 1\ntitle: rev-item\nstate: in_progress\nproject: rev\ncreated: '2026-01-01'\n---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n").unwrap();
    let gh = base.join("gh");
    fs::create_dir_all(&gh).unwrap();
    executable(&base.join("bin/gh"), r#"#!/bin/sh
out=""
for a in "$@"; do case "$a" in query=*) a="query=$(printf %s "$a" | tr '\n' ' ' | cut -c7-60)";; esac; out="$out|$a"; done
echo "$out" >> "$FAKE_GH_LOG"
q="$*"
case "$q" in
  *"baseRefName headRefOid"*) cat "$FAKE_GH/pr.json";;
  *"addPullRequestReview(input"*) echo '{"data":{"addPullRequestReview":{"pullRequestReview":{"id":"R_new"}}}}';;
  *addPullRequestReviewThreadReply*)
    case "$q" in
      *thread=T_bad*) echo '{"data":{"addPullRequestReviewThreadReply":{"comment":null}}}';;
      *thread=T_err*) echo '{"errors":[{"message":"reply refused"}],"data":null}';;
      *thread=T_long*) printf '{"errors":[{"message":"%s"}],"data":null}' "$(printf 'x%.0s' $(seq 1 400))";;
      *) echo '{"data":{"addPullRequestReviewThreadReply":{"comment":{"id":"C_r","url":"u"}}}}';;
    esac;;
  *addPullRequestReviewThread*)
    case "$q" in
      *path=bad.rs*) echo '{"data":{"addPullRequestReviewThread":{"thread":null}}}';;
      *path=err.rs*) echo '{"errors":[{"message":"line is not part of the diff"}],"data":null}';;
      *path=long.rs*) printf '{"errors":[{"message":"%s"}],"data":null}' "$(printf 'y%.0s' $(seq 1 400))";;
      *) echo '{"data":{"addPullRequestReviewThread":{"thread":{"id":"TH"}}}}';;
    esac;;
  *addPullRequestReviewComment*)
    case "$q" in
      *path=cf.rs*) echo '{"errors":[{"message":"position is invalid"}],"data":null}';;
      *) echo '{"data":{"addPullRequestReviewComment":{"comment":{"id":"CM"}}}}';;
    esac;;
  *resolveReviewThread*|*unresolveReviewThread*)
    case "$q" in
      *id=T_none*) echo '{"data":{"resolveReviewThread":{"thread":null},"unresolveReviewThread":{"thread":null}}}';;
      *id=T_err*) echo "gh exploded" >&2; exit 1;;
      *unresolveReviewThread*) echo '{"data":{"unresolveReviewThread":{"thread":{"id":"T","isResolved":false}}}}';;
      *) echo '{"data":{"resolveReviewThread":{"thread":{"id":"T","isResolved":true}}}}';;
    esac;;
esac
"#);
    ReviewWorld { base: base.to_path_buf(), root: w.root, home: w.home, repo, transcripts: w.transcripts, base_sha, c1, head }
}

fn write_pr(w: &ReviewWorld, node: serde_json::Value) {
    fs::write(w.base.join("gh/pr.json"), serde_json::json!({"data": {"repository": {"pullRequest": node}}}).to_string()).unwrap();
}

#[test]
fn the_review_actions_match_ruby() {
    let base = tempfile::tempdir().unwrap();
    let bases = [base.path().join("a"), base.path().join("b")];
    let worlds = [review_world(&bases[0]), review_world(&bases[1])];
    let open = |w: &ReviewWorld, reviews: serde_json::Value| serde_json::json!({"id": "PR_1", "baseRefName": "main", "headRefOid": w.head, "state": "OPEN", "reviews": {"nodes": reviews}});
    let env = |i: usize| -> Vec<(&'static str, String)> {
        let w = &worlds[i];
        vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("PATH", format!("{}:{}", w.base.join("bin").display(), std::env::var("PATH").unwrap())), ("FAKE_GH", w.base.join("gh").display().to_string()), ("FAKE_GH_LOG", w.base.join("gh.log").display().to_string()), ("QM_TMUX_SOCKET", format!("qm-test-review-{i}"))]
    };
    for w in &worlds {
        write_pr(w, open(w, serde_json::json!([])));
    }
    let ruby = start_env(false, &worlds[0].root, &worlds[0].home, &env(0));
    let rust = start_env(true, &worlds[1].root, &worlds[1].home, &env(1));
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let norm = |text: String, i: usize| -> String {
        let w = &worlds[i];
        let mut t = text.replace(&w.base.display().to_string(), "BASE");
        for (sha, name) in [(&w.base_sha, "BASESHA"), (&w.c1, "C1SHA"), (&w.head, "HEADSHA")] {
            t = t.replace(sha.as_str(), name).replace(&sha[..7], &format!("{name}7"));
        }
        let t = regex::Regex::new(r#""(createdAt|updatedAt)":\s*"[^"]*""#).unwrap().replace_all(&t, "\"$1\":\"STAMP\"").into_owned();
        regex::Regex::new(r#"\b[0-9a-f]{12}\b"#).unwrap().replace_all(&t, "ID").into_owned()
    };
    let state = |i: usize| -> String {
        let w = &worlds[i];
        let mut out = String::new();
        tree_text(&w.root.join("drafts"), "drafts", &mut out);
        out += &fs::read_to_string(w.base.join("gh.log")).unwrap_or_default();
        let log = fs::read_to_string(w.root.join("qm.log")).unwrap_or_default();
        out += &regex::Regex::new(r"(?m)^\S+ \[").unwrap().replace_all(&log, "[");
        norm(out, i)
    };
    let act = |cmd: &str, slug: &str, args: &serde_json::Value| serde_json::json!({"cmd": cmd, "slug": slug, "args": args}).to_string();
    let finished = |i: usize, port: u16| -> String {
        let a = request(port, "GET", "/api/state", &[], "", true);
        let v: serde_json::Value = serde_json::from_slice(&a.body).unwrap_or_default();
        norm(v["finished"].to_string(), i)
    };
    let step = |label: &str, body: &str| {
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, body, true), request(rust.port, "POST", "/api/action", &origin, body, true));
        let (sa, sb) = (norm(shown(&a, ruby.port), 0), norm(shown(&b, rust.port), 1));
        assert_eq!(sa, sb, "{label}: reply to {body}");
        if sa.contains("\"background\":true") {
            std::thread::sleep(std::time::Duration::from_millis(2500));
            assert_eq!(finished(0, ruby.port), finished(1, rust.port), "{label}: the job's result");
        }
        assert_eq!(state(0), state(1), "{label}: what happened after {body}");
    };
    let step_w = |label: &str, build: &dyn Fn(&ReviewWorld) -> String| {
        let (ba, bb) = (build(&worlds[0]), build(&worlds[1]));
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, &ba, true), request(rust.port, "POST", "/api/action", &origin, &bb, true));
        let (sa, sb) = (norm(shown(&a, ruby.port), 0), norm(shown(&b, rust.port), 1));
        assert_eq!(sa, sb, "{label}: reply to {bb}");
        assert_eq!(state(0), state(1), "{label}: what happened after {bb}");
    };
    let put_drafts = |entries: &dyn Fn(&ReviewWorld) -> serde_json::Value| {
        for w in &worlds {
            fs::create_dir_all(w.root.join("drafts")).unwrap();
            fs::write(w.root.join("drafts/rev-item.json"), serde_json::to_string_pretty(&serde_json::json!({"v": 1, "entries": entries(w)})).unwrap()).unwrap();
        }
    };
    let gh = |w: &ReviewWorld, id: &str, path: &str, line: i64, code: &str, commit: &str, range: Option<&str>, subject: &str| {
        let mut e = serde_json::json!({"id": id, "kind": "github-comment", "comment": format!("comment {id}"), "rev": 1, "createdAt": "2026-04-01T00:00:00Z", "updatedAt": "2026-04-01T00:00:00Z",
            "path": path, "side": "new", "line": line, "code": code, "repo": "acme/widgets", "number": 5, "base": w.base_sha, "commit": commit, "subject": subject, "author": "user"});
        if let Some(r) = range {
            e["range"] = serde_json::json!(r);
        }
        e
    };
    let reply = |id: &str, thread: &str| serde_json::json!({"id": id, "kind": "thread-reply", "comment": format!("reply {id}"), "rev": 1, "createdAt": "2026-04-01T00:00:00Z", "updatedAt": "2026-04-01T00:00:00Z", "thread": thread, "repo": "acme/widgets", "number": 5, "url": "u", "path": "a.rs", "line": 3, "author": "user"});
    let all = |w: &ReviewWorld| {
        serde_json::json!([
            gh(w, "d1", "a.rs", 8, "a8 c2", &w.head, None, "line"),
            {"id": "d2", "kind": "github-comment", "comment": "a range", "rev": 1, "path": "a.rs", "side": "new", "line": 8, "start_line": 7, "start_side": "new", "code": "a7\na8 c2", "repo": "acme/widgets", "number": 5, "base": w.base_sha, "commit": w.head, "subject": "line", "author": "claude"},
            gh(w, "d3", "bad.rs", 2, "b2 new", &w.head, None, "line"),
            gh(w, "d4", "err.rs", 2, "e2 new", &w.head, None, "line"),
            gh(w, "d5", "a.rs", 3, "a3 c1", &w.c1, Some(&w.c1[..7]), "line"),
            gh(w, "d6", "cf.rs", 2, "f2 c1", &w.c1, Some(&w.c1[..7]), "line"),
            gh(w, "d7", "a.rs", 1, "x", &w.head, None, "file"),
            gh(w, "d8", "gone.rs", 1, "x", &w.head, None, "file"),
            {"id": "d9", "kind": "github-comment", "comment": "a range on a commit", "rev": 1, "path": "a.rs", "side": "new", "line": 3, "start_line": 2, "start_side": "new", "code": "a2\na3 c1", "range": &w.c1[..7], "repo": "acme/widgets", "number": 5, "base": w.base_sha, "commit": w.c1, "subject": "line", "author": "user"},
            {"id": "d10", "kind": "github-comment", "comment": "a removed line", "rev": 1, "path": "a.rs", "side": "old", "line": 3, "code": "a3", "repo": "acme/widgets", "number": 5, "base": w.base_sha, "commit": w.head, "subject": "line", "author": "user"},
            {"id": "d11", "kind": "github-comment", "comment": "uppercase side", "rev": 1, "path": "a.rs", "side": "LEFT", "line": 8, "code": "a8", "repo": "acme/widgets", "number": 5, "base": w.base_sha, "commit": w.head, "subject": "line", "author": "user"},
            gh(w, "d12", "long.rs", 2, "l2 new", &w.head, None, "line"),
            reply("r4", "T_long"),
            reply("r1", "T_ok"),
            reply("r2", "T_bad"),
            reply("r3", "T_err"),
            {"id": "other", "kind": "github-comment", "comment": "another PR", "rev": 1, "path": "a.rs", "side": "new", "line": 8, "code": "a8 c2", "repo": "acme/other", "number": 9, "base": w.base_sha, "commit": w.head, "subject": "line", "author": "user"},
            {"id": "chat", "kind": "diff-comment", "comment": "chat one", "rev": 1, "path": "a.rs", "side": "new", "line": 2}
        ])
    };
    put_drafts(&all);
    let args = |extra: serde_json::Value| {
        let mut a = serde_json::json!({"repo": "acme/widgets", "number": 5});
        for (k, v) in extra.as_object().unwrap() {
            a[k] = v.clone();
        }
        a
    };
    // the confirmation: counts per route, replies, moved lines and an existing pending review
    step("preview of everything", &act("review-preview", "rev-item", &args(serde_json::json!({}))));
    step("preview of two", &act("review-preview", "rev-item", &args(serde_json::json!({"ids": ["d1", "d5"]}))));
    step("preview of replies only", &act("review-preview", "rev-item", &args(serde_json::json!({"ids": ["r1", "r2"]}))));
    step("preview of nothing chosen", &act("review-preview", "rev-item", &args(serde_json::json!({"ids": ["chat"]}))));
    step("preview of another PR", &act("review-preview", "rev-item", &serde_json::json!({"repo": "acme/other", "number": 9})));
    step("preview of an unknown PR", &act("review-preview", "rev-item", &serde_json::json!({"repo": "acme/widgets", "number": 77})));
    step("a bad repo", &act("review-preview", "rev-item", &serde_json::json!({"repo": "nope", "number": 5})));
    step("a bad number", &act("review-preview", "rev-item", &serde_json::json!({"repo": "acme/widgets", "number": "x"})));
    step("a hex number", &act("review-preview", "rev-item", &serde_json::json!({"repo": "acme/widgets", "number": "0x5"})));
    step("no number", &act("review-preview", "rev-item", &serde_json::json!({"repo": "acme/widgets"})));
    step("preview on an item without a repo", &act("review-preview", "orphan", &args(serde_json::json!({}))));
    step("preview on a missing item", &act("review-preview", "nowhere", &args(serde_json::json!({}))));
    for w in &worlds {
        write_pr(w, open(w, serde_json::json!([{"id": "R_other", "viewerDidAuthor": false, "comments": {"totalCount": 9}}, {"id": "R_mine", "viewerDidAuthor": true, "comments": {"totalCount": 4}}])));
    }
    step("preview with a pending review", &act("review-preview", "rev-item", &args(serde_json::json!({}))));
    for w in &worlds {
        let mut node = open(w, serde_json::json!([]));
        node["state"] = serde_json::json!("MERGED");
        write_pr(w, node);
    }
    step("preview of a merged PR", &act("review-preview", "rev-item", &args(serde_json::json!({}))));
    for w in &worlds {
        write_pr(w, serde_json::Value::Null);
    }
    step("preview of a PR GitHub doesn't know", &act("review-preview", "rev-item", &args(serde_json::json!({}))));
    for w in &worlds {
        let mut node = open(w, serde_json::json!([]));
        node["headRefOid"] = serde_json::json!("0000000000000000000000000000000000000000");
        write_pr(w, node);
    }
    step("preview when the head moved", &act("review-preview", "rev-item", &args(serde_json::json!({}))));
    // posting: a thread per head draft, a position for a commit one, a fallback, replies and the failures kept
    for w in &worlds {
        write_pr(w, open(w, serde_json::json!([])));
    }
    step("post some", &act("review-post", "rev-item", &args(serde_json::json!({"ids": ["d1", "d2", "d5", "d7"]}))));
    step("post the rest", &act("review-post", "rev-item", &args(serde_json::json!({}))));
    step("post what failed again", &act("review-post", "rev-item", &args(serde_json::json!({}))));
    step("post nothing", &act("review-post", "rev-item", &args(serde_json::json!({"ids": []}))));
    put_drafts(&all);
    for w in &worlds {
        write_pr(w, open(w, serde_json::json!([{"id": "R_mine", "viewerDidAuthor": true, "comments": {"totalCount": 4}}])));
    }
    step("post into an existing review", &act("review-post", "rev-item", &args(serde_json::json!({"ids": ["d1", "r1", "d8"]}))));
    let log = fs::read_to_string(worlds[1].base.join("gh.log")).unwrap();
    for needle in ["subject=LINE", "subject=FILE", "startSide=RIGHT", "|-F|startLine=7", "|-F|position=", "|-H|GraphQL-Features: pull_request_review_comment_position|", "|-f|commit=", "|-f|thread=T_bad", "pr=PR_1"] {
        assert!(log.contains(needle), "the posts should have made {needle}: {log}");
    }
    step("post only outdated ones", &act("review-post", "rev-item", &args(serde_json::json!({"ids": ["d8"]}))));
    for w in &worlds {
        let mut node = open(w, serde_json::json!([]));
        node["state"] = serde_json::json!("CLOSED");
        write_pr(w, node);
    }
    put_drafts(&all);
    step("post to a closed PR", &act("review-post", "rev-item", &args(serde_json::json!({}))));
    // resolving threads
    step("resolve", &act("pr-thread-resolve", "", &serde_json::json!({"repo": "acme/widgets", "number": 5, "thread": "T_x1"})));
    step("unresolve", &act("pr-thread-resolve", "", &serde_json::json!({"repo": "acme/widgets", "number": "5", "thread": "T_x1", "resolve": false})));
    step("resolve with a truthy flag", &act("pr-thread-resolve", "", &serde_json::json!({"repo": "acme/widgets", "number": 5, "thread": "T_x2", "resolve": null})));
    step("resolve a thread GitHub lost", &act("pr-thread-resolve", "", &serde_json::json!({"repo": "acme/widgets", "number": 5, "thread": "T_none"})));
    step("resolve when gh fails", &act("pr-thread-resolve", "", &serde_json::json!({"repo": "acme/widgets", "number": 5, "thread": "T_err"})));
    step("resolve a bad thread", &act("pr-thread-resolve", "", &serde_json::json!({"repo": "acme/widgets", "number": 5, "thread": "a b"})));
    step("resolve no thread", &act("pr-thread-resolve", "", &serde_json::json!({"repo": "acme/widgets", "number": 5})));
    step("resolve a bad repo", &act("pr-thread-resolve", "", &serde_json::json!({"repo": 5, "number": 5, "thread": "T"})));
    step("resolve a bad number", &act("pr-thread-resolve", "", &serde_json::json!({"repo": "acme/widgets", "number": 0, "thread": "T"})));
    // GitHub drafts put, moved and placed: the anchor is checked against the PR head with git
    put_drafts(&|w| {
        serde_json::json!([
            gh(w, "g1", "a.rs", 8, "a8 c2", &w.head, None, "line"),
            {"id": "dn", "kind": "diff-comment", "comment": "pr scope chat draft", "rev": 2, "path": "a.rs", "side": "new", "line": 7, "code": "a7", "repo": "acme/widgets", "number": 5, "base": w.base_sha, "commit": w.head},
            {"id": "dc", "kind": "diff-comment", "comment": "plain", "rev": 1, "path": "a.rs", "side": "new", "line": 2},
            {"id": "gf", "kind": "github-comment", "comment": "failed before", "rev": 3, "failed": "422", "pin": "commit", "path": "cf.rs", "side": "new", "line": 2, "code": "f2 c1", "repo": "acme/widgets", "number": 5, "base": w.base_sha, "commit": w.c1, "range": &w.c1[..7], "subject": "line", "author": "claude"}
        ])
    });
    let put = |extra: serde_json::Value| {
        move |w: &ReviewWorld| {
            let mut a = serde_json::json!({"kind": "github-comment", "path": "a.rs", "side": "new", "line": 8, "code": "a8 c2", "comment": "put one", "repo": "acme/widgets", "number": 5, "base": w.base_sha, "commit": w.head});
            for (k, v) in extra.as_object().unwrap() {
                if v.is_null() {
                    a.as_object_mut().unwrap().remove(k);
                } else {
                    a[k] = v.clone();
                }
            }
            serde_json::json!({"cmd": "draft-put", "slug": "rev-item", "args": a}).to_string()
        }
    };
    for (label, extra) in [
        ("put a review draft", serde_json::json!({})),
        ("put it again", serde_json::json!({})),
        ("put a new comment on it", serde_json::json!({"comment": "changed"})),
        ("put as claude", serde_json::json!({"author": "claude", "comment": "claude's"})),
        ("put the same text as the user", serde_json::json!({"author": "user", "comment": "claude's"})),
        ("put a range", serde_json::json!({"line": 8, "start_line": 7, "start_side": "new", "code": "a7\na8 c2", "comment": "range"})),
        ("put a file comment", serde_json::json!({"subject": "file", "line": 1, "code": "", "comment": "file"})),
        ("put a file comment of a missing file", serde_json::json!({"subject": "file", "path": "none.rs", "comment": "x"})),
        ("put a line outside the diff", serde_json::json!({"path": "bad.rs", "line": 99, "code": "zzz"})),
        ("put the wrong text", serde_json::json!({"code": "wrong"})),
        ("put a removed line", serde_json::json!({"side": "old", "line": 3, "code": "a3"})),
        ("put on a commit's line", serde_json::json!({"line": 3, "code": "a3 c1", "commit": "C1", "range": "C1"})),
        ("put without a base", serde_json::json!({"base": null})),
        ("put with a short base", serde_json::json!({"base": "abc"})),
        ("put without a commit", serde_json::json!({"commit": null})),
        ("put without a repo", serde_json::json!({"repo": null})),
        ("put with a bad repo", serde_json::json!({"repo": "nope"})),
        ("put with a string number", serde_json::json!({"number": "5", "comment": "n"})),
        ("put with a zero number", serde_json::json!({"number": 0})),
        ("put with a hex number", serde_json::json!({"number": "0x5"})),
        ("put with a bad author", serde_json::json!({"author": "robot"})),
        ("put with a bad subject", serde_json::json!({"subject": "word"})),
        ("put a blank review draft with a short base", serde_json::json!({"comment": "", "base": "abc"})),
        ("put a chat draft with a short base", serde_json::json!({"kind": "diff-comment", "base": "abc"})),
        ("put a blank comment (removes)", serde_json::json!({"comment": "  "})),
        ("put a blank comment (nothing there)", serde_json::json!({"line": 2, "comment": ""})),
        ("put a PR-scope chat draft", serde_json::json!({"kind": "diff-comment", "line": 7, "code": "a7", "comment": "chat"})),
        ("put a PR-scope chat draft without a commit", serde_json::json!({"kind": "diff-comment", "commit": null})),
        ("put without a path", serde_json::json!({"path": null})),
        ("put a branch", serde_json::json!({"branch": "feat/x", "line": 2, "code": "a2", "comment": "branch"})),
        ("put another kind", serde_json::json!({"kind": "annotation"})),
    ] {
        let extra = extra.clone();
        step_w(label, &|w| {
            let fix = |v: &serde_json::Value| match v {
                serde_json::Value::String(t) if t == "C1" => serde_json::json!(w.c1),
                other => other.clone(),
            };
            let mapped: serde_json::Map<String, serde_json::Value> = extra.as_object().unwrap().iter().map(|(k, v)| (k.clone(), fix(v))).collect();
            put(serde_json::Value::Object(mapped))(w)
        });
    }
    let simple = |cmd: &'static str, args: serde_json::Value| move |_: &ReviewWorld| serde_json::json!({"cmd": cmd, "slug": "rev-item", "args": args}).to_string();
    step_w("move a PR-scope draft to the review", &simple("draft-move", serde_json::json!({"id": "dn", "to": "github"})));
    step_w("move it again", &simple("draft-move", serde_json::json!({"id": "dn", "to": "github"})));
    step_w("move a draft without a PR", &simple("draft-move", serde_json::json!({"id": "dc", "to": "github"})));
    step_w("move one back to the chat", &simple("draft-move", serde_json::json!({"id": "dn", "to": "chat"})));
    step_w("move a missing one", &simple("draft-move", serde_json::json!({"id": "zz", "to": "github"})));
    step_w("move to nowhere", &simple("draft-move", serde_json::json!({"id": "dn", "to": "gitlab"})));
    put_drafts(&|w| serde_json::json!([gh(w, "g1", "a.rs", 8, "a8 c2", &w.head, None, "line"), {"id": "dn", "kind": "diff-comment", "comment": "x", "rev": 1, "path": "a.rs", "side": "new", "line": 7, "code": "wrong", "repo": "acme/widgets", "number": 5, "base": w.base_sha, "commit": w.head}, {"id": "dm", "kind": "diff-comment", "comment": "no base", "rev": 1, "path": "a.rs", "side": "new", "line": 7, "code": "a7", "repo": "acme/widgets", "number": 5, "commit": w.head}]));
    step_w("move one the diff refuses", &simple("draft-move", serde_json::json!({"id": "dn", "to": "github"})));
    step_w("move one without a base", &simple("draft-move", serde_json::json!({"id": "dm", "to": "github"})));
    let place = |args: serde_json::Value| {
        move |w: &ReviewWorld| {
            let mut a = args.clone();
            for k in ["commit", "base"] {
                if a.get(k) == Some(&serde_json::json!(format!("<{k}>"))) {
                    a[k] = serde_json::json!(if k == "commit" { w.head.clone() } else { w.base_sha.clone() });
                }
            }
            serde_json::json!({"cmd": "draft-place", "slug": "rev-item", "args": a}).to_string()
        }
    };
    put_drafts(&|w| serde_json::json!([gh(w, "g1", "a.rs", 3, "a3 c1", &w.c1, Some(&w.c1[..7]), "line"), {"id": "dc", "kind": "diff-comment", "comment": "plain", "rev": 1, "path": "a.rs", "side": "new", "line": 2}]));
    step_w("pin to its commit", &place(serde_json::json!({"id": "g1", "how": "commit"})));
    step_w("place on a line of the head", &place(serde_json::json!({"id": "g1", "how": "line", "path": "a.rs", "side": "new", "line": 8, "code": "a8 c2", "commit": "<commit>", "base": "<base>"})));
    step_w("place on a line that isn't there", &place(serde_json::json!({"id": "g1", "how": "line", "path": "a.rs", "side": "new", "line": 8, "code": "other", "commit": "<commit>", "base": "<base>"})));
    step_w("place as a file comment", &place(serde_json::json!({"id": "g1", "how": "file", "path": "ignored.rs", "commit": "<commit>", "base": "<base>"})));
    step_w("place somewhere odd", &place(serde_json::json!({"id": "g1", "how": "moon"})));
    step_w("place with no how", &place(serde_json::json!({"id": "g1"})));
    step_w("place a number", &place(serde_json::json!({"id": "g1", "how": 5})));
    step_w("place a chat draft", &place(serde_json::json!({"id": "dc", "how": "commit"})));
    step_w("place a missing draft", &place(serde_json::json!({"id": "zz", "how": "commit"})));
    step_w("place with no base", &place(serde_json::json!({"id": "g1", "how": "file", "commit": "<commit>"})));
}

#[test]
fn issue_preview_and_start_match_ruby() {
    let base = tempfile::tempdir().unwrap();
    let bases = [base.path().join("a"), base.path().join("b")];
    let chat = base.path().join("fake-chat");
    executable(&chat, "#!/bin/sh\nexec sleep 300\n");
    let mut worlds = vec![];
    let mut baseline: Vec<Vec<String>> = vec![];
    for (i, b) in bases.iter().enumerate() {
        let w = attention_world(b, std::time::SystemTime::now(), None);
        let origin = b.join("acme/widgets.git");
        fs::create_dir_all(&origin).unwrap();
        git(&origin, &["init", "-q", "--bare", "-b", "main"]);
        git(&w.repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
        git(&w.repo, &["push", "-q", "origin", "main"]);
        git(&w.repo, &["remote", "set-head", "origin", "main"]);
        git(&w.repo, &["branch", "issue/14-old-work"]);
        let plain = b.join("plain-repo");
        common::repo(&plain, "main");
        fs::write(w.root.join("config.yml"), format!("default_project: demo\nprojects:\n  plain: {}\n  demo: {}\n  other: {}\n", plain.display(), w.repo.display(), w.repo.display())).unwrap();
        let item = |slug: &str, project: &str, state: &str, extra: &str, description: &str| {
            fs::write(w.root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\ntitle: {slug}\nstate: {state}\nproject: {project}\ncreated: '2026-01-01'\n{extra}---\n\n## Description\n\n{description}\n\n## Notes\n\n## Snapshots\n")).unwrap();
        };
        item("by-branch", "demo", "in_progress", "branches:\n- issue/12-the-thing\n", "d");
        item("by-url", "demo", "in_progress", "", "see https://github.com/acme/widgets/issues/13 for more");
        item("a-url-prefix", "demo", "in_progress", "", "see https://github.com/acme/widgets/issues/130 only");
        item("a-url-23", "demo", "in_progress", "", "mentions https://github.com/acme/widgets/issues/23");
        item("z-branch-23", "demo", "in_progress", "branches:\n- issue/23-held\n", "d");
        item("dup-a", "demo", "in_progress", "branches:\n- issue/21-first\n", "d");
        item("dup-b", "demo", "in_progress", "branches:\n- issue/21-second\n", "d");
        item("linked-elsewhere", "plain", "in_progress", "branches:\n- issue/22-linked\n", "d");
        git(&w.repo, &["branch", "issue/22-linked"]);
        item("a-resolved", "demo", "resolved", "branches:\n- issue/17-old\n", "d");
        item("both", "demo", "in_progress", "branches:\n- issue/17-newer\n", "d");
        item("drafting", "demo", "in_progress", "", "where issues are drafted");
        // what the state builds may touch on their own is not compared; what an action touches or makes is
        baseline.push(fs::read_dir(w.root.join("work-items")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| !["a-resolved.md", "both.md", "drafting.md"].contains(&n.as_str())).collect::<Vec<String>>());
        let gh = b.join("gh");
        fs::create_dir_all(&gh).unwrap();
        let issue = |n: i64, title: &str, state: &str| fs::write(gh.join(format!("issue-{n}.json")), serde_json::json!({"number": n, "title": title, "state": state, "url": format!("https://github.com/acme/widgets/issues/{n}")}).to_string()).unwrap();
        issue(12, "The thing", "OPEN");
        issue(13, "Linked by url", "OPEN");
        issue(14, "Old work comes back", "OPEN");
        issue(15, "A brand new issue with a very long title that has to be cut at a word boundary", "OPEN");
        issue(16, "Closed one", "CLOSED");
        issue(17, "Seventeen", "OPEN");
        issue(18, "Über naïve — título", "OPEN");
        issue(19, "!!!", "OPEN");
        issue(20, "aaaaaaaaaa bbbbbbbbbb cccccccccc dddddddd eee", "OPEN");
        issue(21, "Two holders", "OPEN");
        issue(22, "Linked elsewhere", "OPEN");
        issue(23, "Held by branch and url", "OPEN");
        fs::write(gh.join("issue-500.json"), "not json").unwrap();
        fs::write(gh.join("issue-501.json"), "[1]").unwrap();
        executable(&b.join("bin/gh"), "#!/bin/sh\necho \"$*\" >> \"$FAKE_GH_LOG\"\ncase \"$1 $2\" in\n  \"issue view\") f=\"$FAKE_GH/issue-$3.json\"; [ -f \"$f\" ] && cat \"$f\" || { echo \"Could not resolve to an issue\" >&2; exit 1; };;\n  \"repo view\") echo main;;\nesac\n");
        let _ = i;
        worlds.push(w);
    }
    let env = |i: usize| -> Vec<(&'static str, String)> {
        let w = &worlds[i];
        vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("PATH", format!("{}:{}", bases[i].join("bin").display(), std::env::var("PATH").unwrap())), ("FAKE_GH", bases[i].join("gh").display().to_string()), ("FAKE_GH_LOG", bases[i].join("gh.log").display().to_string()), ("QM_TMUX_SOCKET", format!("qm-test-issue-{i}")), ("QM_CHAT_COMMAND", chat.display().to_string())]
    };
    for i in 0..2 {
        tmux_on(&format!("qm-test-issue-{i}"), &["kill-server"]);
    }
    let ruby = start_env(false, &worlds[0].root, &worlds[0].home, &env(0));
    let rust = start_env(true, &worlds[1].root, &worlds[1].home, &env(1));
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let norm = |text: String, i: usize| -> String { text.replace(&bases[i].display().to_string(), "BASE").replace(&worlds[i].home.display().to_string(), "HOME") };
    let state = |i: usize| -> String {
        let mut out = String::new();
        // the world's own items are the state builds' business (their clocks); the issue items are compared
        let mut all = String::new();
        tree_text(&worlds[i].root.join("work-items"), "work-items", &mut all);
        for block in all.split("== ").filter(|b| !b.is_empty()) {
            let name = block.lines().next().unwrap_or("").trim_start_matches("work-items/");
            if !baseline[i].iter().any(|n| n == name) {
                out += &format!("== {block}");
            }
        }
        out += &git(&worlds[i].repo, &["worktree", "list"]);
        out += &git(&worlds[i].repo, &["branch", "--list", "--format=%(refname:short) %(objectname:short)"]);
        // the state builds ask gh for each linked branch's PR in the background: not what is compared
        out += &fs::read_to_string(bases[i].join("gh.log")).unwrap_or_default().lines().filter(|l| !l.starts_with("pr view")).map(|l| format!("{l}\n")).collect::<String>();
        out += &tmux_picture(&format!("qm-test-issue-{i}"));
        let log = fs::read_to_string(worlds[i].root.join("qm.log")).unwrap_or_default();
        out += &regex::Regex::new(r"(?m)^\S+ \[").unwrap().replace_all(&log, "[");
        let t = norm(out, i);
        regex::Regex::new(r"(?m)^(created|\s*- 20\d\d-\d\d-\d\dT[^\n]*|[^\n]*20\d\d-\d\d-\d\d[^\n]*)$").unwrap().replace_all(&t, "DATE").into_owned()
    };
    let act = |cmd: &str, slug: &str, args: serde_json::Value| serde_json::json!({"cmd": cmd, "slug": slug, "args": args}).to_string();
    let step = |label: &str, body: &str| {
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, body, true), request(rust.port, "POST", "/api/action", &origin, body, true));
        let (sa, sb) = (norm(shown(&a, ruby.port), 0), norm(shown(&b, rust.port), 1));
        assert_eq!(sa, sb, "{label}: reply to {body}");
        if sa.contains("\"background\":true") {
            std::thread::sleep(std::time::Duration::from_millis(2500));
            let finished = |i: usize, port: u16| -> String {
                let a = request(port, "GET", "/api/state", &[], "", true);
                let v: serde_json::Value = serde_json::from_slice(&a.body).unwrap_or_default();
                norm(v["finished"].to_string(), i)
            };
            assert_eq!(finished(0, ruby.port), finished(1, rust.port), "{label}: the job's result");
        }
        let (xa, xb) = (state(0), state(1));
        if xa != xb {
            let at = xa.bytes().zip(xb.bytes()).position(|(p, q)| p != q).unwrap_or(xa.len().min(xb.len()));
            let from = xa[..at].rfind('\n').map(|n| n.saturating_sub(300)).unwrap_or(0);
            panic!("{label}: what happened after {body}: first difference at byte {at}\nRuby: …{}\nRust: …{}", &xa[from.min(xa.len())..(at + 400).min(xa.len())], &xb[from.min(xb.len())..(at + 400).min(xb.len())]);
        }
    };
    for (label, args) in [
        ("an item holds the branch", serde_json::json!({"number": "#12"})),
        ("an item holds the url", serde_json::json!({"number": 13})),
        ("a url that only starts alike", serde_json::json!({"number": "130"})),
        ("a branch no item links", serde_json::json!({"number": "14"})),
        ("a new issue", serde_json::json!({"number": 15})),
        ("a closed issue", serde_json::json!({"number": 16})),
        ("resolved and unresolved holders", serde_json::json!({"number": 17})),
        ("a title with accents", serde_json::json!({"number": 18})),
        ("a title with nothing to slug", serde_json::json!({"number": 19})),
        ("a title cut exactly at the limit", serde_json::json!({"number": 20})),
        ("two unresolved holders", serde_json::json!({"number": 21})),
        ("a branch another item links", serde_json::json!({"number": 22})),
        ("a holder by branch and one by url", serde_json::json!({"number": 23})),
        ("a chosen project", serde_json::json!({"number": 15, "project": "plain"})),
        ("a project without a root", serde_json::json!({"number": 15, "project": "nowhere"})),
        ("blank project", serde_json::json!({"number": 15, "project": "  "})),
        ("no number", serde_json::json!({})),
        ("zero", serde_json::json!({"number": 0})),
        ("leading zero", serde_json::json!({"number": "012"})),
        ("text", serde_json::json!({"number": "abc"})),
        ("padded", serde_json::json!({"number": " #15 "})),
        ("an issue that doesn't exist", serde_json::json!({"number": 404})),
        ("an answer that isn't JSON", serde_json::json!({"number": 500})),
    ] {
        step(label, &act("issue-preview", "", args));
    }
    step("start an existing issue's item", &act("issue-start", "", serde_json::json!({"number": 12})));
    step("start one that is resolved", &act("issue-start", "", serde_json::json!({"number": 17})));
    step("start a new issue", &act("issue-start", "drafting", serde_json::json!({"number": 15, "from": "drafting"})));
    step("start it again", &act("issue-start", "", serde_json::json!({"number": 15})));
    step("start a reused branch", &act("issue-start", "", serde_json::json!({"number": 14})));
    step("start a closed issue", &act("issue-start", "", serde_json::json!({"number": 16})));
    step("start from a missing item", &act("issue-start", "", serde_json::json!({"number": 18, "from": "nowhere"})));
    step("start in another project", &act("issue-start", "", serde_json::json!({"number": 18, "project": "plain", "from": ""})));
    step("start with no number", &act("issue-start", "", serde_json::json!({})));
    for i in 0..2 {
        tmux_on(&format!("qm-test-issue-{i}"), &["kill-server"]);
    }
}

#[test]
fn branch_stats_match_ruby() {
    let base = tempfile::tempdir().unwrap();
    let bases = [base.path().join("a"), base.path().join("b")];
    let worlds = [workspace_world(&bases[0], false), workspace_world(&bases[1], false)];
    for (i, w) in worlds.iter().enumerate() {
        let wt = bases[i].join("wt-w");
        let write = |path: &str, text: &str| {
            let p = wt.join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        };
        write("app/models/thing.rb", "a\nb\nc\n");
        write("app/javascript/widget.tsx", "x\ny\n");
        write("spec/thing_spec.rb", "t\n");
        git(&wt, &["add", "."]);
        git(&wt, &["commit", "-q", "-m", "Add the thing | with a pipe in its subject"]);
        write("docs/guide.md", "doc\ndoc\n");
        write("config/locales/en.yml", "en: 1\n");
        write("db/schema.rb", "schema\n");
        write("db/migrate/001_x.rb", "m\n");
        write("app/javascript/api/client.ts", "g\n");
        git(&wt, &["add", "."]);
        git(&wt, &["commit", "-q", "-m", "Docs, locales, a migration and generated code"]);
        write("config/application.rb", "c\n");
        write("openapi.yml", "o\n");
        git(&wt, &["add", "."]);
        git(&wt, &["commit", "-q", "-m", "Config and openapi"]);
        let bin = bases[i].join("bin");
        executable(&bin.join("claude"), "#!/bin/sh\necho \"$*\" | cut -c1-80 >> \"$FAKE_CLAUDE_LOG\"\nsleep 1\n[ -f \"$FAKE_FAIL/claude\" ] && { echo \"model unavailable\" >&2; exit 2; }\n[ -f \"$FAKE_FAIL/long\" ] && { printf 'y%.0s' $(seq 1 300) >&2; exit 2; }\n[ -f \"$FAKE_FAIL/empty\" ] && exit 0\necho \"The first commit carries the change. Mostly hand-written.\"\n");
        fs::create_dir_all(bases[i].join("fail")).unwrap();
        let _ = w;
    }
    let env = |i: usize| -> Vec<(&'static str, String)> {
        let w = &worlds[i];
        vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("PATH", format!("{}:{}:{}", bases[i].join("bin").display(), bases[i].join("tools").display(), std::env::var("PATH").unwrap())), ("FAKE_CLAUDE_LOG", bases[i].join("claude.log").display().to_string()), ("FAKE_FAIL", bases[i].join("fail").display().to_string()), ("FAKE_TOOL_LOG", bases[i].join("tool.log").display().to_string()), ("QM_TMUX_SOCKET", format!("qm-test-stats-{i}"))]
    };
    let ruby = start_env(false, &worlds[0].root, &worlds[0].home, &env(0));
    let rust = start_env(true, &worlds[1].root, &worlds[1].home, &env(1));
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let norm = |text: String, i: usize| -> String {
        let t = text.replace(&bases[i].display().to_string(), "BASE");
        let t = regex::Regex::new(r#"20\d\d-\d\d-\d\dT\d\d:\d\d:\d\d[+-Z0-9:]*"#).unwrap().replace_all(&t, "STAMP").into_owned();
        let shas: Vec<String> = git(&worlds[i].repo, &["rev-list", "--all"]).lines().map(String::from).collect();
        shas.iter().fold(t, |t, sha| t.replace(sha.as_str(), "SHA").replace(&sha[..10], "SHA10").replace(&sha[..7], "SHA7"))
    };
    let cache = |i: usize| norm(fs::read_to_string(worlds[i].root.join("branch-stats.json")).unwrap_or_default(), i);
    let get = |path: &str| {
        let (a, b) = (request(ruby.port, "GET", path, &[], "", true), request(rust.port, "GET", path, &[], "", true));
        let (sa, sb) = (norm(shown(&a, ruby.port), 0), norm(shown(&b, rust.port), 1));
        assert_eq!(sa, sb, "GET {path}");
        assert_eq!(cache(0), cache(1), "the cache after GET {path}");
        sa
    };
    let act = |cmd: &str, slug: &str, args: serde_json::Value| serde_json::json!({"cmd": cmd, "slug": slug, "args": args}).to_string();
    let finished = |i: usize, port: u16| -> String {
        let a = request(port, "GET", "/api/state", &[], "", true);
        let v: serde_json::Value = serde_json::from_slice(&a.body).unwrap_or_default();
        norm(v["finished"].to_string(), i)
    };
    let post = |label: &str, body: &str| {
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, body, true), request(rust.port, "POST", "/api/action", &origin, body, true));
        let (sa, sb) = (norm(shown(&a, ruby.port), 0), norm(shown(&b, rust.port), 1));
        assert_eq!(sa, sb, "{label}: reply to {body}");
        if sa.contains("\"background\":true") {
            let running = get("/api/branch-stats/w-exist");
            if label == "commentary" {
                assert!(running.contains("\"commentaryRunning\":true"), "{label}: should be running: {running}");
            }
            std::thread::sleep(std::time::Duration::from_millis(2200));
            assert_eq!(finished(0, ruby.port), finished(1, rust.port), "{label}: the job's result");
        }
        let log = |i: usize| norm(fs::read_to_string(bases[i].join("claude.log")).unwrap_or_default(), i);
        assert_eq!(log(0), log(1), "{label}: what the model was asked");
        assert_eq!(cache(0), cache(1), "{label}: the cache");
    };
    let first = get("/api/branch-stats/w-exist");
    for needle in ["\"stats\":{", "\"mergeBase\"", "with a pipe in its subject", "\"generated\":", "\"largest\":[{", "\"commentaryRunning\":false"] {
        assert!(first.contains(needle), "{needle} in {first}");
    }
    get("/api/branch-stats/w-exist");
    let computed = |i: usize| -> String { fs::read_to_string(worlds[i].root.join("branch-stats.json")).unwrap().lines().find(|l| l.contains("computedAt")).unwrap_or("").to_string() };
    let before = computed(1);
    std::thread::sleep(std::time::Duration::from_millis(1200));
    get("/api/branch-stats/w-exist?refresh=1");
    assert_ne!(before, computed(1), "a refresh computes the stats again");
    get("/api/branch-stats/w-exist?branch=feat-w");
    get("/api/branch-stats/w-feat");
    get("/api/branch-stats/w-feat?branch=feat-a");
    get("/api/branch-stats/w-exist?branch=nowhere");
    get("/api/branch-stats/w-exist?branch=");
    get("/api/branch-stats/w-stack");
    get("/api/branch-stats/w-main");
    get("/api/branch-stats/missing-item");
    get("/api/branch-stats/orphan");
    post("commentary", &act("branch-stats-commentary", "w-exist", serde_json::json!({})));
    get("/api/branch-stats/w-exist");
    post("commentary again", &act("branch-stats-commentary", "w-exist", serde_json::json!({"branch": "feat-w"})));
    std::fs::write(bases[0].join("fail/claude"), "").unwrap();
    std::fs::write(bases[1].join("fail/claude"), "").unwrap();
    post("commentary when claude fails", &act("branch-stats-commentary", "w-exist", serde_json::json!({})));
    std::fs::remove_file(bases[0].join("fail/claude")).unwrap();
    std::fs::remove_file(bases[1].join("fail/claude")).unwrap();
    std::fs::write(bases[0].join("fail/long"), "").unwrap();
    std::fs::write(bases[1].join("fail/long"), "").unwrap();
    post("commentary when claude fails at length", &act("branch-stats-commentary", "w-exist", serde_json::json!({})));
    std::fs::remove_file(bases[0].join("fail/long")).unwrap();
    std::fs::remove_file(bases[1].join("fail/long")).unwrap();
    std::fs::write(bases[0].join("fail/empty"), "").unwrap();
    std::fs::write(bases[1].join("fail/empty"), "").unwrap();
    post("commentary when claude says nothing", &act("branch-stats-commentary", "w-exist", serde_json::json!({})));
    std::fs::remove_file(bases[0].join("fail/empty")).unwrap();
    std::fs::remove_file(bases[1].join("fail/empty")).unwrap();
    post("commentary before any stats", &act("branch-stats-commentary", "w-feat", serde_json::json!({"branch": "feat-a"})));
    post("commentary on a missing item", &act("branch-stats-commentary", "nowhere", serde_json::json!({})));
    post("commentary on an unknown branch", &act("branch-stats-commentary", "w-exist", serde_json::json!({"branch": "nowhere"})));
    // a new commit moves the head: the entry no longer matches and is replaced
    for (i, _) in worlds.iter().enumerate() {
        let wt = bases[i].join("wt-w");
        fs::write(wt.join("app/models/more.rb"), "z\n").unwrap();
        git(&wt, &["add", "."]);
        git(&wt, &["commit", "-q", "-m", "A fourth commit"]);
    }
    let moved = get("/api/branch-stats/w-exist");
    assert!(moved.contains("A fourth commit") && moved.contains("\"commentary\":null"), "{moved}");
    for i in 0..2 {
        tmux_on(&format!("qm-test-stats-{i}"), &["kill-server"]);
    }
}

#[test]
fn build_progress_matches_ruby() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let session_dir = w.transcripts.join("p/d-live");
    let workflows = session_dir.join("subagents/workflows");
    let j = |v: serde_json::Value| format!("{v}\n");
    let user = |text: serde_json::Value, ts: &str| j(serde_json::json!({"type": "user", "timestamp": ts, "message": {"role": "user", "content": text}}));
    let asst = |blocks: serde_json::Value| j(serde_json::json!({"type": "assistant", "timestamp": "2026-03-01T10:05:00Z", "message": {"content": blocks}}));
    let write = |path: &Path, text: &str| {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    // a long activity: more than twenty entries, long texts, every kind of tool input
    let mut busy = user(serde_json::json!("relayed request"), "2026-03-01T09:00:00Z");
    busy += &user(serde_json::json!("[Workflow harness: you are agent 3]\nand more framing here. Your task text follows:\n\nBuild the alpha feature:\n  - do   this\n  - do that"), "2026-03-01T09:01:00.500Z");
    busy += &user(serde_json::json!([{"type": "text", "text": "ignored? no: the last one counts"}, {"type": "image"}, "junk"]), "2026-03-01T09:02:00Z");
    for i in 0..14 {
        busy += &asst(serde_json::json!([{"type": "text", "text": format!("step {i}: {}", "word ".repeat(60))}, {"type": "tool_use", "name": "Bash", "input": {"command": format!("cargo test {i}"), "description": ""}}]));
    }
    busy += &asst(serde_json::json!([{"type": "tool_use", "name": "Read", "input": {"file_path": "src/a.rs"}}, {"type": "tool_use", "name": "Odd", "input": {"zzz": [1, 2], "n": 5}}, {"type": "tool_use", "name": "Str", "input": "plain text input"}, {"type": "tool_use", "name": "None"}, {"type": "text", "text": "  "}, "junk", {"type": "thinking"}]));
    busy += &j(serde_json::json!({"type": "user", "message": {"content": "tool result"}}));
    busy += "not json at all\n";
    let simple = |task: &str| user(serde_json::json!(task), "2026-03-01T09:00:00Z") + &asst(serde_json::json!([{"type": "text", "text": "working"}]));
    let handed = user(serde_json::json!("done soon"), "2026-03-01T09:00:00Z") + &asst(serde_json::json!([{"type": "tool_use", "name": "StructuredOutput", "input": {"ok": true}}]));
    // run r1: a journal says who launched and who ended
    let r1 = workflows.join("r1");
    let journal = [
        serde_json::json!({"type": "started", "key": "k1", "agentId": "a1", "label": "build:alpha", "phase": "build"}),
        serde_json::json!({"type": "started", "key": "k2", "agentId": "a2", "label": "review:alpha", "phase": "review"}),
        serde_json::json!({"type": "started", "key": "k3", "agentId": "a3", "label": "queue#1"}),
        serde_json::json!({"type": "started", "key": "k4", "agentId": "a4", "label": "Spec CHECKER"}),
        serde_json::json!({"type": "started", "key": "k5", "agentId": "a5", "label": "custom label"}),
        serde_json::json!({"type": "started", "key": "k6", "agentId": "a6", "label": "integrate"}),
        serde_json::json!({"type": "started", "key": "k7", "agentId": "", "label": "no agent"}),
        serde_json::json!({"type": "started", "key": "k8", "agentId": "a8"}),
        serde_json::json!({"type": "started", "key": "k9", "agentId": "a9", "label": "missing transcript"}),
        serde_json::json!({"type": "started", "key": "k10", "agentId": "a10", "label": "build:gamma"}),
        serde_json::json!({"type": "failed", "key": "k10"}),
        serde_json::json!({"type": "started", "key": "k11", "agentId": "a11", "label": "queueing up"}),
        serde_json::json!({"type": "started", "key": "k1", "agentId": "a1", "label": "build:alpha", "phase": "build again"}),
        serde_json::json!({"type": "result", "key": "k2"}),
        serde_json::json!({"type": "mystery", "key": "k3"}),
    ];
    write(&r1.join("journal.jsonl"), &(journal.iter().map(|e| format!("{e}\n")).collect::<String>() + "garbage line\n"));
    write(&r1.join("agent-a1.jsonl"), &busy);
    for id in ["a2", "a3", "a4", "a5", "a6", "a8", "a10", "a11"] {
        write(&r1.join(format!("agent-{id}.jsonl")), &simple(&format!("task of {id}")));
    }
    let big = user(serde_json::json!("big agent"), "2026-03-01T09:00:00Z") + &asst(serde_json::json!([{"type": "text", "text": "early words"}])) + &j(serde_json::json!({"type": "user", "message": {"content": "x".repeat(120_000)}})) + &asst(serde_json::json!([{"type": "text", "text": "middle words"}])) + &j(serde_json::json!({"type": "user", "message": {"content": "y".repeat(120_000)}}));
    write(&r1.join("agent-a5.jsonl"), &big);
    write(&r1.join("agent-a8.meta.json"), "{\"description\": \"from the meta\", \"workflowPhase\": \"meta phase\"}");
    // run r2: no journal — an agent whose transcript ends in its hand-back is done
    let r2 = workflows.join("r2");
    write(&r2.join("agent-b1.jsonl"), &simple("beta work"));
    write(&r2.join("agent-b1.meta.json"), "{\"description\": \"build:beta\", \"workflowPhase\": \"building\"}");
    write(&r2.join("agent-b2.jsonl"), &handed);
    write(&r2.join("agent-b3.jsonl"), &simple("no meta at all"));
    // runs that ended, are ending, or wait
    for (run, result) in [("r3", "{\"status\": \"completed\"}"), ("r4", "{half"), ("r5", "{\"status\": \"pending\"}"), ("r6", "{\"status\": \"\"}"), ("r7", "{}"), ("r8", "{\"status\": \"failed\"}")] {
        write(&workflows.join(run).join("agent-c1.jsonl"), &simple(&format!("run {run}")));
        write(&session_dir.join("workflows").join(format!("{run}.json")), result);
    }
    write(&workflows.join("not-a-run.txt"), "x");
    let wf2 = w.transcripts.join("p/wf2-live");
    write(&wf2.join("subagents/workflows/w1/agent-z1.jsonl"), &simple("second item's task"));
    write(&wf2.join("subagents/workflows/w1/agent-z1.meta.json"), "{\"description\": \"build:zeta\"}");
    write(&w.transcripts.join("p/wf2-live.jsonl"), "{}\n");
    let log = fs::read_to_string(w.root.join("sessions.jsonl")).unwrap();
    fs::write(w.root.join("sessions.jsonl"), log + &format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"wf2-live\",\"cwd\":\"/srv/app\",\"pid\":{},\"transcript_path\":\"{}\",\"ts\":\"2026-03-01T00:00:00+00:00\"}}\n", std::process::id(), w.transcripts.join("p/wf2-live.jsonl").display())).unwrap();
    fs::write(w.root.join("work-items/wf2.md"), "---\nv: 1\ntitle: wf2\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\nsessions:\n- id: wf2-live\n---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n").unwrap();
    let bin = base.path().join("bin2");
    executable(&bin.join("claude"), "#!/bin/sh\necho \"$*\" | cut -c1-120 >> \"$FAKE_CLAUDE_LOG\"\ncat >> \"$FAKE_CLAUDE_LOG\"\nmode=$(cat \"$FAKE_MODE\" 2>/dev/null)\ncase \"$mode\" in\n  plain) echo '{\"result\": \"  all going fine  \", \"structured_output\": {\"agents\": \"nope\"}}';;\n  plainblank) echo '{\"result\": \" \"}';;\n  garbage) echo 'just words';;\n  array) echo '[1]';;\n  badagents) echo '{\"structured_output\": {\"agents\": [{\"label\": \"x\"}, 5]}}';;\n  fail) echo 'quota exceeded' >&2; exit 3;;\n  failout) echo 'only on stdout'; exit 3;;\n  *) echo '{\"structured_output\": {\"agents\": [{\"label\": \"build:alpha\", \"percent\": 45.5, \"doing\": \"  writing tests \"}, {\"label\": \"build:beta\", \"percent\": 150, \"doing\": \"x\"}, {\"label\": \"review:beta\", \"percent\": \"7\", \"doing\": \"\"}, {\"label\": \"invented\", \"percent\": 1, \"doing\": \"nope\"}, {\"label\": \"custom label\", \"percent\": -5}, {\"label\": \"integrate\", \"percent\": 12, \"doing\": 7}]}, \"result\": \"x\"}';;\nesac\n");
    let root2 = base.path().join("root2");
    copy_dir(&w.root, &root2);
    let env = |i: usize| -> Vec<(&'static str, String)> { vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("PATH", format!("{}:{}:{}", bin.display(), w.bin.display(), std::env::var("PATH").unwrap())), ("FAKE_GH_DATA", w.gh_data.display().to_string()), ("FAKE_GH_LOG", base.path().join(format!("gh{i}.log")).display().to_string()), ("FAKE_CLAUDE_LOG", base.path().join(format!("claude{i}.log")).display().to_string()), ("FAKE_MODE", base.path().join(format!("mode{i}")).display().to_string()), ("QM_TMUX_SOCKET", format!("qm-test-build-{i}"))] };
    let ruby = start_env(false, &w.root, &w.home, &env(0));
    let rust = start_env(true, &root2, &w.home, &env(1));
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    for srv in [&ruby, &rust] {
        request(srv.port, "GET", "/api/state", &[], "", true);
    }
    std::thread::sleep(std::time::Duration::from_millis(900));
    let first = compare_state(&ruby, &rust, "the first state");
    for needle in ["\"buildActive\":[\"degrade\",\"wf2\"]", "\"buildProgress\":{}"] {
        assert!(first.contains(needle), "{needle} missing");
    }
    let norm = |t: String| regex::Regex::new(r"working for \d+ min").unwrap().replace_all(&regex::Regex::new(r#"20\d\d-\d\d-\d\dT[0-9:.]+(Z|[+-]\d\d:\d\d)"#).unwrap().replace_all(&t, "STAMP"), "working for N min").into_owned();
    let act = |cmd: &str, slug: &str| serde_json::json!({"cmd": cmd, "slug": slug, "args": {}}).to_string();
    let step = |label: &str, mode: &str, slug: &str| {
        fs::write(base.path().join("mode0"), mode).unwrap();
        fs::write(base.path().join("mode1"), mode).unwrap();
        let body = act("build-progress", slug);
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, &body, true), request(rust.port, "POST", "/api/action", &origin, &body, true));
        assert_eq!(norm(shown(&a, ruby.port)), norm(shown(&b, rust.port)), "{label}: reply");
        std::thread::sleep(std::time::Duration::from_millis(1200));
        let state = compare_state(&ruby, &rust, label);
        let fin = |port: u16| -> String {
            let v: serde_json::Value = serde_json::from_slice(&request(port, "GET", "/api/state", &[], "", true).body).unwrap_or_default();
            norm(v["finished"].to_string())
        };
        assert_eq!(fin(ruby.port), fin(rust.port), "{label}: finished");
        let log = |i: usize| norm(fs::read_to_string(base.path().join(format!("claude{i}.log"))).unwrap_or_default());
        assert_eq!(log(0), log(1), "{label}: what the model was asked");
        norm(state)
    };
    step("the second item first", "", "wf2");
    let s1 = step("estimates", "", "degrade");
    for needle in ["\"percent\":46", "\"percent\":100", "no estimate", "\"percent\":0", "writing tests"] {
        assert!(s1.contains(needle), "{needle} in {s1}");
    }
    for (label, mode) in [("a plain answer", "plain"), ("a blank answer", "plainblank"), ("garbage", "garbage"), ("an array", "array"), ("bad agents", "badagents"), ("claude fails", "fail"), ("claude fails on stdout", "failout")] {
        step(label, mode, "degrade");
    }
    step("an item with nothing running", "", "fridge-old");
    step("the second item again", "", "wf2");
    step("a missing item", "", "nowhere");
    // a task that ended drops its line, the whole estimate once nothing it covered is going
    step("estimates again", "", "degrade");
    for p in [&r1, &r2] {
        let _ = p;
    }
    fs::write(r1.join("journal.jsonl"), journal.iter().filter(|e| e["agentId"] != "a1").map(|e| format!("{e}\n")).collect::<String>()).unwrap();
    compare_state(&ruby, &rust, "a task ended");
    tmux_on("qm-test-build-0", &["kill-server"]);
    tmux_on("qm-test-build-1", &["kill-server"]);
}

#[test]
fn the_status_board_mirror_and_answers_match_ruby() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let url = "https://claude.ai/artifact/abc123?x=1";
    let item = |root: &Path, slug: &str, extra: &str| fs::write(root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\ntitle: {slug}\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\n{extra}---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n")).unwrap();
    let root2 = base.path().join("root2");
    let answers = [w.root.join("answers"), root2.join("answers")];
    let write_items = |root: &Path, dir: &Path| {
        item(root, "board-item", &format!("board: '{url}'\nboard_answers_dir: {}\n", dir.display()));
        item(root, "board-noans", &format!("board: '{url}'\n"));
        item(root, "board-other", "board: https://claude.ai/code/artifact/0a1b2c3d-uuid/\n");
        item(root, "board-bare", "board: just-a-key/\n");
    };
    write_items(&w.root, &answers[0]);
    let tool = |input: serde_json::Value, response: serde_json::Value| serde_json::json!({"tool_name": "mcp__claude__ArtifactData", "tool_input": input, "tool_response": response});
    let committed = serde_json::json!("update committed");
    let line = |v: &serde_json::Value| format!("{v}\n");
    let set = |id: &str, data: serde_json::Value| tool(serde_json::json!({"action": "set", "url": url, "collection": "tasks", "doc_id": id, "data": data}), serde_json::json!("set committed"));
    let mut log = String::new();
    log += &line(&set("t1", serde_json::json!({"status": "RUNNING", "title": "First", "order": 2, "batch": "2026-01-01-a", "note": "n", "extra": 1})));
    log += &line(&set("t2", serde_json::json!({"status": "BLOCKED", "title": "Second", "order": 1.5, "batch": "2026-01-01-a", "batchLabel": "Batch A", "updatedAt": "writer clock"})));
    log += &line(&set("t3", serde_json::json!({"status": "blocked", "title": "Third", "batch": "2026-02-01-b", "dependsOn": ["t1"], "order": 1})));
    log += &line(&set("t4", serde_json::json!({"title": "No status, no batch", "gone": {"__delete__": true}, "nested": {"a": {"__delete__": true}, "b": 1}})));
    log += &line(&set("t5", serde_json::json!({"status": "DONE", "title": "Fifth", "batch": "2026-02-01-b", "order": 1})));
    log += &line(&serde_json::json!({"ts": "2026-03-01T10:00:00Z", "payload": set("t6", serde_json::json!({"status": "RUNNING", "title": "Wrapped"}))}));
    log += &line(&{ let mut v = set("t7", serde_json::json!({"status": "RUNNING", "title": "Stamped by ts"})); v["ts"] = serde_json::json!("2026-03-02T10:00:00Z"); v });
    log += &line(&tool(serde_json::json!({"action": "update", "url": url, "collection": "tasks", "doc_id": "t1", "data": {"note": {"__delete__": true}, "status": "BLOCKED", "meta": {"x": 1, "y": {"z": 2}}}}), committed.clone()));
    log += &line(&tool(serde_json::json!({"action": "update", "url": url, "collection": "tasks", "doc_id": "t1", "data": {"meta": {"y": {"z": {"__delete__": true}, "w": 3}}, "title": "First, renamed"}}), committed.clone()));
    log += &line(&tool(serde_json::json!({"action": "update", "url": url, "collection": "tasks", "doc_id": "fresh", "data": {"status": "RUNNING", "title": "created by update", "batch": "2026-03-01-c"}}), committed.clone()));
    log += &line(&tool(serde_json::json!({"action": "str_replace", "url": url, "collection": "tasks", "doc_id": "t2", "field": "title", "old_str": "Second", "new_str": "2nd"}), committed.clone()));
    log += &line(&tool(serde_json::json!({"action": "str_replace", "url": url, "collection": "tasks", "doc_id": "t5", "field": "title", "old_str": "th", "new_str": "TH"}), committed.clone()));
    log += &line(&set("t8", serde_json::json!({"status": "RUNNING", "title": "aa bb aa"})));
    log += &line(&set("t9", serde_json::json!({"status": "RUNNING", "title": "zz zz", "note": {"__delete__": true, "x": 1}})));
    log += &line(&tool(serde_json::json!({"action": "str_replace", "url": url, "collection": "tasks", "doc_id": "t9", "field": "title", "old_str": "zz", "new_str": "Y"}), committed.clone()));
    log += &line(&serde_json::json!({"payload": {"ts": "2026-03-03T10:00:00Z", "tool_name": "ArtifactData", "tool_input": {"action": "set", "url": url, "collection": "tasks", "doc_id": "t10", "data": {"status": "RUNNING", "title": "ts inside the payload"}}, "tool_response": "ok"}}));
    log += &line(&serde_json::json!({"ts": "2026-03-04T10:00:00Z", "payload": set("t11", serde_json::json!({"status": "RUNNING", "title": "own clock wins", "updatedAt": "own"}))}));
    log += &line(&set("plus+id@x:y~z", serde_json::json!({"status": "BLOCKED", "title": "odd id"})));
    log += &line(&tool(serde_json::json!({"action": "str_replace", "url": url, "collection": "tasks", "doc_id": "t8", "field": "title", "old_str": "aa", "new_str": "X"}), committed.clone()));
    log += &line(&tool(serde_json::json!({"action": "str_replace", "url": url, "collection": "tasks", "doc_id": "t8", "field": "title", "old_str": "aa", "new_str": "X", "replace_all": true}), committed.clone()));
    log += &line(&tool(serde_json::json!({"action": "str_replace", "url": url, "collection": "tasks", "doc_id": "t8", "field": "title", "old_str": "bb"}), committed.clone()));
    log += &line(&tool(serde_json::json!({"action": "str_replace", "url": url, "collection": "tasks", "doc_id": "nothere", "field": "title", "old_str": "a", "new_str": "b"}), committed.clone()));
    log += &line(&tool(serde_json::json!({"action": "batch", "url": url, "writes": [{"op": "set", "collection": "tasks", "doc_id": "b1", "data": {"status": "RUNNING", "title": "batched one", "batch": "2026-03-01-c", "order": 2}}, {"op": "update", "collection": "tasks", "doc_id": "b1", "data": {"order": 1}}, {"op": "set", "collection": "tasks", "doc_id": "b2", "data": {"status": "RUNNING", "title": "batched two", "batch": "2026-03-01-c"}}, {"op": "delete", "collection": "tasks", "doc_id": "b2"}, {"op": "set", "collection": "other", "doc_id": "o1", "data": {"title": "another collection"}}, {"op": "nonsense", "collection": "tasks", "doc_id": "b3", "data": {}}, "junk", {"collection": 5, "doc_id": "b4", "op": "set", "data": {}}]}), serde_json::json!({"content": [{"type": "text", "text": "batch committed"}]})));
    log += &line(&tool(serde_json::json!({"action": "batch", "url": url, "writes": "nope"}), committed.clone()));
    log += &line(&tool(serde_json::json!({"action": "delete", "url": url, "collection": "tasks", "doc_id": "t5"}), committed.clone()));
    // refused or failed writes leave no trace; reads neither
    for response in [serde_json::json!({"is_error": true}), serde_json::json!({"isError": true, "content": "x"}), serde_json::json!({"error": "denied"}), serde_json::json!("Error: version conflict"), serde_json::json!("<tool_use_error>nope</tool_use_error>"), serde_json::json!("nothing was written"), serde_json::json!("write refused"), serde_json::json!({"content": [{"type": "text", "text": "a conflict happened"}]}), serde_json::json!([{"text": "ok"}, "nothing is written"])] {
        log += &line(&tool(serde_json::json!({"action": "set", "url": url, "collection": "tasks", "doc_id": "refused", "data": {"title": "must not appear", "status": "RUNNING"}}), response));
    }
    for response in [serde_json::json!("committed write to row \"merge-conflict\""), serde_json::json!({"error": false, "content": "done"}), serde_json::json!({"text": "fine"}), serde_json::json!(null), serde_json::json!(5)] {
        log += &line(&tool(serde_json::json!({"action": "set", "url": url, "collection": "tasks", "doc_id": "ok-quoted", "data": {"title": "survives", "status": "RUNNING"}}), response));
    }
    log += &line(&tool(serde_json::json!({"action": "get", "url": url, "collection": "tasks", "doc_id": "t1"}), committed.clone()));
    log += &line(&serde_json::json!({"tool_name": "Bash", "tool_input": {"action": "set", "url": url, "collection": "tasks", "doc_id": "bash", "data": {"title": "not ArtifactData"}}, "tool_response": "ok"}));
    log += &line(&serde_json::json!({"tool_name": "ArtifactData", "tool_input": "text", "tool_response": "ok"}));
    log += &line(&tool(serde_json::json!({"action": "set", "url": "", "collection": "tasks", "doc_id": "nourl", "data": {"title": "x"}}), committed.clone()));
    log += "not json\n[1]\n\"s\"\n\n";
    // another board, a link with a path form, and data from a file
    let from_file = base.path().join("data.json");
    fs::write(&from_file, "{\"status\": \"RUNNING\", \"title\": \"from a file\", \"batch\": \"2026-03-01-c\"}").unwrap();
    fs::write(base.path().join("array.json"), "[1]").unwrap();
    log += &line(&tool(serde_json::json!({"action": "set", "url": url, "collection": "tasks", "doc_id": "file1", "file_path": from_file.display().to_string()}), committed.clone()));
    log += &line(&tool(serde_json::json!({"action": "set", "url": url, "collection": "tasks", "doc_id": "file2", "file_path": base.path().join("array.json").display().to_string()}), committed.clone()));
    log += &line(&tool(serde_json::json!({"action": "set", "url": url, "collection": "tasks", "doc_id": "file3", "file_path": "/nonexistent/file.json"}), committed.clone()));
    log += &line(&tool(serde_json::json!({"action": "set", "url": "https://claude.ai/code/artifact/0a1b2c3d-uuid", "collection": "tasks", "doc_id": "u1", "data": {"status": "BLOCKED", "title": "other board", "batch": "2026-01-01-a", "order": 1}}), committed.clone()));
    log += &line(&tool(serde_json::json!({"action": "set", "url": "just-a-key", "collection": "tasks", "doc_id": "k1", "data": {"status": "RUNNING", "title": "bare key"}}), committed.clone()));
    log += &line(&tool(serde_json::json!({"action": "set", "url": url, "collection": "tasks", "doc_id": "numeric-order", "data": {"title": "odd order", "order": "3", "batch": 7}}), committed.clone()));
    fs::write(w.root.join("board-writes.jsonl"), &log).unwrap();
    copy_dir(&w.root, &root2);
    write_items(&root2, &answers[1]);
    let env = vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string())];
    let ruby = start_env(false, &w.root, &w.home, &env);
    let rust = start_env(true, &root2, &w.home, &env);
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    for srv in [&ruby, &rust] {
        request(srv.port, "GET", "/api/state", &[], "", true);
    }
    std::thread::sleep(std::time::Duration::from_millis(900));
    let rows_of = |state: &str, slug: &str| -> String {
        let body = state.splitn(3, " | ").nth(2).unwrap();
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        v["items"].as_array().unwrap().iter().find(|i| i["slug"] == slug).map(|i| i["boardRows"].to_string()).unwrap_or_default()
    };
    let first = compare_state(&ruby, &rust, "the board folded");
    let rows = rows_of(&first, "board-item");
    for needle in ["\"First, renamed\"", "\"title\":\"2nd\"", "\"X  X\"", "survives", "batched one", "from a file", "created by update", "Wrapped", "\"updatedAt\":\"2026-03-02T10:00:00Z\"", "\"updatedAt\":\"writer clock\""] {
        assert!(rows.contains(needle), "{needle} in {rows}");
    }
    for needle in ["must not appear", "not ArtifactData", "batched two", "\"t5\""] {
        assert!(!rows.contains(needle), "{needle} in {rows}");
    }
    assert!(rows_of(&first, "board-other").contains("other board") && rows_of(&first, "board-bare").contains("bare key"));
    // appended lines are folded on the next build, a partial line only once it is whole
    let more = line(&set("late", serde_json::json!({"status": "RUNNING", "title": "appended later"})));
    let partial = line(&set("partial", serde_json::json!({"status": "RUNNING", "title": "split in two"})));
    for root in [&w.root, &root2] {
        let mut f = fs::OpenOptions::new().append(true).open(root.join("board-writes.jsonl")).unwrap();
        use std::io::Write;
        f.write_all(more.as_bytes()).unwrap();
        f.write_all(&partial.as_bytes()[..30]).unwrap();
    }
    let second = compare_state(&ruby, &rust, "lines appended");
    assert!(rows_of(&second, "board-item").contains("appended later") && !rows_of(&second, "board-item").contains("split in two"));
    for root in [&w.root, &root2] {
        let mut f = fs::OpenOptions::new().append(true).open(root.join("board-writes.jsonl")).unwrap();
        use std::io::Write;
        f.write_all(&partial.as_bytes()[30..]).unwrap();
    }
    let third = compare_state(&ruby, &rust, "the partial line completed");
    assert!(rows_of(&third, "board-item").contains("split in two"));
    // the answers: only a blocked row, once, as a file its sub-agent polls for
    let act = |slug: &str, args: serde_json::Value| serde_json::json!({"cmd": "board-answer", "slug": slug, "args": args}).to_string();
    let tree = |i: usize| -> String {
        let mut out = String::new();
        tree_text(&answers[i], "answers", &mut out);
        out
    };
    let long_id = "a".repeat(201);
    let max_id = "b".repeat(200);
    for (label, slug, args) in [
        ("answer a blocked row", "board-item", serde_json::json!({"doc_id": "t2", "text": "Go ahead"})),
        ("answer a lowercase blocked row", "board-item", serde_json::json!({"doc_id": "t3", "text": "multi\nline\n"})),
        ("answer a running row", "board-item", serde_json::json!({"doc_id": "t6", "text": "x"})),
        ("answer a row without status", "board-item", serde_json::json!({"doc_id": "t4", "text": "x"})),
        ("answer an updated blocked row", "board-item", serde_json::json!({"doc_id": "t1", "text": "  yes  "})),
        ("answer a row with every allowed character", "board-item", serde_json::json!({"doc_id": "plus+id@x:y~z", "text": "ok"})),
        ("answer a missing row", "board-item", serde_json::json!({"doc_id": "nope", "text": "x"})),
        ("answer with a bad id", "board-item", serde_json::json!({"doc_id": "../t2", "text": "x"})),
        ("answer with a dot id", "board-item", serde_json::json!({"doc_id": ".", "text": "x"})),
        ("answer with two dots", "board-item", serde_json::json!({"doc_id": "..", "text": "x"})),
        ("answer with a space id", "board-item", serde_json::json!({"doc_id": "a b", "text": "x"})),
        ("answer with a long id", "board-item", serde_json::json!({"doc_id": long_id, "text": "x"})),
        ("answer with the longest id", "board-item", serde_json::json!({"doc_id": max_id, "text": "x"})),
        ("answer with an empty text", "board-item", serde_json::json!({"doc_id": "t2", "text": "  \n"})),
        ("answer with no text", "board-item", serde_json::json!({"doc_id": "t2"})),
        ("answer with no id", "board-item", serde_json::json!({"text": "x"})),
        ("answer on an item without an answers dir", "board-noans", serde_json::json!({"doc_id": "t2", "text": "x"})),
        ("answer on an item without a board", "fridge-old", serde_json::json!({"doc_id": "t2", "text": "x"})),
        ("answer on a missing item", "nowhere", serde_json::json!({"doc_id": "t2", "text": "x"})),
        ("answer a numeric id", "board-item", serde_json::json!({"doc_id": 5, "text": 7})),
    ] {
        let body = act(slug, args);
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, &body, true), request(rust.port, "POST", "/api/action", &origin, &body, true));
        assert_eq!(shown(&a, ruby.port), shown(&b, rust.port), "{label}: reply to {body}");
        assert_eq!(tree(0).replace(&answers[0].display().to_string(), "A"), tree(1).replace(&answers[1].display().to_string(), "A"), "{label}: the answer files after {body}");
    }
    assert!(tree(1).contains("== answers/t2.md\nGo ahead\n"), "{}", tree(1));
    // a rotated (shrunken) file is refolded from the start
    for root in [&w.root, &root2] {
        fs::write(root.join("board-writes.jsonl"), line(&set("only", serde_json::json!({"status": "RUNNING", "title": "after rotation"})))).unwrap();
    }
    let rotated = compare_state(&ruby, &rust, "the file shrank");
    assert!(rows_of(&rotated, "board-item").contains("after rotation") && !rows_of(&rotated, "board-item").contains("First"), "{}", rows_of(&rotated, "board-item"));
}

const BENCH_CLAUDE: &str = r##"#!/bin/sh
# the stand-in agent and verdict model of a benchmark run
for a in "$@"; do [ ${#a} -lt 120 ] && echo "arg: $a" >> "$FAKE_DIR/claude.log"; done
echo "prompt starts: $(echo "$2" | head -1)" >> "$FAKE_DIR/claude.log"
case "$*" in
  *--allowedTools*)
    mode=$(cat "$FAKE_DIR/plan-mode" 2>/dev/null)
    cwd=$(cat "$FAKE_DIR/cwd")
    echo '{"type":"assistant","message":{"content":[{"type":"text","text":"thinking"},{"type":"tool_use","name":"Bash","input":{"command":"ls   -la"}}]}}'
    echo 'a stream line that is no JSON'
    echo '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/x/y.rb"}},{"type":"tool_use","name":"Odd"}]}}'
    case "$mode" in
      slow) sleep 6;;
    esac
    case "$mode" in
      fail) printf 'Traceback (most recent call last)\nRuntimeError: planner exploded in a very long way %s\n' "$(printf 'x%.0s' $(seq 1 400))" >&2; exit 1;;
      failwarn) printf 'warning: an old thing\nsomething with ArgumentError: bad\nlast line\n' >&2; exit 1;;
      failsilent) exit 1;;
      nobench) exit 0;;
    esac
    cp "$FAKE_DIR/bench.rb" "$QM_BENCH_DIR/bench.rb"
    echo '{"t":"plan","stages":["main","#5"],"cases":[{"subject":"S1","case":"c1","why":"first because"},{"subject":"S1","case":"c2","why":"second because"},"junk"]}' >> "$QM_BENCH_DIR/events.jsonl"
    echo '{"t":"subject","subject":"S1","about":"the first subject"}' >> "$QM_BENCH_DIR/events.jsonl"
    echo 'not json' >> "$QM_BENCH_DIR/events.jsonl"
    case "$mode" in
      norun) exit 0;;
      badrun) echo '{nope' > "$QM_BENCH_DIR/run.json"; exit 0;;
      nocwd) echo '{"cwd": "/nonexistent/place", "clone": false}' > "$QM_BENCH_DIR/run.json"; exit 0;;
      cwdnum) echo '{"cwd": 5}' > "$QM_BENCH_DIR/run.json"; exit 0;;
      clone) echo "{\"cwd\": \"$cwd\", \"clone\": true}" > "$QM_BENCH_DIR/run.json"; exit 0;;
    esac
    echo "{\"cwd\": \"$cwd\", \"clone\": false}" > "$QM_BENCH_DIR/run.json"
    exit 0;;
  *)
    mode=$(cat "$FAKE_DIR/verdict-mode" 2>/dev/null)
    case "$mode" in
      fail) echo "verdict model down" >&2; exit 2;;
      failout) echo "only stdout"; exit 2;;
      empty) exit 0;;
    esac
    echo "main to #5 is about 2x faster; no id differences."
    exit 0;;
esac
"##;

const BENCH_RAILS: &str = r##"#!/bin/sh
# the stand-in `bin/rails runner bench.rb`
mode=$(cat "$FAKE_DIR/rails-mode" 2>/dev/null)
echo "rails $* url=$DATABASE_URL run=$QM_BENCH_RUN" >> "$FAKE_DIR/rails.log"
case "$mode" in
  fail) printf 'warning: unused\nbench.rb:3:in `go'"'"': ArgumentError: boom in the bench\nnext line\n' >&2; exit 1;;
  failsilent) echo "only stdout"; exit 1;;
  empty) exit 0;;
  slow) sleep 6;;
  chatty) for n in $(seq 1 1400); do echo "log line $n ----------------------------------------------" >&2; done;;
  slowfail) sleep 3; echo "late failure" >&2; exit 1;;
esac
echo '{"t":"case","subject":"S1","case":"c1","stage":"main","ms":120.5,"queries":4,"plan":"p1.txt"}' >> "$QM_BENCH_DIR/events.jsonl"
echo '{"t":"case","subject":"S1","case":"c1","stage":"#5","ms":60,"queries":3}' >> "$QM_BENCH_DIR/events.jsonl"
echo '{"t":"ids","subject":"S1","case":"c1","stage":"#5","ids":"same","vs":"main"}' >> "$QM_BENCH_DIR/events.jsonl"
echo '{"t":"case","subject":"S1","case":"c2","stage":"main","timeout":true}' >> "$QM_BENCH_DIR/events.jsonl"
echo '{"t":"case","subject":"S2","case":"extra","stage":"main","ms":1}' >> "$QM_BENCH_DIR/events.jsonl"
mkdir -p "$QM_BENCH_DIR/plans"; echo "Seq Scan on things" > "$QM_BENCH_DIR/plans/p1.txt"
echo "| subject | case | main | #5 |"
echo "| S1 | c1 | 120.5 | 60 |"
exit 0
"##;

#[test]
fn benchmark_runs_match_ruby() {
    let base = tempfile::tempdir().unwrap();
    let bases = [base.path().join("a"), base.path().join("b")];
    let worlds = [workspace_world(&bases[0], false), workspace_world(&bases[1], false)];
    for (i, w) in worlds.iter().enumerate() {
        let b = &bases[i];
        let skill = b.join("skill");
        fs::create_dir_all(skill.join("bin")).unwrap();
        fs::write(skill.join("SKILL.md"), "skill").unwrap();
        let config = fs::read_to_string(w.root.join("config.yml")).unwrap();
        let root_line = format!("    root: {}\n", w.repo.display());
        assert!(config.contains(&root_line), "{config}");
        fs::write(w.root.join("config.yml"), config.replace(&root_line, &format!("{root_line}    benchmark:\n      database: bench_db\n      skill: {}\n      notes: |-\n        first note\n        second note\n", skill.display()))).unwrap();
        let wt = b.join("wt-w");
        fs::create_dir_all(wt.join("bin")).unwrap();
        executable(&wt.join("bin/rails"), BENCH_RAILS);
        fs::create_dir_all(b.join("fake")).unwrap();
        fs::write(b.join("fake/bench.rb"), "# a bench\n").unwrap();
        fs::write(b.join("fake/cwd"), wt.display().to_string()).unwrap();
        executable(&b.join("bin/claude"), BENCH_CLAUDE);
        executable(&b.join("bin/createdb"), "#!/bin/sh\necho \"createdb $*\" >> \"$FAKE_DIR/db.log\"\n[ -f \"$FAKE_DIR/createdb-fails\" ] && { echo \"database already exists\" >&2; exit 1; }\nexit 0\n");
        executable(&b.join("bin/dropdb"), "#!/bin/sh\necho \"dropdb $*\" >> \"$FAKE_DIR/db.log\"\n[ -f \"$FAKE_DIR/dropdb-fails\" ] && { echo \"ERROR: database is being accessed\" >&2; exit 1; }\nexit 0\n");
        executable(&b.join("bin/df"), "#!/bin/sh\necho 'Filesystem 1024-blocks Used Available Capacity Mounted'\nif [ -f \"$FAKE_DIR/small-disk\" ]; then echo '/dev/x 100 50 1048576 50% /'; else echo '/dev/x 100 50 104857600 50% /'; fi\n");
        executable(&b.join("bin/terminal-notifier"), "#!/bin/sh\nout=\"\"\nwhile [ $# -gt 0 ]; do case \"$1\" in -execute) shift;; *) out=\"$out|$1\";; esac; shift; done\necho \"$out\" >> \"$FAKE_DIR/notify.log\"\n");
    }
    let env = |i: usize| -> Vec<(&'static str, String)> {
        let w = &worlds[i];
        vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("PATH", format!("{}:{}:{}", bases[i].join("bin").display(), bases[i].join("tools").display(), std::env::var("PATH").unwrap())), ("FAKE_DIR", bases[i].join("fake").display().to_string()), ("FAKE_TOOL_LOG", bases[i].join("tool.log").display().to_string()), ("QM_TMUX_SOCKET", format!("qm-test-bench-{i}"))]
    };
    let ruby = start_env(false, &worlds[0].root, &worlds[0].home, &env(0));
    let rust = start_env(true, &worlds[1].root, &worlds[1].home, &env(1));
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let norm = |text: String, i: usize| -> String {
        let t = text.replace(&bases[i].display().to_string(), "BASE");
        let t = regex::Regex::new(r"20\d\d\d\d\d\d_\d\d\d\d\d\d(_\d+)?").unwrap().replace_all(&t, "RUNID").into_owned();
        let t = regex::Regex::new(r"20\d\d-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?(Z|[+-]\d\d:\d\d)").unwrap().replace_all(&t, "STAMP").into_owned();
        let t = regex::Regex::new(r#""pid":\s*\d+"#).unwrap().replace_all(&t, "\"pid\":PID").into_owned();
        let t = regex::Regex::new(r#""durationS":\d+"#).unwrap().replace_all(&t, "\"durationS\":N").into_owned();
        let shas: Vec<String> = git(&worlds[i].repo, &["rev-list", "--all"]).lines().map(String::from).collect();
        shas.iter().fold(t, |t, sha| t.replace(sha.as_str(), "SHA"))
    };
    let tree = |i: usize| -> String {
        let mut out = String::new();
        let mut dirs: Vec<PathBuf> = fs::read_dir(worlds[i].root.join("benchmarks")).map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect()).unwrap_or_default();
        dirs.sort();
        for d in dirs {
            for f in ["meta.json", "events.jsonl", "bench.rb", "run.json", "bench.out", "bench.log", "verdict.md", "pipeline.log", "agent.jsonl"] {
                if let Ok(t) = fs::read_to_string(d.join(f)) {
                    out += &format!("== {}/{f}\n{t}\n", d.file_name().unwrap().to_string_lossy());
                }
            }
            if let Ok(plans) = fs::read_dir(d.join("plans")) {
                let mut names: Vec<_> = plans.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
                names.sort();
                out += &format!("== plans: {names:?}\n");
            }
        }
        out += &fs::read_to_string(bases[i].join("fake/db.log")).unwrap_or_default();
        out += &fs::read_to_string(bases[i].join("fake/notify.log")).unwrap_or_default();
        out += &fs::read_to_string(bases[i].join("fake/claude.log")).unwrap_or_default();
        out += &fs::read_to_string(bases[i].join("fake/rails.log")).unwrap_or_default();
        let mut notes = String::new();
        tree_text(&worlds[i].root.join("work-items"), "work-items", &mut notes);
        out += notes.split("== ").filter(|b| b.starts_with("work-items/w-exist.md")).collect::<String>().as_str();
        let log = fs::read_to_string(worlds[i].root.join("qm.log")).unwrap_or_default();
        out += &regex::Regex::new(r"(?m)^\S+ \[").unwrap().replace_all(&log, "[");
        norm(out, i)
    };
    let set = |file: &str, value: &str| {
        for b in &bases {
            fs::write(b.join("fake").join(file), value).unwrap();
        }
    };
    let unset = |file: &str| {
        for b in &bases {
            let _ = fs::remove_file(b.join("fake").join(file));
        }
    };
    let act = |cmd: &str, slug: &str, args: serde_json::Value| serde_json::json!({"cmd": cmd, "slug": slug, "args": args}).to_string();
    let latest = |i: usize| -> Option<String> {
        let mut ids: Vec<String> = fs::read_dir(worlds[i].root.join("benchmarks")).map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| !n.starts_with("2099")).collect()).unwrap_or_default();
        ids.sort();
        ids.pop()
    };
    let wait_final = |what: &str| {
        for _ in 0..400 {
            let done = |i: usize| latest(i).is_some_and(|id| fs::read_to_string(worlds[i].root.join("benchmarks").join(&id).join("meta.json")).map(|t| ["\"done\"", "\"failed\"", "\"cancelled\""].iter().any(|s| t.contains(&format!("\"state\": {s}")))).unwrap_or(false));
            if done(0) && done(1) {
                std::thread::sleep(std::time::Duration::from_millis(250));
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("{what}: the runs did not end: {} / {}", tree(0), tree(1));
    };
    let post = |label: &str, body: &str| -> String {
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, body, true), request(rust.port, "POST", "/api/action", &origin, body, true));
        let (sa, sb) = (norm(shown(&a, ruby.port), 0), norm(shown(&b, rust.port), 1));
        assert_eq!(sa, sb, "{label}: reply to {body}");
        sa
    };
    let post_each = |label: &str, build: &dyn Fn(usize) -> String| -> String {
        let (ba, bb) = (build(0), build(1));
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, &ba, true), request(rust.port, "POST", "/api/action", &origin, &bb, true));
        let (sa, sb) = (norm(shown(&a, ruby.port), 0), norm(shown(&b, rust.port), 1));
        assert_eq!(sa, sb, "{label}: reply to {bb}");
        sa
    };
    let compare = |label: &str| {
        let (ta, tb) = (tree(0), tree(1));
        if ta != tb {
            let at = ta.bytes().zip(tb.bytes()).position(|(x, y)| x != y).unwrap_or(ta.len().min(tb.len()));
            let from = ta[..at].rfind('\n').map(|n| n.saturating_sub(200)).unwrap_or(0);
            let _ = (fs::write("/tmp/claude-0/-home-user-quartermaster/12fbb9d9-0a41-588c-9a2f-a40933a68cd9/scratchpad/ruby-tree.txt", &ta), fs::write("/tmp/claude-0/-home-user-quartermaster/12fbb9d9-0a41-588c-9a2f-a40933a68cd9/scratchpad/rust-tree.txt", &tb));
            panic!("{label}: first difference at byte {at} (trees in {})\nRuby: …{}\nRust: …{}", base.path().display(), &ta[from.min(ta.len())..(at + 200).min(ta.len())], &tb[from.min(tb.len())..(at + 200).min(tb.len())]);
        }
    };
    let get = |path: &str| {
        let (a, b) = (request(ruby.port, "GET", path, &[], "", true), request(rust.port, "GET", path, &[], "", true));
        let (sa, sb) = (norm(shown(&a, ruby.port), 0), norm(shown(&b, rust.port), 1));
        assert_eq!(sa, sb, "GET {path}");
        sa
    };
    let run_of = |i: usize| latest(i).unwrap_or_default();
    let flow = |label: &str, plan: &str, rails: &str, verdict: &str, args: serde_json::Value| {
        set("plan-mode", plan);
        set("rails-mode", rails);
        set("verdict-mode", verdict);
        let reply = post(label, &act("benchmark-start", "w-exist", args));
        if reply.contains("\"ok\":true") {
            wait_final(label);
        }
        compare(label);
        reply
    };
    // disabled, enabled without runs
    let first = get("/api/benchmark/w-exist");
    assert!(first.contains("\"enabled\":true") && first.contains("\"runs\":[]") && first.contains("\"run\":null"), "{first}");
    assert!(get("/api/benchmark/pr-no-project").contains("\"enabled\":false"));
    post("an item without benchmarks", &act("benchmark-start", "pr-no-project", serde_json::json!({})));
    post("a missing item", &act("benchmark-start", "nowhere", serde_json::json!({})));
    post("a bad model", &act("benchmark-start", "w-exist", serde_json::json!({"model": "gpt"})));
    post("a model that is no string", &act("benchmark-start", "w-exist", serde_json::json!({"model": 5})));
    post("an unknown branch", &act("benchmark-start", "w-exist", serde_json::json!({"branch": "nowhere"})));
    // a run that works
    let ok = flow("a run that works", "", "", "", serde_json::json!({"branch": "feat-w", "request": "  look at the thing  ", "model": "sonnet"}));
    assert!(ok.contains("\"run\":\"RUNID\""), "{ok}");
    let report = get("/api/benchmark/w-exist?branch=feat-w");
    for needle in ["\"state\":\"done\"", "\"verdict\":\"main to #5", "\"subjects\":[{\"subject\":\"S1\"", "\"timeout\":true", "\"ids\":\"same\"", "\"logs\":[", "\"canRerun\":true", "\"S2\""] {
        assert!(report.contains(needle), "{needle} in {report}");
    }
    let (ida, idb) = (run_of(0), run_of(1));
    for (file, route) in [("p1.txt", "benchmark-plan"), ("../meta.json", "benchmark-plan"), ("nothere", "benchmark-plan"), ("", "benchmark-plan")] {
        let (a, b) = (request(ruby.port, "GET", &format!("/api/{route}/{ida}/{file}"), &[], "", true), request(rust.port, "GET", &format!("/api/{route}/{idb}/{file}"), &[], "", true));
        assert_eq!(norm(shown(&a, ruby.port), 0), norm(shown(&b, rust.port), 1), "plan {file}");
    }
    for name in ["bench.log", "pipeline.log", "bench.out", "run.json", "bench.rb", "agent.jsonl", "nothere", ""] {
        let (a, b) = (request(ruby.port, "GET", &format!("/api/benchmark-log/{ida}/{name}"), &[], "", true), request(rust.port, "GET", &format!("/api/benchmark-log/{idb}/{name}"), &[], "", true));
        assert_eq!(norm(shown(&a, ruby.port), 0), norm(shown(&b, rust.port), 1), "log {name}");
    }
    for path in ["/api/benchmark-plan/bad/p1.txt", "/api/benchmark-plan/", "/api/benchmark-log/20990101_000000/bench.log", "/api/benchmark/w-exist?branch=feat-w&run=nope", "/api/benchmark/w-exist?branch=feat-w&run=20990101_000000", "/api/benchmark/nowhere", "/api/benchmark/w-exist?branch=other"] {
        get(path);
    }
    // the state's benchmark fields
    let state = |i: usize, port: u16| norm(String::from_utf8_lossy(&request(port, "GET", "/api/state", &[], "", true).body).into_owned(), i);
    for srv in [(0usize, &ruby), (1, &rust)] {
        let s = state(srv.0, srv.1.port);
        assert!(s.contains("\"benchmark\":null") && s.contains("\"benchmarkable\":true") && s.contains("\"benchmarked\":{\"feat-w\":{\"id\":\"RUNID\""), "{s}");
    }
    // a re-run of its script
    post_each("a re-run", &|i| act("benchmark-rerun", "w-exist", serde_json::json!({"run": if i == 0 { ida.clone() } else { idb.clone() }})));
    wait_final("the re-run");
    compare("the re-run");
    post("a re-run of nothing", &act("benchmark-rerun", "w-exist", serde_json::json!({"run": "20990101_000000"})));
    post("a re-run of a bad id", &act("benchmark-rerun", "w-exist", serde_json::json!({"run": "../x"})));
    // failures at every step
    for (label, plan, rails, verdict) in [
        ("the planner fails", "fail", "", ""), ("the planner fails with a warning first", "failwarn", "", ""), ("the planner fails silently", "failsilent", "", ""),
        ("no bench.rb", "nobench", "", ""), ("no run.json", "norun", "", ""), ("run.json that does not parse", "badrun", "", ""), ("a cwd that is gone", "nocwd", "", ""), ("a cwd that is a number", "cwdnum", "", ""),
        ("rails fails", "", "fail", ""), ("rails fails silently", "", "failsilent", ""), ("rails prints nothing", "", "empty", ""),
        ("the verdict fails", "", "", "fail"), ("the verdict fails on stdout", "", "", "failout"), ("the verdict is empty", "", "", "empty"),
    ] {
        flow(label, plan, rails, verdict, serde_json::json!({"branch": "feat-w"}));
    }
    flow("a chatty script", "", "chatty", "", serde_json::json!({"branch": "feat-w"}));
    for name in ["bench.log"] {
        let (a, b) = (request(ruby.port, "GET", &format!("/api/benchmark-log/{}/{name}", run_of(0)), &[], "", true), request(rust.port, "GET", &format!("/api/benchmark-log/{}/{name}", run_of(1)), &[], "", true));
        assert_eq!(norm(shown(&a, ruby.port), 0), norm(shown(&b, rust.port), 1), "the tail of a long log");
        assert!(shown(&b, rust.port).len() > 60_000 && !shown(&b, rust.port).contains("log line 1 "), "only the last 64 KB");
    }
    // a clone: room check, createdb, dropdb
    set("small-disk", "");
    flow("a clone without room", "clone", "", "", serde_json::json!({}));
    unset("small-disk");
    set("createdb-fails", "");
    flow("createdb fails", "clone", "", "", serde_json::json!({}));
    unset("createdb-fails");
    flow("a clone that is dropped", "clone", "", "", serde_json::json!({}));
    set("dropdb-fails", "");
    flow("a clone that cannot be dropped", "clone", "", "", serde_json::json!({}));
    flow("a clone that cannot be dropped, and rails fails", "clone", "fail", "", serde_json::json!({}));
    unset("dropdb-fails");
    // runs whose pipeline is not what meta.json says
    let by_hand = |i: usize, id: &str, state: &str, pid: &str, started: &str| {
        let dir = worlds[i].root.join("benchmarks").join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("meta.json"), format!("{{\n  \"v\": 1,\n  \"id\": \"{id}\",\n  \"slug\": \"w-exist\",\n  \"project\": \"demo\",\n  \"database\": \"bench_db\",\n  \"root\": \"{}\",\n  \"checkout\": \"{}\",\n  \"branch\": \"hand\",\n  \"request\": \"\",\n  \"model\": \"opus\",\n  \"stages\": [],\n  \"head\": \"abc\",\n  \"startedAt\": \"{started}\",\n  \"finishedAt\": null,\n  \"state\": \"{state}\",\n  \"error\": null,\n  \"pid\": {pid}\n}}", worlds[i].repo.display(), worlds[i].repo.display())).unwrap();
    };
    let old_start = (chrono::Utc::now() - chrono::Duration::seconds(3600)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    for i in 0..2 {
        by_hand(i, "20990101_000001", "planning", "999999", &old_start);
        by_hand(i, "20990101_000002", "measuring", &std::process::id().to_string(), &old_start);
        by_hand(i, "20990101_000003", "planning", "null", &old_start);
    }
    let hand = get("/api/benchmark/w-exist?branch=hand");
    for needle in ["\"busy\":null", "pipeline died"] {
        assert!(hand.contains(needle), "{needle} in {hand}");
    }
    let hand_state = |i: usize, port: u16| norm(String::from_utf8_lossy(&request(port, "GET", "/api/state", &[], "", true).body).into_owned(), i);
    assert!(hand_state(0, ruby.port).contains("\"benchmark\":null") && hand_state(1, rust.port).contains("\"benchmark\":null"));
    // one at a time, machine-wide; a cancel
    set("plan-mode", "slow");
    set("rails-mode", "");
    set("verdict-mode", "");
    post("a slow run starts", &act("benchmark-start", "w-exist", serde_json::json!({})));
    std::thread::sleep(std::time::Duration::from_millis(600));
    let busy = get("/api/benchmark/w-exist?branch=feat-w");
    assert!(busy.contains("\"state\":\"planning\"") && busy.contains("\"activity\":\"Odd\""), "{busy}");
    let busy_reply = post("a second start while busy", &act("benchmark-start", "w-exist", serde_json::json!({})));
    assert!(busy_reply.contains("\"busy\":{"), "{busy_reply}");
    post_each("a re-run while busy", &|i| act("benchmark-rerun", "w-exist", serde_json::json!({"run": if i == 0 { ida.clone() } else { idb.clone() }})));
    let s = state(1, rust.port);
    assert!(s.contains("\"benchmark\":{\"id\":\"RUNID\""), "{s}");
    set("dropdb-fails", "");
    post_each("a cancel while planning, dropdb failing", &|i| act("benchmark-cancel", "w-exist", serde_json::json!({"run": run_of(i)})));
    std::thread::sleep(std::time::Duration::from_millis(1500));
    compare("a run cancelled while planning");
    unset("dropdb-fails");
    set("plan-mode", "");
    set("rails-mode", "slowfail");
    post("a run to cancel while measuring", &act("benchmark-start", "w-exist", serde_json::json!({})));
    for _ in 0..80 {
        if fs::read_to_string(bases[0].join("root/benchmarks").join(run_of(0)).join("meta.json")).is_ok_and(|t| t.contains("\"measuring\"")) && fs::read_to_string(bases[1].join("root/benchmarks").join(run_of(1)).join("meta.json")).is_ok_and(|t| t.contains("\"measuring\"")) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    set("dropdb-fails", "");
    post_each("a cancel while measuring, dropdb failing", &|i| act("benchmark-cancel", "w-exist", serde_json::json!({"run": run_of(i)})));
    std::thread::sleep(std::time::Duration::from_millis(4500));
    compare("a run cancelled while measuring");
    unset("dropdb-fails");
    set("plan-mode", "slow");
    set("rails-mode", "");
    post("a slow run for the plain cancel", &act("benchmark-start", "w-exist", serde_json::json!({})));
    std::thread::sleep(std::time::Duration::from_millis(500));
    post_each("a cancel", &|i| act("benchmark-cancel", "w-exist", serde_json::json!({"run": run_of(i)})));
    std::thread::sleep(std::time::Duration::from_millis(1500));
    compare("a cancelled run");
    post_each("a cancel of a cancelled run", &|i| act("benchmark-cancel", "w-exist", serde_json::json!({"run": run_of(i)})));
    post("a cancel of nothing", &act("benchmark-cancel", "w-exist", serde_json::json!({"run": "20990101_000000"})));
    // a branch that moved: the report says so and a re-run is refused
    for (i, _) in worlds.iter().enumerate() {
        let wt = bases[i].join("wt-w");
        fs::write(wt.join("moved.txt"), "moved\n").unwrap();
        git(&wt, &["add", "moved.txt"]);
        git(&wt, &["commit", "-q", "-m", "moved"]);
    }
    set("plan-mode", "");
    let moved = get("/api/benchmark/w-exist?branch=feat-w&run={FIRST}".replace("{FIRST}", &ida).as_str());
    assert!(moved.contains("\"moved\":true") && moved.contains("\"canRerun\":false"), "{moved}");
    let refused = post_each("a re-run after the branch moved", &|i| act("benchmark-rerun", "w-exist", serde_json::json!({"run": if i == 0 { ida.clone() } else { idb.clone() }})));
    assert!(refused.contains("moved since it ran"), "{refused}");
    // two runs that both look live: the report and the state name different ones
    for i in 0..2 {
        by_hand(i, "20990101_000004", "planning", "null", &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        by_hand(i, "20990101_000005", "planning", "null", &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    }
    let two = get("/api/benchmark/w-exist?branch=hand");
    assert!(two.contains("\"busy\":{\"id\":\"RUNID\""), "{two}");
    post("a start while two look live", &act("benchmark-start", "w-exist", serde_json::json!({})));
    let both = hand_state(1, rust.port);
    assert!(both.contains("\"benchmark\":{\"id\":\"RUNID\""), "{both}");
    let busy_of = |t: &str| regex::Regex::new(r#""benchmark":(\{[^}]*\}|null)"#).unwrap().captures(t).map(|c| c[1].to_string());
    assert_eq!(busy_of(&hand_state(0, ruby.port)), busy_of(&both), "the state with two live runs");
    for i in 0..2 {
        tmux_on(&format!("qm-test-bench-{i}"), &["kill-server"]);
    }
}

#[test]
fn distilling_matches_ruby() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let transcripts = w.transcripts.join("p");
    let big = |n: usize| (0..n).map(|i| format!("{{\"line\":{i},\"pad\":\"{}\"}}\n", "x".repeat(60))).collect::<String>();
    let session = |id: &str, text: &str, pid: u32| {
        let path = transcripts.join(format!("{id}.jsonl"));
        fs::write(&path, text).unwrap();
        let log = fs::read_to_string(w.root.join("sessions.jsonl")).unwrap();
        fs::write(w.root.join("sessions.jsonl"), log + &format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"/srv/app\",\"pid\":{pid},\"transcript_path\":\"{}\",\"ts\":\"2026-03-01T00:00:00+00:00\"}}\n", path.display())).unwrap();
    };
    session("dist-big", &big(8000), 999999); // about 560 KB: the tail is cut
    session("dist-small", "{\"line\":1}\n", 999999);
    session("dist-newer", "{\"line\":2}\n", 999999);
    session("dist-tie-a", "{\"line\":\"a\"}\n", 999999);
    session("dist-tie-b", "{\"line\":\"b\"}\n", 999999);
    let log = fs::read_to_string(w.root.join("sessions.jsonl")).unwrap();
    fs::write(w.root.join("sessions.jsonl"), log + "{\"v\":1,\"event\":\"end\",\"session_id\":\"dist-newer\",\"ts\":\"2026-03-05T00:00:00+00:00\"}\n{\"v\":1,\"event\":\"end\",\"session_id\":\"dist-tie-a\",\"ts\":\"2026-03-06T00:00:00+00:00\"}\n{\"v\":1,\"event\":\"end\",\"session_id\":\"dist-tie-b\",\"ts\":\"2026-03-06T00:00:00+00:00\"}\n").unwrap();
    let item = |slug: &str, sessions: &[&str], body: &str| {
        let list: String = sessions.iter().map(|s| format!("- id: {s}\n")).collect();
        fs::write(w.root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\ntitle: {slug}\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\nsessions:\n{list}---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n{body}")).unwrap();
    };
    item("dist-item", &["dist-big", "dist-small", "dist-newer"], "");
    item("dist-tie", &["dist-tie-a", "dist-tie-b"], "");
    item("dist-done", &["dist-small", "dist-newer"], "\n### 2026-02-01 — session dist-newer\n\nolder snapshot\n\n### 2026-02-02 — session other\n\nkept\n");
    item("dist-none", &["d-nolog", "d-notranscript"], "");
    let bin = base.path().join("bin2");
    executable(&bin.join("claude"), "#!/bin/sh\nn=$(ls \"$FAKE_DIR\" | grep -c '^stdin-')\ncat > \"$FAKE_DIR/stdin-$n\"\necho \"$* size=$(wc -c < \"$FAKE_DIR/stdin-$n\") head=$(head -c 60 \"$FAKE_DIR/stdin-$n\" | head -1)\" | cut -c1-200 >> \"$FAKE_DIR/claude.log\"\nsleep \"$(cat \"$FAKE_DIR/delay\" 2>/dev/null || echo 0)\"\nmode=$(cat \"$FAKE_DIR/mode\" 2>/dev/null)\ncase \"$mode\" in\n  fail) echo 'quota exceeded' >&2; exit 3;;\n  failout) echo 'only stdout'; exit 3;;\n  empty) exit 0;;\n  toolong) echo 'Prompt is too long'; exit 1;;\n  toolong-once) if [ \"$n\" -lt \"$(cat \"$FAKE_DIR/short-until\")\" ]; then echo 'prompt is too long (x tokens)'; exit 1; fi;;\nesac\nprintf '  The goal was X.\\n\\nNext: Y.  \\n'\n");
    let root2 = base.path().join("root2");
    copy_dir(&w.root, &root2);
    let fakes = [base.path().join("fake-a"), base.path().join("fake-b")];
    for f in &fakes {
        fs::create_dir_all(f).unwrap();
    }
    let env = |i: usize| -> Vec<(&'static str, String)> { vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("PATH", format!("{}:{}:{}", bin.display(), w.bin.display(), std::env::var("PATH").unwrap())), ("FAKE_GH_DATA", w.gh_data.display().to_string()), ("FAKE_GH_LOG", base.path().join(format!("gh{i}.log")).display().to_string()), ("FAKE_DIR", fakes[i].display().to_string()), ("QM_TMUX_SOCKET", format!("qm-test-distill-{i}"))] };
    let ruby = start_env(false, &w.root, &w.home, &env(0));
    let rust = start_env(true, &root2, &w.home, &env(1));
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let roots = [&w.root, &root2];
    let norm = |t: String, i: usize| t.replace(&roots[i].display().to_string(), "ROOT").replace(&base.path().display().to_string(), "BASE");
    let items_of = |i: usize| -> String {
        let mut out = String::new();
        tree_text(&roots[i].join("work-items"), "work-items", &mut out);
        let log = fs::read_to_string(roots[i].join("qm.log")).unwrap_or_default();
        out += &regex::Regex::new(r"(?m)^\S+ \[").unwrap().replace_all(&log, "[");
        let t = norm(out, i);
        regex::Regex::new(r"(?m)^### 20\d\d-\d\d-\d\d").unwrap().replace_all(&t, "### DATE").into_owned()
    };
    let finished = |i: usize, port: u16| -> String {
        let v: serde_json::Value = serde_json::from_slice(&request(port, "GET", "/api/state", &[], "", true).body).unwrap_or_default();
        norm(format!("{} / {}", v["distilling"], v["distilled"]), i)
    };
    let set = |file: &str, value: &str| {
        for f in &fakes {
            fs::write(f.join(file), value).unwrap();
        }
    };
    let act = |slug: &str, args: serde_json::Value| serde_json::json!({"cmd": "distill", "slug": slug, "args": args}).to_string();
    let step = |label: &str, slug: &str, args: serde_json::Value, wait_ms: u64| {
        let body = act(slug, args);
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, &body, true), request(rust.port, "POST", "/api/action", &origin, &body, true));
        assert_eq!(norm(shown(&a, ruby.port), 0), norm(shown(&b, rust.port), 1), "{label}: reply to {body}");
        std::thread::sleep(std::time::Duration::from_millis(wait_ms));
        assert_eq!(finished(0, ruby.port), finished(1, rust.port), "{label}: the state's distill fields");
        assert_eq!(items_of(0), items_of(1), "{label}: files after {body}");
        let log = |i: usize| norm(fs::read_to_string(fakes[i].join("claude.log")).unwrap_or_default(), i);
        assert_eq!(log(0), log(1), "{label}: what the model was asked");
    };
    set("mode", "");
    step("an explicit session", "dist-item", serde_json::json!({"session": "dist-small"}), 800);
    step("the same again (needs a confirmation)", "dist-item", serde_json::json!({"session": "dist-small"}), 300);
    step("forced", "dist-item", serde_json::json!({"session": "dist-small", "force": true}), 800);
    step("a big transcript is cut to its tail", "dist-item", serde_json::json!({"session": "dist-big"}), 800);
    step("bare: the newest resumable one", "dist-item", serde_json::json!({}), 800);
    step("bare with two equally recent sessions", "dist-tie", serde_json::json!({}), 800);
    step("bare, already distilled", "dist-done", serde_json::json!({}), 300);
    step("bare with expiring transcripts (several)", "degrade", serde_json::json!({}), 1200);
    step("a session that isn't attached", "dist-item", serde_json::json!({"session": "d-soon"}), 300);
    step("a session without a transcript", "dist-item", serde_json::json!({"session": "d-nolog"}), 300);
    step("no session has a transcript", "dist-none", serde_json::json!({}), 300);
    step("a missing item", "nowhere", serde_json::json!({}), 100);
    step("a session that is no string", "dist-item", serde_json::json!({"session": 5}), 100);
    set("mode", "fail");
    step("claude fails", "dist-item", serde_json::json!({"session": "dist-newer"}), 800);
    set("mode", "failout");
    step("claude fails on stdout", "dist-item", serde_json::json!({"session": "dist-newer"}), 800);
    set("mode", "empty");
    step("claude says nothing", "dist-item", serde_json::json!({"session": "dist-newer"}), 800);
    set("mode", "toolong");
    step("the prompt is always too long", "dist-item", serde_json::json!({"session": "dist-big"}), 1500);
    set("mode", "toolong-once");
    for f in &fakes {
        let n = fs::read_dir(f).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("stdin-")).count();
        fs::write(f.join("short-until"), (n + 2).to_string()).unwrap();
    }
    step("too long twice, then it fits", "dist-item", serde_json::json!({"session": "dist-big", "force": true}), 1500);
    set("mode", "");
    set("delay", "1");
    let body = act("dist-item", serde_json::json!({"session": "dist-newer", "force": true}));
    let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, &body, true), request(rust.port, "POST", "/api/action", &origin, &body, true));
    assert_eq!(norm(shown(&a, ruby.port), 0), norm(shown(&b, rust.port), 1));
    let again = (request(ruby.port, "POST", "/api/action", &origin, &body, true), request(rust.port, "POST", "/api/action", &origin, &body, true));
    assert_eq!(norm(shown(&again.0, ruby.port), 0), norm(shown(&again.1, rust.port), 1), "a second distill while one runs");
    assert!(String::from_utf8_lossy(&again.1.body).contains("already being distilled"));
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(finished(0, ruby.port), finished(1, rust.port));
    for i in 0..2 {
        tmux_on(&format!("qm-test-distill-{i}"), &["kill-server"]);
    }
}

const ROTATION_CHAT: &str = r##"#!/bin/sh
# a stand-in chat: registers its session, answers the wrap-up prompt with a Stop and ends on /exit
sid="${FAKE_SID:-fresh-$$}"
printf '{"v":1,"event":"start","session_id":"%s","cwd":"%s","pid":%s,"transcript_path":"%s","ts":"%s"}\n' "$sid" "$(pwd)" "$$" "$FAKE_T/$sid.jsonl" "$(date -u +%Y-%m-%dT%H:%M:%S+00:00)" >> "$QM_ROOT/sessions.jsonl"
echo '{}' > "$FAKE_T/$sid.jsonl"
echo "started $sid item=${QM_ITEM-unset} args=$*" >> "$FAKE_REC"
ev() { printf '{"v":1,"ts":"%s","event":"%s","session_id":"%s"}\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$1" "$sid" >> "$QM_ROOT/events.jsonl"; }
while IFS= read -r line; do
  echo "$sid got: $line" >> "$FAKE_REC"
  case "$line" in
    "/exit") echo "$sid exiting" >> "$FAKE_REC"; exit 0;;
    Wrap*|"This work item"*) ev UserPromptSubmit; sleep 1; ev Stop;;
  esac
done
"##;

#[test]
fn rotations_match_ruby() {
    let base = tempfile::tempdir().unwrap();
    let w = attention_world(base.path(), std::time::SystemTime::now(), None);
    let chat = base.path().join("rot-chat");
    executable(&chat, ROTATION_CHAT);
    let bin = base.path().join("bin2");
    executable(&bin.join("claude"), "#!/bin/sh\ncat > /dev/null\necho \"$*\" | cut -c1-80 >> \"$FAKE_DIR/claude.log\"\nprintf 'a distilled snapshot\\n'\n");
    let sessions = ["a", "b", "c", "d", "e", "f", "g", "h", "mid", "out", "nocap"];
    let item = |root: &Path, slug: &str, sids: &[&str], notes: &str| {
        let list: String = sids.iter().map(|s| format!("- id: {s}\n")).collect();
        fs::write(root.join(format!("work-items/{slug}.md")), format!("---\nv: 1\ntitle: {slug}\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\nsessions:\n{list}---\n\n## Description\n\nd\n\n## Notes\n{notes}\n## Snapshots\n")).unwrap();
    };
    let notes = "\n- 2026-03-01 [decision]: chose X\n- 2026-03-02 [open]: still to do\n- 2026-03-03 [question]: is it Y?\n- 2026-03-04 [history]: not carried\n";
    let root2 = base.path().join("root2");
    let fakes = [base.path().join("fake-a"), base.path().join("fake-b")];
    let t = base.path().join("t");
    fs::create_dir_all(&t).unwrap();
    for f in &fakes {
        fs::create_dir_all(f).unwrap();
    }
    let roots = [w.root.clone(), root2.clone()];
    let slugs = ["rot-live", "rot-close", "rot-finish", "rot-self", "rot-dead", "rot-dead2"];
    // sessions that exist in the log, one per chat plus special ones
    let mut log = fs::read_to_string(w.root.join("sessions.jsonl")).unwrap();
    for id in ["dead-1", "dead-2", "dead-3", "dead-4", "mid-live", "out-live", "nocap-live"] {
        let path = w.transcripts.join(format!("p/{id}.jsonl"));
        fs::write(&path, "{}\n").unwrap();
        let pid = if id.ends_with("-live") { std::process::id() } else { 999999 };
        log += &format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"/srv/app\",\"pid\":{pid},\"transcript_path\":\"{}\",\"ts\":\"{}\"}}\n", path.display(), if id == "out-live" { "2030-01-01T00:00:00+00:00" } else { "2026-03-01T00:00:00+00:00" });
    }
    fs::write(w.root.join("sessions.jsonl"), log).unwrap();
    for root in [&w.root] {
        item(root, "rot-live", &["rot-live-sid"], notes);
        item(root, "rot-close", &["rot-close-sid"], notes);
        item(root, "rot-finish", &["rot-finish-sid"], notes);
        item(root, "rot-self", &["rot-self-sid"], notes);
        item(root, "rot-dead", &["dead-1"], notes);
        item(root, "rot-dead2", &["dead-2"], "");
        item(root, "rot-mid", &["mid-live"], notes);
        item(root, "rot-out", &["out-live"], notes);
        item(root, "rot-none", &[], notes);
        item(root, "rot-two", &["out-live", "rot-two-sid"], notes);
        item(root, "rot-outpane", &["out-live"], notes);
        fs::write(root.join("work-items/rot-dead3.md"), "---\nv: 1\ntitle: rot-dead3\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\nsessions:\n- id: dead-3\n---\n\n## Description\n\nd\n\n## Notes\n\n- 2026-03-01 [decision]: x\n\n## Snapshots\n\n### 2026-02-01 — session dead-3\n\nalready distilled\n").unwrap();
        fs::write(root.join("work-items/rot-dead4.md"), "---\nv: 1\ntitle: rot-dead4\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\nsessions:\n- id: dead-4\n---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n\n### 2026-02-01 — session dead-4extra\n\nanother session's\n").unwrap();
        item(root, "rot-nocap", &["nocap-never-captured"], notes);
    }
    let _ = (sessions, slugs);
    copy_dir(&w.root, &root2);
    let sockets = ["qm-test-rot-0", "qm-test-rot-1"];
    let env = |i: usize| -> Vec<(&'static str, String)> { vec![("HOME", w.home.display().to_string()), ("QM_TRANSCRIPTS_DIR", w.transcripts.display().to_string()), ("PATH", format!("{}:{}:{}", bin.display(), w.bin.display(), std::env::var("PATH").unwrap())), ("FAKE_GH_DATA", w.gh_data.display().to_string()), ("FAKE_GH_LOG", base.path().join(format!("gh{i}.log")).display().to_string()), ("FAKE_DIR", fakes[i].display().to_string()), ("FAKE_T", t.display().to_string()), ("FAKE_REC", fakes[i].join("rec.log").display().to_string()), ("QM_CHAT_COMMAND", chat.display().to_string()), ("QM_TMUX_SOCKET", sockets[i].to_string())] };
    for s in sockets {
        tmux_on(s, &["kill-server"]);
    }
    // a chat in its pane, started as the GUI would have: the tmux server holds the environment the chats inherit
    let start_chat = |i: usize, slug: &str, sid: &str| {
        let status = Command::new("tmux").args(["-L", sockets[i], "new-session", "-d", "-s", slug, "-n", slug, "-x", "160", "-y", "48", "env", &format!("FAKE_SID={sid}"), "sh", chat.to_str().unwrap()]).env_remove("TMUX").env("QM_ROOT", roots[i].display().to_string()).env("FAKE_T", t.display().to_string()).env("FAKE_REC", fakes[i].join("rec.log").display().to_string()).status().unwrap();
        assert!(status.success());
    };
    let ruby = start_env(false, &w.root, &w.home, &env(0));
    let rust = start_env(true, &root2, &w.home, &env(1));
    let origin = [("Origin", "http://127.0.0.1:PORT".to_string()), ("Content-Type", "application/json".to_string())];
    let norm = |text: String, i: usize| -> String {
        let t = text.replace(&roots[i].display().to_string(), "ROOT").replace(&base.path().display().to_string(), "BASE").replace(&format!("fake-{}", if i == 0 { "a" } else { "b" }), "fake-X");
        let t = regex::Regex::new(r"20\d\d-\d\d-\d\dT\d\d:\d\d:\d\d(Z|[+-]\d\d:\d\d)").unwrap().replace_all(&t, "STAMP").into_owned();
        regex::Regex::new(r"fresh-\d+").unwrap().replace_all(&t, "fresh-PID").into_owned()
    };
    let state = |i: usize| -> String {
        let mut out = String::new();
        let mut notes = String::new();
        tree_text(&roots[i].join("work-items"), "work-items", &mut notes);
        out += notes.split("== ").filter(|b| b.starts_with("work-items/rot-")).collect::<String>().as_str();
        out += &fs::read_to_string(fakes[i].join("rec.log")).unwrap_or_default().lines().filter(|l| !l.contains("fresh-")).map(|l| format!("{l}\n")).collect::<String>();
        out += &fs::read_to_string(fakes[i].join("claude.log")).unwrap_or_default();
        out += &tmux_picture(sockets[i]);
        let log = fs::read_to_string(roots[i].join("qm.log")).unwrap_or_default();
        out += &regex::Regex::new(r"(?m)^\S+ \[").unwrap().replace_all(&log, "[");
        norm(out, i)
    };
    let results: std::cell::RefCell<[Vec<String>; 2]> = std::cell::RefCell::new([vec![], vec![]]);
    let jobs_running = |i: usize, port: u16| -> bool {
        let v: serde_json::Value = serde_json::from_slice(&request(port, "GET", "/api/state", &[], "", true).body).unwrap_or_default();
        if let Some(list) = v["finished"].as_array() {
            results.borrow_mut()[i].extend(list.iter().map(|r| norm(r.to_string(), i)));
        }
        v["jobs"].as_array().is_some_and(|j| !j.is_empty())
    };
    let act = |slug: &str, args: serde_json::Value| serde_json::json!({"cmd": "rotate", "slug": slug, "args": args}).to_string();
    let post = |label: &str, body: &str| -> String {
        let (a, b) = (request(ruby.port, "POST", "/api/action", &origin, body, true), request(rust.port, "POST", "/api/action", &origin, body, true));
        let (sa, sb) = (norm(shown(&a, ruby.port), 0), norm(shown(&b, rust.port), 1));
        assert_eq!(sa, sb, "{label}: reply to {body}");
        sa
    };
    let settle = |label: &str| {
        for _ in 0..300 {
            std::thread::sleep(std::time::Duration::from_millis(200));
            if !jobs_running(0, ruby.port) && !jobs_running(1, rust.port) {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(400));
        let (sa, sb) = (state(0), state(1));
        if sa != sb {
            let at = sa.bytes().zip(sb.bytes()).position(|(x, y)| x != y).unwrap_or(sa.len().min(sb.len()));
            panic!("{label}: first difference at byte {at}\nRuby: …{}\nRust: …{}", &sa[at.saturating_sub(300)..(at + 300).min(sa.len())], &sb[at.saturating_sub(300)..(at + 300).min(sb.len())]);
        }
        let (ra, rb) = { let mut all = results.borrow_mut(); (std::mem::take(&mut all[0]), std::mem::take(&mut all[1])) };
        assert_eq!(ra, rb, "{label}: the job's result");
    };
    // refusals first: nothing happens
    for i in 0..2 {
        start_chat(i, "rot-mid", "mid-live");
        start_chat(i, "rot-nocap", "nocap-never-captured-x");
    }
    let mut events = fs::read_to_string(w.root.join("events.jsonl")).unwrap_or_default();
    events += "{\"v\":1,\"ts\":\"2026-03-01T10:00:00Z\",\"event\":\"UserPromptSubmit\",\"session_id\":\"mid-live\"}\n";
    for r in &roots {
        fs::write(r.join("events.jsonl"), &events).unwrap();
    }
    post("a chat that is mid-turn", &act("rot-mid", serde_json::json!({})));
    post("a chat with no captured session", &act("rot-nocap", serde_json::json!({})));
    post("a chat outside the GUI", &act("rot-out", serde_json::json!({})));
    post("a close with no chat open", &act("rot-out", serde_json::json!({"close": true})));
    post("no session to rotate from", &act("rot-none", serde_json::json!({})));
    post("a missing item", &act("nowhere", serde_json::json!({})));
    post("self: not open in the gui", &act("rot-self", serde_json::json!({"self": true})));
    post("self: a session that isn't the chat's", &act("rot-mid", serde_json::json!({"self": true, "session": "nope"})));
    post("self: a chat with no live session", &act("rot-nocap", serde_json::json!({"self": true})));
    settle("the refusals");
    // a live chat hands over: wrap-up typed, its Stop awaited, /exit, the rotation recorded, a fresh chat in its pane
    for i in 0..2 {
        start_chat(i, "rot-live", "rot-live-sid");
    }
    std::thread::sleep(std::time::Duration::from_millis(600));
    let reply = post("rotate a live chat", &act("rot-live", serde_json::json!({})));
    assert!(reply.contains("\"mode\":\"wrap_up\"") && reply.contains("\"close\":false"), "{reply}");
    post("a second rotate while it runs", &act("rot-live", serde_json::json!({})));
    settle("a live rotation");
    let after = fs::read_to_string(root2.join("work-items/rot-live.md")).unwrap();
    assert!(after.contains("rotations:") && after.contains("via: wrap-up") && after.contains("decisions: 2") && after.contains("carried:\n  - 1\n  - 2\n  - 3"), "{after}");
    assert!(fs::read_to_string(fakes[1].join("rec.log")).unwrap().contains("rot-live-sid got: Wrap up for a context rotation"), "{}", fs::read_to_string(fakes[1].join("rec.log")).unwrap());
    // wrap up and close
    for i in 0..2 {
        start_chat(i, "rot-close", "rot-close-sid");
    }
    std::thread::sleep(std::time::Duration::from_millis(600));
    let reply = post("wrap up and close", &act("rot-close", serde_json::json!({"close": true})));
    assert!(reply.contains("\"close\":true"), "{reply}");
    settle("wrap up and close");
    // finish: closed, then resolved
    for i in 0..2 {
        start_chat(i, "rot-finish", "rot-finish-sid");
    }
    std::thread::sleep(std::time::Duration::from_millis(600));
    post("finish", &act("rot-finish", serde_json::json!({"finish": true})));
    settle("finish");
    assert!(fs::read_to_string(root2.join("work-items/rot-finish.md")).unwrap().contains("state: resolved"));
    // a chat asks for its own hand-over: it waits for that chat's Stop after the request
    for i in 0..2 {
        start_chat(i, "rot-self", "rot-self-sid");
    }
    std::thread::sleep(std::time::Duration::from_millis(600));
    let reply = post("self rotation", &act("rot-self", serde_json::json!({"self": true})));
    assert!(reply.contains("\"mode\":\"self\""), "{reply}");
    std::thread::sleep(std::time::Duration::from_millis(1200));
    for r in &roots {
        let mut f = fs::OpenOptions::new().append(true).open(r.join("events.jsonl")).unwrap();
        use std::io::Write;
        let stamp = (chrono::Utc::now() + chrono::Duration::seconds(1)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        writeln!(f, "{{\"v\":1,\"ts\":\"{stamp}\",\"event\":\"UserPromptSubmit\",\"session_id\":\"rot-self-sid\"}}").unwrap();
        writeln!(f, "{{\"v\":1,\"ts\":\"{stamp}\",\"event\":\"Stop\",\"session_id\":\"rot-self-sid\"}}").unwrap();
    }
    settle("self rotation");
    // self rotation names the pane's own chat: one outside the pane is refused, and the right one picked
    for i in 0..2 {
        start_chat(i, "rot-outpane", "rot-outpane-sid");
        start_chat(i, "rot-two", "rot-two-sid");
    }
    std::thread::sleep(std::time::Duration::from_millis(600));
    post("self: the session runs outside the pane", &act("rot-outpane", serde_json::json!({"self": true, "session": "out-live"})));
    post("self: the live session found in the pane", &act("rot-two", serde_json::json!({"self": true})));
    std::thread::sleep(std::time::Duration::from_millis(1200));
    for r in &roots {
        let mut f = fs::OpenOptions::new().append(true).open(r.join("events.jsonl")).unwrap();
        use std::io::Write;
        let stamp = (chrono::Utc::now() + chrono::Duration::seconds(1)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        writeln!(f, "{{\"v\":1,\"ts\":\"{stamp}\",\"event\":\"UserPromptSubmit\",\"session_id\":\"rot-two-sid\"}}").unwrap();
        writeln!(f, "{{\"v\":1,\"ts\":\"{stamp}\",\"event\":\"Stop\",\"session_id\":\"rot-two-sid\"}}").unwrap();
    }
    settle("self rotation in a pane with two live sessions");
    // a dead chat: its transcript is distilled, then a fresh chat starts
    post("a dead chat rotates", &act("rot-dead", serde_json::json!({})));
    settle("a dead chat");
    post("a dead chat that was distilled already", &act("rot-dead", serde_json::json!({})));
    settle("a dead chat, again");
    post("a dead chat with no notes", &act("rot-dead2", serde_json::json!({})));
    settle("a dead chat with no notes");
    post("a dead chat whose session was distilled", &act("rot-dead3", serde_json::json!({})));
    settle("a dead chat, distilled before");
    post("a dead chat next to another session's snapshot", &act("rot-dead4", serde_json::json!({})));
    settle("a dead chat, a snapshot of another session");
    post("a dead chat, wrapping up to close", &act("rot-dead", serde_json::json!({"close": true})));
    post("a dead chat, finishing", &act("rot-dead", serde_json::json!({"finish": true})));
    for s in sockets {
        tmux_on(s, &["kill-server"]);
    }
}
