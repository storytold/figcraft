//! Live boolean editing: selecting operands on the canvas, results that follow an operand during a
//! drag or a point edit (one undo step each), per-operand operations from the boolean commands
//! and `node.set`, the operands outlined on the canvas, flatten, inspect and junk parameters.

use serde_json::{Value, json};

use crate::Session;
use crate::booleans::{distinct_op, operand_op, outlined_operands, shown_op};
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind};
use figcraft_doc::{BoolOp, NodeId, NodeKind};
use figcraft_geom::Shape;

pub(crate) fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

pub(crate) fn ev(kind: PointerKind, x: f64, y: f64, cmd: bool) -> PointerEvent {
    PointerEvent { cmd, ..PointerEvent::at(kind, x, y, 1.0) }
}

pub(crate) fn click(s: &mut Session, x: f64, y: f64, cmd: bool) {
    s.pointer(&ev(PointerKind::Down, x, y, cmd)).unwrap();
    s.pointer(&ev(PointerKind::Up, x, y, cmd)).unwrap();
}

/// A layer's filled area (a boolean group's result; none of these have holes).
pub(crate) fn area(s: &Session, id: NodeId) -> f64 {
    let doc = s.doc().unwrap();
    figcraft_render::node_outline(doc, doc.node(id).unwrap()).unwrap().0.area().abs()
}

/// A 100 × 100 square at the origin minus one at (50, 50): (group, bottom, top), nothing
/// selected, snapping off.
fn subtract(s: &mut Session) -> (NodeId, NodeId, NodeId) {
    let a = id_of(&ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 100}))["id"]).unwrap();
    let b = id_of(&ex(s, "shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100}))["id"]).unwrap();
    let g = id_of(&ex(s, "bool.subtract", json!({"ids": [a, b]}))["id"]).unwrap();
    ex(s, "select.none", json!({}));
    let ds = s.ds_mut().unwrap();
    ds.view.snap_objects = false;
    ds.view.snap_pixel = false;
    (g, a, b)
}

#[test]
fn clicks_select_the_group_and_reach_operands() {
    let mut s = Session::new();
    let (g, a, b) = subtract(&mut s);
    assert!((area(&s, g) - 7500.0).abs() < 1.0);
    click(&mut s, 20.0, 20.0, false);
    assert_eq!(s.selection(), vec![g], "a click selects the group");
    click(&mut s, 20.0, 20.0, true);
    assert_eq!(s.selection(), vec![a], "Cmd-click selects the operand");
    // With an operand selected, a click on another picks it.
    click(&mut s, 120.0, 120.0, false);
    assert_eq!(s.selection(), vec![b]);
    // A double click goes into the group, then into vector edit mode on the operand.
    ex(&mut s, "select.set", json!({"ids": [g]}));
    s.pointer(&ev(PointerKind::DoubleClick, 120.0, 120.0, false)).unwrap();
    assert_eq!(s.selection(), vec![b], "double click: the operand under the pointer");
    s.pointer(&ev(PointerKind::DoubleClick, 120.0, 120.0, false)).unwrap();
    let ve = s.ds().unwrap().vector_edit.clone().expect("vector edit on the operand");
    assert_eq!(ve.id, b);
    assert!(matches!(s.doc().unwrap().node(b).unwrap().kind, NodeKind::Vector { .. }));
    assert_eq!(s.doc().unwrap().node(b).unwrap().parent, Some(g), "still an operand");
    assert!((area(&s, g) - 7500.0).abs() < 1.0, "converting it changes nothing");
    // The canvas outlines every operand of the group while one is edited.
    assert_eq!(outlined_operands(s.doc().unwrap(), &s.selection(), Some(b)), vec![a, b]);
    assert!(outlined_operands(s.doc().unwrap(), &[g], None).is_empty(), "not for the group itself");
}

#[test]
fn moving_an_operand_updates_the_result_during_the_drag() {
    let mut s = Session::new();
    let (g, a, _) = subtract(&mut s);
    ex(&mut s, "select.set", json!({"ids": [a]}));
    s.pointer(&ev(PointerKind::Down, 20.0, 20.0, false)).unwrap();
    s.pointer(&ev(PointerKind::Move, 30.0, 20.0, false)).unwrap();
    s.pointer(&ev(PointerKind::Move, 40.0, 20.0, false)).unwrap();
    // Mid-drag: A spans 20..120, so the cut is 70 × 50.
    assert!((area(&s, g) - 6500.0).abs() < 1.0, "live: {}", area(&s, g));
    s.pointer(&ev(PointerKind::Up, 40.0, 20.0, false)).unwrap();
    let doc = s.doc().unwrap();
    assert!((area(&s, g) - 6500.0).abs() < 1.0, "after: {} {:?} {:?}", area(&s, g), doc.world_bounds(a), doc.node(g).unwrap().children);
    // The group's box follows its operands.
    let gb = s.doc().unwrap().world_bounds(g).unwrap();
    assert!((gb.x0 - 20.0).abs() < 1e-6, "{gb:?}");
    // One undo step for the drag.
    s.undo().unwrap();
    assert!((area(&s, g) - 7500.0).abs() < 1.0);
    assert!((s.doc().unwrap().world_bounds(a).unwrap().x0).abs() < 1e-6);
}

#[test]
fn editing_an_operands_points_updates_the_result_live() {
    let mut s = Session::new();
    let (g, a, _) = subtract(&mut s);
    ex(&mut s, "vector.edit.begin", json!({"id": a}));
    // Drag A's top-left corner 50 to the left: A becomes a 12,500 trapezoid, the cut stays.
    s.pointer(&ev(PointerKind::Down, 0.0, 0.0, false)).unwrap();
    s.pointer(&ev(PointerKind::Move, -25.0, 0.0, false)).unwrap();
    s.pointer(&ev(PointerKind::Move, -50.0, 0.0, false)).unwrap();
    assert!((area(&s, g) - 10000.0).abs() < 1.0, "live: {}", area(&s, g));
    s.pointer(&ev(PointerKind::Up, -50.0, 0.0, false)).unwrap();
    assert!((area(&s, g) - 10000.0).abs() < 1.0);
    s.undo().unwrap();
    assert!((area(&s, g) - 7500.0).abs() < 1.0, "one undo step: {}", area(&s, g));
}

#[test]
fn boolean_commands_set_operand_and_group_operations() {
    let mut s = Session::new();
    let (g, a, b) = subtract(&mut s);
    let c = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 200, "y": 0, "width": 50, "height": 50}))["id"]).unwrap();
    ex(&mut s, "node.reparent", json!({"ids": [c], "parent": g}));
    assert_eq!(s.doc().unwrap().node(c).unwrap().parent, Some(g));
    // Subtracting the top operand removes it; uniting it adds its 2,500.
    assert!((area(&s, g) - 7500.0).abs() < 1.0);
    let r = ex(&mut s, "bool.union", json!({"ids": [c]}));
    assert_eq!(r["operation"], json!("union"));
    assert_eq!(operand_op(s.doc().unwrap(), c), Some(BoolOp::Union));
    assert_eq!(distinct_op(s.doc().unwrap(), c), Some(BoolOp::Union));
    assert!((area(&s, g) - 10000.0).abs() < 1.0, "{}", area(&s, g));
    assert_eq!(s.ds().unwrap().doc.node(g).unwrap().children.len(), 3, "no new group");
    // Undo is one step; the operand follows the group again when set to its operation.
    s.undo().unwrap();
    assert!((area(&s, g) - 7500.0).abs() < 1.0);
    s.redo().unwrap();
    ex(&mut s, "bool.subtract", json!({"ids": [c]}));
    assert_eq!(s.doc().unwrap().node(c).unwrap().bool_op, None);
    // The controls show the selection's operation.
    ex(&mut s, "bool.intersect", json!({"ids": [b]}));
    assert_eq!(shown_op(s.doc().unwrap(), &[b]), Some(BoolOp::Intersect));
    assert_eq!(shown_op(s.doc().unwrap(), &[b, c]), None, "mixed");
    assert_eq!(shown_op(s.doc().unwrap(), &[g]), Some(BoolOp::Subtract));
    // (A ∩ B) − C: the 2,500 overlap.
    assert!((area(&s, g) - 2500.0).abs() < 1.0, "{}", area(&s, g));
    // One boolean group selected: its operation changes and every operand follows it.
    ex(&mut s, "bool.union", json!({"ids": [g]}));
    let doc = s.doc().unwrap();
    assert!(matches!(doc.node(g).unwrap().kind, NodeKind::BooleanOp { op: BoolOp::Union }));
    assert_eq!(doc.node(g).unwrap().name, "Union", "a default name follows the operation");
    assert!([a, b, c].iter().all(|i| doc.node(*i).unwrap().bool_op.is_none()));
    assert!((area(&s, g) - 20000.0).abs() < 1.0, "{}", area(&s, g));
    // Anything else is wrapped in a new group, whose operands follow it.
    let d = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 400, "y": 0, "width": 10, "height": 10}))["id"]).unwrap();
    let r = ex(&mut s, "bool.exclude", json!({"ids": [g, d]}));
    let outer = id_of(&r["id"]).unwrap();
    assert_ne!(outer, g);
    assert_eq!(s.doc().unwrap().node(g).unwrap().parent, Some(outer));
    // Flatten bakes the mixed result.
    ex(&mut s, "bool.subtract", json!({"ids": [c]}));
    let before = area(&s, g);
    let r = ex(&mut s, "bool.flatten", json!({"ids": [g]}));
    let flat = id_of(&r["id"]).unwrap();
    assert!((before - 17500.0).abs() < 1.0, "(A ∪ B) − C, C off to the side: {before}");
    assert!((area(&s, flat) - before).abs() < 2.0, "{} vs {before}", area(&s, flat));
}

#[test]
fn node_set_and_inspect_carry_operand_operations() {
    let mut s = Session::new();
    let (g, a, b) = subtract(&mut s);
    ex(&mut s, "node.set", json!({"id": b, "props": {"booleanOperation": "intersect"}}));
    assert_eq!(s.doc().unwrap().node(b).unwrap().bool_op, Some(BoolOp::Intersect));
    let n = ex(&mut s, "node.get", json!({"id": b}));
    assert_eq!(n["operandOperation"], json!("intersect"));
    ex(&mut s, "node.set", json!({"id": b, "props": {"operandOperation": null}}));
    assert_eq!(s.doc().unwrap().node(b).unwrap().bool_op, None);
    assert!(ex(&mut s, "node.get", json!({"id": b})).get("operandOperation").is_none());
    // On the group, booleanOperation is its own.
    ex(&mut s, "node.set", json!({"id": g, "props": {"booleanOperation": "exclude"}}));
    assert!(matches!(s.doc().unwrap().node(g).unwrap().kind, NodeKind::BooleanOp { op: BoolOp::Exclude }));
    // Errors: outside boolean groups, bad names.
    let lone = id_of(&ex(&mut s, "shape.ellipse", json!({"x": 300, "y": 300, "width": 10, "height": 10}))["id"]).unwrap();
    assert!(s.execute("node.set", &json!({"id": lone, "props": {"booleanOperation": "union"}})).is_err());
    assert!(s.execute("node.set", &json!({"id": a, "props": {"operandOperation": "sideways"}})).is_err());
    // Ungrouping releases the operands' own operations.
    ex(&mut s, "node.set", json!({"id": b, "props": {"operandOperation": "union"}}));
    ex(&mut s, "object.ungroup", json!({"ids": [g]}));
    assert_eq!(s.doc().unwrap().node(b).unwrap().bool_op, None);
}

#[test]
fn boolean_commands_survive_junk() {
    let junk = [
        json!({}),
        json!(null),
        json!({"ids": "x"}),
        json!({"ids": [999_999]}),
        json!({"ids": [1e300, -1]}),
        json!({"id": {"nested": true}}),
        json!({"props": {"booleanOperation": [1, 2]}}),
    ];
    for cmd in ["bool.union", "bool.subtract", "bool.intersect", "bool.exclude", "bool.flatten", "node.set"] {
        for j in &junk {
            let mut s = Session::new();
            let (g, _, b) = subtract(&mut s);
            ex(&mut s, "select.set", json!({"ids": [b]}));
            if let Err(crate::EngineError::Internal { cmd, msg }) = s.execute(cmd, j) {
                panic!("{cmd} panicked on {j}: {msg}");
            }
            if let Some(n) = s.doc().unwrap().node(g) {
                let _ = figcraft_render::node_outline(s.doc().unwrap(), n);
            }
        }
    }
    // node.set with junk operations on an operand.
    let mut s = Session::new();
    let (_, _, b) = subtract(&mut s);
    for v in [json!(7), json!({"op": "union"}), json!(""), json!(true)] {
        assert!(s.execute("node.set", &json!({"id": b, "props": {"operandOperation": v}})).is_err(), "{v}");
    }
}
