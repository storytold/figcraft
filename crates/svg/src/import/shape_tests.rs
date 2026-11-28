//! Primitive recognition in path data, placement under transforms, dash offsets and layer names.

use figcraft_geom::{Affine, BezPath, Point, Rect};

use super::shape::{Prim, place, recognize};
use super::{dash_offset, layer_name};

fn path(d: &str) -> BezPath {
    BezPath::from_svg(d).unwrap()
}

#[test]
fn rectangles_are_found_with_their_corner_radius() {
    assert_eq!(recognize(&path("M10 20 H60 V50 H10 Z"), false), Some(Prim::Rect { rect: Rect::new(10.0, 20.0, 60.0, 50.0), radius: 0.0 }));
    // Any direction, any starting corner.
    assert!(matches!(recognize(&path("M60 50 V20 H10 V50 Z"), false), Some(Prim::Rect { radius: 0.0, .. })));
    // Rounded: corners as arcs (as cubics).
    let rounded = figcraft_geom::rounded_rect(80.0, 40.0, [8.0; 4], 0.0);
    let Some(Prim::Rect { rect, radius }) = recognize(&rounded, false) else { panic!("{:?}", recognize(&rounded, false)) };
    assert_eq!(rect, Rect::new(0.0, 0.0, 80.0, 40.0));
    assert_eq!(radius, 8.0);
    // A pill: fully rounded ends.
    let pill = figcraft_geom::rounded_rect(80.0, 20.0, [10.0; 4], 0.0);
    assert!(matches!(recognize(&pill, false), Some(Prim::Rect { radius: 10.0, .. })));
    // Back at the start without closing: a corner only when there's no stroke to cap the ends.
    assert!(matches!(recognize(&path("M0 0 H40 V40 H0 V0"), true), Some(Prim::Rect { .. })));
    assert_eq!(recognize(&path("M0 0 H40 V40 H0 V0"), false), None);
}

#[test]
fn circles_and_ellipses_are_found() {
    // Closed by ending where it starts.
    let e = figcraft_geom::ellipse(60.0, 30.0, 0.0, std::f64::consts::TAU, 0.0);
    let Some(Prim::Ellipse(r)) = recognize(&e, true) else { panic!("{:?}", recognize(&e, true)) };
    assert!((r.width() - 60.0).abs() < 1e-9 && (r.height() - 30.0).abs() < 1e-9 && r.x0.abs() < 1e-9, "{r:?}");
    // A circle is an ellipse, not a fully rounded square.
    let c = figcraft_geom::ellipse(40.0, 40.0, 0.0, std::f64::consts::TAU, 0.0);
    assert!(matches!(recognize(&c, true), Some(Prim::Ellipse(_))));
    // SVG arcs.
    assert!(matches!(recognize(&path("M0 20 A20 20 0 0 1 40 20 A20 20 0 0 1 0 20 Z"), false), Some(Prim::Ellipse(_))));
}

#[test]
fn other_outlines_are_not_primitives() {
    for d in [
        // A triangle, an open path, two subpaths.
        "M0 0 L40 0 L20 30 Z",
        "M0 0 H40 V40 H0",
        "M0 0 H40 V40 H0 Z M10 10 H30 V30 H10 Z",
        // Elliptical corners.
        "M10 0 H70 A10 5 0 0 1 80 5 V25 A10 5 0 0 1 70 30 H10 A10 5 0 0 1 0 25 V5 A10 5 0 0 1 10 0 Z",
        // Half a circle, a flat box, a square traced twice.
        "M0 20 A20 20 0 0 1 40 20 Z",
        "M0 0 H40 V0 H0 Z",
        "M0 0 L40 0 L40 40 L0 40 L0 0 L40 0 L40 40 L0 40 Z",
    ] {
        assert!(!matches!(recognize(&path(d), false), Some(Prim::Rect { .. } | Prim::Ellipse(_))), "{d}: {:?}", recognize(&path(d), false));
    }
}

#[test]
fn two_point_paths_are_lines() {
    assert_eq!(recognize(&path("M1 2 L30 40"), false), Some(Prim::Line(Point::new(1.0, 2.0), Point::new(30.0, 40.0))));
    assert_eq!(recognize(&path("M1 2 L1 2"), false), None);
    assert_eq!(recognize(&path("M0 0 L10 0 L20 0"), false), None);
}

#[test]
fn placement_keeps_rotation_and_flips_but_not_skew() {
    let world = Affine::rotate(0.5) * Affine::scale_non_uniform(2.0, 3.0);
    let (t, w, h) = place(world, Point::new(1.0, 1.0), 10.0, 10.0).unwrap();
    assert!((w - 20.0).abs() < 1e-9 && (h - 30.0).abs() < 1e-9);
    let c = t.as_coeffs();
    assert!((c[1].atan2(c[0]) - 0.5).abs() < 1e-9 && (c[0] * c[0] + c[1] * c[1] - 1.0).abs() < 1e-9);
    // The box's origin lands where the transform puts it.
    assert!((t * Point::ZERO - world * Point::new(1.0, 1.0)).hypot() < 1e-9);
    assert!(place(Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 10.0]), Point::ZERO, 5.0, 5.0).is_some());
    assert!(place(Affine::skew(0.5, 0.0), Point::ZERO, 5.0, 5.0).is_none());
    assert!(place(Affine::scale(0.0), Point::ZERO, 5.0, 5.0).is_none());
    assert!(place(Affine::translate((f64::NAN, 0.0)), Point::ZERO, 5.0, 5.0).is_none());
}

#[test]
fn dash_offsets_rotate_the_pattern() {
    assert_eq!(dash_offset(&[20.0, 10.0], 0.0, true), Some(vec![20.0, 10.0]));
    assert_eq!(dash_offset(&[20.0, 10.0], 30.0, true), Some(vec![20.0, 10.0]));
    // Into the dash.
    assert_eq!(dash_offset(&[20.0, 10.0], 5.0, false), Some(vec![15.0, 10.0, 5.0, 0.0]));
    // Into the gap: a zero-length dash first, which only butt caps draw as nothing.
    assert_eq!(dash_offset(&[20.0, 10.0], 25.0, true), Some(vec![0.0, 5.0, 20.0, 5.0]));
    assert_eq!(dash_offset(&[20.0, 10.0], 25.0, false), None);
    // Backwards.
    assert_eq!(dash_offset(&[20.0, 10.0], -5.0, true), Some(vec![0.0, 5.0, 20.0, 5.0]));
    for bad in [f64::NAN, f64::INFINITY] {
        assert_eq!(dash_offset(&[20.0, 10.0], bad, true), None);
    }
    assert_eq!(dash_offset(&[0.0, 0.0], 3.0, true), None);
}

#[test]
fn layer_names_decode_id_escapes() {
    assert_eq!(layer_name("My_x20_Layer", "Group"), "My Layer");
    assert_eq!(layer_name("a_x2C__x20_b", "Group"), "a, b");
    assert_eq!(layer_name("bad_xZZ_escape", "Group"), "bad_xZZ_escape");
    assert_eq!(layer_name("tail_x2", "Group"), "tail_x2");
    assert_eq!(layer_name("ctl_x07_", "Group"), "ctl_x07_");
    assert_eq!(layer_name("", "Group"), "Group");
    assert_eq!(layer_name("é_x41_", "Group"), "éA");
}

#[test]
fn doctype_with_internal_subset_is_still_svg() {
    let s = "<?xml version=\"1.0\"?>\n<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\" [\n\t<!ENTITY ns_svg \"http://www.w3.org/2000/svg\">\n]>\n<svg xmlns=\"&ns_svg;\" width=\"10\" height=\"10\"/>";
    assert!(super::looks_like_svg(s));
    let quoted = "<!DOCTYPE svg [ <!ENTITY a \"x>y]\"> ]><svg/>";
    assert!(super::looks_like_svg(quoted));
    let commented = "<!DOCTYPE svg [ <!-- don't ] > --> <!ENTITY a \"b\"> ]><svg/>";
    assert!(super::looks_like_svg(commented));
    assert!(!super::looks_like_svg("<!DOCTYPE html [ <svg/> ]><html/>"));
}
