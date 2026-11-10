//! `round_corners`: every sharp corner of a closed path, convex or concave, rounds with a circular
//! arc; smooth joins and straight-run leftovers of the sweep do not count; a radius larger than the
//! edges allow shrinks to fit.

use figcraft_doc::BoolOp;
use figcraft_geom::kurbo::{Affine, BezPath, PathEl, PathSeg, Point, Shape, Vec2};
use figcraft_geom::{ellipse, rounded_rect, split_subpaths};

use crate::{boolean, boolean_step, round_corners};

fn rect(x: f64, y: f64, w: f64, h: f64) -> BezPath {
    kurbo_rect(x, y, w, h).to_path(0.1)
}

fn kurbo_rect(x: f64, y: f64, w: f64, h: f64) -> figcraft_geom::Rect {
    figcraft_geom::Rect::new(x, y, x + w, y + h)
}

fn l_shape() -> BezPath {
    boolean(&[rect(0.0, 0.0, 200.0, 100.0), rect(150.0, 0.0, 100.0, 200.0)], BoolOp::Union).unwrap()
}

/// A (200 by 100, radius 10) with a 100 square cut out of its top edge: the notch of the measurement.
fn notch() -> BezPath {
    let a = rounded_rect(200.0, 100.0, [10.0; 4], 0.0);
    boolean_step(&a, &rect(50.0, -50.0, 100.0, 100.0), BoolOp::Subtract).unwrap()
}

/// Every `CurveTo` as `[from, c1, c2, to]`.
fn arcs(path: &BezPath) -> Vec<[Point; 4]> {
    let mut cur = Point::ZERO;
    let mut out = Vec::new();
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => cur = p,
            PathEl::CurveTo(a, b, c) => {
                out.push([cur, a, b, c]);
                cur = c;
            }
            _ => {}
        }
    }
    out
}

fn near(a: Point, b: Point) -> bool {
    (a - b).hypot() < 1e-3
}

fn pt(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

/// Some curve runs through exactly these four points, in either direction.
fn has_arc(path: &BezPath, want: [(f64, f64); 4]) -> bool {
    let w = want.map(|(x, y)| pt(x, y));
    let rev = [w[3], w[2], w[1], w[0]];
    arcs(path).iter().any(|a| a.iter().zip(w).all(|(p, q)| near(*p, q)) || a.iter().zip(rev).all(|(p, q)| near(*p, q)))
}

/// Some curve joins these two end points, in either direction.
fn has_arc_between(path: &BezPath, from: (f64, f64), to: (f64, f64)) -> bool {
    let (f, t) = (pt(from.0, from.1), pt(to.0, to.1));
    arcs(path).iter().any(|a| (near(a[0], f) && near(a[3], t)) || (near(a[0], t) && near(a[3], f)))
}

fn passes_through(path: &BezPath, p: Point) -> bool {
    path.elements().iter().any(|el| match el {
        PathEl::MoveTo(q) | PathEl::LineTo(q) | PathEl::CurveTo(_, _, q) => near(*q, p),
        _ => false,
    })
}

fn unit(v: Vec2) -> Option<Vec2> {
    (v.hypot() > 1e-9).then(|| v / v.hypot())
}

fn start_dir(s: PathSeg) -> Option<Vec2> {
    let c = s.to_cubic();
    [c.p1, c.p2, c.p3].into_iter().find_map(|p| unit(p - c.p0))
}

fn end_dir(s: PathSeg) -> Option<Vec2> {
    let c = s.to_cubic();
    [c.p2, c.p1, c.p0].into_iter().find_map(|p| unit(c.p3 - p))
}

/// The largest direction change at any vertex of a closed subpath, in radians.
fn sharpest_turn(path: &BezPath) -> f64 {
    let mut worst: f64 = 0.0;
    for sub in split_subpaths(path) {
        let segs: Vec<PathSeg> = sub.segments().collect();
        for (i, s) in segs.iter().enumerate() {
            let next = segs[(i + 1) % segs.len()];
            if let (Some(a), Some(b)) = (end_dir(*s), start_dir(next)) {
                worst = worst.max(a.dot(b).clamp(-1.0, 1.0).acos());
            }
        }
    }
    worst
}

fn area(p: &BezPath) -> f64 {
    p.area().abs()
}

fn all_finite(p: &BezPath) -> bool {
    crate::finite(p)
}

#[test]
fn every_sharp_corner_of_the_result_rounds_convex_and_concave() {
    let r = round_corners(&l_shape(), 20.0, 0.0);
    assert_eq!(arcs(&r).len(), 6);
    assert!(has_arc(&r, [(230.0, 0.0), (241.046, 0.0), (250.0, 8.95431), (250.0, 20.0)]));
    assert!(has_arc(&r, [(150.0, 120.0), (150.0, 108.954), (141.046, 100.0), (130.0, 100.0)]));
    assert!((area(&r) - (35000.0 - 4.0 * 85.84)).abs() < 0.5, "{}", area(&r));
    assert_eq!(r.winding(pt(1.0, 1.0)), 0);
    assert_ne!(r.winding(pt(148.0, 102.0)), 0);
    let bb = r.bounding_box();
    assert!((bb.width() - 250.0).abs() < 1e-6 && (bb.height() - 200.0).abs() < 1e-6);
}

#[test]
fn a_childs_rounded_corners_are_kept_and_do_not_stack() {
    let r = round_corners(&notch(), 20.0, 0.0);
    assert_eq!(arcs(&r).len(), 8);
    assert!(has_arc(&r, [(30.0, 0.0), (41.0457, 0.0), (50.0, 8.954), (50.0, 20.0)]));
    assert!(has_arc(&r, [(50.0, 30.0), (50.0, 41.0457), (58.954, 50.0), (70.0, 50.0)]));
    assert!(has_arc_between(&r, (130.0, 50.0), (150.0, 30.0)));
    assert!(has_arc_between(&r, (150.0, 20.0), (170.0, 0.0)));
    assert!(has_arc(&r, [(0.0, 10.0), (0.0, 4.477), (4.477, 0.0), (10.0, 0.0)]));
}

#[test]
fn a_radius_larger_than_the_edges_clamps_to_what_they_hold() {
    let r = round_corners(&notch(), 500.0, 0.0);
    assert!(has_arc(&r, [(25.0, 0.0), (38.8071, 0.0), (50.0, 11.1929), (50.0, 25.0)]));
    assert!(has_arc(&r, [(50.0, 25.0), (50.0, 38.807), (61.193, 50.0), (75.0, 50.0)]));
    assert!(has_arc_between(&r, (125.0, 50.0), (150.0, 25.0)));
    assert!(has_arc_between(&r, (150.0, 25.0), (175.0, 0.0)));
    assert!(has_arc(&r, [(0.0, 10.0), (0.0, 4.477), (4.477, 0.0), (10.0, 0.0)]));
    assert!(all_finite(&r));
    let l = round_corners(&l_shape(), 500.0, 0.0);
    assert!(has_arc_between(&l, (0.0, 50.0), (50.0, 0.0)));
    assert!(has_arc_between(&l, (150.0, 0.0), (250.0, 100.0)));
    assert!((area(&round_corners(&l_shape(), 1e300, 0.0)) - area(&l)).abs() < 1e-6);
}

#[test]
fn nothing_sharp_nothing_changes() {
    let round = rounded_rect(100.0, 60.0, [20.0; 4], 0.0);
    let oval = ellipse(100.0, 60.0, 0.0, std::f64::consts::TAU, 0.0);
    assert_eq!(round_corners(&round, 20.0, 0.0), round);
    assert_eq!(round_corners(&oval, 20.0, 0.0), oval);
    let l = l_shape();
    for radius in [0.0, -5.0, f64::NAN, f64::INFINITY] {
        assert_eq!(round_corners(&l, radius, 0.0), l);
    }
    let mut open = BezPath::new();
    open.move_to((0.0, 0.0));
    open.line_to((50.0, 0.0));
    open.line_to((50.0, 50.0));
    assert_eq!(round_corners(&open, 10.0, 0.0), open);
}

#[test]
fn points_the_sweep_leaves_on_an_edge_are_not_corners() {
    let mut p = BezPath::new();
    for (i, q) in [(0.0, 0.0), (20.0, 0.0), (100.0, 0.0), (100.0, 0.0), (100.0, 60.0), (100.0 + 1e-11, 60.0), (100.0, 100.0), (0.0, 100.0)]
        .into_iter()
        .enumerate()
    {
        if i == 0 {
            p.move_to(q);
        } else {
            p.line_to(q);
        }
    }
    p.close_path();
    let r = round_corners(&p, 500.0, 0.0);
    assert_eq!(arcs(&r).len(), 4);
    assert!(has_arc_between(&r, (0.0, 50.0), (50.0, 0.0)));
}

fn two_circles(op: BoolOp) -> BezPath {
    let c = ellipse(100.0, 100.0, 0.0, std::f64::consts::TAU, 0.0);
    boolean(&[c.clone(), Affine::translate((70.0, 0.0)) * c], op).unwrap()
}

#[test]
fn corners_beside_curves_round_too() {
    for op in [BoolOp::Union, BoolOp::Intersect] {
        let sharp = two_circles(op);
        assert!(sharpest_turn(&sharp) > 1.0);
        let r = round_corners(&sharp, 10.0, 0.0);
        assert!(sharpest_turn(&r) < 1e-3, "{}", sharpest_turn(&r));
        assert!(all_finite(&r));
        match op {
            BoolOp::Union => assert!(area(&r) > area(&sharp)),
            _ => assert!(area(&r) < area(&sharp)),
        }
    }
}

#[test]
fn smoothing_draws_the_polygon_profile_between_straight_edges() {
    let l = l_shape();
    let plain = round_corners(&l, 20.0, 0.0);
    let smooth = round_corners(&l, 20.0, 0.6);
    assert!(passes_through(&smooth, pt(0.0, 32.0)) && passes_through(&smooth, pt(32.0, 0.0)));
    assert!((area(&smooth) - area(&plain)).abs() > 1.0);
    let circles = two_circles(BoolOp::Union);
    assert_eq!(round_corners(&circles, 10.0, 0.6), round_corners(&circles, 10.0, 0.0));
}

#[test]
fn junk_never_panics() {
    let mut bad = BezPath::new();
    bad.move_to((f64::NAN, 0.0));
    bad.line_to((10.0, f64::INFINITY));
    bad.close_path();
    assert_eq!(round_corners(&bad, 5.0, 0.0).elements().len(), 3);
    assert!(round_corners(&BezPath::new(), 5.0, 0.0).elements().is_empty());
    let mut one = BezPath::new();
    one.move_to((0.0, 0.0));
    one.line_to((10.0, 0.0));
    one.close_path();
    assert!(all_finite(&round_corners(&one, 5.0, 0.5)));
    assert!(all_finite(&round_corners(&rect(0.0, 0.0, 1.0, 0.5), 0.1, 0.0)));
    let mut zig = BezPath::new();
    zig.move_to((0.0, 0.0));
    for i in 1..40_000 {
        zig.line_to((f64::from(i) * 0.5, f64::from(i % 2)));
    }
    zig.close_path();
    assert!(all_finite(&round_corners(&zig, 2.0, 0.0)));
}
