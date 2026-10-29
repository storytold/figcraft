//! Diffing and applying documents directly (the engine-level round trips run real commands).

use std::collections::BTreeMap;
use std::sync::Arc;

use figcraft_color::{Color, Paint};
use figcraft_doc::{Comment, DevSnapshot, Document, ImageData, Message, Node, NodeId, NodeKind, Style, StyleKind};

use crate::change::{Change, DocProp, Entry, Json, Prop};
use crate::replica::{Mode, Replica};
use crate::{Error, diff, message};

/// `apply(diff(a, b), a) == b`, positions included, also after an encode/decode round trip.
fn round_trip(a: &Replica, b: &Document) -> Vec<Change> {
    let d = diff(a, b).unwrap();
    let mut r = a.clone();
    r.apply(&d.changes, Mode::Strict).unwrap();
    assert_eq!(r.document(), b, "{:#?}", d.changes);
    assert_eq!(r, d.replica, "positions agree");
    let wire: Vec<Change> = message::decode(&message::encode(&d.changes).unwrap(), usize::MAX).unwrap();
    assert_eq!(wire, d.changes, "binary round trip");
    let json: Vec<Change> = message::decode_json(&message::encode_json(&d.changes).unwrap(), usize::MAX).unwrap();
    assert_eq!(json, d.changes, "JSON round trip");
    d.changes
}

fn rect(doc: &mut Document, parent: NodeId, name: &str) -> NodeId {
    doc.create(parent, NodeKind::Rectangle, name, 0.0, 0.0, 10.0, 10.0).unwrap()
}

fn setup() -> (Replica, NodeId, NodeId, Vec<NodeId>) {
    let mut doc = Document::new("File");
    let page = doc.first_page().unwrap();
    let frame = doc.create(page, NodeKind::Frame, "Frame", 0.0, 0.0, 200.0, 200.0).unwrap();
    let kids = (0..5).map(|i| rect(&mut doc, frame, &format!("R{i}"))).collect();
    (Replica::new(doc), page, frame, kids)
}

#[test]
fn property_edits_send_only_what_changed() {
    let (a, _, _, kids) = setup();
    let mut b = a.document().clone();
    b.get_mut(kids[1]).unwrap().opacity = 0.5;
    b.get_mut(kids[1]).unwrap().fills = vec![Paint::solid(Color::from_rgb8_const(1, 2, 3))];
    let changes = round_trip(&a, &b);
    assert_eq!(changes.len(), 2);
    assert!(changes.iter().all(|c| matches!(c, Change::Set { id, prop: Prop::Opacity | Prop::Fills, .. } if *id == kids[1])));
    assert!(round_trip(&a, a.document()).is_empty(), "nothing changed");
}

#[test]
fn creations_moves_reorders_and_deletions_round_trip() {
    let (a, page, frame, kids) = setup();
    let mut b = a.document().clone();
    // A new group with a new child inside the frame, one rectangle moved into it, one deleted,
    // one reordered to the bottom, one moved to the page.
    let g = b.create(frame, NodeKind::Group, "G", 5.0, 5.0, 1.0, 1.0).unwrap();
    let inner = rect(&mut b, g, "Inner");
    b.place(kids[0], g, 0).unwrap();
    b.remove(kids[1]).unwrap();
    b.reorder(kids[4], 0).unwrap();
    b.place(kids[2], page, 0).unwrap();
    let changes = round_trip(&a, &b);
    let creates: Vec<NodeId> = changes.iter().filter_map(|c| if let Change::Create { id, .. } = c { Some(*id) } else { None }).collect();
    assert_eq!(creates, vec![g, inner], "parents before children");
    assert!(changes.contains(&Change::Delete { id: kids[1] }));
    // kids[3] kept its place and key.
    assert!(!changes.iter().any(|c| c.node() == Some(kids[3])));
}

#[test]
fn deleting_a_subtree_sends_one_delete_and_moves_out_first() {
    let (a, page, frame, kids) = setup();
    let mut b = a.document().clone();
    b.place(kids[0], page, 0).unwrap();
    b.remove(frame).unwrap();
    let changes = round_trip(&a, &b);
    let deletes: Vec<&Change> = changes.iter().filter(|c| matches!(c, Change::Delete { .. })).collect();
    assert_eq!(deletes, vec![&Change::Delete { id: frame }]);
    let moved = changes.iter().position(|c| matches!(c, Change::Set { id, prop: Prop::Parent, .. } if *id == kids[0])).unwrap();
    let deleted = changes.iter().position(|c| matches!(c, Change::Delete { .. })).unwrap();
    assert!(moved < deleted);
}

#[test]
fn pages_add_reorder_and_delete() {
    let (a, page, _, _) = setup();
    let mut b = a.document().clone();
    let p2 = b.add_page("Two");
    let p3 = b.add_page("Three");
    rect(&mut b, p3, "On three");
    b.pages = vec![p3, page, p2];
    let after = round_trip(&a, &b);
    assert!(after.iter().any(|c| matches!(c, Change::Create { id, parent: None, .. } if *id == p3)));
    let a2 = diff(&a, &b).unwrap().replica;
    let mut c = b.clone();
    c.remove(page).unwrap();
    round_trip(&a2, &c);
}

#[test]
fn kind_changes_and_transforms_round_trip() {
    let (a, _, frame, kids) = setup();
    let mut b = a.document().clone();
    b.get_mut(frame).unwrap().kind = NodeKind::Component { description: "A card".into(), props: BTreeMap::new(), prop_order: Vec::new() };
    b.get_mut(kids[0]).unwrap().kind = NodeKind::Ellipse { start: 0.0, sweep: 3.0, inner: 0.25 };
    b.get_mut(kids[0]).unwrap().transform = figcraft_geom::Affine::new([0.5, 0.1, -0.1, 2.0, 1.0 / 3.0, -7.25]);
    round_trip(&a, &b);
}

fn style(id: &str) -> Style {
    Style {
        id: id.into(),
        name: format!("S {id}"),
        description: String::new(),
        kind: StyleKind::Paint { paints: vec![Paint::solid(Color::BLACK)] },
        bound: Default::default(),
    }
}

#[test]
fn document_level_data_round_trips() {
    let (a, page, _, kids) = setup();
    let mut b = a.document().clone();
    b.name = "Renamed".into();
    b.document_colors.push(Color::from_rgb8_const(9, 9, 9));
    b.styles = vec![style("s1"), style("s2"), style("s3")];
    b.comments.push(Comment {
        id: 7,
        page,
        x: 1.0,
        y: 2.0,
        anchor: None,
        first: Message { author: "Ada".into(), text: "Hi".into(), created: 1, edited: 0, reactions: vec![], agent: false },
        resolved: false,
        // An AI agent's reply reaches the others with its name and marker.
        replies: vec![Message { agent: true, ..Message::new("Claude (agent)", "Done", 2) }],
        read_by: None,
    });
    b.images.insert("hash".into(), ImageData { bytes: Arc::new(vec![1, 2, 3, 250]), width: 2, height: 1 });
    let snap = DevSnapshot { nodes: vec![Arc::new(b.node(kids[0]).unwrap().clone())] };
    b.dev_snapshots.insert(kids[0], Arc::new(snap));
    let changes = round_trip(&a, &b);
    assert!(changes.contains(&Change::SetDoc { prop: DocProp::Name, value: Json::of("Renamed").unwrap() }));
    let a2 = diff(&a, &b).unwrap().replica;
    // Edit one style, remove one, reorder: per-entry changes plus the order.
    let mut c = b.clone();
    c.styles[0].name = "First".into();
    c.styles.remove(1);
    c.styles.reverse();
    c.images.clear();
    c.comments[0].resolved = true;
    let changes = round_trip(&a2, &c);
    assert!(changes.contains(&Change::Remove { entry: Entry::Style("s2".into()) }));
    assert!(changes.contains(&Change::Remove { entry: Entry::Image("hash".into()) }));
    assert!(changes.iter().any(|ch| matches!(ch, Change::SetDoc { prop: DocProp::StyleOrder, .. })));
    // Duplicate ids fall back to the whole list.
    let a3 = diff(&a2, &c).unwrap().replica;
    let mut d = c.clone();
    d.styles.push(style("s1"));
    let changes = round_trip(&a3, &d);
    assert!(matches!(changes.as_slice(), [Change::SetDoc { prop: DocProp::Styles, .. }]));
}

#[test]
fn snapshots_rebuild_the_same_state() {
    let (a, page, frame, _) = setup();
    let mut b = a.document().clone();
    b.add_page("Second");
    b.name = "Snap".into();
    b.reorder(frame, 0).unwrap();
    rect(&mut b, page, "Top");
    b.styles.push(style("s"));
    let r = diff(&a, &b).unwrap().replica;
    let snap = r.snapshot().unwrap();
    let back = Replica::from_snapshot(&snap).unwrap();
    assert_eq!(back, r);
    assert!(Replica::from_snapshot(&[]).is_err(), "a snapshot needs a page");
}

#[test]
fn diffing_costs_the_size_of_the_edit() {
    let mut doc = Document::new("Big");
    let page = doc.first_page().unwrap();
    let ids: Vec<NodeId> = (0..20_000).map(|i| rect(&mut doc, page, &format!("R{i}"))).collect();
    let a = Replica::new(doc);
    let mut b = a.document().clone();
    b.get_mut(ids[123]).unwrap().name = "Edited".into();
    let t = std::time::Instant::now();
    let changes = round_trip(&a, &b);
    assert_eq!(changes.len(), 1);
    // Inserting one layer in the middle writes one key; its siblings keep theirs.
    let mut c = a.document().clone();
    let n = Node::new(c.alloc_id(), NodeKind::Ellipse { start: 0.0, sweep: 1.0, inner: 0.0 }, "Mid", 1.0, 1.0);
    c.insert(n, page, Some(10_000)).unwrap();
    let changes = round_trip(&a, &c);
    assert_eq!(changes.len(), 1);
    assert!(t.elapsed().as_secs_f64() < 5.0, "debug builds included");
}

#[test]
fn keys_respace_before_they_grow_long() {
    let (mut a, _, frame, _) = setup();
    // Keep inserting at the same spot: keys would grow, so the parent is re-spaced now and then.
    for i in 0..400 {
        let mut b = a.document().clone();
        let n = Node::new(b.alloc_id(), NodeKind::Rectangle, format!("N{i}"), 1.0, 1.0);
        b.insert(n, frame, Some(1)).unwrap();
        round_trip(&a, &b);
        a = diff(&a, &b).unwrap().replica;
    }
    assert!(a.positions().values().all(|k| k.len() <= crate::frac::KEY_RESPACE_LEN));
}

#[test]
fn invalid_changes_are_refused_and_lenient_mode_skips_them() {
    let (a, page, frame, kids) = setup();
    let set = |id: NodeId, prop: Prop, v: serde_json::Value| Change::Set { id, prop, value: Json(v) };
    let frame_kind = || Json::of(&NodeKind::Frame).unwrap();
    let bad: Vec<(Change, &str)> = vec![
        (set(NodeId(999), Prop::Name, "x".into()), "unknown node"),
        (set(kids[0], Prop::Opacity, "loud".into()), "wrong type"),
        (set(kids[0], Prop::Opacity, 2.0.into()), "out of range"),
        (set(kids[0], Prop::Width, (-1.0).into()), "negative size"),
        (set(frame, Prop::Parent, serde_json::json!(kids[0])), "into a leaf"),
        (set(frame, Prop::Parent, serde_json::Value::Null), "a frame without a parent"),
        (set(page, Prop::Parent, serde_json::json!(frame)), "a page into a frame"),
        (set(kids[0], Prop::Position, "not a key".into()), "bad key"),
        (set(kids[0], Prop::Kind, serde_json::json!({"type": "page"})), "becoming a page"),
        (set(frame, Prop::Kind, serde_json::json!({"type": "rectangle"})), "a leaf with children"),
        (Change::Create { id: kids[0], kind: frame_kind(), parent: Some(page), position: "a5".into(), props: vec![] }, "existing id"),
        (Change::Create { id: NodeId(500), kind: frame_kind(), parent: None, position: "a5".into(), props: vec![] }, "parentless frame"),
        (Change::Create { id: NodeId(500), kind: frame_kind(), parent: Some(NodeId(404)), position: "a5".into(), props: vec![] }, "missing parent"),
        (Change::Delete { id: page }, "the last page"),
        (Change::Put { entry: Entry::Style("a".into()), value: Json::of(&style("b")).unwrap() }, "entry id mismatch"),
    ];
    for (c, why) in bad {
        let mut r = a.clone();
        assert!(r.apply(std::slice::from_ref(&c), Mode::Strict).is_err(), "{why}: {c:?}");
        let mut r = a.clone();
        let out = r.apply(&[c, set(kids[0], Prop::Name, "ok".into())], Mode::Lenient).unwrap();
        assert!(out.skipped <= 1, "{why}");
        assert_eq!(r.document().node(kids[0]).unwrap().name, "ok");
    }
    // A cycle.
    let mut r = a.clone();
    let inner = Change::Create { id: NodeId(600), kind: frame_kind(), parent: Some(frame), position: "a9".into(), props: vec![] };
    let cycle = set(frame, Prop::Parent, serde_json::json!(600));
    assert!(matches!(r.apply(&[inner, cycle], Mode::Strict), Err(Error::Invalid(_))));
    // Deleting what's already gone does nothing.
    let mut r = a.clone();
    assert!(r.apply(&[Change::Delete { id: NodeId(12345) }, Change::Remove { entry: Entry::Comment(1) }], Mode::Strict).is_ok());
}

/// Where `id` goes among `kids`, as placement worked before it searched: after every other sibling
/// that sorts before it by `(key, id)`, counted one by one.
fn place_counting(kids: &mut Vec<NodeId>, keys: &BTreeMap<NodeId, String>, id: NodeId) {
    kids.retain(|c| *c != id);
    let key = |c: NodeId| keys.get(&c).map_or("", String::as_str);
    let at = kids.iter().filter(|c| (key(**c), **c) < (key(id), id)).count();
    kids.insert(at, id);
}

#[test]
fn many_sibling_placements_keep_the_order_of_counting_them() {
    // The reference counts, so it's quadratic: placing 20,000 here took 20 s before the search
    // and 0.3 s after (debug build).
    const N: u64 = 5_000;
    let mut doc = Document::new("Order");
    let page = doc.first_page().unwrap();
    let parents = [page, doc.create(page, NodeKind::Frame, "A", 0.0, 0.0, 10.0, 10.0).unwrap()];
    let mut r = Replica::new(doc);
    let mut rng = crate::sim::Rng::new(1065);
    // Few whole keys and the fractions between them: many ties (broken by id), many fractions.
    let whole = crate::frac::n_between(None, None, 12).unwrap();
    let mut pool = whole.clone();
    pool.extend(whole.windows(2).map(|w| crate::frac::between(Some(&w[0]), Some(&w[1])).unwrap()));
    let mut keys: BTreeMap<NodeId, String> = parents.iter().map(|p| (*p, r.key(*p).to_string())).collect();
    let mut kids: BTreeMap<NodeId, Vec<NodeId>> = parents.iter().map(|p| (*p, r.document().node(*p).unwrap().children.clone())).collect();
    let mut parent_of = BTreeMap::new();
    // One batch of creations in a scrambled id order, under either parent.
    let mut creates = Vec::new();
    for i in 0..N {
        let (id, p, key) = (NodeId(1_000_000 + (i * 7919) % N), parents[rng.below(2)], pool[rng.below(pool.len())].clone());
        keys.insert(id, key.clone());
        parent_of.insert(id, p);
        place_counting(kids.get_mut(&p).unwrap(), &keys, id);
        creates.push(Change::Create { id, kind: Json::of(&NodeKind::Rectangle).unwrap(), parent: Some(p), position: key, props: vec![] });
    }
    // Then a batch of moves: to another key (often a tie), or under either parent (its own too).
    let mut moves = Vec::new();
    for _ in 0..N / 2 {
        let id = NodeId(1_000_000 + rng.below(N as usize) as u64);
        if rng.chance(0.5) {
            let key = pool[rng.below(pool.len())].clone();
            keys.insert(id, key.clone());
            place_counting(kids.get_mut(&parent_of[&id]).unwrap(), &keys, id);
            moves.push(Change::Set { id, prop: Prop::Position, value: Json::of(&key).unwrap() });
        } else {
            let p = parents[rng.below(2)];
            let old = parent_of.insert(id, p).unwrap();
            kids.get_mut(&old).unwrap().retain(|c| *c != id);
            place_counting(kids.get_mut(&p).unwrap(), &keys, id);
            moves.push(Change::Set { id, prop: Prop::Parent, value: Json::of(&Some(p)).unwrap() });
        }
    }
    r.apply(&creates, Mode::Strict).unwrap();
    r.apply(&moves, Mode::Strict).unwrap();
    for p in parents {
        assert_eq!(r.document().node(p).unwrap().children, kids[&p], "children of {p}");
    }
}
