//! Carries tool calls from the MCP server Claude Code starts to the app that started it: a private
//! loopback socket for one run, opened with a key only that run's child processes get. The general
//! control channel stays off, so other programs can't reach this run.
//!
//! The first line is the key. Then each request is `{"method", "params"}` and each reply
//! `{"ok": true, "result"}` or `{"ok": false, "error"}`.

use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::time::Duration;

use figcraft_mcp::Backend;
use serde_json::{Value, json};

use crate::Cancel;

/// The environment variable that gives the child process `port:key`.
pub const ENV: &str = "FIGCRAFT_AGENT_BRIDGE";

const MAX_REQUEST: u64 = 32 * 1024 * 1024;
/// A rendered image comes back as base64 in one line.
const MAX_REPLY: u64 = 512 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(50);
/// Wrong keys tried before the run's socket closes.
const MAX_BAD_KEYS: usize = 8;

/// Compare keys without stopping at the first difference.
fn same_key(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |d, (x, y)| d | (x ^ y)) == 0
}

/// Read one line into `line`, waiting while the run goes on. `false`: closed, stopped or too long.
fn read_line(reader: &mut BufReader<TcpStream>, line: &mut Vec<u8>, cancel: &Cancel) -> bool {
    line.clear();
    loop {
        if cancel.load(Ordering::Relaxed) || line.len() as u64 > MAX_REQUEST {
            return false;
        }
        let room = MAX_REQUEST + 1 - line.len() as u64;
        match (&mut *reader).take(room).read_until(b'\n', line) {
            Ok(0) => return false,
            Ok(_) if line.last() == Some(&b'\n') => return true,
            Ok(_) => {}
            // A timeout keeps what arrived in `line`.
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => {}
            Err(_) => return false,
        }
    }
}

/// Answer one authenticated connection until it closes or the run stops.
fn answer(stream: TcpStream, mut reader: BufReader<TcpStream>, backend: &mut dyn Backend, cancel: &Cancel) -> std::io::Result<()> {
    let mut out = stream;
    let mut line = Vec::new();
    while read_line(&mut reader, &mut line, cancel) {
        let Ok(v) = serde_json::from_slice::<Value>(&line) else { break };
        let method = v.get("method").and_then(Value::as_str).unwrap_or_default();
        let params = v.get("params").cloned().unwrap_or_else(|| json!({}));
        let reply = match backend.call(method, params) {
            Ok(result) => json!({"ok": true, "result": result}),
            Err(error) => json!({"ok": false, "error": error}),
        };
        writeln!(out, "{reply}")?;
    }
    Ok(())
}

/// Serve the run's socket: the first connection with `key` gets `backend`, and when it ends so does
/// this. Returns at once when the run stops.
pub fn serve(listener: &TcpListener, key: &str, backend: &mut dyn Backend, cancel: &Cancel) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    let mut bad = 0;
    while !cancel.load(Ordering::Relaxed) && bad < MAX_BAD_KEYS {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(POLL);
                continue;
            }
            Err(e) => return Err(e),
        };
        // An accepted socket can inherit the listener's non-blocking mode.
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(POLL))?;
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut first = Vec::new();
        if read_line(&mut reader, &mut first, cancel) && same_key(first.trim_ascii(), key.as_bytes()) {
            return answer(stream, reader, backend, cancel);
        }
        bad += 1;
    }
    Ok(())
}

/// The child process's end: a [`Backend`] that sends every call to the app.
pub struct Client {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
    port: u16,
}

impl Client {
    /// Connect to the run's socket on `port` with `key`.
    pub fn connect(port: u16, key: &str) -> std::io::Result<Self> {
        let mut writer = TcpStream::connect(("127.0.0.1", port))?;
        let reader = BufReader::new(writer.try_clone()?);
        writeln!(writer, "{key}")?;
        Ok(Self { writer, reader, port })
    }

    /// From [`ENV`]'s `port:key`.
    pub fn from_env() -> Result<Self, String> {
        let value = std::env::var(ENV).map_err(|_| format!("{ENV} is not set: only FigCraft's Agent tab starts this"))?;
        let (port, key) = value.split_once(':').ok_or_else(|| format!("{ENV} is not port:key"))?;
        let port = port.parse().map_err(|_| format!("{ENV} is not port:key"))?;
        Self::connect(port, key).map_err(|e| format!("can't reach FigCraft: {e}"))
    }
}

impl Backend for Client {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        writeln!(self.writer, "{}", json!({"method": method, "params": params})).map_err(|e| format!("the FigCraft run ended: {e}"))?;
        let mut line = Vec::new();
        let read = (&mut self.reader).take(MAX_REPLY).read_until(b'\n', &mut line).map_err(|e| format!("the FigCraft run ended: {e}"))?;
        if read == 0 {
            return Err("the FigCraft run ended".into());
        }
        let reply: Value = serde_json::from_slice(&line).map_err(|e| format!("bad reply from FigCraft: {e}"))?;
        match reply.get("ok").and_then(Value::as_bool) {
            Some(true) => Ok(reply.get("result").cloned().unwrap_or(Value::Null)),
            _ => Err(reply.get("error").and_then(Value::as_str).unwrap_or("FigCraft refused the call").to_string()),
        }
    }

    fn has_ui(&self) -> bool {
        false
    }

    fn describe(&self) -> String {
        format!("FigCraft's Agent tab on port {}", self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;

    impl Backend for Echo {
        fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
            if method == "fail" { Err("no".into()) } else { Ok(json!({"method": method, "params": params})) }
        }
        fn has_ui(&self) -> bool {
            false
        }
        fn describe(&self) -> String {
            "echo".into()
        }
    }

    fn server(cancel: Cancel) -> (u16, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        (port, std::thread::spawn(move || serve(&listener, "secret", &mut Echo, &cancel).unwrap()))
    }

    #[test]
    fn calls_and_errors_travel_both_ways() {
        let (port, done) = server(Cancel::default());
        let mut client = Client::connect(port, "secret").unwrap();
        assert_eq!(client.call("x", json!({"a": 1})).unwrap(), json!({"method": "x", "params": {"a": 1}}));
        assert_eq!(client.call("fail", json!({})), Err("no".to_string()));
        drop(client);
        done.join().unwrap();
    }

    #[test]
    fn a_wrong_key_gets_nothing_and_the_right_one_still_can_connect() {
        let (port, done) = server(Cancel::default());
        let mut wrong = Client::connect(port, "guess").unwrap();
        assert!(wrong.call("x", json!({})).is_err());
        let mut right = Client::connect(port, "secret").unwrap();
        assert!(right.call("x", json!({})).is_ok());
        drop(right);
        done.join().unwrap();
    }

    #[test]
    fn too_many_wrong_keys_close_the_socket() {
        let (port, done) = server(Cancel::default());
        for _ in 0..MAX_BAD_KEYS {
            let mut wrong = Client::connect(port, "guess").unwrap();
            assert!(wrong.call("x", json!({})).is_err());
        }
        done.join().unwrap();
    }

    #[test]
    fn stopping_the_run_ends_a_waiting_connection() {
        let cancel = Cancel::default();
        let (port, done) = server(cancel.clone());
        let mut client = Client::connect(port, "secret").unwrap();
        assert!(client.call("x", json!({})).is_ok());
        cancel.store(true, Ordering::Relaxed);
        done.join().unwrap();
    }

    #[test]
    fn keys_compare_whole() {
        assert!(same_key(b"abc", b"abc"));
        assert!(!same_key(b"abc", b"abd"));
        assert!(!same_key(b"abc", b"abcd"));
    }
}
