//! `qm pr-requires`: adding, noting, removing and listing PR prerequisites in a git checkout whose
//! origin is a GitHub repo, with a fake `gh` that answers from files and logs every call. Each step
//! prints what the command said, what the store holds afterwards and the `gh` calls so far;
//! generated from Ruby.

mod common;
use common::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn fake_gh(dir: &Path, data: &Path) {
    fs::create_dir_all(dir).unwrap();
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> \"{data}/calls\"\nD=\"{data}\"\ncase \"$1 $2\" in\n\"pr view\")\n  if [ \"$3\" = \"--json\" ]; then f=current; else f=$(echo \"$3\" | tr '/:' '__'); fi\n  if [ -f \"$D/pr_$f\" ]; then cat \"$D/pr_$f\"; exit 0; fi\n  echo \"no pull requests found for $3\" >&2; exit 1 ;;\n\"api repos/acme/widgets/commits/aaaaaaa\") echo {SHA_A}; exit 0 ;;\n\"api repos/acme/widgets/commits/bbbbbbbb\") echo {SHA_B}; exit 0 ;;\n\"api repos/acme/widgets/commits/ccccccc\") echo not-a-sha; exit 0 ;;\n\"api repos/acme/widgets/commits/ddddddd1\") echo dddddddd1ddddddddddddddddddddddddddddddd; exit 0 ;;\n\"api repos/acme/widgets/commits/ddddddd2\") echo dddddddd2ddddddddddddddddddddddddddddddd; exit 0 ;;\nesac\necho \"gh: unexpected $*\" >&2; exit 1\n",
        data = data.display()
    );
    fs::write(dir.join("gh"), script).unwrap();
    fs::set_permissions(dir.join("gh"), fs::Permissions::from_mode(0o755)).unwrap();
}

fn view(data: &Path, name: &str, url: &str) {
    fs::write(data.join(format!("pr_{name}")), format!("{{\"url\":\"{url}\"}}")).unwrap();
}

#[test]
fn prerequisites_are_recorded_noted_removed_and_listed() {
    let base = tempfile::tempdir().unwrap();
    let (root, bin, data) = (base.path().join("root"), base.path().join("bin"), base.path().join("ghdata"));
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&data).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["remote", "add", "origin", "git@github.com:Acme/Widgets.git"]);
    fake_gh(&bin, &data);
    // GitHub's own spelling of each PR URL
    for (key, url) in [
        ("current", "https://github.com/acme/widgets/pull/7"),
        ("https___github.com_acme_widgets_pull_7", "https://github.com/acme/widgets/pull/7"),
        ("https___github.com_acme_widgets_pull_3", "https://github.com/acme/widgets/pull/3"),
        ("https___github.com_acme_widgets_pull_4", "https://github.com/acme/widgets/pull/4"),
        ("https___github.com_acme_widgets_pull_9", "https://github.com/acme/widgets/pull/9"),
        ("https___github.com_other_repo_pull_1", "https://github.com/other/repo/pull/1"),
        ("https___github.com_Acme_Widgets_pull_3", "https://github.com/acme/widgets/pull/3"),
        ("https___github.com_Acme_Widgets_pull_5", "https://github.com/acme/widgets/pull/5"),
    ] {
        view(&data, key, url);
    }
    fs::write(data.join("pr_https___github.com_acme_widgets_pull_8"), "{\"url\":\"not a url\"}").unwrap();
    fs::write(data.join("pr_https___github.com_acme_widgets_pull_10"), "this is not json").unwrap();
    let qm = Qm::at(&root);
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let env = [("PATH", path.as_str()), ("CLAUDE_CODE_SESSION_ID", "sess-env")];
    let steps: Vec<Vec<&str>> = vec![
        vec!["pr-requires", "--list"],
        vec!["pr-requires", "#3"],
        vec!["pr-requires", "3"],
        vec!["pr-requires", "https://github.com/acme/widgets/pull/4", "--note", "  run the backfill\nthen merge  "],
        vec!["pr-requires", "acme/widgets#9", "--pr", "https://github.com/acme/widgets/pull/7", "--session", "sess-flag"],
        vec!["pr-requires", "aaaaaaa"],
        vec!["pr-requires", "aaaaaaa", "--note", "first line\r\nsecond line\n\nfourth"],
        vec!["pr-requires", "bbbbbbbb", "--pr", "7"],
        vec!["pr-requires", "#3", "--note", ""],
        vec!["pr-requires", "#4", "--note", "replaced note"],
        vec!["pr-requires", "#4", "--note", ""],
        vec!["pr-requires", "#3", "--pr", "5"],
        vec!["pr-requires", "--remove", "#3", "--pr", "5"],
        vec!["pr-requires", "ddddddd1"],
        vec!["pr-requires", "ddddddd2"],
        vec!["pr-requires", "--remove", "dddddddd"],
        vec!["pr-requires", "--remove", "dddddddd1"],
        vec!["pr-requires", "--remove", "dddddddd2"],
        vec!["pr-requires", "#7"],
        vec!["pr-requires", "https://github.com/acme/widgets/pull/7"],
        vec!["pr-requires", "https://github.com/other/repo/pull/1"],
        vec!["pr-requires", "ccccccc"],
        vec!["pr-requires", "zzz"],
        vec!["pr-requires", ""],
        vec!["pr-requires", "https://github.com/acme/widgets/pull/8"],
        vec!["pr-requires", "https://github.com/acme/widgets/pull/10"],
        vec!["pr-requires", "https://github.com/acme/widgets/pull/99"],
        vec!["pr-requires", "--list"],
        vec!["pr-requires", "--list", "--pr", "7"],
        vec!["pr-requires", "--list", "--pr", "5"],
        vec!["pr-requires", "--list", "--pr", "https://github.com/acme/widgets/pull/99"],
        vec!["pr-requires", "--list", "--pr", ""],
        vec!["pr-requires", "--list", "--pr", "nonsense"],
        vec!["pr-requires", "--remove", "aaaaaaa"],
        vec!["pr-requires", "--remove", "bbbb"],
        vec!["pr-requires", "--remove", "#9"],
        vec!["pr-requires", "--remove", "#9"],
        vec!["pr-requires", "--remove", "nope", "--pr", "7"],
        vec!["pr-requires", "--remove", "#3", "--pr", "7"],
        vec!["pr-requires", "--list"],
        vec!["pr-requires", "--remove", "#4"],
        vec!["pr-requires", "--list"],
        vec!["pr-requires"],
        vec!["pr-requires", "a", "b"],
        vec!["pr-requires", "--list", "extra"],
        vec!["pr-requires", "--remove", "--list"],
        vec!["pr-requires", "--remove", "x", "--note", "n"],
        vec!["pr-requires", "--list", "--note", "n"],
    ];
    let mut out = String::new();
    // the deploy cache the list reads (never refreshed by the CLI)
    let cache = serde_json::json!({"v": 1, "releases": {}, "pulls": {
        "https://github.com/acme/widgets/pull/3": {"state": "MERGED", "mergeCommit": SHA_B, "ts": 1},
        "https://github.com/acme/widgets/pull/4": {"state": "OPEN", "ts": 1},
        "https://github.com/acme/widgets/pull/9": {"state": "CLOSED", "ts": 1},
    }, "verdicts": {SHA_A: {"tag": "2026.10.8", "deployed": true, "ts": 1}, SHA_B: {"tag": "2026.10.7", "deployed": false, "ts": 1}}});
    for (n, args) in steps.iter().enumerate() {
        if n == 27 {
            fs::write(root.join("pr-deployment.json"), cache.to_string()).unwrap();
        }
        let o = qm.run_with(args, "", &env);
        let store = fs::read_to_string(root.join("pr-prerequisites.json")).unwrap_or_else(|_| "(no store)".into());
        let store = regex::Regex::new(r#""addedAt": "[^"]+""#).unwrap().replace_all(&store, "\"addedAt\": \"T\"");
        out.push_str(&format!("$ qm {args:?} → {}\n--- stdout\n{}--- stderr\n{}--- store\n{store}\n", o.code, o.stdout, o.stderr));
    }
    out.push_str(&format!("=== gh calls\n{}", fs::read_to_string(data.join("calls")).unwrap_or_default()));
    let out = regex::Regex::new(r"added \d{4}-\d\d-\d\dT[0-9:]+Z").unwrap().replace_all(&out, "added T").into_owned();
    golden("pr_requires.txt", &out.replace(base.path().to_str().unwrap(), "BASE"));
}

#[test]
fn shapes_ruby_reads_in_its_own_way_go_to_ruby() {
    let base = tempfile::tempdir().unwrap();
    let qm = Qm::at(&base.path().join("root"));
    fs::create_dir_all(base.path().join("root")).unwrap();
    for args in [vec!["pr-requires", "--lis"], vec!["pr-requires", "--note=x", "#1"], vec!["pr-requires", "--bogus"], vec!["pr-requires", "--pr"], vec!["pr-requires", "--"]] {
        let o = qm.run(&args);
        assert!(o.code != 0 || !o.stdout.is_empty(), "{args:?}");
    }
    let text: Vec<String> = [vec!["pr-requires", "--lis"], vec!["pr-requires", "--bogus"], vec!["pr-requires", "--pr"]].iter().map(|a| { let o = qm.run(a); format!("{} {} {}", o.code, o.stdout, o.stderr) }).collect();
    golden("pr_requires_ruby_only.txt", &text.join("\n---\n"));
}

#[test]
fn a_hand_edited_store_lists_the_way_ruby_lists_it() {
    let base = tempfile::tempdir().unwrap();
    let (root, bin, data) = (base.path().join("root"), base.path().join("bin"), base.path().join("ghdata"));
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&data).unwrap();
    fake_gh(&bin, &data);
    fs::write(
        root.join("pr-prerequisites.json"),
        serde_json::json!({"v": 1, "extra": [1], "prs": {
            "https://github.com/acme/widgets/pull/2": [
                {"kind": "pr", "ref": "https://github.com/acme/widgets/pull/1", "repo": "acme/widgets", "addedAt": "2026-01-01T00:00:00Z", "note": "carriage\r"},
                {"kind": "commit", "ref": SHA_A, "repo": "acme/widgets", "addedAt": "2026-01-02T00:00:00Z", "note": ""},
                {"kind": "commit", "ref": SHA_B, "repo": "acme/widgets", "addedAt": "2026-01-03T00:00:00Z", "note": "a\r\nb\rc\n"}
            ],
            "https://github.com/acme/widgets/pull/3": []
        }})
        .to_string(),
    )
    .unwrap();
    let qm = Qm::at(&root);
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let env = [("PATH", path.as_str())];
    let o = qm.run_with(&["pr-requires", "--list"], "", &env);
    golden("pr_requires_hand_edited.txt", &format!("{} {}{}", o.code, o.stdout, o.stderr));
}
