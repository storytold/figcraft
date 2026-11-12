//! One-sided widths with the Variable width tool: Alt-dragging a handle or the path changes one
//! side, a plain drag evens them out, the width point command's `side`, `widthProfile` points with
//! two sides, and what outline stroke and export make of them.

use serde_json::{Value, json};

use super::*;
use crate::EngineError;
use crate::tools::{PointerEvent, PointerKind};
use figcraft_doc::{ExportFormat, WidthSide};

fn ex(s: &mut Session, c: &str, p: Value) -> Value {
    s.execute(c, &p).unwrap_or_else(|e| panic!("{c}: {e}"))
}

fn ev(kind: PointerKind, x: f64, y: f64, alt: bool) -> PointerEvent {
    PointerEvent { alt, ..PointerEvent::at(kind, x, y, 1.0) }
}

fn drag(s: &mut Session, from: (f64, f64), to: (f64, f64), alt: bool) {
    s.pointer(&ev(PointerKind::Down, from.0, from.1, alt)).unwrap();
    s.pointer(&ev(PointerKind::Move, (from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0, alt)).unwrap();
    s.pointer(&ev(PointerKind::Move, to.0, to.1, alt)).unwrap();
    s.pointer(&ev(PointerKind::Up, to.0, to.1, alt)).unwrap();
}

/// An open path from (0, 50) to (100, 50) (its right side is below it) with a 10 wide centred
/// stroke, in vector edit mode with the Variable width tool.
fn path(s: &mut Session) -> NodeId {
    let r = ex(s, "vector.create", json!({"points": [[0, 50], [100, 50]], "closed": false}));
    let id = crate::params::id_of(&r["id"]).unwrap();
    ex(s, "node.set", json!({"id": id, "props": {"strokes": ["#000000"], "strokeWeight": 10, "strokeAlign": "center"}}));
    ex(s, "vector.edit.begin", json!({"id": id}));
    ex(s, "vector.edit.tool", json!({"tool": "width"}));
    id
}

/// The side scales of width point `i`, rounded.
fn sides(s: &Session, id: NodeId, i: usize) -> (f64, f64) {
    let p = s.doc().unwrap().node(id).unwrap().stroke.profile().unwrap().points[i];
    let (l, r) = p.sides();
    ((l * 1e6).round() / 1e6, (r * 1e6).round() / 1e6)
}

fn ve(s: &Session) -> VectorEdit {
    s.ds().unwrap().vector_edit.clone().unwrap()
}

#[test]
fn alt_dragging_a_handle_changes_that_side_only() {
    let mut s = Session::new();
    let id = path(&mut s);
    // A point 40 wide in the middle: 20 either side.
    ex(&mut s, "vector.edit.widthPoint", json!({"t": 0.5, "width": 40}));
    assert_eq!(sides(&s, id, 1), (4.0, 4.0));
    s.pointer(&ev(PointerKind::Hover, 50.0, 70.0, true)).unwrap();
    assert_eq!(s.tool.overlay.width_hover, Some(WHit::Handle { index: 1, side: WidthSide::Right }));
    // Alt-drag the right handle in to 10 from the centre: the left keeps its 20.
    drag(&mut s, (50.0, 70.0), (50.0, 60.0), true);
    assert_eq!(sides(&s, id, 1), (4.0, 2.0));
    let hs = width_handles(s.doc().unwrap(), &ve(&s), 1.0);
    let h = hs.iter().find(|h| h.index == 1).unwrap();
    assert!(h.one_sided && (h.sides[0][1] - 60.0).abs() < 1e-6 && (h.sides[1][1] - 30.0).abs() < 1e-6, "{h:?}");
    assert!((h.width - 30.0).abs() < 1e-6);
    // The painted stroke follows: 20 above, 10 below in the middle.
    let b = s.doc().unwrap().world_render_bounds(id).unwrap();
    assert!(b.y0 <= 30.0 + 1e-6, "{b:?}");
    // One undo step; a plain drag makes both sides as wide again.
    s.undo().unwrap();
    assert_eq!(sides(&s, id, 1), (4.0, 4.0));
    s.redo().unwrap();
    drag(&mut s, (50.0, 30.0), (50.0, 35.0), false);
    assert_eq!(sides(&s, id, 1), (3.0, 3.0));
    // Alt past the centre line leaves that side at nothing.
    drag(&mut s, (50.0, 35.0), (50.0, 55.0), true);
    assert_eq!(sides(&s, id, 1), (0.0, 3.0));
}

#[test]
fn alt_dragging_the_path_adds_a_point_widening_the_side_the_pointer_goes() {
    let mut s = Session::new();
    let id = path(&mut s);
    // From the path at a quarter of the way, up (the left side) to 15 out.
    drag(&mut s, (25.0, 50.0), (25.0, 35.0), true);
    let pts = s.doc().unwrap().node(id).unwrap().stroke.profile().unwrap().points.clone();
    assert_eq!(pts.len(), 3);
    assert_eq!(sides(&s, id, 1), (3.0, 1.0), "{pts:?}");
    s.undo().unwrap();
    assert!(s.doc().unwrap().node(id).unwrap().stroke.profile().is_none(), "adding and sizing is one step");
}

#[test]
fn the_width_point_command_sets_one_side() {
    let mut s = Session::new();
    let id = path(&mut s);
    ex(&mut s, "vector.edit.widthPoint", json!({"t": 0.5, "width": 20}));
    // `width` with a side: the distance from the centre line to that edge.
    ex(&mut s, "vector.edit.widthPoint", json!({"index": 1, "side": "left", "width": 15}));
    assert_eq!(sides(&s, id, 1), (3.0, 2.0));
    ex(&mut s, "vector.edit.widthPoint", json!({"index": 1, "side": "right", "scale": 0.5}));
    assert_eq!(sides(&s, id, 1), (3.0, 0.5));
    // Adding with a side widens that side of what is there.
    let r = ex(&mut s, "vector.edit.widthPoint", json!({"t": 0.9, "side": "right", "width": 25}));
    let i = r["index"].as_u64().unwrap() as usize;
    let (l, rr) = sides(&s, id, i);
    assert!((rr - 5.0).abs() < 1e-6 && l > 0.9 && l < 3.0, "{l} {rr}");
    // State and inspect report two-sided points as [t, left, right].
    let n = ex(&mut s, "node.get", json!({"id": id}));
    let p1: Vec<f64> = n["widthProfile"]["points"][1].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    assert!(p1.len() == 3 && [0.5, 3.0, 0.5].iter().zip(&p1).all(|(a, b)| (a - b).abs() < 1e-9), "{p1:?}");
    // A width without a side evens the point out again.
    ex(&mut s, "vector.edit.widthPoint", json!({"index": 1, "width": 10}));
    assert_eq!(sides(&s, id, 1), (1.0, 1.0));
    assert!(s.execute("vector.edit.widthPoint", &json!({"index": 1, "side": "up", "width": 3})).is_err());
}

#[test]
fn profiles_with_two_sides_round_trip_and_outline() {
    let mut s = Session::new();
    let id = path(&mut s);
    ex(&mut s, "vector.edit.end", json!({}));
    // [t, left, right] and {t, left, right}: all of the width below the path.
    ex(&mut s, "node.set", json!({"id": id, "props": {"widthProfile": [[0, 0, 2], {"t": 1, "left": 0, "right": 2}]}}));
    assert_eq!(sides(&s, id, 0), (0.0, 2.0));
    assert_eq!(sides(&s, id, 1), (0.0, 2.0));
    let back = ex(&mut s, "node.get", json!({"id": id}))["widthProfile"].clone();
    ex(&mut s, "node.set", json!({"id": id, "props": {"widthProfile": null}}));
    ex(&mut s, "node.set", json!({"id": id, "props": {"widthProfile": back}}));
    assert_eq!(sides(&s, id, 1), (0.0, 2.0));
    // Exports render it; outline stroke makes a vector 10 tall, all below the path.
    let doc = s.doc().unwrap().clone();
    for f in [ExportFormat::Svg, ExportFormat::Pdf, ExportFormat::Png] {
        assert!(!crate::cmd::file::export_bytes(&doc, &[id], f, 1.0).unwrap().is_empty());
    }
    let r = ex(&mut s, "vector.outlineStroke", json!({"ids": [id]}));
    let v = crate::params::id_of(&r["ids"][0]).unwrap();
    let b = s.doc().unwrap().world_bounds(v).unwrap();
    assert!((b.y0 - 50.0).abs() < 0.05 && (b.y1 - 60.0).abs() < 0.05 && (b.width() - 100.0).abs() < 0.05, "{b:?}");
}

#[test]
fn one_sided_widths_survive_junk() {
    let junk = [
        json!({"index": 1, "side": 7, "width": 1}),
        json!({"index": 1, "side": "left", "width": "NaN"}),
        json!({"index": 1, "side": "right", "scale": 1e308}),
        json!({"t": 0.5, "side": "left", "scale": -1e9}),
        json!({"index": 99, "side": "left", "width": 1}),
    ];
    for j in &junk {
        let mut s = Session::new();
        let id = path(&mut s);
        ex(&mut s, "vector.edit.widthPoint", json!({"t": 0.5, "width": 20}));
        if let Err(EngineError::Internal { cmd, msg }) = s.execute("vector.edit.widthPoint", j) {
            panic!("{cmd} panicked on {j}: {msg}");
        }
        let p = s.doc().unwrap().node(id).unwrap().stroke.profile().cloned().unwrap();
        assert!(p.points.iter().all(|q| q.scale.is_finite() && q.bias.is_finite() && (-1.0..=1.0).contains(&q.bias)), "{j}: {p:?}");
    }
    for v in [json!([[0, 1, "x"]]), json!([{"t": 0, "left": 1}]), json!([[0, -5, 1e308], [1, f64::NAN, 2]])] {
        let mut s = Session::new();
        let id = path(&mut s);
        let _ = s.execute("node.set", &json!({"id": id, "props": {"widthProfile": v}}));
        if let Some(p) = s.doc().unwrap().node(id).unwrap().stroke.profile() {
            assert!(p.points.iter().all(|q| q.scale.is_finite() && q.scale >= 0.0 && q.bias.is_finite()));
        }
    }
}
