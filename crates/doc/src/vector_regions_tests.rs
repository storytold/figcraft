//! Regions as areas (`vector_regions.rs`): the inside test by each region's own fill rule, the last
//! region winning where regions overlap, curved edges, loops with segments stored backwards, a
//! region's points and empty or damaged networks.

use figcraft_geom::Point;

use crate::vector::{Region, Segment, VectorNetwork, Vertex, WindingRule};

/// Four points and four straight segments of a square at (`x0`, `y0`), clockwise on screen (y
/// down), appended to `n`. Returns the segment indexes in ring order.
fn square(n: &mut VectorNetwork, x0: f64, y0: f64, size: f64) -> Vec<usize> {
    let v0 = n.vertices.len();
    for (x, y) in [(x0, y0), (x0 + size, y0), (x0 + size, y0 + size), (x0, y0 + size)] {
        n.vertices.push(Vertex::new(x, y));
    }
    let s0 = n.segments.len();
    for k in 0..4 {
        n.segments.push(Segment::line(v0 + k, v0 + (k + 1) % 4));
    }
    (s0..s0 + 4).collect()
}

/// The 7.1 network: a 200 square (points 0..3, segments 0..3) with a 80 square inside it (points
/// 4..7, segments 4..7), both wound the same way, and no regions yet.
fn nested() -> VectorNetwork {
    let mut n = VectorNetwork::default();
    square(&mut n, 0.0, 0.0, 200.0);
    square(&mut n, 60.0, 60.0, 80.0);
    n
}

fn region(loops: Vec<Vec<usize>>, winding: WindingRule) -> Region {
    Region { loops, winding, fills: None, ..Default::default() }
}

#[test]
fn region_fills_default_to_inheritance_and_keep_explicit_empty() {
    let legacy: Region = serde_json::from_value(serde_json::json!({"loops": []})).unwrap();
    assert_eq!(legacy.fills, None);
    let empty = Region { fills: Some(Vec::new()), ..legacy };
    let value = serde_json::to_value(empty).unwrap();
    assert!(value["fills"].as_array().is_some_and(Vec::is_empty));
    let inherited = serde_json::to_value(region(Vec::new(), WindingRule::NonZero)).unwrap();
    assert!(inherited.get("fills").is_none());
}

#[test]
fn region_paint_space_roundtrips_and_rejects_nonfinite_inverse() {
    use crate::RegionPaintSpace;
    use figcraft_geom::Affine;
    let mut r = region(Vec::new(), WindingRule::EvenOdd);
    assert_eq!(r.paint_frame(100.0, 40.0), (100.0, 40.0, Affine::IDENTITY));
    r.paint_space = Some(RegionPaintSpace { width: 30.0, height: 20.0, transform: Affine::translate((4.0, 8.0)) });
    let decoded: Region = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
    assert_eq!(decoded, r);
    assert_eq!(decoded.paint_frame(100.0, 40.0), (30.0, 20.0, Affine::translate((4.0, 8.0))));
    r.paint_space.as_mut().unwrap().transform = Affine::scale_non_uniform(1e-320, 1.0);
    assert_eq!(r.paint_frame(100.0, 40.0), (100.0, 40.0, Affine::IDENTITY));
}

fn p(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

#[test]
fn the_inside_test_follows_each_regions_own_fill_rule() {
    let mut n = nested();
    n.regions = vec![region(vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]], WindingRule::NonZero)];
    // 1. Non-zero: the centre counts (both rings wound the same way).
    assert_eq!(n.region_at(p(100.0, 100.0)), Some(0));
    // 4. Outside.
    assert_eq!(n.region_at(p(300.0, 100.0)), None);
    n.regions[0].winding = WindingRule::EvenOdd;
    // 2. Even-odd: the centre is a hole; 3. the ring is inside.
    assert_eq!(n.region_at(p(100.0, 100.0)), None);
    assert_eq!(n.region_at(p(20.0, 100.0)), Some(0));
    assert_eq!(n.region_at(p(300.0, 100.0)), None);
    // 5. The inner ring wound the other way, non-zero: a hole.
    let mut r = nested();
    for s in &mut r.segments[4..8] {
        std::mem::swap(&mut s.a, &mut s.b);
    }
    r.regions = vec![region(vec![vec![0, 1, 2, 3], vec![7, 6, 5, 4]], WindingRule::NonZero)];
    assert_eq!(r.region_at(p(100.0, 100.0)), None);
    assert_eq!(r.region_at(p(20.0, 100.0)), Some(0));
    // Regions of one path may use different rules.
    let mut m = nested();
    m.regions = vec![
        region(vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]], WindingRule::NonZero),
        region(vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]], WindingRule::EvenOdd),
    ];
    assert_eq!(m.region_at(p(100.0, 100.0)), Some(0), "the even-odd region has a hole there, the non-zero one doesn't");
    assert_eq!(m.region_at(p(20.0, 100.0)), Some(1));
}

#[test]
fn the_last_overlapping_region_wins() {
    let mut n = nested();
    n.regions = vec![region(vec![vec![0, 1, 2, 3]], WindingRule::NonZero), region(vec![vec![4, 5, 6, 7]], WindingRule::NonZero)];
    // 6, 7.
    assert_eq!(n.region_at(p(100.0, 100.0)), Some(1));
    assert_eq!(n.region_at(p(20.0, 100.0)), Some(0));
    // 8. Swapped: the outer square (region 0 before, listed last now) wins in the middle.
    n.regions.swap(0, 1);
    assert_eq!(n.region_at(p(100.0, 100.0)), Some(1));
    assert_eq!(n.region_points(1), vec![0, 1, 2, 3], "index 1 is the outer square now");
    assert_eq!(n.region_at(p(20.0, 100.0)), Some(1));
}

#[test]
fn curved_edges_count_where_the_curve_runs() {
    let mut n = VectorNetwork {
        vertices: vec![Vertex::new(0.0, 0.0), Vertex::new(120.0, 0.0)],
        segments: vec![Segment { a: 0, b: 1, ta: (0.0, -80.0), tb: (0.0, -80.0) }, Segment::line(1, 0)],
        regions: vec![region(vec![vec![0, 1]], WindingRule::NonZero)],
    };
    // 9. Inside the bulge, above the chord; 10. below the chord; 11. above the curve's top (-60).
    assert_eq!(n.region_at(p(60.0, -40.0)), Some(0));
    assert_eq!(n.region_at(p(60.0, 20.0)), None);
    assert_eq!(n.region_at(p(60.0, -70.0)), None);
    assert_eq!(n.region_at(p(60.0, -58.0)), Some(0), "just under the top of the curve");
    // The lens walks toward the point it shares with the second segment, whichever way that is.
    n.segments.swap(0, 1);
    n.regions = vec![region(vec![vec![0, 1]], WindingRule::NonZero)];
    assert_eq!(n.region_at(p(60.0, -40.0)), Some(0));
    assert_eq!(n.region_at(p(60.0, -70.0)), None);
}

#[test]
fn a_single_curved_segment_loop_encloses_its_area() {
    let n = VectorNetwork {
        vertices: vec![Vertex::new(0.0, 0.0)],
        segments: vec![Segment { a: 0, b: 0, ta: (100.0, -100.0), tb: (-100.0, -100.0) }],
        regions: vec![region(vec![vec![0]], WindingRule::NonZero)],
    };
    assert_eq!(n.region_at(p(0.0, -40.0)), Some(0));
    assert_eq!(n.region_at(p(0.0, 40.0)), None);
}

#[test]
fn segments_stored_backwards_are_walked_the_right_way() {
    let mut n = VectorNetwork::default();
    square(&mut n, 0.0, 0.0, 200.0);
    // Segment 1 stored as 2 → 1 and segment 3 as 0 → 3.
    n.segments[1] = Segment::line(2, 1);
    n.segments[3] = Segment::line(0, 3);
    n.regions = vec![region(vec![vec![0, 1, 2, 3]], WindingRule::NonZero)];
    // 12, 13.
    assert_eq!(n.region_at(p(100.0, 100.0)), Some(0));
    assert_eq!(n.region_at(p(300.0, 100.0)), None);
    let walked = n.loop_walk(&[0, 1, 2, 3]);
    assert_eq!(walked, vec![(0, false), (1, true), (2, false), (3, true)]);
    // 14. The first segment reversed too: unchanged.
    n.segments[0] = Segment::line(1, 0);
    assert_eq!(n.region_at(p(100.0, 100.0)), Some(0));
    assert_eq!(n.region_at(p(300.0, 100.0)), None);
    // The walk equals the same loop stored consistently: one closed contour of the same area.
    use figcraft_geom::Shape;
    let mut consistent = VectorNetwork::default();
    square(&mut consistent, 0.0, 0.0, 200.0);
    consistent.regions = n.regions.clone();
    let (a, b) = (n.region_path(0).unwrap().area().abs(), consistent.region_path(0).unwrap().area().abs());
    assert!((a - 40_000.0).abs() < 1e-6 && (b - 40_000.0).abs() < 1e-6, "{a} {b}");
}

#[test]
fn a_loop_that_does_not_close_is_closed_implicitly() {
    let mut n = VectorNetwork::default();
    square(&mut n, 0.0, 0.0, 200.0);
    n.regions = vec![region(vec![vec![0, 1, 2]], WindingRule::NonZero)];
    // The walk ends at point 3 and closes straight back to point 0.
    assert_eq!(n.region_at(p(100.0, 100.0)), Some(0));
    assert_eq!(n.region_at(p(300.0, 100.0)), None);
}

#[test]
fn missing_segments_are_skipped_wherever_they_sit_in_a_loop() {
    use figcraft_geom::Shape;
    let mut n = VectorNetwork::default();
    square(&mut n, 0.0, 0.0, 200.0);
    n.regions = vec![region(vec![vec![0, 1, 2]], WindingRule::NonZero)];
    let control = n.region_path(0).unwrap();
    let control_walk = n.loop_walk(&[0, 1, 2]);
    for lp in [vec![99, 0, 1, 2], vec![0, 99, 1, 2], vec![99, 98, 0, 1, 2]] {
        n.regions = vec![region(vec![lp.clone()], WindingRule::NonZero)];
        assert_eq!(n.loop_walk(&lp), control_walk, "{lp:?}");
        assert_eq!(n.region_at(p(100.0, 100.0)), Some(0), "{lp:?}");
        assert!(n.region_contains(0, p(100.0, 100.0)), "{lp:?}");
        let path = n.region_path(0).unwrap();
        assert_eq!(path.elements().len(), control.elements().len(), "{lp:?}");
        assert!((path.area().abs() - control.area().abs()).abs() < 1e-6, "{lp:?}");
    }
    n.regions = vec![region(vec![vec![99]], WindingRule::NonZero)];
    assert!(n.region_path(0).is_none());
    assert!(!n.region_contains(0, p(100.0, 100.0)));
}

#[test]
fn a_regions_points_are_listed_once() {
    let mut n = nested();
    n.regions = vec![region(vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]], WindingRule::NonZero)];
    // 15.
    assert_eq!(n.region_points(0), vec![0, 1, 2, 3, 4, 5, 6, 7]);
    // 16.
    n.regions = vec![region(vec![vec![0, 1, 2, 3]], WindingRule::NonZero), region(vec![vec![4, 5, 6, 7]], WindingRule::NonZero)];
    assert_eq!(n.region_points(1), vec![4, 5, 6, 7]);
    assert!(n.region_points(9).is_empty());
    // 17. A missing segment is ignored.
    n.regions = vec![region(vec![vec![0, 1, 99, 2, 3]], WindingRule::NonZero)];
    assert_eq!(n.region_points(0), vec![0, 1, 2, 3]);
    assert_eq!(n.region_at(p(100.0, 100.0)), Some(0));
}

#[test]
fn empty_and_damaged_networks_have_no_regions_and_never_panic() {
    // 18.
    let mut n = nested();
    assert_eq!(n.region_at(p(100.0, 100.0)), None);
    let empty = VectorNetwork::default();
    assert_eq!(empty.region_at(p(0.0, 0.0)), None);
    assert!(empty.region_path(0).is_none());
    assert!(empty.region_points(0).is_empty());
    assert_eq!(empty.region_winding(0, p(0.0, 0.0)), None);
    // Loops naming missing segments, segments naming missing points, empty loops, junk points.
    n.regions = vec![region(vec![vec![], vec![42, 43]], WindingRule::EvenOdd), region(vec![vec![0, 1, 2, 3]], WindingRule::NonZero)];
    n.segments.push(Segment::line(1, 500));
    n.regions.push(region(vec![vec![8, 8, 8]], WindingRule::NonZero));
    assert_eq!(n.region_at(p(100.0, 100.0)), Some(1));
    assert!(n.region_path(0).is_none());
    assert!(n.region_path(2).is_none());
    for q in [p(f64::NAN, 0.0), p(f64::INFINITY, 1.0), p(1e300, -1e300)] {
        let _ = n.region_at(q);
    }
}

#[test]
fn region_points_leave_out_segment_ends_that_do_not_exist() {
    let mut n = VectorNetwork::default();
    let ring = square(&mut n, 0.0, 0.0, 100.0);
    n.regions = vec![region(vec![ring], WindingRule::NonZero)];
    assert_eq!(n.region_points(0), vec![0, 1, 2, 3]);
    n.segments[3] = Segment::line(3, 9);
    assert_eq!(n.region_points(0), vec![0, 1, 2, 3]);
    assert!(n.region_path(0).is_some());
    n.regions = vec![region(vec![vec![99]], WindingRule::NonZero)];
    assert!(n.region_points(0).is_empty());
}

#[test]
fn the_topology_key_ignores_moved_points_but_not_new_segments() {
    let mut n = nested();
    n.regions = vec![region(vec![vec![0, 1, 2, 3]], WindingRule::NonZero)];
    let k = n.topology_key();
    n.translate(15.0, 25.0);
    n.segments[0].ta = (5.0, 5.0);
    assert_eq!(n.topology_key(), k, "moving points and handles keeps it");
    n.regions.push(region(vec![vec![4, 5, 6, 7]], WindingRule::NonZero));
    assert_ne!(n.topology_key(), k);
    let k = n.topology_key();
    n.regions.swap(0, 1);
    assert_ne!(n.topology_key(), k, "reordered regions");
    let k = n.topology_key();
    n.segments.push(Segment::line(0, 2));
    assert_ne!(n.topology_key(), k);
}
