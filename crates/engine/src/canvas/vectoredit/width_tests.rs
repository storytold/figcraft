//! Variable width strokes: the width profile command, the Variable width tool in vector edit mode
//! (gestures, keys, undo) and what outline stroke and export make of a profile.

use serde_json::{Value, json};

use super::*;
use crate::EngineError;
use crate::tools::{PointerEvent, PointerKind};
use figcraft_doc::{ExportFormat, WidthPreset, WidthProfile, WidthSide};

fn ex(s: &mut Session, c: &str, p: Value) -> Value {
    s.execute(c, &p).unwrap_or_else(|e| panic!("{c}: {e}"))
}

fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
    PointerEvent::at(kind, x, y, 1.0)
}

fn drag(s: &mut Session, from: (f64, f64), to: (f64, f64)) {
    s.pointer(&ev(PointerKind::Down, from.0, from.1)).unwrap();
    s.pointer(&ev(PointerKind::Move, (from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0)).unwrap();
    s.pointer(&ev(PointerKind::Move, to.0, to.1)).unwrap();
    s.pointer(&ev(PointerKind::Up, to.0, to.1)).unwrap();
}

/// An open path from (0, 50) to (100, 50) with a 10 wide centred stroke.
fn path(s: &mut Session) -> NodeId {
    let r = ex(s, "vector.create", json!({"points": [[0, 50], [100, 50]], "closed": false}));
    let id = crate::params::id_of(&r["id"]).unwrap();
    ex(s, "node.set", json!({"id": id, "props": {"strokes": ["#000000"], "strokeWeight": 10, "strokeAlign": "center"}}));
    id
}

fn profile(s: &Session, id: NodeId) -> Option<WidthProfile> {
    s.doc().unwrap().node(id).unwrap().stroke.width_profile.clone()
}

fn points(s: &Session, id: NodeId) -> Vec<(f64, f64)> {
    profile(s, id).map(|p| p.points.iter().map(|q| ((q.t * 1e6).round() / 1e6, (q.scale * 1e6).round() / 1e6)).collect()).unwrap_or_default()
}

fn ve(s: &Session) -> VectorEdit {
    s.ds().unwrap().vector_edit.clone().unwrap()
}

#[test]
fn width_profile_command_sets_presets_and_points() {
    let mut s = Session::new();
    let id = path(&mut s);
    ex(&mut s, "stroke.widthProfile", json!({"ids": [id], "preset": "eye"}));
    assert_eq!(profile(&s, id), WidthPreset::Eye.profile());
    // Inspect reports it (preset and points), and node.set takes it back.
    let n = ex(&mut s, "node.get", json!({"id": id}));
    assert_eq!(n["widthProfile"]["preset"], json!("eye"));
    let back = n["widthProfile"].clone();
    ex(&mut s, "stroke.widthProfile", json!({"ids": [id], "points": [[1, 2], [0, 0.5]]}));
    assert_eq!(points(&s, id), vec![(0.0, 0.5), (1.0, 2.0)], "sorted");
    ex(&mut s, "node.set", json!({"id": id, "props": {"widthProfile": back}}));
    assert_eq!(profile(&s, id), WidthPreset::Eye.profile());
    ex(&mut s, "node.set", json!({"id": id, "props": {"widthProfile": [{"t": 0.5, "scale": 3}]}}));
    assert_eq!(points(&s, id), vec![(0.5, 3.0)]);
    // Uniform (or nothing) clears it; one undo step each.
    ex(&mut s, "stroke.widthProfile", json!({"ids": [id], "preset": "uniform"}));
    assert_eq!(profile(&s, id), None);
    s.undo().unwrap();
    assert_eq!(points(&s, id), vec![(0.5, 3.0)]);
    ex(&mut s, "stroke.widthProfile", json!({"ids": [id]}));
    assert_eq!(profile(&s, id), None);
    assert!(s.execute("stroke.widthProfile", &json!({"ids": [id], "preset": "zigzag"})).is_err());
    assert!(s.execute("stroke.widthProfile", &json!({"ids": [id], "points": [[0, "x"]]})).is_err());
    assert!(s.execute("node.set", &json!({"id": id, "props": {"widthProfile": 7}})).is_err());
}

#[test]
fn width_tool_drags_add_slide_and_size_points_in_one_step_each() {
    let mut s = Session::new();
    let id = path(&mut s);
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    assert!(key(&mut s, "W", true, false, false).unwrap());
    assert_eq!(ve(&s).tool, VTool::Width);
    // Hovering the stroke offers to add a point there.
    s.pointer(&ev(PointerKind::Hover, 50.0, 52.0)).unwrap();
    assert!(matches!(s.tool.overlay.width_hover, Some(WHit::Path { t, side: WidthSide::Right }) if (t - 0.5).abs() < 1e-9));
    // Dragging from the path adds a point and sets its width: 20 from the centre = 40 wide.
    drag(&mut s, (50.0, 50.0), (50.0, 70.0));
    assert_eq!(points(&s, id), vec![(0.0, 1.0), (0.5, 4.0), (1.0, 1.0)]);
    assert_eq!(ve(&s).width_point, Some(1));
    let b = s.doc().unwrap().world_render_bounds(id).unwrap();
    assert!(b.y0 <= 30.0 + 1e-6 && b.y1 >= 70.0 - 1e-6, "paint bounds follow the profile: {b:?}");
    s.undo().unwrap();
    assert_eq!(profile(&s, id), None, "adding and sizing is one step");
    s.redo().unwrap();
    // Its handles sit 20 either side; dragging one sets the width.
    let hs = width_handles(s.doc().unwrap(), &ve(&s), 1.0);
    let h = hs.iter().find(|h| h.index == 1).unwrap();
    assert!((h.sides[0][1] - 70.0).abs() < 1e-6 || (h.sides[1][1] - 70.0).abs() < 1e-6, "{h:?}");
    s.pointer(&ev(PointerKind::Hover, 50.0, 30.0)).unwrap();
    assert_eq!(s.tool.overlay.width_hover, Some(WHit::Handle { index: 1, side: WidthSide::Left }));
    drag(&mut s, (50.0, 30.0), (50.0, 45.0));
    assert_eq!(points(&s, id), vec![(0.0, 1.0), (0.5, 1.0), (1.0, 1.0)]);
    // Dragging the point slides it along the path.
    drag(&mut s, (50.0, 50.0), (80.0, 58.0));
    assert_eq!(points(&s, id), vec![(0.0, 1.0), (0.8, 1.0), (1.0, 1.0)]);
    s.undo().unwrap();
    assert_eq!(points(&s, id)[1], (0.5, 1.0));
    // Delete removes the selected width point (not the path's points).
    s.redo().unwrap();
    assert!(key(&mut s, "Delete", false, false, false).unwrap());
    assert_eq!(points(&s, id), vec![(0.0, 1.0), (1.0, 1.0)]);
    assert_eq!(network(s.doc().unwrap(), id).unwrap().vertices.len(), 2);
    // A click away from the stroke deselects.
    s.pointer(&ev(PointerKind::Down, 50.0, 0.0)).unwrap();
    s.pointer(&ev(PointerKind::Up, 50.0, 0.0)).unwrap();
    assert_eq!(ve(&s).width_point, None);
}

#[test]
fn width_point_command_adds_moves_and_removes() {
    let mut s = Session::new();
    let id = path(&mut s);
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    let r = ex(&mut s, "vector.edit.widthPoint", json!({"t": 0.25, "width": 30}));
    assert_eq!(r["index"], json!(1));
    assert_eq!(points(&s, id), vec![(0.0, 1.0), (0.25, 3.0), (1.0, 1.0)]);
    ex(&mut s, "vector.edit.widthPoint", json!({"index": 1, "t": 0.75, "scale": 0.5}));
    assert_eq!(points(&s, id), vec![(0.0, 1.0), (0.75, 0.5), (1.0, 1.0)]);
    // Without a width the point keeps the width already there.
    let before = profile(&s, id).unwrap();
    ex(&mut s, "vector.edit.widthPoint", json!({"action": "add", "t": 0.5}));
    let p = profile(&s, id).unwrap();
    assert!((p.points[1].scale - before.scale_at(0.5)).abs() < 1e-9, "{p:?}");
    let st = ex(&mut s, "vector.edit.state", json!({}));
    assert_eq!(st["widthPoints"].as_array().unwrap().len(), 4);
    for i in [3, 2, 1, 0] {
        ex(&mut s, "vector.edit.widthPoint", json!({"action": "remove", "index": i}));
    }
    assert_eq!(profile(&s, id), None, "removing every point makes the stroke uniform");
    assert!(s.execute("vector.edit.widthPoint", &json!({"index": 5, "t": 0.5})).is_err());
    assert!(s.execute("vector.edit.widthPoint", &json!({"action": "remove", "index": 0})).is_err());
    assert!(s.execute("vector.edit.widthPoint", &json!({"action": "spin"})).is_err());
    assert!(s.execute("vector.edit.widthPoint", &json!({"action": "add"})).is_err(), "add needs t");
    ex(&mut s, "vector.edit.end", json!({}));
    assert!(matches!(s.execute("vector.edit.widthPoint", &json!({"t": 0.5})), Err(EngineError::Disabled(..))));
}

#[test]
fn outline_stroke_and_exports_follow_the_profile() {
    let mut s = Session::new();
    let id = path(&mut s);
    ex(&mut s, "stroke.widthProfile", json!({"ids": [id], "points": [[0, 0], [0.5, 4], [1, 0]]}));
    // SVG writes the varying outline as a filled path (non-zero), PDF and PNG render.
    let doc = s.doc().unwrap().clone();
    let svg = String::from_utf8(crate::cmd::file::export_bytes(&doc, &[id], ExportFormat::Svg, 1.0).unwrap()).unwrap();
    assert!(svg.contains("<path") && !svg.contains("evenodd"), "{svg}");
    let pdf = crate::cmd::file::export_bytes(&doc, &[id], ExportFormat::Pdf, 1.0).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    let png = crate::cmd::file::export_bytes(&doc, &[id], ExportFormat::Png, 1.0).unwrap();
    assert!(!png.is_empty());
    // Outline stroke makes a vector shaped like the varying stroke: 40 tall in the middle.
    let r = ex(&mut s, "vector.outlineStroke", json!({"ids": [id]}));
    let v = crate::params::id_of(&r["ids"][0]).unwrap();
    let b = s.doc().unwrap().world_bounds(v).unwrap();
    assert!((b.height() - 40.0).abs() < 0.5 && (b.width() - 100.0).abs() < 0.5, "{b:?}");
    let n = s.doc().unwrap().node(v).unwrap();
    assert!(n.stroke.profile().is_none() || n.strokes.is_empty());
}

#[test]
fn width_commands_survive_junk() {
    let junk = [
        json!({}),
        json!(null),
        json!({"index": -1, "t": "NaN", "width": 1e308, "scale": -5}),
        json!({"action": "remove", "index": 1e12}),
        json!({"action": "add", "t": 1e300, "scale": 1e300}),
        json!({"preset": 7, "points": [[0, 0], [1], "x"]}),
        json!({"points": {"points": {"preset": "eye"}}}),
    ];
    for cmd in ["stroke.widthProfile", "vector.edit.widthPoint"] {
        for j in &junk {
            let mut s = Session::new();
            let id = path(&mut s);
            ex(&mut s, "vector.edit.begin", json!({"id": id}));
            if let Err(EngineError::Internal { cmd, msg }) = s.execute(cmd, j) {
                panic!("{cmd} panicked on {j}: {msg}");
            }
            if let Some(p) = profile(&s, id) {
                assert!(p.points.iter().all(|q| q.t.is_finite() && (0.0..=1.0).contains(&q.t) && q.scale.is_finite()), "{cmd} {j}: {p:?}");
            }
        }
    }
}
