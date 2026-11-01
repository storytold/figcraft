//! Join codes over the network: a file shared on the local network joined by its code (two
//! engine sessions over loopback), what the browser's driver does with an online code, a server
//! older than join codes, and the links that name a code. (A `figcraft-server` handing codes out
//! and resolving them is tested in its own crate, with the same link.)

use std::net::{Ipv4Addr, TcpListener};
use std::sync::Arc;
use std::time::{Duration, Instant};

use figcraft_collab::code::{self, Code, Local};
use figcraft_collab::message::{self, ClientMsg, PROTOCOL, ServerMsg};
use figcraft_engine::Session;
use figcraft_engine::live::{Event, Status};
use serde_json::json;
use tungstenite::{Bytes, Message};

use crate::driver::{Driver, Socket};
use crate::guest::GuestConfig;
use crate::inbox::Inbox;
use crate::invite::{Endpoint, Invite, Lookup};
use crate::loopback_tests::{fast_limits, quick};
use crate::{NetLink, manage};

fn session(name: &str) -> Session {
    let mut s = Session::new();
    s.live.install(Box::new(NetLink::new(Arc::new(|| {})).with_guest_config(quick)));
    s.execute("pref.userName", &json!({"name": name})).unwrap();
    s
}

/// Pump the sessions (as their windows would every frame) until `f` holds.
fn pump(all: &mut [&mut Session], what: &str, f: impl Fn(&[&mut Session]) -> bool) {
    let start = Instant::now();
    loop {
        for s in all.iter_mut() {
            s.live_poll();
            s.live_sync();
        }
        if f(all) {
            return;
        }
        let states: Vec<String> = all.iter().map(|s| s.live_summary().to_string()).collect();
        assert!(start.elapsed() < Duration::from_secs(15), "timed out waiting for {what}: {states:#?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn live(s: &Session) -> bool {
    s.live().status == Status::Live && s.live().syncing()
}

fn layers(s: &Session) -> usize {
    s.doc().map_or(0, |d| d.node_count())
}

#[test]
fn a_local_share_is_joined_by_its_code_and_edits_sync() {
    let (mut host, mut guest) = (session("Hana"), session("Ada"));
    // This machine only, so the test needs no network; the code says where and the secret.
    let shared = host.execute("collab.share", &json!({"via": "local", "bind": "local"})).unwrap();
    assert_eq!((&shared["status"], &shared["share"]["via"], &shared["share"]["access"]), (&json!("live"), &json!("local"), &json!("edit")));
    let shown = shared["code"].as_str().unwrap().to_string();
    assert_eq!(shown.len(), 23, "twenty symbols in fives: {shown}");
    let Code::Local(local) = code::parse(&shown).unwrap() else { panic!("not a local code") };
    assert_eq!(local.ip, Ipv4Addr::LOCALHOST);
    assert_eq!(shared["address"], format!("127.0.0.1:{}", local.port));
    assert_eq!(shared["link"], local.link(), "the code and the invite link name the same session");
    assert!(shared["share"]["viewCode"].is_null(), "everyone edits on the local network");
    // Typed the way people type: lower case, spaces for the dashes.
    let typed = shown.to_lowercase().replace('-', " ");
    assert_eq!(guest.execute("collab.join", &json!({"code": typed})).unwrap()["status"], "connecting");
    pump(&mut [&mut host, &mut guest], "the join", |s| live(s[1]) && s[0].live().peers.len() == 1);
    assert_eq!(guest.live_summary()["access"], "edit");
    // Edits go both ways.
    let before = layers(&host);
    host.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 30})).unwrap();
    pump(&mut [&mut host, &mut guest], "the host's edit", |s| layers(s[1]) == before + 1);
    guest.execute("shape.ellipse", &json!({"x": 80, "y": 10, "width": 40, "height": 30})).unwrap();
    pump(&mut [&mut host, &mut guest], "the guest's edit", |s| layers(s[0]) == before + 2);
    // The link joins the same session (an older app, or a paste).
    let mut third = session("Bo");
    third.execute("collab.join", &json!({"link": shared["link"]})).unwrap();
    pump(&mut [&mut host, &mut guest, &mut third], "the link's join", |s| live(s[2]) && layers(s[2]) == before + 2);
    // Stopping ends it: the code no longer joins.
    assert_eq!(host.execute("collab.stop", &json!({})).unwrap()["ended"], true);
    pump(&mut [&mut host, &mut guest, &mut third], "the end", |s| !s[1].live().status.active() && !s[2].live().status.active());
    let mut late = session("Cy");
    late.execute("collab.join", &json!({"code": shown})).unwrap();
    pump(&mut [&mut late], "the refusal", |s| matches!(s[0].live().status, Status::Failed { .. }));
    // The port is free again: another program's host may have taken it, and that host refuses
    // the code's secret.
    let label = late.live().status.label();
    assert!(label.contains("Couldn't join") || label.contains("The host refused"), "{label}");
}

#[test]
fn a_local_code_with_the_wrong_secret_or_a_typo_doesnt_join() {
    let (mut host, mut guest) = (session("Hana"), session("Ada"));
    let shared = host.execute("collab.share", &json!({"via": "local", "bind": "local"})).unwrap();
    let Code::Local(local) = code::parse(shared["code"].as_str().unwrap()).unwrap() else { panic!("not a local code") };
    // A code for the same address with another secret: well formed, refused by the host.
    let guess = Local::new(local.ip, local.port, local.secret ^ 1).code();
    guest.execute("collab.join", &json!({"code": guess})).unwrap();
    pump(&mut [&mut host, &mut guest], "the refusal", |s| matches!(s[1].live().status, Status::Failed { .. }));
    assert!(guest.live().status.label().contains("wrong session token"), "{:?}", guest.live().status);
    assert!(host.live().peers.is_empty());
    // A typing mistake is caught before anything connects.
    let mut typo: Vec<char> = local.code().chars().collect();
    // The last symbol contains only checksum bits. Changing payload bits can collide with
    // the eight-bit checksum, so it does not guarantee a malformed code.
    let last = typo.last_mut().unwrap();
    *last = if *last == 'A' { 'B' } else { 'A' };
    let e = guest.execute("collab.join", &json!({"code": typo.iter().collect::<String>()})).unwrap_err().to_string();
    assert!(e.contains("typing mistake"), "{e}");
    // A view-only code doesn't exist here, and says where it does.
    let e = host.execute("collab.share", &json!({"access": "view"})).unwrap_err().to_string();
    assert!(e.contains("share it online"), "{e}");
}

/// An invite naming only `server` (what a join code is asked at).
fn code_at(server: &Endpoint) -> Invite {
    Invite { server: server.clone(), file: String::new(), token: String::new() }
}

#[test]
fn links_can_name_a_join_code() {
    let (invite, found) = Invite::parse_join("figcraft://join?server=wss://share.example/&code=k7m-2qx-9td").unwrap();
    assert_eq!((invite.server.url(), invite.file.as_str(), invite.token.as_str()), ("wss://share.example:443/".to_string(), "", ""));
    assert_eq!(found, Some(Lookup::Code("K7M2QX9TD".into())));
    let (invite, found) = Invite::parse_join(" figcraft://join?host=127.0.0.1:47900&code=K7M2QX9TD ").unwrap();
    assert_eq!((invite.server, found), (Endpoint::plain("127.0.0.1:47900"), Some(Lookup::Code("K7M2QX9TD".into()))));
    // A link with a file and a token is read as before.
    let link = "figcraft://join?host=127.0.0.1:47900&file=abc&token=AbCdEfGhIjKlMnOpQrStUv";
    assert_eq!(Invite::parse_join(link).unwrap(), (Invite::parse(link).unwrap(), None));
    for bad in [
        "figcraft://join?server=wss://share.example/&code=",
        "figcraft://join?server=wss://share.example/&code=K7M",
        "figcraft://join?code=K7M2QX9TD",
    ] {
        assert!(Invite::parse_join(bad).is_err(), "{bad}");
    }
    // Where a file's link is needed (team libraries), a code isn't one.
    let e = Invite::parse("figcraft://join?server=wss://share.example/&code=K7M2QX9TD").unwrap_err();
    assert!(e.contains("join code") && !e.contains("K7M"), "{e}");
}

/// A server from before join codes: whatever it is asked first, it can't read it (what its
/// decoder says about a message it doesn't know).
fn old_server() -> (Endpoint, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let at = Endpoint::plain(listener.local_addr().unwrap().to_string());
    let t = std::thread::spawn(move || {
        for stream in listener.incoming().take(2).flatten() {
            let Ok(mut ws) = tungstenite::accept(stream) else { continue };
            // read() returns Result<Message>, not a lock guard; this test server deliberately
            // ignores whatever the client sent first.
            let _ = ws.read(); // NOSONAR
            let refused = ServerMsg::Refused { reason: "that message couldn't be read".into() };
            let _ = ws.send(Message::Binary(Bytes::from(message::encode(&refused).unwrap())));
            let _ = ws.close(None);
            while ws.read().is_ok() {}
        }
    });
    (at, t)
}

/// A server from before join codes that manages a file's links as it always did: two connections,
/// the first makes links, the second (asked for codes) can't read the request.
fn old_server_with_links() -> (Endpoint, std::thread::JoinHandle<()>) {
    use figcraft_collab::message::{Admin, AdminReply, LinkInfo, NewLink};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let at = Endpoint::plain(listener.local_addr().unwrap().to_string());
    let t = std::thread::spawn(move || {
        let mut made = 0;
        for stream in listener.incoming().take(2).flatten() {
            let Ok(mut ws) = tungstenite::accept(stream) else { continue };
            let say = |ws: &mut tungstenite::WebSocket<std::net::TcpStream>, m: ServerMsg| {
                let _ = ws.send(Message::Binary(Bytes::from(message::encode(&m).unwrap())));
            };
            while let Ok(Message::Binary(b)) = ws.read() {
                match message::decode::<ClientMsg>(&b, usize::MAX) {
                    Ok(ClientMsg::OpenHosting(_)) => say(&mut ws, ServerMsg::Hosting(Box::default())),
                    Ok(ClientMsg::Admin(Admin::CreateLink { file, access, label })) => {
                        made += 1;
                        let link = LinkInfo { id: format!("link{made}"), file, access, label, created: 0 };
                        say(&mut ws, ServerMsg::Admin(Box::new(AdminReply::Link(NewLink { link, token: format!("TokenTokenTokenToken0{made}") }))));
                    }
                    _ => {
                        say(&mut ws, ServerMsg::Refused { reason: "that message couldn't be read".into() });
                        break;
                    }
                }
            }
            let _ = ws.close(None);
            while ws.read().is_ok() {}
        }
    });
    (at, t)
}

#[test]
fn a_server_older_than_join_codes_says_so_and_the_links_still_work() {
    let (server, thread) = old_server();
    let limits = fast_limits();
    // Asking it about a code: the join fails with what to do instead.
    let e = manage::resolve(&code_at(&server), &Lookup::Code("K7M2QX9TD".into()), &limits, Duration::from_secs(5)).unwrap_err();
    assert!(e.starts_with(manage::REFUSED) && e.contains("couldn't be read"), "{e}");
    let mut guest = session("Ada");
    guest.execute("pref.publishServer", &json!({"url": server.url()})).unwrap();
    guest.execute("collab.join", &json!({"code": "K7M-2QX-9TD"})).unwrap();
    pump(&mut [&mut guest], "the answer", |s| matches!(s[0].live().status, Status::Failed { .. }));
    let why = guest.live().status.label();
    assert!(why.contains("doesn't know join codes yet") && why.contains("invite link"), "{why}");
    thread.join().unwrap();
    // Sharing a file there: it makes the links (it always could) but no codes, without an error:
    // the app shows the links.
    let (server, thread) = old_server_with_links();
    let shared =
        manage::share(&server, None, "0123456789abcdef", "AbCdEfGhIjKlMnOpQrStUv", &[], true, &limits, Duration::from_secs(5)).unwrap().unwrap();
    assert!(shared.edit.code.is_none() && shared.view.as_ref().is_some_and(|v| v.code.is_none()));
    assert!(shared.edit.link.contains("&file=0123456789abcdef&token=TokenTokenTokenToken01") && shared.edit.id == "link1");
    thread.join().unwrap();
}

#[test]
fn a_server_that_cant_be_reached_says_so() {
    // A port nothing listens on.
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut guest = session("Ada");
    guest.execute("pref.publishServer", &json!({"url": format!("ws://127.0.0.1:{port}/")})).unwrap();
    guest.execute("collab.join", &json!({"code": "K7M-2QX-9TD"})).unwrap();
    pump(&mut [&mut guest], "the failure", |s| matches!(s[0].live().status, Status::Failed { .. }));
    assert!(guest.live().status.label().starts_with("Couldn't reach the server"), "{:?}", guest.live().status);
}

#[test]
fn an_app_hosted_session_knows_no_online_codes() {
    let mut host = session("Hana");
    let shared = host.execute("collab.host", &json!({"port": 0})).unwrap();
    let server = Endpoint::plain(shared["address"].as_str().unwrap());
    let e = manage::resolve(&code_at(&server), &Lookup::Code("K7M2QX9TD".into()), &fast_limits(), Duration::from_secs(5)).unwrap_err();
    assert!(e.contains("hosted by an app"), "{e}");
    // Junk in place of a code is refused the same way, and the session goes on.
    for junk in ["", "💥", &"Z".repeat(4000)] {
        assert!(manage::resolve(&code_at(&server), &Lookup::Code(junk.into()), &fast_limits(), Duration::from_secs(5)).is_err());
    }
    assert_eq!(host.live_summary()["status"], "live");
}

/// A socket that records what the driver does (the browser's, scripted).
#[derive(Default)]
struct Script {
    opened: Vec<String>,
    sent: Vec<ClientMsg>,
    closes: usize,
}

impl Socket for Script {
    fn open(&mut self, url: &str) -> Result<(), String> {
        self.opened.push(url.to_string());
        Ok(())
    }
    fn send(&mut self, bytes: &[u8]) -> bool {
        self.sent.push(message::decode(bytes, usize::MAX).unwrap());
        true
    }
    fn close(&mut self) {
        self.closes += 1;
    }
}

/// The browser's driver joining with a code at a server.
fn driver(t0: web_time::Instant) -> (Driver<Script>, Inbox) {
    let (invite, found) = Invite::parse_join("figcraft://join?server=ws://127.0.0.1:47900/&code=K7M2QX9TD").unwrap();
    let inbox = Inbox::new(Arc::new(|| {}));
    let cfg = GuestConfig { lookup: found, ..GuestConfig::new(invite, "Ada".into()) };
    (Driver::start(Script::default(), cfg, inbox.clone(), t0), inbox)
}

fn failure(inbox: &Inbox) -> String {
    inbox.drain().into_iter().find_map(|e| if let Event::Status(Status::Failed { reason }) = e { Some(reason) } else { None }).unwrap_or_default()
}

#[test]
fn the_browser_asks_about_the_code_then_joins_the_file() {
    let t0 = web_time::Instant::now();
    let (mut d, inbox) = driver(t0);
    d.opened(t0);
    let [ClientMsg::Resolve(ask)] = d.sock.sent.as_slice() else { panic!("asked {:?}", d.sock.sent) };
    assert_eq!((ask.protocol, ask.code.as_str()), (PROTOCOL, "K7M2QX9TD"));
    // The answer: a new connection to the same server, and a hello for the file.
    let answer = ServerMsg::Resolved { file: "0123456789abcdef".into(), token: "AbCdEfGhIjKlMnOpQrStUv".into() };
    d.message(&message::encode(&answer).unwrap(), t0);
    assert_eq!((d.sock.opened.len(), d.sock.closes), (2, 1));
    assert!(!d.done());
    d.opened(t0);
    let Some(ClientMsg::Hello(hello)) = d.sock.sent.last() else { panic!("sent {:?}", d.sock.sent) };
    assert_eq!((hello.file.as_str(), hello.token.as_str(), hello.name.as_str()), ("0123456789abcdef", "AbCdEfGhIjKlMnOpQrStUv", "Ada"));
    assert!(failure(&inbox).is_empty());
}

#[test]
fn the_browser_shows_why_a_code_didnt_join() {
    let t0 = web_time::Instant::now();
    // The server's own words for a code it doesn't know.
    let (mut d, inbox) = driver(t0);
    d.opened(t0);
    d.message(&message::encode(&ServerMsg::Refused { reason: "No session with that code".into() }).unwrap(), t0);
    assert!(d.done());
    assert_eq!(failure(&inbox), "No session with that code");
    // An older server.
    let (mut d, inbox) = driver(t0);
    d.opened(t0);
    d.message(&message::encode(&ServerMsg::Refused { reason: "that message couldn't be read".into() }).unwrap(), t0);
    assert!(failure(&inbox).contains("doesn't know join codes yet"));
    // Nobody answers: no retries while still asking.
    let (mut d, inbox) = driver(t0);
    d.tick(t0 + Duration::from_secs(30));
    assert!(d.done());
    assert!(failure(&inbox).starts_with("Couldn't reach the server"));
    // An answer that isn't a file.
    let (mut d, inbox) = driver(t0);
    d.opened(t0);
    d.message(&message::encode(&ServerMsg::Resolved { file: "../etc".into(), token: "short".into() }).unwrap(), t0);
    assert!(d.done());
    assert!(failure(&inbox).contains("isn't a file"));
    // Leaving while asking is just leaving.
    let (mut d, inbox) = driver(t0);
    d.leave(t0);
    assert!(d.done() && failure(&inbox).is_empty());
}

#[test]
fn links_can_name_a_file_of_a_servers_team_list() {
    let link = "figcraft://join?server=wss://figcraft.example/team/&file=0123456789abcdef&team=1";
    let (invite, found) = Invite::parse_join(link).unwrap();
    assert_eq!(
        (invite.server.url(), invite.file.as_str(), invite.token.as_str(), found),
        ("wss://figcraft.example:443/team/".to_string(), "0123456789abcdef", "", Some(Lookup::Team))
    );
    assert_eq!(Invite::parse_join(&invite.team_link()).unwrap(), (invite.clone(), Some(Lookup::Team)));
    let plain = Invite::new("127.0.0.1:47900", "abc", "");
    assert_eq!(plain.team_link(), "figcraft://join?host=127.0.0.1:47900&file=abc&team=1");
    // It names no token, so it isn't a link to join with directly; it needs a file.
    assert!(Invite::parse(link).is_err_and(|e| e.contains("team list")));
    assert!(Invite::parse_join("figcraft://join?server=wss://figcraft.example/&team=1").is_err());
    assert!(Invite::parse_join("figcraft://join?server=wss://figcraft.example/&file=../x&team=1").is_err());
}

/// The browser's driver joining a file of a server's team list.
fn team_driver(t0: web_time::Instant) -> (Driver<Script>, Inbox) {
    let (invite, found) = Invite::parse_join("figcraft://join?server=ws://127.0.0.1:47900/&file=0123456789abcdef&team=1").unwrap();
    let inbox = Inbox::new(Arc::new(|| {}));
    let cfg = GuestConfig { lookup: found, ..GuestConfig::new(invite, "Ada".into()) };
    (Driver::start(Script::default(), cfg, inbox.clone(), t0), inbox)
}

#[test]
fn the_browser_asks_the_team_list_for_the_link_then_joins_the_file() {
    let t0 = web_time::Instant::now();
    let (mut d, inbox) = team_driver(t0);
    d.opened(t0);
    let [ClientMsg::ResolveTeam(ask)] = d.sock.sent.as_slice() else { panic!("asked {:?}", d.sock.sent) };
    assert_eq!((ask.protocol, ask.file.as_str()), (PROTOCOL, "0123456789abcdef"));
    let answer = ServerMsg::Resolved { file: "0123456789abcdef".into(), token: "AbCdEfGhIjKlMnOpQrStUv".into() };
    d.message(&message::encode(&answer).unwrap(), t0);
    d.opened(t0);
    let Some(ClientMsg::Hello(hello)) = d.sock.sent.last() else { panic!("sent {:?}", d.sock.sent) };
    assert_eq!((hello.file.as_str(), hello.token.as_str()), ("0123456789abcdef", "AbCdEfGhIjKlMnOpQrStUv"));
    assert!(failure(&inbox).is_empty());
    // Another file in the answer than the one asked about isn't taken.
    let (mut d, inbox) = team_driver(t0);
    d.opened(t0);
    d.message(&message::encode(&ServerMsg::Resolved { file: "fedcba9876543210".into(), token: "AbCdEfGhIjKlMnOpQrStUv".into() }).unwrap(), t0);
    assert!(d.done() && failure(&inbox).contains("isn't a file"));
    // A server from before team lists.
    let (mut d, inbox) = team_driver(t0);
    d.opened(t0);
    d.message(&message::encode(&ServerMsg::Refused { reason: "that message couldn't be read".into() }).unwrap(), t0);
    assert!(failure(&inbox).contains("has no team list"));
}
