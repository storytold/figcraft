//! Brushes: stretch shapes bend along the path, scatter copies are spaced, capped and repeatable,
//! and a width profile (pen pressure) scales them.

use figcraft_doc::{Brush, StrokeAlign, StrokeProps, WidthProfile};
use figcraft_geom::kurbo::{BezPath, Point, Shape};

use crate::brush::{MAX_COPIES, PRESETS, outline};

fn line(len: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((0.0, 0.0));
    p.line_to((len, 0.0));
    p
}

fn brush(preset: &str) -> Brush {
    Brush { preset: preset.into(), ..Default::default() }
}

fn props(weight: f64, b: &Brush) -> StrokeProps {
    StrokeProps { weight, align: StrokeAlign::Center, brush: Some(b.clone()), ..Default::default() }
}

/// What a brush paints stays within half [`StrokeProps::max_weight`] of the path, which the
/// stroke's bounds go by: plain, with copies grown, turned and strayed all they can, and with a
/// width profile thinning or widening it (strays keep their distance, whatever the profile).
#[test]
fn every_preset_draws_inside_its_reach() {
    for p in PRESETS {
        let wild = Brush { size_jitter: 1.0, rotation_jitter: 1.0, scatter: 4.0, ..brush(p.id) };
        for (b, profile) in [(brush(p.id), None), (wild.clone(), None), (wild.clone(), Some(0.2)), (wild, Some(2.0))] {
            let k = StrokeProps { width_profile: profile.map(|s| WidthProfile::new([(0.0, s), (1.0, s)])), ..props(10.0, &b) };
            let out = outline(&line(200.0), 10.0, &k, &b).unwrap();
            let bb = out.bounding_box();
            let reach = k.max_weight() / 2.0;
            assert!(!out.elements().is_empty(), "{}", p.id);
            assert!(
                bb.y0 >= -reach && bb.y1 <= reach && bb.x0 >= -reach && bb.x1 <= 200.0 + reach,
                "{} {b:?} {profile:?}: {bb:?}, reach {reach}",
                p.id
            );
        }
    }
    assert!(outline(&line(10.0), 1.0, &props(1.0, &brush("nope")), &brush("nope")).is_none());
}

#[test]
fn stretch_bends_with_the_path() {
    // Down a vertical line, the taper's widest point sits halfway down, across x.
    let mut p = BezPath::new();
    p.move_to((0.0, 0.0));
    p.line_to((0.0, 100.0));
    let b = brush("taper");
    let out = outline(&p, 20.0, &props(20.0, &b), &b).unwrap();
    let bb = out.bounding_box();
    assert!((bb.height() - 100.0).abs() < 1.0 && (bb.width() - 15.0).abs() < 1.5, "{bb:?}");
    assert_ne!(out.winding(Point::new(0.0, 50.0)), 0, "filled in the middle");
    assert_eq!(out.winding(Point::new(6.0, 2.0)), 0, "pointed at the start");
}

#[test]
fn scatter_is_spaced_capped_and_repeatable() {
    let b = Brush { spacing: 2.0, size_jitter: 0.5, rotation_jitter: 1.0, scatter: 1.0, seed: 7, ..brush("dots") };
    let a = outline(&line(200.0), 10.0, &props(10.0, &b), &b).unwrap();
    assert_eq!(a, outline(&line(200.0), 10.0, &props(10.0, &b), &b).unwrap(), "the same seed, the same copies");
    let c = Brush { seed: 8, ..b.clone() };
    assert_ne!(a, outline(&line(200.0), 10.0, &props(10.0, &c), &c).unwrap());
    // 200 long, a copy every 20: ten circles (each a move-to).
    let moves = |p: &BezPath| p.elements().iter().filter(|e| matches!(e, figcraft_geom::kurbo::PathEl::MoveTo(_))).count();
    assert_eq!(moves(&a), 10);
    // A tiny spacing on a long path stops at the cap.
    let dense = Brush { spacing: 0.0, ..brush("dots") };
    assert!(moves(&outline(&line(1e6), 1.0, &props(1.0, &dense), &dense).unwrap()) <= MAX_COPIES + 1);
}

#[test]
fn a_width_profile_scales_the_brush() {
    let b = brush("dots");
    let mut pr = props(10.0, &b);
    pr.width_profile = Some(WidthProfile::new([(0.0, 0.2), (1.0, 2.0)]));
    let out = outline(&line(300.0), 10.0, &pr, &b).unwrap();
    let parts = figcraft_geom::split_subpaths(&out);
    let (first, last) = (parts.first().unwrap().bounding_box(), parts.last().unwrap().bounding_box());
    assert!(last.width() > first.width() * 3.0, "{first:?} {last:?}");
}

#[test]
fn an_unsanitized_profile_is_bounded_before_sampling() {
    // 100k points, all past the 64-point cap and out of range: the brush reads the clamped profile.
    let raw = WidthProfile { points: (0..100_000).map(|i| figcraft_doc::WidthPoint::new(f64::from(i), 1e300)).collect() };
    let b = brush("taper");
    let mut p = props(1.0, &b);
    p.width_profile = Some(raw);
    let out = outline(&line(50.0), 1.0, &p, &b).unwrap();
    assert!(out.bounding_box().height() <= 2.0 * WidthProfile::MAX_SCALE);
}
