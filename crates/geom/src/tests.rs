use super::*;

fn area(p: &BezPath) -> f64 {
    p.area().abs()
}

#[test]
fn plain_rect_area() {
    let p = rounded_rect(100.0, 50.0, [0.0; 4], 0.0);
    assert!((area(&p) - 5000.0).abs() < 1e-6);
}

#[test]
fn rounded_rect_area_matches_circle_corners() {
    let r = 10.0;
    let p = rounded_rect(100.0, 50.0, [r; 4], 0.0);
    let expected = 5000.0 - 4.0 * r * r + PI * r * r;
    assert!((area(&p) - expected).abs() < 0.5, "{} vs {}", area(&p), expected);
}

#[test]
fn full_radius_is_capsule_and_bounds_hold() {
    let p = rounded_rect(100.0, 40.0, [1000.0; 4], 0.0);
    let b = p.bounding_box();
    assert!((b.width() - 100.0).abs() < 1e-6 && (b.height() - 40.0).abs() < 1e-6);
}

#[test]
fn smoothing_stays_in_bounds_and_reduces_area_less_than_sharp() {
    for s in [0.0, 0.3, 0.6, 1.0] {
        let p = rounded_rect(200.0, 120.0, [24.0, 0.0, 40.0, 8.0], s);
        let b = p.bounding_box();
        assert!(b.x0 >= -1e-6 && b.y0 >= -1e-6 && b.x1 <= 200.0 + 1e-6 && b.y1 <= 120.0 + 1e-6, "s={s} {b:?}");
        assert!(area(&p) < 200.0 * 120.0);
    }
    // More smoothing removes more area at the same radius.
    let a0 = area(&rounded_rect(200.0, 200.0, [40.0; 4], 0.0));
    let a1 = area(&rounded_rect(200.0, 200.0, [40.0; 4], 1.0));
    assert!(a1 < a0);
}

#[test]
fn degenerate_inputs_dont_panic() {
    let _ = rounded_rect(f64::NAN, -1.0, [f64::INFINITY; 4], f64::NAN);
    let _ = rounded_rect(0.0, 0.0, [5.0; 4], 0.5);
    let _ = polygon_points(0.0, 0.0, 0);
    let _ = star_points(10.0, 10.0, 1, -3.0);
    let _ = rounded_polygon(&[], 4.0);
    let _ = rounded_polygon(&[Point::ZERO, Point::ZERO, Point::ZERO], 4.0);
    let _ = smoothed_polygon(&[Point::ZERO, Point::ZERO, Point::ZERO], 4.0, 0.6);
    let _ = smoothed_polygon(&[Point::ZERO, Point::new(10.0, 0.0), Point::new(20.0, 0.0)], 4.0, 1.0);
    let _ = smoothed_polygon(&polygon_points(10.0, 10.0, 5), f64::INFINITY, f64::NAN);
    let _ = smoothed_polygon(&polygon_points(10.0, 10.0, 5), 3.0, f64::INFINITY);
    let _ = ellipse(f64::NAN, 10.0, f64::NAN, 100.0, 2.0);
    // (corner, first edge, second edge, room, radius, smoothing): none of these is a corner to round.
    let x = Vec2::new(1.0, 0.0);
    let rows = [
        (Point::ZERO, x, Vec2::new(0.0, 1.0), 10.0, 0.0, 0.6),
        (Point::ZERO, x, Vec2::new(0.0, 1.0), 0.0, 5.0, 0.6),
        (Point::ZERO, x, x, 10.0, 5.0, 0.6),
        (Point::ZERO, x, -x, 10.0, 5.0, 0.6),
        (Point::ZERO, Vec2::ZERO, x, 10.0, 5.0, 0.6),
        (Point::ZERO, Vec2::new(f64::NAN, 0.0), x, 10.0, 5.0, 0.6),
        (Point::new(f64::INFINITY, 0.0), x, Vec2::new(0.0, 1.0), 10.0, 5.0, 0.6),
    ];
    for (corner, u1, u2, room, r, s) in rows {
        assert!(smoothed_corner(corner, u1, u2, room, r, s).is_none(), "{corner:?} {u1:?} {u2:?} {room} {r} {s}");
    }
}

#[test]
fn smoothed_corner_draws_the_polygon_corner() {
    use kurbo::ParamCurve;
    // The corner of a square drawn by itself follows the smoothed polygon's corner and joins its
    // edges and its own pieces without a kink.
    for s in [0.0, 0.6, 1.0] {
        let curves = smoothed_corner(Point::ZERO, Vec2::new(0.0, 1.0), Vec2::new(1.0, 0.0), 50.0, 20.0, s).unwrap();
        let mut path = BezPath::new();
        path.move_to(curves[0].p0);
        for q in &curves {
            path.curve_to(q.p1, q.p2, q.p3);
        }
        path.line_to((100.0, 0.0));
        path.line_to((100.0, 100.0));
        path.line_to((0.0, 100.0));
        path.close_path();
        let want = area(&rounded_rect(100.0, 100.0, [20.0, 0.0, 0.0, 0.0], s));
        assert!((area(&path) - want).abs() < 0.05, "s={s}: {} vs {want}", area(&path));
        assert!((curves[0].p0 - Point::new(0.0, 20.0 * (1.0 + s))).hypot() < 1e-9, "s={s}: it starts 1 + smoothing radii along the edge");
        let last = curves[curves.len() - 1];
        assert!((last.p3 - Point::new(20.0 * (1.0 + s), 0.0)).hypot() < 1e-9, "s={s}: and ends as far along the other");
        let dir = |c: &kurbo::CubicBez, t: f64| {
            let d = if t < 0.5 { c.eval(1e-6) - c.eval(0.0) } else { c.eval(1.0) - c.eval(1.0 - 1e-6) };
            d.normalize()
        };
        assert!(dir(&curves[0], 0.0).dot(Vec2::new(0.0, -1.0)) > 0.999, "s={s}: it leaves along the first edge");
        assert!(dir(&last, 1.0).dot(Vec2::new(1.0, 0.0)) > 0.999, "s={s}: it arrives along the second edge");
        for pair in curves.windows(2) {
            assert!(dir(&pair[0], 1.0).dot(dir(&pair[1], 0.0)) > 0.999, "s={s}: its pieces join smoothly");
        }
    }
}

/// The measurement's sanity bound is the builders' cap, inclusive: a path built at `MAX_COORD`
/// has coordinates exactly there and still measures; one past it doesn't.
#[test]
fn paths_at_the_cap_still_measure() {
    use crate::measure::{finite_path, polylines};
    let at_cap = rounded_rect(MAX_COORD, MAX_COORD, [0.0; 4], 0.0);
    let b = at_cap.bounding_box();
    assert_eq!((b.x1, b.y1), (MAX_COORD, MAX_COORD), "corners exactly at the cap");
    assert!(finite_path(&at_cap));
    assert!(polylines(&at_cap).is_some(), "a rectangle at the cap measures");
    let mut line = BezPath::new();
    line.move_to(Point::new(-MAX_COORD, 0.0));
    line.line_to(Point::new(MAX_COORD, -MAX_COORD));
    assert!(polylines(&line).is_some(), "a line between the cap's corners measures");
    // A curve at the cap: the cubics approximating the arc have control points past the cap, and
    // the path still measures.
    let arc = ellipse(MAX_COORD, 1.0, -0.15, 0.2, 0.0);
    let reach = arc
        .elements()
        .iter()
        .flat_map(|el| match el {
            kurbo::PathEl::CurveTo(a, b, _) => vec![a.x, b.x],
            _ => vec![],
        })
        .fold(0.0, f64::max);
    assert!(reach > MAX_COORD, "a control point lies past the cap: {reach}");
    assert!(finite_path(&arc));
    assert!(polylines(&arc).is_some(), "an arc at the cap measures");
    let mut past = BezPath::new();
    past.move_to(Point::new(0.0, 0.0));
    past.line_to(Point::new(2.0 * MAX_COORD, 0.0));
    assert!(!finite_path(&past));
    assert!(polylines(&past).is_none(), "a coordinate well past the cap is refused");
}

#[test]
fn absurd_sizes_build_bounded_paths() {
    // kurbo subdivides an arc in proportion to the sixth root of its radius, so without a cap the
    // segment count grows without bound with the input: a 1e36 ellipse was about a million
    // elements and 1e60 never returned. Sizes are untrusted (files, commands), so they are capped.
    // At the cap itself (1e9) an arc is still a few dozen segments.
    for mag in [MAX_COORD, 1e36, 1e300, f64::MAX] {
        let paths = [
            ("ellipse", ellipse(mag, mag, 0.0, PI, 0.0)),
            ("ring", ellipse(mag, mag, 0.0, TAU, 0.5)),
            ("rounded_rect", rounded_rect(mag, mag, [mag; 4], 0.0)),
            ("smoothed rect", rounded_rect(mag, mag, [mag; 4], 0.6)),
            ("smoothed_polygon", smoothed_polygon(&polygon_points(mag, mag, 5), mag, 0.6)),
        ];
        for (name, p) in paths {
            assert!(p.elements().len() < 1_000, "{name} at {mag:e}: {} elements", p.elements().len());
            let b = p.bounding_box();
            assert!([b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite() && v.abs() <= 1.01 * MAX_COORD), "{name} at {mag:e}: {b:?}");
        }
        // Points are the caller's to keep sane; a huge radius is still capped.
        let p = smoothed_polygon(&[Point::new(0.0, 0.0), Point::new(mag, 0.0), Point::new(mag, mag)], mag, 0.6);
        assert!(p.elements().len() < 1_000, "huge points at {mag:e}: {} elements", p.elements().len());
    }
    // NaN, infinities and negative sizes build nothing or a bounded path with a finite box.
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e36, -1.0] {
        let paths = [
            ("ellipse", ellipse(v, v, v, v, v)),
            ("rounded_rect", rounded_rect(v, v, [v; 4], v)),
            ("smoothed_polygon", smoothed_polygon(&polygon_points(100.0, 100.0, 5), v, v)),
        ];
        for (name, p) in paths {
            assert!(p.elements().len() < 1_000, "{name} at {v}: {} elements", p.elements().len());
            let b = p.bounding_box();
            assert!([b.x0, b.y0, b.x1, b.y1].iter().all(|x| x.is_finite()), "{name} at {v}: {b:?}");
        }
    }
    // Sizes a document can hold are untouched.
    let p = rounded_rect(1e6, 1e6, [1e5; 4], 0.6);
    let b = p.bounding_box();
    assert!((b.width() - 1e6).abs() < 1e-3 && (b.height() - 1e6).abs() < 1e-3, "{b:?}");
}

#[test]
fn nearest_refuses_a_non_finite_point() {
    let mut p = BezPath::new();
    p.move_to((0.0, 0.0));
    p.line_to((10.0, 0.0));
    let lines = measure::polylines(&p).unwrap();
    assert_eq!(lines[0].nearest(Point::new(f64::NAN, 0.0)), None);
    assert_eq!(lines[0].nearest(Point::new(0.0, f64::INFINITY)), None);
    let (t, d) = lines[0].nearest(Point::new(5.0, 3.0)).unwrap();
    assert!((t - 0.5).abs() < 1e-9 && (d - 3.0).abs() < 1e-9);
}

#[test]
fn polygon_first_vertex_top_center() {
    let pts = polygon_points(100.0, 100.0, 3);
    assert_eq!(pts.len(), 3);
    assert!((pts[0].x - 50.0).abs() < 1e-9 && pts[0].y.abs() < 1e-9);
}

#[test]
fn star_has_twice_the_points() {
    assert_eq!(star_points(100.0, 100.0, 5, 0.38).len(), 10);
}

#[test]
fn rounded_polygon_shrinks_area() {
    let pts = polygon_points(100.0, 100.0, 6);
    let sharp = area(&rounded_polygon(&pts, 0.0));
    let round = area(&rounded_polygon(&pts, 10.0));
    assert!(round < sharp && round > sharp * 0.8);
}

#[test]
fn smoothed_polygon_matches_the_rectangle_corner() {
    // A four-sided polygon is a square turned 45°: smoothed, it has the rounded rectangle's area.
    let side = 50.0 * std::f64::consts::SQRT_2;
    let pts = polygon_points(100.0, 100.0, 4);
    for (r, s) in [(10.0, 0.0), (10.0, 0.6), (12.0, 1.0), (30.0, 0.6), (1000.0, 0.6)] {
        let poly = area(&smoothed_polygon(&pts, r, s));
        let rect = area(&rounded_rect(side, side, [r; 4], s));
        assert!((poly - rect).abs() < 0.05, "r={r} s={s}: {poly} vs {rect}");
    }
}

#[test]
fn smoothed_polygon_smooths_every_corner() {
    use kurbo::{ParamCurve, ParamCurveArclen, ParamCurveNearest};
    let near = |p: &BezPath, at: Point| p.segments().map(|seg| seg.nearest(at, 1e-9).distance_sq).fold(f64::INFINITY, f64::min);
    // (points, convex: smoothing only cuts area away)
    let shapes = [(polygon_points(100.0, 100.0, 3), true), (polygon_points(120.0, 80.0, 6), true), (star_points(100.0, 100.0, 5, 0.38), false)];
    for (pts, convex) in shapes {
        let plain = smoothed_polygon(&pts, 8.0, 0.0);
        assert_eq!(plain, rounded_polygon(&pts, 8.0), "no smoothing draws the plain rounded corners");
        let sharp = area(&rounded_polygon(&pts, 0.0));
        let mut last = area(&plain);
        for s in [0.3, 0.6, 1.0] {
            let p = smoothed_polygon(&pts, 8.0, s);
            let a = area(&p);
            assert!(if convex { a < last } else { (a - last).abs() > 1e-3 }, "s={s}: more smoothing changes the area ({a} after {last})");
            assert!((a - sharp).abs() < sharp * 0.2, "s={s}: the shape stays close to its points");
            last = a;
            // The outline has no kinks: each piece leaves in the direction the one before arrives.
            let segs: Vec<kurbo::PathSeg> = p.segments().filter(|seg| seg.arclen(1e-6) > 1e-6).collect();
            for (i, seg) in segs.iter().enumerate() {
                let next = segs[(i + 1) % segs.len()];
                let out = (seg.eval(1.0) - seg.eval(1.0 - 1e-6)).normalize();
                let into = (next.eval(1e-6) - next.eval(0.0)).normalize();
                assert!(out.dot(into) > 0.999, "s={s}: piece {i} joins the next smoothly ({out:?} then {into:?})");
            }
            let b = p.bounding_box();
            assert!(b.x0 >= -1e-6 && b.y0 >= -1e-6 && b.x1 <= 120.0 + 1e-6 && b.y1 <= 100.0 + 1e-6, "s={s} {b:?}");
            // Every vertex is cut off, and the middle of every edge is still on the outline.
            for (i, v) in pts.iter().enumerate() {
                let next = pts[(i + 1) % pts.len()];
                let mid = v.midpoint(next);
                assert!(near(&p, mid) < 1e-6, "s={s}: edge {i} keeps its middle");
                assert!(near(&p, *v) > 0.25, "s={s}: vertex {i} is rounded");
            }
        }
    }
}

#[test]
fn ellipse_ring_area() {
    let full = area(&ellipse(100.0, 100.0, 0.0, TAU, 0.0));
    assert!((full - PI * 2500.0).abs() < 2.0);
    let ring = ellipse(100.0, 100.0, 0.0, TAU, 0.5);
    // Winding area of a ring with a reversed inner contour: outer - inner.
    let a = ring.area().abs();
    assert!((a - PI * (2500.0 - 625.0)).abs() < 5.0, "{a}");
    let half = area(&ellipse(100.0, 100.0, 0.0, PI, 0.0));
    assert!((half - PI * 1250.0).abs() < 2.0);
}

#[test]
fn rotation_roundtrip() {
    for deg in [0.0, 30.0, -45.0, 90.0, 179.0] {
        let xf = from_position_rotation(10.0, 20.0, deg);
        assert!((rotation_degrees(xf) - deg).abs() < 1e-9, "{deg}");
        assert_eq!(translation(xf), Point::new(10.0, 20.0));
    }
}

#[test]
fn set_rotation_keeps_center() {
    let xf = from_position_rotation(10.0, 20.0, 0.0);
    let r = set_rotation_about_center(xf, 100.0, 50.0, 90.0);
    let c0 = xf * Point::new(50.0, 25.0);
    let c1 = r * Point::new(50.0, 25.0);
    assert!((c0 - c1).hypot() < 1e-9);
    assert!((rotation_degrees(r) - 90.0).abs() < 1e-9);
    // A flipped box stays flipped and reads back the rotation set, whichever way it was flipped.
    for flip in [Affine::FLIP_X, Affine::FLIP_Y] {
        for deg in [0.0, 30.0, -120.0] {
            let r = set_rotation_about_center(xf * flip, 100.0, 50.0, deg);
            assert!(is_flipped(r) && (rotation_degrees(r) - deg).abs() < 1e-9, "{deg}");
            assert!((xf * flip * Point::new(50.0, 25.0) - r * Point::new(50.0, 25.0)).hypot() < 1e-9);
        }
    }
}

#[test]
fn round_to_handles_large_precision_without_changing_finite_values() {
    assert_eq!(round_to(1.25, 400), 1.25);
    assert_eq!(round_to(-1.25, 400), -1.25);
    assert_eq!(round_to(1.23456, 2), 1.23);
}

#[test]
fn reverse_subpaths_preserves_open_and_closed() {
    let mut open = BezPath::new();
    open.move_to((0.0, 0.0));
    open.line_to((10.0, 0.0));
    open.line_to((10.0, 10.0));
    let r = <BezPath as ReverseSubpaths>::reverse_subpaths(&open);
    assert!(!r.elements().iter().any(|e| matches!(e, kurbo::PathEl::ClosePath)), "{:?}", r.elements());

    let mut closed = open.clone();
    closed.close_path();
    let r = <BezPath as ReverseSubpaths>::reverse_subpaths(&closed);
    assert!(matches!(r.elements().last(), Some(kurbo::PathEl::ClosePath)), "{:?}", r.elements());
}

#[test]
fn a_huge_ellipse_arc_is_a_few_cubics() {
    let big = ellipse(1e300, 1e300, 0.3, 2.0, 0.5);
    assert!(big.elements().len() < 200, "{}", big.elements().len());
}

#[test]
fn a_huge_ring_is_a_few_cubics() {
    let ring = ellipse(1e300, 1e300, 0.0, TAU, 0.5);
    assert!(ring.elements().len() < 200, "{}", ring.elements().len());
}

#[test]
fn a_huge_smoothed_star_is_a_few_cubics() {
    let star = smoothed_polygon(&star_points(1e300, 1e300, 5, 0.5), 1e299, 0.5);
    assert!(star.elements().len() < 400, "{}", star.elements().len());
}

#[test]
fn a_huge_rounded_rect_is_a_few_cubics() {
    let rect = rounded_rect(1e300, 1e300, [1e299; 4], 0.0);
    assert!(rect.elements().len() < 100, "{}", rect.elements().len());
}

#[test]
fn an_infinite_radius_is_a_few_cubics() {
    let mut n = 0;
    kurbo::Arc::new((0.0, 0.0), (f64::INFINITY, 1.0), 0.0, 1.0, 0.0).to_cubic_beziers(arc_tolerance(f64::INFINITY), |_, _, _| n += 1);
    assert!(n < 16, "{n}");
}

#[test]
fn svg_path_refuses_numbers_past_the_limit_before_parsing() {
    let start = std::time::Instant::now();
    for d in ["M0 0 A1e300 1e300 0 0 1 1 1", "M0 0A1e300-1e300 0 0 1 1 1", "M 0 0 L 1e10 5", "M0 0L-2e9.5 0", "M0 0 L 1e999 0"] {
        assert!(svg_path(d).is_err(), "{d}");
    }
    assert!(start.elapsed() < std::time::Duration::from_secs(2), "{:?}", start.elapsed());
    for d in ["M0 0 L 1e9 5 A 5 5 0 0 1 10 10 Z", "M0 0 L-2.5 0.5 L1e-3 -4e2 Z", "M10,20 l1,-2.5e+1"] {
        assert!(svg_path(d).is_ok(), "{d}");
    }
    assert!(!number_past("M0 0L-2.5.5 1e-3-4e2z", 1e3));
    assert!(number_past("M0 0L-2.5.5 1e-3-4e3z", 1e3));
    assert!(number_past("M.5.5e10", 1e3));
}
