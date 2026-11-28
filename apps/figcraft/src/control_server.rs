//! Localhost JSON-lines control server: each line `{"id", "method", "params"}` is queued for the UI
//! thread; the reply line `{"id", "ok", "result" | "error"}` is written when the frame handled it.

use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use figcraft_engine::events::{self, EventLog};
use figcraft_ui_egui::{ControlReply, ControlRequest, RequestGate, RequestState};
use serde_json::{Value, json};

type Queue = Arc<Mutex<Vec<ControlRequest>>>;
type CtxSlot = Arc<Mutex<Option<egui::Context>>>;

const MAX_LINE: usize = 32 * 1024 * 1024;
/// Bytes of request lines all connections hold at once (#1111): a line is counted as it is read
/// and until its reply is written, so eight clients can't make the app hold eight full lines.
const MAX_IN_FLIGHT: usize = 64 * 1024 * 1024;
const MAX_CONNECTIONS: usize = 8;
const MAX_QUEUED_REQUESTS: usize = 8;
const TIMEOUT: Duration = Duration::from_secs(60);
/// The longest a request's `timeoutMs` can make it wait: a client can raise the wait, never lift it.
const MAX_TIMEOUT: Duration = Duration::from_secs(600);
/// How long the accepting thread waits after a failed accept. Accepting can keep failing, for
/// example while the process has no file descriptors left, and retrying at once would keep a
/// core busy until it recovers.
const ACCEPT_RETRY: Duration = Duration::from_millis(100);

/// Why a request's transport wait ended without a control reply.
#[derive(Debug, PartialEq, Eq)]
enum WaitFailure {
    Timeout,
    /// The UI side dropped its sender before replying (the app is closing).
    Disconnected,
    /// The UI started the request before the timeout and is still running it: it takes effect
    /// when it finishes, though nobody waits for its reply (#1673).
    StillRunning,
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
        // It may have finished just now: its reply is the truth then.
        RequestState::Running => rx.try_recv().map_err(|_| WaitFailure::StillRunning),
        RequestState::Open | RequestState::Cancelled => Err(WaitFailure::Timeout),
    }
}

/// How long to wait for the reply to request `v`: its `timeoutMs` (a whole number, at most
/// [`MAX_TIMEOUT`]), else `default` (#1696).
fn request_timeout(v: &Value, default: Duration) -> Duration {
    v.get("timeoutMs").and_then(Value::as_u64).map_or(default, |ms| Duration::from_millis(ms).min(MAX_TIMEOUT))
}

/// What a request that got no reply after `waited` is told. Only a request that needs a drawn frame
/// can be held up by the window being out of sight; any other was either never started (the app
/// was busy, so it was dropped and never applied) or is still running and will be applied (#1673).
fn timeout_message(failure: &WaitFailure, needs_frame: bool, waited: Duration) -> String {
    let secs = waited.as_secs_f64().ceil();
    match failure {
        WaitFailure::Disconnected => "the FigCraft window closed before answering".into(),
        WaitFailure::StillRunning => format!(
            "the app is still running this request after {secs} s: it will take effect when it finishes, but its reply is lost. Check the document before retrying"
        ),
        WaitFailure::Timeout if needs_frame => {
            "timed out waiting for the UI: screenshots and input need the FigCraft window on screen (is it minimised or covered?)".into()
        }
        WaitFailure::Timeout => {
            format!("timed out after {secs} s before the app started this request (it was busy, or its window isn't drawing): it was not applied")
        }
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

/// Bind `port`, waiting up to `busy` for it while another process holds it: an app started again on
/// other graphics ([`crate::graphics`]) starts beside the one ending, which still holds it.
pub fn bind(port: u16, busy: Duration) -> std::io::Result<TcpListener> {
    let since = std::time::Instant::now();
    loop {
        match TcpListener::bind(("127.0.0.1", port)) {
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && since.elapsed() < busy => std::thread::sleep(Duration::from_millis(50)),
            bound => return bound,
        }
    }
}

pub fn start(port: u16, busy: Duration, queue: Queue, ctx: CtxSlot, events: Arc<EventLog>) {
    let listener = match bind(port, busy) {
        Ok(l) => l,
        Err(e) => {
            log::error!("control server: can't bind 127.0.0.1:{port}: {e} (another app may own it; pick another port)");
            eprintln!("FigCraft control server failed to bind port {port}: {e}");
            return;
        }
    };
    eprintln!("FigCraft control server listening on 127.0.0.1:{port}");
    let spawned = std::thread::Builder::new().name("figcraft-control".into()).spawn(move || {
        let mut workers = Vec::new();
        let budget = Arc::new(AtomicUsize::new(0));
        for stream in accepted(listener.incoming(), || std::thread::sleep(ACCEPT_RETRY)) {
            spawn_connection(stream, queue.clone(), ctx.clone(), events.clone(), &budget, &mut workers, TIMEOUT);
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

/// Since when a connection has been waiting for its client (`None` while the UI runs one of its
/// requests).
type Idle = Arc<Mutex<Option<Instant>>>;

/// One connection's thread, a handle on its socket and since when it has been idle.
struct Worker {
    thread: std::thread::JoinHandle<()>,
    stream: TcpStream,
    idle: Idle,
}

fn spawn_connection(
    stream: TcpStream,
    queue: Queue,
    ctx: CtxSlot,
    events: Arc<EventLog>,
    budget: &Arc<AtomicUsize>,
    workers: &mut Vec<Worker>,
    timeout: Duration,
) {
    workers.retain(|worker| !worker.thread.is_finished());
    if workers.len() >= MAX_CONNECTIONS {
        // A client keeping slots it doesn't use can't lock others out (#1110): the connection idle
        // longest (between requests or in `events.poll`; clients reconnect) gives its slot up. One
        // whose request the UI is running keeps it.
        let idle = |w: &Worker| *w.idle.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let longest = workers.iter().enumerate().filter_map(|(i, w)| Some((i, idle(w)?))).min_by_key(|(_, since)| *since);
        let Some((i, _)) = longest else { return };
        let _ = workers.swap_remove(i).stream.shutdown(Shutdown::Both);
    }
    let Ok(handle) = stream.try_clone() else { return };
    let idle: Idle = Arc::new(Mutex::new(Some(Instant::now())));
    let (budget, mine) = (budget.clone(), idle.clone());
    match std::thread::Builder::new().name("figcraft-control-conn".into()).spawn(move || {
        let _ = serve_with_events(stream, queue, ctx, events, &budget, &mine, timeout);
    }) {
        Ok(thread) => workers.push(Worker { thread, stream: handle, idle }),
        Err(e) => log::error!("control connection thread: {e}"),
    }
}

/// Bytes of a line reserved from the shared budget, given back when dropped.
struct Held<'a> {
    budget: &'a AtomicUsize,
    bytes: usize,
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        self.budget.fetch_sub(self.bytes, Ordering::SeqCst);
    }
}

/// What [`read_line`] read.
enum Line<'a> {
    End,
    /// A line, its newline included if it had one; longer than [`MAX_LINE`] when it was too long.
    Read(Held<'a>),
    /// The connections hold [`MAX_IN_FLIGHT`] bytes already.
    Busy,
}

/// Read a line of at most [`MAX_LINE`] bytes plus its newline into `line`, taking its bytes from
/// `budget` as they arrive.
fn read_line<'a>(reader: &mut impl BufRead, line: &mut Vec<u8>, budget: &'a AtomicUsize) -> std::io::Result<Line<'a>> {
    let mut held = Held { budget, bytes: 0 };
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            return Ok(if line.is_empty() { Line::End } else { Line::Read(held) });
        }
        let room = MAX_LINE + 1 - line.len();
        let (take, done) = match buf.iter().position(|b| *b == b'\n') {
            Some(i) if i < room => (i + 1, true),
            _ => (buf.len().min(room), false),
        };
        if budget.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |used| used.checked_add(take).filter(|t| *t <= MAX_IN_FLIGHT)).is_err() {
            return Ok(Line::Busy);
        }
        held.bytes += take;
        line.extend_from_slice(&buf[..take]);
        reader.consume(take);
        if done || line.len() > MAX_LINE {
            return Ok(Line::Read(held));
        }
    }
}

/// Serve one control connection: read bounded JSON lines, reject oversized frames and a
/// full queue, queue one request, wait under its gate, and write one reply. Non-JSON input
/// answers with an error and closes the connection so HTTP body lines cannot become commands.
/// Serve a connection with a private event log (the tests; the app shares its session's log).
#[cfg(test)]
fn serve(out: TcpStream, queue: Queue, ctx: CtxSlot, timeout: Duration) -> std::io::Result<()> {
    serve_with_events(out, queue, ctx, Arc::new(EventLog::default()), &AtomicUsize::new(0), &Mutex::new(None), timeout)
}

fn serve_with_events(
    mut out: TcpStream,
    queue: Queue,
    ctx: CtxSlot,
    events: Arc<EventLog>,
    budget: &AtomicUsize,
    idle: &Mutex<Option<Instant>>,
    timeout: Duration,
) -> std::io::Result<()> {
    out.set_read_timeout(Some(timeout))?;
    out.set_write_timeout(Some(timeout))?;
    let mut reader = BufReader::new(out.try_clone()?);
    let set_idle = |since: Option<Instant>| *idle.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = since;
    loop {
        set_idle(Some(Instant::now()));
        // A new buffer each time: an idle connection keeps no large line's capacity.
        let mut line = Vec::new();
        let _held = match read_line(&mut reader, &mut line, budget)? {
            Line::End => break,
            Line::Read(held) => held,
            Line::Busy => {
                let error = "the control server holds too many large requests: retry when they are answered";
                writeln!(out, "{}", json!({"id": null, "ok": false, "error": error}))?;
                break;
            }
        };
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
                // A poll waits on the shared log, not on a frame: it needs no UI thread and no
                // request slot, and it works while the window is minimised.
                if method == "events.poll" {
                    let reply = match event_poll(&events, &params) {
                        Ok(result) => json!({"id": id, "ok": true, "result": result}),
                        Err(error) => json!({"id": id, "ok": false, "error": error}),
                    };
                    writeln!(out, "{reply}")?;
                    continue;
                }
                let waits = request_timeout(&v, timeout);
                let (tx, rx) = std::sync::mpsc::channel();
                let gate = RequestGate::new();
                let mut pending = queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                pending.retain(|request| !request.is_cancelled());
                if pending.len() >= MAX_QUEUED_REQUESTS {
                    drop(pending);
                    writeln!(out, "{}", json!({"id": id, "ok": false, "error": "control request queue is full"}))?;
                    continue;
                }
                let needs_frame = matches!(method.as_str(), "ui.screenshot" | "ui.input");
                set_idle(None);
                pending.push(ControlRequest { id: id.clone(), method, params, reply: tx, state: gate.clone() });
                drop(pending);
                if let Some(c) = ctx.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
                    c.request_repaint();
                }
                match wait_for_reply(&rx, &gate, waits) {
                    Ok(r) => match r.result {
                        Ok(res) => json!({"id": r.id, "ok": true, "result": res}),
                        Err(e) => json!({"id": r.id, "ok": false, "error": e}),
                    },
                    Err(failure) => json!({"id": id, "ok": false, "error": timeout_message(&failure, needs_frame, waits)}),
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

/// `events.poll {after?, timeoutMs?, topics?, limit?}`: read the shared log, waiting up to 25 s.
fn event_poll(log: &EventLog, params: &Value) -> Result<Value, String> {
    let after = params.get("after").and_then(Value::as_u64).unwrap_or(0);
    let timeout = params.get("timeoutMs").and_then(Value::as_u64).unwrap_or(20_000).min(25_000);
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(100).clamp(1, 1000) as usize;
    let topics = events::topics(params.get("topics"))?;
    let result = log.poll(after, &topics, limit, Duration::from_millis(timeout));
    serde_json::to_value(result).map_err(|e| format!("serialize events: {e}"))
}

#[cfg(test)]
#[path = "control_server_tests.rs"]
mod tests;
