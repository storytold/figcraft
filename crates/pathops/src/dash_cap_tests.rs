//! The Dash cap draws both ends of every dash, on Dash and Custom strokes alike. Unset, dashes keep
//! the end point caps they have always taken, so older files look the same.

use figcraft_doc::{StrokeAlign, StrokeCap, StrokeProps, WidthPreset};
use figcraft_geom::Point;
use figcraft_geom::kurbo::{BezPath, Shape};

use crate::{stroke_center, stroke_outline};

/// 200 long: a `10, 10` pattern fits it as it is, a half dash at each end.
fn line() -> BezPath {
    let mut p = BezPath::new();
    p.move_to((0.0, 0.0));
    p.line_to((200.0, 0.0));
    p
}

fn dashed(dash_cap: Option<StrokeCap>) -> StrokeProps {
    StrokeProps { weight: 4.0, dashes: vec![10.0, 10.0], dash_cap, align: StrokeAlign::Center, ..Default::default() }
}

fn area(props: &StrokeProps) -> f64 {
    stroke_center(&line(), 4.0, props).area().abs()
}

#[test]
fn the_cap_draws_both_ends_of_each_dash() {
    let dashes = 11.0; // Nine whole ones and the two halves at the ends.
    let band = 200.0 / 2.0 * 4.0;
    assert!((area(&dashed(Some(StrokeCap::None))) - band).abs() < 0.01);
    let square = area(&dashed(Some(StrokeCap::Square))) - band;
    assert!((square - dashes * 2.0 * 2.0 * 4.0).abs() < 0.01, "{square}");
    let round = area(&dashed(Some(StrokeCap::Round))) - band;
    assert!((round - dashes * std::f64::consts::PI * 4.0).abs() < 1.0, "{round}");
}

#[test]
fn unset_keeps_the_end_point_caps_and_set_overrides_them() {
    let caps = |dash_cap| StrokeProps { cap_start: StrokeCap::Round, cap_end: StrokeCap::Square, ..dashed(dash_cap) };
    let old = stroke_center(&line(), 4.0, &caps(None));
    assert!(old.area().abs() > area(&dashed(None)), "dashes still take them");
    assert_eq!(caps(None).band_caps(), [StrokeCap::Round, StrokeCap::Square]);
    assert_eq!(area(&caps(Some(StrokeCap::None))), area(&dashed(Some(StrokeCap::None))));
}

#[test]
fn a_solid_stroke_ignores_it() {
    let solid = |dash_cap| StrokeProps { dashes: vec![], cap_end: StrokeCap::Square, ..dashed(dash_cap) };
    assert_eq!(area(&solid(Some(StrokeCap::Round))), area(&solid(None)));
}

#[test]
fn zero_length_dashes_become_dots_of_the_cap() {
    let dots = |cap| StrokeProps { dashes: vec![0.0, 20.0], ..dashed(Some(cap)) };
    assert!(area(&dots(StrokeCap::Round)) > 100.0);
    assert!((area(&dots(StrokeCap::Square)) - 11.0 * 16.0).abs() < 0.1);
    assert!(area(&dots(StrokeCap::None)) < 1e-6);
}

#[test]
fn arrowheads_stay_on_the_path_ends() {
    let arrow = |dash_cap| StrokeProps { cap_end: StrokeCap::TriangleArrow, ..dashed(dash_cap) };
    let p = stroke_outline(&line(), &arrow(Some(StrokeCap::Round)), false);
    let plain = stroke_outline(&line(), &dashed(Some(StrokeCap::Round)), false);
    assert!(p.bounding_box().y1 > plain.bounding_box().y1, "the head is still drawn");
}

#[test]
fn a_variable_width_dash_takes_it_too() {
    let wide = |cap| StrokeProps { width_profile: WidthPreset::Eye.profile(), ..dashed(cap) };
    let (none, square) =
        (stroke_outline(&line(), &wide(Some(StrokeCap::None)), false), stroke_outline(&line(), &wide(Some(StrokeCap::Square)), false));
    // The dash at the middle ends at 105.
    let past = Point::new(106.0, 0.0);
    assert_eq!(none.winding(past), 0);
    assert_ne!(square.winding(past), 0);
}
