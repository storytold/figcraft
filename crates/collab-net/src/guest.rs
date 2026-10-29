//! Joining a session: the client end of the transport, with reconnect.
//!
//! A supervisor thread connects, says hello, then writes (presence throttled to about 30 a
//! second, latest wins; a ping every keep-alive interval) while a reader thread takes in what the
//! host sends.
//! When the connection drops after having been live, it reconnects with growing pauses; a
//! refusal or the host ending the session is final.
//!
//! Publishing ([`Guest::publish`]) first uploads the document to a server over a managing
//! connection, then joins the new file with its edit link on the same thread.

use std::io;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use figcraft_collab::message::{self, ClientMsg, HostInfo, Presence, ServerMsg};
use figcraft_doc::Document;
use figcraft_engine::live::{Event, Published, Status};
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, Role as WsRole, WebSocket};
use tungstenite::{Bytes, Message};

use crate::inbox::Inbox;
use crate::invite::{Endpoint, Invite};
pub use crate::join::GuestConfig;
use crate::join::{Next, Outcome, Resuming, Seen, Taken, after, closed, hello, resolved, take, too_large, unresolved};
use crate::manage;
use crate::net::{Duplex, NetLimits, config, open, shutdown};
use crate::requests;
pub use crate::requests::Publish;
use crate::throttle::{PRESENCE_INTERVAL, Throttle, earliest};

/// A joined (or joining) session. Dropping it leaves.
pub struct Guest {
    cmd: Sender<Cmd>,
    thread: Option<JoinHandle<()>>,
}

enum Cmd {
    Presence(Presence),
    /// For the server, in order (document changes).
    Send(ClientMsg),
    Leave,
    /// The reader of connection `epoch` got its welcome.
    Welcomed(u64),
    /// Connection `epoch` ended.
    Lost(u64, Outcome),
}

impl Guest {
    /// Start joining (returns at once; progress arrives in `inbox`).
    pub fn start(mut cfg: GuestConfig, inbox: Inbox) -> io::Result<Guest> {
        let (tx, rx) = channel();
        let me = tx.clone();
        let thread = std::thread::Builder::new().name("live-guest".into()).spawn(move || {
            // A join code: the server says which file it opens, then the join goes on as with a link.
            if let Some(code) = cfg.code.clone() {
                inbox.push(Event::Status(Status::Connecting));
                let asked =
                    manage::resolve(&cfg.invite.server, &code, &cfg.limits, cfg.connect_timeout).map_err(|e| match e.strip_prefix(manage::REFUSED) {
                        Some(why) => Outcome::Refused(why.to_string()),
                        None => Outcome::Retry(e),
                    });
                if let Err(o) = asked.and_then(|(file, token)| resolved(&mut cfg, file, token)) {
                    if let Next::Stop(Some(e)) = after(unresolved(&cfg, o), &cfg, false, &mut 0, &inbox) {
                        inbox.push(e);
                    }
                    return;
                }
            }
            supervise(&cfg, &inbox, &rx, &me);
        })?;
        Ok(Guest { cmd: tx, thread: Some(thread) })
    }

    /// Upload `job.doc` to the server as a new file, then join it with its edit link (returns at
    /// once: [`Event::Published`] with the file's links, then the session, arrive in `inbox`).
    /// `cfg` says how to join; its invite is replaced by the new file's.
    pub fn publish(job: Publish, cfg: GuestConfig, inbox: Inbox) -> io::Result<Guest> {
        let (tx, rx) = channel();
        let me = tx.clone();
        let thread = std::thread::Builder::new().name("live-publish".into()).spawn(move || {
            inbox.push(Event::Status(Status::Connecting));
            let (cfg, published, hosting) = match upload(&job, cfg) {
                Ok(done) => done,
                Err(e) => return inbox.push(Event::Status(Status::Failed { reason: format!("Couldn't publish to the server: {e}") })),
            };
            // Before anyone hears the file is there, so what others join with shows with it. The
            // file's first links stay its owner's (this app joins with one): others get their own,
            // which stopping takes away again.
            let shared = job.share.then(|| {
                let owner = published.owner.as_deref().unwrap_or_default();
                let made = manage::share(&job.server, job.key.as_deref(), &published.file, owner, &[], true, &cfg.limits, cfg.connect_timeout);
                made.and_then(|s| s.ok_or_else(|| "the server didn't make the links".to_string()))
            });
            if let Some(h) = hosting {
                inbox.push(Event::Hosting(h));
            }
            inbox.push(Event::Published(published));
            if let Some(shared) = shared {
                inbox.push(Event::Shared(shared));
            }
            // Left while uploading: the file stays on the server, nobody joins it. Anything else
            // that came meanwhile (presence) waits for the session.
            let mut waiting = Vec::new();
            loop {
                match rx.try_recv() {
                    Ok(Cmd::Leave) | Err(TryRecvError::Disconnected) => return,
                    Ok(c) => waiting.push(c),
                    Err(TryRecvError::Empty) => break,
                }
            }
            for c in waiting {
                let _ = me.send(c);
            }
            supervise(&cfg, &inbox, &rx, &me);
        })?;
        Ok(Guest { cmd: tx, thread: Some(thread) })
    }

    /// Your latest presence.
    pub fn presence(&self, p: Presence) {
        let _ = self.cmd.send(Cmd::Presence(p));
    }

    /// A message from the sync client (sent once the connection is welcomed; dropped when it
    /// breaks: the next welcome starts over).
    pub fn send(&self, msg: ClientMsg) {
        let _ = self.cmd.send(Cmd::Send(msg));
    }

    /// Leave (returns at once: a connection attempt in progress ends on its own).
    pub fn leave(&mut self) {
        let _ = self.cmd.send(Cmd::Leave);
    }

    /// The connection thread has ended.
    pub fn finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

impl Drop for Guest {
    fn drop(&mut self) {
        self.leave();
        if let Some(t) = self.thread.take()
            && t.is_finished()
        {
            let _ = t.join();
        }
    }
}

/// A connected socket: reader and writer ends.
struct Conn {
    reader: WebSocket<Duplex>,
    writer: WebSocket<Duplex>,
    sock: TcpStream,
}

/// Keep `doc` on `server` as a new file named `name`, with the server's `key` (or, without one, as
/// anyone may on a server open to the public): its edit link, where it is with its links (and its
/// owner token without a key), and what the server says about its hosting (without a key).
pub(crate) fn upload_file(
    server: &Endpoint,
    key: Option<&str>,
    name: &str,
    doc: &Document,
    limits: &NetLimits,
    timeout: Duration,
) -> Result<(Invite, Published, Option<HostInfo>), String> {
    let publish = vec![requests::upload(name, doc)?];
    let (replies, hosting) = match key {
        Some(key) => (manage::request(server, key, publish, limits, timeout)?, None),
        None => {
            let (info, replies) = manage::hosting(server, "", "", publish, limits, timeout)?;
            (replies, Some(info))
        }
    };
    let (edit, published) = requests::uploaded(server, replies.into_iter().next())?;
    Ok((edit, published, hosting))
}

/// Upload the document; the join settings for the new file, its links and the server's hosting.
fn upload(job: &Publish, mut cfg: GuestConfig) -> Result<(GuestConfig, Published, Option<HostInfo>), String> {
    let (edit, published, hosting) = upload_file(&job.server, job.key.as_deref(), &job.name, &job.doc, &cfg.limits, cfg.connect_timeout)?;
    cfg.invite = edit;
    Ok((cfg, published, hosting))
}

fn connect(cfg: &GuestConfig) -> Result<Conn, String> {
    let (rd, wr, sock) = open(&cfg.invite.server, cfg.connect_timeout, &cfg.limits)?;
    let (reader, _) = tungstenite::client::client_with_config(cfg.invite.endpoint(), rd, Some(config(cfg.limits.max_server_message)))
        .map_err(|e| format!("{} doesn't answer as a FigCraft session: {e}", cfg.invite.server.display()))?;
    let writer = WebSocket::from_raw_socket(wr, WsRole::Client, Some(config(cfg.limits.max_client_message)));
    reader.get_ref().set_read_timeout(Some(cfg.limits.idle_timeout)).map_err(|e| e.to_string())?;
    Ok(Conn { reader, writer, sock })
}

fn supervise(cfg: &GuestConfig, inbox: &Inbox, rx: &Receiver<Cmd>, me: &Sender<Cmd>) {
    let resume: Arc<Resuming> = Arc::new(std::sync::Mutex::new(cfg.resume));
    let mut attempt = 0u32;
    let mut was_live = false;
    let mut pending: Option<Presence> = None;
    let mut epoch = 0u64;
    inbox.push(Event::Status(Status::Connecting));
    loop {
        epoch += 1;
        let outcome = match connect(cfg) {
            Ok(c) => session(c, cfg, inbox, rx, me, epoch, &resume, &mut pending, &mut was_live, &mut attempt),
            Err(e) => Outcome::Retry(e),
        };
        match after(outcome, cfg, was_live, &mut attempt, inbox) {
            Next::Stop(e) => {
                if let Some(e) = e {
                    inbox.push(e);
                }
                return;
            }
            Next::Reconnect(pause) => {
                let until = Instant::now() + pause;
                loop {
                    match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
                        Ok(Cmd::Leave) | Err(RecvTimeoutError::Disconnected) => return,
                        Ok(Cmd::Presence(p)) => pending = Some(p),
                        // From the old connection's client: the next welcome starts over.
                        Ok(_) => {}
                        Err(RecvTimeoutError::Timeout) => break,
                    }
                }
            }
        }
    }
}

/// One connection: say hello, then write until it ends.
fn session(
    c: Conn,
    cfg: &GuestConfig,
    inbox: &Inbox,
    rx: &Receiver<Cmd>,
    me: &Sender<Cmd>,
    epoch: u64,
    resume: &Arc<Resuming>,
    pending: &mut Option<Presence>,
    was_live: &mut bool,
    attempt: &mut u32,
) -> Outcome {
    let Conn { reader, mut writer, sock } = c;
    let reader = {
        let ctx = ReaderCtx { inbox: inbox.clone(), tx: me.clone(), epoch, cfg: cfg.clone(), resume: resume.clone() };
        match std::thread::Builder::new().name("live-guest-read".into()).spawn(move || read_loop(reader, &ctx)) {
            Ok(t) => t,
            Err(e) => {
                shutdown(&sock);
                return Outcome::Retry(e.to_string());
            }
        }
    };
    // A ping every keep-alive interval, whatever else is sent: presence gets no answer, and the
    // reader counts only what the host sends.
    let mut last_ping = Instant::now();
    let hello = hello(cfg, resume);
    let mut welcomed = false;
    let mut throttle = Throttle::new(PRESENCE_INTERVAL);
    let outcome = if send(&mut writer, &hello) {
        loop {
            let ping_at = last_ping + cfg.limits.keepalive;
            let wake = earliest([throttle.due(), Some(ping_at)]).unwrap_or(ping_at);
            let mut out = None;
            let mut now_send = Vec::new();
            match rx.recv_timeout(wake.saturating_duration_since(Instant::now())) {
                Ok(Cmd::Send(m)) if welcomed => now_send.push(m),
                // Made for the previous connection (the sync client sends again what the server
                // didn't get once it has caught up with this connection's welcome).
                Ok(Cmd::Send(_)) => {}
                Ok(Cmd::Presence(p)) if welcomed => out = throttle.offer(p, Instant::now()),
                Ok(Cmd::Presence(p)) => *pending = Some(p),
                Ok(Cmd::Welcomed(e)) if e == epoch => {
                    welcomed = true;
                    *was_live = true;
                    *attempt = 0;
                    out = pending.take().and_then(|p| throttle.offer(p, Instant::now()));
                }
                Ok(Cmd::Lost(e, o)) if e == epoch => break o,
                Ok(Cmd::Leave) | Err(RecvTimeoutError::Disconnected) => {
                    let _ = writer.close(Some(CloseFrame { code: CloseCode::Normal, reason: "".into() }));
                    break Outcome::Left;
                }
                // From an earlier connection.
                Ok(Cmd::Welcomed(_) | Cmd::Lost(..)) => {}
                Err(RecvTimeoutError::Timeout) => {}
            }
            if !now_send.iter().all(|m| send(&mut writer, m)) {
                break Outcome::Retry("the connection dropped".into());
            }
            let now = Instant::now();
            let out = out.or_else(|| throttle.take_due(now));
            let ok = out.is_none_or(|p| send(&mut writer, &ClientMsg::Presence(p)));
            let ok = ok
                && (now < last_ping + cfg.limits.keepalive || {
                    last_ping = now;
                    send(&mut writer, &ClientMsg::Ping)
                });
            if !ok {
                break Outcome::Retry("the connection dropped".into());
            }
        }
    } else {
        Outcome::Retry("the connection dropped".into())
    };
    shutdown(&sock);
    let _ = reader.join();
    outcome
}

/// Send one message; false when the connection is gone.
fn send(ws: &mut WebSocket<Duplex>, msg: &ClientMsg) -> bool {
    message::encode(msg).is_ok_and(|b| ws.send(Message::Binary(Bytes::from(b))).is_ok())
}

struct ReaderCtx {
    inbox: Inbox,
    tx: Sender<Cmd>,
    epoch: u64,
    cfg: GuestConfig,
    resume: Arc<Resuming>,
}

fn read_loop(mut ws: WebSocket<Duplex>, ctx: &ReaderCtx) {
    let mut seen = Seen::default();
    let outcome = loop {
        match ws.read() {
            Ok(Message::Binary(b)) => match message::decode::<ServerMsg>(&b, ctx.cfg.limits.max_server_message) {
                Ok(m) => {
                    let welcomed = || {
                        let _ = ctx.tx.send(Cmd::Welcomed(ctx.epoch));
                    };
                    match take(m, &ctx.cfg, &ctx.inbox, &mut seen, &ctx.resume, &welcomed) {
                        Taken::Continue | Taken::Welcomed => {}
                        Taken::End(o) => break o,
                    }
                }
                Err(e) => break Outcome::Refused(format!("the host sent a message this version can't read ({e})")),
            },
            Ok(Message::Close(f)) => break f.map_or_else(|| closed(0, ""), |f| closed(f.code.into(), &f.reason)),
            Ok(Message::Text(_)) => break Outcome::Refused("the host sent a text message".into()),
            Ok(_) => {}
            Err(tungstenite::Error::Capacity(_)) => break too_large(&ctx.cfg.limits),
            Err(e) => break Outcome::Retry(e.to_string()),
        }
    };
    let _ = ctx.tx.send(Cmd::Lost(ctx.epoch, outcome));
}
