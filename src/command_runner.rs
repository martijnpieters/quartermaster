//! The one door to external tools: a command run with its output captured. Only read-only probes
//! (a dev server's status, a test-DB dry run) go through the Rust side; anything that changes the
//! machine stays in Ruby.

use std::process::{Command, Stdio};

use crate::data_root;

pub struct CmdResult {
    pub out: String,
    pub err: String,
    pub success: bool,
}

/// The user's personal tools live in ~/bin, which a GUI started from a launcher (or a hook) may
/// not have on PATH.
pub(crate) fn resolve_program(name: &str) -> String {
    resolve(name)
}

fn resolve(name: &str) -> String {
    if name.contains('/') {
        return name.to_string();
    }
    let personal = data_root::expand_path("~/bin").join(name);
    let executable = std::fs::metadata(&personal).is_ok_and(|m| {
        use std::os::unix::fs::PermissionsExt;
        m.is_file() && m.permissions().mode() & 0o111 != 0
    });
    if executable { personal.to_string_lossy().into_owned() } else { name.to_string() }
}

/// A GUI on a throwaway data root is fenced off from the dev server (`PfFence`): the configured server is
/// the user's one real server whatever `QM_ROOT` says, so every command of it that changes what it serves
/// is refused here. Reads (`<command> status`) pass.
static FENCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_fenced(on: bool) {
    FENCED.store(on, std::sync::atomic::Ordering::SeqCst);
}

/// Why the dev server is off in this GUI (`PfFence.reason`).
pub fn fence_reason() -> String {
    format!("the dev server is off in this GUI: {} is not the live qm data root", crate::cli_format::short_path(Some(&data_root::root().to_string_lossy())))
}

/// Whether this GUI's data root is the user's own (`Quartermaster.live_root?`).
pub fn live_root() -> bool {
    data_root::root() == data_root::expand_path("~/.claude/quartermaster")
}

fn changes_server(argv: &[&str]) -> bool {
    let base = |p: &str| std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let config = crate::project_config::ProjectConfig::current();
    config.dev_server_commands().iter().any(|c| base(c) == base(argv.first().copied().unwrap_or(""))) && argv.get(1) != Some(&"status")
}

pub fn call(argv: &[&str]) -> CmdResult {
    if FENCED.load(std::sync::atomic::Ordering::SeqCst) && changes_server(argv) {
        return CmdResult { out: String::new(), err: fence_reason(), success: false };
    }
    run(argv, None)
}

/// As `call`, in `chdir` (Ruby's `CommandRunner#call(argv, chdir:)`).
pub fn call_in(argv: &[&str], chdir: &str) -> CmdResult {
    if FENCED.load(std::sync::atomic::Ordering::SeqCst) && changes_server(argv) {
        return CmdResult { out: String::new(), err: fence_reason(), success: false };
    }
    run(argv, Some(chdir))
}

/// What Ruby's `SystemCallError#message` says for a command that would not start.
fn spawn_error(e: &std::io::Error, name: &str) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => format!("No such file or directory - {name}"),
        std::io::ErrorKind::PermissionDenied => format!("Permission denied - {name}"),
        _ => e.to_string(),
    }
}

fn run(argv: &[&str], chdir: Option<&str>) -> CmdResult {
    let Some((first, rest)) = argv.split_first() else { return CmdResult { out: String::new(), err: "empty command".into(), success: false } };
    let program = resolve(first);
    let mut cmd = Command::new(&program);
    cmd.args(rest).stdin(Stdio::null());
    if let Some(dir) = chdir {
        cmd.current_dir(dir);
    }
    match cmd.output() {
        Ok(o) => CmdResult { out: String::from_utf8_lossy(&o.stdout).into_owned(), err: String::from_utf8_lossy(&o.stderr).into_owned(), success: o.status.success() },
        Err(e) => CmdResult { out: String::new(), err: spawn_error(&e, &program), success: false },
    }
}
