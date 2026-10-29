//! Dropped connections: the client keeps editing offline, reconnects with its id and last
//! sequence number, catches up with the batches it missed (or a snapshot once they're no longer
//! logged), settles what the server did with its batches meanwhile, and sends the rest.

use figcraft_doc::{Document, NodeKind};

use crate::message::{self, ClientMsg, Hello, ServerMsg};
use crate::sim::Sim;
use crate::sim_tests::{Width, base};
use crate::undo_tests::run_with_undo;
use crate::{BatchKind, CatchUp, Client, Limits, NoRecompute, Notice, Outgoing, Server};

fn hello() -> Hello {
    let ClientMsg::Hello(h) = Client::hello("f", "tok", "Ada", "#FF8800") else { panic!("not a hello") };
    h
}

/// Join `s` and return the client and the welcome's other messages.
fn join(s: &mut Server) -> Client {
    let (_, out) = s.join(hello()).unwrap();
    let Some(Outgoing::To(_, welcome)) = out.into_iter().next() else { panic!("no welcome") };
    Client::join(welcome).unwrap()
}

/// Deliver everything in `out` meant for `c`.
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

/// `c` comes back: it says hello again and gets its welcome. Returns what it sends then.
fn come_back(s: &mut Server, c: &mut Client) -> Vec<ClientMsg> {
    let ClientMsg::Hello(h) = c.reconnect(hello()) else { panic!() };
    let (id, out) = s.join(h).unwrap();
    assert_eq!(id, c.id(), "a reconnecting client keeps its id");
    deliver(c, &out)
}

fn edit(c: &mut Client, f: impl FnOnce(&mut Document)) -> Option<ClientMsg> {
    let mut d = c.document().clone();
    f(&mut d);
    c.edit(&d).unwrap()
}

#[test]
fn a_batch_whose_confirmation_was_lost_is_confirmed_by_the_catch_up() {
    let mut s = Server::new(base(), "tok", Limits::default());
    let mut a = join(&mut s);
    let page = s.document().first_page().unwrap();
    let msg = edit(&mut a, |d| {
        d.create(page, NodeKind::Rectangle, "R", 0.0, 0.0, 5.0, 5.0).unwrap();
    })
    .unwrap();
    // The server applies it, but the connection drops before the echo arrives.
    let _lost = s.receive(a.id(), msg);
    a.disconnect();
    s.leave(a.id());
    let resend = come_back(&mut s, &mut a);
    assert!(resend.is_empty(), "nothing is sent twice: {resend:?}");
    assert_eq!(a.pending(), 0);
    assert_eq!(a.view(), s.replica());
    assert!(a.can_undo(), "the confirmed edit stays undoable");
}

#[test]
fn a_batch_refused_while_offline_is_dropped_with_a_reason() {
    let mut s = Server::new(base(), "tok", Limits::default());
    let mut a = join(&mut s);
    let mut b = join(&mut s);
    let page = s.document().first_page().unwrap();
    let f = s.document().node(page).unwrap().children[0];
    let del = edit(&mut b, |d| d.remove(f).unwrap()).unwrap();
    let rename = edit(&mut a, |d| d.node_mut(f).unwrap().name = "Mine".into()).unwrap();
    let out = s.receive(b.id(), del);
    deliver(&mut b, &out);
    // a's rename is refused, but a never hears it.
    let _lost = s.receive(a.id(), rename);
    a.disconnect();
    s.leave(a.id());
    // a keeps working offline: queued, nothing goes out.
    assert!(edit(&mut a, |d| d.node_mut(page).unwrap().name = "Page A".into()).is_none());
    assert_eq!(a.pending(), 2);
    let resend = come_back(&mut s, &mut a);
    assert_eq!(resend.len(), 1, "the offline edit goes out");
    let notices = a.take_notices();
    assert!(matches!(notices.as_slice(), [Notice::Rejected { kind: BatchKind::Edit, .. }]), "{notices:?}");
    let out = s.receive(a.id(), resend.into_iter().next().unwrap());
    deliver(&mut a, &out);
    deliver(&mut b, &out);
    assert_eq!(a.view(), s.replica());
    assert_eq!(b.view(), s.replica());
    assert_eq!(s.document().node(page).unwrap().name, "Page A");
}

#[test]
fn a_long_absence_catches_up_with_a_snapshot() {
    let mut s = Server::new(base(), "tok", Limits { max_log_batches: 3, ..Limits::default() });
    let mut a = join(&mut s);
    let mut b = join(&mut s);
    let page = s.document().first_page().unwrap();
    a.disconnect();
    s.leave(a.id());
    for i in 0..5 {
        let m = edit(&mut b, |d| d.node_mut(page).unwrap().name = format!("Page {i}")).unwrap();
        let out = s.receive(b.id(), m);
        deliver(&mut b, &out);
    }
    let ClientMsg::Hello(h) = a.reconnect(hello()) else { panic!() };
    let (_, out) = s.join(h).unwrap();
    assert!(matches!(&out[0], Outgoing::To(_, ServerMsg::Welcome { catch_up: CatchUp::Snapshot(_), .. })));
    deliver(&mut a, &out);
    assert_eq!(a.view(), s.replica());
    // A short absence gets the missed batches only.
    a.disconnect();
    s.leave(a.id());
    let m = edit(&mut b, |d| d.node_mut(page).unwrap().name = "Later".into()).unwrap();
    let out = s.receive(b.id(), m);
    deliver(&mut b, &out);
    let ClientMsg::Hello(h) = a.reconnect(hello()) else { panic!() };
    let (_, out) = s.join(h).unwrap();
    assert!(matches!(&out[0], Outgoing::To(_, ServerMsg::Welcome { catch_up: CatchUp::Delta(d), .. }) if d.len() == 1));
    deliver(&mut a, &out);
    assert_eq!(a.view(), s.replica());
}

#[test]
fn the_default_log_is_bounded_in_bytes_and_a_client_behind_it_catches_up_with_a_snapshot() {
    // The defaults are what a desktop host runs with.
    let limits = Limits::default();
    assert_eq!(limits.max_log_bytes, 64 << 20);
    let mut s = Server::new(base(), "tok", limits);
    let mut a = join(&mut s);
    let mut b = join(&mut s);
    let page = s.document().first_page().unwrap();
    a.disconnect();
    s.leave(a.id());
    // Nine renames of 8 MiB each: more than the log holds, in a few batches.
    for i in 0..9 {
        let m = edit(&mut b, |d| d.node_mut(page).unwrap().name = format!("{i}{}", "y".repeat(8 << 20))).unwrap();
        let out = s.receive(b.id(), m);
        deliver(&mut b, &out);
        assert!(s.log_bytes() <= limits.max_log_bytes, "{} bytes kept", s.log_bytes());
    }
    assert!(s.log_bytes() > 0, "recent batches are still kept");
    let ClientMsg::Hello(h) = a.reconnect(hello()) else { panic!() };
    let (_, out) = s.join(h).unwrap();
    assert!(matches!(&out[0], Outgoing::To(_, ServerMsg::Welcome { catch_up: CatchUp::Snapshot(_), .. })));
    deliver(&mut a, &out);
    assert_eq!(a.view(), s.replica());
    // Within the log, a short absence still gets the missed batch only.
    a.disconnect();
    s.leave(a.id());
    let m = edit(&mut b, |d| d.node_mut(page).unwrap().name = "Later".into()).unwrap();
    let out = s.receive(b.id(), m);
    deliver(&mut b, &out);
    let ClientMsg::Hello(h) = a.reconnect(hello()) else { panic!() };
    let (_, out) = s.join(h).unwrap();
    assert!(matches!(&out[0], Outgoing::To(_, ServerMsg::Welcome { catch_up: CatchUp::Delta(d), .. }) if d.len() == 1));
    deliver(&mut a, &out);
    assert_eq!(a.view(), s.replica());
}

#[test]
fn after_a_snapshot_undo_leaves_what_others_changed_meanwhile() {
    let mut s = Server::new(base(), "tok", Limits { max_log_batches: 2, ..Limits::default() });
    let mut a = join(&mut s);
    let mut b = join(&mut s);
    let page = s.document().first_page().unwrap();
    let f = s.document().node(page).unwrap().children[0];
    // a renames the page and the frame (one step), confirmed; then goes away for a while.
    let m = edit(&mut a, |d| {
        d.node_mut(page).unwrap().name = "A page".into();
        d.node_mut(f).unwrap().name = "A frame".into();
    })
    .unwrap();
    let out = s.receive(a.id(), m);
    deliver(&mut a, &out);
    deliver(&mut b, &out);
    a.disconnect();
    s.leave(a.id());
    for i in 0..4 {
        let m = edit(&mut b, |d| d.node_mut(f).unwrap().name = format!("B frame {i}")).unwrap();
        let out = s.receive(b.id(), m);
        deliver(&mut b, &out);
    }
    let ClientMsg::Hello(h) = a.reconnect(hello()) else { panic!() };
    let (_, out) = s.join(h).unwrap();
    assert!(matches!(&out[0], Outgoing::To(_, ServerMsg::Welcome { catch_up: CatchUp::Snapshot(_), .. })));
    deliver(&mut a, &out);
    // Undo: the page name goes back, the frame keeps b's later name.
    let msgs = a.undo(&NoRecompute).unwrap();
    assert_eq!(msgs.len(), 1);
    let out = s.receive(a.id(), msgs.into_iter().next().unwrap());
    deliver(&mut a, &out);
    assert_eq!(s.document().node(page).unwrap().name, "Page 1");
    assert_eq!(s.document().node(f).unwrap().name, "B frame 3");
    assert_eq!(a.view(), s.replica());
}

#[test]
fn a_copy_sent_again_is_ignored_and_a_stale_connection_is_taken_over() {
    let mut s = Server::new(base(), "tok", Limits::default());
    let mut a = join(&mut s);
    let page = s.document().first_page().unwrap();
    let m = edit(&mut a, |d| d.node_mut(page).unwrap().name = "Once".into()).unwrap();
    let out = s.receive(a.id(), m.clone());
    assert_eq!(s.seq(), 1);
    // The same batch again (a copy sent around a reconnect): nothing happens.
    assert!(s.receive(a.id(), m).is_empty());
    assert_eq!(s.seq(), 1);
    deliver(&mut a, &out);
    // a comes back before the server noticed its old connection is gone: same id, no "full".
    let mut tight = Server::new(base(), "tok", Limits { max_peers: 1, ..Limits::default() });
    let mut b = join(&mut tight);
    let resend = come_back(&mut tight, &mut b);
    assert!(resend.is_empty());
    assert_eq!(tight.peers().count(), 1);
    assert_eq!(b.view(), tight.replica());
}

#[test]
fn an_unknown_client_joins_as_new_and_its_queue_is_dropped() {
    let mut s = Server::new(base(), "tok", Limits::default());
    let mut a = join(&mut s);
    let page = s.document().first_page().unwrap();
    a.disconnect();
    edit(&mut a, |d| d.node_mut(page).unwrap().name = "Offline".into());
    // A different session (it never gave out a's id).
    let mut other = Server::new(base(), "tok", Limits::default());
    join(&mut other);
    let ClientMsg::Hello(h) = a.reconnect(hello()) else { panic!() };
    let (id, out) = other.join(h).unwrap();
    let Some(Outgoing::To(_, welcome)) = out.into_iter().next() else { panic!("no welcome") };
    assert!(a.receive(welcome, &NoRecompute).unwrap().is_empty());
    assert_eq!(a.id(), id);
    assert_eq!(a.pending(), 0);
    assert_eq!(a.view(), other.replica());
    assert!(matches!(a.take_notices().as_slice(), [Notice::Lost { batches: 1 }]));
}

#[test]
fn the_session_state_survives_a_restart() {
    let mut s = Server::new(base(), "tok", Limits::default());
    let mut a = join(&mut s);
    let page = s.document().first_page().unwrap();
    let m = edit(&mut a, |d| d.node_mut(page).unwrap().name = "Saved".into()).unwrap();
    let out = s.receive(a.id(), m);
    deliver(&mut a, &out);
    // Saved in both encodings and read back.
    let state = s.state().unwrap();
    let bytes = message::encode(&state).unwrap();
    assert_eq!(message::decode::<crate::SessionState>(&bytes, usize::MAX).unwrap(), state);
    let json = message::encode_json(&state).unwrap();
    let state = message::decode_json::<crate::SessionState>(&json, usize::MAX).unwrap();
    let mut restarted = Server::restore(state, "tok", Limits::default()).unwrap();
    assert_eq!(restarted.replica(), s.replica());
    assert_eq!(restarted.seq(), s.seq());
    // The client-id counter carries on: a new participant never gets an id handed out before.
    let b = join(&mut restarted);
    assert!(b.id() > a.id());
    // a reconnects to the restarted server and keeps going.
    a.disconnect();
    let resend = come_back(&mut restarted, &mut a);
    assert!(resend.is_empty());
    assert_eq!(a.view(), restarted.replica());
}

#[test]
fn random_drops_and_reconnects_converge() {
    for seed in 0..10 {
        run_with_undo(seed, 3, 90, &NoRecompute, true, None);
    }
    // A short log: long absences fall back to snapshots.
    let mut snapshots = 0;
    for seed in 20..30 {
        let limits = Limits { max_log_batches: 4, ..Limits::default() };
        let sim = Sim::with_limits(base(), 3, seed, 12, limits).unwrap();
        let sim = run_with_undo(seed, 3, 90, &Width, true, Some(sim));
        snapshots += sim.snapshots;
    }
    assert!(snapshots > 0, "no reconnect needed a snapshot");
}

#[test]
fn resume_needs_the_issued_secret() {
    let mut s = Server::new(base(), "tok", Limits { max_peers: 2, ..Limits::default() });
    let mut host = join(&mut s);
    let guest = join(&mut s);
    // A participant who knows the (public) session and the host's id can't take that id over,
    // whether the session is full or not, and with view-only access or not.
    let forged = message::Resume { client_id: host.id(), ..guest.resume() };
    assert_eq!(forged.session, host.resume().session);
    let refused = s.join_as(Hello { resume: Some(forged), ..hello() }, message::Access::View);
    assert!(matches!(refused, Err(ServerMsg::Refused { .. })), "a forged resume of a live id is refused: {refused:?}");
    s.leave(guest.id());
    let (id, _) = s.join_as(Hello { resume: Some(forged), ..hello() }, message::Access::View).unwrap();
    assert_ne!(id, host.id(), "a forged resume joins as new");
    // The real client still comes back as itself.
    let resend = come_back(&mut s, &mut host);
    assert!(resend.is_empty());
}
