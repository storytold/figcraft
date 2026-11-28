//! The shape builder: regions, merge and remove commands, the tool's gestures, keys and undo.

use serde_json::{Value, json};

use crate::shapebuilder::{arrangement, current};
use crate::tools::{PointerEvent, PointerKind};
use crate::{EngineError, Session, ToolId, params};
use figcraft_doc::{NodeId, NodeKind};
use figcraft_geom::Shape;

fn ex(s: &mut Session, c: &str, p: Value) -> Value {
    s.execute(c, &p).unwrap_or_else(|e| panic!("{c}: {e}"))
}

fn id(v: &Value) -> NodeId {
    params::id_of(&v["id"]).unwrap()
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64, fill: &str) -> NodeId {
    id(&ex(s, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h, "props": {"fill": fill}})))
}

/// Two 10×10 squares overlapping in a 5×5 square: A (red) at the origin, B (blue) above it at 5,5.
fn squares(s: &mut Session) -> (NodeId, NodeId) {
    let a = rect(s, 0.0, 0.0, 10.0, 10.0, "#ff0000");
    let b = rect(s, 5.0, 5.0, 10.0, 10.0, "#0000ff");
    s.set_selection(vec![a, b]);
    (a, b)
}

/// A layer's filled area in page units.
fn area(s: &Session, id: NodeId) -> f64 {
    let doc = s.doc().unwrap();
    let (mut p, _) = figcraft_render::node_outline(doc, doc.node(id).unwrap()).unwrap();
    p.apply_affine(doc.world_transform(id));
    p.area().abs()
}

fn fill_hex(s: &Session, id: NodeId) -> String {
    s.doc().unwrap().node(id).unwrap().fills[0].solid_color().unwrap().to_hex()
}

fn ev(kind: PointerKind, x: f64, y: f64, alt: bool) -> PointerEvent {
    PointerEvent { alt, ..PointerEvent::at(kind, x, y, 1.0) }
}

fn drag(s: &mut Session, pts: &[(f64, f64)], alt: bool) {
    let (first, rest) = pts.split_first().unwrap();
    s.pointer(&ev(PointerKind::Down, first.0, first.1, alt)).unwrap();
    for p in rest {
        s.pointer(&ev(PointerKind::Move, p.0, p.1, alt)).unwrap();
    }
    let last = pts.last().unwrap();
    s.pointer(&ev(PointerKind::Up, last.0, last.1, alt)).unwrap();
}

#[test]
fn overlapping_squares_make_three_regions() {
    let mut s = Session::new();
    let (a, b) = squares(&mut s);
    let r = ex(&mut s, "shapeBuilder.regions", json!({}));
    assert_eq!(r["sources"], json!([a, b]));
    let regions = r["regions"].as_array().unwrap();
    assert_eq!(regions.len(), 3, "{r}");
    let arr = arrangement(&mut s, &[a, b]).unwrap();
    let only_a = arr.region_at((2.0, 2.0).into()).unwrap();
    let both = arr.region_at((7.0, 7.0).into()).unwrap();
    let only_b = arr.region_at((12.0, 12.0).into()).unwrap();
    assert_eq!(arr.regions[only_a].sources, vec![a]);
    assert_eq!(arr.regions[both].sources, vec![b, a], "top-most first");
    assert_eq!(arr.regions[only_b].sources, vec![b]);
    assert!(arr.region_at((20.0, 20.0).into()).is_none());
    // Every region reports a point inside itself.
    for (i, reg) in regions.iter().enumerate() {
        let p = reg["point"].as_array().unwrap();
        assert_eq!(arr.region_at((p[0].as_f64().unwrap(), p[1].as_f64().unwrap()).into()), Some(i));
    }
    // Cached per document version and ids.
    assert!(std::sync::Arc::ptr_eq(&arr, &arrangement(&mut s, &[a, b]).unwrap()));
    assert!(current(&s).is_some_and(|c| std::sync::Arc::ptr_eq(&c, &arr)));
    rect(&mut s, 40.0, 40.0, 5.0, 5.0, "#00ff00");
    assert!(current(&s).is_none(), "a new document version (and selection) invalidates the cache");
}

#[test]
fn merge_the_overlap_into_a_new_shape() {
    let mut s = Session::new();
    let (a, b) = squares(&mut s);
    let undo = s.ds().unwrap().history.undo.len();
    let r = ex(&mut s, "shapeBuilder.merge", json!({"points": [[7, 7]]}));
    let m = id(&r);
    assert!((area(&s, m) - 25.0).abs() < 0.01, "{}", area(&s, m));
    assert_eq!(fill_hex(&s, m), "0000FF", "styled like the top-most shape over the region");
    assert!((area(&s, a) - 75.0).abs() < 0.01, "A loses the overlap: {}", area(&s, a));
    assert!((area(&s, b) - 75.0).abs() < 0.01, "B loses the overlap: {}", area(&s, b));
    assert_eq!(fill_hex(&s, a), "FF0000");
    let doc = s.doc().unwrap();
    assert!(matches!(doc.node(a).unwrap().kind, NodeKind::Vector { .. }));
    assert_eq!(doc.index_in_parent(m), doc.index_in_parent(b).map(|i| i + 1), "just above the style source");
    let sel = s.selection();
    assert!(sel.contains(&a) && sel.contains(&b) && sel.contains(&m), "{sel:?}");
    // One undo step brings the rectangles back.
    assert_eq!(s.ds().unwrap().history.undo.len(), undo + 1);
    assert!(s.undo().unwrap());
    let doc = s.doc().unwrap();
    assert!(!doc.contains(m));
    assert!(matches!(doc.node(a).unwrap().kind, NodeKind::Rectangle));
    assert!((area(&s, a) - 100.0).abs() < 1e-6);
}

#[test]
fn merging_every_region_unites_the_shapes() {
    let mut s = Session::new();
    let (a, b) = squares(&mut s);
    let r = ex(&mut s, "shapeBuilder.merge", json!({"regions": [0, 1, 2]}));
    let m = id(&r);
    assert_eq!(r["removed"], json!([a, b]));
    assert!((area(&s, m) - 175.0).abs() < 0.01, "{}", area(&s, m));
    assert_eq!(s.selection(), vec![m]);
    // Merging a lone shape's only region is a no-op.
    let before = s.doc().unwrap().clone();
    let r = ex(&mut s, "shapeBuilder.merge", json!({"regions": [0]}));
    assert!(r["id"].is_null());
    assert_eq!(*s.doc().unwrap(), before);
}

#[test]
fn remove_regions() {
    let mut s = Session::new();
    let (a, b) = squares(&mut s);
    ex(&mut s, "shapeBuilder.remove", json!({"points": [[7, 7], [12, 12]]}));
    assert!(!s.doc().unwrap().contains(b), "B had nothing left");
    assert!((area(&s, a) - 75.0).abs() < 0.01);
    assert_eq!(s.selection(), vec![a]);
    let e = s.execute("shapeBuilder.remove", &json!({"points": [[100, 100]]}));
    assert!(matches!(e, Err(EngineError::BadParams { .. })), "{e:?}");
}

#[test]
fn curved_shapes_keep_their_curves() {
    let mut s = Session::new();
    let a = id(&ex(&mut s, "shape.ellipse", json!({"x": 0, "y": 0, "width": 100, "height": 100})));
    let b = id(&ex(&mut s, "shape.ellipse", json!({"x": 50, "y": 0, "width": 100, "height": 100})));
    s.set_selection(vec![a, b]);
    let r = ex(&mut s, "shapeBuilder.regions", json!({}));
    assert_eq!(r["regions"].as_array().unwrap().len(), 3);
    let m = id(&ex(&mut s, "shapeBuilder.merge", json!({"points": [[75, 50]]})));
    // The lens of two r=50 circles 50 apart: 2r²·acos(d/2r) − (d/2)·√(4r² − d²).
    let lens = 2.0 * 2500.0 * (0.5f64).acos() - 25.0 * (10000.0f64 - 2500.0).sqrt();
    assert!((area(&s, m) - lens).abs() < lens * 0.01, "{} vs {lens}", area(&s, m));
    let doc = s.doc().unwrap();
    let NodeKind::Vector { network } = &doc.node(m).unwrap().kind else { panic!("not a vector") };
    assert!(network.segments.iter().any(|g| !g.is_straight()), "the lens keeps curved edges");
    let full = std::f64::consts::PI * 2500.0;
    assert!((area(&s, a) - (full - lens)).abs() < full * 0.01);
}

#[test]
fn tool_click_drag_alt_and_keys() {
    let mut s = Session::new();
    let (a, b) = squares(&mut s);
    ex(&mut s, "tool.shapeBuilder", json!({}));
    assert_eq!(s.tool.tool, ToolId::ShapeBuilder);
    // Hover highlights the region under the pointer.
    s.pointer(&ev(PointerKind::Hover, 7.0, 7.0, false)).unwrap();
    let both = current(&s).unwrap().region_at((7.0, 7.0).into()).unwrap();
    assert_eq!(s.tool.overlay.builder_regions, vec![both]);
    assert!(!s.tool.overlay.builder_remove);
    s.pointer(&ev(PointerKind::Hover, 7.0, 7.0, true)).unwrap();
    assert!(s.tool.overlay.builder_remove, "Alt removes");
    // Alt-drag from A's own part across the overlap removes both regions: one undo step.
    let undo = s.ds().unwrap().history.undo.len();
    drag(&mut s, &[(2.0, 2.0), (4.0, 4.0), (7.0, 7.0)], true);
    assert_eq!(s.ds().unwrap().history.undo.len(), undo + 1);
    assert!(!s.doc().unwrap().contains(a));
    assert!((area(&s, b) - 75.0).abs() < 0.01);
    assert!(s.tool.overlay.builder_regions.is_empty());
    assert!(s.undo().unwrap());
    // A click merges the region under it: B keeps only the overlap.
    s.set_selection(vec![a, b]);
    drag(&mut s, &[(12.0, 12.0)], false);
    let sel = s.selection();
    assert_eq!(sel.len(), 3, "{sel:?}");
    assert!((area(&s, b) - 25.0).abs() < 0.01);
    // A click outside every region selects the layer there; Enter finishes the tool.
    let c = {
        s.set_tool(ToolId::Move);
        let c = rect(&mut s, 40.0, 40.0, 5.0, 5.0, "#00ff00");
        s.set_selection(vec![a]);
        s.set_tool(ToolId::ShapeBuilder);
        c
    };
    drag(&mut s, &[(42.0, 42.0)], false);
    assert_eq!(s.selection(), vec![c]);
    assert!(s.tool_key("Enter", false, false, false).unwrap());
    assert_eq!(s.tool.tool, ToolId::Move);
    assert!(!s.tool_key("Enter", false, false, false).unwrap(), "Enter is not the tool's outside it");
}

#[test]
fn remove_mode_and_escape() {
    let mut s = Session::new();
    let (a, _) = squares(&mut s);
    ex(&mut s, "tool.shapeBuilder", json!({}));
    ex(&mut s, "shapeBuilder.mode", json!({"mode": "remove"}));
    s.pointer(&ev(PointerKind::Down, 2.0, 2.0, false)).unwrap();
    assert!(s.tool.busy());
    // Escape cancels the gesture, then leaves the tool.
    s.escape().unwrap();
    assert!(!s.tool.busy());
    assert!(s.doc().unwrap().contains(a));
    // In Remove mode a plain click removes and Alt merges.
    drag(&mut s, &[(2.0, 2.0)], false);
    assert!((area(&s, a) - 25.0).abs() < 0.01);
    s.escape().unwrap();
    assert_eq!(s.tool.tool, ToolId::Move);
    assert!(s.execute("shapeBuilder.mode", &json!({"mode": "sideways"})).is_err());
}

#[test]
fn a_group_named_many_times_is_walked_once() {
    let mut s = Session::new();
    let (a, b) = squares(&mut s);
    let g = id(&ex(&mut s, "object.group", json!({"ids": [a, b]})));
    let r = ex(&mut s, "shapeBuilder.regions", json!({"ids": vec![g; 500]}));
    assert_eq!(r["sources"], json!([a, b]));
}

#[test]
fn groups_rotated_and_ineligible_layers() {
    let mut s = Session::new();
    let (a, b) = squares(&mut s);
    let g = id(&ex(&mut s, "object.group", json!({"ids": [a, b]})));
    let line = id(&ex(&mut s, "shape.line", json!({"x": 0, "y": 5, "width": 20})));
    let frame = id(&ex(&mut s, "frame.create", json!({"x": 100, "y": 100, "width": 50, "height": 50})));
    let r = ex(&mut s, "shapeBuilder.regions", json!({"ids": [g, line, frame]}));
    assert_eq!(r["sources"], json!([a, b, line]), "groups give their shapes; lines take part through their strokes; frames don't");
    // The merged shape lands in the group, which stays selected.
    s.set_selection(vec![g]);
    let m = id(&ex(&mut s, "shapeBuilder.merge", json!({"points": [[7, 7]]})));
    assert_eq!(s.doc().unwrap().node(m).unwrap().parent, Some(g));
    assert_eq!(s.selection(), vec![g]);
    // Rotated shapes reshape in their own axes and keep their rotation.
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0, "#ff0000");
    let b = rect(&mut s, 5.0, 0.0, 10.0, 10.0, "#0000ff");
    ex(&mut s, "node.set", json!({"id": b, "props": {"rotation": 30}}));
    let t_before = s.doc().unwrap().node(b).unwrap().transform;
    let total = area(&s, a) + area(&s, b);
    s.set_selection(vec![a, b]);
    let regions = ex(&mut s, "shapeBuilder.regions", json!({}));
    let n = regions["regions"].as_array().unwrap().len();
    assert!(n >= 3, "{regions}");
    let both = arrangement(&mut s, &[a, b]).unwrap().regions.iter().position(|r| r.sources.len() == 2).unwrap();
    let m = id(&ex(&mut s, "shapeBuilder.merge", json!({"regions": [both]})));
    let after = area(&s, a) + area(&s, b) + 2.0 * area(&s, m);
    assert!((after - total).abs() < 0.05, "areas are conserved: {after} vs {total}");
    let t_after = s.doc().unwrap().node(b).unwrap().transform;
    let rot = |t: figcraft_geom::Affine| t.as_coeffs()[1].atan2(t.as_coeffs()[0]);
    assert!((rot(t_before) - rot(t_after)).abs() < 1e-9);
}

#[test]
fn junk_params_never_panic() {
    let junk = [
        json!({}),
        json!(null),
        json!({"regions": [-1, 1e99, "x", null, 0.5, 2]}),
        json!({"points": [[f64::MAX, 1], ["NaN", "x"], [1], 5, null]}),
        json!({"ids": ["x", -1, 99999], "regions": [0]}),
        json!({"mode": 7}),
    ];
    for cmd in ["shapeBuilder.regions", "shapeBuilder.merge", "shapeBuilder.remove", "shapeBuilder.mode"] {
        for j in &junk {
            let mut s = Session::new();
            squares(&mut s);
            if let Err(EngineError::Internal { cmd, msg }) = s.execute(cmd, j) {
                panic!("{cmd} panicked on {j}: {msg}");
            }
        }
    }
    // Degenerate geometry: a shape scaled to nothing, a huge one and a self-overlapping vector.
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 0.0, 10.0, "#ff0000");
    let b = rect(&mut s, -1e8, -1e8, 2e8, 2e8, "#0000ff");
    let v = id(&ex(&mut s, "vector.create", json!({"points": [[0, 0], [20, 20], [20, 0], [0, 20]], "closed": true})));
    s.set_selection(vec![a, b, v]);
    let _ = s.execute("shapeBuilder.regions", &json!({}));
    for p in [[10.0, 5.0], [5.0, 10.0], [1e7, 1e7]] {
        if let Err(EngineError::Internal { msg, .. }) = s.execute("shapeBuilder.merge", &json!({"points": [p]})) {
            panic!("merge panicked: {msg}");
        }
    }
}
