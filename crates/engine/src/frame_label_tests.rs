//! Frame name labels as pointer targets: a press on a label (its area as the canvas drew it,
//! turned with a rotated frame) selects the frame and drags it in one undo step, hovering it
//! highlights the frame, a double-click asks the UI to rename it there, and labels of locked,
//! hidden, deleted or other pages' frames do nothing.

use figcraft_doc::NodeId;
use figcraft_geom::Point;
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind, label_at};

fn ex(s: &mut Session, c: &str, p: Value) -> Value {
    s.execute(c, &p).unwrap()
}

/// A 200 × 100 frame at (100, 100) turned 30°, with its label: a 60 × 14 strip just outside its
/// top edge, from its top-left corner, as the canvas would publish it at zoom 1.
fn setup() -> (Session, NodeId, [Point; 4]) {
    let mut s = Session::new();
    let f = id_of(&ex(&mut s, "frame.create", json!({"x": 100, "y": 100, "width": 200, "height": 100}))["id"]).unwrap();
    ex(&mut s, "node.set", json!({"id": f, "rotation": 30}));
    let w = s.doc().unwrap().world_transform(f);
    let q = [(0.0, -18.0), (60.0, -18.0), (60.0, -4.0), (0.0, -4.0)].map(|(x, y)| w * Point::new(x, y));
    s.tool.labels = vec![(f, q)];
    (s, f, q)
}

fn center(q: &[Point; 4]) -> Point {
    Point::new(q.iter().map(|p| p.x).sum::<f64>() / 4.0, q.iter().map(|p| p.y).sum::<f64>() / 4.0)
}

fn ev(kind: PointerKind, p: Point) -> PointerEvent {
    PointerEvent::at(kind, p.x, p.y, 1.0)
}

#[test]
fn pressing_a_rotated_frames_label_selects_and_drags_the_frame() {
    let (mut s, f, q) = setup();
    let c = center(&q);
    let page = s.page().unwrap();
    // The label lies outside the frame: nothing else is there.
    assert_eq!(s.doc().unwrap().hit_test(page, c, 1.0, None), None);
    // Hover highlights the frame.
    s.pointer(&ev(PointerKind::Hover, c)).unwrap();
    assert_eq!(s.ds().unwrap().hover, Some(f));
    let undo = s.ds().unwrap().history.undo.len();
    let before = s.doc().unwrap().world_bounds(f).unwrap();
    s.pointer(&ev(PointerKind::Down, c)).unwrap();
    assert_eq!(s.selection(), vec![f]);
    s.pointer(&ev(PointerKind::Move, c + figcraft_geom::Vec2::new(40.0, 25.0))).unwrap();
    s.pointer(&ev(PointerKind::Up, c + figcraft_geom::Vec2::new(40.0, 25.0))).unwrap();
    let after = s.doc().unwrap().world_bounds(f).unwrap();
    assert!((after.x0 - before.x0 - 40.0).abs() < 1.0 && (after.y0 - before.y0 - 25.0).abs() < 1.0, "{before:?} → {after:?}");
    assert_eq!(s.ds().unwrap().history.undo.len(), undo + 1);
    // Shift+press on the label of a selected frame takes it out of the selection.
    let mut shift = ev(PointerKind::Down, center(&s.tool.labels[0].1));
    shift.shift = true;
    s.pointer(&shift).unwrap();
    s.pointer(&PointerEvent { kind: PointerKind::Up, ..shift }).unwrap();
    assert!(s.selection().is_empty());
}

#[test]
fn a_click_on_a_selected_frames_label_selects_only_it() {
    let (mut s, f, q) = setup();
    let other = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 500, "y": 500, "width": 40, "height": 40}))["id"]).unwrap();
    ex(&mut s, "select.set", json!({"ids": [f, other]}));
    // A point on the label's left edge, clear of the selection box's resize handles, lies outside
    // the selection's bounds (the strip starts left of the frame's left edge there).
    let p = q[0] + (q[3] - q[0]) * 0.2 + (q[1] - q[0]) * 0.02;
    assert!(s.doc().unwrap().bounds_of(&[f, other]).is_some_and(|b| !b.contains(p)), "the label point is outside the selection bounds");
    assert_eq!(label_at(s.doc().unwrap(), s.page().unwrap(), &s.tool.labels, p), Some(f), "the point is on the label");
    s.pointer(&ev(PointerKind::Down, p)).unwrap();
    s.pointer(&ev(PointerKind::Up, p)).unwrap();
    assert_eq!(s.selection(), vec![f], "the label outside the bounds still narrows the selection to its frame");
}

#[test]
fn labels_of_frames_that_cant_be_picked_do_nothing() {
    let (mut s, f, q) = setup();
    let c = center(&q);
    let page = s.page().unwrap();
    assert_eq!(label_at(s.doc().unwrap(), page, &s.tool.labels, c), Some(f));
    // Just outside the strip.
    assert_eq!(label_at(s.doc().unwrap(), page, &s.tool.labels, q[0] + (q[0] - q[2]) * 0.1), None);
    ex(&mut s, "node.set", json!({"id": f, "locked": true}));
    assert_eq!(label_at(s.doc().unwrap(), page, &s.tool.labels, c), None);
    s.pointer(&ev(PointerKind::Down, c)).unwrap();
    s.pointer(&ev(PointerKind::Up, c)).unwrap();
    assert!(s.selection().is_empty(), "a locked frame isn't picked by its label");
    ex(&mut s, "node.set", json!({"id": f, "locked": false, "visible": false}));
    assert_eq!(label_at(s.doc().unwrap(), page, &s.tool.labels, c), None);
    ex(&mut s, "node.set", json!({"id": f, "visible": true}));
    // Another page: the old page's labels don't apply.
    ex(&mut s, "page.add", json!({"name": "Two"}));
    let two = s.page().unwrap();
    assert_ne!(two, page);
    assert_eq!(label_at(s.doc().unwrap(), two, &s.tool.labels, c), None);
    // Deleted, and junk areas.
    ex(&mut s, "page.select", json!({"id": page}));
    ex(&mut s, "edit.delete", json!({"ids": [f]}));
    assert_eq!(label_at(s.doc().unwrap(), page, &s.tool.labels, c), None);
    let nan = [Point::new(f64::NAN, 0.0); 4];
    let flat = [Point::new(1.0, 1.0); 4];
    assert_eq!(label_at(s.doc().unwrap(), page, &[(f, nan), (f, flat)], Point::new(1.0, 1.0)), None);
}

#[test]
fn double_clicking_a_label_asks_to_rename_its_frame() {
    let (mut s, f, q) = setup();
    let c = center(&q);
    let before = s.doc().unwrap().world_bounds(f).unwrap();
    let undo = s.ds().unwrap().history.undo.len();
    for kind in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::DoubleClick] {
        s.pointer(&ev(kind, c)).unwrap();
    }
    assert_eq!(s.selection(), vec![f]);
    let asked = s.ui_requests.iter().find(|r| r.kind == "canvas.renameLabel").map(|r| id_of(&r.data["id"]));
    assert_eq!(asked, Some(Some(f)), "the UI renames the frame at its label");
    assert_eq!(s.doc().unwrap().world_bounds(f), Some(before), "not moved");
    assert_eq!(s.ds().unwrap().history.undo.len(), undo, "nothing to undo yet");
    // Other tools ignore the label.
    s.ui_requests.clear();
    s.set_tool(crate::tools::ToolId::Hand);
    s.pointer(&ev(PointerKind::DoubleClick, c)).unwrap();
    assert!(s.ui_requests.iter().all(|r| r.kind != "canvas.renameLabel"));
}
