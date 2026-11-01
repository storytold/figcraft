//! Recovery of your online files never loses an edit. The copy of a document that was one of them
//! when a run ended without closing has a note beside it (the server, the file, who the app was
//! there and the server's last change it had; never a token). The next start joins the file again
//! as its owner and keeps the copy until the outcome is known: the copy is ahead of the server
//! (its changes are applied on top and sent, as the client the app was), the same (it goes), both
//! changed (it opens beside the file as a recovered copy), or the server can't be reached (it
//! opens as any other copy). Without the record the copy opens as any other.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use figcraft_engine::Session;
use figcraft_engine::figcraft_collab::message::{ClientMsg, ServerMsg};
use figcraft_engine::figcraft_collab::{Client, Limits, Outgoing, Server};
use figcraft_engine::live::{Access, Event, HostOptions, LiveLink, Presence, PublishOptions, Published, Resume, Role, SessionInfo, Status};
use serde_json::json;

use crate::recovery::{Recovery, recover, recover_all};

const SERVER: &str = "wss://share.example/";
const OWNER: &str = "OwnerSecretOwnerSecret";
const EDIT: &str = "figcraft://join?server=wss://share.example/&file=f1&token=EditSecretEditSecret00";

/// What the link was asked, what the app sent, and the events handed out next (as the session
/// started last).
#[derive(Default)]
struct Script {
    joined: Vec<(String, Option<Resume>)>,
    sent: Vec<ClientMsg>,
    events: Vec<Event>,
    session: u64,
}

struct Link(Arc<Mutex<Script>>);

impl LiveLink for Link {
    fn host(&mut self, _: u64, _: HostOptions) -> Result<SessionInfo, String> {
        Err("not in this test".into())
    }
    fn join(&mut self, session: u64, link: &str, name: &str) -> Result<(), String> {
        self.join_resuming(session, link, name, None)
    }
    fn join_resuming(&mut self, session: u64, link: &str, _: &str, resume: Option<Resume>) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        s.session = session;
        s.joined.push((link.to_string(), resume));
        Ok(())
    }
    fn publish(&mut self, session: u64, _: PublishOptions) -> Result<(), String> {
        self.0.lock().unwrap().session = session;
        Ok(())
    }
    fn leave(&mut self, _: u64) {}
    fn presence(&mut self, _: u64, _: &Presence) {}
    fn send(&mut self, _: u64, msg: ClientMsg) {
        self.0.lock().unwrap().sent.push(msg);
    }
    fn poll(&mut self) -> Vec<(u64, Event)> {
        let mut s = self.0.lock().unwrap();
        let session = s.session;
        std::mem::take(&mut s.events).into_iter().map(|e| (session, e)).collect()
    }
}

type Shared = Arc<Mutex<Script>>;

fn session() -> (Session, Shared) {
    let mut s = Session::new();
    let script = Shared::default();
    s.live.install(Box::new(Link(script.clone())));
    (s, script)
}

fn root(name: &str) -> PathBuf {
    let r = std::env::temp_dir().join(format!("figcraft-recovery-online-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&r);
    r
}

fn info() -> SessionInfo {
    SessionInfo {
        role: Role::Guest,
        me: 1,
        name: "Me".into(),
        color: "#0D99FF".into(),
        file_name: "Homepage".into(),
        pages: vec![],
        link: EDIT.into(),
        url: String::new(),
        address: SERVER.into(),
        network: true,
        access: Access::Edit,
    }
}

/// Someone joins `server` (as the client `resume` names, if it knows it): their id and welcome.
fn join(server: &mut Server, resume: Option<Resume>) -> (u32, ServerMsg) {
    let ClientMsg::Hello(mut hello) = Client::hello("f1", "", "Me", "") else { panic!("no hello") };
    hello.resume = resume;
    let (id, outs) = server.join(hello).unwrap();
    let welcome = outs.into_iter().find_map(|o| if let Outgoing::To(_, w @ ServerMsg::Welcome { .. }) = o { Some(w) } else { None }).unwrap();
    (id, welcome)
}

/// What the app sent since the last call reaches the server; whether every batch was accepted.
/// The server's confirmations wait for the app's next poll.
fn deliver(script: &Shared, server: &mut Server, from: u32) -> bool {
    let sent = std::mem::take(&mut script.lock().unwrap().sent);
    let mut accepted = true;
    for m in sent {
        for o in server.receive(from, m) {
            match o {
                Outgoing::To(_, ServerMsg::Reject { .. }) => accepted = false,
                Outgoing::All(m @ ServerMsg::Changes { .. }) => script.lock().unwrap().events.push(Event::Sync(m)),
                _ => {}
            }
        }
    }
    accepted
}

fn layers(doc: &figcraft_engine::doc::Document) -> usize {
    doc.node(doc.pages[0]).map_or(0, |p| p.children.len())
}

/// A run whose open file was shared online: the app, what it sent, the server and the app's id
/// there. One rectangle is on the server.
fn shared() -> (Session, Shared, Server, u32) {
    let (mut s, script) = session();
    s.execute("pref.publishServer", &json!({"url": SERVER})).unwrap();
    s.execute("file.rename", &json!({"name": "Homepage"})).unwrap();
    s.execute("collab.share", &json!({})).unwrap();
    let published = Published {
        server: "share.example".into(),
        file: "f1".into(),
        name: "Homepage".into(),
        edit: EDIT.into(),
        view: String::new(),
        owner: Some(OWNER.into()),
        expires: None,
    };
    let mut server = Server::new(s.doc().unwrap().clone(), "", Limits::default());
    let (me, welcome) = join(&mut server, None);
    script.lock().unwrap().events.extend([Event::Published(published), Event::Welcome { info: info(), peers: vec![] }, Event::Sync(welcome)]);
    s.live_poll();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    assert!(deliver(&script, &mut server, me));
    s.live_poll();
    assert_eq!(layers(server.document()), 1);
    (s, script, server, me)
}

/// The run ends without closing, its copies written; the folder it leaves (as another process's).
fn crash(s: &Session, root: &Path) -> PathBuf {
    let mut r = Recovery::start(root).unwrap();
    r.write(s, true);
    drop(r);
    let left = root.join("crashed");
    std::fs::rename(root.join(std::process::id().to_string()), &left).unwrap();
    left
}

fn copies(dir: &Path, ext: &str) -> usize {
    std::fs::read_dir(dir).map_or(0, |d| d.flatten().filter(|f| f.path().extension().is_some_and(|e| e == ext)).count())
}

/// The next run: only the preferences survive. Its copies of online files wait for their files.
fn restart(crashed: &Session, root: &Path) -> (Session, Shared, Recovery, usize) {
    let (mut next, script) = session();
    next.prefs = serde_json::from_value(serde_json::to_value(&crashed.prefs).unwrap()).unwrap();
    let recovered = recover_all(root, &mut next);
    let mut r = Recovery::start(root).unwrap();
    r.adopt(recovered.waiting);
    (next, script, r, recovered.copies)
}

/// The file arrives in the next run from `server`, as the join asked.
fn arrive(next: &mut Session, script: &Shared, server: &mut Server) -> u32 {
    let resume = script.lock().unwrap().joined.last().and_then(|j| j.1);
    let (id, welcome) = join(server, resume);
    script.lock().unwrap().events.extend([Event::Welcome { info: info(), peers: vec![] }, Event::Sync(welcome)]);
    next.live_poll();
    id
}

#[test]
fn a_copy_ahead_of_the_server_has_its_edits_applied_and_sent() {
    let root = root("ahead");
    let (mut s, script, mut server, me) = shared();
    // An edit the server never got (the connection was down, or the app died first).
    s.execute("shape.ellipse", &json!({"x": 30, "y": 0, "width": 10, "height": 10})).unwrap();
    script.lock().unwrap().sent.clear();
    let left = crash(&s, &root);
    let note = std::fs::read_to_string(left.join(format!("{}.online", s.docs[0].uid))).unwrap();
    let at = s.online_of(s.docs[0].uid).unwrap().2;
    assert_eq!(note, format!("{SERVER}\nf1\n{} {} {} {}", at.session, at.client_id, at.seq, at.secret), "which file, who the app was, what it had");
    assert!(!note.contains(OWNER) && !note.contains("token="), "never a secret");
    let (mut next, script, mut r, opened) = restart(&s, &root);
    assert_eq!(opened, 0, "nothing comes back as a plain copy");
    {
        let sc = script.lock().unwrap();
        let [(link, resume)] = sc.joined.as_slice() else { panic!("joined {}", sc.joined.len()) };
        assert_eq!(link, EDIT, "joined again as its owner");
        assert_eq!(*resume, Some(Resume { seq: u64::MAX, ..at }), "as the client it was, asking for the whole file");
    }
    // The copy stays until the outcome is known.
    assert!(r.tick(&next, 0.0).is_some(), "it looks again while the file is on its way");
    assert_eq!((copies(&left, "figcraft"), copies(&left, "online")), (1, 1));
    // The file arrives: the server lacks the ellipse; it goes on top as one edit and is sent.
    let id = arrive(&mut next, &script, &mut server);
    assert_eq!(id, me, "the server gave the id back");
    assert_eq!(next.message.take().as_deref(), Some("Recovered your changes to Homepage that hadn't reached the server"));
    assert_eq!((next.docs.len(), layers(&next.docs[1].doc)), (2, 2), "in the file's own tab, no copy beside it");
    assert!(deliver(&script, &mut server, id), "the server takes it (new layers are that client's to create)");
    assert_eq!(layers(server.document()), 2, "everyone gets the edit");
    assert_eq!(next.execute("collab.peers", &json!({})).unwrap()["status"], "live");
    // Only now the old copy goes, after this run has copied what it became.
    r.tick(&next, 1.0);
    assert!(!left.exists(), "the crashed run's folder is gone");
    let mine = root.join(std::process::id().to_string());
    assert_eq!((copies(&mine, "figcraft"), copies(&mine, "online")), (1, 1), "covered by this run's own copy");
    drop(r);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_online_copy_stays_while_what_it_became_cant_be_copied() {
    let root = root("ahead-blocked");
    let (mut s, script, mut server, _) = shared();
    s.execute("shape.ellipse", &json!({"x": 30, "y": 0, "width": 10, "height": 10})).unwrap();
    script.lock().unwrap().sent.clear();
    let left = crash(&s, &root);
    let (mut next, script, mut r, _) = restart(&s, &root);
    arrive(&mut next, &script, &mut server);
    assert!(next.docs[1].is_dirty(), "the copy's edit is on top, not saved anywhere yet");
    // Something is in the way of this run's copy: writing it fails.
    let mine = root.join(std::process::id().to_string()).join(format!("{}.figcraft", next.docs[1].uid));
    std::fs::create_dir_all(&mine).unwrap();
    r.tick(&next, 1.0);
    assert_eq!((copies(&left, "figcraft"), copies(&left, "online")), (1, 1), "kept: the work is nowhere else on disk");
    // Once it can be written, the old copy goes.
    std::fs::remove_dir(&mine).unwrap();
    next.execute("shape.rectangle", &json!({"x": 50, "y": 0, "width": 4, "height": 4})).unwrap();
    r.tick(&next, 2.0);
    assert!(!left.exists());
    drop(r);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_copy_the_server_already_has_is_discarded() {
    let root = root("equal");
    let (mut s, script, mut server, me) = shared();
    // Sent and accepted, but the app died before it heard so.
    s.execute("shape.ellipse", &json!({"x": 30, "y": 0, "width": 10, "height": 10})).unwrap();
    assert!(deliver(&script, &mut server, me));
    let left = crash(&s, &root);
    let (mut next, script, mut r, _) = restart(&s, &root);
    assert_eq!(copies(&left, "figcraft"), 1);
    arrive(&mut next, &script, &mut server);
    assert_eq!((next.docs.len(), layers(&next.docs[1].doc)), (2, 2), "the file as the server has it");
    assert!(script.lock().unwrap().sent.iter().all(|m| !matches!(m, ClientMsg::Changes { .. })), "nothing to send");
    assert!(!next.docs[1].is_dirty());
    r.tick(&next, 1.0);
    assert!(!left.exists());
    drop(r);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_copy_that_cant_be_merged_opens_beside_the_file() {
    let root = root("conflict");
    let (mut s, script, mut server, _) = shared();
    s.execute("shape.ellipse", &json!({"x": 30, "y": 0, "width": 10, "height": 10})).unwrap();
    script.lock().unwrap().sent.clear();
    let left = crash(&s, &root);
    // Meanwhile someone else changes the file on the server.
    let (other, welcome) = join(&mut server, None);
    let mut them = Client::join(welcome).unwrap();
    let mut doc = them.document().clone();
    let page = doc.pages[0];
    doc.create(page, figcraft_engine::doc::NodeKind::Rectangle, "Theirs", 50.0, 50.0, 5.0, 5.0).unwrap();
    let batch = them.edit(&doc).unwrap().unwrap();
    assert!(!server.receive(other, batch).iter().any(|o| matches!(o, Outgoing::To(_, ServerMsg::Reject { .. }))));
    let (mut next, script, mut r, _) = restart(&s, &root);
    arrive(&mut next, &script, &mut server);
    // The server's file is untouched; the copy is a tab of its own, named as what it is.
    let names: Vec<&str> = next.docs.iter().map(|d| d.doc.name.as_str()).collect();
    assert_eq!(names, ["Untitled", "Homepage", "Homepage (recovered copy)"]);
    assert_eq!(next.active, 1, "the file stays in front");
    assert_eq!(layers(&next.docs[1].doc), 2, "the rectangle and theirs");
    assert_eq!(layers(&next.docs[2].doc), 2, "the rectangle and the ellipse that never got there");
    assert!(next.docs[2].is_dirty() && next.docs[2].path.is_none());
    assert_eq!(
        next.message.take().as_deref(),
        Some("Homepage changed on the server before your last changes reached it: they are open beside it as “Homepage (recovered copy)”")
    );
    assert!(script.lock().unwrap().sent.iter().all(|m| !matches!(m, ClientMsg::Changes { .. })), "nothing is forced onto the server");
    r.tick(&next, 1.0);
    assert!(!left.exists());
    assert_eq!(copies(&root.join(std::process::id().to_string()), "figcraft"), 1, "the recovered tab has this run's copy");
    drop(r);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_copy_whose_server_cant_be_reached_opens_as_any_other() {
    let root = root("offline");
    let (mut s, script, _, _) = shared();
    s.execute("shape.ellipse", &json!({"x": 30, "y": 0, "width": 10, "height": 10})).unwrap();
    script.lock().unwrap().sent.clear();
    let left = crash(&s, &root);
    let (mut next, script, mut r, _) = restart(&s, &root);
    r.tick(&next, 0.0);
    assert_eq!(copies(&left, "figcraft"), 1, "kept while the join is on its way");
    script.lock().unwrap().events.push(Event::Status(Status::Failed { reason: "Couldn't reach the server (timed out)".into() }));
    next.live_poll();
    let ds = next.docs.last().unwrap();
    assert_eq!((ds.doc.name.as_str(), layers(&ds.doc), ds.is_dirty(), ds.path.is_none()), ("Homepage", 2, true, true));
    assert_eq!(next.active, next.docs.len() - 1);
    assert!(next.message.take().unwrap_or_default().contains("your recovered copy is open"));
    assert_eq!(next.prefs.online_files.len(), 1, "the record stays: the file is still yours on the server");
    r.tick(&next, 1.0);
    assert!(!left.exists());
    assert_eq!(copies(&root.join(std::process::id().to_string()), "figcraft"), 1);
    drop(r);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_server_that_forgot_the_client_gets_no_layers_it_would_refuse() {
    let root = root("forgot");
    let (mut s, script, _, _) = shared();
    s.execute("shape.ellipse", &json!({"x": 30, "y": 0, "width": 10, "height": 10})).unwrap();
    script.lock().unwrap().sent.clear();
    crash(&s, &root);
    // The server lost its session (restored from a backup): the same file, another session, so
    // the app joins as someone new and can't create the layers of who it was.
    let mut restored = Server::new(Session::new().doc().unwrap().clone(), "", Limits::default());
    let (mut next, script, r, _) = restart(&s, &root);
    arrive(&mut next, &script, &mut restored);
    assert_eq!(next.docs.last().map(|d| d.doc.name.as_str()), Some("Homepage (recovered copy)"));
    assert!(script.lock().unwrap().sent.iter().all(|m| !matches!(m, ClientMsg::Changes { .. })));
    drop(r);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn without_its_record_the_copy_opens_as_any_other() {
    let root = root("fallback");
    let (s, _, _, _) = shared();
    crash(&s, &root);
    // The record was forgotten (or the preferences lost): the copy is what there is.
    let (mut next, script) = session();
    next.docs.clear();
    assert_eq!(recover(&root, &mut next), 1);
    assert!(script.lock().unwrap().joined.is_empty());
    assert!(next.docs.first().is_some_and(|d| d.is_dirty() && d.path.is_none()));
    assert!(std::fs::read_dir(&root).unwrap().next().is_some(), "the copy stays until a run has its own");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_note_follows_its_copy_and_goes_when_the_file_leaves_its_session() {
    let root = root("note");
    let (mut s, _, _, _) = shared();
    let mut r = Recovery::start(&root).unwrap();
    r.write(&s, true);
    let dir = root.join(std::process::id().to_string());
    assert_eq!((copies(&dir, "figcraft"), copies(&dir, "online")), (1, 1));
    // Left: the copy is an ordinary one from the next write on.
    s.execute("collab.leave", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap();
    r.write(&s, true);
    assert_eq!((copies(&dir, "figcraft"), copies(&dir, "online")), (1, 0));
    r.finish();
    assert!(std::fs::read_dir(&root).unwrap().next().is_none());
    let _ = std::fs::remove_dir_all(&root);
}
