//! Per-side stroke weights on rounded rectangles, drawn like CSS borders: square corners, a bar
//! along a rounded edge, tapers through corners, alignments, inner corners that no longer fit,
//! smoothing and degenerate weights.

use figcraft_doc::StrokeAlign;
use figcraft_geom::kurbo::{BezPath, Point, Shape};
use figcraft_geom::rounded_rect;

use crate::{rect_side_strokes, rounded_side_strokes};

const W: f64 = 120.0;
const H: f64 = 60.0;

/// The area of a ring whose hole runs against its outer edge.
fn area(p: &BezPath) -> f64 {
    p.area().abs()
}

fn painted(p: &BezPath, x: f64, y: f64) -> bool {
    p.winding(Point::new(x, y)) % 2 != 0
}

#[test]
fn square_corners_paint_what_rect_side_strokes_paints() {
    for align in [StrokeAlign::Inside, StrokeAlign::Center, StrokeAlign::Outside] {
        let sides = [1.0, 2.0, 3.0, 4.0];
        let a = area(&rounded_side_strokes(W, H, [0.0; 4], 0.0, sides, align));
        assert!((a - area(&rect_side_strokes(W, H, sides, align))).abs() < 1e-6, "{align:?}: {a}");
    }
}

/// `border-l-4 rounded-r-lg`.
#[test]
fn a_left_border_on_a_right_rounded_box_is_a_straight_bar() {
    let bar = rounded_side_strokes(W, H, [0.0, 8.0, 8.0, 0.0], 0.0, [0.0, 0.0, 0.0, 4.0], StrokeAlign::Inside);
    assert!((area(&bar) - 4.0 * H).abs() < 1e-6, "{}", area(&bar));
    assert!(painted(&bar, 2.0, 30.0));
    for (x, y) in [(119.9, 30.0), (117.0, 1.0), (60.0, 0.1), (60.0, 59.9), (4.1, 30.0)] {
        assert!(!painted(&bar, x, y), "({x}, {y})");
    }
}

#[test]
fn one_weight_on_every_side_is_the_uniform_ring() {
    let ring = rounded_side_strokes(W, H, [8.0; 4], 0.0, [2.0; 4], StrokeAlign::Inside);
    let want = area(&rounded_rect(W, H, [8.0; 4], 0.0)) - area(&rounded_rect(W - 4.0, H - 4.0, [6.0; 4], 0.0));
    assert!((area(&ring) - want).abs() < 1e-6, "{} vs {want}", area(&ring));
    assert_eq!(ring.winding(Point::new(60.0, 30.0)), 0, "a hole under non-zero too");
}

#[test]
fn center_and_outside_grow_the_box_and_its_corners() {
    for (align, out, inset) in [(StrokeAlign::Center, 1.0, 1.0), (StrokeAlign::Outside, 2.0, 0.0)] {
        let a = area(&rounded_side_strokes(W, H, [8.0; 4], 0.0, [2.0; 4], align));
        let outer = area(&rounded_rect(W + 2.0 * out, H + 2.0 * out, [8.0 + out; 4], 0.0));
        let hole = area(&rounded_rect(W - 2.0 * inset, H - 2.0 * inset, [8.0 - inset; 4], 0.0));
        assert!((a - (outer - hole)).abs() < 1e-6, "{align:?}: {a} vs {}", outer - hole);
    }
}

/// 8 × 7 inner ellipses at the bottom corners.
#[test]
fn a_bottom_border_tapers_into_the_bottom_corners() {
    let bottom = rounded_side_strokes(W, H, [8.0; 4], 0.0, [0.0, 0.0, 1.0, 0.0], StrokeAlign::Inside);
    assert!(painted(&bottom, 60.0, 59.5) && painted(&bottom, 8.0, 59.5) && painted(&bottom, 112.0, 59.5));
    for (x, y) in [(60.0, 0.5), (0.5, 30.0), (0.5, 52.0), (119.5, 52.0), (4.0, 4.0)] {
        assert!(!painted(&bottom, x, y), "({x}, {y})");
    }
}

#[test]
fn a_weight_past_the_radius_leaves_square_inner_corners() {
    let top = rounded_side_strokes(W, H, [8.0; 4], 0.0, [10.0, 0.0, 0.0, 2.0], StrokeAlign::Inside);
    assert!(!painted(&top, 2.2, 10.2) && !painted(&top, 119.8, 10.2));
    assert!(painted(&top, 60.0, 9.8) && painted(&top, 1.8, 30.0));
}

/// A weight past `w − R` leaves inner corners wider than the inner box: they shrink to fit, so the
/// hole stays inside the outer edge, across or down.
#[test]
fn inner_corners_shrink_to_fit_a_narrow_inner_box() {
    let wide = rounded_side_strokes(20.0, 20.0, [10.0; 4], 0.0, [0.0, 0.0, 0.0, 12.0], StrokeAlign::Inside);
    assert!(painted(&wide, 6.0, 10.0) && !painted(&wide, 16.0, 10.0));
    assert!(!painted(&wide, 17.5, 3.0) && !painted(&wide, 17.5, 17.0), "nothing past the outer edge");
    let tall = rounded_side_strokes(20.0, 20.0, [10.0; 4], 0.0, [12.0, 0.0, 0.0, 0.0], StrokeAlign::Inside);
    assert!(painted(&tall, 10.0, 6.0) && !painted(&tall, 10.0, 16.0));
    assert!(!painted(&tall, 3.0, 17.5) && !painted(&tall, 17.0, 17.5), "nothing past the outer edge");
    let pill = rounded_side_strokes(30.0, 20.0, [10.0; 4], 0.0, [0.0, 0.0, 0.0, 25.0], StrokeAlign::Inside);
    assert!(painted(&pill, 12.0, 10.0) && !painted(&pill, 27.0, 10.0));
    assert!(!painted(&pill, 24.0, 0.5) && !painted(&pill, 24.0, 19.5), "nothing past the outer edge");
}

#[test]
fn smoothed_corners_keep_the_stroke_inside_the_fill() {
    let smooth = rounded_side_strokes(W, H, [16.0; 4], 0.6, [0.0, 0.0, 2.0, 0.0], StrokeAlign::Inside);
    let fill = rounded_rect(W, H, [16.0; 4], 0.6);
    let grid = (0..1200).flat_map(|i| (0..600).map(move |j| Point::new(f64::from(i) * 0.1 + 0.05, f64::from(j) * 0.1 + 0.05)));
    let stray = grid.filter(|pt| painted(&smooth, pt.x, pt.y) && (pt.y < H / 2.0 || fill.winding(*pt) == 0));
    assert_eq!(stray.count(), 0, "painted outside the fill or along the top");
    assert!(painted(&smooth, 60.0, 59.5));
}

#[test]
fn center_and_outside_strokes_follow_smoothed_corners() {
    for (align, out, inset) in [(StrokeAlign::Center, 1.0, 1.0), (StrokeAlign::Outside, 2.0, 0.0)] {
        let a = area(&rounded_side_strokes(W, H, [16.0; 4], 0.6, [2.0; 4], align));
        let outer = area(&rounded_rect(W + 2.0 * out, H + 2.0 * out, [16.0 + out; 4], 0.6));
        let hole = area(&rounded_rect(W - 2.0 * inset, H - 2.0 * inset, [16.0 - inset; 4], 0.6));
        assert!((a - (outer - hole)).abs() < 1e-3, "{align:?}: {a} vs {}", outer - hole);
    }
}

#[test]
fn weights_wider_than_the_box_leave_the_outer_edge() {
    let full = rounded_side_strokes(20.0, 10.0, [4.0; 4], 0.0, [6.0; 4], StrokeAlign::Inside);
    assert!((area(&full) - area(&rounded_rect(20.0, 10.0, [4.0; 4], 0.0))).abs() < 1e-6);
}

#[test]
fn no_weight_paints_nothing() {
    assert!(rounded_side_strokes(W, H, [8.0; 4], 0.0, [0.0; 4], StrokeAlign::Inside).elements().is_empty());
    let junk = [f64::NAN, -4.0, f64::INFINITY, 0.0];
    assert!(rounded_side_strokes(W, H, [8.0; 4], 0.0, junk, StrokeAlign::Inside).elements().is_empty());
}
