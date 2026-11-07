//! The GUI header's ☕ toggle (`Caffeinate`): keeps the Mac from idle-sleeping while chats and agents
//! run. On spawns `caffeinate -i -w <gui pid>` as a child of the GUI, off ends that one child. The `-w`
//! makes caffeinate exit with the GUI, so nothing outlives the server and a restarted GUI starts off.
//! Auto mode holds the child only while some session is busy; the other modes ignore `sync`.
//! `QM_CAFFEINATE_BINARY` names another program (tests do); Ruby has the one path.

use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use serde_json::{json, Value};

struct Inner {
    child: Option<Child>,
    mode: &'static str,
}

static STATE: Mutex<Inner> = Mutex::new(Inner { child: None, mode: "off" });

fn lock() -> std::sync::MutexGuard<'static, Inner> {
    STATE.lock().unwrap_or_else(|p| p.into_inner())
}

fn binary() -> String {
    std::env::var("QM_CAFFEINATE_BINARY").unwrap_or_else(|_| "/usr/bin/caffeinate".into())
}

/// A child that died on its own (killed from outside) reads as off; this also reaps it.
fn running(inner: &mut Inner) -> bool {
    match inner.child.as_mut().map(|c| c.try_wait()) {
        Some(Ok(None)) => true,
        _ => {
            inner.child = None;
            false
        }
    }
}

fn hold(inner: &mut Inner) -> Result<(), String> {
    if !running(inner) {
        let binary = binary();
        let child = Command::new(&binary).args(["-i", "-w", &std::process::id().to_string()]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|_| format!("No such file or directory - {binary}"))?;
        inner.child = Some(child);
    }
    Ok(())
}

fn release(inner: &mut Inner) {
    if running(inner) {
        if let Some(mut child) = inner.child.take() {
            unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
            let _ = child.wait();
        }
    }
    inner.child = None;
}

fn status_of(inner: &mut Inner) -> Value {
    let on = running(inner);
    json!({"on": on, "pid": inner.child.as_ref().map(|c| c.id()), "mode": inner.mode})
}

pub fn status() -> Value {
    status_of(&mut lock())
}

pub fn mode() -> &'static str {
    lock().mode
}

/// Idempotent: a second "on" keeps the running child. A failing spawn leaves the mode "on", as Ruby does.
pub fn on() -> Result<Value, String> {
    let mut inner = lock();
    inner.mode = "on";
    hold(&mut inner)?;
    Ok(status_of(&mut inner))
}

pub fn off() -> Value {
    let mut inner = lock();
    inner.mode = "off";
    release(&mut inner);
    status_of(&mut inner)
}

/// Switches to auto and applies the current verdict straight away.
pub fn auto(busy: bool) -> Result<Value, String> {
    lock().mode = "auto";
    sync(busy)
}

pub fn sync(busy: bool) -> Result<Value, String> {
    let mut inner = lock();
    if inner.mode != "auto" {
        return Ok(status_of(&mut inner));
    }
    if busy {
        hold(&mut inner)?;
    } else {
        release(&mut inner);
    }
    Ok(status_of(&mut inner))
}
