//! The transport and the host end to end over loopback (127.0.0.1, ephemeral ports): joining and
//! refusals, presence relay and throttling, leaving and dropping, limits, malformed frames, and
//! reconnecting. Every test stops its host and closes its sockets.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use figcraft_collab::message::{self, CatchUp, ClientMsg, Hello, Presence, ServerMsg, Signal};
use figcraft_collab::{Client, Limits};
use figcraft_doc::{Document, NodeId};
use figcraft_engine::live::{Event, PEOPLE, Status, TOO_LARGE_TO_JOIN};
use tungstenite::protocol::WebSocket;
use tungstenite::{Bytes, Message};

use crate::guest::{Guest, GuestConfig};
use crate::host::{ENDED, Host, HostConfig, REMOVED};
use crate::inbox::Inbox;
use crate::invite::Invite;
use crate::net::{Bucket, NetLimits};
use crate::throttle::Throttle;

const TOKEN: &str = "AbCdEfGhIjKlMnOpQrStUv";
const FILE: &str = "0123456789abcdef";
const WAIT: Duration = Duration::from_secs(10);

/// One participant's view: the events its inbox got so far.
pub(crate) struct Probe {
    pub inbox: Inbox,
    pub events: Vec<Event>,
}

impl Probe {
    pub fn new() -> Self {
        Self { inbox: Inbox::new(Arc::new(|| {})), events: Vec::new() }
    }

    /// Wait until `f` holds for the events so far.
    pub fn wait(&mut self, what: &str, f: impl Fn(&[Event]) -> bool) {
        let start = Instant::now();
        loop {
            self.events.extend(self.inbox.drain());
            if f(&self.events) {
                return;
            }
            assert!(start.elapsed() < WAIT, "timed out waiting for {what}: {:#?}", self.events);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The client id this guest was welcomed with (latest welcome).
    pub fn me(&self) -> u32 {
        self.events.iter().rev().find_map(|e| if let Event::Welcome { info, .. } = e { Some(info.me) } else { None }).unwrap()
    }

    pub fn presence_of(&self, id: u32) -> Option<&Presence> {
        self.events.iter().rev().find_map(|e| match e {
            Event::Presence(from, p) if *from == id => Some(p),
            _ => None,
        })
    }

    fn joined(&self) -> Vec<u32> {
        self.events.iter().filter_map(|e| if let Event::PeerJoined(p) = e { Some(p.client_id) } else { None }).collect()
    }

    fn left(&self) -> Vec<u32> {
        self.events.iter().filter_map(|e| if let Event::PeerLeft(id) = e { Some(*id) } else { None }).collect()
    }
}

fn welcomed(e: &[Event]) -> bool {
    e.iter().any(|e| matches!(e, Event::Welcome { .. }))
}

pub(crate) fn fast_limits() -> NetLimits {
    NetLimits { hello_timeout: Duration::from_secs(5), ..NetLimits::default() }
}

fn start_with(limits: NetLimits, core: Limits) -> (Host, Probe, Invite) {
    let probe = Probe::new();
    let mut doc = Document::new("Shared file");
    doc.repair();
    let cfg = HostConfig {
        addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        any_port_if_taken: false,
        file: FILE.into(),
        token: TOKEN.into(),
        doc,
        name: "Hana".into(),
        limits,
        core,
    };
    let host = Host::start(cfg, probe.inbox.clone()).unwrap();
    let invite = Invite::new(host.addr().to_string(), FILE, TOKEN);
    (host, probe, invite)
}

pub(crate) fn start() -> (Host, Probe, Invite) {
    start_with(fast_limits(), Limits::default())
}

pub(crate) fn quick(mut c: GuestConfig) -> GuestConfig {
    c.backoff = Duration::from_millis(20);
    c.max_backoff = Duration::from_millis(100);
    c.limits = fast_limits();
    c
}

fn join(invite: &Invite, name: &str) -> (Guest, Probe) {
    let probe = Probe::new();
    let g = Guest::start(quick(GuestConfig::new(invite.clone(), name.into())), probe.inbox.clone()).unwrap();
    (g, probe)
}

fn presence(x: f64, y: f64) -> Presence {
    Presence {
        cursor: Some((NodeId(1), x, y)),
        selection: Some(vec![NodeId(7)]),
        viewport: Some((NodeId(1), 0.0, 0.0, 800.0, 600.0)),
        editing_text: None,
    }
}

/// A bare WebSocket client (sends what it's told, no throttling).
struct Raw(WebSocket<TcpStream>);

impl Raw {
    fn connect(addr: SocketAddr) -> Raw {
        let s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(WAIT)).unwrap();
        let (ws, _) = tungstenite::client::client(format!("ws://{addr}/"), s).unwrap();
        Raw(ws)
    }

    fn send_bytes(&mut self, b: Vec<u8>) -> bool {
        self.0.send(Message::Binary(Bytes::from(b))).is_ok()
    }

    fn send(&mut self, m: &ClientMsg) -> bool {
        self.send_bytes(message::encode(m).unwrap())
    }

    fn hello(addr: SocketAddr, token: &str) -> Raw {
        let mut r = Raw::connect(addr);
        assert!(r.send(&Client::hello(FILE, token, "Raw", "")));
        r
    }

    /// The next server message; `None` once the connection closed.
    fn next(&mut self) -> Option<ServerMsg> {
        loop {
            match self.0.read() {
                Ok(Message::Binary(b)) => return Some(message::decode(&b, 64 << 20).unwrap()),
                Ok(Message::Close(_)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
    }

    /// Read until the connection ends; the refusal it got, if any.
    fn refusal(&mut self) -> Option<String> {
        let mut why = None;
        while let Some(m) = self.next() {
            if let ServerMsg::Refused { reason } = m {
                why = Some(reason);
            }
        }
        why
    }
}

fn connections_settle(host: &Host, n: usize) {
    let start = Instant::now();
    while host.connections() != n {
        assert!(start.elapsed() < WAIT, "{} connections, expected {n}", host.connections());
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn guests_join_and_see_each_other() {
    let (mut host, mut hp, invite) = start();
    let (_a, mut ap) = join(&invite, "Ada");
    ap.wait("Ada's welcome and colour", |e| e.iter().filter(|e| matches!(e, Event::Welcome { .. })).count() >= 2);
    let Some(Event::Welcome { info, peers }) = ap.events.iter().rev().find(|e| matches!(e, Event::Welcome { .. })) else { panic!("no welcome") };
    assert_eq!(info.name, "Ada");
    assert!(
        ap.events.iter().any(
            |e| matches!(e, Event::Sync(ServerMsg::Welcome { catch_up: figcraft_collab::CatchUp::Snapshot(snapshot), .. }) if !snapshot.is_empty())
        ),
        "the welcome carries the document for the sync client"
    );
    assert_eq!(peers.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Hana"], "the host is already here");
    assert_eq!(peers[0].color, PEOPLE[0]);
    assert_eq!(info.color, PEOPLE[1], "the host hands out the next free colour");
    let ada = info.me;
    assert_ne!(ada, host.me().client_id);
    hp.wait("the host sees Ada", |_| true);
    hp.wait("the host sees Ada", |e| e.iter().any(|e| matches!(e, Event::PeerJoined(p) if p.client_id == ada && p.name == "Ada")));

    // A guest without a name gets one.
    let (_b, mut bp) = join(&invite, "");
    bp.wait("the second guest's colour", |e| e.iter().filter(|e| matches!(e, Event::Welcome { .. })).count() >= 2);
    let Some(Event::Welcome { info, peers }) = bp.events.iter().rev().find(|e| matches!(e, Event::Welcome { .. })) else { panic!("no welcome") };
    assert_eq!(info.name, "Guest 1");
    assert_eq!(info.color, PEOPLE[2]);
    assert_eq!(peers.len(), 2);
    let b = info.me;
    ap.wait("Ada sees the newcomer", |e| e.iter().any(|e| matches!(e, Event::PeerJoined(p) if p.client_id == b)));
    assert!(!ap.joined().contains(&ada), "nobody is told about themselves as a peer");
    host.stop();
    assert_eq!(host.connections(), 0);
}

#[test]
fn a_wrong_token_or_file_is_refused() {
    let (mut host, mut hp, invite) = start();
    let (g, mut gp) = join(&Invite { token: "WrongWrongWrongWrong00".into(), ..invite.clone() }, "Eve");
    gp.wait("a refusal", |e| e.iter().any(|e| matches!(e, Event::Status(Status::Failed { reason }) if reason.contains("token"))));
    assert!(!welcomed(&gp.events));
    let (_g2, mut gp2) = join(&Invite { file: "ffffffffffffffff".into(), ..invite.clone() }, "Eve");
    gp2.wait("a refusal", |e| e.iter().any(|e| matches!(e, Event::Status(Status::Failed { reason }) if reason.contains("another file"))));
    // Nobody got in; the refused connections are closed.
    hp.events.extend(hp.inbox.drain());
    assert!(hp.joined().is_empty());
    connections_settle(&host, 0);
    let start = Instant::now();
    while !g.finished() {
        assert!(start.elapsed() < WAIT);
        std::thread::sleep(Duration::from_millis(5));
    }
    // The secret never shows in what the UI is told.
    assert!(gp.events.iter().all(|e| !format!("{e:?}").contains("WrongWrong")));
    host.stop();
}

#[test]
fn unreachable_hosts_fail_at_once() {
    // A port nothing listens on.
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let (_g, mut gp) = join(&Invite::new(free.to_string(), FILE, TOKEN), "Ada");
    gp.wait("a failure", |e| e.iter().any(|e| matches!(e, Event::Status(Status::Failed { reason }) if reason.starts_with("Couldn't join"))));
    assert!(!gp.events.iter().any(|e| matches!(e, Event::Status(Status::Reconnecting { .. }))), "a first join doesn't retry");
}

#[test]
fn presence_is_relayed_both_ways() {
    let (mut host, mut hp, invite) = start();
    let (a, mut ap) = join(&invite, "Ada");
    let (_b, mut bp) = join(&invite, "Bo");
    ap.wait("welcome", welcomed);
    bp.wait("welcome", welcomed);
    let ada = ap.me();
    hp.wait("both joined", |e| e.iter().filter(|e| matches!(e, Event::PeerJoined(_))).count() == 2);
    a.presence(presence(10.0, 20.0));
    hp.wait("Ada's cursor at the host", |e| {
        e.iter().any(|e| matches!(e, Event::Presence(id, p) if *id == ada && p.cursor == Some((NodeId(1), 10.0, 20.0))))
    });
    bp.wait("Ada's cursor at Bo", |e| e.iter().any(|e| matches!(e, Event::Presence(id, p) if *id == ada && p.selection == Some(vec![NodeId(7)]))));
    ap.events.extend(ap.inbox.drain());
    assert!(ap.presence_of(ada).is_none(), "your own presence doesn't come back");
    // The host's own presence reaches the guests.
    let hana = host.me().client_id;
    host.presence(presence(5.0, 6.0));
    ap.wait("the host's cursor", |e| e.iter().any(|e| matches!(e, Event::Presence(id, p) if *id == hana && p.cursor == Some((NodeId(1), 5.0, 6.0)))));
    // Non-finite numbers never travel.
    a.presence(Presence { cursor: Some((NodeId(1), f64::NAN, 1.0)), ..presence(0.0, 0.0) });
    bp.wait("the sanitised cursor", |e| e.iter().any(|e| matches!(e, Event::Presence(id, p) if *id == ada && p.cursor.is_none())));
    host.stop();
}

#[test]
fn a_newcomer_sees_where_everyone_already_is() {
    let (mut host, _hp, invite) = start();
    let (a, mut ap) = join(&invite, "Ada");
    ap.wait("welcome", welcomed);
    let ada = ap.me();
    a.presence(presence(42.0, 43.0));
    host.presence(presence(1.0, 2.0));
    std::thread::sleep(Duration::from_millis(150));
    let (_b, mut bp) = join(&invite, "Bo");
    let hana = host.me().client_id;
    bp.wait("everyone's last presence", |e| {
        let has = |who: u32, x: f64| e.iter().any(|e| matches!(e, Event::Presence(id, p) if *id == who && p.cursor.map(|c| c.1) == Some(x)));
        has(ada, 42.0) && has(hana, 1.0)
    });
    host.stop();
}

#[test]
fn presence_is_throttled_and_the_latest_wins() {
    let (mut host, _hp, invite) = start();
    let addr: SocketAddr = invite.server.host.parse().unwrap();
    let mut watcher = Raw::hello(addr, TOKEN);
    assert!(matches!(watcher.next(), Some(ServerMsg::Welcome { .. })));
    let mut sender = Raw::hello(addr, TOKEN);
    let Some(ServerMsg::Welcome { client_id: from, .. }) = sender.next() else { panic!("no welcome") };
    // 200 updates as fast as the socket takes them.
    let t0 = Instant::now();
    for i in 0..200 {
        assert!(sender.send(&ClientMsg::Presence(presence(f64::from(i), 0.0))));
    }
    let sent_in = t0.elapsed();
    let mut got = Vec::new();
    while got.last() != Some(&199.0) {
        match watcher.next() {
            Some(ServerMsg::Presence { client_id, presence }) if client_id == from => got.push(presence.cursor.unwrap().1),
            Some(_) => {}
            None => panic!("the watcher was disconnected"),
        }
    }
    let span = t0.elapsed().max(sent_in);
    let most = (span.as_secs_f64() / 0.033).ceil() as usize + 2;
    assert!(got.len() <= most, "{} updates relayed in {span:?} (at most {most})", got.len());
    assert!(got.len() < 200, "coalesced");
    assert_eq!(got.last(), Some(&199.0), "the last update always arrives");
    assert!(got.windows(2).all(|w| w[0] < w[1]), "in order");
    host.stop();
}

#[test]
fn leaving_and_dropping_clean_up() {
    let (mut host, mut hp, invite) = start();
    let (mut a, mut ap) = join(&invite, "Ada");
    let (_b, mut bp) = join(&invite, "Bo");
    ap.wait("welcome", welcomed);
    bp.wait("welcome", welcomed);
    let ada = ap.me();
    a.presence(presence(1.0, 1.0));
    hp.wait("Ada's cursor", |e| e.iter().any(|e| matches!(e, Event::Presence(id, _) if *id == ada)));
    connections_settle(&host, 2);
    a.leave();
    hp.wait("Ada left", |e| e.iter().any(|e| matches!(e, Event::PeerLeft(id) if *id == ada)));
    bp.wait("Bo sees Ada leave", |e| e.iter().any(|e| matches!(e, Event::PeerLeft(id) if *id == ada)));
    connections_settle(&host, 1);
    // A connection that just vanishes (no close) counts as leaving too.
    let addr: SocketAddr = invite.server.host.parse().unwrap();
    let mut raw = Raw::hello(addr, TOKEN);
    let Some(ServerMsg::Welcome { client_id: rid, .. }) = raw.next() else { panic!("no welcome") };
    drop(raw);
    hp.wait("the dropped one left", |e| e.iter().any(|e| matches!(e, Event::PeerLeft(id) if *id == rid)));
    connections_settle(&host, 1);
    // A later newcomer doesn't see the leavers.
    let (_c, mut cp) = join(&invite, "Cy");
    cp.wait("welcome", welcomed);
    let Some(Event::Welcome { peers, .. }) = cp.events.iter().find(|e| matches!(e, Event::Welcome { .. })) else { panic!("no welcome") };
    assert_eq!(peers.len(), 2, "the host and Bo: {peers:?}");
    assert!(hp.left().contains(&ada));
    host.stop();
    bp.wait("the end of the session", |e| e.iter().any(|e| matches!(e, Event::Ended { reason } if reason == ENDED)));
    assert_eq!(host.connections(), 0);
}

#[test]
fn a_full_session_refuses_more() {
    let (mut host, _hp, invite) = start_with(fast_limits(), Limits { max_peers: 2, ..Limits::default() });
    let (_a, mut ap) = join(&invite, "Ada");
    ap.wait("welcome", welcomed);
    let (_b, mut bp) = join(&invite, "Bo");
    bp.wait("a refusal", |e| e.iter().any(|e| matches!(e, Event::Status(Status::Failed { reason }) if reason.contains("full"))));
    host.stop();
}

#[test]
fn too_many_connections_are_turned_away() {
    let (mut host, _hp, invite) = start_with(NetLimits { max_connections: 2, ..fast_limits() }, Limits::default());
    let addr: SocketAddr = invite.server.host.parse().unwrap();
    let mut a = Raw::hello(addr, TOKEN);
    let mut b = Raw::hello(addr, TOKEN);
    assert!(a.next().is_some() && b.next().is_some());
    // The third socket is closed before the handshake.
    let s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(WAIT)).unwrap();
    assert!(tungstenite::client::client(format!("ws://{addr}/"), s).is_err());
    host.stop();
}

#[test]
fn oversized_messages_end_the_connection() {
    let (mut host, mut hp, invite) = start_with(NetLimits { max_client_message: 4096, ..fast_limits() }, Limits::default());
    let addr: SocketAddr = invite.server.host.parse().unwrap();
    let mut raw = Raw::hello(addr, TOKEN);
    let Some(ServerMsg::Welcome { client_id, .. }) = raw.next() else { panic!("no welcome") };
    raw.send(&ClientMsg::Presence(Presence { selection: Some((0..5000).map(NodeId).collect()), ..Presence::default() }));
    assert!(raw.refusal().is_some_and(|r| r.starts_with("message too large")));
    hp.wait("it left", |e| e.iter().any(|e| matches!(e, Event::PeerLeft(id) if *id == client_id)));
    // The host still serves others.
    let (_a, mut ap) = join(&invite, "Ada");
    ap.wait("welcome", welcomed);
    host.stop();
}

#[test]
fn flooding_ends_the_connection() {
    let (mut host, _hp, invite) = start_with(NetLimits { rate: 5.0, burst: 20.0, ..fast_limits() }, Limits::default());
    let addr: SocketAddr = invite.server.host.parse().unwrap();
    let mut raw = Raw::hello(addr, TOKEN);
    assert!(matches!(raw.next(), Some(ServerMsg::Welcome { .. })));
    for _ in 0..100 {
        if !raw.send(&ClientMsg::Ping) {
            break;
        }
    }
    assert_eq!(raw.refusal().as_deref(), Some("too many messages"));
    host.stop();
}

#[test]
fn malformed_frames_are_refused() {
    let (mut host, _hp, invite) = start();
    let addr: SocketAddr = invite.server.host.parse().unwrap();
    // Garbage instead of a hello, a wrong wire version, text, and something other than a hello.
    type Case = (Box<dyn Fn(&mut Raw)>, &'static str);
    let cases: Vec<Case> = vec![
        (Box::new(|r| assert!(r.send_bytes(vec![1, 255, 255, 255, 255, 9]))), "that message couldn't be read"),
        (
            Box::new(|r| {
                let mut b = message::encode(&Client::hello(FILE, TOKEN, "x", "")).unwrap();
                b[0] = 9;
                assert!(r.send_bytes(b));
            }),
            "that message couldn't be read",
        ),
        (Box::new(|r| assert!(r.0.send(Message::Text("hello".into())).is_ok())), "only binary messages are spoken here"),
        (Box::new(|r| assert!(r.send(&ClientMsg::Ping))), "say hello first"),
    ];
    for (send, why) in cases {
        let mut raw = Raw::connect(addr);
        send(&mut raw);
        assert_eq!(raw.refusal().as_deref(), Some(why));
    }
    // After joining, garbage still ends the connection with a refusal.
    let mut raw = Raw::hello(addr, TOKEN);
    assert!(matches!(raw.next(), Some(ServerMsg::Welcome { .. })));
    assert!(raw.send_bytes(vec![1, 2, 200, 200, 200]));
    assert_eq!(raw.refusal().as_deref(), Some("that message couldn't be read"));
    connections_settle(&host, 0);
    host.stop();
}

#[test]
fn random_frames_never_hurt_the_host() {
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::TestRunner;
    let (mut host, _hp, invite) = start();
    let addr: SocketAddr = invite.server.host.parse().unwrap();
    let mut runner = TestRunner::deterministic();
    let mut join_first = false;
    let frames = proptest::collection::vec(proptest::collection::vec(proptest::num::u8::ANY, 0..200), 1..6);
    for _ in 0..40 {
        let tree = frames.new_tree(&mut runner).unwrap();
        let mut raw = Raw::connect(addr);
        // Sometimes join first, so the frames reach the session itself.
        join_first = !join_first;
        if join_first {
            raw.send(&Client::hello(FILE, TOKEN, "Fuzz", ""));
        }
        for mut f in tree.current() {
            if let Some(b) = f.first_mut() {
                *b = 1;
            }
            if !raw.send_bytes(f) {
                break;
            }
        }
        drop(raw);
    }
    let (_a, mut ap) = join(&invite, "Ada");
    ap.wait("welcome after the fuzzing", welcomed);
    host.stop();
    assert_eq!(host.connections(), 0);
}

#[test]
fn silent_connections_time_out() {
    let (mut host, _hp, invite) = start_with(NetLimits { hello_timeout: Duration::from_millis(200), ..fast_limits() }, Limits::default());
    let addr: SocketAddr = invite.server.host.parse().unwrap();
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(WAIT)).unwrap();
    let t0 = Instant::now();
    let mut buf = [0u8; 64];
    // The host hangs up: end of stream or a reset.
    assert!(!matches!(s.read(&mut buf), Ok(n) if n > 0));
    assert!(t0.elapsed() < Duration::from_secs(5));
    connections_settle(&host, 0);
    host.stop();
}

#[test]
fn connections_without_a_hello_are_told_why_they_end() {
    // A socket's read timeout can end a moment before the hello's deadline; the connection still
    // gets the refusal. Twelve at once, so that happens to some of them.
    let (mut host, _hp, invite) = start_with(NetLimits { hello_timeout: Duration::from_secs(1), ..fast_limits() }, Limits::default());
    let addr: SocketAddr = invite.server.host.parse().unwrap();
    std::thread::scope(|s| {
        let waits: Vec<_> = (0..12).map(|_| s.spawn(move || Raw::connect(addr).refusal())).collect();
        for w in waits {
            assert_eq!(w.join().unwrap().as_deref(), Some("no hello in time"));
        }
    });
    host.stop();
}

#[test]
fn guests_reconnect_after_a_network_failure() {
    let (mut host, mut hp, invite) = start();
    let (g, mut gp) = join(&invite, "Ada");
    gp.wait("welcome", welcomed);
    let first = gp.me();
    host.cut_connections();
    gp.wait("reconnecting", |e| e.iter().any(|e| matches!(e, Event::Status(Status::Reconnecting { attempt: 1 }))));
    gp.wait("welcome again", |e| e.iter().filter(|e| matches!(e, Event::Welcome { .. })).count() >= 3);
    let second = gp.me();
    assert_eq!(first, second, "a reconnect resumes as the same client");
    assert!(
        gp.events.iter().any(|e| matches!(e, Event::Sync(ServerMsg::Welcome { catch_up: figcraft_collab::CatchUp::Delta(_), .. }))),
        "it catches up with what it missed, not the whole file"
    );
    hp.wait("it left and came back", |e| {
        e.iter().any(|e| matches!(e, Event::PeerLeft(id) if *id == first))
            && e.iter().filter(|e| matches!(e, Event::PeerJoined(p) if p.client_id == second)).count() >= 2
    });
    // Presence flows again on the new connection.
    g.presence(presence(9.0, 9.0));
    hp.wait("the cursor again", |e| e.iter().any(|e| matches!(e, Event::Presence(id, _) if *id == second)));
    host.stop();
    gp.wait("the end", |e| e.iter().any(|e| matches!(e, Event::Ended { .. })));
}

#[test]
fn throttle_and_bucket() {
    let t0 = Instant::now();
    let ms = |n: u64| t0 + Duration::from_millis(n);
    let mut t = Throttle::new(Duration::from_millis(33));
    assert_eq!(t.offer(1, ms(0)), Some(1));
    assert_eq!(t.offer(2, ms(10)), None);
    assert_eq!(t.offer(3, ms(20)), None);
    assert_eq!(t.due(), Some(ms(33)));
    assert_eq!(t.take_due(ms(30)), None);
    assert_eq!(t.take_due(ms(33)), Some(3), "the latest waiting value");
    assert_eq!(t.due(), None);
    assert_eq!(t.offer(4, ms(100)), Some(4));
    let mut b = Bucket::new(10.0, 3.0);
    let now = Instant::now();
    assert!(b.take(now) && b.take(now) && b.take(now));
    assert!(!b.take(now));
    assert!(b.take(now + Duration::from_millis(150)), "refills at the rate");
}

#[test]
fn quiet_guests_stay_and_silent_connections_are_dropped() {
    let limits = NetLimits { idle_timeout: Duration::from_millis(600), keepalive: Duration::from_millis(150), ..fast_limits() };
    let (mut host, mut hp, invite) = start_with(limits, Limits::default());
    let probe = Probe::new();
    let mut cfg = quick(GuestConfig::new(invite.clone(), "Ada".into()));
    // The guest pings as often as the host expects. Its own idle timeout stays long: it also bounds
    // the wait for the welcome, which a loaded machine can take longer than 600 ms to send.
    cfg.limits.keepalive = limits.keepalive;
    let _g = Guest::start(cfg, probe.inbox.clone()).unwrap();
    let mut gp = probe;
    gp.wait("welcome", welcomed);
    // A joined raw client that never says anything again.
    let addr: SocketAddr = invite.server.host.parse().unwrap();
    let mut raw = Raw::hello(addr, TOKEN);
    let Some(ServerMsg::Welcome { client_id: silent, .. }) = raw.next() else { panic!("no welcome") };
    hp.wait("the silent one is dropped", |e| e.iter().any(|e| matches!(e, Event::PeerLeft(id) if *id == silent)));
    std::thread::sleep(Duration::from_millis(1500));
    gp.events.extend(gp.inbox.drain());
    hp.events.extend(hp.inbox.drain());
    let ada = gp.me();
    assert!(!gp.events.iter().any(|e| matches!(e, Event::Status(Status::Reconnecting { .. }))), "pings keep a quiet guest in: {:?}", gp.events);
    assert!(!hp.left().contains(&ada));
    host.stop();
}

/// The document in a welcome is compressed only for a client whose hello says it reads that; a
/// client from before gets it as it always did (#1665).
#[test]
fn welcomes_are_packed_only_for_clients_that_read_them() {
    let (mut host, _hp, invite) = start();
    let addr: SocketAddr = invite.server.host.parse().unwrap();
    let mut old = Raw::connect(addr);
    let ClientMsg::Hello(h) = Client::hello(FILE, TOKEN, "Old", "") else { panic!("not a hello") };
    assert!(old.send(&ClientMsg::Hello(Hello { client_version: "0.12.1 signals hosting comments".into(), ..h })));
    let Some(ServerMsg::Welcome { catch_up: CatchUp::Snapshot(plain), .. }) = old.next() else { panic!("no plain welcome") };
    let mut new = Raw::hello(addr, TOKEN);
    let Some(mut welcome @ ServerMsg::Welcome { catch_up: CatchUp::Packed(_), .. }) = new.next() else { panic!("no packed welcome") };
    message::unpack(&mut welcome, 64 << 20).unwrap();
    assert!(matches!(welcome, ServerMsg::Welcome { catch_up: CatchUp::Snapshot(s), .. } if s == plain), "the same document");
    host.stop();
}

#[test]
fn signals_reach_only_clients_that_read_them_and_are_rate_limited() {
    let (mut host, mut hp, invite) = start();
    let addr: SocketAddr = invite.server.host.parse().unwrap();
    hp.wait("the host's own ready", |e| e.iter().any(|e| matches!(e, Event::Signal(_, s) if s.kind == Signal::READY)));
    // A client from before signals: its hello doesn't announce them.
    let mut old = Raw::connect(addr);
    let ClientMsg::Hello(h) = Client::hello(FILE, TOKEN, "Old", "") else { panic!("not a hello") };
    assert!(old.send(&ClientMsg::Hello(Hello { client_version: "0.3.1".into(), ..h })));
    assert!(matches!(old.next(), Some(ServerMsg::Welcome { .. })));
    let mut new = Raw::hello(addr, TOKEN);
    let Some(ServerMsg::Welcome { client_id: me, .. }) = new.next() else { panic!("no welcome") };
    assert_eq!(new.next(), Some(ServerMsg::Signal { client_id: me, signal: Signal::ready() }));
    // The host chats, then moves its pointer: the old client gets the pointer only.
    host.send(ClientMsg::Signal(Signal::chat("hi")));
    host.presence(presence(1.0, 2.0));
    let until_presence = |r: &mut Raw| {
        let mut got = Vec::new();
        loop {
            match r.next() {
                Some(ServerMsg::Presence { .. }) | None => return got,
                Some(m) => got.push(m),
            }
        }
    };
    let old_got = until_presence(&mut old);
    assert!(!old_got.iter().any(|m| matches!(m, ServerMsg::Signal { .. })), "{old_got:?}");
    let new_got = until_presence(&mut new);
    let hana = host.me().client_id;
    assert!(new_got.contains(&ServerMsg::Signal { client_id: hana, signal: Signal::chat("hi") }), "{new_got:?}");
    // A flood of signals: the excess is dropped (the connection stays), the host gets them cleaned.
    for i in 0..200 {
        assert!(new.send(&ClientMsg::Signal(Signal::chat(&format!("{i}\u{7}")))));
    }
    assert!(new.send(&ClientMsg::Presence(presence(3.0, 4.0))));
    hp.wait("the flood's end", |e| e.iter().any(|e| matches!(e, Event::Presence(id, _) if *id == me)));
    let chats: Vec<&Signal> = hp.events.iter().filter_map(|e| if let Event::Signal(id, s) = e { (*id == me).then_some(s) } else { None }).collect();
    assert!(chats.len() >= 40 && chats.len() < 200, "{} of 200 got through", chats.len());
    assert!(chats.iter().all(|s| s.kind == Signal::CHAT && s.text.ends_with(' ')), "cleaned on the way");
    assert!(new.send(&ClientMsg::Ping));
    assert_eq!(until_presence_or_pong(&mut new), Some(ServerMsg::Pong), "still connected");
    host.stop();
}

/// The next message that isn't someone's presence or signal.
fn until_presence_or_pong(r: &mut Raw) -> Option<ServerMsg> {
    loop {
        match r.next() {
            Some(ServerMsg::Presence { .. } | ServerMsg::Signal { .. }) => {}
            m => return m,
        }
    }
}

#[test]
fn the_host_removes_someone_and_they_cant_come_back() {
    let (mut host, mut hp, invite) = start();
    let (_a, mut ap) = join(&invite, "Ada");
    let (b, mut bp) = join(&invite, "Bo");
    ap.wait("welcome", welcomed);
    bp.wait("welcome", welcomed);
    let (ada, bo) = (ap.me(), bp.me());
    connections_settle(&host, 2);
    // The host can't remove themselves, nor someone who isn't there: nothing happens.
    host.remove(host.me().client_id);
    host.remove(9999);
    host.remove(ada);
    // Ada is told the session is over for her (no reconnecting); the others see her leave.
    ap.wait("Ada removed", |e| e.iter().any(|e| matches!(e, Event::Ended { reason } if reason == REMOVED)));
    hp.wait("the host sees Ada leave", |e| e.iter().any(|e| matches!(e, Event::PeerLeft(id) if *id == ada)));
    bp.wait("Bo sees Ada leave", |e| e.iter().any(|e| matches!(e, Event::PeerLeft(id) if *id == ada)));
    connections_settle(&host, 1);
    assert!(!ap.events.iter().any(|e| matches!(e, Event::Status(Status::Reconnecting { .. }))), "a removed guest doesn't reconnect");
    assert!(!hp.left().contains(&bo), "only Ada was removed");
    // Her address can't join again with the same invite (every guest here comes from 127.0.0.1).
    let (_again, mut againp) = join(&invite, "Ada");
    againp.wait("a refusal", |e| e.iter().any(|e| matches!(e, Event::Status(Status::Failed { reason }) if reason.contains("removed"))));
    assert!(!welcomed(&againp.events));
    // Bo, already in, stays and can still talk to the host.
    b.presence(presence(3.0, 4.0));
    hp.wait("Bo's cursor", |e| e.iter().any(|e| matches!(e, Event::Presence(id, _) if *id == bo)));
    host.stop();
    assert_eq!(host.connections(), 0);
}

#[test]
fn a_welcome_over_the_size_limit_says_the_file_is_too_large() {
    let mut doc = Document::new("B".repeat(1 << 20));
    doc.repair();
    let probe = Probe::new();
    let cfg = HostConfig {
        addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        any_port_if_taken: false,
        file: FILE.into(),
        token: TOKEN.into(),
        doc,
        name: "Hana".into(),
        limits: fast_limits(),
        core: Limits::default(),
    };
    let mut host = Host::start(cfg, probe.inbox.clone()).unwrap();
    let invite = Invite::new(host.addr().to_string(), FILE, TOKEN);

    let mut guest_cfg = quick(GuestConfig::new(invite, "Ada".into()));
    guest_cfg.limits = NetLimits { max_server_message: 1 << 20, ..fast_limits() };
    let mut gp = Probe::new();
    let _g = Guest::start(guest_cfg, gp.inbox.clone()).unwrap();
    let too_large = TOO_LARGE_TO_JOIN.replace("{mb}", "1");
    gp.wait("the file too large to join", |e| e.iter().any(|ev| matches!(ev, Event::Status(Status::Failed { reason }) if *reason == too_large)));
    assert!(!welcomed(&gp.events));
    host.stop();
}

/// How long a connection that never says hello stays open while `poke` sends it something every
/// 300 ms (told whether it's the first time); `None`: still open after 6 s. A thread of its own
/// reads the socket and notes when the host ends the stream, so pongs queued ahead of the close
/// and a slow poke loop don't delay it.
fn pre_hello_lifetime(mut poke: impl FnMut(&mut Raw, bool) -> bool) -> Option<Duration> {
    let (mut host, _hp, invite) = start_with(NetLimits { hello_timeout: Duration::from_secs(1), ..fast_limits() }, Limits::default());
    let addr: SocketAddr = invite.server.host.parse().unwrap();
    let mut r = Raw::connect(addr);
    let mut watch = r.0.get_ref().try_clone().unwrap();
    let t0 = Instant::now();
    let (ended_tx, ended) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut buf = [0u8; 1024];
        let until = Instant::now() + WAIT;
        loop {
            match watch.read(&mut buf) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) && Instant::now() < until => {}
                Err(_) => break,
            }
        }
        let _ = ended_tx.send(Instant::now());
    });
    let (mut closed, mut first) = (None, true);
    while t0.elapsed() < Duration::from_secs(6) {
        if !poke(&mut r, std::mem::take(&mut first)) {
            closed = Some(t0.elapsed());
            break;
        }
        if let Ok(at) = ended.recv_timeout(Duration::from_millis(300)) {
            closed = Some(at.saturating_duration_since(t0));
            break;
        }
    }
    host.stop();
    let _ = reader.join();
    closed
}

#[test]
fn pings_do_not_keep_a_connection_without_a_hello_alive() {
    let closed = pre_hello_lifetime(|r, _| r.0.send(Message::Ping(Bytes::new())).is_ok() && r.0.flush().is_ok());
    assert!(closed.is_some_and(|t| t < Duration::from_secs(3)), "pinged pre-hello connection survived: {closed:?}");
}

#[test]
fn a_hello_trickled_in_fragments_does_not_outlast_the_deadline() {
    // A binary message begun and never finished: one masked byte per frame, each in time for the
    // read timeout.
    let closed = pre_hello_lifetime(|r, first| r.0.get_mut().write_all(&[if first { 0x02 } else { 0x00 }, 0x81, 0, 0, 0, 0, b'x']).is_ok());
    assert!(closed.is_some_and(|t| t < Duration::from_secs(3)), "trickled pre-hello connection survived: {closed:?}");
}
