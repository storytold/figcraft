//! The browser client's connection logic ([`Driver`]) on native: against a scripted socket with a
//! made-up clock (hello, holding presence until the welcome, throttling, pings, a silent host,
//! reconnecting as the same client, refusals, giving up), and against a real host over loopback through
//! a small blocking socket, which checks that what the browser sends is what the host speaks.

use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use figcraft_collab::message::{self, ClientMsg, Presence, ServerMsg};
use figcraft_collab::{Client, Limits, Outgoing, Server};
use figcraft_doc::{Document, NodeId};
use figcraft_engine::live::{Event, Status, TOO_LARGE_TO_JOIN};
use tungstenite::protocol::WebSocket;
use tungstenite::{Bytes, Message};

use crate::driver::{Driver, Socket};
use crate::guest::GuestConfig;
use crate::host::{ENDED, Host, HostConfig};
use crate::inbox::Inbox;
use crate::invite::Invite;
use crate::loopback_tests::fast_limits;

const TOKEN: &str = "AbCdEfGhIjKlMnOpQrStUv";
const FILE: &str = "0123456789abcdef";

/// A socket that records what the driver does.
#[derive(Default)]
struct Script {
    opened: Vec<String>,
    sent: Vec<ClientMsg>,
    closes: usize,
    /// `open` fails (the browser refused the URL).
    refuse: bool,
    /// `send` fails (the connection broke).
    broken: bool,
}

impl Socket for Script {
    fn open(&mut self, url: &str) -> Result<(), String> {
        if self.refuse {
            return Err("SecurityError".into());
        }
        self.opened.push(url.to_string());
        Ok(())
    }
    fn send(&mut self, bytes: &[u8]) -> bool {
        if self.broken {
            return false;
        }
        self.sent.push(message::decode(bytes, usize::MAX).unwrap());
        true
    }
    fn close(&mut self) {
        self.closes += 1;
    }
}

fn invite() -> Invite {
    Invite::new("127.0.0.1:47900", FILE, TOKEN)
}

fn start(sock: Script, t0: Instant) -> (Driver<Script>, Inbox) {
    let inbox = Inbox::new(Arc::new(|| {}));
    (Driver::start(sock, GuestConfig::new(invite(), "Ada".into()), inbox.clone(), t0), inbox)
}

fn shared() -> Document {
    let mut doc = Document::new("Shared file");
    doc.repair();
    doc
}

/// The welcome a session server sends Ada, encoded.
fn welcome() -> Vec<u8> {
    let mut server = Server::new(shared(), TOKEN, Limits::default());
    let ClientMsg::Hello(h) = Client::hello(FILE, TOKEN, "Ada", "") else { panic!("no hello") };
    let (_, outs) = server.join(h).unwrap();
    let w = outs.into_iter().find_map(|o| if let Outgoing::To(_, w @ ServerMsg::Welcome { .. }) = o { Some(w) } else { None }).unwrap();
    message::encode(&w).unwrap()
}

fn encode(m: &ServerMsg) -> Vec<u8> {
    message::encode(m).unwrap()
}

fn statuses(events: &[Event]) -> Vec<Status> {
    events.iter().filter_map(|e| if let Event::Status(s) = e { Some(s.clone()) } else { None }).collect()
}

fn cursor(x: f64) -> Presence {
    Presence { cursor: Some((NodeId(1), x, 0.0)), ..Presence::default() }
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn hello_then_the_welcome_releases_presence() {
    let t0 = Instant::now();
    let (mut d, inbox) = start(Script::default(), t0);
    assert_eq!(d.sock.opened, vec!["ws://127.0.0.1:47900/"], "the token isn't in the URL");
    assert_eq!(statuses(&inbox.drain()), vec![Status::Connecting]);
    // Nothing is due but giving up on a connection that never opens.
    assert_eq!(d.deadline(), Some(t0 + Duration::from_secs(5)));
    d.opened(t0 + ms(10));
    let [ClientMsg::Hello(h)] = d.sock.sent.as_slice() else { panic!("{:?}", d.sock.sent) };
    assert_eq!((h.file.as_str(), h.token.as_str(), h.name.as_str()), (FILE, TOKEN, "Ada"));
    assert_eq!(h.resume, None, "a new participant");
    // Before the welcome: presence is held (latest wins); changes were made for no connection the
    // server knows, so they're dropped (the sync client sends again what the server didn't get).
    d.presence(cursor(1.0), t0 + ms(20));
    d.presence(cursor(2.0), t0 + ms(21));
    d.send(ClientMsg::Ack { seq: 1 }, t0 + ms(22));
    d.send(ClientMsg::Ack { seq: 2 }, t0 + ms(23));
    assert_eq!(d.sock.sent.len(), 1);
    d.message(&welcome(), t0 + ms(30));
    assert_eq!(d.sock.sent[1..], [ClientMsg::Presence(cursor(2.0))], "the latest presence; no stale changes");
    d.send(ClientMsg::Ack { seq: 3 }, t0 + ms(31));
    assert_eq!(d.sock.sent.last(), Some(&ClientMsg::Ack { seq: 3 }), "welcomed: changes go out");
    let events = inbox.drain();
    assert!(
        matches!(events.as_slice(), [Event::Welcome { info, peers }, Event::Sync(ServerMsg::Welcome { .. })] if !info.network && peers.is_empty())
    );
    assert!(!d.done());
}

#[test]
fn presence_is_throttled_and_quiet_connections_ping() {
    let t0 = Instant::now();
    let (mut d, _inbox) = start(Script::default(), t0);
    d.opened(t0);
    d.message(&welcome(), t0);
    d.presence(cursor(1.0), t0);
    d.presence(cursor(2.0), t0 + ms(5));
    d.presence(cursor(3.0), t0 + ms(10));
    assert_eq!(d.sock.sent.last(), Some(&ClientMsg::Presence(cursor(1.0))));
    // The latest waits for the interval, and the driver asks to be woken for it.
    let due = d.deadline().unwrap();
    assert_eq!(due, t0 + crate::throttle::PRESENCE_INTERVAL);
    d.tick(due);
    assert_eq!(d.sock.sent.last(), Some(&ClientMsg::Presence(cursor(3.0))));
    let n = d.sock.sent.len();
    // A ping every keep-alive interval from the connection's start, presence or not (the host
    // answers, which counts as traffic).
    let ping_at = d.deadline().unwrap();
    assert_eq!(ping_at, t0 + Duration::from_secs(15));
    d.tick(ping_at - ms(1));
    assert_eq!(d.sock.sent.len(), n, "not yet");
    d.message(&encode(&ServerMsg::Pong), ping_at - ms(1));
    d.tick(ping_at);
    assert_eq!(d.sock.sent.last(), Some(&ClientMsg::Ping));
}

/// A viewer moving the pointer sends presence all the time, which the host doesn't answer: the
/// pings still go out, and the host's pongs keep the connection (before, each presence put the
/// next ping off, nothing was heard for the idle timeout, and the client dropped the connection).
#[test]
fn pings_go_out_while_presence_does_and_the_pongs_keep_the_connection() {
    let t0 = Instant::now();
    let (mut d, inbox) = start(Script::default(), t0);
    d.opened(t0);
    d.message(&welcome(), t0);
    inbox.drain();
    let mut answered = 0;
    for i in 1..=900u32 {
        let now = t0 + ms(100 * u64::from(i));
        d.presence(cursor(f64::from(i)), now);
        while let Some(due) = d.deadline().filter(|t| *t <= now) {
            d.tick(due);
        }
        // The host answers every ping (and nothing else) at once.
        let pinged = d.sock.sent[answered..].contains(&ClientMsg::Ping);
        answered = d.sock.sent.len();
        if pinged {
            d.message(&encode(&ServerMsg::Pong), now);
        }
    }
    assert!(!d.done());
    assert_eq!(d.sock.closes, 0, "90 s of presence and pongs: still connected");
    assert!(statuses(&inbox.drain()).is_empty());
    let pings = d.sock.sent.iter().filter(|m| **m == ClientMsg::Ping).count();
    assert!(pings >= 5, "a ping every 15 s ({pings})");
}

#[test]
fn a_welcome_still_downloading_gets_the_long_reply_timeout() {
    let t0 = Instant::now();
    let (mut d, inbox) = start(Script::default(), t0);
    d.opened(t0);
    let limits = GuestConfig::new(invite(), "Ada".into()).limits;
    // The welcome (the whole document) is one message that is not heard until complete: past the
    // idle timeout the join must not be given up.
    let past_idle = t0 + limits.idle_timeout + Duration::from_secs(1);
    d.tick(past_idle);
    assert!(!d.done());
    assert_eq!(d.sock.closes, 0);
    assert_eq!(statuses(&inbox.drain()), vec![Status::Connecting]);
    // It does arrive; from then on a silent host is dropped at the idle timeout.
    d.message(&welcome(), past_idle);
    let silent = past_idle + limits.idle_timeout;
    d.tick(silent - ms(1));
    assert_eq!(d.sock.closes, 0);
    d.tick(silent);
    assert_eq!(d.sock.closes, 1);
}

#[test]
fn a_host_that_never_welcomes_is_given_up_on_at_the_reply_timeout() {
    let t0 = Instant::now();
    let (mut d, _inbox) = start(Script::default(), t0);
    d.opened(t0);
    let limit = t0 + GuestConfig::new(invite(), "Ada".into()).limits.request_reply_timeout;
    d.tick(limit);
    assert_eq!(d.sock.closes, 1);
}

#[test]
fn a_silent_host_counts_as_dropped_and_the_client_reconnects() {
    let t0 = Instant::now();
    let (mut d, inbox) = start(Script::default(), t0);
    d.opened(t0);
    d.message(&welcome(), t0);
    let first_id = inbox.drain().iter().find_map(|e| if let Event::Welcome { info, .. } = e { Some(info.me) } else { None }).unwrap();
    // Pings go out, nothing comes back for 45 s.
    for s in [15, 30] {
        d.tick(t0 + Duration::from_secs(s));
    }
    assert!(!d.done());
    let silent = t0 + Duration::from_secs(45);
    assert_eq!(d.deadline(), Some(silent));
    d.tick(silent);
    assert_eq!(d.sock.closes, 1);
    assert_eq!(statuses(&inbox.drain()), vec![Status::Reconnecting { attempt: 1 }]);
    // Changes made while disconnected are dropped (the next welcome starts over); presence waits.
    d.send(ClientMsg::Ack { seq: 9 }, silent);
    d.presence(cursor(7.0), silent);
    let again = d.deadline().unwrap();
    assert_eq!(again, silent + ms(500), "the first pause");
    d.tick(again);
    assert_eq!(d.sock.opened.len(), 2);
    d.opened(again);
    // Back as the same client, from the last sequence number it got.
    let Some(ClientMsg::Hello(h)) = d.sock.sent.last() else { panic!("no hello") };
    let r = h.resume.expect("resumes");
    assert_eq!((r.client_id, r.seq), (first_id, 0));
    d.message(&welcome(), again);
    assert_eq!(d.sock.sent.iter().filter(|m| matches!(m, ClientMsg::Ack { seq: 9 })).count(), 0);
    assert_eq!(d.sock.sent.last(), Some(&ClientMsg::Presence(cursor(7.0))), "your presence once you're back");
    assert!(inbox.drain().iter().any(|e| matches!(e, Event::Welcome { .. })));
}

#[test]
fn reconnects_back_off_then_give_up() {
    let t0 = Instant::now();
    let (mut d, inbox) = start(Script::default(), t0);
    d.opened(t0);
    d.message(&welcome(), t0);
    let mut now = t0;
    let mut pauses = Vec::new();
    d.closed(1006, "", now);
    while !d.done() {
        let next = d.deadline().unwrap();
        pauses.push(next - now);
        now = next;
        d.tick(now);
        // The host is gone: every attempt fails before it opens.
        d.closed(1006, "", now);
    }
    let secs: Vec<f64> = pauses.iter().map(Duration::as_secs_f64).collect();
    assert_eq!(secs, [0.5, 1.0, 2.0, 4.0, 8.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0]);
    let st = statuses(&inbox.drain());
    assert_eq!(st.len(), 14, "connecting, 12 reconnects, then failed");
    assert!(matches!(st.last(), Some(Status::Failed { reason }) if reason.starts_with("Lost the connection to the host")), "{st:?}");
    assert_eq!(d.deadline(), None);
}

#[test]
fn a_first_join_that_cant_reach_the_host_fails_at_once() {
    let t0 = Instant::now();
    // The browser gives up on the socket before it opens.
    let (mut d, inbox) = start(Script::default(), t0);
    d.closed(1006, "", t0 + ms(30));
    assert!(d.done());
    let st = statuses(&inbox.drain());
    assert!(
        matches!(st.as_slice(), [Status::Connecting, Status::Failed { reason }] if reason == "Couldn't join the live session: no answer from 127.0.0.1:47900"),
        "{st:?}"
    );
    // No answer within the connect timeout.
    let (mut d, inbox) = start(Script::default(), t0);
    d.tick(t0 + Duration::from_secs(5));
    assert!(d.done());
    assert!(matches!(statuses(&inbox.drain()).last(), Some(Status::Failed { reason }) if reason.contains("no answer")));
    // The browser refuses to open the URL at all.
    let (d, inbox) = start(Script { refuse: true, ..Script::default() }, t0);
    assert!(d.done());
    assert!(matches!(statuses(&inbox.drain()).last(), Some(Status::Failed { reason }) if reason.contains("SecurityError")));
}

#[test]
fn refusals_and_the_end_of_the_session_are_final() {
    let t0 = Instant::now();
    let (mut d, inbox) = start(Script::default(), t0);
    d.opened(t0);
    d.message(&encode(&ServerMsg::Refused { reason: "wrong token".into() }), t0);
    assert!(d.done() && d.sock.closes == 1);
    assert!(matches!(statuses(&inbox.drain()).last(), Some(Status::Failed { reason }) if reason == "The host refused: wrong token"));
    // A close frame saying the host went away: the session ended.
    let (mut d, inbox) = start(Script::default(), t0);
    d.opened(t0);
    d.message(&welcome(), t0);
    d.closed(1001, "", t0);
    assert!(d.done());
    assert!(inbox.drain().iter().any(|e| matches!(e, Event::Ended { reason } if reason == ENDED)));
    // Garbage and text frames are refusals too.
    for text in [false, true] {
        let (mut d, inbox) = start(Script::default(), t0);
        d.opened(t0);
        if text {
            d.text(t0);
        } else {
            d.message(&[0xFF, 1, 2, 3], t0);
        }
        assert!(d.done());
        assert!(matches!(statuses(&inbox.drain()).last(), Some(Status::Failed { reason }) if reason.starts_with("The host refused")));
    }
    // A message over the size limit: the file is too large for this app, not refused by the host.
    let inbox = Inbox::new(Arc::new(|| {}));
    let mut cfg = GuestConfig::new(invite(), "Ada".into());
    cfg.limits.max_server_message = 2 << 20;
    let mut d = Driver::start(Script::default(), cfg, inbox.clone(), t0);
    d.opened(t0);
    d.message(&vec![0; (2 << 20) + 1], t0);
    assert!(d.done());
    assert!(matches!(statuses(&inbox.drain()).last(), Some(Status::Failed { reason }) if *reason == TOO_LARGE_TO_JOIN.replace("{mb}", "2")));
}

#[test]
fn leaving_closes_and_reports_nothing() {
    let t0 = Instant::now();
    let (mut d, inbox) = start(Script::default(), t0);
    d.opened(t0);
    d.message(&welcome(), t0);
    inbox.drain();
    d.leave(t0);
    assert!(d.done());
    assert_eq!(d.sock.closes, 1);
    assert_eq!(d.deadline(), None);
    // Late events from the closed socket change nothing.
    d.closed(1000, "", t0);
    d.message(&welcome(), t0);
    d.text(t0);
    d.tick(t0 + Duration::from_secs(60));
    assert!(inbox.drain().is_empty());
}

#[test]
fn a_broken_send_reconnects() {
    let t0 = Instant::now();
    let (mut d, inbox) = start(Script::default(), t0);
    d.opened(t0);
    d.message(&welcome(), t0);
    inbox.drain();
    d.sock.broken = true;
    d.send(ClientMsg::Ping, t0);
    assert_eq!(statuses(&inbox.drain()), vec![Status::Reconnecting { attempt: 1 }]);
}

/// A blocking tungstenite socket standing in for the browser's: events are read by [`pump`].
#[derive(Default)]
struct Loopback {
    ws: Option<WebSocket<TcpStream>>,
    just_opened: bool,
}

impl Socket for Loopback {
    fn open(&mut self, url: &str) -> Result<(), String> {
        let host = url.trim_start_matches("ws://").trim_end_matches('/');
        let stream = TcpStream::connect(host).map_err(|e| e.to_string())?;
        let (ws, _) = tungstenite::client(url, stream).map_err(|e| e.to_string())?;
        ws.get_ref().set_read_timeout(Some(ms(5))).map_err(|e| e.to_string())?;
        self.ws = Some(ws);
        self.just_opened = true;
        Ok(())
    }
    fn send(&mut self, bytes: &[u8]) -> bool {
        self.ws.as_mut().is_some_and(|ws| ws.send(Message::Binary(Bytes::copy_from_slice(bytes))).is_ok())
    }
    fn close(&mut self) {
        if let Some(mut ws) = self.ws.take() {
            let _ = ws.close(None);
            let _ = ws.flush();
        }
    }
}

/// Feed the driver what arrived, as the browser's event loop would.
fn pump(d: &mut Driver<Loopback>) {
    let now = Instant::now();
    if std::mem::take(&mut d.sock.just_opened) {
        d.opened(now);
    }
    while let Some(ws) = d.sock.ws.as_mut() {
        match ws.read() {
            Ok(Message::Binary(b)) => d.message(&b, now),
            Ok(Message::Close(f)) => {
                let (code, reason) = f.map_or((1005, String::new()), |f| (f.code.into(), f.reason.to_string()));
                d.closed(code, &reason, now);
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => break,
            Err(_) => d.closed(1006, "", now),
        }
    }
    if d.deadline().is_some_and(|t| t <= now) {
        d.tick(now);
    }
}

#[test]
fn joins_a_real_host_and_edits_with_it() {
    let host_probe = Inbox::new(Arc::new(|| {}));
    let mut host = Host::start(
        HostConfig {
            addr: ([127, 0, 0, 1], 0).into(),
            any_port_if_taken: false,
            file: FILE.into(),
            token: TOKEN.into(),
            doc: shared(),
            name: "Hana".into(),
            limits: fast_limits(),
            core: Limits::default(),
        },
        host_probe.clone(),
    )
    .unwrap();
    let invite = Invite::new(host.addr().to_string(), FILE, TOKEN);
    let inbox = Inbox::new(Arc::new(|| {}));
    let mut d = Driver::start(Loopback::default(), GuestConfig::new(invite, "Ada".into()), inbox.clone(), Instant::now());
    let mut events = Vec::new();
    let wait = |d: &mut Driver<Loopback>, events: &mut Vec<Event>, what: &str, f: &dyn Fn(&[Event]) -> bool| {
        let start = Instant::now();
        while !f(events) {
            assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for {what}: {events:#?}");
            pump(d);
            events.extend(inbox.drain());
        }
    };
    wait(&mut d, &mut events, "the welcome", &|e| e.iter().any(|e| matches!(e, Event::Sync(ServerMsg::Welcome { .. }))));
    let Some(Event::Sync(w)) = events.iter().find(|e| matches!(e, Event::Sync(ServerMsg::Welcome { .. }))).cloned() else { panic!("no welcome") };
    let mut client = Client::join(w).unwrap();
    // Ada adds a page; the host's server accepts it and confirms it to her.
    let mut doc = client.document().clone();
    let page = doc.add_page("From the browser");
    let batch = client.edit(&doc).unwrap().unwrap();
    d.send(batch, Instant::now());
    wait(&mut d, &mut events, "the confirmation", &|e| e.iter().any(|e| matches!(e, Event::Sync(ServerMsg::Changes { .. }))));
    // The host's own window (a client of its server) gets the page too.
    let host_events: Vec<Event> = {
        let start = Instant::now();
        let mut got = Vec::new();
        while !got.iter().any(|e| matches!(e, Event::Sync(ServerMsg::Changes { .. }))) {
            assert!(start.elapsed() < Duration::from_secs(10), "the host never got the change: {got:#?}");
            pump(&mut d);
            got.extend(host_probe.drain());
        }
        got
    };
    assert!(host_events.iter().any(|e| matches!(e, Event::PeerJoined(p) if p.name == "Ada")));
    // Presence reaches the host.
    d.presence(cursor(42.0), Instant::now());
    let start = Instant::now();
    loop {
        pump(&mut d);
        if host_probe.drain().iter().any(|e| matches!(e, Event::Presence(_, p) if *p == cursor(42.0))) {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "presence never arrived");
    }
    // The host stops: the browser hears the session ended.
    host.stop();
    wait(&mut d, &mut events, "the end", &|e| e.iter().any(|e| matches!(e, Event::Ended { .. })));
    assert!(d.done());
    assert!(doc.contains(page));
}
