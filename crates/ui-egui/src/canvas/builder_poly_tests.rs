//! The shape builder's region outlines: repeats dropped and sub-pixel slivers left out, so the
//! tessellator never sees a zero-length segment at a hairpin (that draws a spike across the canvas).

use super::clean_poly;
use egui::pos2;

#[test]
fn a_contour_loses_the_point_it_closes_back_to() {
    // A square whose flattened contour ends where it started, as kurbo's flatten emits it.
    let sq = vec![pos2(0.0, 0.0), pos2(10.0, 0.0), pos2(10.0, 10.0), pos2(0.0, 10.0), pos2(0.0, 0.0)];
    let out = clean_poly(sq).expect("a square is drawn");
    assert_eq!(out, vec![pos2(0.0, 0.0), pos2(10.0, 0.0), pos2(10.0, 10.0), pos2(0.0, 10.0)]);
}

#[test]
fn repeated_points_and_stray_values_are_dropped() {
    // Triplicated points at the corners are what makes the tessellator divide by a zero normal.
    let poly = vec![pos2(0.0, 0.0), pos2(0.0, 0.0), pos2(0.0, 0.0), pos2(20.0, 0.0), pos2(f32::NAN, 5.0), pos2(20.0, 20.0), pos2(0.0, 20.0)];
    let out = clean_poly(poly).expect("a square is drawn");
    assert_eq!(out, vec![pos2(0.0, 0.0), pos2(20.0, 0.0), pos2(20.0, 20.0), pos2(0.0, 20.0)]);
}

#[test]
fn a_sliver_a_fraction_of_a_pixel_thin_is_left_out() {
    // A sliver region that doubles back on itself: 100 px long, 0.002 px thick, closing back.
    let sliver = vec![pos2(0.0, 0.0), pos2(100.0, 0.0), pos2(100.0, 0.002), pos2(0.0, 0.0)];
    assert!(clean_poly(sliver).is_none());
    // Two points (a doubled-back line without thickness) and a single point aren't areas either.
    assert!(clean_poly(vec![pos2(0.0, 0.0), pos2(10.0, 0.0)]).is_none());
    assert!(clean_poly(vec![pos2(3.0, 4.0)]).is_none());
}

#[test]
fn a_thin_but_visible_region_is_kept() {
    // 2 px wide and 100 px long: thin, but a real area.
    let strip = vec![pos2(0.0, 0.0), pos2(100.0, 0.0), pos2(100.0, 2.0), pos2(0.0, 2.0)];
    let out = clean_poly(strip).expect("a 2 px strip is drawn");
    assert_eq!(out.len(), 4);
}
