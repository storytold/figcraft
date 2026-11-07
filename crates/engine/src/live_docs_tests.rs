//! Live sessions are per document, against the scripted link: every open document has its own
//! session or none, several run at once, the `collab.*` commands and `collab.peers` are the
//! active document's, a join shows where it was started until its file opens in its own tab, and
//! closing a document ends its session only.

use std::sync::{Arc, Mutex};

use figcraft_collab::message::{ClientMsg, ServerMsg};
use figcraft_collab::{Client, Outgoing, Server};
use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::live::{Event, PUBLISHING, Presence, Role, Signal, Status};
use crate::live_tests::{Script, info, peer, push, relay, serve, session};
use crate::{EngineError, Session};

const LINK: &str = "figcraft://join?host=a:1&file=f&token=t";

/// Hand `events` to the session as link session `id`'s.
fn feed(s: &mut Session, script: &Arc<Mutex<Script>>, id: u64, events: Vec<Event>) {
    script.lock().unwrap().session = id;
    push(script, events);
    s.live_poll();
}

fn uid(s: &Session) -> u64 {
    s.ds().unwrap().uid
}

fn id_of(v: &Value) -> u64 {
    v["session"].as_u64().unwrap()
}

fn run(s: &mut Session, cmd: &str, params: Value) -> Value {
    s.execute(cmd, &params).unwrap()
}

/// Host the active document and start syncing it: its uid, its session id, and its session's
/// server with your client id there.
fn host(s: &mut Session, script: &Arc<Mutex<Script>>) -> (u64, u64, Server, u32) {
    let id = id_of(&run(s, "collab.host", json!({})));
    let (server, me, welcome) = serve(s.doc().unwrap());
    feed(s, script, id, vec![welcome]);
    assert!(s.live_shared());
    (uid(s), id, server, me)
}

/// Two documents, each hosted and syncing (the second in front): their uids, session ids and
/// the scripted link.
fn two_hosted() -> (Session, Arc<Mutex<Script>>, [u64; 2], [u64; 2]) {
    let (mut s, script) = session();
    let (ua, a, ..) = host(&mut s, &script);
    run(&mut s, "file.new", json!({"name": "Second"}));
    let (ub, b, ..) = host(&mut s, &script);
    (s, script, [ua, ub], [a, b])
}

#[test]
fn each_document_has_its_own_session() {
    let (mut s, script) = session();
    let a = run(&mut s, "collab.host", json!({}));
    let ua = uid(&s);
    // A new document isn't in the first one's session.
    run(&mut s, "file.new", json!({}));
    let off = run(&mut s, "collab.peers", json!({}));
    for key in ["session", "role", "link", "url", "address", "fileName", "document", "me"] {
        assert_eq!(off[key], Value::Null, "{key}: {off}");
    }
    assert_eq!((&off["status"], &off["syncing"], &off["readOnly"]), (&json!("off"), &json!(false), &json!(false)));
    assert_eq!(off["peers"], json!([]));
    assert_eq!(off["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(
        (&off["sessions"][0]["session"], &off["sessions"][0]["document"], &off["sessions"][0]["link"]),
        (&a["session"], &json!(0), &a["link"])
    );
    assert_eq!((s.live().id, s.live().status.clone()), (0, Status::Off));
    assert!(s.live_mut().is_none() && s.active_read_only().is_none());
    assert_eq!(run(&mut s, "collab.stop", json!({}))["ended"], false, "nothing to stop here");
    assert_eq!(s.live_of(ua).status, Status::Live, "and the other file's session isn't stopped");
    hosting_a_second_session(&mut s, &script, &a, ua);
}

fn hosting_a_second_session(s: &mut Session, script: &Arc<Mutex<Script>>, a: &Value, ua: u64) {
    // Hosting it too starts a second session; the first goes on.
    let b = run(s, "collab.host", json!({}));
    assert_ne!(a["session"], b["session"]);
    assert_ne!(a["link"], b["link"]);
    assert_eq!((b["document"].clone(), b["sessions"].as_array().unwrap().len()), (json!(1), 2));
    {
        let sc = script.lock().unwrap();
        assert_eq!((sc.hosted.len(), sc.left), (2, 0));
    }
    people_come_and_go(s, script, a, &b, ua);
}

fn people_come_and_go(s: &mut Session, script: &Arc<Mutex<Script>>, a: &Value, b: &Value, ua: u64) {
    let ub = uid(s);
    // People come and go in their own session.
    let (a_id, b_id) = (id_of(a), id_of(b));
    feed(s, script, a_id, vec![Event::PeerJoined(peer(5, "Ada"))]);
    feed(s, script, b_id, vec![Event::PeerJoined(peer(6, "Bo")), Event::PeerJoined(peer(7, "Cy"))]);
    assert_eq!((s.live_of(ua).peers.len(), s.live_of(ub).peers.len()), (1, 2));
    let names = |v: &Value| v["peers"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let p = s.live_summary();
    assert_eq!((names(&p), &p["link"]), (vec!["Bo".to_string(), "Cy".to_string()], &b["link"]));
    assert_eq!((&p["sessions"][0]["peers"], &p["sessions"][1]["peers"]), (&json!(1), &json!(2)));
    // The file in front decides what `collab.peers` and the commands are about.
    run(s, "doc.switch", json!({"index": 0}));
    let p = run(s, "collab.peers", json!({}));
    assert_eq!((names(&p), &p["link"], &p["document"], &p["session"]), (vec!["Ada".to_string()], &a["link"], &json!(0), &a["session"]));
    assert_eq!(run(s, "collab.follow", json!({"client": 5}))["follow"], 5);
    assert!(s.execute("collab.follow", &json!({"client": 6})).is_err(), "Bo is in the other file's session");
    run(s, "doc.switch", json!({"index": 1}));
    assert!(s.execute("collab.follow", &json!({"client": 5})).is_err());
    assert_eq!((s.live().follow, s.live_of(ua).follow), (None, Some(5)));
    // Stopping ends this file's session only.
    assert_eq!(run(s, "collab.stop", json!({}))["ended"], true);
    assert_eq!(script.lock().unwrap().ended, [b_id]);
    assert_eq!((s.live().status.clone(), s.live_of(ua).status.clone()), (Status::Off, Status::Live));
    // Someone ending or leaving the other one doesn't bring this one back.
    feed(s, script, b_id, vec![Event::PeerJoined(peer(8, "Late"))]);
    assert_eq!((s.live.iter().count(), s.live_of(ua).peers.len()), (1, 1), "what a session still sent after it ended is dropped");
    // Closing a file ends its session.
    run(s, "file.close", json!({"index": 0, "force": true}));
    s.live_sync();
    assert_eq!(script.lock().unwrap().ended, [b_id, a_id]);
    assert_eq!(s.live.iter().count(), 0);
    assert_eq!(s.message.as_deref(), Some("The live session's file was closed, so you left the session"));
}

#[test]
fn closing_one_document_leaves_the_others_session_running() {
    let (mut s, script, [ua, ub], [a, b]) = two_hosted();
    run(&mut s, "file.close", json!({"index": 1, "force": true}));
    s.live_sync();
    assert_eq!(script.lock().unwrap().ended, [b]);
    assert_eq!((s.live_of(ua).status.clone(), s.live_of(ub).status.clone()), (Status::Live, Status::Off));
    assert_eq!((uid(&s), s.live().id), (ua, a), "the file left in front shows its own session");
    // It still syncs.
    script.lock().unwrap().msgs_to.clear();
    run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}));
    s.live_sync();
    assert_eq!(script.lock().unwrap().msgs_to, [a]);
}

#[test]
fn edits_and_presence_stay_in_their_session() {
    let (mut s, script) = session();
    let (ua, a, ..) = host(&mut s, &script);
    run(&mut s, "file.new", json!({}));
    let (ub, b, mut server, me) = host(&mut s, &script);
    s.live_sync();
    {
        let mut sc = script.lock().unwrap();
        sc.msgs_to.clear();
        sc.sent_to.clear();
        sc.sent.clear();
    }
    // An edit of the file in front goes to its session, as that file's own undo step.
    let rect = NodeId(run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [rect.0]}));
    s.live_sync();
    assert_eq!(script.lock().unwrap().msgs_to, [b]);
    assert_eq!(s.undo_depth().0, 1);
    assert!(!s.docs.iter().find(|d| d.uid == ua).unwrap().doc.contains(rect));
    let confirmed = relay(&script, &mut server, me);
    feed(&mut s, &script, b, confirmed);
    // Your selection and view go to that session; the other one isn't told anything new.
    {
        let sc = script.lock().unwrap();
        assert_eq!(sc.sent_to, [b]);
        assert_eq!(sc.sent[0].selection, Some(vec![rect]));
    }
    // With the other file in front: its session gets your presence there, this one gets none
    // (no cursor, selection or view of a file you don't look at).
    run(&mut s, "doc.switch", json!({"index": 0}));
    assert_eq!(s.undo_depth().0, 0, "undo is each file's own");
    run(&mut s, "collab.cursor", json!({"x": 3, "y": 4}));
    {
        let sc = script.lock().unwrap();
        let last = |id: u64| sc.sent_to.iter().zip(&sc.sent).rev().find(|(to, _)| **to == id).map(|(_, p)| p.clone()).unwrap();
        assert_eq!(last(b), Presence::default());
        assert_eq!((last(a).cursor.map(|c| (c.1, c.2)), last(a).selection), (Some((3.0, 4.0)), Some(vec![])));
    }
    assert_eq!((s.live().cursor, s.live_of(ub).cursor), (Some((3.0, 4.0)), None));
    // What others do in the file behind arrives there, and only there.
    let ClientMsg::Hello(h) = Client::hello("f", "", "Ada", "") else { panic!("no hello") };
    let (ada, outs) = server.join(h).unwrap();
    let w = outs.into_iter().find_map(|o| if let Outgoing::To(_, w @ ServerMsg::Welcome { .. }) = o { Some(w) } else { None }).unwrap();
    let mut other = Client::join(w).unwrap();
    let mut doc = other.document().clone();
    doc.node_mut(rect).unwrap().name = "Theirs".into();
    let batch = other.edit(&doc).unwrap().unwrap();
    let remote: Vec<Event> =
        server.receive(ada, batch).into_iter().filter_map(|o| if let Outgoing::All(m) = o { Some(Event::Sync(m)) } else { None }).collect();
    let before = s.ds().unwrap().revision;
    feed(&mut s, &script, b, remote);
    assert_eq!(s.docs.iter().find(|d| d.uid == ub).unwrap().doc.node(rect).unwrap().name, "Theirs");
    assert_eq!(s.ds().unwrap().revision, before, "the file in front didn't change");
    assert_eq!(uid(&s), ua, "and stays in front");
}

#[test]
fn a_join_shows_where_it_started_until_its_file_opens() {
    let (mut s, script) = session();
    let first = uid(&s);
    let r = run(&mut s, "collab.join", json!({"link": LINK}));
    let join = id_of(&r);
    assert_eq!((&r["status"], &r["document"]), (&json!("connecting"), &Value::Null));
    assert_eq!(s.live().status, Status::Connecting);
    // Another file doesn't show it; the list of sessions does.
    run(&mut s, "file.new", json!({}));
    let p = s.live_summary();
    assert_eq!((&p["status"], &p["sessions"][0]["status"], &p["sessions"][0]["document"]), (&json!("off"), &json!("connecting"), &Value::Null));
    // Its file opens as a tab of its own, in front, with the session.
    let mut theirs = figcraft_doc::Document::new("Team file");
    theirs.repair();
    let (_, me, welcome) = serve(&theirs);
    feed(&mut s, &script, join, vec![Event::Welcome { info: info(Role::Guest, me), peers: vec![peer(1, "Hana")] }, welcome]);
    assert_eq!((s.docs.len(), s.active), (3, 2));
    assert_eq!((s.doc().unwrap().name.as_str(), s.live().id, s.live().syncing()), ("Team file", join, true));
    assert_eq!(s.live_summary()["document"], 2);
    assert_eq!(s.live_of(first).status, Status::Off, "the tab it was started from is in no session");
    assert_eq!(s.live.iter().count(), 1);
}

#[test]
fn joining_from_a_file_in_a_session_leaves_that_session_alone() {
    let (mut s, script) = session();
    let host = id_of(&run(&mut s, "collab.host", json!({})));
    let first = uid(&s);
    // A link that can't be used changes nothing.
    assert!(matches!(s.execute("collab.join", &json!({"link": "http://x"})), Err(EngineError::BadParams { .. })));
    assert_eq!((s.live.iter().count(), script.lock().unwrap().left), (1, 0));
    let join = id_of(&run(&mut s, "collab.join", json!({"link": LINK})));
    let p = run(&mut s, "collab.peers", json!({}));
    assert_eq!((&p["status"], &p["role"], id_of(&p)), (&json!("live"), &json!("host"), host), "this file still shows its own session");
    assert_eq!(p["sessions"].as_array().unwrap().len(), 2);
    // The join fails: the reason is said, and the file's own session is as it was.
    feed(&mut s, &script, join, vec![Event::Status(Status::Failed { reason: "wrong session token".into() })]);
    assert_eq!(s.message.as_deref(), Some("wrong session token"));
    assert_eq!((s.live().status.clone(), s.live().role()), (Status::Live, Some(Role::Host)));
    let p = s.live_summary();
    assert_eq!((&p["sessions"][1]["status"], &p["sessions"][1]["reason"]), (&json!("failed"), &json!("wrong session token")));
    assert_eq!(script.lock().unwrap().ended, [join], "the link is told the failed session is over");
    // The next join from here takes the failed one's place.
    let again = id_of(&run(&mut s, "collab.join", json!({"link": LINK})));
    assert_eq!(s.live.iter().map(|l| l.id).collect::<Vec<_>>(), [host, again]);
    // From a file that isn't in a session, a failed join shows there, and nowhere else.
    run(&mut s, "file.new", json!({}));
    let idle = id_of(&run(&mut s, "collab.join", json!({"link": LINK})));
    feed(&mut s, &script, idle, vec![Event::Status(Status::Failed { reason: "can't reach the host".into() })]);
    assert_eq!(s.live_summary()["reason"], "can't reach the host");
    assert_eq!(s.live_of(first).status, Status::Live);
    run(&mut s, "file.new", json!({}));
    assert_eq!(s.live_summary()["status"], "off", "a new file starts with no session");
    run(&mut s, "doc.switch", json!({"index": 1}));
    assert_eq!(run(&mut s, "collab.leave", json!({}))["ended"], false);
    assert_eq!(s.live_summary()["status"], "off");
}

#[test]
fn a_join_outlives_the_tab_it_was_started_from() {
    let (mut s, script) = session();
    run(&mut s, "file.new", json!({}));
    let join = id_of(&run(&mut s, "collab.join", json!({"link": LINK})));
    let failed = id_of(&run(&mut s, "collab.join", json!({"link": LINK})));
    feed(&mut s, &script, failed, vec![Event::Status(Status::Failed { reason: "no".into() })]);
    run(&mut s, "file.close", json!({"index": 1, "force": true}));
    s.live_sync();
    // The one on its way goes on, shown on the file now in front; the failed one is gone.
    assert_eq!(s.live.iter().map(|l| l.id).collect::<Vec<_>>(), [join]);
    assert_eq!((s.live().id, s.live().status.clone()), (join, Status::Connecting));
    assert_eq!(script.lock().unwrap().ended, [failed, failed], "told when it failed and when its tab closed; never the join on its way");
}

#[test]
fn publishing_one_file_keeps_the_others_editable() {
    let (mut s, script) = session();
    let host = id_of(&run(&mut s, "collab.host", json!({})));
    let first = uid(&s);
    run(&mut s, "file.new", json!({}));
    let r = run(&mut s, "collab.publish", json!({"server": "ws://127.0.0.1:47900/?key=k"}));
    assert_eq!((&r["status"], &r["readOnly"], &r["document"]), (&json!("connecting"), &json!(true), &json!(1)));
    assert_eq!(s.active_read_only(), Some(PUBLISHING));
    assert!(s.live_read_only(first).is_none());
    run(&mut s, "doc.switch", json!({"index": 0}));
    let p = s.live_summary();
    assert_eq!((&p["role"], &p["readOnly"], id_of(&p)), (&json!("host"), &json!(false), host));
    assert!(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).is_ok());
    // Publishing a file again replaces its own session, not the other file's.
    run(&mut s, "doc.switch", json!({"index": 1}));
    let again = id_of(&run(&mut s, "collab.publish", json!({"server": "ws://127.0.0.1:47900/?key=k"})));
    assert_eq!(script.lock().unwrap().ended, [id_of(&r)]);
    assert_eq!(s.live.iter().map(|l| l.id).collect::<Vec<_>>(), [host, again]);
}

#[test]
fn chat_and_spotlight_stay_in_their_session() {
    let (mut s, script, [ua, ub], [a, b]) = two_hosted();
    // Only the second file's host reads signals.
    feed(&mut s, &script, b, vec![Event::Signal(1, Signal::ready()), Event::PeerJoined(peer(3, "Ada"))]);
    script.lock().unwrap().msgs_to.clear();
    assert_eq!(run(&mut s, "collab.cursorChat", json!({"text": "here"}))["chat"], "here");
    assert_eq!(run(&mut s, "collab.spotlight", json!({"on": true}))["spotlighting"], true);
    assert_eq!(script.lock().unwrap().msgs_to, [b, b]);
    feed(&mut s, &script, b, vec![Event::Signal(3, Signal::chat("hello"))]);
    assert_eq!((s.live().chat.as_str(), s.live().spotlighting, s.live().chats.get(&3).map(String::as_str)), ("here", true, Some("hello")));
    // The first file's session has none of it, and can't send any.
    run(&mut s, "doc.switch", json!({"index": 0}));
    assert_eq!(uid(&s), ua);
    let p = s.live_summary();
    assert_eq!((&p["chat"], &p["spotlighting"], &p["signals"], id_of(&p)), (&json!(""), &json!(false), &json!(false), a));
    assert!(matches!(s.execute("collab.cursorChat", &json!({"text": "hi"})), Err(EngineError::Disabled(..))));
    assert_eq!(run(&mut s, "collab.cursorChat", json!({}))["open"], false);
    assert!(s.live_of(ub).spotlighting, "still spotlighting in the other file");
    // A report goes to the session of the file in front, which here isn't a server's.
    assert!(s.execute("collab.report", &json!({"reason": "spam"})).is_err());
}

#[test]
fn collab_commands_survive_junk_with_several_sessions() {
    let junk = [
        json!(null),
        json!([1, 2]),
        json!({"port": 1e300, "bind": 5, "link": 7, "client": -3, "x": "NaN", "y": {}, "session": -1, "document": 1e300}),
        json!({"link": "", "client": 4_294_967_296u64, "session": "x", "document": {}}),
        json!({"text": 5, "on": "yes"}),
        json!({"text": "\u{0}\u{7f}".repeat(10_000), "on": null}),
    ];
    for cmd in crate::all_commands().iter().filter(|c| c.id.starts_with("collab.")) {
        for j in &junk {
            let (mut s, script, _, [a, _]) = two_hosted();
            run(&mut s, "collab.join", json!({"link": LINK}));
            if let Err(EngineError::Internal { msg, .. }) = s.execute(cmd.id, j) {
                panic!("{} panicked on {j}: {msg}", cmd.id);
            }
            // Whatever it did, the sessions still take what arrives.
            feed(&mut s, &script, a, vec![Event::PeerJoined(peer(9, "Late")), Event::Status(Status::Off)]);
            s.live_sync();
            assert!(s.live.get(a).is_none());
        }
    }
}
