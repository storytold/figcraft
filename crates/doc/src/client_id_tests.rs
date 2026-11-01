//! Client-scoped ids (collaboration), raw structure edits and page-less documents.

use crate::{DocError, Document, Node, NodeId, NodeKind};
use figcraft_geom::Affine;

fn rect(doc: &mut Document, parent: NodeId) -> NodeId {
    doc.create(parent, NodeKind::Rectangle, "R", 0.0, 0.0, 10.0, 10.0).unwrap()
}

const C3: u64 = 3 << 32;
const C5: u64 = 5 << 32;

#[test]
fn single_user_allocation_is_unchanged() {
    let mut d = Document::new("a");
    let page = d.first_page().unwrap();
    assert_eq!(page, NodeId(1));
    assert_eq!(d.client_id(), 0);
    assert_eq!(rect(&mut d, page), NodeId(2));
    assert_eq!(d.next_id(), NodeId(3));
    // Inserting any id above the counter moves it, as before.
    d.insert(Node::new(NodeId(C5 | 9), NodeKind::Rectangle, "x", 1.0, 1.0), page, None).unwrap();
    assert_eq!(d.next_id(), NodeId((C5 | 9) + 1));
}

#[test]
fn clients_allocate_in_their_own_space_and_never_collide() {
    let mut base = Document::new("a");
    let page = base.first_page().unwrap();
    rect(&mut base, page);
    let (mut a, mut b) = (base.clone(), base.clone());
    a.set_client_id(3);
    b.set_client_id(5);
    let ia: Vec<NodeId> = (0..3).map(|_| rect(&mut a, page)).collect();
    let ib: Vec<NodeId> = (0..3).map(|_| rect(&mut b, page)).collect();
    assert_eq!(ia, vec![NodeId(C3 | 1), NodeId(C3 | 2), NodeId(C3 | 3)]);
    assert_eq!(ib, vec![NodeId(C5 | 1), NodeId(C5 | 2), NodeId(C5 | 3)]);
    // Someone else's ids arriving don't move this client's counter; its own higher ids do.
    a.insert(Node::new(NodeId(C5 | 77), NodeKind::Rectangle, "theirs", 1.0, 1.0), page, None).unwrap();
    assert_eq!(a.next_id(), NodeId(C3 | 4));
    a.insert(Node::new(NodeId(C3 | 40), NodeKind::Rectangle, "mine", 1.0, 1.0), page, None).unwrap();
    assert_eq!(a.alloc_id(), NodeId(C3 | 41));
    // Old ids (client 0) don't count either.
    a.reserve_ids(NodeId(1_000));
    assert_eq!(a.next_id(), NodeId(C3 | 42));
}

#[test]
fn setting_a_client_continues_after_its_existing_ids() {
    let mut d = Document::new("a");
    let page = d.first_page().unwrap();
    d.insert(Node::new(NodeId(C3 | 7), NodeKind::Rectangle, "earlier session", 1.0, 1.0), page, None).unwrap();
    d.set_client_id(3);
    assert_eq!(d.client_id(), 3);
    assert_eq!(d.alloc_id(), NodeId(C3 | 8));
    // Setting the same client again keeps the counter (handed-out ids stay handed out).
    d.set_client_id(3);
    assert_eq!(d.alloc_id(), NodeId(C3 | 9));
    // Back to single-user: after the highest id of any space.
    d.set_client_id(0);
    assert_eq!(d.alloc_id(), NodeId((C3 | 7) + 1));
    // repair() keeps the client.
    d.set_client_id(5);
    d.repair();
    assert_eq!(d.client_id(), 5);
    assert_eq!(d.alloc_id(), NodeId(C5 | 1));
}

#[test]
fn the_counter_is_saved_but_never_compared_and_the_client_never_saved() {
    let mut a = Document::new("a");
    let mut b = a.clone();
    b.set_client_id(9);
    b.alloc_id();
    assert_eq!(a, b, "same content, different counters");
    let json = serde_json::to_value(&b).unwrap();
    assert_eq!(json["next_id"], serde_json::json!((9u64 << 32) | 2));
    let back: Document = serde_json::from_value(json).unwrap();
    assert_eq!(back.client_id(), 0);
    assert_eq!(back.next_id(), NodeId((9u64 << 32) | 2));
    a.set_client_id(9);
    assert_eq!(a.next_id(), NodeId((9u64 << 32) | 1));
}

#[test]
fn place_keeps_the_local_transform_and_refuses_bad_moves() {
    let mut d = Document::new("a");
    let page = d.first_page().unwrap();
    let f = d.create(page, NodeKind::Frame, "F", 100.0, 100.0, 50.0, 50.0).unwrap();
    let g = d.create(f, NodeKind::Frame, "G", 10.0, 10.0, 20.0, 20.0).unwrap();
    let r = rect(&mut d, page);
    d.get_mut(r).unwrap().transform = Affine::translate((5.0, 6.0));
    d.place(r, f, 0).unwrap();
    assert_eq!(d.node(r).unwrap().transform, Affine::translate((5.0, 6.0)));
    assert_eq!(d.node(r).unwrap().parent, Some(f));
    assert_eq!(d.node(f).unwrap().children, vec![r, g]);
    d.place(r, f, 99).unwrap();
    assert_eq!(d.node(f).unwrap().children, vec![g, r], "same parent, index clamped");
    assert_eq!(d.place(f, g, 0), Err(DocError::Cycle(f, g)));
    assert_eq!(d.place(f, f, 0), Err(DocError::Cycle(f, f)));
    assert_eq!(d.place(g, r, 0), Err(DocError::NotContainer(r)));
    assert!(d.place(page, f, 0).is_err());
    assert!(d.place(NodeId(999), f, 0).is_err());
    assert!(d.validate().is_empty());
}

/// A page-less document takes pages inserted with their own ids, refuses a duplicate or a layer
/// that isn't a page, and repairs into a file with a page.
#[test]
fn pages_can_be_inserted_onto_a_page_less_base() {
    let mut d = Document::without_pages("snapshot");
    assert!(d.pages.is_empty() && d.node_count() == 0);
    let p = Node::new(
        NodeId(C3 | 1),
        NodeKind::Page { background: figcraft_color::Color::DEFAULT_PAGE, flows: vec![], prototype: Default::default() },
        "P",
        0.0,
        0.0,
    );
    d.insert_page(p.clone()).unwrap();
    assert_eq!(d.pages, vec![NodeId(C3 | 1)]);
    assert!(d.insert_page(p).is_err(), "duplicate");
    assert!(d.insert_page(Node::new(NodeId(4), NodeKind::Frame, "F", 1.0, 1.0)).is_err(), "not a page");
    assert!(d.validate().is_empty());
    // The page-less base repairs into a file with a page.
    let mut e = Document::without_pages("x");
    e.repair();
    assert_eq!(e.pages.len(), 1);
}
