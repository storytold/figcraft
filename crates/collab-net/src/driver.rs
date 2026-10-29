//! The browser client's connection logic, free of the browser so it runs (and is tested) on any
//! target: a [`Driver`] reacts to socket events and to the clock the way the native guest's
//! threads do — hello on open (resuming as the same client after a drop), presence held until the
//! welcome and changes made for an earlier connection dropped (the sync client sends again what
//! the server didn't get), presence throttled to about 30 a second (latest wins), a ping every
//! keep-alive interval, a connection that goes silent counted as dropped, and reconnects with
//! growing pauses after a live connection drops. The browser layer (`web.rs`) feeds it the
//! WebSocket's events and wakes it at [`Driver::deadline`].
//!
//! A join by code ([`GuestConfig::code`]) first asks the server which file the code opens on a
//! connection of its own, then joins that file as with a link; a server that doesn't know the
//! code (or can't be reached) ends the join with the reason.
//!
//! Publishing ([`Driver::publish`]) and the requests sent in the background
//! ([`Driver::asking`]) first ask a server things on managing connections of their own (see
//! `exchange.rs`), each waiting for its answers as long as a large upload may take; a publish then
//! joins the new file.

use figcraft_collab::message::{self, ClientMsg, PROTOCOL, Presence, Resolve, ServerMsg};
use figcraft_engine::live::{Event, Status};
// `std::time::Instant` on native builds; the browser's clock in the web build.
use web_time::Instant;

use crate::exchange::{self, Exchange, Then};
use crate::inbox::Inbox;
use crate::invite::Endpoint;
use crate::join::{GuestConfig, Next, Outcome, Resuming, Seen, Taken, after, closed, hello, resolved, take, too_large, unresolved};
use crate::requests::Publish;
use crate::throttle::{PRESENCE_INTERVAL, Throttle, earliest};

/// A WebSocket the driver opens, writes and closes; what it receives comes back through the
/// driver's event methods.
pub(crate) trait Socket {
    /// Start connecting to `url` (one connection at a time: the driver closes the last first).
    fn open(&mut self, url: &str) -> Result<(), String>;
    /// Send one binary message; false when the connection can't take it.
    fn send(&mut self, bytes: &[u8]) -> bool;
    /// Close the connection normally; it reports nothing more.
    fn close(&mut self);
}

/// One connection attempt.
struct Conn {
    started: Instant,
    opened: bool,
    welcomed: bool,
    seen: Seen,
    /// The last ping (or the connection's start): one goes out every keep-alive interval, whatever
    /// else is sent, since presence gets no answer and only what the host sends counts as heard.
    last_ping: Instant,
    last_heard: Instant,
}

impl Conn {
    fn new(now: Instant) -> Self {
        Self { started: now, opened: false, welcomed: false, seen: Seen::default(), last_ping: now, last_heard: now }
    }
}

/// A joined (or joining) session over a [`Socket`].
pub(crate) struct Driver<S: Socket> {
    pub(crate) sock: S,
    cfg: GuestConfig,
    inbox: Inbox,
    conn: Option<Conn>,
    was_live: bool,
    attempt: u32,
    /// What a reconnect resumes with.
    resume: Resuming,
    /// Your latest presence while not welcomed (sent with the welcome).
    pending: Option<Presence>,
    throttle: Throttle<Presence>,
    reconnect_at: Option<Instant>,
    /// What this connection asks a server (a managing one), and what follows its answers.
    asking: Option<(Box<Exchange>, Then)>,
    done: bool,
}

impl<S: Socket> Driver<S> {
    fn new(sock: S, cfg: GuestConfig, inbox: Inbox) -> Self {
        Self {
            sock,
            cfg,
            inbox,
            conn: None,
            was_live: false,
            attempt: 0,
            resume: Resuming::default(),
            pending: None,
            throttle: Throttle::new(PRESENCE_INTERVAL),
            reconnect_at: None,
            asking: None,
            done: false,
        }
    }

    /// Start joining (progress arrives in `inbox`).
    pub(crate) fn start(sock: S, cfg: GuestConfig, inbox: Inbox, now: Instant) -> Self {
        let mut d = Self::new(sock, cfg, inbox);
        d.inbox.push(Event::Status(Status::Connecting));
        d.connect(now);
        d
    }

    /// Start with what `next` asks a server (a background request): the events it ends in go to
    /// `inbox`, then the join it ends in, if any, as `cfg` says.
    pub(crate) fn asking(sock: S, cfg: GuestConfig, next: exchange::Next, inbox: Inbox, now: Instant) -> Self {
        let mut d = Self::new(sock, cfg, inbox);
        d.follow(next, now);
        d
    }

    /// Start publishing `job`, then join the new file as `cfg` says (its invite is the new file's).
    pub(crate) fn publish(sock: S, cfg: GuestConfig, job: Publish, inbox: Inbox, now: Instant) -> Self {
        inbox.push(Event::Status(Status::Connecting));
        Self::asking(sock, cfg, exchange::publish(job), inbox, now)
    }

    /// The session is over (left, refused, ended or given up): nothing more will happen.
    pub(crate) fn done(&self) -> bool {
        self.done
    }

    /// Ask the next server, or tell the UI and join (or stop).
    fn follow(&mut self, next: exchange::Next, now: Instant) {
        match next {
            exchange::Next::Ask(ex, then) => {
                self.asking = Some((ex, then));
                self.connect(now);
            }
            exchange::Next::Done(events, join) => {
                for e in events {
                    self.inbox.push(e);
                }
                match join {
                    Some(invite) => {
                        self.cfg.invite = invite;
                        self.connect(now);
                    }
                    None => self.done = true,
                }
            }
        }
    }

    /// The server this connection is to.
    fn server(&self) -> &Endpoint {
        self.asking.as_ref().map_or(&self.cfg.invite.server, |(ex, _)| &ex.server)
    }

    fn connect(&mut self, now: Instant) {
        self.reconnect_at = None;
        self.conn = Some(Conn::new(now));
        if let Err(e) = self.sock.open(&self.server().url()) {
            self.dropped(e, now);
        }
    }

    /// The connection broke for this reason: a managing one fails, a session's tries again.
    fn dropped(&mut self, why: String, now: Instant) {
        if self.asking.is_some() { self.answered(Err(why), now) } else { self.end(Outcome::Retry(why), now) }
    }

    /// The managing connection is over (with why it failed): close it, then what follows.
    fn answered(&mut self, r: Result<(), String>, now: Instant) {
        let Some((ex, then)) = self.asking.take() else { return };
        if self.conn.take().is_some() {
            self.sock.close();
        }
        let answers = match r {
            Ok(()) => Ok(ex.answers()),
            Err(e) => Err(ex.failed(e)),
        };
        self.follow(then(answers), now);
    }

    /// The connection is open: say hello (or ask about the join code first, or open what is asked).
    pub(crate) fn opened(&mut self, now: Instant) {
        let Some(c) = self.conn.as_mut() else { return };
        c.opened = true;
        c.last_heard = now;
        let hello = match (&self.asking, &self.cfg.code) {
            (Some((ex, _)), _) => ex.opening().clone(),
            (None, Some(code)) => ClientMsg::Resolve(Resolve { protocol: PROTOCOL, client_version: message::client_version(), code: code.clone() }),
            (None, None) => hello(&self.cfg, &self.resume),
        };
        if !self.write(&hello) {
            self.dropped("the connection dropped".into(), now);
        }
    }

    /// A binary message on a managing connection: the next request, or what follows the answers.
    fn answer(&mut self, bytes: &[u8], now: Instant) {
        let Some((ex, _)) = self.asking.as_mut() else { return };
        let next = match message::decode::<ServerMsg>(bytes, self.cfg.limits.max_server_message) {
            Ok(m) => ex.take(m),
            Err(e) => Err(format!("the server sent a message this version can't read ({e})")),
        };
        let done = ex.done();
        match next {
            Err(e) => self.answered(Err(e), now),
            Ok(Some(req)) if !self.write(&req) => self.answered(Err("the connection to the server dropped".into()), now),
            Ok(_) if done => self.answered(Ok(()), now),
            Ok(_) => {}
        }
    }

    /// A binary message from the host (or the server asked).
    pub(crate) fn message(&mut self, bytes: &[u8], now: Instant) {
        let Some(c) = self.conn.as_mut() else { return };
        c.last_heard = now;
        if self.asking.is_some() {
            return self.answer(bytes, now);
        }
        if bytes.len() > self.cfg.limits.max_server_message {
            return self.end(too_large(&self.cfg.limits), now);
        }
        let taken = match message::decode::<ServerMsg>(bytes, self.cfg.limits.max_server_message) {
            // The join code's answer: join the file it opens, on a new connection.
            Ok(ServerMsg::Resolved { file, token }) if self.cfg.code.is_some() => {
                return match resolved(&mut self.cfg, file, token) {
                    Ok(()) => {
                        self.sock.close();
                        self.connect(now);
                    }
                    Err(o) => self.end(o, now),
                };
            }
            // Single-threaded: the connection counts as welcomed below, before the engine polls.
            Ok(m) => take(m, &self.cfg, &self.inbox, &mut c.seen, &self.resume, &|| {}),
            Err(e) => Taken::End(Outcome::Refused(format!("the host sent a message this version can't read ({e})"))),
        };
        match taken {
            Taken::Continue => {}
            Taken::End(o) => self.end(o, now),
            Taken::Welcomed => {
                c.welcomed = true;
                self.was_live = true;
                self.attempt = 0;
                if let Some(p) = self.pending.take() {
                    self.presence(p, now);
                }
            }
        }
    }

    /// A text message: not something the host sends.
    pub(crate) fn text(&mut self, now: Instant) {
        if self.conn.is_none() {
            return;
        }
        if self.asking.is_some() {
            return self.answered(Err("the server sent a text message".into()), now);
        }
        self.end(Outcome::Refused("the host sent a text message".into()), now);
    }

    /// The connection closed (`code` and `reason` from the close frame; 1006 when it just broke).
    pub(crate) fn closed(&mut self, code: u16, reason: &str, now: Instant) {
        let Some(opened) = self.conn.as_ref().map(|c| c.opened) else { return };
        if self.asking.is_some() {
            let why = match (opened, reason) {
                (false, _) => format!("no answer from {}", self.server().display()),
                (true, "") => "the server closed the connection".to_string(),
                (true, r) => format!("the server closed the connection: {r}"),
            };
            return self.answered(Err(why), now);
        }
        let outcome = match closed(code, reason) {
            Outcome::Retry(_) if !opened => Outcome::Retry(format!("no answer from {}", self.cfg.invite.server.display())),
            Outcome::Retry(_) if code == 1006 => Outcome::Retry("the connection dropped".into()),
            o => o,
        };
        self.end(outcome, now);
    }

    /// Your latest presence (throttled; held until the welcome).
    pub(crate) fn presence(&mut self, p: Presence, now: Instant) {
        if !self.conn.as_ref().is_some_and(|c| c.welcomed) {
            self.pending = Some(p);
            return;
        }
        if let Some(p) = self.throttle.offer(p, now)
            && !self.write(&ClientMsg::Presence(p))
        {
            self.end(Outcome::Retry("the connection dropped".into()), now);
        }
    }

    /// A message from the sync client, in order, on a welcomed connection; dropped otherwise (it
    /// was made for an earlier connection: the sync client sends again what the server didn't get
    /// once it has caught up with the next welcome).
    pub(crate) fn send(&mut self, msg: ClientMsg, now: Instant) {
        if !self.conn.as_ref().is_some_and(|c| c.welcomed) {
            return;
        }
        if !self.write(&msg) {
            self.end(Outcome::Retry("the connection dropped".into()), now);
        }
    }

    /// Leave the session.
    pub(crate) fn leave(&mut self, now: Instant) {
        self.end(Outcome::Left, now);
    }

    /// When [`tick`](Self::tick) should run next.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        if self.done {
            return None;
        }
        let l = &self.cfg.limits;
        let conn = self.conn.as_ref().map(|c| match (c.opened, self.asking.is_some()) {
            (false, _) => [Some(c.started + self.cfg.connect_timeout), None, None],
            // A managing connection waits for each answer.
            (true, true) => [Some(c.last_heard + l.request_reply_timeout), None, None],
            (true, false) => {
                [Some(c.last_ping + l.keepalive), Some(c.last_heard + l.idle_timeout), if c.welcomed { self.throttle.due() } else { None }]
            }
        });
        earliest(conn.into_iter().flatten().chain([self.reconnect_at]))
    }

    /// Do what's due at `now`: reconnect, send held presence or a ping, give up on a connection
    /// that doesn't answer.
    pub(crate) fn tick(&mut self, now: Instant) {
        if self.done {
            return;
        }
        if self.reconnect_at.is_some_and(|t| t <= now) {
            return self.connect(now);
        }
        let l = self.cfg.limits;
        let Some(c) = self.conn.as_ref() else { return };
        let (opened, started, last_heard, last_ping) = (c.opened, c.started, c.last_heard, c.last_ping);
        if !opened {
            if now >= started + self.cfg.connect_timeout {
                self.dropped(format!("no answer from {}", self.server().display()), now);
            }
            return;
        }
        if self.asking.is_some() {
            if now >= last_heard + l.request_reply_timeout {
                self.answered(Err("no answer from the server".into()), now);
            }
            return;
        }
        if now >= last_heard + l.idle_timeout {
            return self.end(Outcome::Retry("the host stopped answering".into()), now);
        }
        let ok = self.throttle.take_due(now).is_none_or(|p| self.write(&ClientMsg::Presence(p)));
        let ok = ok && (now < last_ping + l.keepalive || self.ping(now));
        if !ok {
            self.end(Outcome::Retry("the connection dropped".into()), now);
        }
    }

    /// Send a ping; false when it couldn't go.
    fn ping(&mut self, now: Instant) -> bool {
        if let Some(c) = self.conn.as_mut() {
            c.last_ping = now;
        }
        self.write(&ClientMsg::Ping)
    }

    /// Send one message; false when it couldn't go.
    fn write(&mut self, msg: &ClientMsg) -> bool {
        message::encode(msg).is_ok_and(|b| self.sock.send(&b))
    }

    /// The connection ended: close it, then stop or plan the reconnect.
    fn end(&mut self, outcome: Outcome, now: Instant) {
        // Left while asking a server: what it was asked stays done (a file uploaded stays there),
        // nothing follows.
        self.asking = None;
        // Still asking about the join code: whatever went wrong is final.
        let outcome = if self.cfg.code.is_some() { unresolved(&self.cfg, outcome) } else { outcome };
        if self.conn.take().is_some() {
            self.sock.close();
        }
        self.throttle = Throttle::new(PRESENCE_INTERVAL);
        match after(outcome, &self.cfg, self.was_live, &mut self.attempt, &self.inbox) {
            Next::Stop(e) => {
                if let Some(e) = e {
                    self.inbox.push(e);
                }
                self.done = true;
            }
            Next::Reconnect(pause) => self.reconnect_at = Some(now + pause),
        }
    }
}
