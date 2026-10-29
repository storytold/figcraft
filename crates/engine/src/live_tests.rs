//! Live session state and the `collab.*` commands against a scripted link (the real one, over
//! the network, is tested in `figcraft-collab-net`).

use std::sync::{Arc, Mutex};

use figcraft_collab::message::{ClientMsg, ServerMsg};
use figcraft_collab::{Client, Limits, Outgoing, Server};
use figcraft_doc::{Document, NodeId};
use serde_json::json;

use crate::live::{Access, Event, HostOptions, LiveLink, Peer, Presence, PublishOptions, Role, SessionInfo, Status};
use crate::{EngineError, Session};

/// What the scripted link was asked to do, and the events it hands out next.
#[derive(Default)]
pub(crate) struct Script {
    pub(crate) events: Vec<Event>,
    pub(crate) sent: Vec<Presence>,
    pub(crate) hosted: Vec<HostOptions>,
    joined: Vec<String>,
    pub(crate) left: usize,
    pub(crate) msgs: Vec<ClientMsg>,
    pub(crate) published: Vec<PublishOptions>,
    /// The session started last: the scripted events are its.
    pub(crate) session: u64,
    /// The sessions ended, in order.
    pub(crate) ended: Vec<u64>,
    /// The session each presence in `sent` and each message in `msgs` was for.
    pub(crate) sent_to: Vec<u64>,
    pub(crate) msgs_to: Vec<u64>,
    /// The people removed: `(session, client)`.
    pub(crate) removed: Vec<(u64, u32)>,
}

pub(crate) struct Mock(pub(crate) Arc<Mutex<Script>>);

impl LiveLink for Mock {
    fn host(&mut self, session: u64, opts: HostOptions) -> Result<SessionInfo, String> {
        let mut s = self.0.lock().unwrap();
        s.session = session;
        s.hosted.push(opts);
        // Each session listens on its own port (47900 for the first).
        let mut i = info(Role::Host, 1);
        i.address = format!("127.0.0.1:{}", 47_899 + session);
        i.link = format!("figcraft://join?host={}&file=f&token=t", i.address);
        i.url = format!("ws://{}/?file=f&token=t", i.address);
        Ok(i)
    }
    fn join(&mut self, session: u64, link: &str, _: &str) -> Result<(), String> {
        if !link.starts_with("figcraft://") {
            return Err("not an invite".into());
        }
        let mut s = self.0.lock().unwrap();
        s.session = session;
        s.joined.push(link.to_string());
        Ok(())
    }
    fn publish(&mut self, session: u64, opts: PublishOptions) -> Result<(), String> {
        if !opts.server.starts_with("ws") {
            return Err("not a server address".into());
        }
        let mut s = self.0.lock().unwrap();
        s.session = session;
        s.published.push(opts);
        Ok(())
    }
    fn remove(&mut self, session: u64, client: u32) -> Result<(), String> {
        self.0.lock().unwrap().removed.push((session, client));
        Ok(())
    }
    fn leave(&mut self, session: u64) {
        let mut s = self.0.lock().unwrap();
        s.left += 1;
        s.ended.push(session);
    }
    fn presence(&mut self, session: u64, p: &Presence) {
        let mut s = self.0.lock().unwrap();
        s.sent_to.push(session);
        s.sent.push(p.clone());
    }
    fn send(&mut self, session: u64, msg: ClientMsg) {
        let mut s = self.0.lock().unwrap();
        s.msgs_to.push(session);
        s.msgs.push(msg);
    }
    fn poll(&mut self) -> Vec<(u64, Event)> {
        let mut s = self.0.lock().unwrap();
        let session = s.session;
        std::mem::take(&mut s.events).into_iter().map(|e| (session, e)).collect()
    }
}

pub(crate) fn info(role: Role, me: u32) -> SessionInfo {
    SessionInfo {
        role,
        me,
        name: "Me".into(),
        color: "#0D99FF".into(),
        file_name: "Shared".into(),
        pages: vec![],
        link: "figcraft://join?host=127.0.0.1:47900&file=f&token=t".into(),
        url: "ws://127.0.0.1:47900/?file=f&token=t".into(),
        address: "127.0.0.1:47900".into(),
        network: false,
        access: crate::live::Access::Edit,
    }
}

pub(crate) fn peer(id: u32, name: &str) -> Peer {
    Peer { client_id: id, name: name.into(), color: "#9747FF".into() }
}

pub(crate) fn session() -> (Session, Arc<Mutex<Script>>) {
    let mut s = Session::new();
    let script = Arc::new(Mutex::new(Script::default()));
    s.live.install(Box::new(Mock(script.clone())));
    (s, script)
}

pub(crate) fn push(script: &Arc<Mutex<Script>>, events: Vec<Event>) {
    script.lock().unwrap().events.extend(events);
}

/// A session server for `doc` and the welcome it gives a first client (as `Event::Sync`).
pub(crate) fn serve(doc: &Document) -> (Server, u32, Event) {
    serve_as(doc, Access::Edit)
}

/// [`serve`] for a client that may do `access`.
pub(crate) fn serve_as(doc: &Document, access: Access) -> (Server, u32, Event) {
    let mut server = Server::new(doc.clone(), "", Limits::default());
    let ClientMsg::Hello(h) = Client::hello("f", "", "Me", "") else { panic!("no hello") };
    let (id, outs) = server.join_as(h, access).unwrap();
    let w = outs.into_iter().find_map(|o| if let Outgoing::To(_, w @ ServerMsg::Welcome { .. }) = o { Some(w) } else { None }).unwrap();
    (server, id, Event::Sync(w))
}

/// Feed what the session sent to the server; the server's answers for the session, in order.
pub(crate) fn relay(script: &Arc<Mutex<Script>>, server: &mut Server, from: u32) -> Vec<Event> {
    let msgs = std::mem::take(&mut script.lock().unwrap().msgs);
    let mut out = Vec::new();
    for m in msgs {
        for o in server.receive(from, m) {
            match o {
                Outgoing::To(id, m) | Outgoing::Others(id, m) if id == from && !matches!(m, ServerMsg::Presence { .. }) => out.push(Event::Sync(m)),
                Outgoing::All(m) => out.push(Event::Sync(m)),
                _ => {}
            }
        }
    }
    out
}

fn viewport(page: NodeId, x: f64, y: f64, w: f64, h: f64) -> Presence {
    Presence { viewport: Some((page, x, y, w, h)), ..Presence::default() }
}

#[test]
fn without_a_link_live_sessions_are_unavailable() {
    let mut s = Session::new();
    for (cmd, p) in [("collab.host", json!({})), ("collab.join", json!({"link": "figcraft://join?host=a:1&file=f&token=t"}))] {
        let e = s.execute(cmd, &p).unwrap_err();
        assert!(matches!(e, EngineError::Disabled(..)), "{cmd}: {e}");
    }
    assert_eq!(s.execute("collab.leave", &json!({})).unwrap()["ended"], false);
    let p = s.execute("collab.peers", &json!({})).unwrap();
    assert_eq!(p["available"], false);
    assert_eq!(p["status"], "off");
}

#[test]
fn hosting_and_joining_drive_the_link() {
    let (mut s, script) = session();
    let r = s.execute("collab.host", &json!({"bind": "lan", "port": 50000})).unwrap();
    assert_eq!(r["status"], "live");
    assert_eq!(r["role"], "host");
    assert_eq!(r["link"], "figcraft://join?host=127.0.0.1:47900&file=f&token=t");
    {
        let sc = script.lock().unwrap();
        assert_eq!(sc.hosted.len(), 1);
        assert_eq!(sc.hosted[0].port, Some(50000));
        assert_eq!(sc.hosted[0].bind, crate::live::Bind::Lan);
        assert_eq!(sc.hosted[0].doc.name, "Untitled");
        assert_eq!(sc.hosted[0].name, "", "the default name lets the host pick one");
    }
    for bad in [json!({"bind": "everywhere"}), json!({"port": 70000}), json!({"port": -1}), json!({"port": "x"})] {
        assert!(matches!(s.execute("collab.host", &bad), Err(EngineError::BadParams { .. })), "{bad}");
    }
    assert!(s.execute("collab.host", &json!({"bind": "10.1.2.3"})).is_ok());
    assert!(matches!(s.execute("collab.join", &json!({"link": "http://x"})), Err(EngineError::BadParams { .. })));
    let r = s.execute("collab.join", &json!({"link": " figcraft://join?host=a:1&file=f&token=t "})).unwrap();
    assert_eq!(r["status"], "connecting");
    assert_eq!(script.lock().unwrap().joined, ["figcraft://join?host=a:1&file=f&token=t"]);
    // The join is a session of its own: this file's goes on until it is stopped, and then the
    // join still on its way shows here.
    assert_eq!(s.live().role(), Some(Role::Host));
    assert_eq!(s.execute("collab.stop", &json!({})).unwrap()["ended"], true);
    assert!(script.lock().unwrap().left >= 1);
    assert_eq!(s.live().status, Status::Connecting);
    assert_eq!(s.execute("collab.leave", &json!({})).unwrap()["ended"], true);
    assert_eq!(s.live().status, Status::Off);
    assert_eq!(s.live.iter().count(), 0);
}

#[test]
fn events_build_who_is_here() {
    let (mut s, script) = session();
    s.execute("collab.join", &json!({"link": "figcraft://join?host=a:1&file=f&token=t"})).unwrap();
    push(&script, vec![Event::Welcome { info: info(Role::Guest, 2), peers: vec![peer(1, "Hana")] }, Event::PeerJoined(peer(3, "Bo"))]);
    assert!(s.live_poll());
    assert_eq!(s.live().status, Status::Live);
    assert_eq!(s.live().peers.len(), 2);
    assert_eq!(s.message.as_deref(), Some("Bo joined"));
    // Presence of someone not in the session is ignored; a leaver's presence goes with them.
    push(&script, vec![Event::Presence(9, viewport(NodeId(1), 0.0, 0.0, 10.0, 10.0)), Event::Presence(3, viewport(NodeId(1), 0.0, 0.0, 10.0, 10.0))]);
    s.live_poll();
    assert_eq!(s.live().presence.keys().copied().collect::<Vec<_>>(), [3]);
    push(&script, vec![Event::PeerLeft(3)]);
    s.live_poll();
    assert!(s.live().presence.is_empty());
    assert_eq!(s.message.as_deref(), Some("Bo left"));
    let p = s.live_summary();
    assert_eq!(p["peers"][0]["name"], "Hana");
    assert_eq!(p["fileName"], "Shared");
    // Reconnecting forgets who's here until the next welcome; failing ends it all.
    push(&script, vec![Event::Status(Status::Reconnecting { attempt: 1 })]);
    s.live_poll();
    assert!(s.live().peers.is_empty());
    assert_eq!(s.live_summary()["attempt"], 1);
    push(&script, vec![Event::Status(Status::Failed { reason: "Lost the connection".into() })]);
    s.live_poll();
    assert!(s.live().info.is_none());
    assert_eq!(s.live_summary()["reason"], "Lost the connection");
    assert_eq!(s.message.as_deref(), Some("Lost the connection"));
    assert!(!s.live_poll(), "nothing new");
}

#[test]
fn presence_is_sent_when_it_changes_and_only_when_live() {
    let (mut s, script) = session();
    s.live_sync();
    assert!(script.lock().unwrap().sent.is_empty(), "not in a session");
    assert!(s.live_mut().is_none(), "nothing to set a pointer in");
    s.execute("collab.host", &json!({})).unwrap();
    s.live_mut().unwrap().cursor = Some((10.0, 20.0));
    s.live_sync();
    s.live_sync();
    let sent = script.lock().unwrap().sent.clone();
    assert_eq!(sent.len(), 1, "unchanged presence isn't sent again");
    let page = s.page().unwrap();
    assert_eq!(sent[0].cursor, Some((page, 10.0, 20.0)));
    assert_eq!(sent[0].selection, Some(vec![]));
    let v = sent[0].viewport.unwrap();
    assert_eq!(v.0, page);
    assert!((v.3 - 1200.0).abs() < 1e-9 && (v.4 - 800.0).abs() < 1e-9, "the viewport at 100%: {v:?}");
    // `collab.cursor` moves it (agents); a non-finite pointer never goes out.
    s.execute("collab.cursor", &json!({"x": 1, "y": 2})).unwrap();
    assert_eq!(script.lock().unwrap().sent.last().unwrap().cursor, Some((page, 1.0, 2.0)));
    assert!(s.execute("collab.cursor", &json!({"x": 1})).is_err());
    s.live_mut().unwrap().cursor = Some((f64::NAN, 0.0));
    s.live_sync();
    assert_eq!(script.lock().unwrap().sent.last().unwrap().cursor, None);
    s.execute("collab.cursor", &json!({})).unwrap();
    assert_eq!(s.live().cursor, None);
}

#[test]
fn following_shows_their_viewport_and_page() {
    let (mut s, script) = session();
    s.execute("collab.host", &json!({})).unwrap();
    let second = s.execute("page.add", &json!({"name": "Two"})).unwrap()["id"].as_u64().map(NodeId).unwrap();
    let first = s.doc().unwrap().pages[0];
    s.execute("page.select", &json!({"id": first.0})).unwrap();
    push(&script, vec![Event::PeerJoined(peer(5, "Ada")), Event::Presence(5, viewport(second, 100.0, 200.0, 600.0, 400.0))]);
    s.live_poll();
    assert!(s.execute("collab.follow", &json!({"client": 6})).is_err());
    // Before the document syncs, their page ids aren't this document's: only the viewport counts.
    s.execute("collab.follow", &json!({"client": 5})).unwrap();
    assert_eq!(s.ds().unwrap().page, first);
    assert_eq!(s.ds().unwrap().view.cx, 400.0);
    let (_server, _, welcome) = serve(s.doc().unwrap());
    push(&script, vec![welcome]);
    s.live_poll();
    assert!(s.live_shared());
    s.execute("collab.follow", &json!({"client": 5})).unwrap();
    let ds = s.ds().unwrap();
    assert_eq!(ds.page, second, "their page, in a shared document");
    assert_eq!((ds.view.cx, ds.view.cy), (400.0, 400.0));
    assert!((ds.view.zoom - 2.0).abs() < 1e-9, "their area fills the viewport");
    // A page this document doesn't have: only the viewport.
    push(&script, vec![Event::Presence(5, viewport(NodeId(987_654), 0.0, 0.0, 1200.0, 800.0))]);
    s.live_poll();
    assert_eq!(s.ds().unwrap().page, second);
    assert_eq!(s.ds().unwrap().view.cx, 600.0);
    // Zooming yourself stops following.
    s.ds_mut().unwrap().view.zoom = 3.0;
    s.live_sync();
    assert_eq!(s.live().follow, None);
    // So does Escape, before anything else it does.
    s.execute("collab.follow", &json!({"client": 5})).unwrap();
    s.execute("select.all", &json!({})).ok();
    s.escape().unwrap();
    assert_eq!(s.live().follow, None);
    // And `collab.follow {}`.
    s.execute("collab.follow", &json!({"client": 5})).unwrap();
    assert_eq!(s.execute("collab.follow", &json!({})).unwrap()["follow"], serde_json::Value::Null);
}

#[test]
fn the_host_ending_resets_everything() {
    let (mut s, script) = session();
    s.execute("collab.join", &json!({"link": "figcraft://join?host=a:1&file=f&token=t"})).unwrap();
    push(&script, vec![Event::Welcome { info: info(Role::Guest, 2), peers: vec![peer(1, "Hana")] }]);
    s.live_poll();
    s.execute("collab.follow", &json!({"client": 1})).unwrap();
    push(&script, vec![Event::Ended { reason: "The host ended the session".into() }]);
    s.live_poll();
    assert_eq!(s.live().status, Status::Off);
    assert!(s.live().peers.is_empty() && s.live().follow.is_none() && s.live().info.is_none());
    assert_eq!(s.message.as_deref(), Some("The host ended the session"));
}

#[test]
fn the_host_removes_people_and_guests_cant() {
    let (mut s, script) = session();
    let remove = |s: &mut Session, client: serde_json::Value| s.execute("collab.removePerson", &json!({ "client": client }));
    // Not in a session: nobody to remove.
    assert!(matches!(remove(&mut s, json!(2)), Err(EngineError::BadParams { .. })));
    assert!(!crate::live::can_remove_people(&s));
    s.execute("collab.host", &json!({})).unwrap();
    let id = s.live().id;
    push(&script, vec![Event::PeerJoined(peer(2, "Ada")), Event::PeerJoined(peer(3, "Bo"))]);
    s.live_poll();
    assert!(crate::live::can_remove_people(&s));
    // Only someone who is here, by a client id.
    for bad in [json!(9), json!(1), json!(-2), json!("2"), json!(4_294_967_296u64)] {
        assert!(matches!(remove(&mut s, bad.clone()), Err(EngineError::BadParams { .. })), "{bad}");
    }
    let r = remove(&mut s, json!(2)).unwrap();
    assert_eq!((r["removed"].as_u64(), r["name"].as_str()), (Some(2), Some("Ada")));
    assert_eq!(script.lock().unwrap().removed, [(id, 2)]);
    assert_eq!(s.message.as_deref(), Some("Ada was removed from the session"));
    // The host's own session tells them she left.
    push(&script, vec![Event::PeerLeft(2)]);
    s.live_poll();
    assert_eq!(s.live().peers.keys().copied().collect::<Vec<_>>(), [3]);
    s.execute("collab.stop", &json!({})).unwrap();

    // A guest in someone else's session can't remove anyone.
    let (mut s, script) = session();
    s.execute("collab.join", &json!({"link": "figcraft://join?host=a:1&file=f&token=t"})).unwrap();
    push(&script, vec![Event::Welcome { info: info(Role::Guest, 2), peers: vec![peer(1, "Hana"), peer(3, "Bo")] }]);
    s.live_poll();
    assert!(!crate::live::can_remove_people(&s));
    let e = remove(&mut s, json!(3)).unwrap_err().to_string();
    assert!(e.contains("only the person hosting"), "{e}");
    assert!(script.lock().unwrap().removed.is_empty());
}

#[test]
fn collab_commands_survive_junk() {
    let junk = [
        json!(null),
        json!([1, 2]),
        json!({"port": 1e300, "bind": 5, "link": 7, "client": -3, "x": "NaN", "y": {}}),
        json!({"link": "", "client": 4_294_967_296u64}),
        json!({"text": 5, "on": "yes"}),
        json!({"text": "\u{0}\u{7f}".repeat(10_000), "on": null}),
    ];
    for cmd in crate::all_commands().iter().filter(|c| c.id.starts_with("collab.")) {
        for j in &junk {
            let (mut s, _) = session();
            if let Err(EngineError::Internal { msg, .. }) = s.execute(cmd.id, j) {
                panic!("{} panicked on {j}: {msg}", cmd.id);
            }
        }
    }
}

#[test]
fn edits_sync_through_the_server_and_remote_ones_skip_undo() {
    let (mut s, script) = session();
    s.execute("collab.host", &json!({})).unwrap();
    let (mut server, me, welcome) = serve(s.doc().unwrap());
    push(&script, vec![welcome]);
    s.live_poll();
    assert!(s.live().syncing() && s.live_shared());
    assert!(!s.ds().unwrap().is_dirty(), "starting a session doesn't dirty the file");
    // An edit made while hosting goes out as one batch, and the server confirms it.
    let r = s.execute("shape.rectangle", &json!({"x": 10, "y": 20, "width": 30, "height": 40})).unwrap();
    let rect = NodeId(r["id"].as_u64().unwrap());
    assert_eq!(rect.0 >> 32, u64::from(me), "new layers take ids in this client's space");
    s.live_sync();
    assert_eq!(script.lock().unwrap().msgs.len(), 1);
    s.live_sync();
    assert_eq!(script.lock().unwrap().msgs.len(), 1, "nothing new, nothing sent");
    let back = relay(&script, &mut server, me);
    assert!(server.document().contains(rect), "the server has the rectangle");
    push(&script, back);
    s.live_poll();
    // A gesture is sent once, when it ends.
    s.begin_interaction("Move").unwrap();
    s.preview(|d, _| {
        if let Some(n) = d.node_mut(rect) {
            n.opacity = 0.5;
        }
        Ok(())
    })
    .unwrap();
    s.live_sync();
    assert!(script.lock().unwrap().msgs.is_empty(), "nothing while dragging");
    s.commit_interaction().unwrap();
    s.live_sync();
    assert_eq!(script.lock().unwrap().msgs.len(), 1);
    push(&script, relay(&script, &mut server, me));
    s.live_poll();
    // Someone else recolours it: applied, with no undo step of its own.
    let undo_before = s.ds().unwrap().history.undo.len();
    let ClientMsg::Hello(h) = Client::hello("f", "", "Ada", "") else { panic!("no hello") };
    let (ada, outs) = server.join(h).unwrap();
    let Some(Outgoing::To(_, w)) = outs.into_iter().find(|o| matches!(o, Outgoing::To(_, ServerMsg::Welcome { .. }))) else { panic!("no welcome") };
    let mut other = Client::join(w).unwrap();
    let mut doc = other.document().clone();
    doc.node_mut(rect).unwrap().opacity = 0.25;
    let batch = other.edit(&doc).unwrap().unwrap();
    let mut remote = Vec::new();
    for o in server.receive(ada, batch) {
        if let Outgoing::All(m) = o {
            remote.push(Event::Sync(m));
        }
    }
    push(&script, remote);
    assert!(s.live_poll());
    assert_eq!(s.doc().unwrap().node(rect).unwrap().opacity, 0.25);
    assert_eq!(s.ds().unwrap().history.undo.len(), undo_before, "remote edits never enter your undo history");
    assert_eq!(s.doc().unwrap().node(rect), server.document().node(rect), "the same as the server");
    // While a gesture shows, remote batches wait for it to end.
    let mut doc = other.document().clone();
    doc.node_mut(rect).unwrap().name = "Card".into();
    let batch = other.edit(&doc).unwrap().unwrap();
    let remote: Vec<Event> =
        server.receive(ada, batch).into_iter().filter_map(|o| if let Outgoing::All(m) = o { Some(Event::Sync(m)) } else { None }).collect();
    s.begin_interaction("Move").unwrap();
    push(&script, remote);
    s.live_poll();
    assert_ne!(s.doc().unwrap().node(rect).unwrap().name, "Card");
    s.cancel_interaction().unwrap();
    s.live_sync();
    assert_eq!(s.doc().unwrap().node(rect).unwrap().name, "Card");
    // Closing the session's file leaves the session.
    let uid = s.live().doc_uid.unwrap();
    s.execute("file.new", &json!({})).unwrap();
    let i = s.docs.iter().position(|d| d.uid == uid).unwrap();
    s.execute("file.close", &json!({"index": i, "force": true})).unwrap();
    s.live_sync();
    assert_eq!(s.live_of(uid).status, Status::Off);
    assert_eq!(s.live.iter().count(), 0);
}

#[test]
fn a_guest_opens_the_hosts_file_as_a_new_tab() {
    let (mut s, script) = session();
    let mut host_doc = Document::new("Team file");
    host_doc.repair();
    let (mut server, _, _) = serve(&host_doc);
    let ClientMsg::Hello(h) = Client::hello("f", "", "Ada", "") else { panic!("no hello") };
    let (ada, outs) = server.join(h).unwrap();
    let Some(Outgoing::To(_, w)) = outs.into_iter().find(|o| matches!(o, Outgoing::To(_, ServerMsg::Welcome { .. }))) else { panic!("no welcome") };
    s.execute("collab.join", &json!({"link": "figcraft://join?host=a:1&file=f&token=t"})).unwrap();
    push(&script, vec![Event::Welcome { info: info(Role::Guest, ada), peers: vec![peer(1, "Hana")] }, Event::Sync(w)]);
    s.live_poll();
    assert_eq!(s.docs.len(), 2, "your own file stays open");
    assert_eq!(s.doc().unwrap().name, "Team file");
    assert_eq!(s.live_summary()["fileName"], "Team file");
    assert_eq!(s.message.as_deref(), Some("Joined Team file"));
    assert!(s.live_shared());
    // Your edit reaches the server.
    let r = s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s.live_sync();
    let back = relay(&script, &mut server, ada);
    assert!(server.document().contains(NodeId(r["id"].as_u64().unwrap())));
    push(&script, back);
    s.live_poll();
    // Leaving keeps the file as a local copy.
    s.execute("collab.leave", &json!({})).unwrap();
    assert_eq!(s.docs.len(), 2);
    assert!(s.doc().unwrap().contains(NodeId(r["id"].as_u64().unwrap())));
}

#[test]
fn hosting_commands_check_what_they_are_given() {
    let mut s = Session::new();
    // The publish server preference: a WebSocket address without a key, or nothing.
    assert_eq!(s.execute("pref.publishServer", &json!({"url": " wss://figcraft.example.com/ "})).unwrap()["url"], "wss://figcraft.example.com/");
    for bad in ["https://figcraft.example.com/", "wss://host/?key=secret", "wss://a b/"] {
        assert!(s.execute("pref.publishServer", &json!({"url": bad})).is_err(), "{bad}");
    }
    assert_eq!(s.prefs.publish_server, "wss://figcraft.example.com/", "a refused address changes nothing");
    s.execute("pref.publishServer", &json!({"url": ""})).unwrap();
    assert!(s.prefs.publish_server.is_empty());
    // No network here: the server can't be asked, nothing can be reported.
    let e = s.execute("collab.serverInfo", &json!({"server": "wss://figcraft.example.com/"})).unwrap_err().to_string();
    assert!(e.contains("desktop app or the MCP server"), "{e}");
    let e = s.execute("collab.manageFile", &json!({"server": "wss://x/", "file": "f", "owner": "o", "action": "explode"})).unwrap_err().to_string();
    assert!(e.contains("`action` must be"), "{e}");
    let e = s.execute("collab.report", &json!({"reason": "Spam"})).unwrap_err().to_string();
    assert!(e.contains("join a file"), "{e}");
    assert!(s.execute("collab.report", &json!({"reason": " \n "})).unwrap_err().to_string().contains("few words"));
}

#[test]
fn parse_color_rejects_signs_and_non_hex() {
    use crate::live::parse_color;
    assert_eq!(parse_color("#+1+2+3"), None);
    assert_eq!(parse_color("#-1-2-3"), None);
    assert_eq!(parse_color("#0D99FF"), Some((13, 153, 255)));
}
