//! Duplicating a page copies its prototype connections: navigation between two frames that are both
//! on the page leads between the copies, while a destination outside the copied contents is kept.

use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn frame(s: &mut Session, x: f64) -> NodeId {
    id_of(&ex(s, "frame.create", json!({"x": x, "y": 0, "width": 100, "height": 100}))["id"]).unwrap()
}

fn dest(s: &Session, id: NodeId) -> NodeId {
    s.doc().unwrap().node(id).unwrap().interactions[0].actions[0].dest().unwrap()
}

#[test]
fn page_copy_navigation_uses_the_copied_destination() {
    let mut s = Session::new();
    let first = frame(&mut s, 0.0);
    let second = frame(&mut s, 200.0);
    ex(&mut s, "proto.interaction.add", json!({"id": first, "action": {"type": "navigate", "dest": second}}));
    let copy = id_of(&ex(&mut s, "page.duplicate", json!({}))["id"]).unwrap();
    let children = s.doc().unwrap().node(copy).unwrap().children.clone();
    assert_eq!(children.len(), 2);
    assert_eq!(dest(&s, children[0]), children[1]);
    let visited = Default::default();
    assert_eq!(crate::proto_run::navigate_target(s.doc().unwrap(), dest(&s, children[0]), &visited), Some(children[1]));
    assert_eq!(dest(&s, first), second, "the original keeps its destination");
}

#[test]
fn frame_copy_keeps_an_outside_destination() {
    let mut s = Session::new();
    let first = frame(&mut s, 0.0);
    let second = frame(&mut s, 200.0);
    ex(&mut s, "proto.interaction.add", json!({"id": first, "action": {"type": "navigate", "dest": second}}));
    ex(&mut s, "select.set", json!({"ids": [first]}));
    ex(&mut s, "edit.duplicate", json!({}));
    let copy = s.selection()[0];
    assert_ne!(copy, first);
    assert_eq!(dest(&s, copy), second);
}
