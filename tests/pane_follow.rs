//! `attach` and `new --session`: the session moves between items under the work-item lock and the
//! chat's tmux session follows it. Runs a real tmux server on a throwaway socket, killed afterwards.

mod common;
use common::*;
use std::process::{Command, Stdio};

struct TmuxWorld {
    socket: String,
}

impl TmuxWorld {
    fn new(tag: &str) -> TmuxWorld {
        TmuxWorld { socket: format!("qm-test-pane-{tag}-{}", std::process::id()) }
    }

    fn tmux(&self, args: &[&str]) -> String {
        let out = Command::new("tmux").args(["-L", &self.socket]).args(args).env_remove("TMUX").stdin(Stdio::null()).stderr(Stdio::null()).output().unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// A detached session named `name` whose pane runs a long sleep; its pane pid.
    fn chat(&self, name: &str) -> i64 {
        self.tmux(&["new-session", "-d", "-s", name, "-n", name, "sleep", "600"]);
        self.tmux(&["display-message", "-p", "-t", &format!("={name}:"), "#{pane_pid}"]).trim().parse().expect("a pane pid")
    }

    fn sessions(&self) -> String {
        // session/window: a rename must move both, or `attach -t =name` would not find the window
        let mut names: Vec<String> = self.tmux(&["list-windows", "-a", "-F", "#{session_name}/#{window_name}"]).lines().map(String::from).collect();
        names.sort();
        names.join(" ")
    }
}

impl Drop for TmuxWorld {
    fn drop(&mut self) {
        self.tmux(&["kill-server"]);
        let _ = std::fs::remove_file(format!("/tmp/tmux-{}/{}", unsafe { libc_uid() }, self.socket));
    }
}

unsafe fn libc_uid() -> u32 {
    extern "C" {
        fn getuid() -> u32;
    }
    getuid()
}

fn item_file(slug: &str, sessions: &[&str]) -> String {
    let list = if sessions.is_empty() { String::new() } else { format!("sessions:\n{}", sessions.iter().map(|s| format!("- id: {s}\n")).collect::<String>()) };
    format!("---\nv: 1\ntitle: {slug}\nstate: in_progress\nproject: No project\ncreated: '2026-01-01'\n{list}---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n")
}

fn start(id: &str, pid: i64, extra: &str, n: u32) -> String {
    format!("{{\"v\":1,\"event\":\"start\",\"session_id\":\"{id}\",\"cwd\":\"/srv/app\",\"pid\":{pid},{extra}\"ts\":\"2026-03-{n:02}T00:00:00+00:00\"}}\n")
}

#[test]
fn the_chat_pane_follows_its_session_to_the_new_item() {
    let tm = TmuxWorld::new("follow");
    let qm = Qm::new();
    let mut log = String::new();
    let mut items: Vec<(&str, Vec<&str>)> = vec![];

    let p_old = tm.chat("old-item");
    log += &start("s-pane", p_old, "", 1);
    items.push(("old-item", vec!["s-pane"]));
    items.push(("new-item", vec![]));
    let p_other = tm.chat("other-item");
    log += &start("s-two", p_other, "", 2);
    items.push(("other-item", vec!["s-two"]));
    items.push(("third", vec![]));
    let p_src = tm.chat("src");
    tm.chat("dst");
    log += &start("s-src", p_src, "", 3);
    items.push(("src", vec!["s-src"]));
    items.push(("dst", vec![]));
    let p_aside = tm.chat("asd");
    log += &start("s-aside", p_aside, "\"aside\":\"asd\",", 4);
    items.push(("asd", vec!["s-aside"]));
    items.push(("elsewhere", vec![]));
    tm.chat("mm");
    log += &start("s-mm", 999999, "", 5);
    items.push(("mm", vec!["s-mm"]));
    items.push(("mm-target", vec![]));
    let p_x = tm.chat("x-item");
    log += &start("s-new", p_x, "", 6);
    items.push(("x-item", vec!["s-new"]));
    let p_a1 = tm.chat("a1");
    tm.chat("a2");
    log += &start("s-multi", p_a1, "", 7);
    items.push(("a1", vec!["s-multi"]));
    items.push(("a2", vec!["s-multi"]));
    items.push(("a3", vec![]));
    for (slug, sessions) in &items {
        let path = qm.root.path().join(format!("work-items/{slug}.md"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, item_file(slug, sessions)).unwrap();
    }
    std::fs::write(qm.root.path().join("sessions.jsonl"), log).unwrap();

    let env = [("QM_TMUX_SOCKET", tm.socket.as_str())];
    let steps: Vec<Vec<&str>> = vec![
        vec!["attach", "new-item", "--session", "s-pane"],
        vec!["attach", "third", "--session", "s-two"],
        vec!["attach", "dst", "--session", "s-src"],
        vec!["attach", "elsewhere", "--session", "s-aside"],
        vec!["attach", "mm-target", "--session", "s-mm"],
        vec!["attach", "new-item", "--session", "ghost"],
        vec!["attach", "new-item", "--session", "s-pane"],
        vec!["new", "Fresh", "--session", "s-new"],
        vec!["new", "Fresh again", "--session", "never-seen", "--project", "demo"],
        vec!["attach", "a3", "--session", "s-multi"],
        vec!["attach"],
        vec!["attach", "a3"],
        vec!["attach", "ghost", "--session", "s"],
        vec!["detach", "a3", "--session", "s-multi"],
    ];
    let mut out = String::new();
    for args in steps {
        let o = qm.run_with(&args, "", &env);
        out.push_str(&format!("$ qm {}\nexit {}\n--- stdout\n{}--- stderr\n{}tmux: {}\n\n", args.join(" "), o.code, o.stdout, o.stderr, tm.sessions()));
    }
    for slug in ["old-item", "new-item", "src", "dst", "a1", "a2", "a3", "fresh", "fresh-again"] {
        let text = std::fs::read_to_string(qm.root.path().join(format!("work-items/{slug}.md"))).unwrap_or_default();
        out.push_str(&format!("=== {slug}: {}\n", text.lines().skip_while(|l| !l.starts_with("sessions:")).take_while(|l| *l != "---").collect::<Vec<_>>().join(" | ")));
    }
    golden("pane_follow.txt", &normalize(&out));
}

#[test]
fn a_missing_tmux_server_changes_nothing_and_says_nothing() {
    let qm = Qm::new();
    std::fs::create_dir_all(qm.root.path().join("work-items")).unwrap();
    std::fs::write(qm.root.path().join("work-items/a.md"), item_file("a", &["s1"])).unwrap();
    std::fs::write(qm.root.path().join("work-items/b.md"), item_file("b", &[])).unwrap();
    std::fs::write(qm.root.path().join("sessions.jsonl"), start("s1", 12345, "", 1)).unwrap();
    let o = qm.run_with(&["attach", "b", "--session", "s1"], "", &[("QM_TMUX_SOCKET", "qm-test-no-such-server")]);
    assert_eq!((o.code, o.stdout.as_str(), o.stderr.as_str()), (0, "detached from 'a'\n", ""));
}
