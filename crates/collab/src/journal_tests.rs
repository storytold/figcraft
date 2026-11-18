//! View-only participants and the journal a store keeps between snapshots: a viewer's batches are
//! refused with the reason and rolled back, everything else reaches it; replaying the journal over
//! the last saved state gives the same server (clients resume into it), entries already in the
//! state change nothing, and a log with a gap is refused.

use figcraft_doc::{Document, NodeKind};

use crate::message::{self, Access, ClientMsg, Hello, ServerMsg};
use crate::sim_tests::base;
use crate::{Client, Journal, Limits, NoRecompute, Outgoing, Server, VIEW_ONLY};

fn hello() -> Hello {
    let ClientMsg::Hello(h) = Client::hello("f", "", "Ada", "#FF8800") else { panic!("not a hello") };
    h
}

fn join_as(s: &mut Server, access: Access) -> Client {
    let (_, out) = s.join_as(hello(), access).unwrap();
    let Some(Outgoing::To(_, welcome)) = out.into_iter().next() else { panic!("no welcome") };
    let c = Client::join(welcome).unwrap();
    assert_eq!(c.access(), access, "the welcome says what the client may do");
    c
}

/// Deliver everything in `out` meant for `c`; returns what it sends back.
fn deliver(c: &mut Client, out: &[Outgoing]) -> Vec<ClientMsg> {
    let id = c.id();
    let mut replies = Vec::new();
    for o in out {
        let m = match o {
            Outgoing::To(t, m) if *t == id => m,
            Outgoing::All(m) => m,
            Outgoing::Others(not, m) if *not != id => m,
            _ => continue,
        };
        replies.extend(c.receive(m.clone(), &NoRecompute).unwrap());
    }
    replies
}

fn edit(c: &mut Client, f: impl FnOnce(&mut Document)) -> ClientMsg {
    let mut d = c.document().clone();
    f(&mut d);
    c.edit(&d).unwrap().expect("a change")
}

/// `c` sends `m`; everyone in `all` gets what the server says.
fn send(s: &mut Server, from: u32, m: ClientMsg, all: &mut [&mut Client]) -> Vec<Outgoing> {
    let out = s.receive(from, m);
    for c in all.iter_mut() {
        deliver(c, &out);
    }
    out
}

#[test]
fn a_viewer_sees_everything_and_changes_nothing() {
    let mut s = Server::new(base(), "", Limits::default());
    let mut editor = join_as(&mut s, Access::Edit);
    let mut viewer = join_as(&mut s, Access::View);
    let page = s.document().first_page().unwrap();
    // The editor's edit reaches the viewer.
    let m = edit(&mut editor, |d| {
        d.create(page, NodeKind::Rectangle, "Card", 0.0, 0.0, 10.0, 10.0).unwrap();
    });
    send(&mut s, editor.id(), m, &mut [&mut editor, &mut viewer]);
    assert_eq!(viewer.view(), s.replica());
    // The viewer's edit is refused with the reason, and rolled back.
    let seq = s.seq();
    let m = edit(&mut viewer, |d| d.node_mut(page).unwrap().name = "Mine".into());
    let out = send(&mut s, viewer.id(), m, &mut [&mut editor, &mut viewer]);
    assert!(matches!(out.as_slice(), [Outgoing::To(_, ServerMsg::Reject { reason, .. })] if reason == VIEW_ONLY));
    assert_eq!(s.seq(), seq, "nothing accepted");
    assert_eq!(viewer.view(), s.replica(), "the viewer's copy is the server's again");
    assert!(viewer.take_notices().iter().any(|n| n.message().contains("view")), "the viewer is told why");
    // Presence still goes both ways.
    let out = s.receive(viewer.id(), ClientMsg::Presence(Default::default()));
    assert!(matches!(out.as_slice(), [Outgoing::Others(_, ServerMsg::Presence { .. })]));
    // Joining again with an edit link lifts it.
    let id = viewer.id();
    s.leave(id);
    let mut again = join_as(&mut s, Access::Edit);
    let m = edit(&mut again, |d| d.node_mut(page).unwrap().name = "Now mine".into());
    let out = s.receive(again.id(), m);
    assert!(matches!(out.as_slice(), [Outgoing::All(ServerMsg::Changes(_))]));
}

#[test]
fn managing_messages_are_refused_on_a_session_connection() {
    let mut s = Server::new(base(), "", Limits::default());
    let c = join_as(&mut s, Access::Edit);
    let out = s.receive(c.id(), ClientMsg::Admin(message::Admin::Files));
    assert!(matches!(out.as_slice(), [Outgoing::To(_, ServerMsg::Refused { .. })]));
}

#[test]
fn the_journal_replayed_over_the_last_state_restores_the_server() {
    let mut s = Server::new(base(), "", Limits::default());
    s.journaling(true);
    let mut a = join_as(&mut s, Access::Edit);
    let mut v = join_as(&mut s, Access::View);
    let page = s.document().first_page().unwrap();
    // Saved here: what comes after is only in the journal.
    let saved = s.state().unwrap();
    s.take_journal();
    let mut b = join_as(&mut s, Access::Edit);
    for i in 0..5 {
        let m = edit(&mut a, |d| {
            d.create(page, NodeKind::Rectangle, &format!("A{i}"), f64::from(i), 0.0, 10.0, 10.0).unwrap();
        });
        send(&mut s, a.id(), m, &mut [&mut a, &mut b, &mut v]);
    }
    let m = edit(&mut b, |d| d.node_mut(page).unwrap().name = "Bo's page".into());
    send(&mut s, b.id(), m, &mut [&mut a, &mut b, &mut v]);
    let m = edit(&mut v, |d| d.node_mut(page).unwrap().name = "Refused".into());
    send(&mut s, v.id(), m, &mut [&mut a, &mut b, &mut v]);
    let journal = s.take_journal();
    assert_eq!(journal.iter().filter(|j| matches!(j, Journal::Issued { .. })).count(), 1, "b's id");
    assert_eq!(journal.iter().filter(|j| matches!(j, Journal::Accepted(_))).count(), 6);
    assert_eq!(journal.iter().filter(|j| matches!(j, Journal::Refused { .. })).count(), 1);
    assert!(s.take_journal().is_empty(), "taken");
    // The entries survive both encodings (the store writes the binary one).
    for j in &journal {
        assert_eq!(&message::decode::<Journal>(&message::encode(j).unwrap(), usize::MAX).unwrap(), j);
    }
    // Restart: the saved state, then the journal.
    let mut r = Server::restore(saved, "", Limits::default()).unwrap();
    for j in journal.iter().cloned() {
        r.replay(j).unwrap();
    }
    assert_eq!(r.replica(), s.replica());
    assert_eq!(r.seq(), s.seq());
    let mut live = s.state().unwrap();
    let mut back = r.state().unwrap();
    // The session id is kept; the rest of the lasting state matches too.
    assert_eq!(live.session, back.session);
    live.snapshot.clear();
    back.snapshot.clear();
    assert_eq!(live, back);
    // Replaying again changes nothing (a log that outlived a compaction).
    for j in journal {
        r.replay(j).unwrap();
    }
    assert_eq!(r.replica(), s.replica());
    assert!(r.take_journal().is_empty(), "replays aren't journaled");
    // Every client resumes into the restarted server with nothing to send again, and a newcomer
    // gets an id nobody had.
    for c in [&mut a, &mut b] {
        c.disconnect();
        let ClientMsg::Hello(h) = c.reconnect(hello()) else { panic!() };
        let (id, out) = r.join(h).unwrap();
        assert_eq!(id, c.id());
        assert!(deliver(c, &out).is_empty());
        assert_eq!(c.view(), r.replica());
    }
    let n = join_as(&mut r, Access::Edit);
    assert!(n.id() > b.id());
}

#[test]
fn a_journal_with_a_gap_is_refused() {
    let mut s = Server::new(base(), "", Limits::default());
    s.journaling(true);
    let saved = s.state().unwrap();
    let mut a = join_as(&mut s, Access::Edit);
    let page = s.document().first_page().unwrap();
    for name in ["One", "Two"] {
        let m = edit(&mut a, |d| d.node_mut(page).unwrap().name = name.into());
        send(&mut s, a.id(), m, &mut [&mut a]);
    }
    let journal: Vec<Journal> = s.take_journal().into_iter().filter(|j| !matches!(j, Journal::Accepted(acc) if acc.seq == 1)).collect();
    let mut r = Server::restore(saved, "", Limits::default()).unwrap();
    let errors: Vec<_> = journal.into_iter().filter_map(|j| r.replay(j).err()).collect();
    assert_eq!(errors.len(), 1, "the batch after the gap");
    assert_eq!(r.seq(), 0);
}

/// Restores `state` with one more batch from another client in its log (and in its snapshot,
/// `in_snapshot`, as a server that accepted it saved it), then has `a` resume into it.
fn resume_past(s: &Server, a: &mut Client, batch: Vec<crate::Change>, in_snapshot: bool) -> Server {
    let mut state = s.state().unwrap();
    if in_snapshot {
        state.snapshot.retain(|c| {
            !batch.iter().any(|b| matches!((b, c), (crate::Change::Put { entry: x, .. }, crate::Change::Put { entry: y, .. }) if x == y))
        });
        state.snapshot.extend(batch.iter().cloned());
    }
    state.seq += 1;
    state.log.push(message::Accepted { seq: state.seq, author: a.id() + 1, batch_id: 1, changes: batch });
    let mut r = Server::restore(state, "", Limits::default()).unwrap();
    a.disconnect();
    let ClientMsg::Hello(h) = a.reconnect(hello()) else { panic!("not a hello") };
    let (_, out) = r.join(h).unwrap();
    deliver(a, &out);
    r
}

/// A saved log may hold an image a newer server refuses: a client resuming from before it still
/// catches up, without that image (#1086).
#[test]
fn a_client_resuming_past_a_refused_image_in_the_saved_log_catches_up() {
    let mut s = Server::new(base(), "", Limits::default());
    let mut a = join_as(&mut s, Access::Edit);
    let (key, bytes) = crate::image_tests::forged();
    let r = resume_past(&s, &mut a, vec![crate::image_tests::put(&key, 2, 1, bytes)], false);
    assert!(!a.view().document().images.contains_key(&key));
    assert_eq!(a.view(), r.replica());
}

/// A saved log may give an image other bytes under a key the resuming client already holds (an
/// older server allowed it): the client still catches up, to what the server holds (#1086).
#[test]
fn a_client_resuming_past_a_replaced_image_in_the_saved_log_catches_up() {
    let mut s = Server::new(base(), "", Limits::default());
    let mut a = join_as(&mut s, Access::Edit);
    let m = edit(&mut a, |d| {
        d.images.insert("img1".into(), figcraft_doc::ImageData { bytes: std::sync::Arc::new(b"the first bytes".to_vec()), width: 2, height: 1 });
    });
    send(&mut s, a.id(), m, &mut [&mut a]);
    let r = resume_past(&s, &mut a, vec![crate::image_tests::put("img1", 2, 1, b"other bytes")], true);
    assert_eq!(a.view(), r.replica());
}

/// `a` adds a rectangle showing image `img1`; returns the rectangle.
fn add_image_layer(s: &mut Server, a: &mut Client) -> figcraft_doc::NodeId {
    let page = s.document().first_page().unwrap();
    let mut id = None;
    let m = edit(a, |d| {
        let r = d.create(page, NodeKind::Rectangle, "R", 0.0, 0.0, 10.0, 10.0).unwrap();
        d.get_mut(r).unwrap().fills = vec![figcraft_color::Paint::image("img1", figcraft_color::ImageScaleMode::Fill)];
        d.images.insert("img1".into(), figcraft_doc::ImageData { bytes: std::sync::Arc::new(b"the first bytes".to_vec()), width: 2, height: 1 });
        id = Some(r);
    });
    send(s, a.id(), m, &mut [a]);
    id.unwrap()
}

/// Undoing an image's insertion still works with images in use protected from removal (#1086).
#[test]
fn undoing_an_image_layer_still_works() {
    let mut s = Server::new(base(), "", Limits::default());
    let mut a = join_as(&mut s, Access::Edit);
    let id = add_image_layer(&mut s, &mut a);
    for m in a.undo(&NoRecompute).unwrap() {
        send(&mut s, a.id(), m, &mut [&mut a]);
    }
    assert!(s.document().node(id).is_none(), "the layer is gone");
    assert_eq!(a.view(), s.replica());
}

/// A server's log written before the image checks may remove an image a layer still shows: a
/// restart replays past it, keeping the image and every edit after it (#1086).
#[test]
fn a_restart_replays_past_a_logged_removal_of_an_image_in_use() {
    let mut s = Server::new(base(), "", Limits::default());
    s.journaling(true);
    let mut a = join_as(&mut s, Access::Edit);
    add_image_layer(&mut s, &mut a);
    let saved = s.state().unwrap();
    let page = s.document().first_page().unwrap();
    let mut journal = s.take_journal();
    let base_seq = saved.seq;
    let next = |n: u64| base_seq + n;
    journal.push(Journal::Accepted(message::Accepted {
        seq: next(1),
        author: a.id(),
        batch_id: 90,
        changes: vec![crate::Change::Remove { entry: crate::change::Entry::Image("img1".into()) }],
    }));
    let rename = crate::Change::Set { id: page, prop: crate::change::Prop::Name, value: crate::change::Json(serde_json::json!("Renamed")) };
    journal.push(Journal::Accepted(message::Accepted { seq: next(2), author: a.id(), batch_id: 91, changes: vec![rename] }));
    let mut r = Server::restore(saved, "", Limits::default()).unwrap();
    for j in journal {
        r.replay(j).expect("the log replays");
    }
    assert_eq!(r.seq(), next(2));
    assert!(r.document().images.contains_key("img1"), "the image a layer shows stays");
    assert_eq!(r.document().node(page).unwrap().name, "Renamed", "the edit after it is kept");
}

/// What a resuming client is told to catch up with.
fn catch_up_of(r: &mut Server, a: &mut Client) -> message::CatchUp {
    a.disconnect();
    let ClientMsg::Hello(h) = a.reconnect(hello()) else { panic!("not a hello") };
    let (id, out) = r.join(h).unwrap();
    out.into_iter()
        .find_map(|o| match o {
            Outgoing::To(t, ServerMsg::Welcome { catch_up, .. }) if t == id => Some(catch_up),
            _ => None,
        })
        .expect("a welcome")
}

/// A restart that left an image out of the saved history (an older server accepted it) catches
/// every resuming client up with the snapshot, even one at the same sequence number: a client
/// that kept the image from before would otherwise go on showing what the session no longer
/// has (#1086).
#[test]
fn a_resume_after_history_was_left_out_catches_up_with_the_snapshot() {
    let mut s = Server::new(base(), "", Limits::default());
    let mut a = join_as(&mut s, Access::Edit);
    let mut state = s.state().unwrap();
    let (key, bytes) = crate::image_tests::forged();
    state.snapshot.push(crate::image_tests::put(&key, 2, 1, bytes));
    let mut r = Server::restore(state, "", Limits::default()).unwrap();
    assert!(matches!(catch_up_of(&mut r, &mut a), message::CatchUp::Snapshot(_)), "left out on restore");

    // Left out on replay.
    let mut s = Server::new(base(), "", Limits::default());
    let mut a = join_as(&mut s, Access::Edit);
    let state = s.state().unwrap();
    let mut r = Server::restore(state.clone(), "", Limits::default()).unwrap();
    let batch = vec![crate::image_tests::put(&key, 2, 1, bytes)];
    r.replay(Journal::Accepted(message::Accepted { seq: state.seq + 1, author: a.id(), batch_id: 90, changes: batch })).unwrap();
    assert!(matches!(catch_up_of(&mut r, &mut a), message::CatchUp::Snapshot(_)), "left out on replay");
}

/// A restart that left nothing out keeps resuming clients on the cheaper delta.
#[test]
fn a_resume_after_a_clean_restart_catches_up_with_the_log() {
    let mut s = Server::new(base(), "", Limits::default());
    let mut a = join_as(&mut s, Access::Edit);
    let mut r = Server::restore(s.state().unwrap(), "", Limits::default()).unwrap();
    assert!(matches!(catch_up_of(&mut r, &mut a), message::CatchUp::Delta(_)));
}
