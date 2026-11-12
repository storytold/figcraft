//! Moving an auto layout frame (#1571): a frame that only moves keeps the layers inside it where
//! they are, so a drag doesn't lay its contents out again on every pointer move; a change that can
//! move or resize them still does, and its parent's layout still follows it.

use figcraft_doc::NodeId;
use serde_json::json;

use crate::Session;
use crate::params::id_of;

/// Two rectangles wrapped in an auto layout frame: the session, the frame and the rectangles.
fn stack() -> (Session, NodeId, [NodeId; 2]) {
    let mut s = Session::new();
    let mut make = |y| id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": y, "width": 100, "height": 40})).unwrap()["id"]).unwrap();
    let kids = [make(0), make(60)];
    s.execute("select.set", &json!({"ids": kids})).unwrap();
    s.execute("layout.add", &json!({})).unwrap();
    let frame = s.doc().unwrap().node(kids[0]).and_then(|n| n.parent).unwrap();
    (s, frame, kids)
}

fn local_y(s: &Session, id: NodeId) -> f64 {
    s.doc().unwrap().node(id).unwrap().transform.translation().y
}

#[test]
fn moving_an_auto_layout_frame_leaves_its_contents_alone() {
    let (mut s, frame, [_, second]) = stack();
    assert!(s.doc().unwrap().node(frame).unwrap().layout.is_some());
    let laid_out = local_y(&s, second);
    // Put the second rectangle out of place without anything noticing (as if the layout had
    // never run on it), then move the frame: its contents aren't laid out again.
    {
        let doc = s.ds_mut().unwrap().doc_mut();
        doc.node_mut(second).unwrap().transform = figcraft_doc::geom::Affine::translate((0.0, laid_out + 500.0));
        doc.take_dirty();
    }
    s.execute("node.set", &json!({"ids": [frame.0], "props": {"x": 300, "y": 200}})).unwrap();
    assert_eq!(local_y(&s, second), laid_out + 500.0, "a move doesn't lay the frame out");
    // A change that can move its contents does.
    s.execute("node.set", &json!({"ids": [frame.0], "props": {"width": 160}})).unwrap();
    assert_eq!(local_y(&s, second), laid_out, "a resize lays it out");
}

#[test]
fn a_frame_moved_inside_auto_layout_still_moves_its_siblings() {
    // A stack and a rectangle side by side in an outer auto layout frame: the outer layout still
    // follows when the stack grows.
    let (mut s, inner, [first, _]) = stack();
    let other = id_of(&s.execute("shape.rectangle", &json!({"x": 300, "y": 0, "width": 100, "height": 40})).unwrap()["id"]).unwrap();
    s.execute("select.set", &json!({"ids": [inner.0, other.0]})).unwrap();
    s.execute("layout.add", &json!({})).unwrap();
    let outer = s.doc().unwrap().node(inner).and_then(|n| n.parent).unwrap();
    assert!(s.doc().unwrap().node(outer).unwrap().layout.is_some());
    let before = s.doc().unwrap().world_bounds(other).unwrap();
    s.execute("node.set", &json!({"ids": [first.0], "props": {"width": 200}})).unwrap();
    let after = s.doc().unwrap().world_bounds(other).unwrap();
    assert!(after.x0 > before.x0, "the outer layout followed the inner frame's growth: {before:?} {after:?}");
}
