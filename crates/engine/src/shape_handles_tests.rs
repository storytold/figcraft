//! On-canvas corner radius and star ratio handles (see `shape_handles.rs`).

use figcraft_doc::{NodeId, NodeKind};
use figcraft_geom::Point;
use serde_json::{Value, json};
use std::f64::consts::TAU;

use crate::Session;
use crate::params::id_of;
use crate::shape_handles::{ShapeHandle, handles, hit};
use crate::tools::{PointerEvent, PointerKind};

fn create(s: &mut Session, cmd: &str, p: Value) -> NodeId {
    let id = id_of(&s.execute(cmd, &p).unwrap()["id"]).unwrap();
    s.set_selection(vec![id]);
    id
}

fn drag(s: &mut Session, a: (f64, f64), b: (f64, f64), alt: bool) {
    for (kind, p) in [(PointerKind::Down, a), (PointerKind::Move, b), (PointerKind::Up, b)] {
        let mut e = PointerEvent::at(kind, p.0, p.1, 1.0);
        e.alt = alt;
        s.pointer(&e).unwrap();
    }
}

fn radii(s: &Session, id: NodeId) -> [f64; 4] {
    s.doc().unwrap().node(id).unwrap().corners.radii
}

#[test]
fn rectangle_corner_handles_round_the_corners() {
    let mut s = Session::new();
    let id = create(&mut s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 200, "height": 120}));
    let doc = s.doc().unwrap().clone();
    let hs = handles(&doc, &[id], 1.0);
    assert_eq!(hs.len(), 4, "one handle inside each corner");
    let at = |h: [f64; 2]| Point::new(h[0], h[1]);
    assert_eq!(hs[0].0, ShapeHandle::Radius { corner: 0 });
    assert!(((at(hs[0].1) - Point::new(100.0, 100.0)).hypot() - 10.0).abs() < 1e-9, "at radius 0 a handle sits a little inside its corner");
    assert!(((at(hs[2].1) - Point::new(300.0, 220.0)).hypot() - 10.0).abs() < 1e-9, "the bottom right one too");
    // Dragging the top-left handle along the diagonal rounds every corner, in one undo step.
    drag(&mut s, (hs[0].1[0], hs[0].1[1]), (130.0, 128.0), false);
    let r = radii(&s, id);
    assert!(r.iter().all(|x| (x - 29.0).abs() < 1e-6), "the radius follows the drag along the corner's diagonal: {r:?}");
    assert_eq!(handles(s.doc().unwrap(), &[id], 1.0)[0].1, [129.0, 129.0], "the handle sits at the centre of the corner's arc");
    s.undo().unwrap();
    assert_eq!(radii(&s, id), [0.0; 4], "one undo step");
    // Past the middle it stops at half the short side.
    drag(&mut s, (hs[0].1[0], hs[0].1[1]), (400.0, 400.0), false);
    assert_eq!(radii(&s, id), [60.0; 4], "a radius stops at half the short side");
    // Alt rounds only the corner dragged.
    s.undo().unwrap();
    let br = handles(s.doc().unwrap(), &[id], 1.0)[2].1;
    drag(&mut s, (br[0], br[1]), (290.0, 210.0), true);
    let r = radii(&s, id);
    assert!(r.iter().zip([0.0, 0.0, 10.0, 0.0]).all(|(a, b)| (a - b).abs() < 1e-6), "silent: Alt changes one corner (pinned): {r:?}");
}

#[test]
fn polygon_and_star_handles() {
    let mut s = Session::new();
    let star = create(&mut s, "shape.star", json!({"x": 0, "y": 0, "width": 200, "height": 200}));
    let doc = s.doc().unwrap().clone();
    let hs = handles(&doc, &[star], 1.0);
    let kinds: Vec<ShapeHandle> = hs.iter().map(|h| h.0).collect();
    assert_eq!(
        kinds,
        vec![ShapeHandle::Radius { corner: 0 }, ShapeHandle::Ratio, ShapeHandle::Count],
        "a star has a radius, a ratio and a count handle"
    );
    // The ratio handle sits on the first inner point; dragging it to the centre's side shrinks it.
    let ratio_at = hs[1].1;
    let inner = figcraft_geom::star_points(200.0, 200.0, 5, 0.38)[1];
    assert!((Point::new(ratio_at[0], ratio_at[1]) - inner).hypot() < 1e-6);
    let towards = Point::new(100.0, 100.0) + (inner - Point::new(100.0, 100.0)) * (0.5 / 0.38);
    drag(&mut s, (ratio_at[0], ratio_at[1]), (towards.x, towards.y), false);
    let NodeKind::Star { inner_ratio, .. } = s.doc().unwrap().node(star).unwrap().kind else { panic!("star") };
    assert!((inner_ratio - 0.5).abs() < 1e-6, "the ratio follows the drag: {inner_ratio}");
    // The radius handle rounds the points.
    let top = handles(s.doc().unwrap(), &[star], 1.0)[0].1;
    drag(&mut s, (top[0], top[1]), (top[0], top[1] + 30.0), false);
    assert!(radii(&s, star)[0] > 1.0, "the star's points round");
    let mut s = Session::new();
    let poly = create(&mut s, "shape.polygon", json!({"x": 0, "y": 0, "width": 200, "height": 200}));
    assert_eq!(handles(s.doc().unwrap(), &[poly], 1.0).len(), 2, "a polygon has a radius and a count handle");
}

#[test]
fn handles_show_only_where_they_can_be_used() {
    let mut s = Session::new();
    let small = create(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 40}));
    assert!(handles(s.doc().unwrap(), &[small], 1.0).is_empty(), "too small on screen");
    assert_eq!(handles(s.doc().unwrap(), &[small], 4.0).len(), 4, "zoomed in, they show");
    let big = create(&mut s, "shape.rectangle", json!({"x": 100, "y": 0, "width": 100, "height": 100}));
    assert!(handles(s.doc().unwrap(), &[small, big], 1.0).is_empty(), "only for a single selected layer");
    let e = create(&mut s, "shape.ellipse", json!({"x": 300, "y": 0, "width": 100, "height": 100}));
    assert!(handles(s.doc().unwrap(), &[e], 1.0).is_empty(), "ellipses have arc handles instead");
    s.execute("object.toggleLock", &json!({"ids": [big], "locked": true})).unwrap();
    assert!(handles(s.doc().unwrap(), &[big], 1.0).is_empty(), "locked layers have none");
    assert!(hit(s.doc().unwrap(), &[small], Point::new(f64::NAN, 0.0), 4.0).is_none());
    // In vector edit mode the Move tool shows none.
    let r = create(&mut s, "shape.rectangle", json!({"x": 0, "y": 200, "width": 100, "height": 100}));
    assert_eq!(s.shape_handles().len(), 4);
    s.execute("vector.edit.begin", &json!({"id": r})).unwrap();
    assert!(s.shape_handles().is_empty(), "none while editing points");
}

fn count_of(s: &Session, id: NodeId) -> u32 {
    match s.doc().unwrap().node(id).unwrap().kind {
        NodeKind::Polygon { count } | NodeKind::Star { count, .. } => count,
        _ => panic!("a polygon or star"),
    }
}

fn count_handle(s: &Session, id: NodeId) -> [f64; 2] {
    handles(s.doc().unwrap(), &[id], 1.0).into_iter().find(|h| h.0 == ShapeHandle::Count).expect("a count handle").1
}

/// The page point at `deg` degrees clockwise from the shape's own top, `r` from its centre (200 x 200 box).
fn at_angle(s: &Session, id: NodeId, deg: f64, r: f64) -> (f64, f64) {
    let a = deg.to_radians();
    let p = s.doc().unwrap().world_transform(id) * Point::new(100.0 + r * a.sin(), 100.0 - r * a.cos());
    (p.x, p.y)
}

fn count_drag(s: &mut Session, id: NodeId, deg: f64) {
    let h = count_handle(s, id);
    let to = at_angle(s, id, deg, 80.0);
    drag(s, (h[0], h[1]), to, false);
}

#[test]
fn a_polygon_has_a_count_handle_on_its_first_vertex_clockwise_from_the_top() {
    let mut s = Session::new();
    let poly = create(&mut s, "shape.polygon", json!({"x": 0, "y": 0, "width": 200, "height": 200}));
    s.execute("node.set", &json!({"ids": [poly], "props": {"pointCount": 5}})).unwrap();
    let h = count_handle(&s, poly);
    let v1 = figcraft_geom::polygon_points(200.0, 200.0, 5)[1];
    assert!((Point::new(h[0], h[1]) - v1).hypot() < 1e-6, "on vertex 1, not inset: {h:?} vs {v1:?}");
    assert!((h[0] - (100.0 + 100.0 * (TAU / 5.0).sin())).abs() < 1e-6, "72 degrees from the top");
    // Hovering it finds it, not the radius handle.
    assert_eq!(hit(s.doc().unwrap(), &[poly], Point::new(h[0], h[1]), 1.0).map(|x| x.1), Some(ShapeHandle::Count));
}

#[test]
fn dragging_the_count_handle_sets_the_count_by_the_angle_about_the_centre() {
    let mut s = Session::new();
    let poly = create(&mut s, "shape.polygon", json!({"x": 0, "y": 0, "width": 200, "height": 200}));
    s.execute("node.set", &json!({"ids": [poly], "props": {"pointCount": 5}})).unwrap();
    for (deg, want) in [(90.0, 4), (45.0, 8), (120.0, 3), (0.5, 60), (0.0, 60), (200.0, 3)] {
        count_drag(&mut s, poly, deg);
        assert_eq!(count_of(&s, poly), want, "{deg} degrees");
    }
    // The radius from the centre doesn't matter.
    let h = count_handle(&s, poly);
    let to = at_angle(&s, poly, 90.0, 30.0);
    drag(&mut s, (h[0], h[1]), to, false);
    assert_eq!(count_of(&s, poly), 4, "90 degrees near the centre");
    // The readout is the count.
    let doc = s.doc().unwrap();
    assert_eq!(crate::shape_handles::value(doc, poly, ShapeHandle::Count), Some(4.0));
    // A pointer on the centre has no direction: it gives the most points.
    let h = count_handle(&s, poly);
    drag(&mut s, (h[0], h[1]), (100.0, 100.0), false);
    assert_eq!(count_of(&s, poly), 60, "a pointer on the centre");
}

#[test]
fn dragging_a_stars_count_handle_sets_its_points() {
    let mut s = Session::new();
    let star = create(&mut s, "shape.star", json!({"x": 0, "y": 0, "width": 200, "height": 200}));
    let h = count_handle(&s, star);
    let outer = figcraft_geom::star_points(200.0, 200.0, 5, 0.38)[2];
    assert!((Point::new(h[0], h[1]) - outer).hypot() < 1e-6, "on its first outer point clockwise from the top");
    for (deg, want) in [(90.0, 4), (45.0, 8), (130.0, 3), (1.0, 60)] {
        count_drag(&mut s, star, deg);
        assert_eq!(count_of(&s, star), want, "{deg} degrees");
    }
    let NodeKind::Star { inner_ratio, .. } = s.doc().unwrap().node(star).unwrap().kind else { panic!("star") };
    assert!((inner_ratio - 0.38).abs() < 0.02, "the ratio stays through count drags: {inner_ratio}");
}

#[test]
fn a_count_drag_is_one_undo_step() {
    let mut s = Session::new();
    let poly = create(&mut s, "shape.polygon", json!({"x": 0, "y": 0, "width": 200, "height": 200}));
    s.execute("node.set", &json!({"ids": [poly], "props": {"pointCount": 5}})).unwrap();
    count_drag(&mut s, poly, 45.0);
    assert_eq!(count_of(&s, poly), 8);
    s.undo().unwrap();
    assert_eq!(count_of(&s, poly), 5, "one Ctrl+Z takes the drag back");
}

#[test]
fn a_rotated_polygons_count_angle_is_measured_in_its_own_frame() {
    let mut s = Session::new();
    let poly = create(&mut s, "shape.polygon", json!({"x": 0, "y": 0, "width": 200, "height": 200}));
    s.execute("node.set", &json!({"ids": [poly], "props": {"pointCount": 5, "rotation": 30}})).unwrap();
    for (deg, want) in [(90.0, 4), (45.0, 8)] {
        count_drag(&mut s, poly, deg);
        assert_eq!(count_of(&s, poly), want, "{deg} degrees in the shape's frame");
    }
}
