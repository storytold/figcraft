//! Localhost JSON-lines control server: each line `{"id", "method", "params"}` is queued for the UI
//! thread; the reply line `{"id", "ok", "result" | "error"}` is written when the frame handled it.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use figcraft_engine::events::{self, EventLog};
use figcraft_ui_egui::{ControlReply, ControlRequest, RequestGate, RequestState};
use serde_json::{Value, json};

type Queue = Arc<Mutex<Vec<ControlRequest>>>;
type CtxSlot = Arc<Mutex<Option<egui::Context>>>;

const MAX_LINE: usize = 32 * 1024 * 1024;
/// Connections served at once, one thread each. Room for a few hundred AI agents on one file
/// (each `figcraft-cli call` is one short connection); still a bound, so a runaway client cannot
/// take every thread and file descriptor of the app.
#[cfg(not(test))]
const MAX_CONNECTIONS: usize = 256;
/// Requests waiting for the UI thread, which runs them one at a time: one per connection at most.
#[cfg(not(test))]
const MAX_QUEUED_REQUESTS: usize = 256;
/// The tests fill both limits with real sockets, three file descriptors a connection: they use
/// small ones so they pass where a process may open only 1024 files.
#[cfg(test)]
const MAX_CONNECTIONS: usize = 8;
#[cfg(test)]
const MAX_QUEUED_REQUESTS: usize = 8;
const TIMEOUT: Duration = Duration::from_secs(60);
/// The longest a request's `timeoutMs` can make it wait: a client can raise the wait, never lift it.
const MAX_TIMEOUT: Duration = Duration::from_secs(600);
/// How long the accepting thread waits after a failed accept. Accepting can keep failing, for
/// example while the process has no file descriptors left, and retrying at once would keep a
/// core busy until it recovers.
const ACCEPT_RETRY: Duration = Duration::from_millis(100);
/// The longest the accepting thread spends on a connection it refuses: telling it why, then
/// letting it close.
const REFUSAL_WRITE: Duration = Duration::from_millis(200);
/// How often a connection waiting for the UI looks whether its client is still there.
const CLIENT_CHECK: Duration = Duration::from_secs(1);
/// How long after it was sent a byte a client that left is seen to have refused it. The channel
/// is loopback only: the refusal comes back at once.
const PROBE_ANSWER: Duration = Duration::from_millis(100);

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

/// What a connection waiting for the UI sees of its client.
enum Client {
    /// Nothing says it left.
    There,
    /// It stopped sending, and was just sent the byte that tells whether it left.
    Asked,
    /// It stopped sending but took the byte: it still reads, and has nothing more to show.
    Listening,
    Gone,
}

/// Look at the client without waiting and without taking anything it sent. `asked`: it was
/// already sent the byte.
///
/// A client that left and one that only closed its sending side look the same from here: nothing
/// more comes from either. One byte tells them apart: the first refuses it and the connection
/// breaks, the second takes it and goes on waiting for its reply. The byte is a space, which JSON
/// allows before the reply.
fn look_at(client: &TcpStream, asked: bool) -> Client {
    let no_wait = |e: &std::io::Error| matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted);
    if client.set_nonblocking(true).is_err() {
        return Client::There;
    }
    let seen = match client.peek(&mut [0]) {
        // The end of what it sent reads the same once it has left. What came of the byte tells
        // the two apart: refused, the connection holds the error.
        Ok(0) if asked => match client.take_error() {
            Ok(None) => Client::Listening,
            _ => Client::Gone,
        },
        Ok(0) => {
            let mut to = client;
            match to.write(b" ") {
                Ok(1) => Client::Asked,
                // No room for it now: asked at the next look.
                Err(e) if no_wait(&e) => Client::There,
                _ => Client::Gone,
            }
        }
        Ok(_) => Client::There,
        Err(e) if no_wait(&e) => Client::There,
        Err(_) => Client::Gone,
    };
    // A connection that stays without waits can no longer be served.
    if client.set_nonblocking(false).is_err() { Client::Gone } else { seen }
}

/// [`wait_for_reply`] for a request whose client may leave meanwhile (killed, out of budget, its
/// own timeout). `None` when it left: a request the UI had not started is cancelled there and
/// then, where it would otherwise run for nobody when its turn came.
fn wait_while_client_stays(
    client: &TcpStream,
    rx: &std::sync::mpsc::Receiver<ControlReply>,
    gate: &RequestGate,
    timeout: Duration,
) -> Option<Result<ControlReply, WaitFailure>> {
    let until = std::time::Instant::now() + timeout;
    let left = || until.saturating_duration_since(std::time::Instant::now());
    let mut asked = false;
    loop {
        let pause = if asked { PROBE_ANSWER } else { CLIENT_CHECK };
        if left() <= pause {
            return Some(wait_for_reply(rx, gate, left()));
        }
        match rx.recv_timeout(pause) {
            Ok(reply) => return Some(Ok(reply)),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Some(Err(WaitFailure::Disconnected)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        match look_at(client, asked) {
            Client::There => {}
            Client::Asked => asked = true,
            Client::Listening => return Some(wait_for_reply(rx, gate, left())),
            Client::Gone => {
                // Started already, it takes effect; nobody is there to read what it answers.
                gate.cancel_if_open();
                return None;
            }
        }
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

/// A running control server: the port it listens on and the way to end it.
pub struct Stop {
    port: u16,
    stopping: Arc<AtomicBool>,
    /// The accepted connections, to end them with the server.
    open: Open,
    accepting: std::thread::JoinHandle<()>,
}

/// A handle on each connection still served, under the number the server gave it.
type Open = Arc<Mutex<Vec<(u64, TcpStream)>>>;

/// Lets go of a connection's handle in [`Open`] when its thread ends, however it ends. Without
/// it every connection ever accepted kept a file descriptor, and after a thousand requests the
/// app could open no file and serve no connection.
struct Forget {
    open: Open,
    id: u64,
}

impl Drop for Forget {
    fn drop(&mut self) {
        self.open.lock().unwrap_or_else(std::sync::PoisonError::into_inner).retain(|(id, _)| *id != self.id);
    }
}

impl Stop {
    /// The port the server listens on (the one the system chose when 0 was asked).
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Stop accepting, free the port and end the open connections. The accepting thread waits in
    /// `accept`: a connection from here wakes it, and it is only joined when that got through.
    pub fn stop(self) {
        self.stopping.store(true, Ordering::SeqCst);
        if TcpStream::connect(("127.0.0.1", self.port)).is_ok() {
            let _ = self.accepting.join();
        }
        for (_, connection) in self.open.lock().unwrap_or_else(std::sync::PoisonError::into_inner).drain(..) {
            let _ = connection.shutdown(Shutdown::Both);
        }
    }
}

/// Listen on `port` (0: a free port the system chooses) and serve the connections on their own
/// threads. The error says why the server is not there; the app goes on without it.
pub fn start(port: u16, busy: Duration, queue: Queue, ctx: CtxSlot, events: Arc<EventLog>) -> std::io::Result<Stop> {
    let bound = bind(port, busy).and_then(|l| l.local_addr().map(|a| (l, a.port())));
    let (listener, port) = bound.inspect_err(|e| {
        log::error!("control server: can't bind 127.0.0.1:{port}: {e} (another app may own it; pick another port)");
        eprintln!("FigCraft control server failed to bind port {port}: {e}");
    })?;
    eprintln!("FigCraft control server listening on 127.0.0.1:{port}");
    let stopping = Arc::new(AtomicBool::new(false));
    let open: Open = Arc::default();
    let (stopped, kept) = (stopping.clone(), open.clone());
    let accepting = std::thread::Builder::new().name("figcraft-control".into()).spawn(move || {
        let mut workers = Vec::new();
        let mut served = 0u64;
        for stream in accepted(listener.incoming(), || std::thread::sleep(ACCEPT_RETRY)) {
            if stopped.load(Ordering::SeqCst) {
                break;
            }
            // The handle the server ends the connection with; its thread lets go of it.
            let forget = match stream.try_clone() {
                Ok(copy) => {
                    served += 1;
                    kept.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((served, copy));
                    Some(Forget { open: kept.clone(), id: served })
                }
                Err(e) => {
                    log::error!("control server: connection not tracked: {e}");
                    None
                }
            };
            spawn_tracked(stream, queue.clone(), ctx.clone(), events.clone(), &mut workers, TIMEOUT, forget);
        }
    });
    let accepting = accepting.inspect_err(|e| log::error!("control server thread: {e}"))?;
    Ok(Stop { port, stopping, open, accepting })
}

/// The connections `incoming` accepts, calling `wait` after each failed accept.
fn accepted<T>(incoming: impl Iterator<Item = std::io::Result<T>>, mut wait: impl FnMut()) -> impl Iterator<Item = T> {
    incoming.filter_map(move |stream| stream.map_err(|_| wait()).ok())
}

#[cfg(test)]
fn spawn_connection(
    stream: TcpStream,
    queue: Queue,
    ctx: CtxSlot,
    events: Arc<EventLog>,
    workers: &mut Vec<std::thread::JoinHandle<()>>,
    timeout: Duration,
) {
    spawn_tracked(stream, queue, ctx, events, workers, timeout, None);
}

/// Serve `stream` on its own thread, or refuse it over the limit. `forget` goes with the
/// connection: dropped when it is refused, when no thread could start or when its thread ends.
fn spawn_tracked(
    stream: TcpStream,
    queue: Queue,
    ctx: CtxSlot,
    events: Arc<EventLog>,
    workers: &mut Vec<std::thread::JoinHandle<()>>,
    timeout: Duration,
    forget: Option<Forget>,
) {
    workers.retain(|worker| !worker.is_finished());
    if workers.len() >= MAX_CONNECTIONS {
        refuse(stream);
        return;
    }
    match std::thread::Builder::new().name("figcraft-control-conn".into()).spawn(move || {
        let _forget = forget;
        if let Err(e) = serve_with_events(stream, queue, ctx, events, timeout) {
            // A client that leaves mid-line is ordinary; anything else must be seen.
            if !matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::UnexpectedEof
            ) {
                log::error!("control connection: {e}");
            }
        }
    }) {
        Ok(worker) => workers.push(worker),
        Err(e) => log::error!("control connection thread: {e}"),
    }
}

/// Tell a connection over the limit why it is not served, instead of dropping it without a word
/// (the client then saw only a reset). All of it is bounded by [`REFUSAL_WRITE`]: the accepting
/// thread never waits longer on a client that does not read, or that does not close.
fn refuse(stream: TcpStream) {
    log::warn!("control server: connection refused, {MAX_CONNECTIONS} are already open");
    let error = format!("too many control connections ({MAX_CONNECTIONS} at most): the app is busy, try again shortly");
    end_with(&stream, &error);
}

/// Say why a connection ends, in a line its client reads, then end it: a connection over the
/// limit, a message too large, a line that isn't JSON. All of it is bounded by [`REFUSAL_WRITE`].
fn end_with(mut stream: &TcpStream, error: &str) {
    let until = std::time::Instant::now() + REFUSAL_WRITE;
    if stream.set_write_timeout(Some(REFUSAL_WRITE)).is_err() {
        return;
    }
    // In one write: sent in pieces, the pieces still to go are dropped when the connection closes.
    let line = format!("{}\n", json!({"id": null, "ok": false, "error": error}));
    if stream.write_all(line.as_bytes()).is_err() || stream.shutdown(Shutdown::Write).is_err() {
        return;
    }
    // What the client sends is taken out until it closes: closed over a request not read, the
    // connection is reset, and the client gets an error where the refusal was.
    let mut unread = [0; 4096];
    loop {
        let left = until.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() || stream.set_read_timeout(Some(left)).is_err() {
            break;
        }
        if !matches!(stream.read(&mut unread), Ok(n) if n > 0) {
            break;
        }
    }
}

/// Serve one control connection: read bounded JSON lines, reject oversized frames and a
/// full queue, queue one request, wait under its gate, and write one reply. Non-JSON input
/// answers with an error and closes the connection so HTTP body lines cannot become commands.
/// Serve a connection with a private event log (the tests; the app shares its session's log).
#[cfg(test)]
fn serve(out: TcpStream, queue: Queue, ctx: CtxSlot, timeout: Duration) -> std::io::Result<()> {
    serve_with_events(out, queue, ctx, Arc::new(EventLog::default()), timeout)
}

fn serve_with_events(mut out: TcpStream, queue: Queue, ctx: CtxSlot, events: Arc<EventLog>, timeout: Duration) -> std::io::Result<()> {
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
            end_with(&out, "message too large");
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
                pending.push(ControlRequest { id: id.clone(), method, params, reply: tx, state: gate.clone() });
                drop(pending);
                if let Some(c) = ctx.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
                    c.request_repaint();
                }
                let Some(waited) = wait_while_client_stays(&out, &rx, &gate, waits) else {
                    // Nobody is there to answer, and what else this client sent is for nobody.
                    break;
                };
                match waited {
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
                end_with(&out, &format!("bad JSON: {e}"));
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
