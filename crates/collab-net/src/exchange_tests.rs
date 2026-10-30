//! The browser's requests to a server outside a session ([`crate::exchange`]), run by its
//! [`Driver`] over a scripted socket with a made-up clock: publishing (with and without the
//! server's key, sharing or not, then joining the new file), what a failure leaves, and the
//! background requests the browser can send.

use std::sync::Arc;
use std::time::{Duration, Instant};

use figcraft_collab::message::{self, Access, Admin, AdminReply, ClientMsg, FileInfo, HostInfo, LinkInfo, NewLink, Presence, ServerMsg};
use figcraft_doc::{Document, NodeId};
use figcraft_engine::live::{Event, NetReply, NetRequest, Shared, Status};

use crate::driver::{Driver, Socket};
use crate::exchange::{self, Next};
use crate::inbox::Inbox;
use crate::invite::{Endpoint, Invite};
use crate::join::GuestConfig;
use crate::requests::Publish;

const FILE: &str = "0123456789abcdef";
const TOKEN: &str = "AbCdEfGhIjKlMnOpQrStUv";
const SERVER: &str = "ws://127.0.0.1:47910/";

/// A socket that records what the driver does.
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

fn server() -> Endpoint {
    Endpoint::parse(SERVER).unwrap().0
}

fn job(key: Option<&str>, share: bool) -> Publish {
    let mut doc = Document::new("Poster");
    doc.repair();
    Publish { server: server(), key: key.map(String::from), name: "Poster".into(), doc: Arc::new(doc), share }
}

fn config() -> GuestConfig {
    GuestConfig::new(Invite { server: server(), file: String::new(), token: String::new() }, "Ada".into())
}

/// A background request's driver.
fn run(next: Next, t0: Instant) -> (Driver<Script>, Inbox) {
    let inbox = Inbox::new(Arc::new(|| {}));
    (Driver::asking(Script::default(), config(), next, inbox.clone(), t0), inbox)
}

/// A publish's driver.
fn publish(job: Publish, t0: Instant) -> (Driver<Script>, Inbox) {
    let inbox = Inbox::new(Arc::new(|| {}));
    (Driver::publish(Script::default(), config(), job, inbox.clone(), t0), inbox)
}

fn new_link(id: &str, access: Access, token: &str) -> NewLink {
    NewLink { link: LinkInfo { id: id.into(), file: FILE.into(), access, label: String::new(), created: 0 }, token: token.into() }
}

fn file() -> FileInfo {
    FileInfo { id: FILE.into(), name: "Poster".into(), created: 0, updated: 0, seq: 0, bytes: 0, people: 0 }
}

fn hosting() -> ServerMsg {
    ServerMsg::Hosting(Box::new(HostInfo { open: true, operator: "ops@example.com".into(), ..HostInfo::default() }))
}

fn admin(r: AdminReply) -> ServerMsg {
    ServerMsg::Admin(Box::new(r))
}

fn hosted() -> ServerMsg {
    admin(AdminReply::Hosted {
        file: file(),
        edit: new_link("e0", Access::Edit, TOKEN),
        view: new_link("v0", Access::View, "ViewViewViewViewViewVi"),
        owner: "owner-token".into(),
        expires: Some(9),
    })
}

/// The server says `m` to the driver.
fn say(d: &mut Driver<Script>, m: &ServerMsg, at: Instant) {
    d.message(&message::encode(m).unwrap(), at);
}

fn last(d: &Driver<Script>) -> &ClientMsg {
    d.sock.sent.last().unwrap()
}

fn failure(events: &[Event]) -> Option<&str> {
    events.iter().find_map(|e| if let Event::Status(Status::Failed { reason }) = e { Some(reason.as_str()) } else { None })
}

/// The welcome a server sends Ada in the published file, encoded.
fn welcome() -> ServerMsg {
    let mut doc = Document::new("Poster");
    doc.repair();
    let mut server = figcraft_collab::Server::new(doc, TOKEN, figcraft_collab::Limits::default());
    let ClientMsg::Hello(h) = figcraft_collab::Client::hello(FILE, TOKEN, "Ada", "") else { panic!("no hello") };
    let (_, outs) = server.join(h).unwrap();
    outs.into_iter().find_map(|o| if let figcraft_collab::Outgoing::To(_, w @ ServerMsg::Welcome { .. }) = o { Some(w) } else { None }).unwrap()
}

/// The server makes the links and codes for others, each step checked.
fn make_links(d: &mut Driver<Script>, t0: Instant) {
    // As the file's owner, on a connection of its own.
    assert_eq!((d.sock.opened.len(), d.sock.closes), (2, 1));
    d.opened(t0);
    assert!(matches!(last(d), ClientMsg::OpenHosting(o) if o.file == FILE && o.owner == "owner-token"));
    say(d, &hosting(), t0);
    assert!(matches!(last(d), ClientMsg::Admin(Admin::CreateLink { access: Access::Edit, .. })));
    say(d, &admin(AdminReply::Link(new_link("e1", Access::Edit, "EditEditEditEditEditEd"))), t0);
    assert!(matches!(last(d), ClientMsg::Admin(Admin::CreateLink { access: Access::View, .. })));
    say(d, &admin(AdminReply::Link(new_link("v1", Access::View, "SeeSeeSeeSeeSeeSeeSeeS"))), t0);
    // The comment link, on another.
    d.opened(t0);
    say(d, &hosting(), t0);
    assert!(matches!(last(d), ClientMsg::Admin(Admin::CreateLink { access: Access::Comment, .. })));
    say(d, &admin(AdminReply::Link(new_link("c1", Access::Comment, "SaySaySaySaySaySaySayS"))), t0);
    // Their codes, on another.
    d.opened(t0);
    say(d, &ServerMsg::Pong, t0);
    say(d, &hosting(), t0);
    assert!(matches!(last(d), ClientMsg::Admin(Admin::Code { token, .. }) if token == "EditEditEditEditEditEd"));
    say(d, &admin(AdminReply::Code { code: "ABCDEFGHJ".into() }), t0);
    say(d, &admin(AdminReply::Code { code: "KLMNPQRST".into() }), t0);
    say(d, &admin(AdminReply::Code { code: "VWXYZ0123".into() }), t0);
}

#[test]
fn publishing_without_the_key_uploads_shares_then_joins() {
    let t0 = Instant::now();
    let (mut d, inbox) = publish(job(None, true), t0);
    assert_eq!(d.sock.opened, vec![SERVER], "no key or token in the URL");
    assert_eq!(inbox.drain().len(), 1, "connecting");
    d.opened(t0);
    assert!(matches!(last(&d), ClientMsg::OpenHosting(o) if o.file.is_empty() && o.owner.is_empty()));
    // Presence waits for the session.
    d.presence(Presence { cursor: Some((NodeId(1), 1.0, 2.0)), ..Presence::default() }, t0);
    say(&mut d, &hosting(), t0);
    assert!(matches!(last(&d), ClientMsg::Admin(Admin::Publish { name, .. }) if name == "Poster"));
    say(&mut d, &hosted(), t0);
    make_links(&mut d, t0);
    let events = inbox.drain();
    let [Event::Hosting(h), Event::Published(p), Event::Shared(Ok(Shared { edit, view: Some(view), comment: Some(comment) }))] = events.as_slice()
    else {
        panic!("{events:#?}")
    };
    assert_eq!(h.operator, "ops@example.com");
    assert_eq!((p.file.as_str(), p.owner.as_deref(), p.expires), (FILE, Some("owner-token"), Some(9)));
    assert_eq!((edit.id.as_str(), edit.code.as_deref(), view.code.as_deref()), ("e1", Some("ABCDEFGHJ"), Some("KLMNPQRST")));
    assert!(edit.link.contains("token=EditEditEditEditEditEd"));
    assert_eq!((comment.id.as_str(), comment.code.as_deref()), ("c1", Some("VWXYZ0123")));
    assert!(comment.link.contains("token=SaySaySaySaySaySaySayS"));
    // Then the file, with the owner's edit link.
    assert_eq!(d.sock.opened.len(), 5);
    d.opened(t0);
    let ClientMsg::Hello(hello) = last(&d) else { panic!("{:?}", last(&d)) };
    assert_eq!((hello.file.as_str(), hello.token.as_str()), (FILE, TOKEN));
    say(&mut d, &welcome(), t0);
    assert!(matches!(last(&d), ClientMsg::Presence(_)), "the presence that waited goes out with the welcome");
    assert!(matches!(inbox.drain().first(), Some(Event::Welcome { .. })));
    assert!(!d.done());
}

#[test]
fn publishing_with_the_key_joins_without_sharing() {
    let t0 = Instant::now();
    let (mut d, inbox) = publish(job(Some("k3y"), false), t0);
    d.opened(t0);
    assert!(matches!(last(&d), ClientMsg::Manage(m) if m.key == "k3y"));
    say(&mut d, &admin(AdminReply::Ready), t0);
    let published = AdminReply::Published { file: file(), edit: new_link("e0", Access::Edit, TOKEN), view: new_link("v0", Access::View, TOKEN) };
    say(&mut d, &admin(published), t0);
    let events = inbox.drain();
    assert!(matches!(events.as_slice(), [Event::Status(Status::Connecting), Event::Published(p)] if p.owner.is_none()), "{events:#?}");
    d.opened(t0);
    assert!(matches!(last(&d), ClientMsg::Hello(h) if h.token == TOKEN));
}

#[test]
fn a_failed_publish_says_why_and_joins_nothing() {
    let t0 = Instant::now();
    let refusals = [
        (ServerMsg::Refused { reason: "that message couldn't be read".into() }, "takes files only with its key (it is older than this app)"),
        (ServerMsg::Refused { reason: "this server takes files only with its key".into() }, "the server refused: this server takes files"),
        (admin(AdminReply::Ready), "something other than an answer"),
    ];
    for (m, why) in refusals {
        let (mut d, inbox) = publish(job(None, true), t0);
        d.opened(t0);
        say(&mut d, &m, t0);
        let events = inbox.drain();
        assert!(failure(&events).is_some_and(|r| r.starts_with("Couldn't publish to the server: ") && r.contains(why)), "{events:#?}");
        assert!(d.done() && d.sock.opened.len() == 1, "nothing more is tried");
    }
    // The upload's own answer fails, or the key is wrong.
    let (mut d, inbox) = publish(job(Some("bad"), false), t0);
    d.opened(t0);
    say(&mut d, &admin(AdminReply::Failed { reason: "the file is too large".into() }), t0);
    assert!(failure(&inbox.drain()).is_some_and(|r| r.contains("answered Failed")));
    let (mut d, inbox) = publish(job(Some("k"), false), t0);
    d.opened(t0);
    say(&mut d, &admin(AdminReply::Ready), t0);
    say(&mut d, &admin(AdminReply::Failed { reason: "the file is too large".into() }), t0);
    assert!(failure(&inbox.drain()).is_some_and(|r| r.ends_with("the file is too large")));
    // The server goes away, or never answers.
    let (mut d, inbox) = publish(job(None, false), t0);
    d.opened(t0);
    d.closed(1000, "busy", t0);
    assert!(failure(&inbox.drain()).is_some_and(|r| r.ends_with("the server closed the connection: busy")));
    let (mut d, inbox) = publish(job(None, false), t0);
    d.closed(1006, "", t0);
    assert!(failure(&inbox.drain()).is_some_and(|r| r.contains("no answer from 127.0.0.1:47910")));
    let (mut d, inbox) = publish(job(None, false), t0);
    d.opened(t0);
    d.text(t0);
    assert!(failure(&inbox.drain()).is_some_and(|r| r.ends_with("the server sent a text message")));
}

#[test]
fn a_publish_waits_for_the_server_then_gives_up() {
    let t0 = Instant::now();
    let (mut d, inbox) = publish(job(None, false), t0);
    // Until the connection opens, then as long as a large upload may take.
    assert_eq!(d.deadline(), Some(t0 + Duration::from_secs(5)));
    d.opened(t0);
    let wait = crate::limits::NetLimits::default().request_reply_timeout;
    assert_eq!(d.deadline(), Some(t0 + wait));
    d.tick(t0 + wait - Duration::from_secs(1));
    assert!(!d.done());
    d.tick(t0 + wait);
    assert!(failure(&inbox.drain()).is_some_and(|r| r.contains("no answer from the server")));
    assert!(d.done() && d.deadline().is_none());
    // One that never opens.
    let (mut d, inbox) = publish(job(None, false), t0);
    d.tick(t0 + Duration::from_secs(5));
    assert!(failure(&inbox.drain()).is_some_and(|r| r.contains("no answer from 127.0.0.1:47910")));
}

#[test]
fn leaving_while_publishing_stops_there() {
    let t0 = Instant::now();
    let (mut d, inbox) = publish(job(None, false), t0);
    d.opened(t0);
    inbox.drain();
    d.leave(t0);
    assert!(d.done() && d.sock.closes == 1);
    say(&mut d, &hosting(), t0);
    assert!(inbox.drain().is_empty(), "nothing more happens");
}

#[test]
fn sharing_that_fails_still_joins_the_file() {
    let t0 = Instant::now();
    let (mut d, inbox) = publish(job(None, true), t0);
    d.opened(t0);
    say(&mut d, &hosting(), t0);
    say(&mut d, &hosted(), t0);
    d.opened(t0);
    say(&mut d, &ServerMsg::Refused { reason: "wrong owner token".into() }, t0);
    let events = inbox.drain();
    assert!(matches!(events.last(), Some(Event::Shared(Err(e))) if e.contains("wrong owner token")), "{events:#?}");
    assert_eq!(d.sock.opened.len(), 3, "the file is joined anyway");
    // A server from before join codes (and comment links) shares the edit and view links without
    // codes.
    let (mut d, inbox) = publish(job(None, true), t0);
    d.opened(t0);
    say(&mut d, &hosting(), t0);
    say(&mut d, &hosted(), t0);
    d.opened(t0);
    say(&mut d, &hosting(), t0);
    say(&mut d, &admin(AdminReply::Link(new_link("e1", Access::Edit, TOKEN))), t0);
    say(&mut d, &admin(AdminReply::Link(new_link("v1", Access::View, TOKEN))), t0);
    for _ in ["the comment link", "the codes"] {
        d.opened(t0);
        say(&mut d, &hosting(), t0);
        d.closed(1000, "that message couldn't be read", t0);
    }
    let events = inbox.drain();
    assert!(
        matches!(events.last(), Some(Event::Shared(Ok(s))) if s.edit.code.is_none() && s.view.as_ref().is_some_and(|v| v.code.is_none()) && s.comment.is_none())
    );
}

/// Run background request `req` to its answer against `answers` (one per message the driver sends).
fn background(req: NetRequest, answers: &[ServerMsg]) -> (NetReply, Vec<ClientMsg>) {
    let t0 = Instant::now();
    let (mut d, inbox) = run(exchange::background(7, req).unwrap(), t0);
    let mut connections = 0;
    for m in answers {
        // Each exchange on a connection of its own: it opens, then the server answers.
        if d.sock.opened.len() > connections {
            connections = d.sock.opened.len();
            d.opened(t0);
        }
        say(&mut d, m, t0);
    }
    assert!(d.done(), "{:?}", d.sock.sent);
    let events = inbox.drain();
    let [Event::Net { job: 7, reply }] = events.as_slice() else { panic!("{events:#?}") };
    ((**reply).clone(), d.sock.sent.clone())
}

#[test]
fn background_requests_ask_about_hosting_and_sharing() {
    let req = |r| NetRequest::Hosting { server: SERVER.into(), file: FILE.into(), owner: "owner-token".into(), req: r };
    let (reply, sent) = background(req(None), &[hosting()]);
    assert!(matches!(reply, NetReply::Hosting(Ok((h, None))) if h.open));
    assert!(matches!(sent.as_slice(), [ClientMsg::OpenHosting(o)] if o.owner == "owner-token"));
    // A link it makes comes back whole.
    let create = Admin::CreateLink { file: FILE.into(), access: Access::View, label: String::new() };
    let (reply, _) = background(req(Some(create)), &[hosting(), admin(AdminReply::Link(new_link("v2", Access::View, TOKEN)))]);
    assert!(
        matches!(reply, NetReply::Hosting(Ok((_, Some(AdminReply::Link(l))))) if l.token.starts_with("figcraft://join?") && l.token.contains(TOKEN))
    );
    // Stopping a share revokes its links, with the key when the address carries it.
    let stop =
        NetRequest::Share { server: format!("{SERVER}?key=k3y"), file: FILE.into(), owner: String::new(), revoke: vec!["e1".into()], create: false };
    let (reply, sent) = background(stop, &[admin(AdminReply::Ready), admin(AdminReply::Done)]);
    assert!(matches!(reply, NetReply::Share(Ok(None))));
    assert!(matches!(sent.as_slice(), [ClientMsg::Manage(m), ClientMsg::Admin(Admin::Revoke { link, .. })] if m.key == "k3y" && link == "e1"));
    // Addresses that aren't one answer at once; what the browser doesn't send stays the engine's.
    let bad_hosting = NetRequest::Hosting { server: "nope".into(), file: String::new(), owner: String::new(), req: None };
    let bad_share = NetRequest::Share { server: "http://x".into(), file: FILE.into(), owner: String::new(), revoke: Vec::new(), create: true };
    for bad in [bad_hosting, bad_share] {
        let Some(Next::Done(events, None)) = exchange::background(1, bad) else { panic!("not answered at once") };
        assert!(matches!(events.as_slice(), [Event::Net { reply, .. }] if matches!(**reply, NetReply::Hosting(Err(_)) | NetReply::Share(Err(_)))));
    }
    let plugins = NetRequest::Plugins { server: SERVER.into(), req: figcraft_collab::message::PluginReq::List { query: String::new() } };
    assert!(exchange::background(1, plugins).is_none());
}
