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
    let _ = ellipse(f64::NAN, 10.0, f64::NAN, 100.0, 2.0);
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
