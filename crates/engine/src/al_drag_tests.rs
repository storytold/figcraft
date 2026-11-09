//! Dragging children of wrapping and grid auto layout frames.

use serde_json::json;

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind};
use figcraft_doc::NodeId;

fn id(v: serde_json::Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn drag(s: &mut Session, from: (f64, f64), to: (f64, f64)) {
    s.pointer(&PointerEvent::at(PointerKind::Down, from.0, from.1, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, (from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, to.0, to.1, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, to.0, to.1, 1.0)).unwrap();
}

/// A frame with six 50×30 children in auto layout of `mode`.
fn frame_with_kids(s: &mut Session, props: serde_json::Value) -> (NodeId, Vec<NodeId>) {
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap());
    let kids: Vec<NodeId> =
        (0..6).map(|i| id(s.execute("shape.rectangle", &json!({"x": i * 60, "y": 0, "width": 50, "height": 30, "parent": f})).unwrap())).collect();
    s.execute("layout.add", &json!({"ids": [f]})).unwrap();
    s.execute("node.set", &json!({"id": f, "props": props})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    (f, kids)
}

#[test]
fn wrap_drag_reorders_by_row_and_position() {
    let mut s = Session::new();
    let (f, kids) = frame_with_kids(
        &mut s,
        json!({"layoutMode": "horizontal", "wrap": true, "width": 200, "sizingH": "fixed", "gap": 10, "crossGap": 10, "padding": 0}),
    );
    let b = |s: &Session, i: NodeId| s.doc().unwrap().world_bounds(i).unwrap();
    // Three per row: kid 4 sits in the second row.
    assert!(b(&s, kids[3]).y0 > b(&s, kids[0]).y0, "wrapped into two rows");
    // Drag the first child into the second row, just before kid 5.
    let from = b(&s, kids[0]).center();
    let k5 = b(&s, kids[5]);
    drag(&mut s, (from.x, from.y), (k5.x0 + 5.0, k5.center().y));
    let order = s.doc().unwrap().node(f).unwrap().children.clone();
    assert_eq!(order, vec![kids[1], kids[2], kids[3], kids[4], kids[0], kids[5]], "{order:?}");
    // Right of a row's last child: after it.
    let from = b(&s, kids[5]).center();
    let k1 = b(&s, kids[2]);
    drag(&mut s, (from.x, from.y), (k1.x1 + 4.0, k1.center().y));
    let order = s.doc().unwrap().node(f).unwrap().children.clone();
    assert_eq!(order.get(..3), Some(&[kids[1], kids[2], kids[5]][..]), "{order:?}");
}

#[test]
fn grid_drag_takes_the_cell_under_the_pointer() {
    let mut s = Session::new();
    let (f, kids) = frame_with_kids(&mut s, json!({"layoutMode": "grid", "gridColumns": 3, "gridRows": 2}));
    let geometry = figcraft_layout::grid_geometry(s.doc().unwrap(), f).unwrap();
    let (c2, r1) = (geometry.columns[2], geometry.rows[1]);
    let to = s.doc().unwrap().world_transform(f) * figcraft_geom::Point::new((c2.0 + c2.1) / 2.0, (r1.0 + r1.1) / 2.0);
    let from = s.doc().unwrap().world_bounds(kids[0]).unwrap().center();
    drag(&mut s, (from.x, from.y), (to.x, to.y));
    let cl = s.doc().unwrap().node(kids[0]).unwrap().child_layout.clone();
    assert_eq!((cl.grid_column, cl.grid_row), (Some(2), Some(1)));
    assert!(s.tool.overlay.drop_cell.is_none(), "the cell indicator clears on release");
    s.undo().unwrap();
    let cl = s.doc().unwrap().node(kids[0]).unwrap().child_layout.clone();
    assert_eq!((cl.grid_column, cl.grid_row), (None, None), "one undo step");
}

/// A child dragged to the full space its auto layout frame has for it becomes Fill ("Child objects
/// of an auto layout frame will also be set to Fill container if they are manually resized to the
/// full available space of the parent frame"), as typing that size does; short of it, Fixed.
#[test]
fn resizing_a_child_to_the_full_space_makes_it_fill() {
    let mut s = Session::new();
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap());
    let k = id(s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 30, "parent": f})).unwrap());
    s.execute("layout.add", &json!({"ids": [f]})).unwrap();
    s.execute(
        "node.set",
        &json!({"id": f, "props": {"layoutMode": "vertical", "sizingH": "fixed", "sizingV": "fixed", "width": 200, "height": 200, "padding": 10}}),
    )
    .unwrap();
    s.execute("view.snapPixel", &json!({"on": true})).unwrap();
    let sizing = |s: &Session| s.doc().unwrap().node(k).unwrap().child_layout.sizing_h;
    let edge = |s: &Session| {
        let b = s.doc().unwrap().world_bounds(k).unwrap();
        (b.x1, b.center().y)
    };
    s.execute("select.set", &json!({"ids": [k]})).unwrap();
    let e = edge(&s);
    drag(&mut s, e, (150.0, e.1));
    assert_eq!(sizing(&s), figcraft_doc::Sizing::Fixed, "short of the padding: Fixed");
    let e = edge(&s);
    drag(&mut s, e, (190.0, e.1));
    assert_eq!(s.doc().unwrap().node(k).unwrap().width, 180.0);
    assert_eq!(sizing(&s), figcraft_doc::Sizing::Fill, "to the padding: Fill");
}

fn drag_with(s: &mut Session, from: (f64, f64), to: (f64, f64), set: impl Fn(&mut PointerEvent)) {
    let ev = |kind, x, y| {
        let mut e = PointerEvent::at(kind, x, y, 1.0);
        set(&mut e);
        e
    };
    s.pointer(&PointerEvent::at(PointerKind::Down, from.0, from.1, 1.0)).unwrap();
    s.pointer(&ev(PointerKind::Move, (from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0)).unwrap();
    s.pointer(&ev(PointerKind::Move, to.0, to.1)).unwrap();
    s.pointer(&ev(PointerKind::Up, to.0, to.1)).unwrap();
}

/// Dragging into an auto layout frame: a layer bigger than the frame stays out of it ("If any of
/// the object's dimensions are larger than the parent frame, you won't see the option to add it to
/// the auto layout") unless Cmd/Ctrl is held; with Control (macOS) a layer goes in ignoring the
/// auto layout, where it was dropped.
#[test]
fn dragging_in_keeps_out_what_is_too_big_and_control_ignores_the_layout() {
    let mut s = Session::new();
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 100})).unwrap());
    id(s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 40, "parent": f})).unwrap());
    s.execute("layout.add", &json!({"ids": [f]})).unwrap();
    s.execute("node.set", &json!({"id": f, "props": {"sizingH": "fixed", "sizingV": "fixed", "width": 200, "height": 100}})).unwrap();
    let big = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 300, "width": 300, "height": 50})).unwrap());
    let parent = |s: &Session, n: NodeId| s.doc().unwrap().node(n).unwrap().parent;
    let page = s.page().unwrap();
    s.execute("select.none", &json!({})).unwrap();
    drag(&mut s, (150.0, 325.0), (150.0, 50.0));
    assert_eq!(parent(&s, big), Some(page), "too wide for the frame: it stays out");
    s.undo().unwrap();
    s.execute("select.none", &json!({})).unwrap();
    drag_with(&mut s, (150.0, 325.0), (150.0, 50.0), |e| e.cmd = true);
    assert_eq!(parent(&s, big), Some(f), "Cmd/Ctrl adds it anyway");
    s.undo().unwrap();
    let small = id(s.execute("shape.rectangle", &json!({"x": 400, "y": 300, "width": 20, "height": 20})).unwrap());
    s.execute("select.none", &json!({})).unwrap();
    drag_with(&mut s, (410.0, 310.0), (150.0, 70.0), |e| e.ctrl = true);
    let doc = s.doc().unwrap();
    assert_eq!(parent(&s, small), Some(f));
    assert!(doc.node(small).unwrap().child_layout.absolute, "Control: it ignores the auto layout");
    let b = doc.world_bounds(small).unwrap();
    assert_eq!((b.x0, b.y0), (140.0, 60.0), "where it was dropped");
}
