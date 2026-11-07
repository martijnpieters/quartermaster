use std::ffi::OsStr;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

/// The launcher of the checkout this binary was built from, whatever `QM_RUBY_LAUNCHER` says.
pub fn checkout_launcher() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("quartermaster")
}

/// The Ruby launcher, which still serves every command this binary has not ported.
pub fn launcher() -> PathBuf {
    match std::env::var_os("QM_RUBY_LAUNCHER") {
        Some(p) => PathBuf::from(p),
        None => checkout_launcher(),
    }
}

/// Replaces this process with the Ruby qm, arguments and stdin untouched.
pub fn exec_ruby<S: AsRef<OsStr>>(args: &[S]) -> ! {
    // QM_FROM_RUST tells the launcher this call comes from the Rust binary, so QM_USE_RUST does not bounce it back
    let err = Command::new(launcher()).args(args).env("QM_FROM_RUST", "1").exec();
    eprintln!("qm: cannot run the Ruby launcher {}: {err}", launcher().display());
    std::process::exit(127);
}
