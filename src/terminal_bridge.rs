//! A chat pane: the browser's `/ws/terminal` WebSocket bridged to a PTY running a fresh `tmux attach` for
//! this connection (`TerminalBridge`, `WebSocket`). Binary frames are keystrokes, text frames are
//! `{cols, rows}` resizes. Closing the socket only kills that client — the chat lives on in tmux and the next
//! connection redraws it. The chat ending ends the session, which exits the client and closes the socket.

use std::io::{BufReader, Read, Write};
use std::net::TcpStream;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value};

use crate::data_root;

const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const MAX_PAYLOAD: usize = 1 << 20;
const TEXT: u8 = 1;
const BINARY: u8 = 2;
const CLOSE: u8 = 8;
const PING: u8 = 9;
const PONG: u8 = 10;

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn accept_key(key: &str) -> String {
    let hex = crate::sha1::hex(format!("{key}{GUID}").as_bytes());
    let digest: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0)).collect();
    base64(&digest)
}

/// One frame as the server sends it: FIN set, never masked.
fn encode(payload: &[u8], opcode: u8) -> Vec<u8> {
    let mut frame = vec![0x80 | opcode];
    let len = payload.len();
    if len < 126 {
        frame.push(len as u8);
    } else if len < 65_536 {
        frame.push(126);
        frame.extend((len as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend((len as u64).to_be_bytes());
    }
    frame.extend(payload);
    frame
}

#[derive(Clone)]
struct Socket {
    out: Arc<Mutex<TcpStream>>,
}

impl Socket {
    fn send(&self, payload: &[u8], opcode: u8) -> std::io::Result<()> {
        let frame = encode(payload, opcode);
        self.out.lock().unwrap_or_else(|p| p.into_inner()).write_all(&frame)
    }
    fn close(&self) {
        let _ = self.send(&1000u16.to_be_bytes(), CLOSE);
    }
}

enum Stop {
    Protocol(String),
    Io,
}

fn read_exact(reader: &mut impl Read, count: usize) -> Result<Vec<u8>, Stop> {
    let mut buf = vec![0u8; count];
    reader.read_exact(&mut buf).map_err(|_| Stop::Io)?;
    Ok(buf)
}

/// One frame: `None` at end of stream. Clients must mask (RFC 6455 5.1).
fn read_frame(reader: &mut impl Read) -> Result<Option<(bool, u8, Vec<u8>)>, Stop> {
    let mut head = [0u8; 2];
    if reader.read_exact(&mut head).is_err() {
        return Ok(None);
    }
    let (b0, b1) = (head[0], head[1]);
    let mut len = (b1 & 0x7f) as usize;
    if len == 126 {
        len = u16::from_be_bytes(read_exact(reader, 2)?.try_into().unwrap()) as usize;
    } else if len == 127 {
        len = u64::from_be_bytes(read_exact(reader, 8)?.try_into().unwrap()) as usize;
    }
    if len > MAX_PAYLOAD {
        return Err(Stop::Protocol(format!("frame too large ({len} bytes)")));
    }
    if b1 & 0x80 == 0 {
        return Err(Stop::Protocol("unmasked client frame".into()));
    }
    let mask = read_exact(reader, 4)?;
    let mut data = if len == 0 { vec![] } else { read_exact(reader, len)? };
    for (i, byte) in data.iter_mut().enumerate() {
        *byte ^= mask[i % 4];
    }
    Ok(Some((b0 & 0x80 != 0, b0 & 0x0f, data)))
}

/// `String#to_i` of a JSON value: numbers truncate, strings read their leading integer.
fn to_i(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)).unwrap_or(0),
        Some(Value::String(s)) => {
            let t = s.trim_start();
            let (sign, digits) = match t.strip_prefix('-') {
                Some(rest) => (-1, rest),
                None => (1, t.strip_prefix('+').unwrap_or(t)),
            };
            sign * digits.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse::<i64>().unwrap_or(0)
        }
        _ => 0,
    }
}

fn set_winsize(fd: i32, rows: u16, cols: u16) {
    let size = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
    unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &size) };
}

/// Why a terminal WebSocket upgrade is refused, or `None` to accept it.
pub fn refusal(path: &str, headers: &Map<String, Value>, allowed_host: bool, foreign_origin: bool, token: &str, params: &Map<String, Value>) -> Option<&'static str> {
    let route = path.split('?').next().unwrap_or("");
    if route != "/ws/terminal" {
        return Some("not found");
    }
    if !allowed_host {
        return Some("bad host");
    }
    if headers.get("origin").is_none() || foreign_origin {
        return Some("bad origin");
    }
    let given = params.get("token").and_then(Value::as_str);
    if !given.is_some_and(|g| g.len() == token.len() && g.bytes().zip(token.bytes()).fold(0, |acc, (a, b)| acc | (a ^ b)) == 0) {
        return Some("bad token");
    }
    let session = params.get("session").and_then(Value::as_str).unwrap_or("");
    if !crate::tmux::Tmux::new().live(session) {
        return Some("no such chat");
    }
    if headers.get("sec-websocket-key").is_none() {
        return Some("missing websocket key");
    }
    None
}

pub fn refuse(stream: &mut TcpStream, why: &str) {
    let _ = stream.write_all(format!("HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{why}", why.len()).as_bytes());
}

/// Handshake, then serve the connection until it ends.
pub fn serve(mut stream: TcpStream, mut reader: BufReader<TcpStream>, key: &str, session: &str, cols: i64, rows: i64) {
    let response = format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", accept_key(key));
    if stream.write_all(response.as_bytes()).is_err() {
        return;
    }
    let socket = Socket { out: Arc::new(Mutex::new(stream)) };
    let (cols, rows) = (cols.clamp(1, 500) as u16, rows.clamp(1, 200) as u16);
    let tmux = crate::tmux::Tmux::new();
    let mut master: i32 = 0;
    let mut slave: i32 = 0;
    let size = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
    if unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), &size) } != 0 {
        return;
    }
    let (master_fd, slave_fd) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    let attach = ["-L".to_string(), tmux.socket(), "attach-session".to_string(), "-t".to_string(), format!("={session}")];
    let mut command = Command::new("tmux");
    command.args(&attach).env("TERM", "xterm-256color").env_remove("TMUX");
    let slave_for = |fd: &OwnedFd| fd.try_clone().map(Stdio::from);
    let (Ok(sin), Ok(sout), Ok(serr)) = (slave_for(&slave_fd), slave_for(&slave_fd), slave_for(&slave_fd)) else { return };
    command.stdin(sin).stdout(sout).stderr(serr);
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            libc::ioctl(0, libc::TIOCSCTTY as _, 0);
            Ok(())
        });
    }
    let Ok(mut child) = command.spawn() else { return };
    drop(slave_fd);
    let pid = child.id() as i32;
    let reader_fd = master_fd.try_clone().ok();
    let writer_fd = master_fd.try_clone().ok();
    let (Some(reader_fd), Some(writer_fd)) = (reader_fd, writer_fd) else { return };
    let pump_socket = socket.clone();
    let stopping = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let pump_stopping = stopping.clone();
    let pump = std::thread::spawn(move || {
        let mut pty = std::fs::File::from(reader_fd);
        let mut buf = vec![0u8; 16_384];
        loop {
            match pty.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if pump_socket.send(&buf[..n], BINARY).is_err() {
                        break;
                    }
                }
            }
        }
        if !pump_stopping.load(std::sync::atomic::Ordering::SeqCst) {
            pump_socket.close();
        }
    });
    let mut pty_in = std::fs::File::from(writer_fd);
    let outcome = messages(&mut reader, &socket, |opcode, data| {
        if opcode == TEXT {
            let size: Value = serde_json::from_slice(&data).map_err(|e| Stop::Protocol(format!("unparseable resize: {e}")))?;
            if size.get("cols").is_some_and(|c| !matches!(c, Value::Null | Value::Bool(false))) {
                set_winsize(master_fd.as_raw_fd(), to_i(size.get("rows")).clamp(1, 200) as u16, to_i(size.get("cols")).clamp(1, 500) as u16);
                unsafe { libc::kill(pid, libc::SIGWINCH) };
            }
        } else {
            pty_in.write_all(&data).map_err(|_| Stop::Io)?;
        }
        Ok(())
    });
    if let Err(Stop::Protocol(message)) = outcome {
        data_root::log_error("gui-terminal", "Quartermaster::WebSocket::ProtocolError", &message);
    }
    stopping.store(true, std::sync::atomic::Ordering::SeqCst);
    unsafe { libc::kill(pid, libc::SIGTERM) };
    let _ = child.wait();
    drop(pump);
}

/// Yields each complete data message until the peer closes or the connection drops; control frames are
/// answered here.
fn messages(reader: &mut impl Read, socket: &Socket, mut each: impl FnMut(u8, Vec<u8>) -> Result<(), Stop>) -> Result<(), Stop> {
    let (mut message, mut opcode): (Option<Vec<u8>>, u8) = (None, 0);
    loop {
        let Some((fin, op, data)) = read_frame(reader)? else { return Ok(()) };
        match op {
            CLOSE => {
                socket.close();
                return Ok(());
            }
            PING => {
                let _ = socket.send(&data, PONG);
                continue;
            }
            PONG => continue,
            0 => {
                let Some(m) = message.as_mut() else { return Err(Stop::Protocol("continuation without a message".into())) };
                m.extend(data);
                if m.len() > MAX_PAYLOAD {
                    return Err(Stop::Protocol("message too large".into()));
                }
            }
            _ => {
                message = Some(data);
                opcode = op;
            }
        }
        if fin {
            if let Some(m) = message.take() {
                each(opcode, m)?;
            }
        }
    }
}
