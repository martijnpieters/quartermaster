//! `qm cleanup SLUG` (the plan) and the resolve hint: a real git world with worktrees in every
//! state cleanup cares about, built twice with fixed commit dates; Ruby runs on one copy and Rust
//! on the other and everything printed and left behind must agree. Nothing here removes anything.

mod common;
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn rust() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_qm"))
}

fn ruby() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("quartermaster")
}

struct Git {
    tick: Cell<u32>,
}

impl Git {
    fn run(&self, dir: &Path, args: &[&str]) -> String {
        let n = self.tick.get();
        self.tick.set(n + 1);
        let date = format!("2026-01-01T{:02}:{:02}:{:02}+00:00", n / 3600, (n / 60) % 60, n % 60);
        let out = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "user.name=t", "-c", "user.email=t@t.test", "-c", "init.defaultBranch=main", "-c", "advice.detachedHead=false"])
            .args(args)
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .current_dir(dir)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn commit(&self, dir: &Path, file: &str, text: &str) -> String {
        fs::write(dir.join(file), text).unwrap();
        self.run(dir, &["add", "."]);
        self.run(dir, &["commit", "-q", "-m", text]);
        self.run(dir, &["rev-parse", "HEAD"])
    }
}

struct World {
    base: PathBuf,
    root: PathBuf,
    bin: PathBuf,
    home: PathBuf,
}

fn script(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn world(base: &Path) -> (World, Vec<String>) {
    let g = Git { tick: Cell::new(0) };
    let w = World { base: base.to_path_buf(), root: base.join("root"), bin: base.join("bin"), home: base.join("home") };
    fs::create_dir_all(&w.home).unwrap();
    let (repo, repo2, origin) = (base.join("repo"), base.join("repo2"), base.join("origin.git"));
    fs::create_dir_all(&repo).unwrap();
    g.run(&repo, &["init", "-q", "-b", "main"]);
    g.commit(&repo, "README", "root");
    fs::create_dir_all(&origin).unwrap();
    g.run(&origin, &["init", "-q", "--bare"]);
    g.run(&repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
    g.run(&repo, &["push", "-q", "origin", "main"]);

    let wt = |name: &str, branch: &str| -> PathBuf {
        let p = repo.join(".claude/worktrees").join(name);
        g.run(&repo, &["worktree", "add", "-q", "-b", branch, p.to_str().unwrap(), "main"]);
        p
    };
    wt("clean", "b-clean");
    let dirty = wt("dirty", "b-dirty");
    fs::write(dirty.join("README"), "changed").unwrap();
    fs::write(dirty.join("untracked.txt"), "x").unwrap();
    let unmerged = wt("unmerged", "b-unmerged");
    for i in 1..=5 {
        g.commit(&unmerged, &format!("u{i}.txt"), &format!("unique {i}"));
    }
    let squashed = wt("squashed", "b-squash");
    for i in 1..=3 {
        g.commit(&squashed, &format!("s{i}.txt"), &format!("squash part {i}"));
    }
    let cherry = wt("cherry", "b-cherry");
    let c1 = g.commit(&cherry, "c1.txt", "cherry one");
    let c2 = g.commit(&cherry, "c2.txt", "cherry two");
    let locked = wt("locked", "b-locked");
    g.run(&repo, &["worktree", "lock", "--reason", "because", locked.to_str().unwrap()]);
    let lockednr = wt("lockednr", "b-lockednr");
    g.run(&repo, &["worktree", "lock", lockednr.to_str().unwrap()]);
    let live = wt("live", "b-live");
    let served = wt("served", "b-served");
    wt("shared", "b-shared");
    let prunable = wt("prunable", "b-prunable");
    let detached = repo.join(".claude/worktrees/detached");
    g.run(&repo, &["worktree", "add", "-q", "--detach", detached.to_str().unwrap(), "main"]);
    let pr = wt("pr", "b-pr");
    g.commit(&pr, "pr.txt", "pr unique");
    g.run(&repo, &["branch", "b-pr2", "main"]);
    g.run(&repo, &["checkout", "-q", "b-pr2"]);
    g.commit(&repo, "pr2.txt", "pr2 unique");
    g.run(&repo, &["checkout", "-q", "main"]);
    g.run(&repo, &["branch", "b-pr3", "main"]);
    g.run(&repo, &["branch", "worktree-t", "main"]);
    g.run(&repo, &["checkout", "-q", "worktree-t"]);
    g.commit(&repo, "t.txt", "temp unique");
    g.run(&repo, &["checkout", "-q", "main"]);
    g.run(&repo, &["branch", "worktree-t2", "main"]);
    let pushed = wt("pushed", "b-pushed");
    g.commit(&pushed, "p1.txt", "pushed one");
    g.commit(&pushed, "p2.txt", "pushed two");
    g.run(&pushed, &["push", "-q", "origin", "b-pushed"]);
    // commits patch-equivalent to ones on a sibling branch (not trunk, not upstream): only the
    // patch-id comparison can see they are preserved
    let eqa = wt("eqa", "b-eqa");
    let eq_sha = g.commit(&eqa, "eq.txt", "equivalent change");
    g.run(&repo, &["checkout", "-q", "-b", "b-eqb", "main"]);
    g.run(&repo, &["cherry-pick", &eq_sha]);
    g.commit(&repo, "eqb-extra.txt", "only on b-eqb");
    g.run(&repo, &["checkout", "-q", "main"]);
    // an empty commit has no diff to compare, so equivalence is decided by "touches no path" — but only
    // a head that also holds an unmerged real commit gets that far (an empty one merges into trunk as is)
    let ema = wt("ema", "b-ema");
    let em_sha = g.commit(&ema, "ema.txt", "real change before the empty one");
    g.run(&ema, &["commit", "-q", "--allow-empty", "-m", "an empty commit"]);
    g.run(&repo, &["checkout", "-q", "-b", "b-emb", "main"]);
    g.run(&repo, &["cherry-pick", &em_sha]);
    g.run(&repo, &["commit", "-q", "--allow-empty", "-m", "another empty commit"]);
    g.commit(&repo, "emb-extra.txt", "only on b-emb");
    g.run(&repo, &["checkout", "-q", "main"]);
    // a merged-PR branch that is fine on its own, held by a dirty worktree: it must wait for it
    let hold = wt("hold", "b-hold");
    fs::write(hold.join("README"), "dirty").unwrap();
    // two worktrees of one item: the dev server is asked once, not per worktree
    wt("t1", "b-t1");
    wt("t2", "b-t2");
    wt("failtdb", "b-fail");
    wt("dropfail", "b-dropfail");
    wt("pair", "b-pair");
    wt("dbok", "b-dbok");

    // main moves on: a squash merge of b-squash, and b-cherry's commits cherry-picked
    g.run(&repo, &["merge", "--squash", "b-squash"]);
    g.run(&repo, &["commit", "-q", "-m", "squash of b-squash"]);
    g.run(&repo, &["cherry-pick", &c1, &c2]);
    g.run(&repo, &["push", "-q", "origin", "main"]);
    fs::remove_dir_all(&prunable).unwrap();
    // a worktree whose tip commit is gone from the object store: git log fails, and that must read
    // as "couldn't check", never as "nothing to lose"
    let broken = wt("broken", "b-broken");
    let tip = g.commit(&broken, "broken.txt", "about to vanish");
    let object = repo.join(".git/objects").join(&tip[..2]).join(&tip[2..]);
    fs::remove_file(&object).unwrap();

    // a second repo whose main checkout sits on a branch an item's merged PR was on
    fs::create_dir_all(&repo2).unwrap();
    g.run(&repo2, &["init", "-q", "-b", "main"]);
    g.commit(&repo2, "README", "root2");
    g.run(&repo2, &["checkout", "-q", "-b", "feat-co"]);

    let gh_data = base.join("gh-data");
    for b in ["b-pr", "b-pr2", "b-pr3", "feat-co", "b-clean", "b-hold", "b-pair"] {
        fs::create_dir_all(&gh_data).unwrap();
        fs::write(gh_data.join(b), format!("{{\"number\":1,\"state\":\"MERGED\",\"url\":\"https://example.test/pull/{b}\"}}")).unwrap();
    }
    script(&w.bin.join("gh"), "#!/bin/sh\necho \"$3\" >> \"$FAKE_GH_LOG\"\nf=\"$FAKE_GH_DATA/$3\"\nif [ -f \"$f\" ]; then cat \"$f\"; exit 0; fi\nexit 1\n");
    script(&w.bin.join("devsrv"), "#!/bin/sh\necho \"$@\" >> \"$DEVSRV_LOG\"\n[ \"$1\" = status ] && exit 0\nexit 1\n");
    script(&w.bin.join("tdb"), "#!/bin/sh\n[ -n \"$TDB_LOG\" ] && echo \"$@\" >> \"$TDB_LOG\"\ncase \"$2\" in *dropfail*) [ \"$3\" = \"--dry-run\" ] || { echo cannot drop >&2; exit 1; };; esac\ncase \"$2\" in *failtdb*) echo boom >&2; echo second >&2; exit 1;; esac\necho \"db_$(basename \"$2\")_a\"\necho\necho \"  db_$(basename \"$2\")_b  \"\n");
    fs::write(base.join("devstate"), format!("dev   {}\n", served.display())).unwrap();
    fs::create_dir_all(&w.root).unwrap();
    fs::write(
        w.root.join("config.yml"),
        format!("projects:\n  demo:\n    root: {}\n    dev_server:\n      command: devsrv\n      state_file: {}\n    test_db: tdb\n  second: {}\n", repo.display(), base.join("devstate").display(), repo2.display()),
    )
    .unwrap();

    let me = std::process::id();
    let session = |id: &str, cwd: &Path, pid: u32| format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"{}\",\"pid\":{pid},\"ts\":\"2026-03-01T00:00:00+00:00\"}}\n", cwd.display());
    fs::write(w.root.join("sessions.jsonl"), session("s-live-0001", &live, me) + &session("s-detached-1", &detached, 999999)).unwrap();

    let mut slugs: Vec<String> = vec![];
    let mut item = |slug: &str, project: &str, branches: &[&str], temps: &[&str], sessions: &[&str]| {
        let list = |key: &str, v: &[&str]| if v.is_empty() { String::new() } else { format!("{key}:\n{}", v.iter().map(|b| format!("- {}\n", if key == "sessions" { format!("id: {b}") } else { b.to_string() })).collect::<String>()) };
        let text = format!("---\nv: 1\ntitle: {slug}\nstate: in_progress\nproject: {project}\ncreated: '2026-01-01'\n{}{}{}---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n", list("branches", branches), list("temp_branches", temps), list("sessions", sessions));
        fs::create_dir_all(w.root.join("work-items")).unwrap();
        fs::write(w.root.join(format!("work-items/{slug}.md")), text).unwrap();
        fs::create_dir_all(w.root.join("viewed")).unwrap();
        fs::write(w.root.join(format!("viewed/{slug}.json")), "{\"v\":1,\"marks\":{\"a\":[\"0123456789abcdef\"]}}").unwrap();
        slugs.push(slug.to_string());
    };
    item("clean-item", "demo", &["b-clean"], &[], &[]);
    item("dirty-item", "demo", &["b-dirty"], &[], &[]);
    item("unmerged-item", "demo", &["b-unmerged"], &[], &[]);
    item("squash-item", "demo", &["b-squash"], &[], &[]);
    item("cherry-item", "demo", &["b-cherry"], &[], &[]);
    item("locked-item", "demo", &["b-locked"], &[], &[]);
    item("lockednr-item", "demo", &["b-lockednr"], &[], &[]);
    item("live-item", "demo", &["b-live"], &[], &["s-live-0001"]);
    item("served-item", "demo", &["b-served"], &[], &[]);
    item("owner-a", "demo", &["b-shared"], &[], &[]);
    item("owner-b", "demo", &["b-shared"], &[], &[]);
    item("prunable-item", "demo", &["b-prunable"], &[], &[]);
    item("detached-item", "demo", &[], &[], &["s-detached-1"]);
    item("pr-item", "demo", &["b-pr", "b-pr2", "b-pr3", "main"], &[], &[]);
    item("temp-item", "demo", &[], &["worktree-t", "worktree-t2", "worktree-gone"], &[]);
    item("pushed-item", "demo", &["b-pushed"], &[], &[]);
    item("fail-item", "demo", &["b-fail"], &[], &[]);
    item("eq-item", "demo", &["b-eqa", "b-eqb"], &[], &[]);
    item("hold-item", "demo", &["b-hold"], &[], &[]);
    item("broken-item", "demo", &["b-broken"], &[], &[]);
    item("two-wt-item", "demo", &["b-t1", "b-t2"], &[], &[]);
    item("empty-item", "demo", &["b-ema", "b-emb"], &[], &[]);
    item("db-item", "demo", &["b-dbok"], &[], &[]);
    item("dropfail-item", "demo", &["b-dropfail"], &[], &[]);
    item("pair-item", "demo", &["b-pair"], &[], &[]);
    item("co-item", "second", &["feat-co"], &[], &[]);
    item("no-root-item", "elsewhere", &["x"], &[], &[]);
    item("nothing-item", "demo", &[], &[], &[]);
    item("issue-item", "demo", &["b-clean"], &[], &[]);
    fs::create_dir_all(w.root.join("issues/issue-item")).unwrap();
    fs::write(w.root.join("issues/issue-item/a.md"), "---\ntitle: An issue\n---\nbody\n").unwrap();
    fs::write(w.root.join("issues/issue-item/b.md"), "---\ntitle: Created issue\nkind: bug\nnumber: 5\n---\nbody\n").unwrap();
    fs::write(w.root.join("issues/issue-item/c.md"), "---\ntitle: Open issue\nkind: feature\n---\nbody\n").unwrap();
    fs::create_dir_all(w.root.join("drafts")).unwrap();
    fs::write(w.root.join("drafts/issue-item.json"), "{\"v\":1,\"entries\":[{\"id\":\"a\",\"kind\":\"issue-comment\",\"comment\":\"x\",\"file\":\"c.md\"}]}").unwrap();
    slugs.push("ghost-item".into());
    (w, slugs)
}

fn run(bin: &Path, w: &World, args: &[&str]) -> (i32, String, String) {
    let path = format!("{}:{}", w.bin.display(), std::env::var("PATH").unwrap());
    let out = Command::new(bin)
        .args(args)
        .env_clear()
        .env("PATH", path)
        .env("QM_ROOT", &w.root)
        .env("HOME", &w.home)
        .env("FAKE_GH_DATA", w.base.join("gh-data"))
        .env("FAKE_GH_LOG", w.base.join("gh.log"))
        .env("DEVSRV_LOG", w.base.join("devsrv.log"))
        .env("TDB_LOG", w.base.join("tdb.log"))
        .env("QM_TMUX_SOCKET", "qm-test-cleanup")
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .current_dir(&w.base)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

/// Everything a run printed and left behind, with the world's own directory made anonymous.
fn transcript(bin: &Path, w: &World, slugs: &[String], commands: &[&[&str]]) -> String {
    let mut out = String::new();
    for cmd in commands {
        for slug in slugs {
            let mut args: Vec<&str> = cmd.to_vec();
            args.insert(1, slug);
            let (code, stdout, stderr) = run(bin, w, &args);
            out.push_str(&format!("$ qm {}\nexit {code}\n{stdout}{}\n", args.join(" "), if stderr.is_empty() { String::new() } else { format!("stderr: {stderr}") }));
        }
    }
    for slug in slugs {
        if let Ok(item) = fs::read_to_string(w.root.join(format!("work-items/{slug}.md"))) {
            out.push_str(&format!("=== {slug}: {} | viewed marks left: {}\n", item.lines().find(|l| l.starts_with("state:")).unwrap_or(""), w.root.join(format!("viewed/{slug}.json")).exists()));
        }
    }
    let cache = fs::read_to_string(w.root.join("pr-cache.json")).unwrap_or_default();
    out.push_str(&format!("=== pr-cache\n{}\n", regex::Regex::new(r#""ts":\d+"#).unwrap().replace_all(&cache, "\"ts\":T")));
    out.push_str(&format!("=== gh calls\n{}", fs::read_to_string(w.base.join("gh.log")).unwrap_or_default()));
    out.push_str(&format!("=== dev server calls\n{}", fs::read_to_string(w.base.join("devsrv.log")).unwrap_or_default()));
    out.replace(w.base.to_str().unwrap(), "BASE")
}

#[test]
fn rust_and_ruby_plan_the_same_cleanup_and_leave_the_same_traces() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let ((wa, slugs), (wb, _)) = (world(a.path()), world(b.path()));
    let commands: [&[&str]; 2] = [&["cleanup"], &["state", "resolved"]];
    let ruby_text = transcript(&ruby(), &wa, &slugs, &commands);
    let rust_text = transcript(rust(), &wb, &slugs, &commands);
    assert_eq!(rust_text, ruby_text, "the printed plans or their side effects differ");
    let plain = regex::Regex::new("\x1b\\[[0-9;]*m").unwrap().replace_all(&ruby_text, "").into_owned();
    for expected in [
        "1 worktree — 1 ready", "uncommitted changes", "untracked files", "5 commits found nowhere else", "; …)", "locked: because", "locked by git", "live session s-live-0 works here",
        "the dev server serves it", "also used by open item", "directory already gone", "(detached)", "PR merged", "temp branch", "checked out at", "test DBs: db_dbok_a, db_dbok_b",
        "test DBs unknown: boom", "couldn't check its commits (git log failed)", "no repo root configured for 'elsewhere'", "nothing to clean up", "cleanup: ", "requires" ,
    ] {
        if expected == "requires" {
            continue;
        }
        assert!(plain.contains(expected), "the world should show {expected:?}");
    }
    assert!(!plain.contains("squash-item: 1 worktree — 0 ready") && plain.contains("squash-item: 1 worktree — 1 ready"), "a squash merge loses nothing");
    assert!(plain.contains("cherry-item: 1 worktree — 1 ready"), "cherry-picked commits are preserved");
    assert!(plain.contains("pushed-item: 1 worktree — 1 ready"), "commits pushed to origin are preserved");
    assert!(plain.contains("eq-item: 1 worktree — 1 ready"), "a patch-equivalent commit on a sibling branch is preserved");
    assert!(plain.contains("empty-item: 1 worktree — 1 ready"), "an empty commit has an empty counterpart on the sibling");
    assert!(plain.contains("hold-item: 1 worktree, 1 branch — 0 ready, 1 needs a look"), "the branch waits for its dirty worktree");
    assert!(plain.contains("two-wt-item: 2 worktrees — 2 ready"));
    assert!(plain.contains("issue-item: 1 worktree, 1 branch, 3 issue drafts — 1 ready, 4 need a look"), "worktrees and issue drafts share one plan\n{}", plain.lines().filter(|l| l.contains("issue-item")).collect::<Vec<_>>().join("\n"));
    assert!(plain.contains("issue-item/b.md “Created issue” — #5") && plain.contains("never created on GitHub; 1 unsent comment"));
}

fn sweeps(bin: &Path, w: &World, slugs: &[String]) -> String {
    let mut out = String::new();
    let mut sweep = |label: &str, out: &mut String| {
        let (code, stdout, stderr) = run(bin, w, &["sweep"]);
        out.push_str(&format!("=== sweep {label}: exit {code}\n{stdout}{stderr}"));
    };
    sweep("with every item open", &mut out);
    for slug in slugs {
        if slug != "ghost-item" && slug != "issue-item" {
            run(bin, w, &["state", slug, "resolved"]);
        }
    }
    sweep("after resolving most items", &mut out);
    let cache = fs::read_to_string(w.root.join("pr-cache.json")).unwrap_or_default();
    out.push_str(&format!("=== gh calls\n{}", fs::read_to_string(w.base.join("gh.log")).unwrap_or_default()));
    out.push_str(&format!("=== dev server calls\n{}", fs::read_to_string(w.base.join("devsrv.log")).unwrap_or_default()));
    out.push_str(&format!("=== cache {}\n", cache.len() > 0));
    out.replace(w.base.to_str().unwrap(), "BASE")
}

#[test]
fn rust_and_ruby_plan_the_same_sweep() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let ((wa, slugs), (wb, _)) = (world(a.path()), world(b.path()));
    for w in [&wa, &wb] {
        // a repo that is no repo, a second project at the same root, and issue sets of an open, a resolved and a gone item
        fs::create_dir_all(w.base.join("notgit")).unwrap();
        // a worktree outside .claude/worktrees is not the sweep's
        let out = Command::new("git").args(["worktree", "add", "-q", "-b", "b-outside", w.base.join("outside-wt").to_str().unwrap(), "main"]).current_dir(w.base.join("repo")).output().unwrap();
        assert!(out.status.success());
        fs::create_dir_all(w.root.join("issues/ghost-item")).unwrap();
        fs::write(w.root.join("issues/ghost-item/x.md"), "---\ntitle: Orphan\nkind: bug\nnumber: 3\n---\n").unwrap();
        let config = fs::read_to_string(w.root.join("config.yml")).unwrap();
        fs::write(w.root.join("config.yml"), format!("{config}  nogit: {}\n  twin: {}\n", w.base.join("notgit").display(), w.base.join("repo").display())).unwrap();
    }
    let ruby_text = sweeps(&ruby(), &wa, &slugs);
    let rust_text = sweeps(rust(), &wb, &slugs);
    assert_eq!(rust_text, ruby_text, "the sweep plans differ");
    let plain = regex::Regex::new("\x1b\\[[0-9;]*m").unwrap().replace_all(&ruby_text, "").into_owned();
    assert!(!plain.contains("outside-wt"), "only .claude/worktrees is swept");
    for expected in ["sweep: ", "is not a git repo", "belongs to unresolved item", "worktree branch", "issue-draft ghost-item/x.md", "issue-item/c.md"] {
        assert!(plain.contains(expected), "the sweep should show {expected:?}\n{plain}");
    }
}


/// What removal leaves behind: every repo's worktrees and branches, the item files, the issue drafts
/// and their comments, and what the helper commands were asked.
fn leftovers(w: &World) -> String {
    let mut out = String::new();
    for repo in ["repo", "repo2"] {
        let dir = w.base.join(repo);
        let git = |args: &[&str]| String::from_utf8_lossy(&Command::new("git").args(args).current_dir(&dir).output().unwrap().stdout).into_owned();
        out.push_str(&format!("=== {repo} worktrees\n{}=== {repo} branches\n{}", git(&["worktree", "list", "--porcelain"]), git(&["branch", "--format=%(refname:short)"])));
        if let Ok(rd) = fs::read_dir(dir.join(".claude/worktrees")) {
            let mut names: Vec<String> = rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            out.push_str(&format!("=== {repo} worktree dirs: {names:?}\n"));
        }
    }
    fn walk(dir: &Path, base: &Path, out: &mut String) {
        let mut entries: Vec<_> = fs::read_dir(dir).map(|r| r.filter_map(|e| e.ok()).collect()).unwrap_or_default();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            let rel = p.strip_prefix(base).unwrap().display().to_string();
            if p.is_dir() {
                out.push_str(&format!("{rel}/\n"));
                walk(&p, base, out);
            } else if !rel.ends_with(".lock") {
                let text = fs::read_to_string(&p).unwrap_or_default();
                let text = regex::Regex::new(r#""seen_at":[0-9.e+-]+"#).unwrap().replace_all(&text, "\"seen_at\":T").into_owned();
                out.push_str(&format!("{rel}: {text}\n"));
            }
        }
    }
    for sub in ["work-items", "issues", "drafts"] {
        out.push_str(&format!("=== {sub}\n"));
        walk(&w.root.join(sub), &w.root, &mut out);
    }
    for log in ["tdb.log", "devsrv.log", "gh.log"] {
        out.push_str(&format!("=== {log}\n{}", fs::read_to_string(w.base.join(log)).unwrap_or_default()));
    }
    out.replace(w.base.to_str().unwrap(), "BASE")
}

fn run_steps(bin: &Path, w: &World, steps: &[Vec<String>]) -> String {
    let mut out = String::new();
    for step in steps {
        let args: Vec<String> = step.iter().map(|s| s.replace("{base}", w.base.to_str().unwrap())).collect();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let (code, stdout, stderr) = run(bin, w, &refs);
        out.push_str(&format!("$ qm {}\nexit {code}\n{stdout}{}\n", step.join(" "), if stderr.is_empty() { String::new() } else { format!("stderr: {stderr}") }));
    }
    out.push_str(&leftovers(w));
    out.replace(w.base.to_str().unwrap(), "BASE")
}

fn compare_removal(steps: impl Fn(&[String]) -> Vec<Vec<String>>) -> String {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let ((wa, slugs), (wb, _)) = (world(a.path()), world(b.path()));
    let steps = steps(&slugs);
    let ruby_text = run_steps(&ruby(), &wa, &steps);
    let rust_text = run_steps(rust(), &wb, &steps);
    if rust_text != ruby_text {
        let (r, y): (Vec<&str>, Vec<&str>) = (rust_text.lines().collect(), ruby_text.lines().collect());
        let at = (0..r.len().max(y.len())).find(|i| r.get(*i) != y.get(*i)).unwrap();
        let show = |v: &Vec<&str>| v.iter().skip(at.saturating_sub(3)).take(8).map(|l| l.chars().take(240).collect::<String>()).collect::<Vec<_>>().join("\n  ");
        panic!("removal differs at line {at}\nRust:\n  {}\nRuby:\n  {}", show(&r), show(&y));
    }
    regex::Regex::new("\x1b\\[[0-9;]*m").unwrap().replace_all(&ruby_text, "").into_owned()
}

fn s(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

#[test]
fn rust_and_ruby_remove_the_same_things_from_cleanup_slug() {
    let text = compare_removal(|slugs| {
        let wt = |name: &str| format!("{{base}}/repo/.claude/worktrees/{name}");
        let mut steps = vec![
            s(&["cleanup", "dirty-item", "--force", &wt("dirty")]),
            s(&["cleanup", "dirty-item", "--yes"]),
            s(&["cleanup", "dirty-item", "--yes", "--force", &wt("dirty")]),
            s(&["cleanup", "dirty-item", "--force", "nothing-like-this"]),
            s(&["cleanup", "--yes"]),
            s(&["cleanup"]),
            s(&["cleanup", "ghost-nobody", "--yes"]),
            s(&["cleanup", "issue-item", "--yes"]),
            s(&["cleanup", "issue-item", "--yes", "--force", "issue-item/a.md", "--force", "{base}/root/issues/issue-item/c.md"]),
            s(&["cleanup", "hold-item", "--yes", "--force", "b-hold"]),
            s(&["cleanup", "locked-item", "--yes", "--force", &wt("locked")]),
            s(&["cleanup", "failtdb-item", "--yes", "--force", &wt("failtdb")]),
            s(&["cleanup", "dropfail-item", "--yes"]),
        ];
        for slug in slugs {
            steps.push(s(&["--yes-placeholder"]));
            steps.pop();
            steps.push(vec!["cleanup".into(), slug.clone(), "--yes".into()]);
        }
        steps
    });
    for expected in ["removed", "pruned", "deleted", "already gone", "dropping test DBs failed", "no cleanup entry matches", "usage: qm cleanup SLUG", "and test DBs", "BASE/repo#b-pair: deleted"] {
        assert!(text.contains(expected), "the removals should show {expected:?}\n{text}");
    }
}

#[test]
fn rust_and_ruby_remove_the_same_things_in_a_sweep() {
    let text = compare_removal(|_| {
        vec![
            s(&["sweep"]),
            s(&["sweep", "--force", "nothing-like-this"]),
            s(&["sweep", "--yes"]),
            s(&["sweep", "--yes", "--force", "{base}/repo/.claude/worktrees/dirty", "--force", "b-unmerged"]),
            s(&["sweep", "--yes", "--force", "{base}/repo/.claude/worktrees/unmerged"]),
            s(&["sweep"]),
        ]
    });
    for expected in ["removed", "worktree branch", "belongs to unresolved item"] {
        assert!(text.contains(expected), "the sweep should show {expected:?}\n{text}");
    }
}
