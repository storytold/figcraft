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

/// Three 60 x 60 rectangles in a row (or column), gap 20, at 0, 80 and 160 along the flow.
fn three(s: &mut Session, mode: &str) -> (NodeId, [NodeId; 3]) {
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 300})).unwrap());
    let kids: Vec<NodeId> =
        (0..3).map(|_| id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 60, "height": 60, "parent": f})).unwrap())).collect();
    s.execute("layout.add", &json!({"ids": [f]})).unwrap();
    s.execute("node.set", &json!({"id": f, "props": {"layoutMode": mode, "gap": 20, "padding": 0, "sizingH": "fixed", "sizingV": "fixed"}})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    (f, [kids[0], kids[1], kids[2]])
}

/// The order after dragging kid `which` by `by` along the flow, grabbed `grab` from its start edge.
fn dragged_order(mode: &str, which: usize, grab: f64, by: f64) -> Vec<usize> {
    let mut s = Session::new();
    let (f, kids) = three(&mut s, mode);
    let vertical = mode == "vertical";
    let b = s.doc().unwrap().world_bounds(kids[which]).unwrap();
    let from = if vertical { (b.center().x, b.y0 + grab) } else { (b.x0 + grab, b.center().y) };
    let to = if vertical { (from.0, from.1 + by) } else { (from.0 + by, from.1) };
    drag(&mut s, from, to);
    let order = s.doc().unwrap().node(f).unwrap().children.clone();
    order.iter().map(|c| kids.iter().position(|k| k == c).unwrap()).collect()
}

#[test]
fn a_child_dragged_right_passes_a_sibling_when_its_leading_edge_reaches_the_siblings_middle() {
    assert_eq!(dragged_order("horizontal", 0, 30.0, 49.0), vec![0, 1, 2]);
    assert_eq!(dragged_order("horizontal", 0, 30.0, 50.0), vec![1, 0, 2]);
    assert_eq!(dragged_order("horizontal", 0, 30.0, 129.0), vec![1, 0, 2]);
    assert_eq!(dragged_order("horizontal", 0, 30.0, 130.0), vec![1, 2, 0]);
}

#[test]
fn where_the_child_is_grabbed_does_not_change_its_slot() {
    assert_eq!(dragged_order("horizontal", 0, 10.0, 49.0), vec![0, 1, 2]);
    assert_eq!(dragged_order("horizontal", 0, 10.0, 50.0), vec![1, 0, 2]);
    assert_eq!(dragged_order("horizontal", 0, 10.0, 129.0), vec![1, 0, 2]);
    assert_eq!(dragged_order("horizontal", 0, 10.0, 130.0), vec![1, 2, 0]);
}

#[test]
fn a_child_dragged_left_passes_a_sibling_by_its_left_edge() {
    assert_eq!(dragged_order("horizontal", 2, 30.0, -49.0), vec![0, 1, 2]);
    assert_eq!(dragged_order("horizontal", 2, 30.0, -50.0), vec![0, 2, 1]);
    assert_eq!(dragged_order("horizontal", 2, 30.0, -130.0), vec![2, 0, 1]);
}

#[test]
fn a_column_reorders_by_the_bottom_edge() {
    assert_eq!(dragged_order("vertical", 0, 30.0, 50.0), vec![1, 0, 2]);
    assert_eq!(dragged_order("vertical", 0, 30.0, 130.0), vec![1, 2, 0]);
}

#[test]
fn a_child_dragged_left_passes_the_second_sibling_one_pixel_later() {
    assert_eq!(dragged_order("horizontal", 2, 30.0, -129.0), vec![0, 2, 1]);
}

#[test]
fn a_column_reorders_at_the_boundaries_both_ways() {
    assert_eq!(dragged_order("vertical", 0, 30.0, 49.0), vec![0, 1, 2]);
    assert_eq!(dragged_order("vertical", 0, 30.0, 129.0), vec![1, 0, 2]);
    assert_eq!(dragged_order("vertical", 2, 30.0, -49.0), vec![0, 1, 2]);
    assert_eq!(dragged_order("vertical", 2, 30.0, -50.0), vec![0, 2, 1]);
    assert_eq!(dragged_order("vertical", 2, 30.0, -129.0), vec![0, 2, 1]);
    assert_eq!(dragged_order("vertical", 2, 30.0, -130.0), vec![2, 0, 1]);
}

#[test]
fn a_drag_with_no_move_along_the_flow_keeps_the_slot() {
    // Back at the start, and a drag across the flow only, with snapping on (the default).
    assert_eq!(dragged_order("horizontal", 1, 30.0, 0.0), vec![0, 1, 2]);
    assert_eq!(dragged_order("vertical", 1, 30.0, 0.0), vec![0, 1, 2]);
    let mut s = Session::new();
    let (f, kids) = three(&mut s, "horizontal");
    let b = s.doc().unwrap().world_bounds(kids[1]).unwrap();
    let from = (b.center().x, b.center().y);
    drag(&mut s, from, (from.0, from.1 + 40.0));
    let order = s.doc().unwrap().node(f).unwrap().children.clone();
    assert_eq!(order, kids.to_vec());
}

#[test]
fn several_dragged_children_use_their_combined_box() {
    // Kids 0 and 1 together span 0..140; moved right by 49 the leading edge is at 189, just short of the middle of kid 2 (190).
    for (by, expected) in [(49.0, vec![0, 1, 2]), (50.0, vec![2, 0, 1])] {
        let mut s = Session::new();
        let (f, kids) = three(&mut s, "horizontal");
        s.execute("select.set", &json!({"ids": [kids[0], kids[1]]})).unwrap();
        let b = s.doc().unwrap().world_bounds(kids[0]).unwrap();
        let from = (b.center().x, b.center().y);
        drag(&mut s, from, (from.0 + by, from.1));
        let order: Vec<usize> = s.doc().unwrap().node(f).unwrap().children.iter().map(|c| kids.iter().position(|k| k == c).unwrap()).collect();
        assert_eq!(order, expected, "by {by}");
    }
}
