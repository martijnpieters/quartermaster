use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn run(cwd: Option<&str>, args: &[&str], stdin: &[u8]) -> Option<Vec<u8>> {
    let cwd = cwd?;
    if !Path::new(cwd).is_dir() {
        return None;
    }
    let mut child = Command::new("git").arg("-C").arg(cwd).args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut input = child.stdin.take()?;
    let data = stdin.to_vec();
    // feed stdin from a thread so a large input and a large output cannot deadlock each other
    let writer = std::thread::spawn(move || {
        let _ = input.write_all(&data);
    });
    let out = child.wait_with_output().ok()?;
    let _ = writer.join();
    out.status.success().then_some(out.stdout)
}

fn strip_bytes(mut v: Vec<u8>) -> Vec<u8> {
    let ws = |b: &u8| matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r');
    while v.last().is_some_and(|b| ws(b) || *b == 0) {
        v.pop();
    }
    let lead = v.iter().take_while(|b| ws(b)).count();
    v.drain(..lead);
    v
}

/// Output of `git -C cwd args` with `stdin` fed to it, stripped; None when cwd is no directory or git fails.
pub fn git_with(cwd: Option<&str>, args: &[&str], stdin: &str) -> Option<String> {
    run(cwd, args, stdin.as_bytes()).map(|o| String::from_utf8_lossy(&strip_bytes(o)).into_owned())
}

/// As `git_with` but byte for byte (a diff may hold text that is not UTF-8).
pub fn git_bytes(cwd: Option<&str>, args: &[&str], stdin: &[u8]) -> Option<Vec<u8>> {
    run(cwd, args, stdin).map(strip_bytes)
}

/// Output of `git -C cwd args`, stripped; None when cwd is no directory or git fails.
pub fn git(cwd: Option<&str>, args: &[&str]) -> Option<String> {
    git_with(cwd, args, "")
}

/// `BranchDiff#raw_git`: unstripped stdout with invalid bytes scrubbed to `?`, for the exit statuses
/// in `ok` (`diff --no-index` exits 1 when the files differ); None otherwise.
pub fn raw(cwd: &str, args: &[&str], ok: &[i32]) -> Option<String> {
    let out = Command::new("git").args(["-c", "core.quotepath=off", "-C", cwd]).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if !ok.contains(&out.status.code()?) {
        return None;
    }
    let mut text = String::new();
    for chunk in out.stdout.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push('?');
        }
    }
    Some(text)
}
