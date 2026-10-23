//! The browser client's connection logic, free of the browser so it runs (and is tested) on any
//! target: a [`Driver`] reacts to socket events and to the clock the way the native guest's
//! threads do — hello on open (resuming as the same client after a drop), presence held until the
//! welcome and changes made for an earlier connection dropped (the sync client sends again what
//! the server didn't get), presence throttled to about 30 a second (latest wins), a ping when quiet, a connection that goes silent counted as
//! dropped, and reconnects with growing pauses after a live connection drops. The browser layer
//! (`web.rs`) feeds it the WebSocket's events and wakes it at [`Driver::deadline`].
//!
//! A join by code ([`GuestConfig::code`]) first asks the server which file the code opens on a
//! connection of its own, then joins that file as with a link; a server that doesn't know the
//! code (or can't be reached) ends the join with the reason.

use figcraft_collab::message::{self, ClientMsg, PROTOCOL, Presence, Resolve, ServerMsg};
use figcraft_engine::live::{Event, Status};
// `std::time::Instant` on native builds; the browser's clock in the web build.
use web_time::Instant;

use crate::inbox::Inbox;
use crate::join::{GuestConfig, Next, Outcome, Resuming, Seen, Taken, after, closed, hello, resolved, take, too_large, unresolved};
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
    fn chunked(&self) -> bool {
        false
    }
    fn fallback_legacy(&mut self) -> bool {
        false
    }
}

/// One connection attempt.
struct Conn {
    started: Instant,
    opened: bool,
    welcomed: bool,
    seen: Seen,
    last_sent: Instant,
    last_heard: Instant,
    transfer: crate::transfer::Receiver,
}

impl Conn {
    fn new(now: Instant) -> Self {
        Self { started: now, opened: false, welcomed: false, seen: Seen::default(), last_sent: now, last_heard: now, transfer: Default::default() }
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
    done: bool,
}

impl<S: Socket> Driver<S> {
    /// Start joining (progress arrives in `inbox`).
    pub(crate) fn start(sock: S, cfg: GuestConfig, inbox: Inbox, now: Instant) -> Self {
        let mut d = Self {
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
            done: false,
        };
        d.inbox.push(Event::Status(Status::Connecting));
        d.connect(now);
        d
    }

    /// The session is over (left, refused, ended or given up): nothing more will happen.
    pub(crate) fn done(&self) -> bool {
        self.done
    }

    fn connect(&mut self, now: Instant) {
        self.reconnect_at = None;
        self.conn = Some(Conn::new(now));
        if let Err(e) = self.sock.open(&self.cfg.invite.endpoint()) {
            self.end(Outcome::Retry(e), now);
        }
    }

    /// The connection is open: say hello (or ask about the join code first).
    pub(crate) fn opened(&mut self, now: Instant) {
        let Some(c) = self.conn.as_mut() else { return };
        c.opened = true;
        c.last_heard = now;
        let hello = match &self.cfg.code {
            Some(code) => ClientMsg::Resolve(Resolve { protocol: PROTOCOL, client_version: message::client_version(), code: code.clone() }),
            None => hello(&self.cfg, &self.resume),
        };
        if !self.write(&hello, now) {
            self.end(Outcome::Retry("the connection dropped".into()), now);
        }
    }

    /// A binary message from the host.
    pub(crate) fn message(&mut self, bytes: &[u8], now: Instant) {
        let Some(c) = self.conn.as_mut() else { return };
        if bytes.len() > self.cfg.limits.max_server_message.saturating_add(crate::transfer::HEADER) {
            return self.end(too_large(&self.cfg.limits), now);
        }
        c.last_heard = now;
        let bytes = match c.transfer.push(bytes, self.sock.chunked(), self.cfg.limits.max_server_message) {
            Ok(Some(b)) => b,
            Ok(None) => return,
            Err(crate::transfer::Error::TooLarge) => return self.end(too_large(&self.cfg.limits), now),
            Err(crate::transfer::Error::Invalid(reason)) => return self.end(Outcome::Refused(reason), now),
        };
        let taken = match message::decode::<ServerMsg>(&bytes, self.cfg.limits.max_server_message) {
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
            Ok(m) => match crate::assets_wire::validate_legacy(&m) {
                Ok(()) => take(m, &self.cfg, &self.inbox, &mut c.seen, &self.resume, &|| {}),
                Err(reason) => Taken::End(Outcome::Refused(reason)),
            },
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
        self.end(Outcome::Refused("the host sent a text message".into()), now);
    }

    /// The connection closed (`code` and `reason` from the close frame; 1006 when it just broke).
    pub(crate) fn closed(&mut self, code: u16, reason: &str, now: Instant) {
        let Some(opened) = self.conn.as_ref().map(|c| c.opened) else { return };
        if !opened && code == 1006 && self.sock.fallback_legacy() {
            self.sock.close();
            self.connect(now);
            return;
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
            && !self.write(&ClientMsg::Presence(p), now)
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
        if !self.write(&msg, now) {
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
        let conn = self.conn.as_ref().map(|c| {
            if c.opened {
                [Some(c.last_sent + l.keepalive), Some(c.last_heard + l.idle_timeout), if c.welcomed { self.throttle.due() } else { None }]
            } else {
                [Some(c.started + self.cfg.connect_timeout), None, None]
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
        let (opened, started, last_heard, last_sent) = (c.opened, c.started, c.last_heard, c.last_sent);
        if !opened {
            if now >= started + self.cfg.connect_timeout {
                self.end(Outcome::Retry(format!("no answer from {}", self.cfg.invite.server.display())), now);
            }
            return;
        }
        if now >= last_heard + l.idle_timeout {
            return self.end(Outcome::Retry("the host stopped answering".into()), now);
        }
        let ok = match self.throttle.take_due(now) {
            Some(p) => self.write(&ClientMsg::Presence(p), now),
            None if now >= last_sent + l.keepalive => self.write(&ClientMsg::Ping, now),
            None => true,
        };
        if !ok {
            self.end(Outcome::Retry("the connection dropped".into()), now);
        }
    }

    /// Send one message; false when it couldn't go.
    fn write(&mut self, msg: &ClientMsg, now: Instant) -> bool {
        if let Some(c) = self.conn.as_mut() {
            c.last_sent = now;
        }
        crate::assets_wire::client(msg, false).is_ok_and(|b| crate::transfer::packets(&b, self.sock.chunked()).all(|p| self.sock.send(&p)))
    }

    /// The connection ended: close it, then stop or plan the reconnect.
    fn end(&mut self, outcome: Outcome, now: Instant) {
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
