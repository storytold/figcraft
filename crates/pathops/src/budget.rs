//! Input caps for the sweep: a boolean, a simplification or an arrangement past them is refused
//! with [`PathOpsError::TooComplex`] instead of running for unbounded time and memory (a crafted
//! file can hold any number of segments, crossing each other as often as it likes).
//!
//! The caps sit far above real designs: the boolean groups in the example files sweep at most 55
//! segments with 155 possible crossings per step and 75 segments over a whole fold, and their
//! largest outline has about 4 000 segments. In a native release build the sweep takes 0.3 s for
//! 200 000 segments and 1.8 s for 2 million crossings, and a union of 1 000 squares one by one
//! (2 million segments swept) takes 1.8 s.

use figcraft_geom::kurbo::{BezPath, ParamCurveExtrema, Rect};

use crate::PathOpsError;

/// The most segments one sweep takes, over all its inputs.
pub const MAX_SEGMENTS: usize = 250_000;

/// The most crossings one sweep may have to split at, counted as the pairs of segments whose
/// bounds overlap (every crossing is one of them).
pub const MAX_CROSSINGS: u64 = 2_000_000;

/// The most segments a fold of several operands sweeps over all its steps ([`FoldBudget`]): each
/// step sweeps the result so far again.
pub const MAX_FOLD_SEGMENTS: usize = 4_000_000;

/// How complex the sweep of some paths is ([`complexity`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Complexity {
    pub segments: usize,
    /// Pairs of segments whose bounds overlap (touching counts), counted up to [`MAX_CROSSINGS`] + 1.
    pub crossings: u64,
}

/// The segments of `paths` and the pairs of them that may cross; `None` past [`MAX_SEGMENTS`]
/// (counting stops there, so a huge input is refused at once).
pub fn complexity<'a>(paths: impl IntoIterator<Item = &'a BezPath>) -> Option<Complexity> {
    let mut boxes = Vec::new();
    for seg in paths.into_iter().flat_map(BezPath::segments) {
        if boxes.len() == MAX_SEGMENTS {
            return None;
        }
        boxes.push(seg.bounding_box());
    }
    Some(Complexity { segments: boxes.len(), crossings: overlapping_pairs(&boxes, MAX_CROSSINGS) })
}

/// Refuse `paths` past [`MAX_SEGMENTS`] or [`MAX_CROSSINGS`].
pub(crate) fn check<'a>(paths: impl IntoIterator<Item = &'a BezPath>) -> Result<(), PathOpsError> {
    match complexity(paths) {
        Some(c) if c.crossings <= MAX_CROSSINGS => Ok(()),
        _ => Err(PathOpsError::TooComplex),
    }
}

/// What a fold of several operands has swept so far, against [`MAX_FOLD_SEGMENTS`].
#[derive(Debug, Default)]
pub struct FoldBudget(usize);

impl FoldBudget {
    /// Count the step combining `acc` with `p`: refused once the fold would sweep more than
    /// [`MAX_FOLD_SEGMENTS`].
    pub fn spend(&mut self, acc: &BezPath, p: &BezPath) -> Result<(), PathOpsError> {
        self.0 = self.0.saturating_add(acc.segments().count()).saturating_add(p.segments().count());
        if self.0 > MAX_FOLD_SEGMENTS { Err(PathOpsError::TooComplex) } else { Ok(()) }
    }
}

/// How many pairs of `boxes` overlap (touching counts), counted up to `limit` + 1, in
/// O(n log n): a sweep across keeps the boxes it is inside, and a box it enters overlaps those
/// of them that neither end above it nor start below it.
fn overlapping_pairs(boxes: &[Rect], limit: u64) -> u64 {
    // The y coordinates by rank, for counting the boxes the sweep is inside by their top and bottom.
    let mut ys: Vec<f64> = boxes.iter().flat_map(|b| [b.y0, b.y1]).collect();
    ys.sort_unstable_by(f64::total_cmp);
    ys.dedup();
    let below = |y: f64| ys.partition_point(|v| *v < y);
    let up_to = |y: f64| ys.partition_point(|v| *v <= y);
    // Entering at x0 before leaving at x1, so boxes that only touch across count.
    let mut events: Vec<(f64, bool, &Rect)> = boxes.iter().flat_map(|b| [(b.x0, false, b), (b.x1, true, b)]).collect();
    events.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let (mut tops, mut bottoms) = (Fenwick::new(ys.len()), Fenwick::new(ys.len()));
    let (mut inside, mut pairs) = (0u64, 0u64);
    for (_, leaving, b) in events {
        if leaving {
            tops.add(below(b.y0), -1);
            bottoms.add(below(b.y1), -1);
            inside = inside.saturating_sub(1);
            continue;
        }
        let ending_above = bottoms.count_below(below(b.y0));
        let starting_below = inside.saturating_sub(tops.count_below(up_to(b.y1)));
        pairs += inside.saturating_sub(ending_above + starting_below);
        if pairs > limit {
            return pairs;
        }
        tops.add(below(b.y0), 1);
        bottoms.add(below(b.y1), 1);
        inside += 1;
    }
    pairs
}

/// Counts by rank, with the count below a rank in O(log n) (a Fenwick tree).
struct Fenwick(Vec<i64>);

impl Fenwick {
    fn new(ranks: usize) -> Self {
        Self(vec![0; ranks + 1])
    }

    fn add(&mut self, rank: usize, by: i64) {
        let mut i = rank + 1;
        while let Some(c) = self.0.get_mut(i) {
            *c += by;
            i += i.isolate_lowest_one();
        }
    }

    /// How many were added at ranks below `rank`.
    fn count_below(&self, rank: usize) -> u64 {
        let (mut i, mut n) = (rank, 0i64);
        while i > 0
            && let Some(c) = self.0.get(i)
        {
            n += c;
            i -= i.isolate_lowest_one();
        }
        u64::try_from(n).unwrap_or(0)
    }
}
