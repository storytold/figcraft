//! Variable width strokes: measuring paths and outlining strokes whose width follows a profile.

use figcraft_doc::{StrokeAlign, StrokeCap, StrokeJoin, StrokeProps, WidthPreset, WidthProfile};
use figcraft_geom::kurbo::{BezPath, Circle, Point, Rect, Shape};

use crate::stroke_outline;
use crate::width::*;

fn line(x1: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((0.0, 0.0));
    p.line_to((x1, 0.0));
    p
}

fn props(weight: f64, profile: Option<WidthProfile>) -> StrokeProps {
    StrokeProps { weight, align: StrokeAlign::Center, width_profile: profile, ..Default::default() }
}

/// Whether the non-zero fill of `p` covers `pt`.
fn covers(p: &BezPath, x: f64, y: f64) -> bool {
    p.winding(Point::new(x, y)) != 0
}

#[test]
fn measures_paths_by_length() {
    let mut p = line(100.0);
    p.move_to((0.0, 50.0));
    p.line_to((0.0, 150.0));
    let lines = polylines(&p).unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].length(), 100.0);
    let (pt, n) = lines[0].at(0.25).unwrap();
    assert_eq!(pt, Point::new(25.0, 0.0));
    assert!((n.x).abs() < 1e-12 && (n.y - 1.0).abs() < 1e-12, "{n:?}");
    assert_eq!(lines[0].at(1.0).unwrap().0, Point::new(100.0, 0.0));
    let (i, t, d) = nearest(&lines, Point::new(30.0, 5.0)).unwrap();
    assert_eq!((i, d), (0, 5.0));
    assert!((t - 0.3).abs() < 1e-12, "{t}");
    let (i, t, _) = nearest(&lines, Point::new(-3.0, 100.0)).unwrap();
    assert_eq!((i, t), (1, 0.5));
    // A closed circle ends where it starts.
    let c = Circle::new((0.0, 0.0), 10.0).to_path(0.01);
    let l = &polylines(&c).unwrap()[0];
    assert!(l.closed);
    assert_eq!(l.points.first(), l.points.last());
    assert!((l.length() - std::f64::consts::TAU * 10.0).abs() < 0.1, "{}", l.length());
}

#[test]
fn width_follows_the_profile() {
    // A wedge from 10 wide to a point: a triangle of area 500.
    let s = stroke_outline(&line(100.0), &props(10.0, WidthPreset::Wedge.profile()), false);
    assert!((s.area().abs() - 500.0).abs() < 2.0, "{}", s.area());
    assert!(covers(&s, 5.0, 4.0) && !covers(&s, 95.0, 4.0), "wide at the start, thin at the end");
    // A uniform profile matches the plain stroke, caps included.
    for cap in [StrokeCap::None, StrokeCap::Round, StrokeCap::Square] {
        let uniform = StrokeProps { cap_start: cap, cap_end: cap, ..props(10.0, Some(WidthProfile::uniform())) };
        let plain = StrokeProps { width_profile: None, ..uniform.clone() };
        let (a, b) = (stroke_outline(&line(100.0), &uniform, false), stroke_outline(&line(100.0), &plain, false));
        assert!((a.area().abs() - b.area().abs()).abs() < 1.0, "{cap:?}: {} vs {}", a.area(), b.area());
        let (ba, bb) = (a.bounding_box(), b.bounding_box());
        assert!((ba.x0 - bb.x0).abs() < 0.1 && (ba.x1 - bb.x1).abs() < 0.1 && (ba.y1 - bb.y1).abs() < 0.1, "{cap:?}: {ba:?} vs {bb:?}");
    }
    // Scales above 1 widen the stroke past its weight.
    let wide = stroke_outline(&line(100.0), &props(10.0, Some(WidthProfile::new([(0.0, 1.0), (0.5, 3.0), (1.0, 1.0)]))), false);
    assert!((wide.bounding_box().y1 - 15.0).abs() < 0.01, "{:?}", wide.bounding_box());
}

#[test]
fn closed_paths_make_a_ring_per_subpath() {
    let c = Circle::new((50.0, 50.0), 40.0).to_path(0.01);
    let s = stroke_outline(&c, &props(10.0, WidthPreset::Eye.profile()), true);
    assert!(!covers(&s, 50.0, 50.0), "the middle stays open");
    // The eye is widest halfway round (the circle starts at its right-hand point).
    assert!(covers(&s, 14.0, 50.0) && covers(&s, 6.0, 50.0));
    assert!(!covers(&s, 94.0, 50.0) && !covers(&s, 86.0, 50.0), "pointed at the start");
    // Inside alignment stays inside the shape.
    let r = Rect::new(0.0, 0.0, 100.0, 100.0).to_path(0.1);
    let inside = stroke_outline(&r, &StrokeProps { align: StrokeAlign::Inside, ..props(10.0, WidthPreset::Taper.profile()) }, true);
    let b = inside.bounding_box();
    assert!(b.x0 >= -0.01 && b.y0 >= -0.01 && b.x1 <= 100.01 && b.y1 <= 100.01, "{b:?}");
    assert!(inside.area().abs() > 100.0);
}

#[test]
fn joins_and_dashes() {
    let mut corner = BezPath::new();
    corner.move_to((0.0, 0.0));
    corner.line_to((100.0, 0.0));
    corner.line_to((100.0, 100.0));
    let at = |join| {
        let p = StrokeProps { join, ..props(20.0, Some(WidthProfile::uniform())) };
        stroke_outline(&corner, &p, false)
    };
    let (miter, bevel, round) = (at(StrokeJoin::Miter), at(StrokeJoin::Bevel), at(StrokeJoin::Round));
    // The outer corner is (110, -10): the miter reaches it, the others cut it.
    assert!(covers(&miter, 109.0, -9.0));
    assert!(!covers(&bevel, 109.0, -9.0) && !covers(&round, 109.0, -9.0));
    assert!(covers(&round, 106.0, -6.0) && !covers(&bevel, 106.0, -6.0), "round bulges past the bevel");
    // The inner corner is filled.
    assert!(covers(&bevel, 95.0, 5.0) && covers(&bevel, 91.0, 9.0));
    // Dashes: half of a 100 long, 2 wide line.
    let dashed = StrokeProps { dashes: vec![10.0, 10.0], ..props(2.0, Some(WidthProfile::uniform())) };
    let s = stroke_outline(&line(100.0), &dashed, false);
    assert!((s.area().abs() - 100.0).abs() < 1.0, "{}", s.area());
    assert!(covers(&s, 2.0, 0.0) && !covers(&s, 10.0, 0.0) && covers(&s, 20.0, 0.0));
    // Dashes keep the width of where they are on the path.
    let tapered = StrokeProps { dashes: vec![10.0, 10.0], ..props(10.0, WidthPreset::Wedge.profile()) };
    let s = stroke_outline(&line(100.0), &tapered, false);
    assert!(covers(&s, 2.0, 4.0) && !covers(&s, 80.0, 4.0));
    // Zero-length dashes with round caps are dots.
    let dots =
        StrokeProps { dashes: vec![0.0, 10.0], cap_start: StrokeCap::Round, cap_end: StrokeCap::Round, ..props(4.0, Some(WidthProfile::uniform())) };
    let s = stroke_outline(&line(100.0), &dots, false);
    assert!(covers(&s, 10.0, 1.0) && !covers(&s, 5.0, 0.0));
}

#[test]
fn hostile_and_huge_input_is_bounded() {
    let mut bad = BezPath::new();
    bad.move_to((f64::NAN, 0.0));
    bad.line_to((1.0, 1.0));
    assert!(polylines(&bad).is_none());
    let _ = stroke_outline(&bad, &props(4.0, WidthPreset::Eye.profile()), false);
    // A path too long to sample falls back to the uniform stroke.
    let mut long = BezPath::new();
    long.move_to((0.0, 0.0));
    for i in 0..(MAX_WIDTH_SAMPLES + 10) {
        long.line_to((i as f64, (i % 2) as f64 * 5.0));
    }
    assert!(variable_outline(&long, 2.0, &props(2.0, None), &WidthProfile::uniform()).is_none());
    assert!(!stroke_outline(&long, &props(2.0, WidthPreset::Eye.profile()), false).elements().is_empty());
    // Zero width, empty paths, a single point and a dash pattern of zeros.
    assert!(variable_outline(&line(10.0), 0.0, &props(0.0, None), &WidthProfile::uniform()).unwrap().elements().is_empty());
    assert!(variable_outline(&BezPath::new(), 2.0, &props(2.0, None), &WidthProfile::uniform()).unwrap().elements().is_empty());
    let mut dot = BezPath::new();
    dot.move_to((5.0, 5.0));
    dot.line_to((5.0, 5.0));
    let _ = stroke_outline(&dot, &props(2.0, WidthPreset::Eye.profile()), false);
    let zeros = StrokeProps { dashes: vec![0.0, 0.0], ..props(2.0, Some(WidthProfile::uniform())) };
    assert!(!stroke_outline(&line(10.0), &zeros, false).elements().is_empty(), "solid when the pattern is empty");
    // A tiny dash pattern on a long path doesn't explode.
    let tiny = StrokeProps { dashes: vec![0.01, 0.01], ..props(2.0, Some(WidthProfile::uniform())) };
    let s = stroke_outline(&line(1e6), &tiny, false);
    assert!(s.elements().len() < 1_000_000);
}
