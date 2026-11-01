//! A session remembers at most [`MAX_DELETED`] deleted node ids for undo (#1121): a long session
//! deleting layer after layer, or a saved state listing more, keeps the highest ids and forgets the
//! lowest.

use std::collections::BTreeSet;

use figcraft_doc::{Document, Node, NodeId, NodeKind};

use crate::message::ClientMsg;
use crate::server::MAX_DELETED;
use crate::{Client, Limits, Outgoing, Server};

fn filled() -> BTreeSet<NodeId> {
    (1..=MAX_DELETED as u64 + 10).map(NodeId).collect()
}

#[test]
fn a_restored_state_keeps_the_highest_deleted_ids() {
    let mut d = Document::new("Shared");
    d.repair();
    let mut state = Server::new(d, "tok", Limits::default()).state().unwrap();
    state.deleted = filled();
    let kept = Server::restore(state, "tok", Limits::default()).unwrap().state().unwrap().deleted;
    assert_eq!(kept.len(), MAX_DELETED);
    assert_eq!(kept.first(), Some(&NodeId(11)), "the lowest went first");
}

#[test]
fn deleting_past_the_cap_forgets_the_lowest() {
    let mut d = Document::new("Shared");
    let page = d.first_page().unwrap();
    // From an earlier session's id space, above every id in `filled`.
    let frame = NodeId((7 << 32) | 1);
    d.insert(Node::new(frame, NodeKind::Frame, "Frame", 100.0, 100.0), page, None).unwrap();
    let mut state = Server::new(d, "tok", Limits::default()).state().unwrap();
    state.deleted = filled();
    let mut s = Server::restore(state, "tok", Limits::default()).unwrap();
    let ClientMsg::Hello(hello) = Client::hello("f", "tok", "Ada", "#FF8800") else { panic!("not a hello") };
    let (id, out) = s.join(hello).unwrap();
    let Some(Outgoing::To(_, welcome)) = out.into_iter().next() else { panic!("no welcome") };
    let mut c = Client::join(welcome).unwrap();
    let mut edited = c.document().clone();
    edited.remove(frame).unwrap();
    let msg = c.edit(&edited).unwrap().expect("a change");
    s.receive(id, msg);
    let deleted = s.state().unwrap().deleted;
    assert_eq!(deleted.len(), MAX_DELETED);
    assert!(deleted.contains(&frame), "the newly deleted frame is remembered");
    assert!(!deleted.contains(&NodeId(11)));
}
