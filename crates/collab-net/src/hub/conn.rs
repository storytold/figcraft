use super::hosting::{Open, spawn};
use super::limits::{PerAddress, ip_key};
use super::queue::{Blocks, HubIn, InFlight, Line, Out, wide_block};
use super::{
    Arc, AtomicBool, AtomicUsize, Bucket, Bytes, ClientMsg, CloseCode, CloseFrame, Duplex, Duration, ErrorResponse, Instant, IpAddr, Message, Meter,
    NetLimits, Ordering, Read, Readers, Receiver, RecvTimeoutError, Request, Response, Role, Sender, ServerMsg, SocketAddr, TcpListener, TcpStream,
    WebSocket, Write, config, io, message, shutdown, sync_channel,
};

/// How long a closing connection waits for the peer to hang up.
const LINGER: Duration = Duration::from_secs(2);

/// Signals (cursor chat, spotlight) a client may send per second on average, and in a burst;
/// beyond that they are dropped.
pub const SIGNAL_RATE: f64 = 20.0;

pub const SIGNAL_BURST: f64 = 40.0;

/// Longest request line an uptime check may have.
const MAX_CHECK_LINE: usize = 2048;

/// An uptime check: a plain `GET` of a path ending in `/healthz` is answered `200 ok` and closed.
/// True when it was one (the connection is done). Anything else is only peeked at, so a WebSocket
/// handshake goes on untouched.
fn health_check(stream: &mut TcpStream, until: Instant) -> bool {
    let mut buf = [0u8; MAX_CHECK_LINE];
    let line = loop {
        let n = match stream.peek(&mut buf) {
            Ok(0) | Err(_) => return false,
            Ok(n) => n,
        };
        let seen = buf.get(..n).unwrap_or_default();
        if !seen.starts_with(b"GET ") && !b"GET ".starts_with(seen) {
            return false;
        }
        if let Some(end) = seen.windows(2).position(|w| w == b"\r\n") {
            break seen.get(..end).unwrap_or_default().to_vec();
        }
        if n == buf.len() || Instant::now() >= until {
            return false;
        }
        // The rest of the line is on its way (it nearly always arrives in one piece).
        std::thread::sleep(Duration::from_millis(10));
    };
    let path = line.split(|b| *b == b' ').nth(1).unwrap_or_default();
    let path = path.split(|b| *b == b'?').next().unwrap_or_default();
    if !path.ends_with(b"/healthz") {
        return false;
    }
    // Read the request (its headers end with a blank line), so closing doesn't reset it.
    let mut req = Vec::new();
    let mut chunk = [0u8; 1024];
    while !req.windows(4).any(|w| w == b"\r\n\r\n") && req.len() < 16 << 10 {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => req.extend_from_slice(chunk.get(..n).unwrap_or_default()),
        }
    }
    let _ = stream
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 3\r\nCache-Control: no-store\r\nConnection: close\r\n\r\nok\n");
    let _ = stream.shutdown(std::net::Shutdown::Write);
    true
}

/// The client a trusted proxy names: the last address of its last `X-Forwarded-For` (the one the
/// proxy added; any before it are whatever the client sent).
fn forwarded_for(req: &Request) -> Option<IpAddr> {
    let last = req.headers().get_all("x-forwarded-for").iter().next_back()?.to_str().ok()?;
    let entry = last.rsplit(',').next()?.trim();
    entry.parse::<IpAddr>().ok().or_else(|| entry.parse::<SocketAddr>().ok().map(|a| a.ip()))
}

/// What every connection shares: the count per address, the bytes queued for all of them and the
/// bytes in flight to the hub.
#[derive(Clone)]
pub(super) struct Shared {
    pub(super) per_address: PerAddress,
    pub(super) queued: Arc<AtomicUsize>,
    pub(super) in_flight: InFlight,
    pub(super) blocks: Blocks,
    /// Bytes being read into messages, all connections together, and who is reading them.
    pub(super) reading: Arc<AtomicUsize>,
    pub(super) readers: Readers,
}

pub(super) fn accept(listener: &TcpListener, hub: &Sender<HubIn>, stop: &AtomicBool, open: &Arc<AtomicUsize>, limits: NetLimits, shared: Shared) {
    let mut next = 0u64;
    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                // Out of sockets or similar: don't spin.
                log::warn!("live session: accept failed: {e}");
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
        };
        if open.load(Ordering::Relaxed) >= limits.max_connections {
            log::info!("live session: too many connections; refusing one");
            continue;
        }
        open.fetch_add(1, Ordering::Relaxed);
        let guard = Open(open.clone());
        next += 1;
        let (conn, hub, shared) = (next, hub.clone(), shared.clone());
        if let Err(e) = spawn("live-conn", move || serve(stream, conn, &hub, limits, guard, &shared)) {
            log::warn!("live session: can't start a connection thread: {e}");
        }
    }
}

/// A close reason that fits a WebSocket close frame (123 bytes).
pub(super) fn reason(s: &str) -> String {
    let mut end = s.len().min(123);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or_default().to_string()
}

pub(super) fn encode(msg: &ServerMsg) -> Option<Bytes> {
    match message::encode(msg) {
        Ok(b) => Some(Bytes::from(b)),
        Err(e) => {
            log::warn!("live session: can't encode a message: {e}");
            None
        }
    }
}

/// Queue `Refused {reason}` and a close for one connection (small: not counted against its caps).
pub(super) fn refuse(out: &Line, why: &str) {
    refused(out, why);
}

/// [`refuse`], and whether the close was queued (it isn't when the queue is full).
pub(super) fn refused(out: &Line, why: &str) -> bool {
    if let Some(b) = encode(&ServerMsg::Refused { reason: why.to_string() }) {
        let _ = out.tx.try_send(Out::Bytes(b, None));
    }
    out.tx.try_send(Out::Close(CloseCode::Policy, reason(why))).is_ok()
}

/// What the server tells a client when it is too busy to take more from it.
const BUSY: &str = "the server is busy: try again in a moment";

/// Read the next binary message (skipping pings, until `deadline`) and its size, or why the connection should end.
fn next_message(ws: &mut WebSocket<Duplex>, max: usize, deadline: Option<Instant>) -> Result<Option<(ClientMsg, usize)>, String> {
    loop {
        return match ws.read() {
            Ok(Message::Binary(b)) => {
                // Whole now: it counts as in flight from here, no longer as being read.
                ws.get_mut().read_done();
                message::decode::<ClientMsg>(&b, max).map(|m| Some((m, b.len()))).map_err(|_| "that message couldn't be read".to_string())
            }
            Ok(Message::Text(_)) => Err("only binary messages are spoken here".into()),
            // Pings keep the per-read timeout alive, but not the overall deadline (slowloris), and
            // a message trickled in is cut off by it (`Duplex::read_until`).
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) | Err(tungstenite::Error::Io(_))
                if deadline.is_some_and(|d| Instant::now() >= d) =>
            {
                Err("no hello in time".into())
            }
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => continue,
            Ok(Message::Close(_)) => Ok(None),
            Err(tungstenite::Error::Capacity(_)) => Err(too_large(max)),
            // The reading budget's refusal (see `Meter`).
            Err(tungstenite::Error::Io(e)) if e.kind() == io::ErrorKind::Other => Err(e.to_string()),
            Err(_) => Ok(None),
        };
    }
}

/// Why a message over `max` bytes is refused.
fn too_large(max: usize) -> String {
    match max >> 20 {
        0 => format!("message too large: this server takes at most {} KiB at once here", max >> 10),
        mib => format!("message too large: this server takes at most {mib} MiB at once (a change that big can't be kept here)"),
    }
}

/// One connection's reader (the writer runs on its own thread).
fn serve(mut stream: TcpStream, conn: u64, hub: &Sender<HubIn>, limits: NetLimits, open: Open, shared: &Shared) {
    let _ = stream.set_nodelay(true);
    let hello_deadline = Instant::now() + limits.hello_timeout;
    if stream.set_read_timeout(Some(limits.hello_timeout)).is_err() || stream.set_write_timeout(Some(limits.write_timeout)).is_err() {
        return;
    }
    if health_check(&mut stream, Instant::now() + limits.hello_timeout) {
        return;
    }
    let Ok(peer) = stream.peer_addr().map(|a| a.ip().to_canonical()) else { return };
    let proxied = limits.trusted_proxy.is_some_and(|p| p.to_canonical() == peer);
    let Ok((mut rd, wr, sock)) = Duplex::split(stream, None) else { return };
    // The handshake and the hello must be in by the deadline, however slowly they trickle in.
    rd.read_until(Some(hello_deadline));
    let cfg = config(limits.max_client_message);
    let mut forwarded = None;
    // The handshake callback's type is tungstenite's (its error is an HTTP response).
    #[allow(clippy::result_large_err)]
    let read_client = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        if proxied {
            forwarded = forwarded_for(req);
        }
        Ok(resp)
    };
    let mut ws = match tungstenite::accept_hdr_with_config(rd, read_client, Some(config(limits.max_hello))) {
        Ok(ws) => ws,
        Err(e) => {
            log::debug!("live session: handshake failed: {e}");
            shutdown(&sock);
            return;
        }
    };
    if proxied && forwarded.is_none() {
        log::debug!("live session: connection {conn} came through the proxy without X-Forwarded-For");
    }
    let ip = ip_key(forwarded.unwrap_or(peer));
    let counted = shared.per_address.enter_all(ip, limits.max_per_address, limits.max_connections);
    let wide = wide_block(ip);
    let block = shared.blocks.get(wide);
    ws.get_mut().meter(Meter::new(
        conn,
        &sock,
        &shared.readers,
        block.reading.clone(),
        shared.reading.clone(),
        limits.max_reading_block,
        limits.max_reading_total,
    ));
    let (tx, rx) = sync_channel::<Out>(limits.queue.max(4));
    let out = Line {
        tx,
        queued: Arc::default(),
        block: block.queued.clone(),
        all: shared.queued.clone(),
        max: limits.max_queued_bytes,
        max_block: limits.max_queued_block,
        max_all: limits.max_queued_total,
    };
    let writer =
        match sock.try_clone().map(|s| spawn("live-write", move || write_loop(WebSocket::from_raw_socket(wr, Role::Server, Some(cfg)), &rx, &s))) {
            Ok(Ok(t)) => t,
            _ => {
                shutdown(&sock);
                return;
            }
        };
    let end = |why: Option<&str>| {
        if let Some(why) = why {
            refuse(&out, why);
        } else {
            let _ = out.tx.try_send(Out::Close(CloseCode::Normal, String::new()));
        }
    };
    if counted.is_none() {
        log::info!("event=refused ip={ip} reason=connections_per_address");
        end(Some("too many connections from your address: close some and try again"));
    } else {
        let hello = next_message(&mut ws, limits.max_hello, Some(hello_deadline));
        // In time or not, the hello's deadline is over: what's read next has timeouts of its own.
        ws.get_mut().read_until(None);
        match hello {
            Ok(Some((
                first @ (ClientMsg::Hello(_)
                | ClientMsg::Manage(_)
                | ClientMsg::OpenLibrary(_)
                | ClientMsg::OpenPlugins(_)
                | ClientMsg::OpenHosting(_)
                | ClientMsg::Resolve(_)
                | ClientMsg::ResolveTeam(_)),
                bytes,
            ))) => {
                // Sessions and the operator may use the last connections; one-shot requests
                // (hosting, libraries, browsing plugins) leave them free.
                let session = matches!(first, ClientMsg::Hello(_) | ClientMsg::Manage(_));
                // A publish uploads a whole file; a large one on a slow link takes the server a
                // while to receive and answer. The manage/hosting connections that carry uploads
                // wait the longer request-reply timeout, not the short request idle.
                let idle = match first {
                    ClientMsg::Manage(_) | ClientMsg::OpenHosting(_) => limits.request_reply_timeout,
                    ClientMsg::Hello(_) => limits.idle_timeout,
                    _ => limits.request_idle,
                };
                let max = match &first {
                    ClientMsg::OpenPlugins(o) if o.key.is_empty() => limits.max_request,
                    _ => limits.max_client_message,
                };
                let reserved = !session && open.0.load(Ordering::Relaxed) > limits.max_connections.saturating_sub(limits.reserve_for_sessions);
                // (In flight is only taken when the connection goes on: the hub gives it back.)
                if reserved || !shared.in_flight.take(bytes, limits.max_in_flight) {
                    end(Some(BUSY));
                } else {
                    let (admitted, verdict) = sync_channel(1);
                    let opened =
                        sock.try_clone().is_ok_and(|s| hub.send(HubIn::Open { conn, ip, first, out: out.clone(), sock: s, bytes, admitted }).is_ok());
                    // Nothing more is read until the hub took the connection in (its token checked):
                    // only then may it send messages of the full size.
                    if opened && verdict.recv().is_ok() && ws.get_ref().set_read_timeout(Some(idle)).is_ok() {
                        ws.set_config(|c| *c = config(max));
                        read_loop(&mut ws, conn, hub, limits, max, &shared.in_flight, &end);
                        let _ = hub.send(HubIn::Gone { conn });
                    }
                }
            }
            Ok(Some(_)) => end(Some("say hello first")),
            Ok(None) => {}
            Err(why) => end(Some(&why)),
        }
    }
    drop(counted);
    // The block's counters go with the last handle on them (the table forgets it then).
    drop(block);
    ws.get_mut().read_until(None);
    linger(&mut ws);
    drop(out);
    // The writer closes the socket once it has sent what's queued (a refusal, a close).
    let _ = writer.join();
    shutdown(&sock);
}

fn read_loop(
    ws: &mut WebSocket<Duplex>,
    conn: u64,
    hub: &Sender<HubIn>,
    limits: NetLimits,
    max: usize,
    in_flight: &InFlight,
    end: &dyn Fn(Option<&str>),
) {
    let mut bucket = Bucket::new(limits.rate, limits.burst);
    let mut signals = Bucket::new(SIGNAL_RATE, SIGNAL_BURST);
    loop {
        match next_message(ws, max, None) {
            Ok(Some((msg, bytes))) => {
                let now = Instant::now();
                if !bucket.take(now) {
                    log::info!("live session: connection {conn} sends too much; disconnecting it");
                    return end(Some("too many messages"));
                }
                // Typing sends a chat signal per keystroke; more than this is dropped.
                if matches!(msg, ClientMsg::Signal(_)) && !signals.take(now) {
                    log::debug!("live session: connection {conn} sends signals too fast; dropping one");
                    continue;
                }
                if !in_flight.take(bytes, limits.max_in_flight) {
                    log::info!("live session: the hub is behind; dropping connection {conn}");
                    return end(Some(BUSY));
                }
                if hub.send(HubIn::Msg { conn, msg, bytes }).is_err() {
                    return;
                }
            }
            Ok(None) => return end(None),
            Err(why) => return end(Some(&why)),
        }
    }
}

fn write_loop(mut ws: WebSocket<Duplex>, rx: &Receiver<Out>, sock: &TcpStream) {
    let mut close = Some((CloseCode::Normal, String::new()));
    while let Ok(o) = rx.recv() {
        match o {
            Out::Bytes(b, _charge) => {
                if ws.send(Message::Binary(b)).is_err() {
                    close = None;
                    break;
                }
            }
            Out::Close(code, why) => {
                close = Some((code, why));
                break;
            }
        }
    }
    if let Some((code, why)) = close {
        // Close politely: the close frame, then no more writing; the reader drains what the peer
        // still sends until it hangs up (closing on unread data would reset the connection and
        // could lose the close frame), or until the linger runs out.
        let _ = ws.close(Some(CloseFrame { code, reason: why.into() }));
        let _ = sock.shutdown(std::net::Shutdown::Write);
        let until = Instant::now() + LINGER;
        while let Ok(_) | Err(RecvTimeoutError::Timeout) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            if Instant::now() >= until {
                break;
            }
        }
    }
    shutdown(sock);
}

/// Read and drop what the peer still sends after the close, until it hangs up (at most
/// [`LINGER`]). Raw bytes, not frames: after a frame over the size limit the WebSocket's reader
/// is mid-frame and mustn't be asked for more.
fn linger(ws: &mut WebSocket<Duplex>) {
    let until = Instant::now() + LINGER;
    let mut buf = [0u8; 4096];
    while Instant::now() < until && ws.get_ref().set_read_timeout(Some(Duration::from_millis(250))).is_ok() {
        match ws.get_mut().read(&mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => {}
            Err(_) => break,
        }
    }
}
