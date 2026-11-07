//! The Scale tool (K): on-canvas handles scale everything by one factor about the anchor point
//! (Alt: the centre), one undo step per drag, and `scale_node` scales every size-like property.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{Handle, PointerEvent, PointerKind, ToolId, anchor_name, anchor_of, scale_factor};
use figcraft_doc::{Effect, EffectKind, NodeId, NodeKind};
use figcraft_geom::{Point, Rect};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&ex(s, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h}))["id"]).unwrap()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().world_bounds(id).unwrap()
}

fn assert_rect(r: Rect, want: [f64; 4]) {
    assert!(near(r.x0, want[0]) && near(r.y0, want[1]) && near(r.x1, want[2]) && near(r.y1, want[3]), "{r:?} != {want:?}");
}

/// Drag from `a` to `b` with the current tool (`alt` held throughout).
fn drag(s: &mut Session, a: (f64, f64), b: (f64, f64), alt: bool) {
    for (kind, (x, y)) in
        [(PointerKind::Down, a), (PointerKind::Move, ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)), (PointerKind::Move, b), (PointerKind::Up, b)]
    {
        let mut ev = PointerEvent::at(kind, x, y, 1.0);
        ev.alt = alt;
        s.pointer(&ev).unwrap();
    }
}

fn undo_len(s: &Session) -> usize {
    s.ds().unwrap().history.undo.len()
}

#[test]
fn anchors_round_trip() {
    for (i, name) in ["tl", "t", "tr", "l", "c", "r", "bl", "b", "br"].into_iter().enumerate() {
        let a = anchor_of(name).unwrap();
        assert_eq!(a, ((i % 3) as u8, (i / 3) as u8));
        assert_eq!(anchor_name(a), name);
    }
    assert_eq!(anchor_of(" BR "), Some((2, 2)));
    assert_eq!(anchor_of("middle"), None);
    assert_eq!(anchor_name((9, 9)), "br");
}

#[test]
fn the_pivot_is_the_anchor_unless_it_is_on_the_dragged_side() {
    let (w, h) = (100.0, 50.0);
    let tl = (0, 0);
    // Bottom-right corner from the top-left anchor: classic, by the axis that moved the most.
    assert_eq!(scale_factor(Handle::Corner(2), Point::new(200.0, 60.0), w, h, tl, false), Some((2.0, Point::new(0.0, 0.0))));
    // Top-right corner: the anchor is on its top side, so the opposite corner stays put.
    assert_eq!(scale_factor(Handle::Corner(1), Point::new(150.0, -25.0), w, h, tl, false), Some((1.5, Point::new(0.0, 50.0))));
    // Right edge: from the anchor; left edge: the anchor is on it, so from the right edge.
    assert_eq!(scale_factor(Handle::Edge(1), Point::new(50.0, 999.0), w, h, tl, false), Some((0.5, Point::new(0.0, 0.0))));
    assert_eq!(scale_factor(Handle::Edge(3), Point::new(-100.0, 0.0), w, h, tl, false), Some((2.0, Point::new(100.0, 25.0))));
    // A centre anchor, or Alt, scales about the centre.
    assert_eq!(scale_factor(Handle::Corner(2), Point::new(150.0, 75.0), w, h, (1, 1), false), Some((2.0, Point::new(50.0, 25.0))));
    assert_eq!(scale_factor(Handle::Edge(2), Point::new(0.0, 75.0), w, h, tl, true), Some((2.0, Point::new(50.0, 25.0))));
    // Past the pivot flips; it never collapses to nothing.
    assert_eq!(scale_factor(Handle::Corner(2), Point::new(-100.0, -50.0), w, h, tl, false).map(|k| k.0), Some(-1.0));
    assert_eq!(scale_factor(Handle::Corner(2), Point::new(0.0, 0.0), w, h, tl, false).map(|k| k.0), Some(0.01));
    // Lines (no height) scale along their length.
    assert_eq!(scale_factor(Handle::Corner(1), Point::new(300.0, 7.0), 100.0, 0.0, tl, false).map(|k| k.0), Some(3.0));
    for bad in [f64::NAN, f64::INFINITY] {
        assert_eq!(scale_factor(Handle::Corner(2), Point::new(bad, 0.0), w, h, tl, false), None);
    }
    assert_eq!(scale_factor(Handle::Rotate(0), Point::new(1.0, 1.0), w, h, tl, false), None);
    assert_eq!(scale_factor(Handle::Corner(2), Point::new(5.0, 5.0), 0.0, 0.0, tl, false), None);
}

#[test]
fn dragging_a_handle_scales_strokes_effects_and_radii_in_one_undo_step() {
    let mut s = Session::new();
    let r = rect(&mut s, 0.0, 0.0, 100.0, 50.0);
    ex(&mut s, "node.set", json!({"id": r, "props": {"strokes": ["#000000"], "strokeWeight": 2, "cornerRadius": 8}}));
    s.doc_mut().unwrap().get_mut(r).unwrap().effects = vec![Effect::drop_shadow()];
    ex(&mut s, "select.set", json!({"ids": [r]}));
    ex(&mut s, "tool.scale", json!({}));
    let before = undo_len(&s);
    drag(&mut s, (100.0, 50.0), (200.0, 100.0), false);
    assert_rect(bounds(&s, r), [0.0, 0.0, 200.0, 100.0]);
    let n = s.doc().unwrap().node(r).unwrap().clone();
    assert!(near(n.stroke.weight, 4.0) && near(n.corners.radii[0], 16.0), "{:?} {:?}", n.stroke.weight, n.corners.radii);
    let EffectKind::DropShadow(sh) = &n.effects[0].kind else { panic!() };
    assert!(near(sh.radius, 8.0) && near(sh.offset.1, 8.0));
    assert_eq!(undo_len(&s), before + 1, "one step per drag");
    assert!(s.tool.overlay.scale.is_none(), "the readout goes with the drag");
    s.undo().unwrap();
    assert_rect(bounds(&s, r), [0.0, 0.0, 100.0, 50.0]);
    assert!(near(s.doc().unwrap().node(r).unwrap().stroke.weight, 2.0));
}

#[test]
fn the_live_readout_shows_the_factor_and_pivot() {
    let mut s = Session::new();
    let r = rect(&mut s, 10.0, 10.0, 100.0, 100.0);
    ex(&mut s, "select.set", json!({"ids": [r]}));
    ex(&mut s, "tool.set", json!({"tool": "scale"}));
    s.pointer(&PointerEvent::at(PointerKind::Down, 110.0, 110.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, 160.0, 160.0, 1.0)).unwrap();
    let hint = s.tool.overlay.scale.unwrap();
    assert!(near(hint.factor, 1.5) && near(hint.pivot[0], 10.0) && near(hint.pivot[1], 10.0), "{hint:?}");
    s.pointer(&PointerEvent::at(PointerKind::Up, 160.0, 160.0, 1.0)).unwrap();
}

#[test]
fn the_anchor_and_alt_choose_what_stays_put() {
    let mut s = Session::new();
    let r = rect(&mut s, 0.0, 0.0, 100.0, 50.0);
    ex(&mut s, "select.set", json!({"ids": [r]}));
    ex(&mut s, "tool.scale", json!({}));
    assert_eq!(ex(&mut s, "tool.scaleAnchor", json!({"anchor": "c"}))["anchor"], json!("c"));
    assert_eq!(ex(&mut s, "tool.scaleAnchor", json!({}))["anchor"], json!("c"), "reads it back");
    assert!(s.execute("tool.scaleAnchor", &json!({"anchor": "nowhere"})).is_err());
    drag(&mut s, (100.0, 50.0), (150.0, 75.0), false);
    assert_rect(bounds(&s, r), [-50.0, -25.0, 150.0, 75.0]);
    // Alt scales from the centre whatever the anchor.
    ex(&mut s, "tool.scaleAnchor", json!({"anchor": "tl"}));
    drag(&mut s, (150.0, 75.0), (125.0, 62.5), true);
    assert_rect(bounds(&s, r), [-25.0, -12.5, 125.0, 62.5]);
}

#[test]
fn frames_scale_their_children_whatever_their_constraints() {
    let mut s = Session::new();
    let f = id_of(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100}))["id"]).unwrap();
    let c = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 150, "y": 60, "width": 40, "height": 30, "parent": f}))["id"]).unwrap();
    ex(&mut s, "node.set", json!({"id": c, "props": {"constraints": {"horizontal": "right", "vertical": "bottom"}}}));
    let t = id_of(&ex(&mut s, "text.create", json!({"text": "Hi", "x": 10, "y": 10, "parent": f}))["id"]).unwrap();
    ex(&mut s, "node.set", json!({"id": t, "props": {"paragraphIndent": 4}}));
    let size = |s: &Session| match &s.doc().unwrap().node(t).unwrap().kind {
        NodeKind::Text(td) => (td.style.font_size, td.paragraph_indent),
        _ => (0.0, 0.0),
    };
    let (fs, _) = size(&s);
    ex(&mut s, "select.set", json!({"ids": [f]}));
    ex(&mut s, "tool.scale", json!({}));
    drag(&mut s, (200.0, 100.0), (300.0, 150.0), false);
    assert_rect(bounds(&s, f), [0.0, 0.0, 300.0, 150.0]);
    assert_rect(bounds(&s, c), [225.0, 90.0, 285.0, 135.0]);
    let (fs2, indent) = size(&s);
    assert!(near(fs2, fs * 1.5) && near(indent, 6.0), "{fs2} {indent}");
}

#[test]
fn several_layers_scale_together_about_the_pivot() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 50.0, 50.0);
    let b = rect(&mut s, 100.0, 50.0, 50.0, 50.0);
    ex(&mut s, "node.set", json!({"ids": [a, b], "props": {"strokes": ["#000000"], "strokeWeight": 1}}));
    ex(&mut s, "select.set", json!({"ids": [a, b]}));
    ex(&mut s, "tool.scale", json!({}));
    let before = undo_len(&s);
    drag(&mut s, (150.0, 100.0), (300.0, 200.0), false);
    assert_rect(bounds(&s, a), [0.0, 0.0, 100.0, 100.0]);
    assert_rect(bounds(&s, b), [200.0, 100.0, 300.0, 200.0]);
    assert!(near(s.doc().unwrap().node(b).unwrap().stroke.weight, 2.0));
    assert_eq!(undo_len(&s), before + 1);
}

#[test]
fn rotated_layers_and_lines_scale_along_their_own_axes() {
    let mut s = Session::new();
    let r = rect(&mut s, 0.0, 0.0, 100.0, 50.0);
    ex(&mut s, "node.set", json!({"id": r, "props": {"rotation": 30}}));
    ex(&mut s, "select.set", json!({"ids": [r]}));
    ex(&mut s, "tool.scale", json!({}));
    let w = s.doc().unwrap().world_transform(r);
    let (origin, from, to) = (w * Point::ZERO, w * Point::new(100.0, 50.0), w * Point::new(200.0, 100.0));
    drag(&mut s, (from.x, from.y), (to.x, to.y), false);
    let doc = s.doc().unwrap();
    let n = doc.node(r).unwrap();
    assert!(near(n.width, 200.0) && near(n.height, 100.0));
    let o = doc.world_transform(r) * Point::ZERO;
    assert!(near(o.x, origin.x) && near(o.y, origin.y), "the anchor corner stays: {o:?} {origin:?}");
    let c = doc.world_transform(r).as_coeffs();
    assert!(near(c[1].atan2(c[0]).to_degrees().abs(), 30.0), "{c:?}");
    // A line scales its length and stroke.
    let l = id_of(&ex(&mut s, "shape.line", json!({"x": 0, "y": 300, "width": 100}))["id"]).unwrap();
    let wt = s.doc().unwrap().node(l).unwrap().stroke.weight;
    ex(&mut s, "select.set", json!({"ids": [l]}));
    drag(&mut s, (100.0, 300.0), (200.0, 300.0), false);
    let n = s.doc().unwrap().node(l).unwrap();
    assert!(near(n.width, 200.0) && near(n.stroke.weight, wt * 2.0), "{} {}", n.width, n.stroke.weight);
}

#[test]
fn dragging_past_the_pivot_turns_the_layer_over() {
    let mut s = Session::new();
    let r = rect(&mut s, 0.0, 0.0, 100.0, 50.0);
    ex(&mut s, "select.set", json!({"ids": [r]}));
    ex(&mut s, "tool.scale", json!({}));
    drag(&mut s, (100.0, 50.0), (-100.0, -50.0), false);
    assert_rect(bounds(&s, r), [-100.0, -50.0, 0.0, 0.0]);
    let n = s.doc().unwrap().node(r).unwrap();
    assert!(near(n.width, 100.0) && near(n.height, 50.0));
}

#[test]
fn scale_node_scales_every_size_like_property() {
    let mut s = Session::new();
    let f = id_of(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100}))["id"]).unwrap();
    ex(
        &mut s,
        "node.set",
        json!({"id": f, "props": {"minWidth": 50, "maxWidth": 400, "grids": [{"kind": "columns", "count": 4, "gutter": 20, "offset": 10, "size": 30, "align": "min"}]}}),
    );
    ex(&mut s, "guide.add", json!({"frame": f, "vertical": true, "offset": 40}));
    ex(&mut s, "transform.scale", json!({"ids": [f], "factor": 2}));
    let n = s.doc().unwrap().node(f).unwrap();
    assert_eq!((n.child_layout.min_w, n.child_layout.max_w), (Some(100.0), Some(800.0)));
    let g = &n.grids[0];
    assert_eq!((g.gutter, g.offset, g.size, g.count), (40.0, 20.0, 60.0, 4));
    assert_eq!(n.guides[0].offset, 80.0);
    assert_eq!(n.width, 400.0);
}

#[test]
fn the_scale_tool_only_acts_on_handles() {
    // Clicking a layer with K selects and moves it like the Move tool.
    let mut s = Session::new();
    let r = rect(&mut s, 0.0, 0.0, 100.0, 50.0);
    s.set_tool(ToolId::Scale);
    drag(&mut s, (50.0, 25.0), (60.0, 35.0), false);
    assert_eq!(s.selection(), vec![r]);
    assert_rect(bounds(&s, r), [10.0, 10.0, 110.0, 60.0]);
    assert!(near(s.doc().unwrap().node(r).unwrap().width, 100.0));
}

#[test]
fn scale_node_scales_vector_point_corner_radius() {
    let mut s = Session::new();
    let v = id_of(&ex(&mut s, "vector.create", json!({"points": [[0, 0], [100, 0], [100, 100], [0, 100]], "closed": true}))["id"]).unwrap();
    ex(&mut s, "node.set", json!({"id": v, "props": {"cornerRadius": 10}}));
    ex(&mut s, "transform.scale", json!({"ids": [v], "factor": 2}));
    let d = s.doc().unwrap();
    let NodeKind::Vector { network } = &d.node(v).unwrap().kind else { panic!("not a vector") };
    assert!(!network.vertices.is_empty());
    for vx in &network.vertices {
        assert_eq!(vx.radius, 20.0, "corner radius scaled with the layer");
    }
}
