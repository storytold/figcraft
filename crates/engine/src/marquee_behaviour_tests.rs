//! Marquee selection, as measured in Figma's web app (2026-11-11, the "marquee rules" and "confirming
//! pass" readings, and the two evening re-measures) and described in the help-center article "Select layers and objects": a drag
//! selects each top-level layer it touches; a partly covered frame or section gives the layers in it
//! the marquee touches, an enclosed one is selected itself; a drag started inside a frame selects that frame
//! only when the marquee also takes another top-level frame (then whole); selecting a frame whole keeps the marquee at the top level; a rotated shape is touched by
//! its outline, an unrotated one by its box.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind};
use figcraft_doc::NodeId;
use figcraft_geom::Point;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": w, "height": h});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id(&ex(s, "shape.rectangle", p))
}

fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
    PointerEvent::at(kind, x, y, 1.0)
}

fn drag(s: &mut Session, a: (f64, f64), b: (f64, f64)) {
    s.pointer(&ev(PointerKind::Down, a.0, a.1)).unwrap();
    s.pointer(&ev(PointerKind::Move, b.0, b.1)).unwrap();
    s.pointer(&ev(PointerKind::Up, b.0, b.1)).unwrap();
}

fn sorted(mut v: Vec<NodeId>) -> Vec<NodeId> {
    v.sort();
    v
}

#[allow(dead_code)]
struct Scene {
    f1: NodeId,
    r1: NodeId,
    r2: NodeId,
    e: NodeId,
    u: NodeId,
    rr: NodeId,
    re: NodeId,
    f2: NodeId,
    g: NodeId,
    s: NodeId,
    r5: NodeId,
}

/// The confirming pass's layout: `f1` (0,0 300x200) holding `r1` (20,20) and `r2` (220,120), both 60x60;
/// ellipse `e` (500,0 200x200); rectangle `u` (900,0 100x100); `rr` rectangle (0,400 200x100) and ellipse
/// `re` (500,400 200x100) turned 45 degrees about their centres; the empty frame `f2` (0,700 200x150);
/// the group `g` of two rectangles (300,700) and (360,760); the section `s` (600,700 300x200) holding `r5`.
fn scene(s: &mut Session) -> Scene {
    let f1 = id(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 200})));
    let r1 = rect(s, 20.0, 20.0, 60.0, 60.0, Some(f1));
    let r2 = rect(s, 220.0, 120.0, 60.0, 60.0, Some(f1));
    let e = id(&ex(s, "shape.ellipse", json!({"x": 500, "y": 0, "width": 200, "height": 200})));
    let u = rect(s, 900.0, 0.0, 100.0, 100.0, None);
    let rr = rect(s, 0.0, 400.0, 200.0, 100.0, None);
    ex(s, "node.set", json!({"id": rr, "props": {"rotation": 45}}));
    let re = id(&ex(s, "shape.ellipse", json!({"x": 500, "y": 400, "width": 200, "height": 100})));
    ex(s, "node.set", json!({"id": re, "props": {"rotation": 45}}));
    let f2 = id(&ex(s, "frame.create", json!({"x": 0, "y": 700, "width": 200, "height": 150})));
    let g1 = rect(s, 300.0, 700.0, 80.0, 80.0, None);
    let g2 = rect(s, 360.0, 760.0, 80.0, 80.0, None);
    let g = id(&ex(s, "object.group", json!({"ids": [g1, g2]})));
    let sec = id(&ex(s, "node.create", json!({"type": "section", "x": 600, "y": 700, "width": 300, "height": 200})));
    let r5 = rect(s, 20.0, 20.0, 60.0, 60.0, Some(sec));
    ex(s, "select.none", json!({}));
    Scene { f1, r1, r2, e, u, rr, re, f2, g, s: sec, r5 }
}

fn selected(s: &mut Session, a: (f64, f64), b: (f64, f64)) -> Vec<NodeId> {
    ex(s, "select.none", json!({}));
    drag(s, a, b);
    let sel = sorted(s.selection());
    ex(s, "select.none", json!({}));
    sel
}

#[test]
fn a_partly_covered_frame_gives_the_layers_it_touches() {
    let mut s = Session::new();
    let k = scene(&mut s);
    assert_eq!(selected(&mut s, (-50.0, -50.0), (60.0, 60.0)), vec![k.r1], "measured case 2: Rectangle 1, not F1");
}

#[test]
fn an_enclosed_frame_is_selected_itself() {
    let mut s = Session::new();
    let k = scene(&mut s);
    assert_eq!(selected(&mut s, (-20.0, -20.0), (320.0, 220.0)), vec![k.f1], "measured b");
}

#[test]
fn an_empty_frame_is_selected_on_touch() {
    let mut s = Session::new();
    let k = scene(&mut s);
    assert_eq!(selected(&mut s, (-20.0, 680.0), (50.0, 720.0)), vec![k.f2], "measured c: Frame 2");
}

#[test]
fn a_group_is_selected_whole_on_touch() {
    let mut s = Session::new();
    let k = scene(&mut s);
    assert_eq!(selected(&mut s, (290.0, 690.0), (310.0, 710.0)), vec![k.g], "measured c: Group 1, not the rectangle");
}

#[test]
fn a_partly_covered_section_gives_the_layers_it_touches() {
    let mut s = Session::new();
    let k = scene(&mut s);
    assert_eq!(selected(&mut s, (590.0, 690.0), (650.0, 750.0)), vec![k.r5], "measured c: Rectangle 5, not the section");
}

#[test]
fn an_unrotated_shape_is_touched_by_its_box() {
    let mut s = Session::new();
    let k = scene(&mut s);
    assert_eq!(selected(&mut s, (490.0, -10.0), (505.0, 5.0)), vec![k.e], "measured case 3: the ellipse's box corner selects it");
    assert_eq!(selected(&mut s, (890.0, -10.0), (905.0, 5.0)), vec![k.u], "control");
}

#[test]
fn a_rotated_rectangle_is_touched_by_its_outline() {
    let mut s = Session::new();
    let k = scene(&mut s);
    assert_eq!(selected(&mut s, (-30.0, 320.0), (4.0, 354.0)), Vec::<NodeId>::new(), "measured case 3: the box corner selects nothing");
    assert_eq!(selected(&mut s, (-30.0, 320.0), (100.0, 450.0)), vec![k.rr]);
}

#[test]
fn a_rotated_ellipse_is_not_touched_by_its_axis_aligned_box() {
    let mut s = Session::new();
    let k = scene(&mut s);
    assert_eq!(selected(&mut s, (470.0, 320.0), (504.0, 354.0)), Vec::<NodeId>::new(), "measured case 3: the box corner selects nothing");
    assert_eq!(selected(&mut s, (470.0, 320.0), (600.0, 450.0)), vec![k.re]);
}

#[test]
fn a_rotated_ellipse_is_not_touched_through_the_corner_of_its_turned_box() {
    let mut s = Session::new();
    let k = scene(&mut s);
    // The re-measure's literal marquees (evening, section 1): corner A, the deeper B, and the control.
    let p = Point::new(692.0, 419.0);
    let doc = s.doc().unwrap();
    let n = doc.node(k.re).unwrap();
    assert!(n.local_rect().contains(doc.world_transform(k.re).inverse() * p), "the probe is inside the turned box");
    assert_eq!(selected(&mut s, (715.0, 411.0), (692.0, 419.0)), Vec::<NodeId>::new(), "measured marquee A");
    assert_eq!(selected(&mut s, (715.0, 411.0), (683.0, 419.0)), Vec::<NodeId>::new(), "measured marquee B");
    assert_eq!(selected(&mut s, (715.0, 411.0), (667.0, 419.0)), vec![k.re], "measured control");
}

#[test]
fn a_drag_from_inside_a_frame_selects_only_what_it_touches() {
    let mut s = Session::new();
    let k = scene(&mut s);
    let start_x = |s: &Session| s.doc().unwrap().world_bounds(k.f1).map(|b| (b.x0, b.y0));
    assert_eq!(selected(&mut s, (150.0, 100.0), (180.0, 150.0)), Vec::<NodeId>::new(), "measured e1");
    assert_eq!(start_x(&s), Some((0.0, 0.0)));
    assert_eq!(selected(&mut s, (150.0, 100.0), (180.0, 250.0)), Vec::<NodeId>::new(), "measured e2");
    assert_eq!(start_x(&s), Some((0.0, 0.0)));
    assert_eq!(selected(&mut s, (150.0, 100.0), (600.0, 100.0)), vec![k.e], "measured e3: the ellipse only");
    assert_eq!(start_x(&s), Some((0.0, 0.0)));
    assert_eq!(selected(&mut s, (150.0, 100.0), (250.0, 150.0)), vec![k.r2], "today's rule, not measured");
    assert_eq!(start_x(&s), Some((0.0, 0.0)));
}

#[test]
fn selecting_a_frame_whole_keeps_the_marquee_at_the_top_level() {
    let mut s = Session::new();
    let k = scene(&mut s);
    // Encloses the empty f2, covers rr, and partly covers f1 over r1: no frame opens, f1 comes whole.
    assert_eq!(
        selected(&mut s, (-20.0, 50.0), (210.0, 860.0)),
        sorted(vec![k.f2, k.f1, k.rr]),
        "the article: selecting a top-level frame selects only top-level layers; measured: a partly covered frame comes whole"
    );
}

#[test]
fn a_marquee_enclosing_an_empty_frame_takes_a_partly_covered_frame_whole() {
    let mut s = Session::new();
    let f1 = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 200})));
    let _r1 = rect(&mut s, 20.0, 20.0, 60.0, 60.0, Some(f1));
    let f2 = id(&ex(&mut s, "frame.create", json!({"x": 400, "y": 0, "width": 200, "height": 150})));
    assert_eq!(selected(&mut s, (650.0, 170.0), (40.0, -10.0)), sorted(vec![f1, f2]), "measured re-measure: F2 and F1, not R1");
}

#[test]
fn a_drag_from_inside_a_frame_takes_it_whole_when_it_takes_another_top_level_frame() {
    let mut s = Session::new();
    let f1 = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 200})));
    let r1 = rect(&mut s, 20.0, 20.0, 60.0, 60.0, Some(f1));
    let _r2 = rect(&mut s, 220.0, 120.0, 60.0, 60.0, Some(f1));
    let f2 = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 400, "width": 200, "height": 150})));
    let rr = rect(&mut s, 400.0, 400.0, 200.0, 100.0, None);
    ex(&mut s, "select.none", json!({}));
    assert_eq!(selected(&mut s, (150.0, 100.0), (90.0, 720.0)), sorted(vec![f1, f2]), "re-measure 2, case 1");
    let sel = selected(&mut s, (150.0, 50.0), (50.0, 720.0));
    assert_eq!(sel, sorted(vec![f1, f2]), "re-measure 2, case 2b: R1 is covered in part and still not selected");
    assert!(!sel.contains(&r1));
    assert_eq!(selected(&mut s, (150.0, 100.0), (450.0, 450.0)), sorted(vec![f1, f2, rr]), "re-measure 2, case 3");
    assert_eq!(selected(&mut s, (290.0, 100.0), (450.0, 450.0)), vec![rr], "re-measure 2, case 3b: the frame only when another is taken");
}
