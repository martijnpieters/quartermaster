//! The Ruby launcher's opt-in hand-over to the Rust binary (`QM_USE_RUST=1`): when it execs, what it
//! passes on, and the cases where it must stay Ruby — unset or other values, a missing or
//! non-executable binary, a call coming back from Rust (`QM_FROM_RUST`).

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn launcher() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("quartermaster")
}

/// A stand-in for the Rust binary: records its arguments, stdin and the switch variables.
fn fake_rust(dir: &Path, code: i32) -> PathBuf {
    let path = dir.join("fake-qm");
    fs::write(&path, format!("#!/bin/sh\n{{ echo \"args: $*\"; echo \"from_rust: ${{QM_FROM_RUST-unset}}\"; echo \"stdin: $(cat)\"; }} > \"{}/seen\"\necho rust-ran\nexit {code}\n", dir.display())).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn run(dir: &Path, env: &[(&str, &str)], args: &[&str], stdin: &str) -> Output {
    let mut cmd = Command::new(launcher());
    cmd.args(args).env("QM_ROOT", dir.join("root")).env_remove("QM_USE_RUST").env_remove("QM_FROM_RUST").env_remove("QM_RUST_BIN");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn the_switch_hands_the_command_to_the_rust_binary() {
    let dir = tempfile::tempdir().unwrap();
    let fake = fake_rust(dir.path(), 7);
    let o = run(dir.path(), &[("QM_USE_RUST", "1"), ("QM_RUST_BIN", fake.to_str().unwrap())], &["note", "slug", "two words"], "payload on stdin");
    assert_eq!(out(&o), "rust-ran\n");
    assert_eq!(o.status.code(), Some(7), "the exit code is the Rust binary's");
    assert_eq!(fs::read_to_string(dir.path().join("seen")).unwrap(), "args: note slug two words\nfrom_rust: unset\nstdin: payload on stdin\n");
}

#[test]
fn it_stays_ruby_unless_asked_and_able() {
    let dir = tempfile::tempdir().unwrap();
    let fake = fake_rust(dir.path(), 0);
    let fake = fake.to_str().unwrap();
    let plain = out(&run(dir.path(), &[], &["list"], ""));
    assert!(!plain.contains("rust-ran"), "{plain}");
    let not_executable = dir.path().join("plain-file");
    fs::write(&not_executable, "#!/bin/sh\necho rust-ran\n").unwrap();
    for (label, env) in [
        ("switch unset", vec![("QM_RUST_BIN", fake)]),
        ("switch 0", vec![("QM_USE_RUST", "0"), ("QM_RUST_BIN", fake)]),
        ("switch true", vec![("QM_USE_RUST", "true"), ("QM_RUST_BIN", fake)]),
        ("switch empty", vec![("QM_USE_RUST", ""), ("QM_RUST_BIN", fake)]),
        ("coming back from Rust", vec![("QM_USE_RUST", "1"), ("QM_FROM_RUST", "1"), ("QM_RUST_BIN", fake)]),
        ("binary missing", vec![("QM_USE_RUST", "1"), ("QM_RUST_BIN", "/nonexistent/qm")]),
        ("binary not executable", vec![("QM_USE_RUST", "1"), ("QM_RUST_BIN", not_executable.to_str().unwrap())]),
        ("no built binary", vec![("QM_USE_RUST", "1")]),
    ] {
        let o = run(dir.path(), &env, &["list"], "");
        assert_eq!(out(&o), plain, "{label}");
        assert!(!dir.path().join("seen").exists(), "{label}: the Rust binary ran");
    }
}

#[test]
fn a_command_the_rust_binary_has_not_ported_comes_back_once_and_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let rust = Path::new(env!("CARGO_BIN_EXE_qm"));
    // `status` is nobody's command: the Ruby launcher answers it with an error and the usage text
    let ruby = run(dir.path(), &[], &["status"], "");
    assert!(!ruby.status.success() && String::from_utf8_lossy(&ruby.stderr).contains("unknown command 'status'"));
    let both = run(dir.path(), &[("QM_USE_RUST", "1"), ("QM_RUST_BIN", rust.to_str().unwrap())], &["status"], "");
    assert_eq!((out(&both), String::from_utf8_lossy(&both.stderr).into_owned(), both.status.code()), (out(&ruby), String::from_utf8_lossy(&ruby.stderr).into_owned(), ruby.status.code()));
    // a ported one is answered by Rust itself
    let listed = run(dir.path(), &[("QM_USE_RUST", "1"), ("QM_RUST_BIN", rust.to_str().unwrap())], &["list"], "");
    assert!(listed.status.success());
}

#[test]
fn without_a_named_binary_it_takes_the_release_build_beside_it() {
    let dir = tempfile::tempdir().unwrap();
    let checkout = dir.path().join("checkout");
    fs::create_dir_all(checkout.join("target/release")).unwrap();
    fs::copy(launcher(), checkout.join("quartermaster")).unwrap();
    std::os::unix::fs::symlink(Path::new(env!("CARGO_MANIFEST_DIR")).join("lib"), checkout.join("lib")).unwrap();
    let fake = fake_rust(dir.path(), 0);
    fs::copy(&fake, checkout.join("target/release/qm")).unwrap();
    let mut cmd = Command::new(checkout.join("quartermaster"));
    let o = cmd.args(["list"]).env("QM_ROOT", dir.path().join("root")).env("QM_USE_RUST", "1").env_remove("QM_RUST_BIN").env_remove("QM_FROM_RUST").stdin(Stdio::null()).output().unwrap();
    assert_eq!(out(&o), "rust-ran\n");
    // a debug build is not taken: the release build is what a user installs
    fs::remove_file(checkout.join("target/release/qm")).unwrap();
    fs::create_dir_all(checkout.join("target/debug")).unwrap();
    fs::copy(&fake, checkout.join("target/debug/qm")).unwrap();
    let o = Command::new(checkout.join("quartermaster")).args(["list"]).env("QM_ROOT", dir.path().join("root")).env("QM_USE_RUST", "1").env_remove("QM_RUST_BIN").env_remove("QM_FROM_RUST").stdin(Stdio::null()).output().unwrap();
    assert!(!out(&o).contains("rust-ran"));
}

#[test]
fn the_marker_does_not_travel_on_to_what_ruby_starts() {
    let dir = tempfile::tempdir().unwrap();
    let (root, bin) = (dir.path().join("root"), dir.path().join("bin"));
    fs::create_dir_all(root.join("work-items")).unwrap();
    fs::create_dir_all(&bin).unwrap();
    fs::write(root.join("work-items/one.md"), "---\nv: 1\ntitle: One\nstate: in_progress\nproject: demo\ncreated: '2026-01-01'\n---\n\n## Description\n\nd\n\n## Notes\n\n## Snapshots\n").unwrap();
    // `ask` starts `claude`; this one says what it inherited
    let claude = bin.join("claude");
    fs::write(&claude, format!("#!/bin/sh\necho \"from_rust: ${{QM_FROM_RUST-unset}}\" > \"{}/inherited\"\ncat > /dev/null\necho '{{\"type\":\"result\",\"structured_output\":{{\"hits\":[]}}}}'\n", dir.path().display())).unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let o = run(dir.path(), &[("QM_USE_RUST", "1"), ("QM_FROM_RUST", "1"), ("PATH", &path)], &["ask", "anything"], "");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(fs::read_to_string(dir.path().join("inherited")).unwrap(), "from_rust: unset\n");
}
