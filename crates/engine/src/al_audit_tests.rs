//! Auto layout behaviour from a user report ("the auto layout sometimes still goes wrong"), each
//! checked against Figma's help center.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::NodeId;

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

/// A horizontal frame 300×100 with gap 10 and no padding.
fn row(s: &mut Session) -> NodeId {
    let f = id(ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 100})));
    ex(s, "layout.add", json!({"ids": [f]}));
    ex(
        s,
        "node.set",
        json!({"id": f, "props": {"layoutMode": "horizontal", "sizingH": "fixed", "sizingV": "fixed", "width": 300, "height": 100, "gap": 10, "padding": 0}}),
    );
    f
}

/// A group set to Fill scales what it holds, as resizing a group does: its box and its contents
/// stay together.
#[test]
fn a_group_set_to_fill_scales_its_contents() {
    let mut s = Session::new();
    let f = row(&mut s);
    ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 20, "parent": f}));
    let a = id(ex(&mut s, "shape.rectangle", json!({"x": 100, "y": 0, "width": 20, "height": 20, "parent": f})));
    let b = id(ex(&mut s, "shape.rectangle", json!({"x": 130, "y": 0, "width": 20, "height": 20, "parent": f})));
    let g = id(ex(&mut s, "object.group", json!({"ids": [a, b]})));
    ex(&mut s, "node.set", json!({"id": g, "props": {"sizingH": "fill", "sizingV": "fill"}}));
    let doc = s.doc().unwrap();
    let gb = doc.world_bounds(g).unwrap();
    assert!((gb.width() - 250.0).abs() < 1e-6 && (gb.height() - 100.0).abs() < 1e-6, "the group fills: {gb:?}");
    let (ab, bb) = (doc.world_bounds(a).unwrap(), doc.world_bounds(b).unwrap());
    let union = ab.union(bb);
    assert!((union.x0 - gb.x0).abs() < 1e-6 && (union.x1 - gb.x1).abs() < 1e-6, "contents span the group: {union:?} in {gb:?}");
    assert!((union.y0 - gb.y0).abs() < 1e-6 && (union.y1 - gb.y1).abs() < 1e-6, "and its height: {union:?} in {gb:?}");
}
