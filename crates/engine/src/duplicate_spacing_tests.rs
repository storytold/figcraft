//! Duplicating at a spacing: a copy Alt-dragged off a layer snaps to that layer too (its edges,
//! centres and equal spacing), and Duplicate repeats the last duplicate's offset while its copies
//! are selected, so Alt-drag (or Duplicate and move) then Cmd+D steps out evenly spaced copies.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind};
use figcraft_doc::NodeId;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn rect(s: &mut Session, x: f64, y: f64) -> NodeId {
    id_of(&ex(s, "shape.rectangle", json!({"x": x, "y": y, "width": 100, "height": 100}))["id"]).unwrap()
}

/// Alt-drag from `from` to `to`: returns the copy (the selection afterwards).
fn alt_drag(s: &mut Session, from: (f64, f64), to: (f64, f64)) -> NodeId {
    let ev = |kind, (x, y): (f64, f64)| PointerEvent { alt: true, ..PointerEvent::at(kind, x, y, 1.0) };
    s.pointer(&ev(PointerKind::Down, from)).unwrap();
    for k in 1..=4 {
        let t = f64::from(k) / 4.0;
        s.pointer(&ev(PointerKind::Move, (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t))).unwrap();
    }
    s.pointer(&ev(PointerKind::Up, to)).unwrap();
    let sel = s.selection();
    assert_eq!(sel.len(), 1, "one copy selected");
    sel[0]
}

/// Duplicate the selection: the copy.
fn cmd_d(s: &mut Session) -> NodeId {
    let out = ex(s, "edit.duplicate", json!({}));
    id_of(&out["ids"][0]).unwrap()
}

fn at(s: &Session, id: NodeId) -> (f64, f64) {
    let b = s.doc().unwrap().world_bounds(id).unwrap();
    (b.x0, b.y0)
}

#[test]
fn an_alt_dragged_copy_snaps_to_the_layer_it_came_from() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    // 2 px below the original's top edge: within the snap distance.
    let copy = alt_drag(&mut s, (50.0, 50.0), (250.0, 52.0));
    assert_ne!(copy, a);
    assert_eq!(at(&s, copy), (200.0, 0.0), "the copy's top lines up with the original's");
    assert_eq!(at(&s, a), (0.0, 0.0), "the original stays");
}

#[test]
fn an_alt_dragged_copy_spaces_equally_with_the_layer_it_came_from() {
    let mut s = Session::new();
    rect(&mut s, 0.0, 0.0);
    rect(&mut s, 200.0, 0.0);
    // B's copy dropped 2 px short of the 100 gap that A and B have.
    let copy = alt_drag(&mut s, (250.0, 50.0), (448.0, 50.0));
    assert_eq!(at(&s, copy), (400.0, 0.0));
}

#[test]
fn duplicate_repeats_the_step_set_by_moving_the_copy() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    ex(&mut s, "select.set", json!({"ids": [a]}));
    let c1 = cmd_d(&mut s);
    assert_eq!(at(&s, c1), (0.0, 0.0), "the first copy lands in place");
    ex(&mut s, "node.set", json!({"ids": [c1], "props": {"x": 150, "y": 20}}));
    let c2 = cmd_d(&mut s);
    assert_eq!(at(&s, c2), (300.0, 40.0), "the next one as far again");
    let c3 = cmd_d(&mut s);
    assert_eq!(at(&s, c3), (450.0, 60.0), "and again");
}

#[test]
fn duplicate_after_an_alt_drag_repeats_its_step() {
    let mut s = Session::new();
    rect(&mut s, 0.0, 0.0);
    let copy = alt_drag(&mut s, (50.0, 50.0), (180.0, 50.0));
    assert_eq!(at(&s, copy), (130.0, 0.0));
    let next = cmd_d(&mut s);
    assert_eq!(at(&s, next), (260.0, 0.0));
}

#[test]
fn duplicate_only_repeats_for_the_last_copies() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    ex(&mut s, "select.set", json!({"ids": [a]}));
    let c1 = cmd_d(&mut s);
    ex(&mut s, "node.set", json!({"ids": [c1], "props": {"x": 150}}));
    // Another selection forgets it: the original duplicates in place.
    ex(&mut s, "select.set", json!({"ids": [a]}));
    ex(&mut s, "select.set", json!({"ids": [c1]}));
    let c2 = cmd_d(&mut s);
    assert_eq!(at(&s, c2), (150.0, 0.0), "the step was forgotten when the selection changed");
    // An offset given explicitly wins over a remembered one.
    ex(&mut s, "node.set", json!({"ids": [c2], "props": {"x": 300}}));
    let out = ex(&mut s, "edit.duplicate", json!({"dx": 10}));
    let c3 = id_of(&out["ids"][0]).unwrap();
    assert_eq!(at(&s, c3), (310.0, 0.0));
}

#[test]
fn undo_doesnt_let_a_new_layer_take_a_remembered_step() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    ex(&mut s, "select.set", json!({"ids": [a]}));
    let c1 = cmd_d(&mut s);
    ex(&mut s, "node.set", json!({"ids": [c1], "props": {"x": 150}}));
    ex(&mut s, "edit.undo", json!({}));
    ex(&mut s, "edit.undo", json!({}));
    assert!(!s.doc().unwrap().contains(c1));
    // Undo gives ids back: the new rectangle may well get the undone copy's.
    let r = rect(&mut s, 500.0, 500.0);
    ex(&mut s, "select.set", json!({"ids": [r]}));
    let c = cmd_d(&mut s);
    assert_eq!(at(&s, c), (500.0, 500.0), "a plain duplicate, in place");
}

#[test]
fn the_step_is_measured_in_page_space_inside_a_rotated_frame() {
    let mut s = Session::new();
    let f = id_of(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 600, "height": 600}))["id"]).unwrap();
    ex(&mut s, "node.set", json!({"ids": [f], "props": {"rotation": 30}}));
    let a = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 50, "y": 50, "width": 40, "height": 40, "parent": f}))["id"]).unwrap();
    ex(&mut s, "select.set", json!({"ids": [a]}));
    let c1 = cmd_d(&mut s);
    ex(&mut s, "node.set", json!({"ids": [c1], "props": {"x": 150}}));
    let (p0, p1) = (at(&s, a), at(&s, c1));
    let c2 = cmd_d(&mut s);
    let p2 = at(&s, c2);
    let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
    assert!(close(p2.0 - p1.0, p1.0 - p0.0) && close(p2.1 - p1.1, p1.1 - p0.1), "{p0:?} {p1:?} {p2:?}");
}

#[test]
fn an_alt_drag_into_auto_layout_sets_no_step() {
    let mut s = Session::new();
    let f = id_of(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 400, "height": 100}))["id"]).unwrap();
    for i in 0..2 {
        ex(&mut s, "shape.rectangle", json!({"x": i * 60, "y": 0, "width": 50, "height": 30, "parent": f}));
    }
    ex(&mut s, "layout.add", json!({"ids": [f]}));
    ex(&mut s, "select.none", json!({}));
    let kids = s.doc().unwrap().node(f).unwrap().children.clone();
    let b = s.doc().unwrap().world_bounds(kids[0]).unwrap();
    let from = (b.center().x, b.center().y);
    alt_drag(&mut s, from, (from.0 + 70.0, from.1));
    assert!(s.ds().unwrap().last_duplicate.is_none(), "the layout placed the copy: no step to repeat");
    let n = s.doc().unwrap().node(f).unwrap().children.len();
    cmd_d(&mut s);
    assert_eq!(s.doc().unwrap().node(f).unwrap().children.len(), n + 1, "Duplicate still adds one to the flow");
}

/// Drag `from` by `by` with no modifier: moving the selection, not copying it.
fn drag_move(s: &mut Session, from: (f64, f64), by: (f64, f64)) {
    let ev = |kind, (x, y): (f64, f64)| PointerEvent::at(kind, x, y, 1.0);
    s.pointer(&ev(PointerKind::Down, from)).unwrap();
    for k in 1..=4 {
        let t = f64::from(k) / 4.0;
        s.pointer(&ev(PointerKind::Move, (from.0 + by.0 * t, from.1 + by.1 * t))).unwrap();
    }
    s.pointer(&ev(PointerKind::Up, (from.0 + by.0, from.1 + by.1))).unwrap();
}

/// The reported path (#567): Cmd+D, drag the copy with the mouse, Cmd+D again. The other tests
/// here set the copy's position with `node.set` or Alt-drag it; dragging is how it is usually done,
/// and the step is measured from where the copy ended up, whichever way it got there.
#[test]
fn duplicate_repeats_the_step_set_by_dragging_the_copy() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    ex(&mut s, "select.set", json!({"ids": [a]}));
    let c1 = cmd_d(&mut s);
    assert_eq!(at(&s, c1), (0.0, 0.0), "the first copy lands in place");

    drag_move(&mut s, (50.0, 50.0), (150.0, 20.0));
    assert_eq!(at(&s, c1), (150.0, 20.0), "the drag moved the copy, it did not make another one");
    assert_eq!(s.selection(), vec![c1], "and left it selected");
    assert_eq!(at(&s, a), (0.0, 0.0), "the original stays");

    let c2 = cmd_d(&mut s);
    assert_eq!(at(&s, c2), (300.0, 40.0), "the next one as far again");
    let c3 = cmd_d(&mut s);
    assert_eq!(at(&s, c3), (450.0, 60.0), "and again");
}

/// And a step set with the arrow keys, which is how a small offset is usually made exact.
#[test]
fn duplicate_repeats_a_nudged_step() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0);
    ex(&mut s, "select.set", json!({"ids": [a]}));
    let c1 = cmd_d(&mut s);
    ex(&mut s, "transform.nudge", json!({"dx": 40, "dy": 0}));
    assert_eq!(at(&s, c1), (40.0, 0.0));
    let c2 = cmd_d(&mut s);
    assert_eq!(at(&s, c2), (80.0, 0.0), "the nudge is the step");
}
