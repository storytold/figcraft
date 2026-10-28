//! Localhost JSON-lines control server: each line `{"id", "method", "params"}` is queued for the UI
//! thread; the reply line `{"id", "ok", "result" | "error"}` is written when the frame handled it.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use figcraft_ui_egui::{ControlReply, ControlRequest, RequestGate, RequestState};
use serde_json::{Value, json};

type Queue = Arc<Mutex<Vec<ControlRequest>>>;
type CtxSlot = Arc<Mutex<Option<egui::Context>>>;

const MAX_LINE: usize = 32 * 1024 * 1024;
const MAX_CONNECTIONS: usize = 8;
const MAX_QUEUED_REQUESTS: usize = 8;
const TIMEOUT: Duration = Duration::from_secs(60);
/// How long the accepting thread waits after a failed accept. Accepting can keep failing, for
/// example while the process has no file descriptors left, and retrying at once would keep a
/// core busy until it recovers.
const ACCEPT_RETRY: Duration = Duration::from_millis(100);
/// Longest first line a client may send to authenticate.
const MAX_AUTH_LINE: usize = 4096;

/// Why a request's transport wait ended without a control reply.
#[derive(Debug, PartialEq, Eq)]
enum WaitFailure {
    Timeout,
    /// The UI side dropped its sender before replying (the app is closing).
    Disconnected,
}

/// Wait for the UI's reply, and settle a timeout under the same gate the UI's final commit uses.
/// A commit that got the gate first has already queued its reply: consume that reply instead of
/// reporting a timeout. If the gate is still open, cancel it so the UI writes nothing.
fn wait_for_reply(rx: &std::sync::mpsc::Receiver<ControlReply>, gate: &RequestGate, timeout: Duration) -> Result<ControlReply, WaitFailure> {
    match rx.recv_timeout(timeout) {
        Ok(reply) => Ok(reply),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => after_timeout(rx, gate),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(WaitFailure::Disconnected),
    }
}

/// The timeout branch of [`wait_for_reply`]. Kept separate so it can be tested deterministically
/// against a gate whose commit already happened.
fn after_timeout(rx: &std::sync::mpsc::Receiver<ControlReply>, gate: &RequestGate) -> Result<ControlReply, WaitFailure> {
    match gate.cancel_if_open() {
        RequestState::Replied => rx.try_recv().map_err(|_| WaitFailure::Disconnected),
        RequestState::Open | RequestState::Cancelled => Err(WaitFailure::Timeout),
    }
}

/// Deferred requests for a hidden window must not make the UI's queue grow without a bound.
pub fn drain_queue(queue: &Queue, target: &mut Vec<ControlRequest>) {
    target.retain(|request| !request.is_cancelled());
    let mut queue = queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    queue.retain(|request| !request.is_cancelled());
    let count = queue.len().min(MAX_QUEUED_REQUESTS.saturating_sub(target.len()));
    target.extend(queue.drain(..count));
}

/// A fresh random token for one launch: 32 bytes, hex.
fn random_token() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(format!("no secure random numbers: {e}")))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Write `token` to `path`, readable only by this user (the file is re-secured if it existed).
fn write_token(path: &Path, token: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(token.as_bytes())
}

/// Equality without an early exit on the first differing byte.
fn same_token(presented: &str, expected: &str) -> bool {
    presented.len() == expected.len() && presented.bytes().zip(expected.bytes()).fold(0u8, |d, (a, b)| d | (a ^ b)) == 0
}

/// Read the client's first line, which must be `{"id", "method": "auth", "params": {"token"}}`
/// with the launch's token. It is read one byte at a time, so nothing past the line is consumed
/// before [`serve`] reads the rest.
fn authenticate(stream: &mut TcpStream, token: &str) -> std::io::Result<()> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while line.len() <= MAX_AUTH_LINE && stream.read(&mut byte)? == 1 && byte[0] != b'\n' {
        line.push(byte[0]);
    }
    let request = serde_json::from_slice::<Value>(&line).ok();
    let id = request.as_ref().and_then(|v| v.get("id").cloned()).unwrap_or(Value::Null);
    let presented = request
        .as_ref()
        .filter(|v| v.get("method").and_then(Value::as_str) == Some("auth"))
        .and_then(|v| v.get("params")?.get("token")?.as_str());
    if presented.is_some_and(|t| same_token(t, token)) {
        writeln!(stream, "{}", json!({"id": id, "ok": true}))?;
        return Ok(());
    }
    writeln!(stream, "{}", json!({"id": id, "ok": false, "error": "control token missing or wrong"}))?;
    Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "control token refused"))
}

pub fn start(port: u16, queue: Queue, ctx: CtxSlot) {
    // Fail closed: without a token file the channel stays off rather than open to any local client.
    let token = match random_token() {
        Ok(token) => token,
        Err(e) => {
            log::error!("control server disabled: {e}");
            eprintln!("FigCraft control server disabled: {e}");
            return;
        }
    };
    let Some(path) = figcraft_mcp::token_path(port) else {
        log::error!("control server disabled: no config folder for the token");
        eprintln!("FigCraft control server disabled: no config folder for the token");
        return;
    };
    if let Err(e) = write_token(&path, &token) {
        log::error!("control server disabled: can't write the token to {}: {e}", path.display());
        eprintln!("FigCraft control server disabled: can't write the token to {}: {e}", path.display());
        return;
    }
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            log::error!("control server: can't bind 127.0.0.1:{port}: {e} (another app may own it; pick another port)");
            eprintln!("FigCraft control server failed to bind port {port}: {e}");
            return;
        }
    };
    eprintln!("FigCraft control server listening on 127.0.0.1:{port}; token in {}", path.display());
    let token: Arc<str> = Arc::from(token);
    let spawned = std::thread::Builder::new().name("figcraft-control".into()).spawn(move || {
        let mut workers = Vec::new();
        for stream in accepted(listener.incoming(), || std::thread::sleep(ACCEPT_RETRY)) {
            spawn_connection(stream, &token, queue.clone(), ctx.clone(), &mut workers, TIMEOUT);
        }
    });
    if let Err(e) = spawned {
        log::error!("control server thread: {e}");
    }
}

/// The connections `incoming` accepts, calling `wait` after each failed accept.
fn accepted<T>(incoming: impl Iterator<Item = std::io::Result<T>>, mut wait: impl FnMut()) -> impl Iterator<Item = T> {
    incoming.filter_map(move |stream| stream.map_err(|_| wait()).ok())
}

fn spawn_connection(stream: TcpStream, token: &Arc<str>, queue: Queue, ctx: CtxSlot, workers: &mut Vec<std::thread::JoinHandle<()>>, timeout: Duration) {
    workers.retain(|worker| !worker.is_finished());
    if workers.len() >= MAX_CONNECTIONS {
        return;
    }
    let token = token.clone();
    match std::thread::Builder::new().name("figcraft-control-conn".into()).spawn(move || {
        let mut stream = stream;
        if stream.set_read_timeout(Some(timeout)).is_err() || stream.set_write_timeout(Some(timeout)).is_err() {
            return;
        }
        if authenticate(&mut stream, &token).is_ok() {
            let _ = serve(stream, queue, ctx, timeout);
        }
    }) {
        Ok(worker) => workers.push(worker),
        Err(e) => log::error!("control connection thread: {e}"),
    }
}

/// Serve one control connection: read bounded JSON lines, reject oversized frames and a
/// full queue, queue one request, wait under its gate, and write one reply. Non-JSON input
/// answers with an error and closes the connection so HTTP body lines cannot become commands.
fn serve(mut out: TcpStream, queue: Queue, ctx: CtxSlot, timeout: Duration) -> std::io::Result<()> {
    out.set_read_timeout(Some(timeout))?;
    out.set_write_timeout(Some(timeout))?;
    let mut reader = BufReader::new(out.try_clone()?);
    let mut line = Vec::new();
    loop {
        line.clear();
        if (&mut reader).take((MAX_LINE + 1) as u64).read_until(b'\n', &mut line)? == 0 {
            break;
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.len() > MAX_LINE {
            writeln!(out, "{}", json!({"id": null, "ok": false, "error": "message too large"}))?;
            break;
        }
        let Ok(line) = std::str::from_utf8(&line) else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(v) => {
                let id = v.get("id").cloned().unwrap_or(Value::Null);
                let method = v.get("method").and_then(Value::as_str).unwrap_or("").to_string();
                let params = v.get("params").cloned().unwrap_or(json!({}));
                let (tx, rx) = std::sync::mpsc::channel();
                let gate = RequestGate::new();
                let mut pending = queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                pending.retain(|request| !request.is_cancelled());
                if pending.len() >= MAX_QUEUED_REQUESTS {
                    drop(pending);
                    writeln!(out, "{}", json!({"id": id, "ok": false, "error": "control request queue is full"}))?;
                    continue;
                }
                pending.push(ControlRequest { id: id.clone(), method, params, reply: tx, state: gate.clone() });
                drop(pending);
                if let Some(c) = ctx.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
                    c.request_repaint();
                }
                match wait_for_reply(&rx, &gate, timeout) {
                    Ok(r) => match r.result {
                        Ok(res) => json!({"id": r.id, "ok": true, "result": res}),
                        Err(e) => json!({"id": r.id, "ok": false, "error": e}),
                    },
                    Err(WaitFailure::Timeout) => {
                        json!({"id": id, "ok": false, "error": "timed out waiting for the UI: screenshots and input need the FigCraft window on screen (is it minimised or covered?)"})
                    }
                    Err(WaitFailure::Disconnected) => {
                        json!({"id": id, "ok": false, "error": "the FigCraft window closed before answering"})
                    }
                }
            }
            Err(e) => {
                // A line that isn't JSON ends the connection: a web page can make the browser send
                // an HTTP request here, and its header lines must not let the JSON lines of its body
                // through as commands.
                writeln!(out, "{}", json!({"id": null, "ok": false, "error": format!("bad JSON: {e}")}))?;
                break;
            }
        };
        writeln!(out, "{reply}")?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "control_server_tests.rs"]
mod tests;
