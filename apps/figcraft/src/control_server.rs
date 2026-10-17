//! Localhost JSON-lines control server: each line `{"id", "method", "params"}` is queued for the UI
//! thread; the reply line `{"id", "ok", "result" | "error"}` is written when the frame handled it.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use figcraft_ui_egui::ControlRequest;
use serde_json::{Value, json};

type Queue = Arc<Mutex<Vec<ControlRequest>>>;
type CtxSlot = Arc<Mutex<Option<egui::Context>>>;

const MAX_LINE: usize = 32 * 1024 * 1024;
const MAX_CONNECTIONS: usize = 8;
const MAX_QUEUED_REQUESTS: usize = 8;
const TIMEOUT: Duration = Duration::from_secs(60);

/// Deferred requests for a hidden window must not make the UI's queue grow without a bound.
pub fn drain_queue(queue: &Queue, target: &mut Vec<ControlRequest>) {
    target.retain(|request| !request.is_cancelled());
    let mut queue = queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    queue.retain(|request| !request.is_cancelled());
    let count = queue.len().min(MAX_QUEUED_REQUESTS.saturating_sub(target.len()));
    target.extend(queue.drain(..count));
}

/// The connections a running server has open, so it can count and close them.
#[derive(Default)]
pub struct Open {
    streams: Mutex<Vec<(u64, TcpStream)>>,
    next: std::sync::atomic::AtomicU64,
}

impl Open {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<(u64, TcpStream)>> {
        self.streams.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn count(&self) -> usize {
        self.lock().len()
    }

    /// Close them all: their requests end and their clients see the connection drop.
    pub fn close_all(&self) {
        for (_, s) in self.lock().iter() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}

/// The control server (the MCP clients' way in, through `figcraft mcp --attach PORT` or
/// `figcraft-cli mcp --attach PORT`), listening on 127.0.0.1 until stopped or dropped.
pub struct ControlServer {
    port: u16,
    stop: Arc<std::sync::atomic::AtomicBool>,
    open: Arc<Open>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ControlServer {
    /// Listen on `port` (127.0.0.1 only).
    pub fn start(port: u16, queue: Queue, ctx: CtxSlot) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let open = Arc::new(Open::default());
        let (stop2, open2) = (stop.clone(), open.clone());
        let thread = std::thread::Builder::new().name("figcraft-control".into()).spawn(move || {
            let mut workers = Vec::new();
            for stream in listener.incoming().flatten() {
                if stop2.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                spawn_tracked(stream, queue.clone(), ctx.clone(), &mut workers, TIMEOUT, Some(open2.clone()));
            }
        })?;
        eprintln!("FigCraft control server listening on 127.0.0.1:{port}");
        Ok(Self { port, stop, open, thread: Some(thread) })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Clients connected now.
    pub fn connections(&self) -> usize {
        self.open.count()
    }

    /// Close every connection; keep listening.
    pub fn disconnect(&self) {
        self.open.close_all();
    }

    /// Stop listening and close every connection; the port is free once this returns.
    pub fn stop(mut self) {
        self.shut_down();
    }

    fn shut_down(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.open.close_all();
        // Wake the accept loop so it sees the flag and drops the listener.
        let _ = TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], self.port)), Duration::from_secs(1));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.shut_down();
        }
    }
}

/// [`spawn_tracked`] without tracking (the tests drive connections one by one).
#[cfg(test)]
fn spawn_connection(stream: TcpStream, queue: Queue, ctx: CtxSlot, workers: &mut Vec<std::thread::JoinHandle<()>>, timeout: Duration) {
    spawn_tracked(stream, queue, ctx, workers, timeout, None);
}

fn spawn_tracked(
    stream: TcpStream,
    queue: Queue,
    ctx: CtxSlot,
    workers: &mut Vec<std::thread::JoinHandle<()>>,
    timeout: Duration,
    open: Option<Arc<Open>>,
) {
    workers.retain(|worker| !worker.is_finished());
    if workers.len() >= MAX_CONNECTIONS {
        return;
    }
    let key = open.as_ref().and_then(|o| {
        let key = o.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        stream.try_clone().ok().map(|c| {
            o.lock().push((key, c));
            key
        })
    });
    match std::thread::Builder::new().name("figcraft-control-conn".into()).spawn(move || {
        let _ = serve(stream, queue, ctx.clone(), timeout);
        if let (Some(o), Some(key)) = (open, key) {
            o.lock().retain(|(k, _)| *k != key);
            // The AI agents tab and the tab strip show who's connected: let them redraw.
            if let Some(c) = ctx.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
                c.request_repaint();
            }
        }
    }) {
        Ok(worker) => workers.push(worker),
        Err(e) => log::error!("control connection thread: {e}"),
    }
}

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
                let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let mut pending = queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                pending.retain(|request| !request.is_cancelled());
                if pending.len() >= MAX_QUEUED_REQUESTS {
                    drop(pending);
                    writeln!(out, "{}", json!({"id": id, "ok": false, "error": "control request queue is full"}))?;
                    continue;
                }
                pending.push(ControlRequest { id: id.clone(), method, params, reply: tx, cancelled: cancelled.clone() });
                drop(pending);
                if let Some(c) = ctx.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
                    c.request_repaint();
                }
                match rx.recv_timeout(timeout) {
                    Ok(r) => match r.result {
                        Ok(res) => json!({"id": r.id, "ok": true, "result": res}),
                        Err(e) => json!({"id": r.id, "ok": false, "error": e}),
                    },
                    Err(_) => {
                        cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
                        json!({"id": id, "ok": false, "error": "timed out waiting for the UI: screenshots and input need the FigCraft window on screen (is it minimised or covered?)"})
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
