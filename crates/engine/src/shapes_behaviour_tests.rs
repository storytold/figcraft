//! Shapes behaviour suite, written from the public documentation of design tools: drawing with
//! Shift and Alt, corner radius rules, point counts and star ratios, arcs, stroke joins and caps,
//! boolean results and the look they take, flatten and outline stroke. Each rule is a table row
//! with what it implies and a one-line derivation; where the documentation is silent the row says
//! so and pins today's behaviour.

// The tables read best as tuples.
#![allow(clippy::type_complexity)]

use figcraft_doc::{BoolOp, Node, NodeId, NodeKind, StrokeCap, StrokeJoin, StrokeProps};
use figcraft_geom::{BezPath, Point, Shape};
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind, ToolId};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn create(s: &mut Session, cmd: &str, p: Value) -> NodeId {
    id_of(&ex(s, cmd, p)["id"]).unwrap()
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().node(id).unwrap().clone()
}

fn ev(kind: PointerKind, p: (f64, f64), shift: bool, alt: bool) -> PointerEvent {
    let mut e = PointerEvent::at(kind, p.0, p.1, 1.0);
    (e.shift, e.alt) = (shift, alt);
    e
}

/// Draw with `tool` from `a` to `b` (a click when they're equal); the new layer.
fn draw(s: &mut Session, tool: ToolId, a: (f64, f64), b: (f64, f64), shift: bool, alt: bool) -> NodeId {
    s.set_tool(tool);
    s.pointer(&ev(PointerKind::Down, a, shift, alt)).unwrap();
    if a != b {
        s.pointer(&ev(PointerKind::Move, ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0), shift, alt)).unwrap();
        s.pointer(&ev(PointerKind::Move, b, shift, alt)).unwrap();
    }
    s.pointer(&ev(PointerKind::Up, b, shift, alt)).unwrap();
    *s.selection().first().unwrap()
}

/// A layer's box on the page: x, y, width, height (rotation ignored).
fn bx(n: &Node) -> (f64, f64, f64, f64) {
    let t = figcraft_geom::translation(n.transform);
    (t.x, t.y, n.width, n.height)
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn drawing_shapes_follows_the_modifier_rules() {
    use ToolId::*;
    // (tool, press, release, shift, alt, box x/y/w/h, derivation)
    let rows: Vec<(ToolId, (f64, f64), (f64, f64), bool, bool, (f64, f64, f64, f64), &str)> = vec![
        (Rectangle, (100.0, 100.0), (160.0, 140.0), false, false, (100.0, 100.0, 60.0, 40.0), "a drag draws from the press to the release"),
        (Rectangle, (100.0, 100.0), (60.0, 70.0), false, false, (60.0, 70.0, 40.0, 30.0), "dragging up and left draws the same box"),
        (Rectangle, (100.0, 100.0), (160.0, 140.0), true, false, (100.0, 100.0, 60.0, 60.0), "Shift draws a square (the longer side wins)"),
        (Rectangle, (100.0, 100.0), (160.0, 140.0), false, true, (40.0, 60.0, 120.0, 80.0), "Alt draws from the centre"),
        (Rectangle, (100.0, 100.0), (160.0, 140.0), true, true, (40.0, 40.0, 120.0, 120.0), "Shift and Alt together: a square from the centre"),
        (Ellipse, (100.0, 100.0), (130.0, 180.0), true, false, (100.0, 100.0, 80.0, 80.0), "Shift draws a circle"),
        (Polygon, (100.0, 100.0), (150.0, 120.0), true, false, (100.0, 100.0, 50.0, 50.0), "Shift draws a regular polygon"),
        (Star, (100.0, 100.0), (120.0, 130.0), false, true, (80.0, 70.0, 40.0, 60.0), "Alt draws a star from the centre"),
        (
            Rectangle,
            (100.0, 100.0),
            (100.0, 100.0),
            false,
            false,
            (100.0, 100.0, 100.0, 100.0),
            "silent: a click without a drag makes a 100 × 100 shape (pinned)",
        ),
    ];
    for (tool, a, b, shift, alt, want, why) in rows {
        let mut s = Session::new();
        let id = draw(&mut s, tool, a, b, shift, alt);
        let got = bx(&node(&s, id));
        assert!(near(got.0, want.0) && near(got.1, want.1) && near(got.2, want.2) && near(got.3, want.3), "{why}: {got:?} != {want:?}");
    }
}

#[test]
fn lines_and_arrows_follow_the_modifier_rules() {
    // (tool, press, release, shift, alt, start point, length, angle in degrees, derivation)
    let len = 60f64.hypot(10.0);
    let rows: Vec<(ToolId, (f64, f64), (f64, f64), bool, bool, (f64, f64), f64, f64, &str)> = vec![
        (
            ToolId::Line,
            (100.0, 100.0),
            (160.0, 110.0),
            false,
            false,
            (100.0, 100.0),
            len,
            10f64.atan2(60.0).to_degrees(),
            "a line runs from the press to the release",
        ),
        (ToolId::Line, (100.0, 100.0), (160.0, 110.0), true, false, (100.0, 100.0), len, 0.0, "Shift keeps a line to 45° steps"),
        (ToolId::Line, (100.0, 100.0), (150.0, 150.0), true, false, (100.0, 100.0), 50f64.hypot(50.0), 45.0, "a 45° drag with Shift stays at 45°"),
        (
            ToolId::Line,
            (100.0, 100.0),
            (160.0, 100.0),
            false,
            true,
            (40.0, 100.0),
            120.0,
            0.0,
            "Alt draws a line from its centre, like the other shapes",
        ),
        (ToolId::Arrow, (100.0, 100.0), (160.0, 100.0), false, false, (100.0, 100.0), 60.0, 0.0, "an arrow is drawn like a line"),
    ];
    for (tool, a, b, shift, alt, start, length, angle, why) in rows {
        let mut s = Session::new();
        let id = draw(&mut s, tool, a, b, shift, alt);
        let n = node(&s, id);
        let p = figcraft_geom::translation(n.transform);
        let rot = -figcraft_geom::rotation_degrees(n.transform);
        assert!(near(p.x, start.0) && near(p.y, start.1), "{why}: starts at {p:?}");
        assert!((n.width - length).abs() < 1e-6, "{why}: length {}", n.width);
        assert!((rot.rem_euclid(360.0) - angle.rem_euclid(360.0)).abs() < 1e-6 || (rot - angle).abs() < 1e-6, "{why}: angle {rot}");
    }
    // The arrow tool's line gets an arrowhead at its end, the line tool's none.
    let mut s = Session::new();
    let arrow = draw(&mut s, ToolId::Arrow, (0.0, 0.0), (80.0, 0.0), false, false);
    let line = draw(&mut s, ToolId::Line, (0.0, 50.0), (80.0, 50.0), false, false);
    assert_eq!(
        (node(&s, arrow).stroke.cap_start, node(&s, arrow).stroke.cap_end),
        (StrokeCap::None, StrokeCap::LineArrow),
        "an arrow's tip is at its end"
    );
    assert_eq!(node(&s, line).stroke.cap_end, StrokeCap::None, "a plain line has no arrowhead");
}

#[test]
fn new_shapes_have_the_documented_defaults() {
    let mut s = Session::new();
    let polygon = draw(&mut s, ToolId::Polygon, (0.0, 0.0), (90.0, 90.0), false, false);
    let star = draw(&mut s, ToolId::Star, (100.0, 0.0), (190.0, 90.0), false, false);
    let line = draw(&mut s, ToolId::Line, (0.0, 200.0), (90.0, 200.0), false, false);
    let rect = draw(&mut s, ToolId::Rectangle, (0.0, 300.0), (90.0, 390.0), false, false);
    let p = node(&s, polygon);
    let st = node(&s, star);
    assert!(matches!(p.kind, NodeKind::Polygon { count: 3 }), "a new polygon is a triangle");
    let NodeKind::Star { count, inner_ratio } = st.kind else { panic!("star") };
    assert_eq!(count, 5, "a new star has five points");
    assert_eq!(figcraft_geom::star_points(st.width, st.height, count, inner_ratio).len(), 10, "five points make ten sides");
    assert!((inner_ratio - 0.38).abs() < 1e-9, "silent: the default ratio is 38% (pinned)");
    assert_eq!(node(&s, line).stroke.align, figcraft_doc::StrokeAlign::Center, "lines stroke on their centre");
    assert_eq!(node(&s, rect).stroke.align, figcraft_doc::StrokeAlign::Inside, "most shapes stroke inside");
    // The polygon's first point is at the top centre.
    let pts = figcraft_geom::polygon_points(p.width, p.height, 3);
    assert!(near(pts[0].x, p.width / 2.0) && near(pts[0].y, 0.0), "the polygon points up");
}

/// Where a closed outline first meets its top edge, measured from the left (a rectangle's
/// top-left radius as drawn).
fn top_left_radius(path: &BezPath) -> f64 {
    path.elements().first().and_then(|e| e.end_point()).map_or(f64::NAN, |p| p.x)
}

#[test]
fn corner_radius_follows_the_documented_rules() {
    // (width, height, radii set, top-left radius drawn, derivation)
    let rows: Vec<(f64, f64, Value, f64, &str)> = vec![
        (200.0, 100.0, json!(20), 20.0, "a radius rounds the corners"),
        (200.0, 100.0, json!(80), 50.0, "silent: a radius larger than half the short side stops there, so corners never overlap (pinned)"),
        (200.0, 100.0, json!([10, 20, 30, 40]), 10.0, "a rectangle's corners can each have their own radius"),
        (200.0, 100.0, json!(-5), 0.0, "a negative radius is none"),
    ];
    for (w, h, radius, want, why) in rows {
        let mut s = Session::new();
        let id = create(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": w, "height": h}));
        ex(&mut s, "node.set", json!({"ids": [id], "props": {"cornerRadius": radius}}));
        let path = figcraft_doc::geometry::shape_path(&node(&s, id)).unwrap();
        assert!((top_left_radius(&path) - want).abs() < 1e-6, "{why}: {}", top_left_radius(&path));
    }
    // Which layers take a radius, and which round each corner on its own.
    let doc_rows: Vec<(NodeKind, bool, bool, &str)> = vec![
        (NodeKind::Rectangle, true, true, "rectangles: a radius, and one per corner"),
        (NodeKind::Frame, true, true, "frames: a radius, and one per corner"),
        (NodeKind::Polygon { count: 5 }, true, false, "polygons: one radius for every corner"),
        (NodeKind::Star { count: 5, inner_ratio: 0.4 }, true, false, "stars: one radius for every corner"),
        (NodeKind::Vector { network: Default::default() }, true, false, "vectors: a radius, and one per point in edit mode"),
        (NodeKind::Line, false, false, "lines and arrows take no radius"),
    ];
    for (kind, radius, independent, why) in doc_rows {
        assert_eq!((kind.has_corner_radius(), kind.has_independent_corners()), (radius, independent), "{why}");
    }
    // A polygon's radius shrinks to fit half of each edge.
    let mut s = Session::new();
    let id = create(&mut s, "shape.polygon", json!({"x": 0, "y": 0, "width": 100, "height": 100, "props": {"pointCount": 4}}));
    ex(&mut s, "node.set", json!({"ids": [id], "props": {"cornerRadius": 1000}}));
    let path = figcraft_doc::geometry::shape_path(&node(&s, id)).unwrap();
    assert!(
        path.winding(Point::new(50.0, 50.0)) != 0 && path.bounding_box().width() > 50.0,
        "silent: a huge radius makes the polygon as round as it gets (pinned)"
    );
}

#[test]
fn corner_smoothing_follows_the_documented_rules() {
    // Smoothing applies to any shape a radius applies to, and only to the entire shape.
    // (layer, radius, derivation)
    let rows: Vec<(&str, f64, &str)> = vec![
        ("shape.rectangle", 20.0, "rectangles smooth their rounded corners"),
        ("shape.polygon", 12.0, "polygons smooth their rounded corners"),
        ("shape.star", 4.0, "stars smooth their rounded corners"),
    ];
    for (cmd, radius, why) in rows {
        let mut s = Session::new();
        let id = create(&mut s, cmd, json!({"x": 0, "y": 0, "width": 100, "height": 100}));
        ex(&mut s, "node.set", json!({"ids": [id], "props": {"cornerRadius": radius}}));
        let round = figcraft_doc::geometry::shape_path(&node(&s, id)).unwrap().area().abs();
        ex(&mut s, "node.set", json!({"ids": [id], "props": {"cornerSmoothing": 0.6}}));
        let smooth = figcraft_doc::geometry::shape_path(&node(&s, id)).unwrap().area().abs();
        assert!((smooth - round).abs() > 1.0, "{why}: {round} then {smooth}");
        ex(&mut s, "node.set", json!({"ids": [id], "props": {"cornerRadius": 0}}));
        let sharp = figcraft_doc::geometry::shape_path(&node(&s, id)).unwrap();
        ex(&mut s, "node.set", json!({"ids": [id], "props": {"cornerSmoothing": 0}}));
        assert_eq!(sharp, figcraft_doc::geometry::shape_path(&node(&s, id)).unwrap(), "{why}: smoothing needs a radius");
    }
    // (kind, takes smoothing, derivation)
    let doc_rows: Vec<(NodeKind, bool, &str)> = vec![
        (NodeKind::Rectangle, true, "rectangles"),
        (NodeKind::Frame, true, "frames"),
        (NodeKind::Polygon { count: 5 }, true, "polygons"),
        (NodeKind::Star { count: 5, inner_ratio: 0.4 }, true, "stars"),
        (NodeKind::Vector { network: Default::default() }, false, "documented for closed vectors; their points round without it here"),
        (NodeKind::Line, false, "lines take no radius, so no smoothing"),
    ];
    for (kind, smoothing, why) in doc_rows {
        assert_eq!(kind.has_corner_smoothing(), smoothing, "{why}");
    }
}

#[test]
fn rounded_vector_points_are_drawn() {
    // A converted 100 × 100 square: a point's radius rounds that corner only (it wasn't drawn).
    let mut s = Session::new();
    let id = create(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 100}));
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    let net = crate::vectoredit::network(s.doc().unwrap(), id).unwrap().clone();
    let corner = net.vertices.iter().position(|v| v.x == 0.0 && v.y == 0.0).unwrap();
    ex(&mut s, "vector.edit.cornerRadius", json!({"radius": 20, "vertices": [corner]}));
    let n = node(&s, id);
    let fill = n.kind.clone();
    let NodeKind::Vector { network } = fill else { panic!("vector") };
    let path = network.fill_path();
    // (point, inside the drawn fill, derivation)
    let rows = [
        (Point::new(2.0, 2.0), false, "the rounded corner is cut away"),
        (Point::new(98.0, 2.0), true, "the other corners stay sharp"),
        (Point::new(20.0, 2.0), true, "the arc ends a radius along each edge"),
        (Point::new(50.0, 50.0), true, "the inside stays filled"),
    ];
    for (p, inside, why) in rows {
        assert_eq!(path.winding(p) != 0, inside, "{why}");
    }
    assert_eq!((n.width, n.height), (100.0, 100.0), "the layer keeps its box");
    // The layer's corner radius rounds every point.
    ex(&mut s, "vector.edit.end", json!({}));
    ex(&mut s, "node.set", json!({"ids": [id], "props": {"cornerRadius": 30}}));
    let NodeKind::Vector { network } = node(&s, id).kind else { panic!("vector") };
    let path = network.fill_path();
    assert!([(2.0, 2.0), (98.0, 2.0), (98.0, 98.0), (2.0, 98.0)].iter().all(|(x, y)| path.winding(Point::new(*x, *y)) == 0), "every corner rounds");
    // A radius larger than the edges allow shrinks to half of each edge.
    ex(&mut s, "node.set", json!({"ids": [id], "props": {"cornerRadius": 500}}));
    let NodeKind::Vector { network } = node(&s, id).kind else { panic!("vector") };
    let circle = network.fill_path();
    assert!(circle.winding(Point::new(50.0, 50.0)) != 0 && circle.winding(Point::new(12.0, 12.0)) == 0, "a square rounds into a circle at most");
    // Undo restores the sharp corners in one step.
    s.undo().unwrap();
    s.undo().unwrap();
    let NodeKind::Vector { network } = node(&s, id).kind else { panic!("vector") };
    assert!(network.fill_path().winding(Point::new(98.0, 2.0)) != 0, "undo, one step per change");
}

#[test]
fn point_counts_and_star_ratios() {
    // (layer, pointCount set, count kept, derivation)
    let rows: Vec<(&str, f64, u32, &str)> = vec![
        ("shape.star", 2.0, 3, "a star has at least 3 points"),
        ("shape.star", 61.0, 60, "a star has at most 60 points"),
        ("shape.star", 7.4, 7, "counts are whole"),
        ("shape.polygon", 2.0, 3, "silent: a polygon has at least 3 sides (pinned)"),
        ("shape.polygon", 100.0, 100, "silent: polygons keep larger counts (pinned)"),
    ];
    for (cmd, set, want, why) in rows {
        let mut s = Session::new();
        let id = create(&mut s, cmd, json!({"x": 0, "y": 0, "width": 100, "height": 100}));
        ex(&mut s, "node.set", json!({"ids": [id], "props": {"pointCount": set}}));
        let count = match node(&s, id).kind {
            NodeKind::Star { count, .. } | NodeKind::Polygon { count } => count,
            _ => 0,
        };
        assert_eq!(count, want, "{why}");
    }
    // The ratio is the inner points' distance as a share of the outer ones'.
    let mut s = Session::new();
    let id = create(&mut s, "shape.star", json!({"x": 0, "y": 0, "width": 100, "height": 100}));
    for (set, want, why) in [(json!(50), 0.5, "a ratio in percent"), (json!(0.25), 0.25, "or as a share")] {
        ex(&mut s, "node.set", json!({"ids": [id], "props": {"innerRadius": set}}));
        let NodeKind::Star { count, inner_ratio } = node(&s, id).kind else { panic!("star") };
        assert!((inner_ratio - want).abs() < 1e-9, "{why}");
        let inner = figcraft_geom::star_points(100.0, 100.0, count, inner_ratio)[1];
        assert!(((inner - Point::new(50.0, 50.0)).hypot() - 50.0 * want).abs() < 1e-9, "{why}: inner points sit at the ratio");
    }
    assert!(s.execute("node.set", &json!({"ids": [id], "props": {"pointCount": "many"}})).is_err(), "junk counts are refused");
}

#[test]
fn arcs_follow_sweep_start_and_ratio() {
    // (sweep %, ratio, point, inside, derivation) on a 100 × 100 ellipse at the origin.
    let rows: Vec<(f64, f64, (f64, f64), bool, &str)> = vec![
        (100.0, 0.0, (50.0, 50.0), true, "a full sweep is the whole ellipse"),
        (50.0, 0.0, (50.0, 80.0), true, "a positive sweep of 50% keeps the half it turns through (clockwise on screen from the right)"),
        (50.0, 0.0, (50.0, 20.0), false, "and leaves the other half out"),
        (-50.0, 0.0, (50.0, 20.0), true, "a negative sweep runs the other way"),
        (100.0, 0.5, (50.0, 50.0), false, "a ratio with a full sweep makes a ring: the middle is open"),
        (100.0, 0.5, (50.0, 10.0), true, "and the band between the radii is filled"),
        (75.0, 0.5, (50.0, 10.0), false, "a broken ring leaves its gap open"),
    ];
    for (sweep, ratio, p, inside, why) in rows {
        let mut s = Session::new();
        let id = create(&mut s, "shape.ellipse", json!({"x": 0, "y": 0, "width": 100, "height": 100}));
        ex(&mut s, "node.set", json!({"ids": [id], "props": {"arcSweep": sweep * 3.6, "arcInner": ratio}}));
        let path = figcraft_doc::geometry::shape_path(&node(&s, id)).unwrap();
        assert_eq!(path.winding(Point::new(p.0, p.1)) != 0, inside, "{why}");
    }
}

/// The stroke of a sharp V (two 100-long arms `deg` apart) as drawn with `props`: how far past
/// the tip it reaches.
fn v_tip_reach(deg: f64, props: &StrokeProps) -> f64 {
    let half = (deg / 2.0).to_radians();
    let mut path = BezPath::new();
    path.move_to((100.0 * half.sin(), 100.0 * half.cos()));
    path.line_to((0.0, 0.0));
    path.line_to((-100.0 * half.sin(), 100.0 * half.cos()));
    -figcraft_pathops::stroke_outline(&path, props, false).bounding_box().y0
}

#[test]
fn stroke_joins_miter_angle_and_caps() {
    let base = StrokeProps { weight: 10.0, join: StrokeJoin::Miter, align: figcraft_doc::StrokeAlign::Center, ..Default::default() };
    assert!((base.miter_angle() - 28.955).abs() < 0.01, "the default miter angle is about 29°");
    // (corner angle, miter angle, join, derivation)
    let rows: Vec<(f64, f64, bool, &str)> = vec![
        (60.0, 28.955, true, "a corner wider than the miter angle keeps its sharp point"),
        (20.0, 28.955, false, "a corner sharper than the miter angle is bevelled"),
        (20.0, 10.0, true, "lowering the miter angle keeps sharper corners pointed"),
    ];
    for (deg, miter, pointed, why) in rows {
        let props = StrokeProps { miter_limit: StrokeProps::miter_limit_for(miter), ..base.clone() };
        let reach = v_tip_reach(deg, &props);
        // A miter reaches w/2 / sin(θ/2) past the tip; a bevel about w/2 · sin(θ/2)'s complement.
        let mitered = 5.0 / (deg / 2.0).to_radians().sin();
        assert_eq!((reach - mitered).abs() < 0.5, pointed, "{why}: reach {reach}, miter {mitered}");
    }
    // The panel's miter angle and node.set's miterAngle are the same setting.
    let mut s = Session::new();
    let id = create(&mut s, "shape.line", json!({"x": 0, "y": 0, "width": 100}));
    ex(&mut s, "node.set", json!({"ids": [id], "props": {"miterAngle": 60}}));
    assert!((node(&s, id).stroke.miter_limit - 2.0).abs() < 1e-9 && (node(&s, id).stroke.miter_angle() - 60.0).abs() < 1e-9, "60° is a limit of 2");
    // Each end of an open path has its own cap.
    ex(&mut s, "node.set", json!({"ids": [id], "props": {"startCap": "round", "endCap": "triangleArrow"}}));
    let st = node(&s, id).stroke;
    assert_eq!((st.cap_start, st.cap_end), (StrokeCap::Round, StrokeCap::TriangleArrow), "start and end caps are set apart");
    let all = ["none", "round", "square", "lineArrow", "triangleArrow", "reversedTriangle", "circleFilled", "diamondFilled"];
    for c in all {
        assert!(s.execute("node.set", &json!({"ids": [id], "props": {"endCap": c}})).is_ok(), "the documented end caps exist: {c}");
    }
    // Arrowheads only go on open paths: a closed shape's stroke has none.
    let mut sq = BezPath::new();
    sq.move_to((0.0, 0.0));
    sq.line_to((100.0, 0.0));
    sq.line_to((100.0, 100.0));
    sq.close_path();
    let arrows = StrokeProps { cap_end: StrokeCap::TriangleArrow, ..base.clone() };
    let closed = figcraft_pathops::stroke_outline(&sq, &arrows, true).bounding_box();
    let plain = figcraft_pathops::stroke_outline(&sq, &base, true).bounding_box();
    assert_eq!(closed, plain, "closed paths have no ends to cap");
}

#[test]
fn boolean_results_and_the_look_they_take() {
    // Bottom square 0..100 (red), top square 50..150 (blue). Points: only the bottom, both, only
    // the top. (operation, inside at each point, takes the top's look, derivation)
    let rows: Vec<(&str, [bool; 3], bool, &str)> = vec![
        ("bool.union", [true, true, true], true, "union merges the shapes and looks like the top layer"),
        ("bool.subtract", [true, false, false], false, "subtract cuts the upper shapes out of the bottom one and looks like the bottom layer"),
        ("bool.intersect", [false, true, false], true, "intersect keeps where they all overlap and looks like the top layer"),
        ("bool.exclude", [true, false, true], true, "exclude keeps where they don't overlap and looks like the top layer"),
    ];
    let pts = [Point::new(25.0, 25.0), Point::new(75.0, 75.0), Point::new(125.0, 125.0)];
    for (cmd, inside, top_look, why) in rows {
        let mut s = Session::new();
        let a = create(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 100, "props": {"fill": "#FF0000"}}));
        let b = create(&mut s, "shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100, "props": {"fill": "#0000FF"}}));
        let g = create(&mut s, cmd, json!({"ids": [a, b]}));
        let gn = node(&s, g);
        let (path, _) = figcraft_render::node_outline(s.doc().unwrap(), &gn).unwrap();
        let w = s.doc().unwrap().world_transform(g).inverse();
        for (p, want) in pts.iter().zip(inside) {
            assert_eq!(path.winding(w * *p) % 2 != 0, want, "{why}: at {p:?}");
        }
        let source = if top_look { b } else { a };
        assert_eq!(gn.fills, node(&s, source).fills, "{why}: fills");
        let op = match cmd {
            "bool.union" => BoolOp::Union,
            "bool.subtract" => BoolOp::Subtract,
            "bool.intersect" => BoolOp::Intersect,
            _ => BoolOp::Exclude,
        };
        assert_eq!(gn.name, op.label(), "silent: the group is named after its operation (pinned)");
        assert_eq!(gn.children, vec![a, b], "the layers stay inside the group (non-destructive)");
    }
    // Shapes that don't overlap intersect to nothing.
    let mut s = Session::new();
    let a = create(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}));
    let b = create(&mut s, "shape.rectangle", json!({"x": 50, "y": 50, "width": 10, "height": 10}));
    let g = create(&mut s, "bool.intersect", json!({"ids": [a, b]}));
    let (path, _) = figcraft_render::node_outline(s.doc().unwrap(), &node(&s, g)).unwrap();
    assert!(path.area().abs() < 1e-6, "no overlap, nothing to show");
}

#[test]
fn flatten_keeps_open_paths() {
    // #377: open paths have no fill outline, so two pen paths flattened to "nothing to flatten"
    // and next to a closed shape they disappeared.
    let mut s = Session::new();
    let a = create(&mut s, "vector.create", json!({"points": [[0, 0], [100, 50], [200, 0]]}));
    let b = create(&mut s, "vector.create", json!({"points": [[0, 100], [100, 150], [200, 100]]}));
    let tri = create(&mut s, "vector.create", json!({"points": [[300, 0], [400, 0], [400, 100]], "closed": true}));
    let v = create(&mut s, "vector.flatten", json!({"ids": [a, b, tri]}));
    let NodeKind::Vector { network } = node(&s, v).kind else { panic!("flatten makes one vector") };
    assert_eq!((network.vertices.len(), network.segments.len()), (9, 7), "both open paths and the triangle");
    assert_eq!(network.regions.len(), 1, "only the triangle is filled");
    let bb = s.doc().unwrap().world_bounds(v).unwrap();
    assert_eq!((bb.x0, bb.y0, bb.x1, bb.y1), (0.0, 0.0, 400.0, 150.0), "nothing moves");

    // Joining two open paths goes the same way.
    let mut s = Session::new();
    let a = create(&mut s, "vector.create", json!({"points": [[0, 0], [100, 0]]}));
    let b = create(&mut s, "vector.create", json!({"points": [[0, 50], [100, 50]]}));
    let v = create(&mut s, "vector.join", json!({"ids": [a, b]}));
    let NodeKind::Vector { network } = node(&s, v).kind else { panic!("join makes one vector") };
    assert_eq!(network.segments.len(), 2);
}

#[test]
fn flatten_and_outline_stroke() {
    // Flatten: one vector layer from the selection, in one step; Alt+Shift+F does it.
    let mut s = Session::new();
    let a = create(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 100}));
    let b = create(&mut s, "shape.ellipse", json!({"x": 50, "y": 50, "width": 100, "height": 100}));
    let page = s.page().unwrap();
    let v = create(&mut s, "vector.flatten", json!({"ids": [a, b]}));
    assert!(matches!(node(&s, v).kind, NodeKind::Vector { .. }), "flatten makes one vector");
    assert_eq!(s.doc().unwrap().node(page).unwrap().children, vec![v], "the flattened layers are gone (destructive)");
    s.undo().unwrap();
    assert_eq!(s.doc().unwrap().node(page).unwrap().children, vec![a, b], "undo brings them back");
    let keys: Vec<&str> = crate::keymap::bindings("vector.flatten", crate::keymap::Platform::Mac).collect();
    assert!(keys.contains(&"Alt+Shift+F"), "Option/Alt+Shift+F flattens: {keys:?}");
    assert!(
        crate::keymap::bindings("vector.outlineStroke", crate::keymap::Platform::Mac).any(|k| k == "Alt+Cmd+O"),
        "Cmd/Ctrl+Alt+O outlines strokes"
    );

    // Outline stroke: the stroke becomes a filled shape, its colour style becomes the fill's, and
    // the outline keeps the layer's opacity and effects.
    let mut s = Session::new();
    let line = create(&mut s, "shape.line", json!({"x": 0, "y": 0, "width": 100, "props": {"strokeWeight": 10, "opacity": 0.5}}));
    ex(&mut s, "style.create", json!({"type": "paint", "name": "Ink", "value": ["#112233"]}));
    ex(&mut s, "style.apply", json!({"ids": [line], "style": "Ink", "target": "stroke"}));
    let before = node(&s, line);
    let out = ex(&mut s, "vector.outlineStroke", json!({"ids": [line]}));
    let v = id_of(&out["ids"][0]).unwrap();
    let n = node(&s, v);
    assert!(n.strokes.is_empty() && n.fills == before.strokes, "the stroke's paints fill the outline");
    assert_eq!(n.styles.fill, before.styles.stroke, "a stroke's colour style becomes the outline's fill style");
    assert_eq!(n.opacity, 0.5, "silent: the outline keeps the layer's opacity (fixed: it was lost)");
    let (path, _) = figcraft_render::node_outline(s.doc().unwrap(), &n).unwrap();
    assert!((path.bounding_box().height() - 10.0).abs() < 1e-6, "the outline is as thick as the stroke was");
}

#[test]
fn outline_stroke_outlines_every_selected_layer() {
    // Fixed (#378): with several filled, stroked shapes selected, only the first was outlined.
    for by_selection in [false, true] {
        let mut s = Session::new();
        let a = create(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}));
        let b = create(&mut s, "shape.rectangle", json!({"x": 100, "y": 0, "width": 50, "height": 50}));
        let line = create(&mut s, "shape.line", json!({"x": 0, "y": 100, "width": 100, "props": {"strokeWeight": 4}}));
        ex(&mut s, "node.set", json!({"ids": [a, b], "props": {"strokes": ["#000000"], "strokeWeight": 4}}));
        let out = if by_selection {
            s.set_selection(vec![a, b, line]);
            ex(&mut s, "vector.outlineStroke", json!({}))
        } else {
            ex(&mut s, "vector.outlineStroke", json!({"ids": [a, b, line]}))
        };
        let ids: Vec<NodeId> = out["ids"].as_array().unwrap().iter().filter_map(id_of).collect();
        assert_eq!(ids.len(), 3, "one result per layer: {out}");
        assert!(matches!(node(&s, ids[0]).kind, NodeKind::Group), "filled shape a becomes a group");
        assert!(matches!(node(&s, ids[1]).kind, NodeKind::Group), "filled shape b becomes a group");
        assert!(matches!(node(&s, ids[2]).kind, NodeKind::Vector { .. }), "the line becomes a vector");
        for id in [a, b] {
            assert!(node(&s, id).strokes.is_empty(), "{id:?} still has its stroke");
        }
        assert!(s.doc().unwrap().node(line).is_none(), "the line was replaced by its outline");
        assert_eq!(s.selection(), ids, "the results are selected");
    }
}
