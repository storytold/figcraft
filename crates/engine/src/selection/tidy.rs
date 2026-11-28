//! Tidy up: the moves that even a selection's gaps the way Figma's web app does.
//!
//! Measured in Figma's web app on 2026-11-12 (top-level rectangles): a row, a column, a 2 x 3 grid
//! and its transpose, and many runs on the gap. Figma keeps the order and the sizes of every layer
//! and keeps the first layer where it is. Rows are the groups of layers whose Y ranges overlap
//! (transitively), columns the groups whose X ranges overlap. A row is one row group of layers
//! side by side: its horizontal gaps are evened and every Y kept; a column is the mirror image.
//! Layers of a row may overlap along X, also in a chain: a negative gap counts in the mean. The one gap is the most
//! common existing gap (the smaller of two equally common ones), else the mean rounded to a whole
//! number, half up. A grid (two or more rows and two or more columns, one layer in every cell) is
//! packed along the axis that has more groups: for 2 rows by 3 columns each column is as wide as
//! its widest layer, centred where its layers were, the columns are evened, every layer is
//! centred in its column and every column is stacked from the selection's top; 3 rows by 2
//! columns is the mirror image. Tidy up is unavailable for fewer than three layers, for layers all
//! in one row and one column (overlapping each other), for a diagonal and for an irregular layout.
//!
//! Not measured, FigCraft's choices: a 2 x 2 grid packed as columns; layers in an auto layout flow
//! making it unavailable (the frame places them, and the measurement could not tell this from an
//! irregular layout); the 2,000 layer cap; rotated layers counting by their box; gaps a pixel
//! apart being different gaps; an explicit `spacing` taking two layers; no rounding in a grid with
//! Snap to pixel grid off.

use std::collections::BTreeSet;

use figcraft_doc::{Document, NodeId};
use figcraft_geom::Rect;

use crate::cmd::edit::roots;

const EPS: f64 = 0.01;
const MAX_LAYERS: usize = 2000;

type Item = (NodeId, Rect);
type Move = (NodeId, f64, f64);

/// What a selection is: a row or a column (`vertical`), or a grid packed in units along an axis
/// (`vertical`: the units are rows).
#[derive(Clone, Copy)]
enum Shape {
    Line { vertical: bool },
    Grid { vertical: bool },
}

/// Whether Tidy up is offered for `ids`: three to 2,000 layers with boxes, none in an auto layout
/// flow, forming a row, a column or a grid.
pub fn can_tidy(doc: &Document, ids: &[NodeId]) -> bool {
    available(doc, ids, 3).is_some()
}

/// The page-space moves that tidy `ids` (empty when unavailable or when nothing would move).
/// `spacing` sets the gap and lets two layers through; `snap` rounds a grid's positions across its
/// packing axis to whole pixels.
pub(crate) fn tidy_moves(doc: &Document, ids: &[NodeId], spacing: Option<f64>, snap: bool) -> Vec<Move> {
    let min = if spacing.is_some() { 2 } else { 3 };
    let Some((items, shape)) = available(doc, ids, min) else { return Vec::new() };
    let moves = match shape {
        Shape::Line { vertical } => line_moves(&items, vertical, spacing),
        Shape::Grid { vertical } => grid_moves(&groups(&items, vertical), vertical, spacing, snap),
    };
    moves.into_iter().filter(|(_, dx, dy)| !(dx.is_finite() && dy.is_finite()) || dx.abs() > 1e-9 || dy.abs() > 1e-9).collect()
}

fn available(doc: &Document, ids: &[NodeId], min: usize) -> Option<(Vec<Item>, Shape)> {
    let mut seen = BTreeSet::new();
    let ids: Vec<NodeId> = roots(doc, ids).into_iter().filter(|i| seen.insert(*i)).collect();
    if ids.len() < min || ids.len() > MAX_LAYERS {
        return None;
    }
    let mut items = Vec::with_capacity(ids.len());
    for id in ids {
        let n = doc.node(id)?;
        if n.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.layout.is_some() && !n.child_layout.absolute) {
            return None;
        }
        let b = doc.world_bounds(id)?;
        if ![b.x0, b.y0, b.x1, b.y1].into_iter().all(f64::is_finite) {
            return None;
        }
        items.push((id, b));
    }
    let shape = classify(&items)?;
    Some((items, shape))
}

/// A row: one row group over two or more column groups (layers sharing a column group overlap).
/// A column: the mirror image. A grid: two or
/// more of each, one layer in every cell, packed along the axis with more groups (columns on a tie).
fn classify(items: &[Item]) -> Option<Shape> {
    let cols = groups(items, false);
    let rows = groups(items, true);
    match (cols.len(), rows.len()) {
        (c, 1) if c >= 2 && cols.iter().all(|g| overlapping(g, false)) => Some(Shape::Line { vertical: false }),
        (1, r) if r >= 2 && rows.iter().all(|g| overlapping(g, true)) => Some(Shape::Line { vertical: true }),
        (c, r) if c >= 2 && r >= 2 && one_per_cell(&cols, &rows) => Some(Shape::Grid { vertical: r > c }),
        _ => None,
    }
}

/// Layers that share a group along a row's (`vertical` false) or column's own axis must overlap
/// each other pairwise on the OTHER axis, not merely sit in the same band: a header above two
/// layers side by side is irregular (a row's column group of the header and one of the pair has Y
/// ranges that don't overlap for the pair). The own axis is already covered by `groups`, which
/// chains transitively, so 50 px squares at X 0, 40, 80, 150 are a row although the first and
/// third don't touch.
fn overlapping(group: &[Item], vertical: bool) -> bool {
    let across = |a: Rect, b: Rect| {
        let (a, b) = (span(a, !vertical), span(b, !vertical));
        a.1.min(b.1) - a.0.max(b.0) > EPS
    };
    group.iter().enumerate().all(|(i, a)| group.iter().skip(i + 1).all(|b| across(a.1, b.1)))
}

fn span(b: Rect, vertical: bool) -> (f64, f64) {
    if vertical { (b.y0, b.y1) } else { (b.x0, b.x1) }
}

/// Columns (`vertical` false) or rows: the layers whose ranges on the axis overlap, transitively,
/// in order along the axis.
fn groups(items: &[Item], vertical: bool) -> Vec<Vec<Item>> {
    let mut sorted = items.to_vec();
    sorted.sort_by(|a, b| span(a.1, vertical).0.total_cmp(&span(b.1, vertical).0));
    let mut out: Vec<Vec<Item>> = Vec::new();
    let mut end = f64::MIN;
    for it in sorted {
        let (lo, hi) = span(it.1, vertical);
        match out.last_mut() {
            Some(group) if lo < end - EPS => {
                group.push(it);
                end = end.max(hi);
            }
            _ => {
                out.push(vec![it]);
                end = hi;
            }
        }
    }
    out
}

/// Every (column, row) cell holds exactly one layer.
fn one_per_cell(cols: &[Vec<Item>], rows: &[Vec<Item>]) -> bool {
    cols.iter().all(|c| rows.iter().all(|r| c.iter().filter(|a| r.iter().any(|b| b.0 == a.0)).count() == 1))
}

/// The one gap an axis is evened to: `spacing`, a single gap, the most common gap (the smaller of
/// two equally common ones), else the mean rounded to a whole number, half up.
fn even_gap(gaps: &[f64], spacing: Option<f64>) -> f64 {
    if let Some(s) = spacing {
        return s;
    }
    if let [only] = gaps {
        return *only;
    }
    let count = |g: f64| gaps.iter().filter(|o| (**o - g).abs() <= EPS).count();
    let best = gaps.iter().copied().map(|g| (count(g), g)).reduce(|a, b| if b.0 > a.0 || (b.0 == a.0 && b.1 < a.1) { b } else { a });
    match best {
        Some((n, g)) if n >= 2 => g,
        _ if gaps.is_empty() => 0.0,
        _ => (gaps.iter().sum::<f64>() / gaps.len() as f64 + 0.5 + 1e-9).floor(),
    }
}

/// The even gap of a line of three or more layers, from its derived gap `gap`: raised so that
/// every layer starts strictly after the end of the layer two places before it. With one gap `g`,
/// layer `i + 2` starts at `end(i) + w(i + 1) + 2g`, so `g > -w(i + 1) / 2` for every interior
/// layer, in whole pixels `floor(-w_min / 2) + 1` for the narrowest interior size `w_min`. Measured
/// in Figma's web app on 2026-11-13: interior widths of 10, 20 and 30 among 100s gave -4, -9 and
/// -14, and widths of 100 left the most common gap, -40, alone. Only those three widths were
/// measured; other widths follow the same formula.
///
/// The first layer has no layer two places before it, but the second must still start after it:
/// `g > -w(0)`, in whole pixels `floor(-w(0)) + 1`. Figma kept the order in every run; a narrow
/// first layer was not measured, so this bound keeps that order rather than following a reading.
fn raise_negative_gap(gap: f64, widths: &[f64]) -> f64 {
    let interior = widths.get(1..widths.len().saturating_sub(1)).unwrap_or(&[]);
    let w_min = interior.iter().copied().fold(f64::INFINITY, f64::min);
    let Some(first) = widths.first().copied() else { return gap };
    if widths.len() < 3 || !w_min.is_finite() || !first.is_finite() {
        return gap;
    }
    gap.max((-w_min / 2.0).floor() + 1.0).max((-first).floor() + 1.0)
}

/// A row (`vertical` false) or a column: the first layer stays, each next one follows the last at
/// the even gap, the other axis is untouched.
fn line_moves(items: &[Item], vertical: bool, spacing: Option<f64>) -> Vec<Move> {
    let mut line = items.to_vec();
    line.sort_by(|a, b| span(a.1, vertical).0.total_cmp(&span(b.1, vertical).0));
    let gaps: Vec<f64> = line
        .windows(2)
        .filter_map(|w| match w {
            [a, b] => Some(span(b.1, vertical).0 - span(a.1, vertical).1),
            _ => None,
        })
        .collect();
    let widths: Vec<f64> = line.iter().map(|i| span(i.1, vertical).1 - span(i.1, vertical).0).collect();
    let gap = match spacing {
        Some(s) => s,
        None => raise_negative_gap(even_gap(&gaps, None), &widths),
    };
    let mut pos = line.first().map_or(0.0, |i| span(i.1, vertical).0);
    let mut out = Vec::with_capacity(line.len());
    for (id, b) in line {
        let (lo, hi) = span(b, vertical);
        let d = pos - lo;
        pos += (hi - lo) + gap;
        out.push(if vertical { (id, 0.0, d) } else { (id, d, 0.0) });
    }
    out
}

fn snap_to(v: f64, snap: bool) -> f64 {
    if snap { (v + 0.5).floor() } else { v }
}

/// A unit's box on the packing axis: where it starts and how wide it is (its widest layer, around
/// the middle of the unit's range).
fn unit_box(unit: &[Item], vertical: bool) -> (f64, f64) {
    let lo = unit.iter().map(|i| span(i.1, vertical).0).fold(f64::MAX, f64::min);
    let hi = unit.iter().map(|i| span(i.1, vertical).1).fold(f64::MIN, f64::max);
    let w = unit.iter().map(|i| span(i.1, vertical).1 - span(i.1, vertical).0).fold(0.0, f64::max);
    ((lo + hi) / 2.0 - w / 2.0, w)
}

/// The gaps inside every unit along the other axis, pooled.
fn unit_gaps(units: &[Vec<Item>], vertical: bool) -> Vec<f64> {
    let mut out = Vec::new();
    for unit in units {
        let mut u = unit.clone();
        u.sort_by(|a, b| span(a.1, !vertical).0.total_cmp(&span(b.1, !vertical).0));
        out.extend(u.windows(2).filter_map(|w| match w {
            [a, b] => Some(span(b.1, !vertical).0 - span(a.1, !vertical).1),
            _ => None,
        }));
    }
    out
}

/// A grid packed in `units` (columns, or rows when `vertical`): the units are evened around where
/// they were, every layer is centred in its unit and each unit is laid out from the selection's
/// start on the other axis.
fn grid_moves(units: &[Vec<Item>], vertical: bool, spacing: Option<f64>, snap: bool) -> Vec<Move> {
    let boxes: Vec<(f64, f64)> = units.iter().map(|u| unit_box(u, vertical)).collect();
    let across: Vec<f64> = boxes
        .windows(2)
        .filter_map(|w| match w {
            [a, b] => Some(b.0 - (a.0 + a.1)),
            _ => None,
        })
        .collect();
    let g_units = even_gap(&across, spacing);
    let g_within = even_gap(&unit_gaps(units, vertical), spacing);
    let start = units.iter().flatten().map(|i| span(i.1, !vertical).0).fold(f64::MAX, f64::min);
    let mut at = boxes.first().map_or(0.0, |b| snap_to(b.0, snap));
    let mut out = Vec::new();
    for (unit, (_, w)) in units.iter().zip(&boxes) {
        let mut u = unit.clone();
        u.sort_by(|a, b| span(a.1, !vertical).0.total_cmp(&span(b.1, !vertical).0));
        let mut pos = start;
        for (id, b) in u {
            let (lo, hi) = span(b, vertical);
            let (lo2, hi2) = span(b, !vertical);
            let d_main = snap_to(at + (w - (hi - lo)) / 2.0, snap) - lo;
            let d_cross = pos - lo2;
            pos += (hi2 - lo2) + g_within;
            out.push(if vertical { (id, d_cross, d_main) } else { (id, d_main, d_cross) });
        }
        at += w + g_units;
    }
    out
}
