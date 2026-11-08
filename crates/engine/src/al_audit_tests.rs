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

fn rects(s: &mut Session, at: &[(f64, f64, f64, f64)]) -> Vec<NodeId> {
    at.iter().map(|(x, y, w, h)| id(ex(s, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h})))).collect()
}

fn origin(s: &Session, n: NodeId) -> (f64, f64) {
    let b = s.doc().unwrap().world_bounds(n).unwrap();
    ((b.x0 * 1000.0).round() / 1000.0, (b.y0 * 1000.0).round() / 1000.0)
}

/// Shift+A on a 2×2 arrangement makes a grid ("Figma will try to determine which auto layout
/// flow—vertical, horizontal, or grid—you want to use"), its children read from the top row left
/// to right, with the gaps they had, every layer where it was.
#[test]
fn add_auto_layout_infers_a_grid_and_keeps_the_arrangement() {
    let mut s = Session::new();
    let k = rects(&mut s, &[(0.0, 0.0, 40.0, 40.0), (60.0, 0.0, 40.0, 40.0), (0.0, 60.0, 40.0, 40.0), (60.0, 60.0, 40.0, 40.0)]);
    let before: Vec<_> = k.iter().map(|n| origin(&s, *n)).collect();
    let f = id_of(&ex(&mut s, "layout.add", json!({"ids": k}))["ids"][0]).unwrap();
    let doc = s.doc().unwrap();
    let al = doc.node(f).unwrap().layout.clone().unwrap();
    assert_eq!(al.mode, figcraft_doc::LayoutMode::Grid);
    assert_eq!((al.grid_columns, al.gap, al.cross_gap), (2, 20.0, 20.0));
    assert_eq!(doc.node(f).unwrap().children, k, "row by row");
    let after: Vec<_> = k.iter().map(|n| origin(&s, *n)).collect();
    assert_eq!(after, before, "every layer stays where it was");
}

/// Layers side by side make a row however tall they are, and an asked-for grid reads row by row.
#[test]
fn add_auto_layout_reads_rows_and_an_asked_for_grid() {
    let mut s = Session::new();
    let k = rects(&mut s, &[(0.0, 0.0, 40.0, 300.0), (80.0, 0.0, 40.0, 300.0)]);
    let f = id_of(&ex(&mut s, "layout.add", json!({"ids": k}))["ids"][0]).unwrap();
    let al = s.doc().unwrap().node(f).unwrap().layout.clone().unwrap();
    assert_eq!((al.mode, al.gap), (figcraft_doc::LayoutMode::Horizontal, 40.0), "side by side: a row");
    let mut s = Session::new();
    // Created out of order: the layer order isn't the reading order.
    let k = rects(&mut s, &[(60.0, 60.0, 40.0, 40.0), (0.0, 60.0, 40.0, 40.0), (60.0, 0.0, 40.0, 40.0), (0.0, 0.0, 40.0, 40.0)]);
    let f = id_of(&ex(&mut s, "layout.add", json!({"ids": k, "direction": "grid"}))["ids"][0]).unwrap();
    assert_eq!(s.doc().unwrap().node(f).unwrap().children, vec![k[3], k[2], k[1], k[0]], "top row first, left to right");
}

/// A layer turned 90° and set to Fill fills the flow with its rotated box: its own height takes
/// the room, so it stays inside the frame and its neighbour follows it.
#[test]
fn a_rotated_layer_fills_with_its_rotated_box() {
    let mut s = Session::new();
    let f = row(&mut s);
    ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 20, "parent": f}));
    let b = id(ex(&mut s, "shape.rectangle", json!({"x": 50, "y": 0, "width": 60, "height": 30, "parent": f})));
    let c = id(ex(&mut s, "shape.rectangle", json!({"x": 120, "y": 0, "width": 50, "height": 40, "parent": f})));
    ex(&mut s, "node.set", json!({"id": b, "props": {"rotation": 90}}));
    ex(&mut s, "node.set", json!({"id": b, "props": {"sizingH": "fill"}}));
    let doc = s.doc().unwrap();
    let (bb, cb) = (doc.world_bounds(b).unwrap(), doc.world_bounds(c).unwrap());
    // 300 less 40 and 50 and two gaps of 10.
    assert!((bb.width() - 190.0).abs() < 1e-6 && (bb.height() - 60.0).abs() < 1e-6, "{bb:?}");
    assert!((cb.x0 - bb.x1 - 10.0).abs() < 1e-6, "the neighbour follows: {cb:?} after {bb:?}");
}
