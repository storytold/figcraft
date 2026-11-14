//! How far a rounded point's arc may reach along its segments (#2334): the whole segment when the
//! segment's other end stays sharp, half of it when that end is rounded too. Figma draws a single
//! rounded point of a 100 × 100 square with radius 100 as an arc from one neighbour to the other.

use figcraft_geom::kurbo::{CubicBez, ParamCurve, PathSeg};
use figcraft_geom::{BezPath, Point};

use crate::vector::VectorNetwork;

/// A 100 × 100 square's points: top left, top right, bottom right, bottom left.
const SQUARE: [Point; 4] = [Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0)];

/// A network through `pts` (closed or open) whose points have radii `radii`.
fn network(pts: &[Point], closed: bool, radii: &[f64]) -> VectorNetwork {
    let mut n = VectorNetwork::from_points(pts, closed);
    for (v, r) in n.vertices.iter_mut().zip(radii) {
        v.radius = *r;
    }
    n
}

/// The cubic curves of `path` (the rounded points' arcs; every other segment is straight).
fn curves(path: &BezPath) -> Vec<CubicBez> {
    path.segments()
        .filter_map(|s| match s {
            PathSeg::Cubic(c) => Some(c),
            _ => None,
        })
        .collect()
}

/// Whether `a` and `b` are the same point to within rounding.
fn near(a: Point, b: Point) -> bool {
    (a - b).hypot() < 1e-9
}

/// Checks that `path` draws, for each `(corner, prev, next, t)` of `want`, a circular arc
/// between the points `t` from `corner` toward `prev` and toward `next` (a right angle, so the
/// circle's radius is `t`), and no other curve.
fn assert_arcs(path: &BezPath, want: &[(Point, Point, Point, f64)], why: &str) {
    let got = curves(path);
    assert_eq!(got.len(), want.len(), "{why}: {got:?}");
    for &(corner, prev, next, t) in want {
        let toward = |p: Point| corner + (p - corner) / (p - corner).hypot() * t;
        let (a, b) = (toward(prev), toward(next));
        let center = a + (b - corner);
        let mid = center + (corner - center) / (corner - center).hypot() * t;
        // Two arcs can share both ends (opposite corners of a square), so the middle picks one.
        let ends = |c: &&CubicBez| (near(c.p0, a) && near(c.p3, b)) || (near(c.p0, b) && near(c.p3, a));
        let passing: Vec<Point> = got.iter().filter(ends).map(|c| c.eval(0.5)).collect();
        assert!(
            passing.iter().any(|&m| near(m, mid)),
            "{why}: no arc at {corner:?} from {a:?} to {b:?} through {mid:?}; arcs between them pass {passing:?}"
        );
    }
}

#[test]
fn a_rounded_points_arc_reaches_as_far_as_its_segments_other_ends_allow() {
    let [tl, tr, br, bl] = SQUARE;
    // (radii of [top left, top right, bottom right, bottom left], each arc as (point, its
    // neighbours, how far it reaches along both), derivation)
    #[allow(clippy::type_complexity)]
    let rows: Vec<([f64; 4], Vec<(Point, Point, Point, f64)>, &str)> = vec![
        ([0.0, 0.0, 100.0, 0.0], vec![(br, tr, bl, 100.0)], "the issue's square: the arc runs from one sharp neighbour to the other"),
        ([0.0, 0.0, 500.0, 0.0], vec![(br, tr, bl, 100.0)], "a larger radius stops at the sharp neighbours"),
        ([0.0, 0.0, 75.0, 0.0], vec![(br, tr, bl, 75.0)], "a radius past half the segments is drawn whole"),
        ([0.0, 0.0, 40.0, 0.0], vec![(br, tr, bl, 40.0)], "a radius that fits half the segments is unchanged"),
        (
            [0.0, 10.0, 100.0, 0.0],
            vec![(tr, tl, br, 10.0), (br, tr, bl, 50.0)],
            "the segment to the rounded top right is shared, half each, and the shorter room wins",
        ),
        (
            [100.0, 0.0, 100.0, 0.0],
            vec![(tl, bl, tr, 100.0), (br, tr, bl, 100.0)],
            "opposite points share no segment: each reaches its sharp neighbours (a lens)",
        ),
        (
            [100.0; 4],
            vec![(tl, bl, tr, 50.0), (tr, tl, br, 50.0), (br, tr, bl, 50.0), (bl, br, tl, 50.0)],
            "every point rounded: a circle, as before",
        ),
    ];
    for (radii, want, why) in rows {
        let net = network(&SQUARE, true, &radii);
        assert_arcs(&net.fill_path(), &want, why);
        assert_arcs(&net.stroke_path(), &want, why);
        // The stored radii stay as they were set.
        assert_eq!(net.vertices.iter().map(|p| p.radius).collect::<Vec<_>>(), radii.to_vec(), "{why}");
    }
}

#[test]
fn an_open_paths_ends_leave_its_rounded_point_the_whole_segments() {
    let pts = [Point::new(0.0, 0.0), Point::new(0.0, 100.0), Point::new(100.0, 100.0)];
    // (radii, derivation): the ends don't turn, so a radius on them rounds nothing and takes no
    // room from the middle point.
    let rows: Vec<([f64; 3], &str)> =
        vec![([0.0, 100.0, 0.0], "sharp ends"), ([100.0, 100.0, 100.0], "ends with a radius of their own stay sharp and leave the room to it")];
    for (radii, why) in rows {
        let net = network(&pts, false, &radii);
        assert_arcs(&net.stroke_path(), &[(pts[1], pts[0], pts[2], 100.0)], why);
    }
}

#[test]
fn a_smoothed_corner_takes_the_room_of_a_sharp_neighbour_before_losing_smoothing() {
    // Radius 40 with smoothing 0.6 eases in from 1.6 × 40 = 64 along each side. Half of a 100
    // side (50) would cut smoothing down; the whole side next to sharp points leaves all of it.
    let net = network(&SQUARE, true, &[0.0, 0.0, 40.0, 0.0]);
    let path = net.rounded_smoothed(0.6).unwrap().fill_path();
    let ends: Vec<Point> = curves(&path).iter().flat_map(|c| [c.p0, c.p3]).collect();
    for p in [Point::new(100.0, 36.0), Point::new(36.0, 100.0)] {
        assert!(ends.iter().any(|e| (*e - p).hypot() < 1e-6), "the smoothed corner starts at {p:?}: {ends:?}");
    }
    assert!(ends.iter().all(|e| e.x >= 36.0 - 1e-6 && e.y >= 36.0 - 1e-6), "and reaches no further: {ends:?}");
}
