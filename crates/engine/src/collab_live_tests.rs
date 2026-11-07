//! Documents in a live session through the engine's live state, with a scripted link and the core
//! server in the test: commands, gestures and groups sync as one step each, `edit.undo` /
//! `edit.redo` take back only the user's own steps, a refused edit comes back with a toast, a
//! dropped connection resumes and sends what was made offline, and outside a session undo is
//! unchanged.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use figcraft_collab::sim::Rng;
use figcraft_collab::{Client, ClientMsg, Limits, Outgoing, Server, ServerMsg};
use figcraft_doc::{Document, NodeId};
use figcraft_geom::Affine;
use serde_json::json;

use crate::Session;
use crate::collab_tests::{base, random_command};
use crate::live::{Event, Role, Status};
use crate::live_tests::{Mock, Script, info};

const LINK: &str = "figcraft://join?host=127.0.0.1:47900&file=f&token=t";

/// One person: an engine session with a scripted link, what it sent that the server hasn't had
/// yet, and what the server sent that it hasn't taken in yet.
struct Person {
    s: Session,
    script: Arc<Mutex<Script>>,
    id: u32,
    up: VecDeque<ClientMsg>,
    down: VecDeque<Event>,
    online: bool,
}

/// A session server (the host's, minus the sockets) and several people joined as guests.
pub(crate) struct Room {
    pub(crate) server: Server,
    people: Vec<Person>,
    rng: Rng,
}

fn welcome_of(out: &[Outgoing], id: u32) -> ServerMsg {
    out.iter().find_map(|o| if let Outgoing::To(to, w @ ServerMsg::Welcome { .. }) = o { (*to == id).then(|| w.clone()) } else { None }).unwrap()
}

impl Room {
    pub(crate) fn new(doc: Document, n: usize, seed: u64) -> Room {
        let mut room = Room { server: Server::new(doc, "t", Limits::default()), people: Vec::new(), rng: Rng::new(seed) };
        for _ in 0..n {
            let mut s = Session::new();
            let script = Arc::new(Mutex::new(Script::default()));
            s.live.install(Box::new(Mock(script.clone())));
            s.execute("collab.join", &json!({"link": LINK})).unwrap();
            let ClientMsg::Hello(h) = Client::hello("f", "t", "Person", "") else { panic!("not a hello") };
            let (id, out) = room.server.join(h).unwrap();
            let w = welcome_of(&out, id);
            script.lock().unwrap().events.extend([Event::Welcome { info: info(Role::Guest, id), peers: vec![] }, Event::Sync(w)]);
            s.live_poll();
            assert!(s.live().syncing() && s.live_doc(s.ds().unwrap().uid), "joined: the shared file is the active tab");
            room.people.push(Person { s, script, id, up: VecDeque::new(), down: VecDeque::new(), online: true });
            room.route(out.into_iter().filter(|o| !matches!(o, Outgoing::To(_, ServerMsg::Welcome { .. }))).collect());
        }
        room
    }

    pub(crate) fn s(&mut self, i: usize) -> &mut Session {
        &mut self.people[i].s
    }

    fn route(&mut self, out: Vec<Outgoing>) {
        for o in out {
            let (msg, to): (ServerMsg, Vec<usize>) = match o {
                Outgoing::To(id, m) => (m, (0..self.people.len()).filter(|i| self.people[*i].id == id).collect()),
                Outgoing::All(m) => (m, (0..self.people.len()).collect()),
                Outgoing::Others(id, m) => (m, (0..self.people.len()).filter(|i| self.people[*i].id != id).collect()),
            };
            if !matches!(msg, ServerMsg::Changes(_) | ServerMsg::Reject { .. } | ServerMsg::Welcome { .. }) {
                continue;
            }
            let to: Vec<usize> = to.into_iter().filter(|i| self.people[*i].online).collect();
            for i in to {
                // Through the binary encoding, like the network.
                let m = figcraft_collab::message::decode(&figcraft_collab::message::encode(&msg).unwrap(), usize::MAX).unwrap();
                self.people[i].down.push_back(Event::Sync(m));
            }
        }
    }

    /// What everyone sent (the UI calls `live_sync` every frame).
    fn pump(&mut self) {
        for p in &mut self.people {
            p.s.live_sync();
            let msgs = std::mem::take(&mut p.script.lock().unwrap().msgs);
            if p.online {
                p.up.extend(msgs);
            }
        }
    }

    /// Deliver one message on a random busy link. False when nothing is in flight.
    fn step(&mut self) -> bool {
        self.pump();
        let busy: Vec<(bool, usize)> = (0..self.people.len())
            .flat_map(|i| [(true, i), (false, i)])
            .filter(|(up, i)| if *up { !self.people[*i].up.is_empty() } else { !self.people[*i].down.is_empty() })
            .collect();
        let Some(&(up, i)) = self.rng.pick(&busy) else { return false };
        if up {
            let msg = self.people[i].up.pop_front().unwrap();
            let out = self.server.receive(self.people[i].id, msg);
            self.route(out);
        } else {
            let p = &mut self.people[i];
            let e = p.down.pop_front().unwrap();
            p.script.lock().unwrap().events.push(e);
            p.s.live_poll();
        }
        self.pump();
        true
    }

    pub(crate) fn settle(&mut self) {
        let mut n = 0;
        while self.step() {
            n += 1;
            assert!(n < 1_000_000, "still busy");
        }
    }

    /// Person `i`'s connection drops: what is in flight is lost; the link says it's reconnecting.
    fn disconnect(&mut self, i: usize) {
        let p = &mut self.people[i];
        p.up.clear();
        p.down.clear();
        p.online = false;
        p.script.lock().unwrap().events.push(Event::Status(Status::Reconnecting { attempt: 1 }));
        p.s.live_poll();
        let out = self.server.leave(self.people[i].id);
        self.route(out);
    }

    /// Person `i` is back: the link says hello with the client's id and last sequence number.
    fn reconnect(&mut self, i: usize) {
        let p = &mut self.people[i];
        let ClientMsg::Hello(mut h) = Client::hello("f", "t", "Person", "") else { panic!("not a hello") };
        h.resume = p.s.live().client().map(Client::resume);
        let (id, out) = self.server.join(h).unwrap();
        assert_eq!(id, p.id, "the same client");
        p.online = true;
        p.down.push_back(Event::Welcome { info: info(Role::Guest, id), peers: vec![] });
        self.route(out);
    }

    /// Everyone holds the server's document, in the session and on the canvas.
    pub(crate) fn check(&self) {
        for (i, p) in self.people.iter().enumerate() {
            let c = p.s.live().client().unwrap();
            assert_eq!(c.pending(), 0, "person {i}");
            assert!(c.view() == self.server.replica(), "person {i}: {}", figcraft_collab::sim::describe(self.server.replica(), c.view()));
            assert!(*p.s.doc().unwrap() == *self.server.document(), "person {i}: the canvas shows the session's document");
        }
    }
}

/// The shared file's first frame and one of its rectangles.
pub(crate) fn frame_and_rect(doc: &Document) -> (NodeId, NodeId) {
    let page = doc.first_page().unwrap();
    let f = doc.node(page).unwrap().children[0];
    (f, doc.node(f).unwrap().children[0])
}

#[test]
fn edits_sync_and_undo_takes_back_only_your_own() {
    let mut room = Room::new(base(), 2, 1);
    let (f, r) = frame_and_rect(room.server.document());
    let old = room.server.document().node(r).unwrap().name.clone();
    room.s(0).execute("node.set", &json!({"id": r, "props": {"name": "Mine"}})).unwrap();
    room.settle();
    room.s(1).execute("transform.move", &json!({"ids": [r], "dx": 25, "dy": 0})).unwrap();
    room.settle();
    room.check();
    let moved = room.server.document().node(r).unwrap().transform;
    // Person 0 undoes: their rename goes, person 1's later move stays.
    assert_eq!(room.s(0).execute("edit.undo", &json!({})).unwrap()["undone"], true);
    room.settle();
    room.check();
    let n = room.server.document().node(r).unwrap();
    assert_eq!((&n.name, n.transform), (&old, moved));
    assert_eq!(room.s(0).status()["redo"], 1);
    assert_eq!(room.s(0).execute("edit.undo", &json!({})).unwrap()["undone"], false, "nothing else of theirs");
    assert_eq!(room.s(1).status()["undo"], 1, "remote edits never enter someone else's history");
    room.s(0).execute("edit.redo", &json!({})).unwrap();
    room.settle();
    room.check();
    assert_eq!(room.server.document().node(r).unwrap().name, "Mine");
    // Deleting a frame with layers and undoing it brings it back whole.
    let before = room.server.replica().clone();
    room.s(1).execute("edit.delete", &json!({"ids": [f]})).unwrap();
    room.settle();
    assert!(!room.server.document().contains(f));
    room.s(1).execute("edit.undo", &json!({})).unwrap();
    room.settle();
    room.check();
    assert!(room.server.replica() == &before);
    // In a live session the snapshot comparison of `collab.diff` doesn't apply.
    assert!(room.s(0).execute("collab.diff", &json!({})).is_err());
    assert!(room.s(0).execute("collab.diff", &json!({"against": "saved"})).is_ok());
}

#[test]
fn a_gesture_is_one_step() {
    let mut room = Room::new(base(), 2, 2);
    let (_, r) = frame_and_rect(room.server.document());
    let start = room.server.document().node(r).unwrap().transform;
    let s = room.s(0);
    s.begin_interaction("Move").unwrap();
    for dx in [5.0, 10.0, 15.0] {
        s.preview(|doc, _| {
            if let Some(n) = doc.node_mut(r) {
                n.transform = Affine::translate((dx, 0.0)) * start;
            }
            Ok(())
        })
        .unwrap();
    }
    // Nothing goes out during the drag; someone else renames the layer meanwhile.
    room.pump();
    assert!(room.people[0].up.is_empty());
    room.s(1).execute("node.set", &json!({"id": r, "props": {"name": "Theirs"}})).unwrap();
    room.settle();
    room.s(0).commit_interaction().unwrap();
    room.settle();
    room.check();
    let n = room.server.document().node(r).unwrap();
    assert_eq!((n.name.as_str(), n.transform), ("Theirs", Affine::translate((15.0, 0.0)) * start));
    assert_eq!(room.s(0).status()["undo"], 1, "one step for the whole drag");
    room.s(0).execute("edit.undo", &json!({})).unwrap();
    room.settle();
    room.check();
    let n = room.server.document().node(r).unwrap();
    assert_eq!((n.name.as_str(), n.transform), ("Theirs", start));
}

#[test]
fn a_group_of_commands_syncs_as_one_step() {
    let mut room = Room::new(base(), 2, 3);
    let (f, r) = frame_and_rect(room.server.document());
    let start = room.server.replica().clone();
    let cmds = vec![
        crate::BatchStep::new("node.set", json!({"id": r, "props": {"name": "A"}})),
        crate::BatchStep::new("node.set", json!({"id": f, "props": {"name": "B"}})),
        crate::BatchStep::new("shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5})),
    ];
    room.s(0).batch("Three", &cmds).unwrap();
    room.pump();
    assert_eq!(room.people[0].up.len(), 1, "one batch");
    room.settle();
    room.check();
    assert_eq!(room.s(0).status()["undo"], 1);
    room.s(0).execute("edit.undo", &json!({})).unwrap();
    room.settle();
    room.check();
    assert!(room.server.replica() == &start, "all three are taken back");
}

#[test]
fn an_undo_waits_for_its_edit_and_shows_at_once() {
    let mut room = Room::new(base(), 2, 6);
    let (_, r) = frame_and_rect(room.server.document());
    let old = room.server.document().node(r).unwrap().name.clone();
    let start = room.server.replica().clone();
    room.s(0).execute("node.set", &json!({"id": r, "props": {"name": "Quick"}})).unwrap();
    room.s(0).execute("edit.undo", &json!({})).unwrap();
    assert_eq!(room.s(0).doc().unwrap().node(r).unwrap().name, old, "shown at once");
    room.pump();
    assert_eq!(room.people[0].up.len(), 1, "only the edit goes out until it is confirmed");
    room.settle();
    room.check();
    assert!(room.server.replica() == &start);
}

#[test]
fn a_refused_edit_is_rolled_back_with_a_toast() {
    let mut room = Room::new(base(), 2, 4);
    let (f, r) = frame_and_rect(room.server.document());
    // Person 1 deletes the frame; person 0 renames a layer inside it before hearing of it.
    room.s(1).execute("edit.delete", &json!({"ids": [f]})).unwrap();
    room.pump();
    let msg = room.people[1].up.pop_front().unwrap();
    let out = room.server.receive(room.people[1].id, msg);
    room.route(out);
    room.s(0).execute("node.set", &json!({"id": r, "props": {"name": "Too late"}})).unwrap();
    room.settle();
    room.check();
    let s = room.s(0);
    assert!(s.message.as_deref().unwrap_or_default().contains("deleted by someone else"), "{:?}", s.message);
    assert_eq!(s.status()["undo"], 0, "no undo step for refused work");
}

#[test]
fn a_dropped_connection_keeps_editing_and_catches_up() {
    let mut room = Room::new(base(), 2, 7);
    let (f, r) = frame_and_rect(room.server.document());
    room.s(0).execute("node.set", &json!({"id": r, "props": {"name": "Sent"}})).unwrap();
    room.pump();
    room.disconnect(0);
    // Offline: edits and undo keep working; nothing goes out.
    room.s(0).execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    room.s(0).execute("edit.undo", &json!({})).unwrap();
    room.s(0).execute("node.set", &json!({"id": f, "props": {"name": "Offline"}})).unwrap();
    room.pump();
    assert!(room.people[0].up.is_empty());
    room.s(1).execute("transform.move", &json!({"ids": [r], "dx": 5, "dy": 5})).unwrap();
    room.settle();
    room.reconnect(0);
    room.settle();
    room.check();
    let doc = room.server.document();
    assert_eq!((doc.node(r).unwrap().name.as_str(), doc.node(f).unwrap().name.as_str()), ("Sent", "Offline"));
}

impl Room {
    /// Person `i` has been told to join again: the server takes them as `resume` says.
    fn welcome_as(&mut self, i: usize, resume: Option<figcraft_collab::Resume>) {
        let ClientMsg::Hello(mut h) = Client::hello("f", "t", "Person", "") else { panic!("not a hello") };
        h.resume = resume;
        let (id, out) = self.server.join(h).unwrap();
        let w = welcome_of(&out, id);
        let p = &mut self.people[i];
        p.id = id;
        p.online = true;
        p.script.lock().unwrap().events.extend([Event::Welcome { info: info(Role::Guest, id), peers: vec![] }, Event::Sync(w)]);
        p.s.live_poll();
    }

    fn event(&mut self, i: usize, e: Event) {
        let p = &mut self.people[i];
        p.script.lock().unwrap().events.push(e);
        p.s.live_poll();
    }
}

#[test]
fn reconnecting_can_be_told_to_try_now_and_shows_as_disconnected() {
    let mut room = Room::new(base(), 1, 7);
    assert!(!room.s(0).live().disconnected());
    assert!(room.s(0).execute("collab.reconnect", &json!({})).is_err(), "nothing was lost");
    room.disconnect(0);
    assert!(room.s(0).live().disconnected());
    assert_eq!(room.s(0).execute("collab.reconnect", &json!({})).unwrap()["status"], "reconnecting");
    let session = room.s(0).live().id;
    assert_eq!(room.people[0].script.lock().unwrap().retried, [session]);
}

#[test]
fn a_session_that_cant_go_on_stays_on_its_tab_and_merges_when_it_joins_again() {
    let mut room = Room::new(base(), 2, 7);
    let (f, r) = frame_and_rect(room.server.document());
    let uid = room.s(0).ds().unwrap().uid;
    let tabs = room.s(0).docs.len();
    room.disconnect(0);
    room.s(0).execute("node.set", &json!({"id": r, "props": {"name": "Offline"}})).unwrap();
    room.event(0, Event::Status(Status::Failed { reason: "The host refused: this link isn't valid".into() }));
    // The tab says so, and is yours to go on working in.
    let live = room.s(0).live();
    assert!(live.disconnected() && live.rejoinable() && live.doc_uid == Some(uid));
    assert!(room.s(0).live_read_only(uid).is_none());
    let made = room.s(0).execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    let made = NodeId(made["id"].as_u64().unwrap());
    // A try that fails leaves it just as it was.
    assert_eq!(room.s(0).execute("collab.reconnect", &json!({})).unwrap()["status"], "connecting");
    assert!(room.s(0).live().disconnected() && !room.s(0).live().rejoinable());
    assert!(room.s(0).live_read_only(uid).is_none(), "it can still be edited while it tries");
    room.event(0, Event::Status(Status::Failed { reason: "Couldn't join the live session: no answer".into() }));
    assert!(room.s(0).live().rejoinable());
    // Someone else's edit lands meanwhile.
    room.s(1).execute("transform.move", &json!({"ids": [f], "dx": 5, "dy": 5})).unwrap();
    let theirs = |d: &Document| d.node(f).unwrap().transform;
    room.settle();
    let moved = theirs(room.server.document());
    // The next try joins in the same tab, as the same client, and everything goes on top.
    room.s(0).execute("collab.reconnect", &json!({})).unwrap();
    let resumed = room.people[0].script.lock().unwrap().resumed.expect("as who you were");
    assert_eq!((resumed.client_id, resumed.seq), (room.people[0].id, u64::MAX));
    room.welcome_as(0, Some(resumed));
    room.settle();
    room.check();
    let s = room.s(0);
    assert_eq!(s.live().status, Status::Live);
    assert!(!s.live().disconnected());
    assert_eq!(s.docs.len(), tabs, "no copy: it all went in");
    let doc = room.server.document();
    assert_eq!(doc.node(r).unwrap().name, "Offline");
    assert!(doc.contains(made), "layers you made too");
    assert_eq!(theirs(doc), moved, "and the other edit is still there");
    assert!(room.s(0).message.as_deref().is_some_and(|m| m.contains("is in the file")));
}

#[test]
fn the_same_thing_changed_both_ways_keeps_the_others_and_opens_your_version_beside_it() {
    let mut room = Room::new(base(), 2, 7);
    let (f, r) = frame_and_rect(room.server.document());
    let tabs = room.s(0).docs.len();
    room.disconnect(0);
    room.s(0).execute("node.set", &json!({"id": r, "props": {"name": "Mine"}})).unwrap();
    room.s(0).execute("node.set", &json!({"id": f, "props": {"name": "Only mine"}})).unwrap();
    room.event(0, Event::Status(Status::Failed { reason: "gone".into() }));
    room.s(1).execute("node.set", &json!({"id": r, "props": {"name": "Theirs"}})).unwrap();
    room.settle();
    room.s(0).execute("collab.reconnect", &json!({})).unwrap();
    let resumed = room.people[0].script.lock().unwrap().resumed;
    room.welcome_as(0, resumed);
    room.settle();
    room.check();
    let doc = room.server.document();
    assert_eq!(doc.node(r).unwrap().name, "Theirs", "what conflicts isn't written over");
    assert_eq!(doc.node(f).unwrap().name, "Only mine", "what doesn't goes in");
    let s = room.s(0);
    assert_eq!(s.docs.len(), tabs + 1);
    let copy = s.docs.last().unwrap();
    assert!(copy.doc.name.ends_with("(offline copy)") && copy.is_dirty(), "closing the copy asks first");
    assert_eq!(copy.doc.node(r).unwrap().name, "Mine");
}

#[test]
fn layers_the_new_client_cant_make_open_beside_the_file() {
    let mut room = Room::new(base(), 1, 7);
    let tabs = room.s(0).docs.len();
    room.disconnect(0);
    let made = room.s(0).execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    let made = NodeId(made["id"].as_u64().unwrap());
    room.event(0, Event::Status(Status::Failed { reason: "gone".into() }));
    room.s(0).execute("collab.reconnect", &json!({})).unwrap();
    // The server takes you as someone new.
    room.welcome_as(0, None);
    room.settle();
    room.check();
    assert!(!room.server.document().contains(made));
    let s = room.s(0);
    assert_eq!(s.docs.len(), tabs + 1);
    assert!(s.docs.last().unwrap().doc.contains(made), "it isn't lost");
}

#[test]
fn a_server_that_forgot_you_keeps_your_offline_edits() {
    let mut room = Room::new(base(), 1, 7);
    let (_, r) = frame_and_rect(room.server.document());
    room.disconnect(0);
    room.s(0).execute("node.set", &json!({"id": r, "props": {"name": "Offline"}})).unwrap();
    // The same file under a new session: it knows no one.
    room.server = Server::new(room.server.document().clone(), "t", Limits::default());
    room.welcome_as(0, None);
    room.settle();
    assert_eq!(room.server.document().node(r).unwrap().name, "Offline");
    assert_eq!(room.s(0).live().status, Status::Live);
}

#[test]
fn joining_again_with_nothing_changed_opens_no_copy() {
    let mut room = Room::new(base(), 1, 7);
    let tabs = room.s(0).docs.len();
    room.disconnect(0);
    room.event(0, Event::Ended { reason: "Sharing was stopped".into() });
    assert!(room.s(0).live().rejoinable(), "the end of a session you were in shows on its tab too");
    assert_eq!(room.s(0).message.as_deref(), Some("Sharing was stopped"));
    room.s(0).execute("collab.reconnect", &json!({})).unwrap();
    let resumed = room.people[0].script.lock().unwrap().resumed;
    room.welcome_as(0, resumed);
    assert_eq!(room.s(0).docs.len(), tabs);
    assert_eq!(room.s(0).live().status, Status::Live);
}

#[test]
fn a_session_out_of_step_joins_again_by_itself() {
    let mut room = Room::new(base(), 1, 7);
    let joins = room.people[0].script.lock().unwrap().joined.len();
    let tabs = room.s(0).docs.len();
    // A batch from far in the future: it can't be applied.
    room.event(0, Event::Sync(ServerMsg::Changes(figcraft_collab::Accepted { seq: 999, author: 42, batch_id: 1, changes: vec![] })));
    assert_eq!(room.s(0).live().status, Status::Connecting);
    assert!(room.s(0).live().disconnected());
    assert_eq!(room.people[0].script.lock().unwrap().joined.len(), joins + 1);
    let resumed = room.people[0].script.lock().unwrap().resumed;
    room.welcome_as(0, resumed);
    assert_eq!(room.s(0).live().status, Status::Live);
    assert_eq!(room.s(0).docs.len(), tabs);
}

#[test]
fn leaving_with_changes_unconfirmed_says_so() {
    let mut room = Room::new(base(), 1, 7);
    let (_, r) = frame_and_rect(room.server.document());
    room.s(0).execute("collab.leave", &json!({})).unwrap();
    assert_eq!(room.s(0).message.as_deref(), Some("You left the session; your copy stays open here"));
    let mut room = Room::new(base(), 1, 7);
    room.s(0).execute("node.set", &json!({"id": r, "props": {"name": "Unconfirmed"}})).unwrap();
    room.s(0).execute("collab.leave", &json!({})).unwrap();
    assert!(room.s(0).message.as_deref().is_some_and(|m| m.contains("before your last changes were confirmed")));
}

#[test]
fn leaving_a_lost_session_makes_the_tab_a_plain_copy() {
    let mut room = Room::new(base(), 1, 7);
    room.disconnect(0);
    room.event(0, Event::Status(Status::Failed { reason: "gone".into() }));
    room.s(0).execute("collab.leave", &json!({})).unwrap();
    assert_eq!(room.s(0).live().status, Status::Off);
    assert!(!room.s(0).live().disconnected());
}

#[test]
fn random_commands_with_undo_and_drops_converge_through_the_engine() {
    for seed in 0..6 {
        let mut room = Room::new(base(), 3, seed);
        let mut rng = Rng::new(seed + 100);
        for _ in 0..80 {
            let i = rng.below(3);
            if rng.chance(0.06) {
                if room.people[i].online {
                    room.disconnect(i);
                } else {
                    room.reconnect(i);
                }
                continue;
            }
            let (cmd, params) = random_command(room.people[i].s.doc().unwrap(), &mut rng, true);
            let _ = room.s(i).execute(cmd, &params);
            for _ in 0..rng.below(4) {
                room.step();
            }
        }
        for i in 0..3 {
            if !room.people[i].online {
                room.reconnect(i);
            }
        }
        room.settle();
        room.check();
    }
}

#[test]
fn leaving_the_session_goes_back_to_snapshot_undo() {
    let mut room = Room::new(base(), 1, 5);
    let (_, r) = frame_and_rect(room.server.document());
    room.s(0).execute("node.set", &json!({"id": r, "props": {"name": "Live"}})).unwrap();
    room.settle();
    let s = room.s(0);
    assert_eq!(s.execute("collab.leave", &json!({})).unwrap()["ended"], true);
    assert_eq!(s.status()["undo"], 0, "the live history doesn't carry over");
    s.execute("node.set", &json!({"id": r, "props": {"name": "Solo"}})).unwrap();
    assert_eq!(s.status()["undo"], 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().node(r).unwrap().name, "Live");
}
