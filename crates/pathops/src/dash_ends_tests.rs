//! A dashed line starts and ends with a half-length dash, whatever its pattern: a whole number of
//! patterns fits the line, dashes and gaps scaled together. A closed path has no ends and runs its
//! pattern as it is. The numbers in [`the_public_example_comes_out_the_same`] are from a public
//! explanation of how the fit works.

use figcraft_doc::{StrokeAlign, StrokeProps, WidthProfile};
use figcraft_geom::Point;
use figcraft_geom::kurbo::{BezPath, Rect, Shape};

use crate::width::{dash_pattern, line_dashes, subpath_dashes};
use crate::{stroke_center, stroke_outline};

fn line(len: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((0.0, 0.0));
    p.line_to((len, 0.0));
    p
}

fn dashed(dashes: &[f64]) -> StrokeProps {
    StrokeProps { weight: 2.0, dashes: dashes.to_vec(), align: StrokeAlign::Center, ..Default::default() }
}

fn covers(p: &BezPath, x: f64) -> bool {
    p.winding(Point::new(x, 0.0)) != 0
}

/// Which of `xs` are on a dash.
fn on(p: &BezPath, xs: &[f64]) -> Vec<bool> {
    xs.iter().map(|x| covers(p, *x)).collect()
}

#[test]
fn a_line_starts_and_ends_with_half_a_dash() {
    // 10 on, 10 off along 100: 5 on, 10 off, 10 on, … 10 off, 5 on.
    let s = stroke_center(&line(100.0), 2.0, &dashed(&[10.0, 10.0]));
    assert_eq!(
        on(&s, &[2.0, 7.0, 13.0, 16.0, 24.0, 26.0, 84.0, 86.0, 94.0, 98.0]),
        [true, false, false, true, true, false, true, false, false, true]
    );
    assert!((s.area().abs() - 100.0).abs() < 0.01, "half of it is dashes: {}", s.area());
}

#[test]
fn a_whole_number_of_patterns_fits_the_line() {
    // 107 is 5 patterns and a bit: scaled to 21.4 each, so it still ends on half a dash.
    let (pattern, offset) = line_dashes(107.0, false, &[10.0, 10.0]);
    assert!((pattern[0] - 10.7).abs() < 1e-9 && (pattern[1] - 10.7).abs() < 1e-9, "{pattern:?}");
    assert!((offset - 5.35).abs() < 1e-9);
    let s = stroke_center(&line(107.0), 2.0, &dashed(&[10.0, 10.0]));
    assert_eq!(on(&s, &[5.0, 6.0, 101.0, 102.0]), [true, false, false, true]);
    // A line shorter than one pattern takes one, scaled down.
    assert_eq!(line_dashes(5.0, false, &[10.0, 10.0]), (vec![2.5, 2.5], 1.25));
    // A pattern that already fits isn't lost to rounding.
    assert_eq!(line_dashes(0.3, false, &[0.1, 0.05]).0.len(), 2);
    let (fitted, _) = line_dashes(3.0 * 0.15, false, &[0.1, 0.05]);
    assert!((fitted[0] - 0.1).abs() < 1e-9, "three patterns, not two: {fitted:?}");
}

#[test]
fn the_public_example_comes_out_the_same() {
    // A 19 px line of [1, 2] draws six patterns of about 3.167.
    let (pattern, offset) = line_dashes(19.0, false, &[1.0, 2.0]);
    assert!((pattern[0] - 1.056).abs() < 1e-3 && (pattern[1] - 2.111).abs() < 1e-3, "{pattern:?}");
    assert!((offset - 0.528).abs() < 1e-3);
}

/// Custom patterns too: the half is of the first dash, which the pattern comes round to at the end.
#[test]
fn a_custom_pattern_ends_on_half_its_first_dash() {
    let s = stroke_center(&line(100.0), 2.0, &dashed(&[12.0, 4.0, 2.0, 2.0]));
    // 6 on, 4 off, 2 on, 2 off, 12 on, …, and the last 6 on.
    assert_eq!(on(&s, &[3.0, 8.0, 11.0, 13.0, 20.0, 97.0, 93.0]), [true, false, true, false, true, true, false]);
    // An odd pattern draws repeated, so it fits as the doubled one.
    let doubled = dash_pattern(&dashed(&[4.0, 2.0, 1.0])).unwrap();
    assert_eq!(doubled, [4.0, 2.0, 1.0, 4.0, 2.0, 1.0]);
    let s = stroke_center(&line(70.0), 2.0, &dashed(&[4.0, 2.0, 1.0]));
    assert_eq!(on(&s, &[1.0, 3.0, 67.5, 69.0]), [true, false, false, true], "half of 4 at each end");
}

#[test]
fn a_closed_path_runs_its_pattern_as_it_is() {
    let rect = Rect::new(0.0, 0.0, 100.0, 50.0).to_path(0.1);
    assert_eq!(subpath_dashes(&rect, &[10.0, 7.0]), (vec![10.0, 7.0], 0.0));
    // The open one measures the same and is fitted.
    let (open, offset) = line_dashes(300.0, false, &[10.0, 7.0]);
    assert!(open[0] != 10.0 && offset > 0.0);
}

#[test]
fn each_subpath_fits_the_pattern_to_its_own_length() {
    let mut two = line(100.0);
    two.move_to((0.0, 50.0));
    two.line_to((107.0, 50.0));
    let s = stroke_center(&two, 2.0, &dashed(&[10.0, 10.0]));
    assert!(s.winding(Point::new(98.0, 0.0)) != 0 && s.winding(Point::new(102.0, 50.0)) != 0, "both end on half a dash");
    assert!(s.winding(Point::new(92.0, 0.0)) == 0 && s.winding(Point::new(99.0, 50.0)) == 0);
}

/// A variable-width stroke is drawn by its own code: its dashes land in the same places.
#[test]
fn a_variable_width_stroke_dashes_the_same_way() {
    let props = StrokeProps { width_profile: Some(WidthProfile::uniform()), ..dashed(&[10.0, 10.0]) };
    let s = stroke_outline(&line(100.0), &props, false);
    assert_eq!(on(&s, &[2.0, 7.0, 13.0, 16.0, 98.0]), [true, false, false, true, true]);
}

/// Nothing to fit: an empty line or a pattern of nothing stays as it is.
#[test]
fn nothing_to_fit_stays_as_it_is() {
    assert_eq!(line_dashes(0.0, false, &[10.0, 10.0]), (vec![10.0, 10.0], 0.0));
    assert_eq!(line_dashes(100.0, false, &[0.0, 0.0]), (vec![0.0, 0.0], 0.0));
    assert!(stroke_center(&BezPath::new(), 2.0, &dashed(&[10.0, 10.0])).elements().is_empty());
}
