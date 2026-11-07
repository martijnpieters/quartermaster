//! The client side of the GUI's `POST /api/action`: how the commands that act on the running GUI's
//! chats (`rotate`, `caffeinate`, `pane-start`, `pane-resume`) reach it. The GUI owns the work — its
//! tmux session, its terminal, the rotation job — so those commands only ask, and with no GUI they
//! say so and do nothing. The request is the one Ruby's `Net::HTTP` sends: loopback, no `Origin`.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use serde_json::{Map, Value};

pub enum Failed {
    /// `abort`'s message, printed as is.
    Abort(String),
    /// A port this port doesn't read like Ruby's `String#to_i`.
    Ruby,
}

/// `Gui.port`: `QM_GUI_PORT` (default 49737) through `to_i`. Only plain 1–65535 is read here.
pub fn port() -> Result<u16, Failed> {
    match std::env::var("QM_GUI_PORT") {
        Err(_) => Ok(49737),
        Ok(v) => v.parse::<u16>().ok().filter(|p| *p > 0 && v.bytes().all(|b| b.is_ascii_digit())).ok_or(Failed::Ruby),
    }
}

fn decode_chunked(mut body: &[u8]) -> Option<Vec<u8>> {
    let mut out = vec![];
    loop {
        let eol = body.windows(2).position(|w| w == b"\r\n")?;
        let size = usize::from_str_radix(std::str::from_utf8(&body[..eol]).ok()?.split(';').next()?.trim(), 16).ok()?;
        body = &body[eol + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(body.get(..size)?);
        body = body.get(size + 2..)?;
    }
}

/// Posts one action and returns the reply; a refusal (`ok` false) aborts with its error.
pub fn call(name: &str, cmd: &str, slug: Option<&str>, args: Value) -> Result<Map<String, Value>, Failed> {
    let port = port()?;
    let abort = |m: String| Failed::Abort(m);
    let mut payload = Map::new();
    payload.insert("cmd".into(), cmd.into());
    if let Some(s) = slug {
        payload.insert("slug".into(), s.into());
    }
    payload.insert("args".into(), args);
    let body = serde_json::to_string(&Value::Object(payload)).unwrap();

    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let no_gui = || abort(format!("qm {name}: no qm gui running on 127.0.0.1:{port} — nothing done (it starts with `qm gui`)"));
    let slow = || abort(format!("qm {name}: the qm gui on 127.0.0.1:{port} did not answer in time — try again"));
    let odd = || abort(format!("qm {name}: 127.0.0.1:{port} did not answer like a qm gui"));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|_| no_gui())?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let request = format!("POST /api/action HTTP/1.1\r\nContent-Type: application/json\r\nConnection: close\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    stream.write_all(request.as_bytes()).map_err(|_| no_gui())?;
    let mut raw = vec![];
    if let Err(e) = stream.read_to_end(&mut raw) {
        return Err(if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) { slow() } else { odd() });
    }
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(slow)?;
    let head = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
    let mut payload = raw[split + 4..].to_vec();
    if head.contains("transfer-encoding: chunked") {
        payload = decode_chunked(&payload).ok_or_else(odd)?;
    } else if let Some(n) = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()) {
        payload.truncate(n);
    }
    let Ok(Value::Object(reply)) = serde_json::from_slice::<Value>(&payload) else { return Err(odd()) };
    if matches!(reply.get("ok"), None | Some(Value::Null) | Some(Value::Bool(false))) {
        let error = match reply.get("error") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
        };
        return Err(abort(format!("qm {name}: {error}")));
    }
    Ok(reply)
}

pub fn truthy(v: Option<&Value>) -> bool {
    !matches!(v, None | Some(Value::Null) | Some(Value::Bool(false)))
}

