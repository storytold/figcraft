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

/// A path through `pts` with straight lines (pencil samples).
fn polyline(pts: &[(f64, f64)]) -> BezPath {
    let mut p = BezPath::new();
    for (k, (x, y)) in pts.iter().enumerate() {
        if k == 0 { p.move_to((*x, *y)) } else { p.line_to((*x, *y)) }
    }
    p
}

/// How far `out` strays from `input`: the largest distance from a point sampled along `out` to the
/// nearest segment of `input` (closing segments included).
fn deviation(out: &BezPath, input: &BezPath) -> f64 {
    use kurbo::{ParamCurve, ParamCurveNearest};
    let segs: Vec<kurbo::PathSeg> = input.segments().collect();
    out.segments()
        .flat_map(|seg| (0..=32).map(move |i| seg.eval(f64::from(i) / 32.0)))
        .map(|p| segs.iter().map(|s| s.nearest(p, 1e-6).distance_sq).fold(f64::INFINITY, f64::min).sqrt())
        .fold(0.0, f64::max)
}

/// Pencil strokes kurbo's fit shot far past keep to their samples (#1741): a stroke that goes out
/// and back to the very point it left, nearly collinear samples, and strokes that turn back along
/// their own line. The first two are from fuzzing pencil-like strokes, where kurbo's curve went
/// about 390 and 160 units off; at whole-pixel coordinates the same shapes happened to fit well.
#[test]
fn simplifying_keeps_to_strokes_kurbo_shot_past() {
    let cases: [(&str, &[(f64, f64)]); 5] = [
        (
            "out and back to the same point",
            &[
                (86.043_565_719_086_97, -173.744_402_349_100_94),
                (87.851_850_739_335_35, -179.202_846_445_954_52),
                (86.043_565_719_086_97, -173.744_402_349_100_94),
            ],
        ),
        (
            "nearly collinear",
            &[
                (209.598_690_794_015_5, -204.909_328_119_739_88),
                (211.277_914_263_870_97, -207.190_974_020_753_4),
                (214.246_506_828_720_84, -211.228_827_590_623_24),
            ],
        ),
        (
            "a dip straight down and back up",
            &[(0.0, 0.0), (10.0, 0.0), (20.0, 2.0), (30.0, 6.0), (30.0, 12.0), (30.0, 6.0), (40.0, 2.0), (50.0, 0.0)],
        ),
        ("a slip back at the end", &[(0.0, 0.0), (10.0, 3.0), (20.0, 8.0), (30.0, 15.0), (30.0, 25.0), (30.0, 22.0)]),
        (
            "a near reversal in a curve",
            &[(0.0, 0.0), (8.0, 2.0), (16.0, 6.0), (24.0, 12.0), (32.0, 20.0), (24.0, 12.000_01), (16.0, 6.0), (8.0, 2.0)],
        ),
    ];
    for (what, pts) in cases {
        keeps_to_one_stroke(what, &polyline(pts));
    }
}

/// Accuracies the simplification is checked at: fine, the Pencil's, and coarse.
const ACCURACIES: [f64; 3] = [0.1, 0.75, 8.0];

/// Check that `input` (one open subpath), simplified at each of [`ACCURACIES`], strays no more
/// than twice the accuracy from it and leaves no part of it further off than that, stays in its
/// box grown by as much, and is still one subpath from its start to its end.
fn keeps_to_one_stroke(what: &str, input: &BezPath) {
    for accuracy in ACCURACIES {
        let out = simplify_path(input, accuracy);
        let (svg, limit) = (out.to_svg(), 2.0 * accuracy);
        let d = deviation(&out, input);
        assert!(d <= limit, "{what} at {accuracy}: strays {d} from the input: {svg}");
        let left = deviation(input, &out);
        assert!(left <= limit, "{what} at {accuracy}: leaves the input {left} off: {svg}");
        let (inb, outb) = (input.bounding_box(), out.bounding_box());
        assert!(inb.inflate(limit, limit).contains_rect(outb), "{what} at {accuracy}: {outb:?} past {inb:?}");
        let moves = out.elements().iter().filter(|e| matches!(e, kurbo::PathEl::MoveTo(_))).count();
        assert_eq!(moves, 1, "{what} at {accuracy}: {svg}");
        // From its start to its end: kurbo evaluates a fitted end on the input, so it is off by
        // rounding at most.
        let ends = |p: &BezPath| p.segments().next().map(|s| s.start()).zip(p.segments().last().map(|s| s.end()));
        let ((start, end), (start_in, end_in)) = (ends(&out).unwrap(), ends(input).unwrap());
        assert!((start - start_in).hypot() < 1e-9 && (end - end_in).hypot() < 1e-9, "{what} at {accuracy}: {svg}");
    }
}

/// Strokes made of curves keep to them as polylines do (#1741): a quadratic or a cubic that goes
/// out and comes straight back, one whose control points sit on its ends (so its direction is
/// read from the control point after them), and nearly collinear curves.
#[test]
fn simplifying_keeps_to_curves_that_turn_back_or_run_straight() {
    let (p, q, r) = (
        (209.598_690_794_015_5, -204.909_328_119_739_88),
        (211.277_914_263_870_97, -207.190_974_020_753_4),
        (214.246_506_828_720_84, -211.228_827_590_623_24),
    );
    // The nearly collinear samples, each line as a cubic with its handles a third of the way along.
    let third = |a: (f64, f64), b: (f64, f64), t: f64| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    let mut straight_cubics = BezPath::new();
    straight_cubics.move_to(p);
    for (a, b) in [(p, q), (q, r)] {
        straight_cubics.curve_to(third(a, b, 1.0 / 3.0), third(a, b, 2.0 / 3.0), b);
    }
    let mut straight_quads = BezPath::new();
    straight_quads.move_to(p);
    straight_quads.quad_to(third(p, q, 0.5), q);
    straight_quads.quad_to(third(q, r, 0.5), r);
    let cases = [
        ("a quadratic out and back", BezPath::from_svg("M0 0 Q15 10 30 0 Q15 10 0 0 Q-15 10 -30 0").unwrap()),
        ("a cubic out and back", BezPath::from_svg("M0 0 C10 8 20 8 30 0 C20 8 10 8 0 0 C-10 8 -20 8 -30 0").unwrap()),
        ("handles on the ends", BezPath::from_svg("M0 0 C0 0 30 4 30 4 C30 4 0 0 0 0 L-30 -4").unwrap()),
        ("a cubic dip down and back up", BezPath::from_svg("M0 0 L10 0 C15 0 20 5 20 12 C20 5 25 0 30 0 L40 0").unwrap()),
        ("nearly collinear cubics", straight_cubics),
        ("nearly collinear quadratics", straight_quads),
    ];
    for (what, input) in cases {
        keeps_to_one_stroke(what, &input);
    }
}

/// `sides` simplified by kurbo one at a time and joined into one subpath.
fn fitted_apart(sides: &[&BezPath], accuracy: f64) -> BezPath {
    let mut out = BezPath::new();
    for side in sides {
        let fit = kurbo::simplify::simplify_bezpath(side.iter(), accuracy, &Default::default());
        let skip = usize::from(!out.elements().is_empty());
        out.extend(fit.elements().iter().copied().skip(skip));
    }
    out
}

/// `curve` cut into `n` cubics, drawn on from where `path` is: forwards, or back from its end.
fn in_pieces(path: &mut BezPath, curve: kurbo::CubicBez, n: u32, back: bool) {
    use kurbo::ParamCurve;
    for i in 0..n {
        let i = if back { n - 1 - i } else { i };
        let c = curve.subsegment(f64::from(i) / f64::from(n)..f64::from(i + 1) / f64::from(n));
        if back { path.curve_to(c.p2, c.p1, c.p0) } else { path.curve_to(c.p1, c.p2, c.p3) }
    }
}

/// Where a path turns straight back it is cut, so the result is kurbo's fit of each side on its
/// own, with a point of the path at the turn. Fitted as one, kurbo rounds the turn off: its curve
/// passes the stray check, but falls short of the turn or takes more segments. A cubic whose
/// control points sit on its ends turns back as a line does.
#[test]
fn simplifying_fits_each_side_of_a_turn_back_on_its_own() {
    use kurbo::ParamCurve;
    let svg = |d: &str| BezPath::from_svg(d).unwrap();
    let s = kurbo::CubicBez::new((0.0, 0.0), (40.0, -60.0), (80.0, 60.0), (120.0, 0.0));
    let (mut s_out, mut s_back) = (BezPath::new(), BezPath::new());
    s_out.move_to(s.p0);
    in_pieces(&mut s_out, s, 8, false);
    s_back.move_to(s.p3);
    in_pieces(&mut s_back, s, 8, true);
    let cases = [
        ("lines", svg("M0 0 L10 0 L20 0 L30 0"), svg("M30 0 L20 0 L10 0")),
        ("handles on the ends", svg("M0 0 L10 0 L20 0 C20 0 30 0 30 0"), svg("M30 0 C30 0 20 0 20 0 L10 0")),
        ("a curve in pieces", s_out, s_back),
    ];
    for (what, out_side, back_side) in cases {
        let turn = out_side.segments().last().map(|seg| seg.end()).unwrap();
        let mut input = out_side.clone();
        input.extend(back_side.elements().iter().copied().skip(1));
        for accuracy in ACCURACIES {
            let ours = simplify_path(&input, accuracy);
            let apart = fitted_apart(&[&out_side, &back_side], accuracy);
            assert_eq!(ours.elements(), apart.elements(), "{what} at {accuracy}");
            assert!(ours.segments().any(|seg| (seg.end() - turn).hypot() < 1e-9), "{what} at {accuracy}: {}", ours.to_svg());
        }
    }
}

/// Where a path never turns back and kurbo's fit holds, the result is kurbo's own, element for
/// element: smooth runs, corners sharper than a right angle, closed subpaths and several subpaths.
/// A fit counts as holding up to twice the accuracy asked for, whatever that is: the large wave's
/// is further off at the coarse accuracy than the Pencil's limit of 1.5.
#[test]
fn simplifying_is_kurbos_where_its_fit_holds() {
    let wave: Vec<(f64, f64)> = (0..=60).map(|i| (f64::from(i) * 4.0, (f64::from(i) / 6.0).sin() * 30.0)).collect();
    let zigzag: Vec<(f64, f64)> = (0..=8).map(|i| (f64::from(i) * 10.0, if i % 2 == 0 { 0.0 } else { 40.0 })).collect();
    let mut closed = polyline(&wave[..20]);
    closed.close_path();
    let mut several = polyline(&wave);
    several.extend(polyline(&zigzag).elements().iter().map(|e| match *e {
        kurbo::PathEl::MoveTo(p) => kurbo::PathEl::MoveTo(p + Vec2::new(0.0, 100.0)),
        kurbo::PathEl::LineTo(p) => kurbo::PathEl::LineTo(p + Vec2::new(0.0, 100.0)),
        e => e,
    }));
    let curves = BezPath::from_svg("M0 0 C10 -20 30 -20 40 0 C50 20 70 20 80 0 Q100 -30 120 0 L140 10 Z M200 0 L240 0").unwrap();
    // A sine 300 high in 40 smooth cubics.
    let sine = |x: f64| ((x / 120.0).sin() * 300.0, (x / 120.0).cos() * 2.5);
    let mut large = BezPath::new();
    large.move_to((0.0, 0.0));
    for i in 0..40 {
        let (x0, x1) = (f64::from(i) * 40.0, f64::from(i + 1) * 40.0);
        let ((y0, d0), (y1, d1)) = (sine(x0), sine(x1));
        large.curve_to((x0 + 40.0 / 3.0, y0 + d0 * 40.0 / 3.0), (x1 - 40.0 / 3.0, y1 - d1 * 40.0 / 3.0), (x1, y1));
    }
    let coarse = simplify_path(&large, 8.0);
    assert!(coarse.elements().len() < 10 && (1.5..=8.0).contains(&deviation(&coarse, &large)), "{}", deviation(&coarse, &large));
    for (what, input) in [
        ("a wave", polyline(&wave)),
        ("a zigzag", polyline(&zigzag)),
        ("closed", closed),
        ("two subpaths", several),
        ("curves", curves),
        ("a large wave", large),
    ] {
        for accuracy in ACCURACIES {
            let ours = simplify_path(&input, accuracy);
            let theirs = kurbo::simplify::simplify_bezpath(input.iter(), accuracy, &Default::default());
            assert_eq!(ours.elements(), theirs.elements(), "{what} at {accuracy}");
        }
    }
}

/// A closed path with a reversal in it stays one closed subpath, and each subpath of several keeps
/// its own start.
#[test]
fn simplifying_keeps_closes_and_subpaths_across_a_reversal() {
    let mut input = polyline(&[(0.0, 0.0), (40.0, 0.0), (40.0, 30.0), (40.0, 20.0), (0.0, 30.0)]);
    input.close_path();
    input.extend(polyline(&[(100.0, 0.0), (100.0, 20.0), (100.0, 10.0)]).elements().iter().copied());
    let out = simplify_path(&input, 0.75);
    let els = out.elements();
    let moves: Vec<Point> = els.iter().filter_map(|e| if let kurbo::PathEl::MoveTo(p) = e { Some(*p) } else { None }).collect();
    assert_eq!(moves, [Point::new(0.0, 0.0), Point::new(100.0, 0.0)], "{}", out.to_svg());
    assert_eq!(els.iter().filter(|e| matches!(e, kurbo::PathEl::ClosePath)).count(), 1, "{}", out.to_svg());
    let close = els.iter().position(|e| matches!(e, kurbo::PathEl::ClosePath)).unwrap();
    assert!(matches!(els[close + 1], kurbo::PathEl::MoveTo(_)), "the first subpath closes before the second starts");
    assert!(deviation(&out, &input) <= 1.5, "{}", out.to_svg());
    // Nothing to draw: nothing out, not a stray move or close.
    let mut empty = BezPath::new();
    empty.move_to((5.0, 5.0));
    empty.line_to((5.0, 5.0));
    empty.close_path();
    assert!(simplify_path(&empty, 0.75).elements().is_empty());
    for d in ["", "M5 5", "M5 5 M6 6", "M5 5 Z"] {
        let input = if d.is_empty() { BezPath::new() } else { BezPath::from_svg(d).unwrap() };
        assert!(simplify_path(&input, 0.75).elements().is_empty(), "{d:?}");
    }
}

/// The subpaths simplified keep their starts: a move with nothing drawn from it between two
/// subpaths leaves no trace, and a segment drawn on from a close starts a subpath where the closed
/// one started, as it does in the path given.
#[test]
fn simplifying_keeps_where_each_subpath_starts() {
    let starts =
        |p: &BezPath| -> Vec<Point> { p.elements().iter().filter_map(|e| if let kurbo::PathEl::MoveTo(p) = e { Some(*p) } else { None }).collect() };
    let between = BezPath::from_svg("M0 0 L10 0 L20 5 M50 50 M100 0 L110 0 L120 5").unwrap();
    let out = simplify_path(&between, 0.75);
    assert_eq!(starts(&out), [Point::new(0.0, 0.0), Point::new(100.0, 0.0)], "{}", out.to_svg());
    assert!(deviation(&out, &between) <= 1.5, "{}", out.to_svg());
    // After the close, the pen is back at (0, 0): the next segments start there, also across a
    // reversal.
    use kurbo::PathEl::{ClosePath, LineTo, MoveTo};
    let on = BezPath::from_vec(vec![
        MoveTo(Point::new(0.0, 0.0)),
        LineTo(Point::new(10.0, 0.0)),
        LineTo(Point::new(10.0, 10.0)),
        ClosePath,
        LineTo(Point::new(20.0, 20.0)),
        LineTo(Point::new(30.0, 20.0)),
        LineTo(Point::new(20.0, 20.0)),
    ]);
    let out = simplify_path(&on, 0.75);
    let els = out.elements();
    let close = els.iter().position(|e| matches!(e, kurbo::PathEl::ClosePath)).unwrap();
    assert_eq!(els.get(close + 1), Some(&kurbo::PathEl::MoveTo(Point::new(0.0, 0.0))), "{}", out.to_svg());
    assert_eq!(starts(&out), [Point::new(0.0, 0.0), Point::new(0.0, 0.0)], "{}", out.to_svg());
    assert_eq!(out.segments().last().map(|s| kurbo::ParamCurve::end(&s)), Some(Point::new(20.0, 20.0)));
    assert!(deviation(&out, &on) <= 1.5, "{}", out.to_svg());
}
