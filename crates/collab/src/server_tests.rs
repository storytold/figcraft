//! The server's ordering and validation, and the client's reconciliation, message by message.

use figcraft_doc::{Document, Node, NodeId, NodeKind};

use crate::change::{Change, Entry, Json, Prop};
use crate::message::{ClientMsg, Hello, PROTOCOL, Presence, ServerMsg};
use crate::replica::Mode;
use crate::{Client, Limits, NoRecompute, Outgoing, Server};

fn hello(token: &str) -> Hello {
    let ClientMsg::Hello(h) = Client::hello("f", token, "Ada", "#FF8800") else { panic!("not a hello") };
    h
}

fn doc() -> (Document, NodeId, NodeId) {
    let mut d = Document::new("Shared");
    let page = d.first_page().unwrap();
    let frame = d.create(page, NodeKind::Frame, "Frame", 0.0, 0.0, 100.0, 100.0).unwrap();
    (d, page, frame)
}

/// Join and return the client built from its welcome.
fn join(server: &mut Server) -> Client {
    let (id, out) = server.join(hello("tok")).unwrap();
    let Some(Outgoing::To(to, welcome)) = out.into_iter().next() else { panic!("no welcome") };
    assert_eq!(to, id);
    Client::join(welcome).unwrap()
}

/// The message the server sends `to` among `out`.
fn for_client(out: &[Outgoing], to: u32) -> Vec<ServerMsg> {
    out.iter()
        .filter_map(|o| match o {
            Outgoing::To(t, m) if *t == to => Some(m.clone()),
            Outgoing::All(m) => Some(m.clone()),
            Outgoing::Others(not, m) if *not != to => Some(m.clone()),
            _ => None,
        })
        .collect()
}

fn edit(c: &mut Client, f: impl FnOnce(&mut Document)) -> ClientMsg {
    let mut d = c.document().clone();
    f(&mut d);
    c.edit(&d).unwrap().expect("a change")
}

#[test]
fn joining_checks_protocol_token_and_room() {
    let (d, _, _) = doc();
    let mut s = Server::new(d, "tok", Limits { max_peers: 2, ..Limits::default() });
    assert!(matches!(s.join(Hello { protocol: PROTOCOL + 1, ..hello("tok") }), Err(ServerMsg::Refused { .. })));
    assert!(matches!(s.join(hello("nope")), Err(ServerMsg::Refused { .. })));
    assert!(matches!(s.join(Hello { name: "x".repeat(65), ..hello("tok") }), Err(ServerMsg::Refused { .. })));
    let a = join(&mut s);
    let (b_id, out) = s.join(hello("tok")).unwrap();
    assert!(
        out.contains(&Outgoing::Others(b_id, ServerMsg::PeerJoined(crate::Peer { client_id: b_id, name: "Ada".into(), color: "#FF8800".into() })))
    );
    assert!(matches!(s.join(hello("tok")), Err(ServerMsg::Refused { .. })), "full");
    assert_ne!(a.id(), b_id);
    assert_eq!(a.document(), s.document(), "the welcome carries the document");
    assert_eq!(a.view(), s.replica());
    // Leaving frees the seat and tells the others.
    assert_eq!(s.leave(b_id), vec![Outgoing::Others(b_id, ServerMsg::PeerLeft { client_id: b_id })]);
    assert!(s.leave(b_id).is_empty());
    assert!(s.join(hello("tok")).is_ok());
}

#[test]
fn client_ids_skip_spaces_used_in_the_file() {
    let (mut d, page, _) = doc();
    d.insert(Node::new(NodeId((7 << 32) | 3), NodeKind::Rectangle, "from session 7", 1.0, 1.0), page, None).unwrap();
    let mut s = Server::new(d, "tok", Limits::default());
    assert_eq!(join(&mut s).id(), 8);
}

#[test]
fn batches_are_ordered_echoed_and_rebroadcast() {
    let (d, page, frame) = doc();
    let mut s = Server::new(d, "tok", Limits::default());
    let mut a = join(&mut s);
    let mut b = join(&mut s);
    let msg = edit(&mut a, |d| {
        d.create(frame, NodeKind::Rectangle, "R", 1.0, 2.0, 3.0, 4.0).unwrap();
    });
    assert_eq!(a.pending(), 1);
    let out = s.receive(a.id(), msg);
    assert_eq!(s.seq(), 1);
    for m in for_client(&out, a.id()) {
        assert!(a.receive(m, &NoRecompute).unwrap().is_empty());
    }
    for m in for_client(&out, b.id()) {
        b.receive(m, &NoRecompute).unwrap();
    }
    assert_eq!(a.pending(), 0);
    assert_eq!(a.view(), s.replica());
    assert_eq!(b.view(), s.replica());
    // The new node's id is in a's space.
    let r = s.document().node(frame).unwrap().children[0];
    assert_eq!(r.0 >> 32, u64::from(a.id()));
    // Ping, ack, presence.
    assert_eq!(s.receive(a.id(), ClientMsg::Ping), vec![Outgoing::To(a.id(), ServerMsg::Pong)]);
    assert!(s.receive(a.id(), ClientMsg::Ack { seq: 99 }).is_empty());
    assert_eq!(s.acked(a.id()), Some(1));
    let p = Presence { cursor: Some((page, 1.0, 2.0)), ..Default::default() };
    let out = s.receive(a.id(), ClientMsg::Presence(p.clone()));
    for m in for_client(&out, b.id()) {
        b.receive(m, &NoRecompute).unwrap();
    }
    assert_eq!(b.presence_of(a.id()), Some(&p));
    assert!(for_client(&out, a.id()).is_empty(), "not echoed");
    // Out-of-place messages.
    assert!(matches!(s.receive(a.id(), ClientMsg::Hello(hello("tok"))).as_slice(), [Outgoing::To(_, ServerMsg::Refused { .. })]));
    assert!(matches!(s.receive(999, ClientMsg::Ping).as_slice(), [Outgoing::To(999, ServerMsg::Refused { .. })]));
}

/// A new batch id (the server ignores a batch id it already processed).
fn next_batch() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1000);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn rejected(s: &mut Server, from: u32, changes: Vec<Change>) -> (String, Vec<Change>) {
    let batch = next_batch();
    let before = s.replica().clone();
    let out = s.receive(from, ClientMsg::Changes { base_seq: s.seq(), batch_id: batch, changes });
    assert_eq!(s.replica(), &before, "nothing applied");
    match out.as_slice() {
        [Outgoing::To(to, ServerMsg::Reject { batch_id, reason, fixes })] if *to == from && *batch_id == batch => (reason.clone(), fixes.clone()),
        other => panic!("not rejected: {other:?}"),
    }
}

#[test]
fn invalid_batches_are_rejected_whole() {
    let (d, page, frame) = doc();
    let mut s = Server::new(d, "tok", Limits { max_changes: 3, ..Limits::default() });
    let a = join(&mut s);
    let me = u64::from(a.id());
    let create = |id: u64, parent: NodeId| Change::Create {
        id: NodeId(id),
        kind: Json::of(&NodeKind::Frame).unwrap(),
        parent: Some(parent),
        position: "a5".into(),
        props: vec![],
    };
    let set = |id: NodeId, prop: Prop, v: serde_json::Value| Change::Set { id, prop, value: Json(v) };
    // Someone else's id space, or the old ids of the file.
    rejected(&mut s, a.id(), vec![create(((me + 1) << 32) | 1, frame)]);
    rejected(&mut s, a.id(), vec![create(77, frame)]);
    // A valid change with an invalid one: neither applies.
    let (reason, _) = rejected(&mut s, a.id(), vec![set(frame, Prop::Name, "ok".into()), set(NodeId(4242), Prop::Name, "x".into())]);
    assert!(reason.contains("deleted by someone else"), "{reason}");
    // A cycle through a new frame.
    rejected(&mut s, a.id(), vec![create((me << 32) | 1, frame), set(frame, Prop::Parent, serde_json::json!((me << 32) | 1))]);
    // Too many changes; based on the future.
    rejected(&mut s, a.id(), vec![set(frame, Prop::Name, "a".into()); 4]);
    let out = s.receive(a.id(), ClientMsg::Changes { base_seq: 5, batch_id: next_batch(), changes: vec![] });
    assert!(matches!(out.as_slice(), [Outgoing::To(_, ServerMsg::Reject { .. })]));
    // Deleting the last page.
    rejected(&mut s, a.id(), vec![Change::Delete { id: page }]);
    assert_eq!(s.seq(), 0);
}

#[test]
fn deleted_nodes_can_come_back_but_ids_are_never_handed_out_twice() {
    let (d, _, frame) = doc();
    let mut s = Server::new(d, "tok", Limits::default());
    let mut a = join(&mut s);
    let b = join(&mut s);
    let gone = s.replica().clone();
    let snap = crate::replica::create_change(gone.document().node(frame).unwrap(), gone.key(frame)).unwrap();
    // b deletes the frame (an old id); a brings it back, as undo would.
    let out = s.receive(b.id(), ClientMsg::Changes { base_seq: 0, batch_id: 1, changes: vec![Change::Delete { id: frame }] });
    assert!(matches!(out.as_slice(), [Outgoing::All(ServerMsg::Changes(..))]));
    for m in for_client(&out, a.id()) {
        a.receive(m, &NoRecompute).unwrap();
    }
    let out = s.receive(a.id(), ClientMsg::Changes { base_seq: 1, batch_id: 1, changes: vec![snap.clone()] });
    assert!(matches!(out.as_slice(), [Outgoing::All(ServerMsg::Changes(..))]), "{out:?}");
    assert_eq!(s.document(), gone.document());
}

#[test]
fn fixes_roll_a_rejected_client_back_to_the_server_state() {
    let (d, _, frame) = doc();
    let mut s = Server::new(d, "tok", Limits::default());
    let mut a = join(&mut s);
    let mut b = join(&mut s);
    // b deletes the frame; meanwhile a renames it and adds a rectangle inside.
    let del = edit(&mut b, |d| d.remove(frame).unwrap());
    let mine = edit(&mut a, |d| {
        d.get_mut(frame).unwrap().name = "Mine".into();
        d.create(frame, NodeKind::Rectangle, "R", 0.0, 0.0, 5.0, 5.0).unwrap();
    });
    let optimistic = a.view().clone();
    let out_b = s.receive(b.id(), del);
    let out_a = s.receive(a.id(), mine);
    let [Outgoing::To(_, ServerMsg::Reject { fixes, .. })] = out_a.as_slice() else { panic!("{out_a:?}") };
    // A client without a confirmed copy can roll back with the fixes: they delete what the server
    // doesn't have (b's delete arrives first on a's connection).
    let mut rolled = optimistic.clone();
    let ServerMsg::Changes(crate::Accepted { changes, .. }) = &for_client(&out_b, a.id())[0] else { panic!() };
    rolled.apply(changes, Mode::Lenient).unwrap();
    rolled.apply(fixes, Mode::Lenient).unwrap();
    assert_eq!(rolled.document(), s.document());
    // The real client replays from its confirmed copy instead.
    for m in for_client(&out_b, a.id()).into_iter().chain(for_client(&out_a, a.id())) {
        a.receive(m, &NoRecompute).unwrap();
    }
    for m in for_client(&out_b, b.id()) {
        b.receive(m, &NoRecompute).unwrap();
    }
    assert_eq!(a.pending(), 0);
    assert_eq!(a.view(), s.replica());
    assert_eq!(b.view(), s.replica());
}

#[test]
fn a_refused_deep_delete_restores_each_node_once_in_linear_time() {
    const DEPTH: usize = 20_000;
    let (mut d, _, frame) = doc();
    let mut chain = vec![frame];
    while chain.len() < DEPTH {
        let parent = *chain.last().unwrap();
        chain.push(d.create(parent, NodeKind::Frame, "F", 0.0, 0.0, 1.0, 1.0).unwrap());
    }
    let mut s = Server::new(d, "tok", Limits::default());
    let a = join(&mut s);
    // Every node of the chain deleted, plus a rename of a node nobody has: refused whole.
    let missing = NodeId(u64::MAX >> 1);
    let mut changes: Vec<Change> = chain.iter().map(|&id| Change::Delete { id }).collect();
    changes.push(Change::Set { id: missing, prop: Prop::Name, value: Json("x".into()) });
    // As many comment threads put, each twice: they go back to not existing, once each.
    let put = |c: u64| Change::Put { entry: Entry::Comment(c), value: Json(serde_json::json!({})) };
    changes.extend((1..=DEPTH as u64).chain(1..=DEPTH as u64).map(put));
    let started = std::time::Instant::now();
    let (_, fixes) = rejected(&mut s, a.id(), changes);
    let took = started.elapsed();
    assert_eq!(fixes.first(), Some(&Change::Delete { id: missing }));
    let restored: Vec<NodeId> = fixes[1..=DEPTH]
        .iter()
        .map(|c| match c {
            Change::Create { id, .. } => *id,
            other => panic!("not a creation: {other:?}"),
        })
        .collect();
    assert_eq!(restored, chain, "each deleted node comes back once, parents first");
    let removed: Vec<Change> = (1..=DEPTH as u64).map(|c| Change::Remove { entry: Entry::Comment(c) }).collect();
    assert_eq!(fixes[DEPTH + 1..], removed[..], "each entry once, in batch order");
    // Walking every deleted node's subtree again and comparing each fix with all the earlier
    // ones took 27 s in a debug build; once each takes 0.1 s.
    assert!(took < std::time::Duration::from_secs(10), "{took:?}");
}

#[test]
fn remote_batches_apply_under_pending_ones() {
    let (d, _, frame) = doc();
    let mut s = Server::new(d, "tok", Limits::default());
    let mut a = join(&mut s);
    let mut b = join(&mut s);
    // a renames the frame and b moves it, at the same time; b's lands first.
    let rename = edit(&mut a, |d| d.get_mut(frame).unwrap().name = "A's name".into());
    let mv = edit(&mut b, |d| d.get_mut(frame).unwrap().transform = figcraft_geom::Affine::translate((50.0, 0.0)));
    let out_b = s.receive(b.id(), mv);
    for m in for_client(&out_b, a.id()) {
        a.receive(m, &NoRecompute).unwrap();
    }
    // a sees b's move and keeps its own pending rename.
    let n = a.document().node(frame).unwrap();
    assert_eq!((n.name.as_str(), n.transform.translation().x), ("A's name", 50.0));
    assert_eq!(a.pending(), 1);
    let out_a = s.receive(a.id(), rename);
    for m in for_client(&out_a, a.id()) {
        a.receive(m, &NoRecompute).unwrap();
    }
    for m in for_client(&out_b, b.id()).into_iter().chain(for_client(&out_a, b.id())) {
        b.receive(m, &NoRecompute).unwrap();
    }
    assert_eq!(a.view(), s.replica());
    assert_eq!(b.view(), s.replica());
    // Messages out of order are protocol errors.
    let stale = ServerMsg::Changes(crate::Accepted { seq: 1, author: b.id(), batch_id: 1, changes: vec![] });
    assert!(a.receive(stale, &NoRecompute).is_err());
    assert!(a.receive(ServerMsg::Reject { batch_id: 9, reason: String::new(), fixes: vec![] }, &NoRecompute).is_err());
    assert!(Client::join(ServerMsg::Pong).is_err());
}

#[test]
fn junk_batches_never_panic_and_never_corrupt_the_document() {
    use crate::change::{DocProp, Entry};
    use crate::sim::Rng;
    let (d, _, _) = doc();
    let mut s = Server::new(d, "tok", Limits::default());
    let a = join(&mut s);
    let me = u64::from(a.id());
    let mut rng = Rng::new(5);
    let values = [
        serde_json::json!(null),
        serde_json::json!(-1),
        serde_json::json!(1e300),
        serde_json::json!("a0V"),
        serde_json::json!("x"),
        serde_json::json!([1, 2, 3]),
        serde_json::json!({"type": "frame"}),
        serde_json::json!({"type": "page"}),
        serde_json::json!({"type": "polygon", "count": 4_000_000_000u64}),
        serde_json::json!([1.0, 0.0, 0.0, 1.0, 5.0, 5.0]),
        serde_json::json!(true),
    ];
    for round in 0..3000u64 {
        let ids: Vec<NodeId> = s.document().ids().collect();
        let id = |rng: &mut Rng| match rng.below(3) {
            0 => NodeId((me << 32) | rng.below(50) as u64),
            1 => NodeId(rng.below(10) as u64),
            _ => rng.pick(&ids).copied().unwrap_or(NodeId(1)),
        };
        let v = |rng: &mut Rng| Json(rng.pick(&values).cloned().unwrap_or_default());
        let n = 1 + rng.below(4);
        let mut changes = Vec::new();
        for _ in 0..n {
            let target = id(&mut rng);
            let parent = if rng.chance(0.2) { None } else { Some(id(&mut rng)) };
            changes.push(match rng.below(7) {
                0 => {
                    Change::Create { id: target, kind: v(&mut rng), parent, position: ["a0", "a1", "Zz", "bad"][rng.below(4)].into(), props: vec![] }
                }
                1 | 2 => Change::Set { id: target, prop: *rng.pick(Prop::FIELDS).unwrap_or(&Prop::Name), value: v(&mut rng) },
                3 => Change::Set {
                    id: target,
                    prop: if rng.chance(0.5) { Prop::Parent } else { Prop::Position },
                    value: Json(serde_json::json!(id(&mut rng).0)),
                },
                4 => Change::Delete { id: target },
                5 => Change::SetDoc {
                    prop: [DocProp::Name, DocProp::StyleOrder, DocProp::Libraries, DocProp::CurrentPage][rng.below(4)],
                    value: v(&mut rng),
                },
                _ => Change::Put { entry: Entry::Comment(rng.below(3) as u64), value: v(&mut rng) },
            });
        }
        let _ = s.receive(a.id(), ClientMsg::Changes { base_seq: s.seq(), batch_id: round + 1, changes });
        assert!(s.document().validate().is_empty(), "{:?}", s.document().validate());
    }
}
