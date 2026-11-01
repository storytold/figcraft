//! Input caps for the sweep: huge inputs are refused at once with a clear error, real ones run.

use std::time::{Duration, Instant};

use figcraft_doc::BoolOp;
use figcraft_geom::kurbo::{BezPath, Rect, Shape};

use crate::{Complexity, FoldBudget, MAX_CROSSINGS, MAX_FOLD_SEGMENTS, MAX_SEGMENTS, PathOpsError, boolean, complexity, faces, simplify};

fn rects(rs: impl IntoIterator<Item = Rect>) -> BezPath {
    let mut p = BezPath::new();
    for r in rs {
        p.extend(r.path_elements(0.1));
    }
    p
}

/// `n` long thin bars across and `n` down: each bar across crosses each bar down 4 times.
fn grid(n: usize) -> (BezPath, BezPath) {
    let len = n as f64 * 4.0;
    let across = rects((0..n).map(|i| Rect::new(0.0, i as f64 * 4.0, len, i as f64 * 4.0 + 2.0)));
    let down = rects((0..n).map(|i| Rect::new(i as f64 * 4.0, 0.0, i as f64 * 4.0 + 2.0, len)));
    (across, down)
}

/// A zigzag of `n` line segments.
fn zigzag(n: usize) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((0.0, 0.0));
    for i in 1..=n {
        p.line_to(((i % 2) as f64, i as f64 * 1e-3));
    }
    p
}

#[test]
fn crossings_count_the_segments_whose_bounds_overlap() {
    let square = rects([Rect::new(0.0, 0.0, 1.0, 1.0)]);
    assert_eq!(complexity([&square]), Some(Complexity { segments: 4, crossings: 4 }), "each side touches its two neighbours");
    let apart = rects([Rect::new(0.0, 0.0, 1.0, 1.0), Rect::new(5.0, 5.0, 6.0, 6.0)]);
    assert_eq!(complexity([&apart]), Some(Complexity { segments: 8, crossings: 8 }));
    let mut plus = BezPath::new();
    plus.move_to((0.0, 5.0));
    plus.line_to((10.0, 5.0));
    plus.move_to((5.0, 0.0));
    plus.line_to((5.0, 10.0));
    assert_eq!(complexity([&plus]).map(|c| c.crossings), Some(1));
    assert_eq!(complexity([]), Some(Complexity::default()));
    // Every crossing is counted, and little else: the bars touch only at their ends.
    let (across, down) = grid(10);
    let c = complexity([&across, &down]).unwrap();
    assert_eq!(c.segments, 80);
    assert!((400..=400 + 12 * 10).contains(&c.crossings), "{c:?}");
}

#[test]
fn a_pattern_of_ten_thousand_dots_is_far_within_the_caps() {
    let dots = rects((0..10_000).map(|i| {
        let (x, y) = (f64::from(i % 100) * 3.0, f64::from(i / 100) * 3.0);
        Rect::new(x, y, x + 2.0, y + 2.0)
    }));
    let c = complexity([&dots]).unwrap();
    assert_eq!(c, Complexity { segments: 40_000, crossings: 40_000 });
    assert!(c.segments * 5 < MAX_SEGMENTS && c.crossings * 40 < MAX_CROSSINGS);
}

#[test]
fn a_boolean_past_the_crossing_cap_is_refused_before_it_runs() {
    // 750 × 750 bars: 2.25 million crossings.
    let (across, down) = grid(750);
    let start = Instant::now();
    assert_eq!(boolean(&[across.clone(), down.clone()], BoolOp::Union), Err(PathOpsError::TooComplex));
    assert!(matches!(faces(&[(across, false), (down, false)]), Err(PathOpsError::TooComplex)));
    assert!(start.elapsed() < Duration::from_secs(5), "{:?}", start.elapsed());
    assert!(PathOpsError::TooComplex.to_string().contains("too complex to combine"));
}

#[test]
fn a_path_past_the_segment_cap_is_refused_without_reading_it_all() {
    assert_eq!(complexity([&zigzag(MAX_SEGMENTS)]).map(|c| c.segments), Some(MAX_SEGMENTS), "at the cap: measured");
    let huge = zigzag(MAX_SEGMENTS + 1);
    assert_eq!(complexity([&huge]), None);
    let start = Instant::now();
    assert_eq!(simplify(&huge), Err(PathOpsError::TooComplex));
    assert!(start.elapsed() < Duration::from_secs(5), "{:?}", start.elapsed());
}

#[test]
fn a_fold_stops_once_it_has_swept_its_budget() {
    let (acc, p) = (zigzag(100_000), rects([Rect::new(0.0, 0.0, 1.0, 1.0)]));
    let mut budget = FoldBudget::default();
    let steps = (0..100).take_while(|_| budget.spend(&acc, &p).is_ok()).count();
    assert_eq!(steps, MAX_FOLD_SEGMENTS / 100_004);
    assert_eq!(budget.spend(&BezPath::new(), &BezPath::new()), Err(PathOpsError::TooComplex), "spent stays spent");
}
